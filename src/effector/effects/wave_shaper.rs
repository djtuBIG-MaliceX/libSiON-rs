//! Port of `effector/effects/si_effect_wave_shaper.{h,cpp}`.
//!
//! tanh-like algebraic soft clip. Quirk kept: `_coefficient` is an `int` in
//! C++, so `2*d/(1-d)` is truncated.

use crate::effector::effect_base::{get_mml_arg, EffectBase, EffectSettings};

pub struct EffectWaveShaper {
    pub settings: EffectSettings,
    pub coefficient: i32,
    pub output_level: f64,
}

impl EffectWaveShaper {
    pub fn new(p_distortion: f64, p_output_level: f64) -> Self {
        let mut effect = EffectWaveShaper {
            settings: EffectSettings::new(),
            coefficient: 0,
            output_level: 0.0,
        };
        effect.set_params(p_distortion, p_output_level);
        effect
    }

    pub fn set_params(&mut self, p_distortion: f64, p_output_level: f64) {
        let mut distortion = p_distortion;
        if distortion >= 1.0 {
            distortion = 0.9999847412109375;
        }

        self.coefficient = (2.0 * distortion / (1.0 - distortion)) as i32;
        self.output_level = p_output_level;
    }
}

impl EffectBase for EffectWaveShaper {
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
        p_channels: i32,
        r_buffer: &mut [f64],
        p_start_index: i32,
        p_length: i32,
    ) -> i32 {
        let start_index = p_start_index << 1;
        let length = p_length << 1;

        let coefficient = self.coefficient as f64;
        let coef = (1.0 + coefficient) * self.output_level;

        if p_channels == 2 {
            let mut i = start_index;
            while i < (start_index + length) {
                let index = i as usize;
                let mut value = r_buffer[index];
                value = coef * value / (1.0 + coefficient * value.abs());
                r_buffer[index] = value;
                i += 1;
            }
        } else {
            let mut i = start_index;
            while i < (start_index + length) {
                let index = i as usize;
                let mut value = r_buffer[index];
                value = coef * value / (1.0 + coefficient * value.abs());
                r_buffer[index] = value;
                r_buffer[index + 1] = value;
                i += 2;
            }
        }

        p_channels
    }

    fn set_by_mml(&mut self, p_args: &[f64]) {
        let distortion = get_mml_arg(p_args, 0, 50.0) / 100.0;
        let output_level = get_mml_arg(p_args, 1, 100.0) / 100.0;
        self.set_params(distortion, output_level);
    }

    fn reset(&mut self) {
        self.set_params(0.5, 1.0);
    }
}

impl Default for EffectWaveShaper {
    fn default() -> Self {
        Self::new(0.5, 1.0)
    }
}
