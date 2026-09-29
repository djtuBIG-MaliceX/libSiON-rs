//! Port of `libSiON-cpp/src/utils/transformer_util.{h,cpp}`.
//!
//! All wave-data transformation entry points (`transform_pcm_data`,
//! [`transform_sampler_data`], [`wave_color_to_vector`]) live here. The
//! `transform_pcm_data` / `transform_sampler_data` implementations and the
//! [`RingWindow`] helper were relocated here from `chip/wave/base.rs`, which
//! now only re-exports them for the wave-data classes (C++ ownership was
//! always this file; see `docs/PENDING.md`).
//!
//! The C++ class is a pure-statics namespace, so this module exposes free
//! functions; the `_amplify_log_data` private static becomes
//! [`amplify_log_data`].

use crate::chip::ref_table::{self, SiopmRefTable};

/// Ring buffer of fixed size with a moving cursor, matching the behavior of
/// `SinglyLinkedList<double>(p_size, 0.0, true)` (ring mode) as used by
/// `SiOPMWaveSamplerData::_seek_head_silence`: the list is filled with
/// `p_size` zero elements, the cursor starts at the first element, and
/// `next()` wraps around the ring.
pub struct RingWindow {
    buf: Vec<f64>,
    pos: usize,
}

impl RingWindow {
    pub fn new(p_size: usize) -> Self {
        RingWindow {
            buf: vec![0.0; p_size],
            pos: 0,
        }
    }

    /// `SinglyLinkedList::get()->value` for the current cursor element.
    pub fn get(&self) -> f64 {
        self.buf[self.pos]
    }

    /// `SinglyLinkedList::get()->value = p_value`.
    pub fn set(&mut self, p_value: f64) {
        self.buf[self.pos] = p_value;
    }

    /// `SinglyLinkedList::next()` on a ring list (always wraps, never null).
    pub fn next(&mut self) {
        self.pos = (self.pos + 1) % self.buf.len();
    }
}

/// C++ `TransformerUtil::_amplify_log_data`.
fn amplify_log_data(r_src: &mut [i32], p_gain: i32) {
    let gain = p_gain & !1;

    for v in r_src.iter_mut() {
        *v -= gain;
    }
}

/// C++ `TransformerUtil::transform_pcm_data`.
/// `calculate_log_table_index` is the pure static
/// `SiOPMRefTable::calculate_log_table_index` (no instance access).
///
/// In the mono->stereo branch the C++ reads `result[j]` after `j += 2`, so
/// every in-range check observes the zero from `resize` (collapsing
/// `max_gain` to 0 and disabling maximization whenever the branch runs more
/// than once) and the final iteration reads one element past the end
/// (undefined behavior). This port keeps the zero-slot reads byte-identical
/// and simply omits the out-of-bounds last read.
pub fn transform_pcm_data(
    p_source: &[f64],
    p_src_channel_count: i32,
    p_channel_count: i32,
    p_maximize: bool,
) -> Vec<i32> {
    let mut result: Vec<i32>;
    let mut max_gain = SiopmRefTable::LOG_TABLE_BOTTOM;

    if p_src_channel_count == p_channel_count || p_channel_count == 0 {
        let target_size = p_source.len();
        result = vec![0; target_size];

        for i in 0..target_size {
            result[i] = ref_table::calculate_log_table_index(p_source[i]);

            if result[i] < max_gain {
                max_gain = result[i];
            }
        }
    } else if p_src_channel_count == 2 {
        // p_channel_count == 1
        let target_size = p_source.len() >> 1;
        result = vec![0; target_size];

        let mut j = 0usize;
        for i in 0..target_size {
            let mut value = p_source[j];
            j += 1;
            value += p_source[j];
            j += 1;

            result[i] = ref_table::calculate_log_table_index(value * 0.5);

            if result[i] < max_gain {
                max_gain = result[i];
            }
        }
    } else {
        // p_src_channel_count == 1, p_channel_count == 2
        let target_size = p_source.len();
        result = vec![0; target_size << 1];

        let mut j = 0usize;
        for i in 0..target_size {
            result[j] = ref_table::calculate_log_table_index(p_source[i]);
            result[j + 1] = result[j];
            j += 2;

            if j < result.len() && result[j] < max_gain {
                max_gain = result[j];
            }
        }
    }

    if p_maximize && max_gain > 1 {
        amplify_log_data(&mut result, max_gain);
    }
    result
}

