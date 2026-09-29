//! `sion-play` — command-line MML player (port of the libSiON-cpp
//! `sion-cpp-play` CLI surface, minus PortAudio which becomes cpal here).
//!
//! Offline: renders to a 16-bit PCM WAV via `hound`. Realtime: cpal output
//! stream; the engine lives on the main thread (Rc<RefCell> everywhere)
//! and feeds a mutex ring buffer that the audio callback drains.

use std::collections::VecDeque;
use std::error::Error;
use std::fs;
use std::process::exit;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use libsion::core::core as sion;
use libsion::{DriverEvent, SiONDriver};

const DEFAULT_MML: &str = "%0,0 v15 o4 c8 d e f g a b >c8 o5 <cdefgab o4 >c2;";

#[derive(Default)]
struct Cli {
    file: Option<String>,
    mml: Option<String>,
    out: Option<String>,
    play: bool,
    time_secs: f64,
    block: i32,
    channels: i32,
    events: bool,
    repeat: bool,
    volume: f64,
    device: Option<String>,
    list_devices: bool,
}

fn usage(err: bool) -> ! {
    let print: fn(&str) = if err {
        |s| eprintln!("{}", s)
    } else {
        |s| println!("{}", s)
    };
    print("sion-play - play or render SiMML through libSiON-rs");
    print("");
    print("usage: sion-play [OPTIONS]");
    print("  -f, --file PATH      MML source file (.txt/.mml/.sionmml)");
    print("  -m, --mml STRING     MML source string (default demo tune if neither)");
    print("  -o, --out FILE.WAV   offline render to 16-bit PCM WAV");
    print("      --play           realtime playback via the default output device");
    print("  -t, --time SECONDS   render/playback length cap (offline default 30)");
    print("  -b, --block N        driver block length: 2048, 4096 or 8192 (default 2048)");
    print("  -c, --channels N     WAV channels: 1 or 2 (default 2)");
    print("      --events         print system commands after compile + live events");
    print("      --repeat         loop realtime playback when the sequence finishes");
    print("      --volume F       master volume 0.0-1.0 (default 1.0)");
    print("      --device NAME    output device name substring or index");
    print("      --list-devices   list output devices and exit");
    exit(if err { 2 } else { 0 });
}

fn parse_args() -> Cli {
    let mut cli = Cli {
        time_secs: -1.0,
        block: 2048,
        channels: 2,
        volume: 1.0,
        ..Default::default()
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut take = |cli_flag: &mut Option<String>| {
            if let Some(v) = args.next() {
                *cli_flag = Some(v);
            } else {
                usage(true);
            }
        };
        match arg.as_str() {
            "-f" | "--file" => take(&mut cli.file),
            "-m" | "--mml" => take(&mut cli.mml),
            "-o" | "--out" => take(&mut cli.out),
            "--play" => cli.play = true,
            "-t" | "--time" => {
                let v = args.next().unwrap_or_else(|| usage(true));
                cli.time_secs = v.parse().unwrap_or_else(|_| usage(true));
            }
            "-b" | "--block" => {
                let v = args.next().unwrap_or_else(|| usage(true));
                cli.block = v.parse().unwrap_or_else(|_| usage(true));
            }
            "-c" | "--channels" => {
                let v = args.next().unwrap_or_else(|| usage(true));
                cli.channels = v.parse().unwrap_or_else(|_| usage(true));
            }
            "--events" => cli.events = true,
            "--repeat" => cli.repeat = true,
            "--volume" => {
                let v = args.next().unwrap_or_else(|| usage(true));
                cli.volume = v.parse().unwrap_or_else(|_| usage(true));
            }
            "--device" => take(&mut cli.device),
            "--list-devices" => cli.list_devices = true,
            "-h" | "--help" => usage(false),
            other => {
                eprintln!("sion-play: unknown argument '{}'", other);
                usage(true);
            }
        }
    }
    if !cli.play && cli.out.is_none() {
        cli.play = true;
    }
    cli
}

fn load_mml(cli: &Cli) -> String {
    let raw = match (&cli.file, &cli.mml) {
        (Some(path), _) => fs::read_to_string(path)
            .unwrap_or_else(|e| fail(&format!("sion-play: cannot read '{}': {}", path, e))),
        (_, Some(mml)) => mml.clone(),
        _ => DEFAULT_MML.to_string(),
    };
    raw.trim_start_matches('\u{feff}').to_string()
}

fn fail(msg: &str) -> ! {
    eprintln!("{}", msg);
    exit(1);
}

fn print_events(driver: &mut SiONDriver, enabled: bool, stopped: Arc<AtomicBool>) {
    driver.on_event = Some(Box::new(move |event: &DriverEvent| {
        let kind = event.get_event_type();
        if enabled {
            match event {
                DriverEvent::Track(track) => {
                    let t = track.borrow();
                    println!(
                        "[event] {} id={} note={} buffer={}",
                        kind,
                        t.get_event_trigger_id(),
                        t.get_note(),
                        t.get_buffer_index()
                    );
                }
                DriverEvent::Event(_) => println!("[event] {}", kind),
            }
        }
        if kind == "stream_stopped" {
            stopped.store(true, Ordering::SeqCst);
        }
    }));
}

