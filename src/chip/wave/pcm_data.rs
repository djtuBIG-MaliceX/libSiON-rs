//! Port of `libSiON-cpp/src/chip/wave/siopm_wave_pcm_data.{h,cpp}`.
//!
//! Float/double evaluation order is preserved verbatim (DSP-frozen). The
//! crossfade path reads the `SiOPMRefTable` singleton; the thread_local
//! `instance()` is captured *before* borrowing `self` (never the reverse) to
//! avoid a re-entrant borrow panic.

use std::cell::RefCell;

use super::base::{extract_wave_data, transform_pcm_data, SiopmWaveBase};
use crate::chip::ref_table::{calculate_log_table_index, instance as ref_table_instance, SiopmRefTable};
use crate::sample_data::SampleData;
use crate::sion_enums::MODULE_PCM;

thread_local! {
    // C++ `static std::vector<double> SiOPMWavePCMData::_sin_table;` — a
    // process-global scratch table for the loop crossfade, only ever rebuilt when
    // its length differs from the requested size. Shared between all instances.
    static SIN_TABLE: RefCell<Vec<f64>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone, Debug)]
pub struct SiopmWavePcmData {
    base: SiopmWaveBase,

    // wavelet: Vec<i32> — log-table indices.
    wavelet: Vec<i32>,
    channel_count: i32,
    sampling_pitch: i32,

    // Wave positions in the sample count.
    start_point: i32,
    end_point: i32,
    // -1 means no looping.
    loop_point: i32,
}

impl SiopmWavePcmData {
    /// C++ `SiOPMWavePCMData::SiOPMWavePCMData(Ref<SampleData> p_data,
    /// int p_sampling_pitch = 4416, int p_src_channel_count = 2,
    /// int p_channel_count = 0)`.
    pub fn new(
        p_data: &SampleData,
        p_sampling_pitch: i32,
        p_src_channel_count: i32,
        p_channel_count: i32,
    ) -> Self {
        let mut data = SiopmWavePcmData {
            base: SiopmWaveBase::new(MODULE_PCM),
            wavelet: Vec::new(),
            channel_count: 0,
            sampling_pitch: 0,
            start_point: 0,
            end_point: 0,
            loop_point: -1,
        };
        data.prepare_wavelet(p_data, p_src_channel_count, p_channel_count);
        data.sampling_pitch = p_sampling_pitch;
        data
    }

    /// C++ `SiOPMWaveBase::get_module_type` (inherited).
    pub fn get_module_type(&self) -> i32 {
        self.base.get_module_type()
    }

    /// C++ `SiOPMWavePCMData::get_wavelet`.
    pub fn get_wavelet(&self) -> Vec<i32> {
        self.wavelet.clone()
    }

    /// C++ `SiOPMWavePCMData::get_channel_count`.
    pub fn get_channel_count(&self) -> i32 {
        self.channel_count
    }

    /// C++ `SiOPMWavePCMData::get_sampling_pitch`.
    pub fn get_sampling_pitch(&self) -> i32 {
        self.sampling_pitch
    }

    /// C++ `SiOPMWavePCMData::get_sample_count`.
    pub fn get_sample_count(&self) -> i32 {
        if self.channel_count > 0 {
            // _wavelet.size() >> (_channel_count - 1)
            (self.wavelet.len() as i32) >> (self.channel_count - 1)
        } else {
            0
        }
    }

    /// C++ `SiOPMWavePCMData::get_sampling_octave`.
    pub fn get_sampling_octave(&self) -> i32 {
        (self.sampling_pitch as f64 * 0.001272264631043257) as i32
    }

    /// C++ `SiOPMWavePCMData::get_start_point`.
    pub fn get_start_point(&self) -> i32 {
        self.start_point
    }

    /// C++ `SiOPMWavePCMData::get_end_point`.
    pub fn get_end_point(&self) -> i32 {
        self.end_point
    }

    /// C++ `SiOPMWavePCMData::get_loop_point`.
    pub fn get_loop_point(&self) -> i32 {
        self.loop_point
    }

    /// C++ `SiOPMWavePCMData::get_initial_sample_index(double p_phase = 0)`.
    pub fn get_initial_sample_index(&self, p_phase: f64) -> i32 {
        (self.start_point as f64 * (1.0 - p_phase) + self.end_point as f64 * p_phase) as i32
    }

