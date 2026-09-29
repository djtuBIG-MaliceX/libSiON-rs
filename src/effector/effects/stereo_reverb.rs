//! Port of `effector/effects/si_effect_stereo_reverb.{h,cpp}`.
//!
//! Schroeder-style multi-tap feedback reverb: each channel's delay ring
//! mixes three weighted taps (`read0/read1/read2`) into the dry signal.

use crate::effector::effect_base::{get_mml_arg, EffectBase, EffectSettings};
use crate::math::clampf;

const DELAY_BUFFER_BITS: i32 = 13;
const DELAY_BUFFER_FILTER: i32 = (1 << DELAY_BUFFER_BITS) - 1;

pub struct EffectStereoReverb {
    pub settings: EffectSettings,
    pub delay_buffer_left: Vec<f64>,
    pub delay_buffer_right: Vec<f64>,
    pub pointer_read0: i32,
    pub pointer_read1: i32,
    pub pointer_read2: i32,
    pub pointer_write: i32,
    pub feedback0: f64,
    pub feedback1: f64,
    pub feedback2: f64,
    pub wet: f64,
}

impl EffectStereoReverb {
    pub fn new(p_delay1: f64, p_delay2: f64, p_feedback: f64, p_wet: f64) -> Self {
        let mut effect = EffectStereoReverb {
            settings: EffectSettings::new(),
            delay_buffer_left: vec![0.0; (1 << DELAY_BUFFER_BITS) as usize],
            delay_buffer_right: vec![0.0; (1 << DELAY_BUFFER_BITS) as usize],
            pointer_read0: 0,
            pointer_read1: 0,
            pointer_read2: 0,
            pointer_write: 0,
            feedback0: 0.0,
            feedback1: 0.0,
            feedback2: 0.0,
            wet: 0.0,
        };
        effect.set_params(p_delay1, p_delay2, p_feedback, p_wet);
        effect
    }

    pub fn set_params(&mut self, p_delay1: f64, p_delay2: f64, p_feedback: f64, p_wet: f64) {
        let delay1 = clampf(p_delay1, 0.01, 0.99);
        let delay2 = clampf(p_delay2, 0.01, 0.99);

        self.pointer_write = (self.pointer_read0 + DELAY_BUFFER_FILTER) & DELAY_BUFFER_FILTER;
        self.pointer_read1 = ((self.pointer_read0 as f64
            + DELAY_BUFFER_FILTER as f64 * (1.0 - delay1)) as i32)
            & DELAY_BUFFER_FILTER;
        self.pointer_read2 = ((self.pointer_read0 as f64
            + DELAY_BUFFER_FILTER as f64 * (1.0 - delay2)) as i32)
            & DELAY_BUFFER_FILTER;

        let feedback = clampf(p_feedback, -0.99, 0.99);
        self.feedback0 = feedback * 0.2;
        self.feedback1 = feedback * 0.3;
        self.feedback2 = feedback * 0.5;

        self.wet = p_wet;
    }

    fn process_channel(
        &mut self,
        r_buffer: &mut [f64],
        p_buffer_index: usize,
        p_right: bool,
    ) {
        let read0 = self.pointer_read0 as usize;
        let read1 = self.pointer_read1 as usize;
        let read2 = self.pointer_read2 as usize;
        let write = self.pointer_write as usize;

        let mut value = if p_right {
            self.delay_buffer_right[read0] * self.feedback0
        } else {
            self.delay_buffer_left[read0] * self.feedback0
        };
        value += if p_right {
            self.delay_buffer_right[read1] * self.feedback1
        } else {
            self.delay_buffer_left[read1] * self.feedback1
        };
        value += if p_right {
            self.delay_buffer_right[read2] * self.feedback2
        } else {
            self.delay_buffer_left[read2] * self.feedback2
        };

        if p_right {
            self.delay_buffer_right[write] = r_buffer[p_buffer_index] - value;
        } else {
            self.delay_buffer_left[write] = r_buffer[p_buffer_index] - value;
        }

        r_buffer[p_buffer_index] *= 1.0 - self.wet;
        r_buffer[p_buffer_index] += value * self.wet;
    }
}

impl EffectBase for EffectStereoReverb {
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
            self.process_channel(r_buffer, index, false);
            self.process_channel(r_buffer, index + 1, true);

            self.pointer_write = (self.pointer_write + 1) & DELAY_BUFFER_FILTER;
            self.pointer_read0 = (self.pointer_read0 + 1) & DELAY_BUFFER_FILTER;
            self.pointer_read1 = (self.pointer_read1 + 1) & DELAY_BUFFER_FILTER;
            self.pointer_read2 = (self.pointer_read2 + 1) & DELAY_BUFFER_FILTER;
            i += 2;
        }

        p_channels
    }

    fn set_by_mml(&mut self, p_args: &[f64]) {
        let delay1 = get_mml_arg(p_args, 0, 70.0) / 100.0;
        let delay2 = get_mml_arg(p_args, 1, 40.0) / 100.0;
        let feedback = get_mml_arg(p_args, 2, 80.0) / 100.0;
        let wet = get_mml_arg(p_args, 3, 100.0) / 100.0;
        self.set_params(delay1, delay2, feedback, wet);
    }

    fn reset(&mut self) {
        self.set_params(0.7, 0.4, 0.8, 0.3);
    }
}

impl Default for EffectStereoReverb {
    fn default() -> Self {
        Self::new(0.7, 0.4, 0.8, 0.3)
    }
}
