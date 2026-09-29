//! Port of `effector/effects/si_effect_stereo_chorus.{h,cpp}`.
//!
//! Dual modulated delay lines with a sine phase table. The LFO phase
//! advances once per chunk (`process_lfo` call), matching the C++ loop.

use crate::effector::effect_base::{get_mml_arg, EffectBase, EffectSettings};
use crate::math;

const DELAY_BUFFER_BITS: i32 = 12;
const DELAY_BUFFER_FILTER: i32 = (1 << DELAY_BUFFER_BITS) - 1;

pub struct EffectStereoChorus {
    pub settings: EffectSettings,
    pub delay_buffer_left: Vec<f64>,
    pub delay_buffer_right: Vec<f64>,
    pub pointer_read: i32,
    pub pointer_write: i32,
    pub feedback: f64,
    pub depth: f64,
    pub wet: f64,
    pub lfo_phase: usize,
    pub lfo_step: i32,
    pub lfo_residue_step: i32,
    pub phase_invert: i32,
    pub phase_table: Vec<i32>,
}

impl EffectStereoChorus {
    pub fn new(
        p_delay_time: f64,
        p_feedback: f64,
        p_frequency: f64,
        p_depth: f64,
        p_wet: f64,
        p_invert_phase: bool,
    ) -> Self {
        let mut effect = EffectStereoChorus {
            settings: EffectSettings::new(),
            delay_buffer_left: vec![0.0; (1 << DELAY_BUFFER_BITS) as usize],
            delay_buffer_right: vec![0.0; (1 << DELAY_BUFFER_BITS) as usize],
            pointer_read: 0,
            pointer_write: 0,
            feedback: 0.0,
            depth: 0.0,
            wet: 0.0,
            lfo_phase: 0,
            lfo_step: 0,
            lfo_residue_step: 0,
            phase_invert: 0,
            phase_table: Vec::new(),
        };
        effect.set_params(p_delay_time, p_feedback, p_frequency, p_depth, p_wet, p_invert_phase);
        effect
    }

    pub fn set_params(
        &mut self,
        p_delay_time: f64,
        p_feedback: f64,
        p_frequency: f64,
        p_depth: f64,
        p_wet: f64,
        p_invert_phase: bool,
    ) {
        crate::err_fail_cond_msg!(
            p_delay_time == 0.0,
            "p_delay_time == 0",
            "SiEffectStereoChorus: Delay cannot be zero."
        );
        crate::err_fail_cond_msg!(
            p_frequency == 0.0,
            "p_frequency == 0",
            "SiEffectStereoChorus: Frequency cannot be zero."
        );
        crate::err_fail_cond_msg!(
            p_depth == 0.0,
            "p_depth == 0",
            "SiEffectStereoChorus: Depth cannot be zero."
        );

        let mut offset = (p_delay_time * 44.1) as i32;
        if offset > DELAY_BUFFER_FILTER {
            offset = DELAY_BUFFER_FILTER;
        }

        self.pointer_write = (self.pointer_read + offset) & DELAY_BUFFER_FILTER;
        self.depth = if ((offset - 4) as f64) < p_depth {
            (offset - 4) as f64
        } else {
            p_depth
        };

        self.feedback = p_feedback;
        if self.feedback >= 1.0 {
            self.feedback = 0.9990234375;
        } else if self.feedback <= -1.0 {
            self.feedback = -0.9990234375;
        }

        let mut table_size = (self.depth * 6.283185307179586) as i32;
        if (table_size as f64 * p_frequency) > 11025.0 {
            table_size = (11025.0 / p_frequency) as i32;
        }
        self.phase_table.resize(table_size as usize, 0);

        if self.lfo_phase >= self.phase_table.len() {
            self.lfo_phase = 0;
        }

        let depth_step = 6.283185307179586 / table_size as f64;
        let mut depth_value = 0.0;
        for i in 0..table_size {
            self.phase_table[i as usize] =
                (math::sin(depth_value) * self.depth + 0.5) as i32;
            depth_value += depth_step;
        }

        self.lfo_step = (44100.0 / (table_size as f64 * p_frequency)) as i32;
        if self.lfo_step < 4 {
            self.lfo_step = 4;
        }
        self.lfo_residue_step = self.lfo_step << 1;

        self.wet = p_wet;
        self.phase_invert = if p_invert_phase { -1 } else { 1 };
    }

