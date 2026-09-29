//! Port of `effector/effects/si_effect_autopan.{h,cpp}`.
//!
//! Sine panning oscillator over a 256-entry ring table (the C++
//! `SinglyLinkedList<double>(256, 0.0, true)` looped list maps to a `Vec`
//! plus a wrap-around cursor). Quirk kept: the LFO cursor advances ONCE per
//! `process_lfo` call (per chunk), not per sample, and `set_params` fills
//! the table from the current cursor position — a re-param mid-stream keeps
//! the cursor where `process` left it.

use crate::effector::effect_base::{get_mml_arg, EffectBase, EffectSettings};
use crate::math;

const BUFFER_SIZE: usize = 256;

pub struct EffectAutopan {
    pub settings: EffectSettings,
    pub stereo: bool,
    pub lfo_step: i32,
    pub lfo_residue_step: i32,
    pub p_left: Vec<f64>,
    pub p_right: Vec<f64>,
    pub cursor_left: usize,
    pub cursor_right: usize,
}

impl EffectAutopan {
    pub fn new(p_frequency: f64, p_stereo_width: f64) -> Self {
        let mut effect = EffectAutopan {
            settings: EffectSettings::new(),
            stereo: false,
            lfo_step: 0,
            lfo_residue_step: 0,
            p_left: vec![0.0; BUFFER_SIZE],
            p_right: vec![0.0; BUFFER_SIZE],
            cursor_left: 0,
            cursor_right: 0,
        };
        effect.set_params(p_frequency, p_stereo_width);
        effect
    }

    fn next_left(&mut self) {
        self.cursor_left = (self.cursor_left + 1) % BUFFER_SIZE;
    }

    fn next_right(&mut self) {
        self.cursor_right = (self.cursor_right + 1) % BUFFER_SIZE;
    }

    pub fn set_params(&mut self, p_frequency: f64, p_stereo_width: f64) {
        self.lfo_step = (172.265625 / (p_frequency * 0.5)) as i32;
        if self.lfo_step <= 4 {
            self.lfo_step = 4;
        }

        let mut width = p_stereo_width;
        if width == 0.0 {
            width = 1.0;
            self.stereo = true;
        }
        width *= 0.01227184630308513;

        for i in -128..128 {
            let value = math::sin(1.5707963267948965 + (i as f64) * width);
            self.p_left[self.cursor_left] = value;
            self.next_left();
            self.p_right[self.cursor_right] = value;
            self.next_right();
        }

        for _ in 0..(BUFFER_SIZE >> 1) {
            self.next_right();
        }
    }

    fn process_lfo_mono(&mut self, r_buffer: &mut [f64], p_start_index: i32, p_length: i32) {
        let left = self.p_left[self.cursor_left];
        let right = self.p_right[self.cursor_right];
        let mut i = p_start_index;
        while i < (p_start_index + p_length) {
            let index = i as usize;
            let value = r_buffer[index];
            r_buffer[index] = value * left;
            r_buffer[index + 1] = value * right;
            i += 2;
        }

        self.next_left();
        self.next_right();
    }

    fn process_lfo_stereo(&mut self, r_buffer: &mut [f64], p_start_index: i32, p_length: i32) {
        let left = self.p_left[self.cursor_left];
        let right = self.p_right[self.cursor_right];
        let mut i = p_start_index;
        while i < (p_start_index + p_length) {
            let index = i as usize;
            let value_left = r_buffer[index];
            let value_right = r_buffer[index + 1];
            r_buffer[index] = value_left * left - value_right * right;
            r_buffer[index + 1] = value_left * right + value_right * left;
            i += 2;
        }

        self.next_left();
        self.next_right();
    }
}

impl EffectBase for EffectAutopan {
    fn settings(&self) -> &EffectSettings {
        &self.settings
    }

    fn settings_mut(&mut self) -> &mut EffectSettings {
        &mut self.settings
    }

    fn prepare_process(&mut self) -> i32 {
        if self.stereo {
            2
        } else {
            1
        }
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

        let mut step = self.lfo_residue_step;
        let max = start_index + length;
        let mut i = start_index;
        while i < (max - step) {
            if self.stereo {
                self.process_lfo_stereo(r_buffer, i, step);
            } else {
                self.process_lfo_mono(r_buffer, i, step);
            }
            i += step;
            step = self.lfo_step << 1;
        }

        if self.stereo {
            self.process_lfo_stereo(r_buffer, i, max - i);
        } else {
            self.process_lfo_mono(r_buffer, i, max - i);
        }

        self.lfo_residue_step = step - (max - i);

        2
    }

    fn set_by_mml(&mut self, p_args: &[f64]) {
        let frequency = get_mml_arg(p_args, 0, 1.0);
        let stereo_width = get_mml_arg(p_args, 1, 100.0) / 100.0;
        self.set_params(frequency, stereo_width);
    }

    fn reset(&mut self) {
        self.lfo_residue_step = 0;
        self.set_params(1.0, 1.0);
    }
}

impl Default for EffectAutopan {
    fn default() -> Self {
        Self::new(1.0, 1.0)
    }
}