/// C++ `TransformerUtil::transform_sampler_data`.
pub fn transform_sampler_data(
    p_source: &[f64],
    p_src_channel_count: i32,
    p_channel_count: i32,
) -> Vec<f64> {
    if p_src_channel_count == p_channel_count || p_channel_count == 0 {
        return p_source.to_vec();
    } else if p_src_channel_count == 2 {
        // p_channel_count == 1
        let target_size = p_source.len() >> 1;
        let mut result = vec![0.0; target_size];

        let mut i = 0usize;
        let mut j = 0usize;
        while i < target_size {
            result[i] = (p_source[j] + p_source[j + 1]) * 0.5;
            i += 1;
            j += 2;
        }
        result
    } else {
        // p_src_channel_count == 1, p_channel_count == 2
        let target_size = p_source.len();
        let mut result = vec![0.0; target_size << 1];

        let mut i = 0usize;
        let mut j = 0usize;
        while i < target_size {
            result[j] = p_source[i];
            result[j + 1] = p_source[i];
            i += 1;
            j += 2;
        }
        result
    }
}

/// C++ `TransformerUtil::wave_color_to_vector`.
pub fn wave_color_to_vector(p_color: u32, p_wave_type: i32) -> Vec<f64> {
    let mut result = Vec::new();

    let mut bits = 0i32;
    let mut length = (SiopmRefTable::SAMPLING_TABLE_SIZE >> 1) as i32;
    while length != 0 {
        bits += 1;
        length >>= 1;
    }
    result.resize(1 << bits, 0.0); // TODO zeroed

    let mut bars = [0.0f64; 7];
    let mut color = p_color;
    for i in 0..7 {
        bars[i] = (color & 15) as f64 * 0.0625;
        color >>= 4;
    }

    let barr = [1, 2, 3, 4, 5, 6, 8];
    let table = ref_table::instance();
    let wave_table = table
        .borrow()
        .get_wave_table(p_wave_type + (color >> 28) as i32)
        .expect("TransformerUtil: wave table must exist");
    let wave_table = wave_table.borrow();
    let envelope_top = (-SiopmRefTable::ENV_TOP) << 3;

    let table_borrow = table.borrow();
    let log_table = &table_borrow.log_table;

    let mut value_max = 0.0f64;

    bits = SiopmRefTable::PHASE_BITS - bits;
    let step = SiopmRefTable::PHASE_MAX >> bits;
    let mut i = 0i32;
    while i < SiopmRefTable::PHASE_MAX {
        let j = (i >> bits) as usize;

        result[j] = 0.0;
        for mult_idx in 0..7 {
            let gain_idx =
                (i.wrapping_mul(barr[mult_idx]) & SiopmRefTable::PHASE_FILTER)
                    >> wave_table.get_fixed_bits();
            let gain = wave_table.wavelet_slice()[gain_idx as usize] + envelope_top;

            result[j] += log_table[gain as usize] as f64 * bars[mult_idx];
        }

        let value = result[j].abs();
        if value_max < value {
            value_max = value;
        }

        i += step;
    }

    value_max = value_max.max(8192.0);
    let value_coef = 1.0 / value_max;
    for v in result.iter_mut() {
        *v *= value_coef;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chip::wave::base::extract_wave_data;
    use crate::chip::wave::pcm_data::SiopmWavePcmData;
    use crate::sample_data::{AudioStreamWav, SampleData, WavFormat};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn sampler_downmix_and_upmix() {
        // stereo -> mono: (L + R) * 0.5 per frame.
        let stereo = [0.0, 1.0, 2.0, 3.0];
        assert_eq!(transform_sampler_data(&stereo, 2, 1), vec![0.5, 2.5]);

        // mono -> stereo: duplicate each sample.
        let mono = [1.0, -2.0];
        assert_eq!(transform_sampler_data(&mono, 1, 2), vec![1.0, 1.0, -2.0, -2.0]);

        // Same counts pass through.
        let same = [0.25, -0.5, 0.75];
        assert_eq!(transform_sampler_data(&same, 2, 2), same.to_vec());

        // p_channel_count == 0 also passes through.
        assert_eq!(transform_sampler_data(&stereo, 2, 0), stereo.to_vec());
    }

    #[test]
    fn pcm_transform_maximize_shares_the_log_table() {
        // calculate_log_table_index(1.0) == 2 -> maximize subtracts (2 & ~1) = 2.
        let mono = transform_pcm_data(&[1.0], 1, 1, true);
        assert_eq!(mono, vec![0]);

        let raw = transform_pcm_data(&[1.0], 1, 1, false);
        assert_eq!(raw, vec![2]);

        // 1 -> 2 channel: interleaved duplicate, maximized like the loop above.
        let dual = transform_pcm_data(&[1.0, 0.5], 1, 2, false);
        assert_eq!(dual.len(), 4);
        assert_eq!(dual[0], dual[1]);
        assert_eq!(dual[2], dual[3]);

        // 2 -> 1 channel downmix: value * 0.5 feeds the log index directly.
        let down = transform_pcm_data(&[1.0, 1.0, -1.0, -1.0], 2, 1, false);
        assert_eq!(down.len(), 2);
        assert_eq!(down[0], ref_table::calculate_log_table_index(1.0));
        assert_eq!(down[1], ref_table::calculate_log_table_index(-1.0));
    }

    #[test]
    fn wave_color_vector_is_normalized_full_scale() {
        let wave = wave_color_to_vector(0x0123_4567, 0);
        assert_eq!(wave.len(), SiopmRefTable::SAMPLING_TABLE_SIZE);

        let max_abs = wave.iter().fold(0.0f64, |acc, v| acc.max(v.abs()));
        assert!(max_abs <= 1.0 + 1e-12);
        assert!(
            wave.iter().any(|v| v.abs() >= 1.0 - 1e-12),
            "peak sample must hit full scale after normalization"
        );
    }

    #[test]
    fn pcm_maximize_is_disabled_by_the_oob_zero_quirk() {
        // C++ checks result[j] AFTER j += 2: the in-range reads hit the
        // zeroed resize slots, so max_gain collapses to 0 and the maximize
        // pass never runs in this branch (two or more source frames).
        let up = transform_pcm_data(&[0.5, 1.0], 1, 2, true);
        assert_eq!(
            up,
            vec![
                ref_table::calculate_log_table_index(0.5),
                ref_table::calculate_log_table_index(0.5),
                ref_table::calculate_log_table_index(1.0),
                ref_table::calculate_log_table_index(1.0),
            ]
        );
    }

    #[test]
    fn synthetic_pcm16_wav_end_to_end() {
        // Four-ish frames of mono 16-bit little-endian PCM with a loop set.
        let samples: [i16; 6] = [0, 16384, -16384, 32767, 0, 0];
        let mut data: Vec<u8> = Vec::new();
        for sample in samples {
            data.extend_from_slice(&sample.to_le_bytes());
        }

        let wav = Rc::new(RefCell::new(AudioStreamWav {
            format: WavFormat::Pcm16,
            data,
            stereo: false,
            loop_start: 1,
            loop_end: 4,
        }));
        let source = SampleData::Wave(wav.clone());
        assert!(source.is_valid());

        // 16-bit mono: two bytes per frame, one channel reported.
        let borrowed = wav.borrow();
        assert_eq!(borrowed.data.len(), samples.len() * 2);
        assert_eq!(borrowed.loop_start * 2, 2);
        assert_eq!(borrowed.loop_end * 2, 8);
        drop(borrowed);

        let mut channel_count = 1i32;
        let raw = extract_wave_data(Some(&wav), &mut channel_count);
        assert_eq!(channel_count, 1);
        assert_eq!(
            raw,
            vec![
                0.0,
                16384f32 as f64 / 32767.0,
                -16384f32 as f64 / 32767.0,
                1.0,
                0.0,
                0.0,
            ]
        );

        // Hand-computed log indices (LOG_TABLE_BOTTOM = 6656, 1 octave =
        // 512 steps, full scale = 2, silence = bottom) after maximize
        // subtracts max_gain = index(1.0) = 2:
        let pcm = SiopmWavePcmData::new(&source, 60 << 6, 1, 0);
        assert_eq!(pcm.get_channel_count(), 1);
        assert_eq!(pcm.get_wavelet(), vec![6654, 512, 513, 0, 6654, 6654]);

        // Loop points set explicitly survive slice().
        let mut pcm = SiopmWavePcmData::new(&source, 60 << 6, 1, 0);
        pcm.slice(1, 4, 2);
        assert_eq!(pcm.get_start_point(), 1);
        assert_eq!(pcm.get_end_point(), 4);
        assert_eq!(pcm.get_loop_point(), 2);
    }
}
