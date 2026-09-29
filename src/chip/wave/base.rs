//! Port of `libSiON-cpp/src/chip/wave/siopm_wave_base.{h,cpp}`.
//!
//! The `TransformerUtil::transform_{pcm,sampler}_data` statics and the
//! `RingWindow` helper (C++ `SinglyLinkedList<double>(22, 0.0, true)` from
//! `src/templates/singly_linked_list.h`) are re-exported here from
//! `crate::utils::transformer_util` so the wave-data classes keep importing
//! them from `super::base` (they are owned by the utils wave, C++
//! `src/utils/transformer_util.cpp`).

use std::cell::RefCell;
use std::rc::Rc;

use crate::sample_data::{AudioStreamWav, WavFormat};

pub use crate::utils::transformer_util::{
    RingWindow, transform_pcm_data, transform_sampler_data,
};

/// Free-function view of `SiopmWaveBase::extract_wave_data` (the C++ static-ish
/// member is called unqualified from the wave-data classes).
pub(crate) fn extract_wave_data(
    p_stream: Option<&Rc<RefCell<AudioStreamWav>>>,
    r_channel_count: &mut i32,
) -> Vec<f64> {
    SiopmWaveBase::extract_wave_data(p_stream, r_channel_count)
}

/// Base class for all SiOPM wave data objects; port of `SiOPMWaveBase`.
#[derive(Clone, Debug)]
pub struct SiopmWaveBase {
    module_type: i32,
}

impl SiopmWaveBase {
    /// C++ `SiOPMWaveBase(SiONModuleType p_module_type = MODULE_MAX)`.
    pub fn new(p_module_type: i32) -> Self {
        SiopmWaveBase {
            module_type: p_module_type,
        }
    }

    /// C++ `SiOPMWaveBase::get_module_type`.
    pub fn get_module_type(&self) -> i32 {
        self.module_type
    }

    /// C++ `SiOPMWaveBase::_extract_wave_data(Ref<AudioStream>, int* r_channel_count)`.
    /// The compat layer only ever carries `AudioStreamWAV` payloads, so the
    /// stream is passed as the already-cast `AudioStreamWav`.
    pub fn extract_wave_data(
        p_stream: Option<&Rc<RefCell<AudioStreamWav>>>,
        r_channel_count: &mut i32,
    ) -> Vec<f64> {
        let Some(stream) = p_stream else {
            return Vec::new();
        };

        let wav = stream.borrow();
        let data_format = wav.format;
        if data_format != WavFormat::Pcm8
            && data_format != WavFormat::Pcm16
        {
            // C++ ERR_FAIL_V_MSG(raw_data, vformat("SiOPMWaveBase: Unsupported WAV file format (%d).", data_format));
            crate::error::err_print_body(
                &format!(
                    "Method/function failed. Returning: []\nSiOPMWaveBase: Unsupported WAV file format ({}).",
                    wav.cpp_ord()
                ),
                false,
            );
            return Vec::new();
        }

        *r_channel_count = if wav.stereo { 2 } else { 1 };
        let mut raw_data: Vec<f64> = Vec::new();

        let mut offset = 0usize;
        while offset < wav.data.len() {
            match data_format {
                WavFormat::Pcm8 => {
                    let value = wav.decode_s8(offset);
                    let sample = (value as f32 as f64) / 127.0; // Max int8.
                    raw_data.push(sample);

                    offset += 1;
                }
                WavFormat::Pcm16 => {
                    let value = wav.decode_s16(offset);
                    let sample = (value as f32 as f64) / 32767.0; // Max int16.
                    raw_data.push(sample);

                    offset += 2;
                }
                _ => {
                    offset += 1;
                }
            }
        }

        raw_data
    }
}