    fn process_channel(
        buffer: &mut [f64],
        p_buffer_index: usize,
        r_delay_buffer: &mut [f64],
        p_delay: i32,
        pointer_read: i32,
        pointer_write: i32,
        feedback: f64,
        wet: f64,
    ) {
        let delay_index = ((pointer_read + p_delay) & DELAY_BUFFER_FILTER) as usize;
        let value = r_delay_buffer[delay_index];
        let next_value = buffer[p_buffer_index] - value * feedback;

        r_delay_buffer[pointer_write as usize] = next_value;
        buffer[p_buffer_index] *= 1.0 - wet;
        buffer[p_buffer_index] += value * wet;
    }

    fn process_lfo(&mut self, r_buffer: &mut [f64], p_start_index: i32, p_length: i32) {
        let delay_left = self.phase_table[self.lfo_phase];
        let delay_right = self.phase_table[self.lfo_phase] * self.phase_invert;

        let mut i = p_start_index;
        while i < (p_start_index + p_length) {
            let index = i as usize;
            let wet = self.wet;
            let feedback = self.feedback;
            let pointer_read = self.pointer_read;
            let pointer_write = self.pointer_write;
            Self::process_channel(
                r_buffer,
                index,
                &mut self.delay_buffer_left,
                delay_left,
                pointer_read,
                pointer_write,
                feedback,
                wet,
            );
            let wet = self.wet;
            let feedback = self.feedback;
            let pointer_read = self.pointer_read;
            let pointer_write = self.pointer_write;
            Self::process_channel(
                r_buffer,
                index + 1,
                &mut self.delay_buffer_right,
                delay_right,
                pointer_read,
                pointer_write,
                feedback,
                wet,
            );

            self.pointer_write = (self.pointer_write + 1) & DELAY_BUFFER_FILTER;
            self.pointer_read = (self.pointer_read + 1) & DELAY_BUFFER_FILTER;
            i += 2;
        }
    }
}

impl EffectBase for EffectStereoChorus {
    fn settings(&self) -> &EffectSettings {
        &self.settings
    }

    fn settings_mut(&mut self) -> &mut EffectSettings {
        &mut self.settings
    }

    fn prepare_process(&mut self) -> i32 {
        self.lfo_phase = 0;
        self.lfo_residue_step = 0;
        self.pointer_read = 0;
        for item in self.delay_buffer_left.iter_mut() {
            *item = 0.0;
        }
        for item in self.delay_buffer_right.iter_mut() {
            *item = 0.0;
        }
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

        let mut step = self.lfo_residue_step;
        let max = start_index + length;
        let mut i = start_index;
        while i < (max - step) {
            self.process_lfo(r_buffer, i, step);

            self.lfo_phase += 1;
            if self.lfo_phase >= self.phase_table.len() {
                self.lfo_phase = 0;
            }

            i += step;
            step = self.lfo_step << 1;
        }

        self.process_lfo(r_buffer, i, max - i);
        self.lfo_residue_step = step - (max - i);

        p_channels
    }

    fn set_by_mml(&mut self, p_args: &[f64]) {
        let delay_time = get_mml_arg(p_args, 0, 20.0);
        let feedback = get_mml_arg(p_args, 1, 20.0) / 100.0;
        let frequency = get_mml_arg(p_args, 2, 4.0);
        let depth = get_mml_arg(p_args, 3, 20.0);
        let wet = get_mml_arg(p_args, 4, 50.0) / 100.0;
        let invert_phase = get_mml_arg(p_args, 5, 0.0) as i32;
        self.set_params(delay_time, feedback, frequency, depth, wet, invert_phase != 0);
    }

    fn reset(&mut self) {
        self.set_params(20.0, 0.2, 4.0, 20.0, 0.5, true);
    }
}

impl Default for EffectStereoChorus {
    fn default() -> Self {
        Self::new(20.0, 0.2, 4.0, 20.0, 0.5, true)
    }
}
