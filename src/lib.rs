//! libSiON-rs — pure-Rust port of libSiON-cpp (GDSiON software synthesizer).
//!
//! Module layout mirrors the C++ source tree of `libSiON-cpp/src`:
//! - `chip`      — SiOPM sound chip (ref tables, waves, channels, params)
//! - `effector`  — filters and effects
//! - `sequencer` — MML compiler + sequencer + tracks
//! - `events`    — dispatch/event queue used by the driver
//! - `utils`     — string/translator/fader helpers
//! - `core`      — SiONCore / SiONData / SiONVoice / SiONDriver public API

pub mod error;
pub mod math;
pub mod random;
pub mod sample_data;
pub mod sion_enums;

pub mod chip;
pub mod core;
pub mod effector;
pub mod events;
pub mod sequencer;
pub mod utils;

#[cfg(feature = "wasm")]
pub mod wasm;

/// C++ `sion` namespace (`sion_core.h`).
pub use core::core as sion;

pub use core::data::SiONData;
pub use core::driver::{DriverEvent, ExceptionMode, SiONDriver, VERSION, VERSION_FLAVOR};
pub use core::voice::SiONVoice;
pub use events::sion_event::SionEvent;
pub use events::sion_track_event::SiONTrackEvent;
pub use sample_data::SampleData;
pub use sequencer::track::SiMMLTrack;
