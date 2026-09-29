//! Port of `effector/effects/si_effect_stereo_delay.{h,cpp}`.
//!
//! Ping-pong capable feedback delay over a power-of-two ring buffer. The
//! C++ cross-channel ping-pong reads/writes the two buffers with an order
//! that matters when `_cross` swaps read sources; the port keeps that
//! write-then-read ordering per frame pair.

use crate::effector::effect_base::{get_mml_arg, EffectBase, EffectSettings};

const DELAY_BUFFER_BITS: i32 = 16;
const DELAY_BUFFER_FILTER: i32 = (1 << DELAY_BUFFER_BITS) - 1;

pub struct EffectStereoDelay {
    pub settings: EffectSettings,
    pub delay_buffer_left: Vec<f64>,
    pub delay_buffer_right: Vec<f64>,
    pub pointer_read: i32,
    pub pointer_write: i32,
    pub feedback: f64,
    pub wet: f64,
    pub cross: bool,
}

impl EffectStereoDelay {
    pub fn new(p_delay_time: f64, p_feedback: f64, p_cross: bool, p_wet: f64) -> Self {
        let mut effect = EffectStereoDelay {
            settings: EffectSettings::new(),
            delay_buffer_left: vec![0.0; (1 << DELAY_BUFFER_BITS) as usize],
            delay_buffer_right: vec![0.0; (1 << DELAY_BUFFER_BITS) as usize],
            pointer_read: 0,
            pointer_write: 0,
            feedback: 0.0,
            wet: 0.0,
            cross: false,
        };
        effect.set_params(p_delay_time, p_feedback, p_cross, p_wet);
        effect
    }

    pub fn set_params(&mut self, p_delay_time: f64, p_feedback: f64, p_cross: bool, p_wet: f64) {
        let mut offset = (p_delay_time * 44.1) as i32;
        if offset > DELAY_BUFFER_FILTER {
            offset = DELAY_BUFFER_FILTER;
        }

        self.pointer_write = (self.pointer_read + offset) & DELAY_BUFFER_FILTER;

        self.feedback = p_feedback;
        if self.feedback >= 1.0 {
            self.feedback = 0.9990234375;
        } else if self.feedback <= -1.0 {
            self.feedback = -0.9990234375;
        }

        self.wet = p_wet;
        self.cross = p_cross;
    }
}

impl EffectBase for EffectStereoDelay {
    fn settings(&self) -> &EffectSettings {
        &self.settings
    }

    fn settings_mut(&mut self) -> &mut EffectSettings {
        &mut self.settings
    }

    fn prepare_process(&mut self) -> i32 {
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

        let mut i = start_index;
        while i < (start_index + length) {
            let index = i as usize;
            let pointer_read = self.pointer_read as usize;
            let pointer_write = self.pointer_write as usize;
            let feedback = self.feedback;
            let wet = self.wet;
            let cross = self.cross;

            let (left, right) = (&mut self.delay_buffer_left, &mut self.delay_buffer_right);
            let value = if cross {
                right[pointer_read]
            } else {
                left[pointer_read]
            };
            left[pointer_write] = r_buffer[index] - value * feedback;
            r_buffer[index] *= 1.0 - wet;
            r_buffer[index] += value * wet;

            let value = if cross {
                left[pointer_read]
            } else {
                right[pointer_read]
            };
            right[pointer_write] = r_buffer[index + 1] - value * feedback;
            r_buffer[index + 1] *= 1.0 - wet;
            r_buffer[index + 1] += value * wet;

            self.pointer_write = (self.pointer_write + 1) & DELAY_BUFFER_FILTER;
            self.pointer_read = (self.pointer_read + 1) & DELAY_BUFFER_FILTER;
            i += 2;
        }

        p_channels
    }

    fn set_by_mml(&mut self, p_args: &[f64]) {
        let delay_time = get_mml_arg(p_args, 0, 250.0);
        let feedback = get_mml_arg(p_args, 1, 25.0) / 100.0;
        let cross = get_mml_arg(p_args, 2, 0.0) as i32;
        let wet = get_mml_arg(p_args, 3, 100.0) / 100.0;
        self.set_params(delay_time, feedback, cross == 1, wet);
    }

    fn reset(&mut self) {
        self.set_params(250.0, 0.25, false, 0.25);
    }
}

impl Default for EffectStereoDelay {
    fn default() -> Self {
        Self::new(250.0, 0.25, false, 0.25)
    }
}
