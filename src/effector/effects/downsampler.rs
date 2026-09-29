//! Port of `effector/effects/si_effect_downsampler.{h,cpp}`.
//!
//! Sample-and-hold decimator with int quantization: averages
//! `sample_size` consecutive samples, truncates `(sum * bit_conv0 /
//! sample_size)` to `i32` (C++ `(int)` cast), rescales by `1/bit_conv0` and
//! holds the value across the block.

use crate::effector::effect_base::{get_mml_arg, EffectBase, EffectSettings};

pub struct EffectDownsampler {
    pub settings: EffectSettings,
    pub frequency_shift: i32,
    pub bit_conv0: f64,
    pub bit_conv1: f64,
    pub channel_count: i32,
}

impl EffectDownsampler {
    pub fn new(p_frequency_shift: i32, p_bitrate: i32, p_channel_count: i32) -> Self {
        let mut effect = EffectDownsampler {
            settings: EffectSettings::new(),
            frequency_shift: 0,
            bit_conv0: 1.0,
            bit_conv1: 1.0,
            channel_count: 2,
        };
        effect.set_params(p_frequency_shift, p_bitrate, p_channel_count);
        effect
    }

    pub fn set_params(&mut self, p_frequency_shift: i32, p_bitrate: i32, p_channel_count: i32) {
        self.frequency_shift = p_frequency_shift;
        self.bit_conv0 = (1i32 << p_bitrate) as f64;
        self.bit_conv1 = 1.0 / self.bit_conv0;
        self.channel_count = p_channel_count;
    }

    fn process_mono(&mut self, r_buffer: &mut [f64], p_start_index: i32, p_length: i32) {
        let sample_size = 2 << self.frequency_shift;
        let bc0 = self.bit_conv0 / sample_size as f64;

        let mut i = p_start_index;
        while i < (p_start_index + p_length) {
            let mut value = 0.0;
            for j in 0..sample_size {
                value += r_buffer[(i + j) as usize];
            }
            let value = (value * bc0) as i32 as f64 * self.bit_conv1;
            for j in 0..sample_size {
                r_buffer[(i + j) as usize] = value;
            }
            i += sample_size;
        }
    }

    fn process_stereo(&mut self, r_buffer: &mut [f64], p_start_index: i32, p_length: i32) {
        let sample_size = 1 << self.frequency_shift;
        let bc0 = self.bit_conv0 / sample_size as f64;

        let mut i = p_start_index;
        while i < (p_start_index + p_length) {
            let mut value_left = 0.0;
            let mut value_right = 0.0;
            for j in 0..sample_size {
                value_left += r_buffer[(i + 2 * j) as usize];
                value_right += r_buffer[(i + 2 * j + 1) as usize];
            }
            let value_left = (value_left * bc0) as i32 as f64 * self.bit_conv1;
            let value_right = (value_right * bc0) as i32 as f64 * self.bit_conv1;
            for j in 0..sample_size {
                r_buffer[(i + 2 * j) as usize] = value_left;
                r_buffer[(i + 2 * j + 1) as usize] = value_right;
            }
            i += 2 * sample_size;
        }
    }
}

impl EffectBase for EffectDownsampler {
    fn settings(&self) -> &EffectSettings {
        &self.settings
    }

    fn settings_mut(&mut self) -> &mut EffectSettings {
        &mut self.settings
    }

    fn prepare_process(&mut self) -> i32 {
        2
    }

    fn process(
        &mut self,
        _p_channels: i32,
        r_buffer: &mut [f64],
        p_start_index: i32,
        p_length: i32,
    ) -> i32 {
        let start_index = p_start_index << 1;
        let length = p_length << 1;

        if self.channel_count == 1 {
            self.process_mono(r_buffer, start_index, length);
        } else {
            self.process_stereo(r_buffer, start_index, length);
        }

        self.channel_count
    }

    fn set_by_mml(&mut self, p_args: &[f64]) {
        let frequency_shift = get_mml_arg(p_args, 0, 0.0) as i32;
        let bitrate = get_mml_arg(p_args, 1, 16.0) as i32;
        let channel_count = get_mml_arg(p_args, 2, 2.0) as i32;
        self.set_params(frequency_shift, bitrate, channel_count);
    }

    fn reset(&mut self) {
        self.set_params(0, 16, 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effector::effect_base::EffectBase;

    #[test]
    fn stereo_hold_maps_exact_sample_blocks() {
        let mut effect = EffectDownsampler::new(1, 16, 2);
        assert_eq!(effect.prepare_process(), 2);

        let mut buffer = vec![0.0, 0.0, 1.0, 1.0, 2.0, 2.0, 3.0, 3.0];
        assert_eq!(effect.process(2, &mut buffer, 0, 4), 2);

        assert_eq!(buffer, vec![0.5, 0.5, 0.5, 0.5, 2.5, 2.5, 2.5, 2.5]);
    }

    #[test]
    fn shift_two_stereo_collapses_whole_ramp() {
        let mut effect = EffectDownsampler::new(2, 16, 2);
        effect.prepare_process();

        let mut buffer = vec![0.0, 0.0, 1.0, 1.0, 2.0, 2.0, 3.0, 3.0];
        effect.process(2, &mut buffer, 0, 4);
        assert_eq!(buffer, vec![1.5, 1.5, 1.5, 1.5, 1.5, 1.5, 1.5, 1.5]);
    }

    #[test]
    fn mono_path_averages_interleaved_pairs_into_both_channels() {
        let mut effect = EffectDownsampler::new(1, 16, 1);
        effect.prepare_process();

        let mut buffer = vec![2.0, 2.0, 4.0, 4.0];
        assert_eq!(effect.process(1, &mut buffer, 0, 2), 1);
        assert_eq!(buffer, vec![3.0, 3.0, 3.0, 3.0]);
    }

    #[test]
    fn grid_values_pass_through_unchanged_at_shift_zero() {
        let mut effect = EffectDownsampler::new(0, 16, 2);
        effect.prepare_process();

        let mut buffer: Vec<f64> = Vec::new();
        for k in 0..4i32 {
            let v = (k * 1024) as f64 / 65536.0;
            buffer.extend([v, v]);
        }
        let expected = buffer.clone();
        effect.process(2, &mut buffer, 0, 4);
        assert_eq!(buffer, expected);
    }
}

impl Default for EffectDownsampler {
    fn default() -> Self {
        Self::new(0, 16, 2)
    }
}
