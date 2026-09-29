//! `sion-mq` — macroquad HUD player for SiMML (wave-11).
//!
//! Optional binary, built only with `--features macroquad`. Opens a
//! 960x540 window showing tempo/position/track stats, a rolling event
//! log, peak + ring-fill meters and playback controls.
//!
//! Threading model mirrors `sion-play`: the engine (Rc/!Send) lives on
//! the main thread inside the `#[macroquad::main]` async loop; the cpal
//! callback only drains a mutex ring buffer that the main thread pumps
//! via `driver.update()` + `render_chunk()`. The `on_event` closure
//! writes ONLY the shared `Hud`, never the driver (it is borrowed
//! during `update()`).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::process::exit;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use macroquad::prelude::*;

use libsion::core::core as sion;
use libsion::{DriverEvent, SiONDriver};

const DEFAULT_MML: &str = "%0,0 t140 v15 l8 o4 cdef gagf edc<b c2;";

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

const LOG_LIMIT: usize = 14;
const LOG_X: f32 = 28.0;
const LOG_Y0: f32 = 124.0;
const LOG_LINE_H: f32 = 21.0;
const LOG_FONT: f32 = 17.0;
const METER_Y: f32 = 448.0;
const METER_W: f32 = 200.0;
const METER_H: f32 = 16.0;

/// Parsed command line (subset of the `sion-play` surface).
#[derive(Default)]
struct Cli {
    file: Option<String>,
    mml: Option<String>,
    time_secs: f64,
    block: i32,
    volume: f64,
    device: Option<String>,
    list_devices: bool,
}

/// Mutable HUD state shared between the `on_event` closure (writer)
/// and the draw pass (reader). Never touched while the driver borrow
/// that triggers the closure is still live beyond the closure itself.
#[derive(Default)]
struct Hud {
    log: VecDeque<String>,
    status: String,
    peak: f32,
    finished: bool,
}

impl Hud {
    fn push(&mut self, line: String) {
        self.log.push_front(line);
        while self.log.len() > LOG_LIMIT {
            self.log.pop_back();
        }
    }
}

/// Per-frame draw inputs collected from driver/ring before borrowing Hud.
struct View {
    source: String,
    bpm: f64,
    position: f64,
    tracks: i32,
    block: i32,
    ring_ratio: f32,
    volume: f64,
}

fn usage(err: bool) -> ! {
    let print: fn(&str) = if err {
        |s| eprintln!("{}", s)
    } else {
        |s| println!("{}", s)
    };
    print("sion-mq - macroquad HUD player for SiMML (build: --features macroquad)");
    print("");
    print("usage: sion-mq [OPTIONS]");
    print("  -f, --file PATH      MML source file (.txt/.mml/.sionmml)");
    print("  -m, --mml STRING     MML source string (default demo tune if neither)");
    print("  -t, --time SECONDS   playback cap (0 = play once to the end, no hard exit)");
    print("  -b, --block N        driver block length: 2048, 4096 or 8192 (default 2048)");
    print("      --volume F       master volume 0.0-1.0 (default 1.0)");
    print("      --device NAME    output device name substring or index");
    print("      --list-devices   list output devices and exit");
    exit(if err { 2 } else { 0 });
}

fn parse_args() -> Cli {
    let mut cli = Cli {
        time_secs: 0.0,
        block: 2048,
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
            "-t" | "--time" => {
                let v = args.next().unwrap_or_else(|| usage(true));
                cli.time_secs = v.parse().unwrap_or_else(|_| usage(true));
            }
            "-b" | "--block" => {
                let v = args.next().unwrap_or_else(|| usage(true));
                cli.block = v.parse().unwrap_or_else(|_| usage(true));
            }
            "--volume" => {
                let v = args.next().unwrap_or_else(|| usage(true));
                cli.volume = v.parse().unwrap_or_else(|_| usage(true));
            }
            "--device" => take(&mut cli.device),
            "--list-devices" => cli.list_devices = true,
            "-h" | "--help" => usage(false),
            other => {
                eprintln!("sion-mq: unknown argument '{}'", other);
                usage(true);
            }
        }
    }
    cli
}

fn load_mml(cli: &Cli) -> String {
    let raw = match (&cli.file, &cli.mml) {
        (Some(path), _) => fs::read_to_string(path)
            .unwrap_or_else(|e| fail(&format!("sion-mq: cannot read '{}': {}", path, e))),
        (_, Some(mml)) => mml.clone(),
        _ => DEFAULT_MML.to_string(),
    };
    raw.trim_start_matches('\u{feff}').to_string()
}

fn fail(msg: &str) -> ! {
    eprintln!("{}", msg);
    exit(1);
}

fn note_name(note: i32) -> String {
    let name = NOTE_NAMES[note.rem_euclid(12) as usize];
    let octave = note.div_euclid(12) - 1;
    format!("{}{}", name, octave)
}