    /// C++ `SiOPMWavePCMData::_prepare_wavelet`.
    fn prepare_wavelet(
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
            SampleData::Int32Array(samples) => {
                for value in samples.borrow().iter() {
                    self.wavelet.push(*value);
                }
            }
            SampleData::Float32Array(samples) => {
                let raw_data: Vec<f64> =
                    samples.borrow().iter().map(|v| *v as f64).collect();

                self.wavelet =
                    transform_pcm_data(&raw_data, source_channels, target_channels, true);
            }
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

                self.wavelet =
                    transform_pcm_data(&raw_data, source_channels, target_channels, true);
            }
            SampleData::Nil => {
                // Nothing to do.
            }
        }

        self.channel_count = target_channels;
        self.end_point = self.get_sample_count() - 1;
    }

    /// C++ `SiOPMWavePCMData::_seek_head_silence`.
    fn seek_head_silence(&self) -> i32 {
        let threshold = SiopmRefTable::LOG_TABLE_BOTTOM
            - SiopmRefTable::LOG_TABLE_RESOLUTION * 14; // 1/128

        let mut i = 0i32;
        while (i as usize) < self.wavelet.len() {
            if self.wavelet[i as usize] < threshold {
                break;
            }
            i += 1;
        }

        i >> (self.channel_count - 1)
    }

    /// C++ `SiOPMWavePCMData::_seek_end_gap`.
    fn seek_end_gap(&self) -> i32 {
        let threshold = SiopmRefTable::LOG_TABLE_BOTTOM
            - SiopmRefTable::LOG_TABLE_RESOLUTION * 2; // 1/4096

        let mut i = self.wavelet.len() as i32 - 1;
        while i > 0 {
            if self.wavelet[i as usize] < threshold {
                break;
            }
            i -= 1;
        }

        (i >> (self.channel_count - 1)) - 100 // 100 = 1 cycle margin
    }

    /// C++ `SiOPMWavePCMData::_slice`.
    fn slice_internal(&mut self) {
        if self.start_point < 0 {
            self.start_point = self.seek_head_silence();
        }

        if self.loop_point < -1 {
            // Set to loop infinitely.
            if self.end_point >= 0 {
                self.loop_tail_samples(-self.loop_point, 0, true);
                if self.start_point >= self.end_point {
                    self.end_point = self.start_point;
                }
            } else {
                self.loop_tail_samples(-self.loop_point, -self.end_point, true);
            }
        } else {
            let wavelet_length = self.get_sample_count();
            if self.end_point < 0 {
                self.end_point = self.seek_end_gap() + self.end_point;
            } else if self.end_point < self.start_point {
                self.end_point = self.start_point;
            } else if self.end_point > wavelet_length {
                self.end_point = wavelet_length - 1;
            }

            if self.loop_point != -1 && self.loop_point < self.start_point {
                self.loop_point = self.start_point;
            } else if self.loop_point > self.end_point {
                self.loop_point = -1;
            }
        }
    }

    /// C++ `SiOPMWavePCMData::slice(p_start_point = -1, p_end_point = -1, p_loop_point = -1)`.
    pub fn slice(&mut self, p_start_point: i32, p_end_point: i32, p_loop_point: i32) {
        self.start_point = p_start_point;
        self.end_point = p_end_point;
        self.loop_point = p_loop_point;

        self.slice_internal();
    }

    /// C++ `SiOPMWavePCMData::loop_tail_samples(p_sample_count = 2205, p_tail_margin = 0, p_crossfade = true)`.
    // `approx_constant`: DSP-frozen literals stay verbatim (CONVENTIONS.md).
    #[allow(clippy::approx_constant)]
    pub fn loop_tail_samples(
        &mut self,
        p_sample_count: i32,
        p_tail_margin: i32,
        p_crossfade: bool,
    ) {
        self.end_point = self.seek_end_gap() - p_tail_margin;

        if self.end_point < (self.start_point + p_sample_count) {
            if self.end_point < self.start_point {
                self.end_point = self.start_point;
            }
            self.loop_point = self.start_point;
            return;
        }

        self.loop_point = self.end_point - p_sample_count;

        if p_crossfade && self.loop_point > (self.start_point + p_sample_count) {
            let max_idx = p_sample_count << (self.channel_count - 1);
            let delta_sin = 1.5707963267948965 / max_idx as f64;

            SIN_TABLE.with(|sin_table| {
                let mut sin_table = sin_table.borrow_mut();
                if sin_table.len() != max_idx as usize {
                    sin_table.resize(max_idx as usize, 0.0); // TODO zeroed

                    let mut sin_value = 0.0f64;
                    for i in 0..max_idx as usize {
                        sin_table[i] = crate::math::sin(sin_value);
                        sin_value += delta_sin;
                    }
                }

                let offset = self.loop_point << (self.channel_count - 1);
                let envelope_top = (-SiopmRefTable::ENV_TOP) << 3;
                let i2n = 1.0 / ((1 << SiopmRefTable::LOG_VOLUME_BITS) as f64);

                // Snapshot the singleton ref first, then release the
                // RefCell borrow before touching `self` (C++ held the raw
                // `log_table` reference across the loop).
                let table = ref_table_instance();
                let table_borrow = table.borrow();
                let log_table = &table_borrow.log_table;

                for i in 0..max_idx as usize {
                    let idx0 = (offset as usize) + i;
                    let idx1 = idx0 - max_idx as usize;
                    let val0 = self.wavelet[idx0] + envelope_top;
                    let val1 = self.wavelet[idx1] + envelope_top;

                    let j = max_idx as usize - 1 - i;
                    self.wavelet[idx0] = calculate_log_table_index(
                        (log_table[val0 as usize] as f64 * sin_table[j]
                            + log_table[val1 as usize] as f64 * sin_table[i])
                            * i2n,
                    );
                }
            });
        }
    }
}