fn dump_system_commands(driver: &SiONDriver) {
    let data = driver.get_data();
    let Some(data) = data else { return };
    let inner = data.borrow().data.clone();
    let commands = inner.borrow().base.get_system_commands();
    for command in commands {
        let c = command.borrow();
        println!("[command] {} {} {}", c.command, c.number, c.content);
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = parse_args();

    if cli.list_devices {
        let host = cpal::default_host();
        for (index, device) in host.output_devices()?.enumerate() {
            println!("[{}] {}", index, device.to_string());
        }
        return Ok(());
    }

    let mml = load_mml(&cli);
    let events_enabled = cli.events;

    sion::initialize();
    let exit_code = run(&cli, &mml, events_enabled);
    sion::finalize();

    match exit_code {
        Ok(()) => Ok(()),
        Err(e) => {
            eprintln!("{}", e);
            exit(1);
        }
    }
}

fn run(cli: &Cli, mml: &str, events_enabled: bool) -> Result<(), Box<dyn Error>> {
    let mut driver = SiONDriver::new(cli.block, cli.channels, 44100, 0);
    driver.set_volume(cli.volume);
    driver.set_auto_stop(true);

    if let Some(path) = &cli.out {
        let secs = if cli.time_secs > 0.0 {
            cli.time_secs
        } else {
            30.0
        };
        let samples = (secs * 44100.0 * cli.channels as f64) as i32;
        println!(
            "sion-play: rendering {} channels x {:.1}s to {}",
            cli.channels, secs, path
        );
        let buffer = driver.render_mml(mml.to_string(), samples, cli.channels, true);
        let peak = buffer
            .iter()
            .fold(0.0f64, |acc, v| acc.max(v.abs()));
        write_wav(path, &buffer, cli.channels)?;
        println!("sion-play: wrote {} frames (peak {:.4})", buffer.len() / cli.channels as usize, peak);
        return Ok(());
    }

    // --- realtime (cpal) ----------------------------------------------------

    let host = cpal::default_host();
    let device = if let Some(spec) = &cli.device {
        let matches: Vec<_> = host
            .output_devices()?
            .filter(|d| {
                let name = d.to_string();
                name.contains(spec.as_str())
                    || spec
                        .parse::<usize>()
                        .map(|_| false)
                        .unwrap_or(false)
            })
            .collect();
        // Index form (same ordering as --list-devices) handled separately.
        if matches.is_empty()
            && let Ok(index) = spec.parse::<usize>()
        {
            let mut all = host.output_devices()?;
            match all.nth(index) {
                Some(device) => device,
                None => fail(&format!("sion-play: no device at index {}", index)),
            }
        } else if let Some(device) = matches.into_iter().next() {
            device
        } else {
            fail(&format!("sion-play: no output device matching '{}'", spec));
        }
    } else {
        host.default_output_device()
            .ok_or("sion-play: no default output device")?
    };

    // The driver is fixed at 44100; prefer a supported f32 stereo config.
    let config = match device.supported_output_configs()?.find(|c| {
        c.channels() == 2
            && (c.min_sample_rate() as u32) <= 44100
            && (c.max_sample_rate() as u32) >= 44100
            && c.sample_format() == cpal::SampleFormat::F32
    }) {
        Some(c) => c.with_sample_rate(44100).config(),
        None => device.default_output_config()?.config(),
    };

    let ring: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
    let stopped = Arc::new(AtomicBool::new(false));
    print_events(&mut driver, events_enabled, stopped.clone());

    let cb_ring = ring.clone();
    let stream = device.build_output_stream(
        config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            let mut queue = cb_ring.lock().unwrap();
            for sample in data.iter_mut() {
                *sample = queue.pop_front().unwrap_or(0.0);
            }
        },
        |err| eprintln!("[audio] {}", err),
        None,
    )?;
    stream.play()?;

    let block = cli.block as usize;
    let want = block * 2 * 4;
    let mut scratch = vec![0.0f32; block * 2];
    let limit = if cli.time_secs > 0.0 {
        Some(Duration::from_secs_f64(cli.time_secs))
    } else {
        None
    };
    let started = Instant::now();

    loop {
        stopped.store(false, Ordering::SeqCst);
        let compiled = driver.compile(mml.to_string());
        if events_enabled {
            dump_system_commands(&driver);
        }
        driver.play(&compiled, true);

        while !stopped.load(Ordering::SeqCst) {
            driver.update();
            while ring.lock().unwrap().len() < want {
                driver.render_chunk(&mut scratch);
                ring.lock().unwrap().extend(scratch.iter().copied());
            }
            thread::sleep(Duration::from_millis(3));
            if let Some(limit) = limit {
                if started.elapsed() > limit {
                    break;
                }
            }
        }

        driver.stop();
        if !cli.repeat {
            break;
        }
    }

    drop(stream);
    println!("sion-play: finished.");
    Ok(())
}

fn write_wav(path: &str, samples: &[f64], channels: i32) -> Result<(), Box<dyn Error>> {
    let spec = hound::WavSpec {
        channels: channels as u16,
        sample_rate: 44100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for sample in samples {
        let scaled = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
        writer.write_sample(scaled)?;
    }
    writer.finalize()?;
    Ok(())
}
