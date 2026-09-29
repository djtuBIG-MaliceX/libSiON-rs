//! SiONVoice parameter containers (wave-3 · chip params).
//!
//! Ports `libSiON-cpp/src/chip/siopm_operator_params.{h,cpp}` and
//! `libSiON-cpp/src/chip/siopm_channel_params.{h,cpp}`. Consumed by
//! `TranslatorUtil` (friend; wave-4) and the SiOPM channels (wave-6).

pub mod channel_params;
pub mod operator_params;
