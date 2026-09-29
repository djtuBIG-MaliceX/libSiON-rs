//! Port of `effector/effects/si_effect_compressor.{h,cpp}`.
//!
//! RMS-window compressor. The C++ `SinglyLinkedList<double>` ring maps to a
//! `Vec<f64>` with a wrap-around cursor; `prepare_process` resets the cursor
//! to the head exactly like `SinglyLinkedList::reset()`.

use crate::effector::effect_base::{get_mml_arg, EffectBase, EffectSettings};
use crate::math::{self, clampf};

pub struct EffectCompressor {
    pub settings: EffectSettings,
    pub window_rms_list: Vec<f64>,
    pub window_cursor: usize,
    pub window_samples: i32,
    pub window_rms_total: f64,
    pub window_rms_averaging: f64,
    pub threshold_squared: f64,
    pub attack_rate: f64,
    pub release_rate: f64,
    pub max_gain: f64,
    pub mixing_level: f64,
    pub gain: f64,
}

impl EffectCompressor {
    pub fn new(
        p_threshold: f64,
        p_window_time: f64,
        p_attack_time: f64,
        p_release_time: f64,
        p_max_gain: f64,
        p_mixing_level: f64,
    ) -> Self {
        let mut effect = EffectCompressor {
            settings: EffectSettings::new(),
            window_rms_list: Vec::new(),
            window_cursor: 0,
            window_samples: 0,
            window_rms_total: 0.0,
            window_rms_averaging: 0.0,
            threshold_squared: 0.0,
            attack_rate: 0.0,
            release_rate: 0.0,
            max_gain: 0.0,
            mixing_level: 0.0,
            gain: 0.0,
        };
        effect.set_params(
            p_threshold,
            p_window_time,
            p_attack_time,
            p_release_time,
            p_max_gain,
            p_mixing_level,
        );
        effect
    }

    pub fn set_params(
        &mut self,
        p_threshold: f64,
        p_window_time: f64,
        p_attack_time: f64,
        p_release_time: f64,
        p_max_gain: f64,
        p_mixing_level: f64,
    ) {
        self.threshold_squared = p_threshold * p_threshold;

        self.window_samples = (p_window_time * 44.1) as i32;
        self.window_rms_averaging = 1.0 / self.window_samples as f64;
        self.window_rms_list = vec![0.0; self.window_samples as usize];

        self.attack_rate = 0.5;
        if p_attack_time != 0.0 {
            self.attack_rate = math::pow(2.0, -1.0 / (p_attack_time * 44.1));
        }

        self.release_rate = 2.0;
        if p_release_time != 0.0 {
            self.release_rate = math::pow(2.0, 1.0 / (p_release_time * 44.1));
        }

        self.max_gain = math::pow(2.0, -p_max_gain / 6.0);
        self.mixing_level = p_mixing_level;
    }
}

impl EffectBase for EffectCompressor {
    fn settings(&self) -> &EffectSettings {
        &self.settings
    }

    fn settings_mut(&mut self) -> &mut EffectSettings {
        &mut self.settings
    }

    fn prepare_process(&mut self) -> i32 {
        self.window_cursor = 0;
        self.window_rms_total = 0.0;
        self.gain = 2.0;
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
        let window_size = self.window_samples as usize;

        let mut i = start_index;
        while i < (start_index + length) {
            let index = i as usize;
            let value_left = r_buffer[index];
            let value_right = r_buffer[index + 1];

            self.window_cursor = (self.window_cursor + 1) % window_size;
            let cursor = self.window_cursor;
            self.window_rms_total -= self.window_rms_list[cursor];
            self.window_rms_list[cursor] = value_left * value_left + value_right * value_right;
            self.window_rms_total += self.window_rms_list[cursor];

            let rms_value = self.window_rms_total * self.window_rms_averaging;
            self.gain *= if rms_value > self.threshold_squared {
                self.attack_rate
            } else {
                self.release_rate
            };
            if self.gain > self.max_gain {
                self.gain = self.max_gain;
            }

            let mut value_left = clampf(value_left * self.gain, -1.0, 1.0);
            let mut value_right = clampf(value_right * self.gain, -1.0, 1.0);
            value_left *= self.mixing_level;
            value_right *= self.mixing_level;

            r_buffer[index] = value_left;
            r_buffer[index + 1] = value_right;
            i += 2;
        }

        p_channels
    }

    fn set_by_mml(&mut self, p_args: &[f64]) {
        let threshold = get_mml_arg(p_args, 0, 70.0) / 100.0;
        let window_time = get_mml_arg(p_args, 1, 50.0);
        let attack_time = get_mml_arg(p_args, 2, 20.0);
        let release_time = get_mml_arg(p_args, 3, 20.0);
        let max_gain = get_mml_arg(p_args, 4, -6.0);
        let mixing_level = get_mml_arg(p_args, 5, 50.0) / 100.0;
        self.set_params(threshold, window_time, attack_time, release_time, max_gain, mixing_level);
    }

    fn reset(&mut self) {
        self.set_params(0.7, 50.0, 20.0, 20.0, -6.0, 0.5);
    }
}

impl Default for EffectCompressor {
    fn default() -> Self {
        Self::new(0.7, 50.0, 20.0, 20.0, -6.0, 0.5)
    }
}