fn list_devices() {
    let host = cpal::default_host();
    match host.output_devices() {
        Ok(devices) => {
            for (index, device) in devices.enumerate() {
                println!("[{}] {}", index, device);
            }
        }
        Err(e) => fail(&format!("sion-mq: cannot enumerate devices: {}", e)),
    }
    exit(0);
}

fn select_device(spec: &str) -> cpal::Device {
    let host = cpal::default_host();
    if let Ok(index) = spec.parse::<usize>()
        && let Some(device) = host
            .output_devices()
            .ok()
            .and_then(|mut devices| devices.nth(index))
    {
        return device;
    }
    host.output_devices()
        .ok()
        .and_then(|mut devices| devices.find(|d| d.to_string().contains(spec)))
        .unwrap_or_else(|| fail(&format!("sion-mq: no output device matching '{}'", spec)))
}

fn build_stream(
    device: &cpal::Device,
    ring: &Arc<Mutex<VecDeque<f32>>>,
) -> cpal::Stream {
    let config = match device.supported_output_configs() {
        Ok(mut configs) => match configs.find(|c| {
            c.channels() == 2
                && c.min_sample_rate() <= 44100
                && c.max_sample_rate() >= 44100
                && c.sample_format() == cpal::SampleFormat::F32
        }) {
            Some(c) => c.with_sample_rate(44100).config(),
            None => device
                .default_output_config()
                .unwrap_or_else(|e| fail(&format!("sion-mq: no output config: {}", e)))
                .config(),
        },
        Err(e) => fail(&format!("sion-mq: cannot query output configs: {}", e)),
    };

    let cb_ring = ring.clone();
    device
        .build_output_stream(
            config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let mut queue = cb_ring.lock().expect("sion-mq: ring lock poisoned");
                for sample in data.iter_mut() {
                    *sample = queue.pop_front().unwrap_or(0.0);
                }
            },
            |err| eprintln!("[audio] {}", err),
            None,
        )
        .unwrap_or_else(|e| fail(&format!("sion-mq: cannot build output stream: {}", e)))
}

