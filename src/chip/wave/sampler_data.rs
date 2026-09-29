//! Port of `libSiON-cpp/src/chip/wave/siopm_wave_sampler_data.{h,cpp}`.
//!
//! The head-silence ring window (C++ `SinglyLinkedList<double>(22, 0.0,
//! true)`) is provided by `super::base::RingWindow`.

use super::base::{extract_wave_data, transform_sampler_data, RingWindow, SiopmWaveBase};
use crate::sample_data::SampleData;
use crate::sion_enums::MODULE_SAMPLE;

#[derive(Clone, Debug)]
pub struct SiopmWaveSamplerData {
    base: SiopmWaveBase,

    wave_data: Vec<f64>,
    channel_count: i32,
    pan: i32,
    // This flag is only available for non-loop samples.
    ignore_note_off: bool,

    // Wave positions in the sample count.
    start_point: i32,
    end_point: i32,
    // -1 means no looping.
    loop_point: i32,
}

impl SiopmWaveSamplerData {
    /// C++ `SiOPMWaveSamplerData(Ref<SampleData> p_data = Ref<SampleData>(),
    /// bool p_ignore_note_off = false, int p_pan = 0,
    /// int p_src_channel_count = 2, int p_channel_count = 0)`.
    pub fn new(
        p_data: &SampleData,
        p_ignore_note_off: bool,
        p_pan: i32,
        p_src_channel_count: i32,
        p_channel_count: i32,
    ) -> Self {
        let mut data = SiopmWaveSamplerData {
            base: SiopmWaveBase::new(MODULE_SAMPLE),
            wave_data: Vec::new(),
            channel_count: 0,
            pan: 0,
            ignore_note_off: false,
            start_point: 0,
            end_point: 0,
            loop_point: -1,
        };
        data.prepare_wave_data(p_data, p_src_channel_count, p_channel_count);
        data.set_ignore_note_off(p_ignore_note_off);
        data.pan = p_pan;
        data
    }

    /// C++ `SiOPMWaveBase::get_module_type` (inherited).
    pub fn get_module_type(&self) -> i32 {
        self.base.get_module_type()
    }

    /// C++ `SiOPMWaveSamplerData::get_wave_data`.
    pub fn get_wave_data(&self) -> Vec<f64> {
        self.wave_data.clone()
    }

    /// C++ `SiOPMWaveSamplerData::get_channel_count`.
    pub fn get_channel_count(&self) -> i32 {
        self.channel_count
    }

    /// C++ `SiOPMWaveSamplerData::get_pan`.
    pub fn get_pan(&self) -> i32 {
        self.pan
    }

    /// C++ `SiOPMWaveSamplerData::get_length`.
    pub fn get_length(&self) -> i32 {
        if self.channel_count > 0 {
            // _wave_data.size() >> (_channel_count - 1)
            (self.wave_data.len() as i32) >> (self.channel_count - 1)
        } else {
            0
        }
    }

    /// C++ `SiOPMWaveSamplerData::is_ignoring_note_off`.
    pub fn is_ignoring_note_off(&self) -> bool {
        self.ignore_note_off
    }

    /// C++ `SiOPMWaveSamplerData::set_ignore_note_off`.
    pub fn set_ignore_note_off(&mut self, p_ignore: bool) {
        self.ignore_note_off = (self.loop_point == -1) && p_ignore;
    }

    /// C++ `SiOPMWaveSamplerData::get_start_point`.
    pub fn get_start_point(&self) -> i32 {
        self.start_point
    }

    /// C++ `SiOPMWaveSamplerData::get_end_point`.
    pub fn get_end_point(&self) -> i32 {
        self.end_point
    }

    /// C++ `SiOPMWaveSamplerData::get_loop_point`.
    pub fn get_loop_point(&self) -> i32 {
        self.loop_point
    }

    /// C++ `SiOPMWaveSamplerData::get_initial_sample_index(double p_phase = 0)`.
    pub fn get_initial_sample_index(&self, p_phase: f64) -> i32 {
        (self.start_point as f64 * (1.0 - p_phase) + self.end_point as f64 * p_phase) as i32
    }

