# libSiON-rs

A behavior-faithful Rust port of [libSiON-cpp](https://github.com/djtuBIG-MaliceX/libSiON-cpp) (based on the GDSiON
software synthesizer, originally SiON for Flash).

Three builds share one engine library (`libSiON`):

- a headless CLI,
- a macroquad windowed player, and
- a WASM playground for the browser.

## Requirements

- Rust 1.85+ (edition 2024), native toolchain (MSVC on Windows).
- Optional (Web): `rustup target add wasm32-unknown-unknown`,
  `cargo install wasm-bindgen-cli`, Python for the dev server.

## Features

| feature      | default | effect                                            |
|--------------|---------|---------------------------------------------------|
| `native-cli` | yes     | `sion-play` bin (pulls `cpal` + `hound`)          |
| `macroquad`  | no      | `sion-mq` bin (pulls `macroquad`)                 |
| `wasm`       | no      | `#[wasm_bindgen] WasmPlayer` cdylib surface       |

The engine itself (`cargo build --no-default-features`) is pure Rust with
only `regex` — it compiles to `wasm32-unknown-unknown`.

## 1. CLI — `sion-play`

Play a `.txt`/`.sionmml` SiMML file offline to WAV or in realtime:

```
cargo run --release --bin sion-play -- -f my_song.sionmml --play
cargo run --release --bin sion-play -- -f my_song.sionmml -o out.wav -t 60
cargo run --release --bin sion-play -- -m "%0,0 t140 v15 l8 o4 cdef g2;" --play
```

```
-f, --file PATH      MML source file (.txt/.mml/.sionmml)
-m, --mml STRING     MML source string (default demo tune if neither)
-o, --out FILE.WAV   offline render to 16-bit PCM WAV
    --play           realtime playback via the default output device
-t, --time SECONDS   render/playback length cap (offline default 30)
-b, --block N        driver block length: 2048, 4096 or 8192 (default 2048)
-c, --channels N     WAV channels: 1 or 2 (default 2)
    --events         print system commands after compile + live events
    --repeat         loop realtime playback when the sequence finishes
    --volume F       master volume 0.0-1.0 (default 1.0)
    --device NAME    output device name substring or index
    --list-devices   list output devices and exit
```

## 2. Windowed player — `sion-mq` (macroquad, optional)

Same engine, but the main loop runs on the macroquad game loop with an
in-window HUD: tempo/stream position/track count, a rolling note-on/off +
system-event log, peak and ring-fill meters, volume. The engine lives on
the main thread; the cpal callback only drains a ring buffer.

```
cargo run --release --features macroquad --bin sion-mq -- -f my_song.sionmml
cargo run --release --features macroquad --bin sion-mq -- -m "..." -t 30 --volume 0.5
```

Accepts the same `-f/-m/-t/--volume/--device` flags. Controls:
`Space` replay, `Esc` quit.

## 3. Web playground — `web/` (WASM)

Paste-able SiMML textarea, Play/Stop/Loop, volume slider, and a canvas
oscilloscope + peak meter, driven by the engine through a
`ScriptProcessorNode`:

```
pwsh web/build.ps1          # wasm target + wasm-bindgen glue into web/pkg/
cd web
python -m http.server 8000  # load in web browser at http://localhost:8000
```

`web/build.ps1 -SkipInstall` skips the rustup/wasm-bindgen-cli checks.
The Rust surface is `WasmPlayer::new(sample_rate, channels)` /
`play(mml)` / `pump(float32array)` / `stop()` / `is_streaming()` /
`set_volume(f64)` (see `src/wasm.rs`).

## Development

```
cargo test --lib                  # engine unit/integration tests
cargo check --all-targets                          # native
cargo check --all-targets --features macroquad     # + sion-mq
cargo check --target wasm32-unknown-unknown --no-default-features --features wasm
```

## AI DISCLOSURE

This specific port is a result of Qwen3.8-flash-next-oQ5-mtp and several minimax code cli and opencode runs, as derived from libSiON-cpp, which in itself is derived from GDSiON, which is derived from SiON.