fn drain_ring(ring: &Arc<Mutex<VecDeque<f32>>>) {
    let deadline = Instant::now() + Duration::from_millis(400);
    while Instant::now() < deadline
        && !ring.lock().expect("sion-mq: ring lock poisoned").is_empty()
    {
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn install_event_handler(driver: &mut SiONDriver, hud: &Rc<RefCell<Hud>>) {
    let hud_events = hud.clone();
    driver.on_event = Some(Box::new(move |event: &DriverEvent| {
        let kind = event.get_event_type();
        let line = match event {
            DriverEvent::Track(track) => {
                let t = track.borrow();
                format!(
                    "{} {:>6} (n={})",
                    t.get_event_type(),
                    note_name(t.get_note()),
                    t.get_note()
                )
            }
            DriverEvent::Event(_) => kind.clone(),
        };
        let mut hud = hud_events.borrow_mut();
        if kind == "stream_started" {
            hud.finished = false;
            hud.status = "playing".to_string();
        } else if kind == "stream_stopped" {
            hud.finished = true;
        }
        hud.push(line);
    }));
}

fn draw_bar(x: f32, label: &str, ratio: f32, color: Color) {
    draw_text(label, x, METER_Y - 6.0, 14.0, GRAY);
    draw_rectangle(x, METER_Y, METER_W, METER_H, Color::from_rgba(40, 44, 52, 255));
    draw_rectangle(
        x,
        METER_Y,
        METER_W * ratio.clamp(0.0, 1.0),
        METER_H,
        color,
    );
}

fn draw_hud(hud: &mut Hud, view: &View) {
    clear_background(Color::from_rgba(14, 16, 22, 255));

    draw_text("sion-mq", LOG_X, 52.0, 34.0, GOLD);
    let source_w = measure_text(&view.source, None, 20, 1.0).width;
    draw_text(
        &view.source,
        960.0 - LOG_X - source_w,
        48.0,
        20.0,
        GRAY,
    );

    let stats = format!(
        "bpm {:.0}   pos {:.1}s   tracks {:>2}   block {}",
        view.bpm, view.position, view.tracks, view.block
    );
    draw_text(&stats, LOG_X, 88.0, 20.0, LIGHTGRAY);

    for (index, line) in hud.log.iter().enumerate() {
        let alpha = (255i32 - index as i32 * 15).max(48) as u8;
        draw_text(
            line,
            LOG_X,
            LOG_Y0 + index as f32 * LOG_LINE_H,
            LOG_FONT,
            Color::from_rgba(200, 205, 215, alpha),
        );
    }

    const PEAK_COLOR: Color = Color::new(0.90, 0.25, 0.25, 1.0);
    draw_bar(LOG_X, "peak", hud.peak, PEAK_COLOR);
    draw_bar(
        LOG_X + METER_W + 40.0,
        "ring",
        view.ring_ratio,
        Color::from_rgba(80, 160, 255, 255),
    );

    if !hud.status.is_empty() {
        draw_text(&hud.status, LOG_X, 500.0, 22.0, YELLOW);
    }
    let footer = format!(
        "[Space] replay   [Esc] quit   vol {:.2}",
        view.volume
    );
    draw_text(&footer, LOG_X, 526.0, 17.0, GRAY);
}

/// Window descriptor consumed by `#[macroquad::main]` (ident form only
/// in macroquad_macro 0.1.8: takes a `fn() -> Conf`).
fn window_conf() -> Conf {
    Conf {
        window_title: "sion-mq".to_string(),
        window_width: 960,
        window_height: 540,
        window_resizable: false,
        ..Default::default()
    }
}

/// macroquad entry: window + main-thread engine loop (960x540).
/// macroquad 0.4.16 has no `window_should_close()` — the title-bar close
/// button terminates the process through miniquad directly, so Escape is
/// the graceful in-loop quit path.
#[macroquad::main(window_conf)]
async fn main() {
    let cli = parse_args();
    if cli.list_devices {
        list_devices();
    }

    let mml = load_mml(&cli);
    let source = match &cli.file {
        Some(path) => format!("file:{}", std::path::Path::new(path).file_name().unwrap_or_default().to_string_lossy()),
        None => "inline".to_string(),
    };

    sion::initialize();

    let mut driver = SiONDriver::new(cli.block, 2, 44100, 0);
    driver.set_volume(cli.volume);
    driver.set_auto_stop(true);

    let host = cpal::default_host();
    let device = match &cli.device {
        Some(spec) => select_device(spec),
        None => host
            .default_output_device()
            .unwrap_or_else(|| fail("sion-mq: no default output device")),
    };

    let ring: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
    let hud: Rc<RefCell<Hud>> = Rc::new(RefCell::new(Hud {
        status: "compiling...".to_string(),
        ..Default::default()
    }));
    install_event_handler(&mut driver, &hud);

    let stream = build_stream(&device, &ring);
    stream
        .play()
        .unwrap_or_else(|e| fail(&format!("sion-mq: cannot start stream: {}", e)));

    let block = cli.block as usize;
    let want = block * 2 * 4;
    let mut scratch = vec![0.0f32; block * 2];
    let limit = if cli.time_secs > 0.0 {
        Some(Duration::from_secs_f64(cli.time_secs))
    } else {
        None
    };

    let mut want_start = true;
    let mut ever_played = false;
    let mut playing = false;
    let mut started = Instant::now();

    loop {
        if want_start {
            hud.borrow_mut().status = "compiling...".to_string();
            let compiled = driver.compile(mml.clone());
            driver.play(&compiled, true);
            playing = true;
            ever_played = true;
            want_start = false;
            started = Instant::now();
        }

        driver.update();

        let mut chunk_peak = 0.0f32;
        if playing {
            while ring.lock().expect("sion-mq: ring lock poisoned").len() < want {
                driver.render_chunk(&mut scratch);
                chunk_peak = chunk_peak.max(scratch.iter().fold(0.0f32, |a, v| a.max(v.abs())));
                ring.lock()
                    .expect("sion-mq: ring lock poisoned")
                    .extend(scratch.iter().copied());
            }
        }

        if playing {
            let timed_out = limit.is_some_and(|l| started.elapsed() > l);
            let done = hud.borrow().finished && !driver.is_streaming();
            if timed_out {
                driver.stop();
                hud.borrow_mut().status = "time limit reached - exiting".to_string();
                drain_ring(&ring);
                break;
            }
            if done {
                driver.stop();
                playing = false;
                hud.borrow_mut().status = "finished - [Space] to replay".to_string();
            }
        }

        if !playing && ever_played && is_key_pressed(KeyCode::Space) {
            want_start = true;
            let mut hud_ref = hud.borrow_mut();
            hud_ref.finished = false;
            hud_ref.peak = 0.0;
        }

        if is_key_pressed(KeyCode::Escape) {
            driver.stop();
            break;
        }

        let ring_len = ring.lock().expect("sion-mq: ring lock poisoned").len();
        let view = View {
            source: source.clone(),
            bpm: driver.get_bpm(),
            position: driver.get_streaming_position(),
            tracks: driver.get_track_count(),
            block: cli.block,
            ring_ratio: ring_len as f32 / want as f32,
            volume: driver.get_volume(),
        };
        {
            let mut hud_ref = hud.borrow_mut();
            hud_ref.peak = if chunk_peak > 0.0 {
                chunk_peak
            } else if playing {
                hud_ref.peak * 0.9
            } else {
                hud_ref.peak * 0.7
            };
            draw_hud(&mut hud_ref, &view);
        }

        next_frame().await;
    }

    drop(stream);
    sion::finalize();
}