    /// C++ `SiOPMWaveSamplerData::_prepare_wave_data`.
    fn prepare_wave_data(
        &mut self,
        p_data: &SampleData,
        p_src_channel_count: i32,
        p_channel_count: i32,
    ) {
        let mut source_channels = p_src_channel_count.clamp(1, 2);
        let mut target_channels = if p_channel_count == 0 {
            source_channels
        } else {
            p_channel_count.clamp(1, 2)
        };

        match p_data {
            SampleData::Float32Array(samples) => {
                let raw_data: Vec<f64> =
                    samples.borrow().iter().map(|v| *v as f64).collect();

                self.wave_data =
                    transform_sampler_data(&raw_data, source_channels, target_channels);
            }

            // Note: the C++ `p_data->wave.is_valid()` guard is a type-level
            // invariant here — `SampleData::Wave` always carries a payload.
            SampleData::Wave(wav) => {
                let raw_data = {
                    let mut cc = source_channels;
                    let raw = extract_wave_data(Some(wav), &mut cc);
                    source_channels = cc;
                    raw
                };
                if p_channel_count == 0 {
                    // Update if necessary.
                    target_channels = source_channels;
                }

                self.wave_data =
                    transform_sampler_data(&raw_data, source_channels, target_channels);
            }

            SampleData::Nil => {
                // Nothing to do.
            }

            // SampleData::INT32_ARRAY and unsupported types fall to default.
            _ => {
                crate::error::err_print_body(
                    "Method/function failed.\nSiOPMWaveSamplerData: Unsupported data type.",
                    false,
                );
            }
        }

        self.channel_count = target_channels;
        self.end_point = self.get_length();
    }

    /// C++ `SiOPMWaveSamplerData::_seek_head_silence`.
    fn seek_head_silence(&self) -> i32 {
        if self.wave_data.is_empty() {
            return 0;
        }

        // Note: Original code is pretty much broken here. The intent seems to be to keep track of the
        // last 22 sample points and check if their sum goes over a threshold. However, the order of
        // calls was wrong, and we ended up adding and immediately removing the value from the sum, and
        // constantly overriding our ring buffer without reusing its values.
        // This method has been adjusted to fix the code according to the assumed intent. But it's not
        // tested, and I can't say if the original idea behind the code is wrong somehow.

        let mut ms_window = RingWindow::new(22); // 0.5ms
        let mut i = 0i32;

        if self.channel_count == 1 {
            let mut ms = 0.0f64;

            while (i as usize) < self.wave_data.len() {
                ms -= ms_window.get();
                ms_window.set(self.wave_data[i as usize] * self.wave_data[i as usize]);
                ms += ms_window.get();

                ms_window.next();

                if ms > 0.0011 {
                    break;
                }
                i += 1;
            }
        } else {
            let mut ms = 0.0f64;

            while (i as usize) < self.wave_data.len() {
                ms -= ms_window.get();
                ms_window.set(
                    self.wave_data[i as usize] * self.wave_data[i as usize]
                        + self.wave_data[i as usize + 1] * self.wave_data[i as usize + 1],
                );
                ms += ms_window.get();

                ms_window.next();

                if ms > 0.0022 {
                    break;
                }
                i += 2;
            }

            i >>= 1;
        }

        i - 22
    }

    /// C++ `SiOPMWaveSamplerData::_seek_end_gap`.
    fn seek_end_gap(&self) -> i32 {
        if self.wave_data.is_empty() {
            return 0;
        }

        let mut i = self.wave_data.len() as i32 - 1;

        if self.channel_count == 1 {
            while i >= 0 {
                let ms = self.wave_data[i as usize] * self.wave_data[i as usize];

                if ms > 0.0001 {
                    break;
                }
                i -= 1;
            }
        } else {
            while i >= 0 {
                let mut ms = self.wave_data[i as usize] * self.wave_data[i as usize];
                ms += self.wave_data[i as usize - 1] * self.wave_data[i as usize - 1];

                if ms > 0.0002 {
                    break;
                }
                i -= 2;
            }

            i >>= 1;
        }

        // SUS: What is 1152? Should be extracted into a clearly named constant.
        i.max(self.get_length() - 1152)
    }

    /// C++ `SiOPMWaveSamplerData::_slice`.
    fn slice_internal(&mut self) {
        if self.start_point < 0 {
            self.start_point = self.seek_head_silence();
        }
        if self.loop_point < 0 {
            self.loop_point = -1;
        }
        if self.end_point < 0 {
            self.end_point = self.seek_end_gap();
        }

        if self.end_point < self.loop_point {
            self.loop_point = -1;
        }
        if self.end_point < self.start_point {
            self.end_point = self.get_length() - 1;
        }
    }

    /// C++ `SiOPMWaveSamplerData::slice(p_start_point = -1, p_end_point = -1, p_loop_point = -1)`.
    pub fn slice(&mut self, p_start_point: i32, p_end_point: i32, p_loop_point: i32) {
        self.start_point = p_start_point;
        self.end_point = p_end_point;
        self.loop_point = p_loop_point;

        self.slice_internal();
    }
}
