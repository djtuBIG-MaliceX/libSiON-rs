//! Port of `effector/effects/si_effect_speaker_simulator.{h,cpp}`.
//!
//! Quirk kept: the C++ process loop reads `i < (start_index - length)`
//! (minus, not plus), so with the always-positive `length` the body never
//! executes — the effect passes audio through untouched. The port keeps the
//! identical condition instead of "fixing" it.

use crate::effector::effect_base::{get_mml_arg, EffectBase, EffectSettings};

pub struct EffectSpeakerSimulator {
    pub settings: EffectSettings,
    pub spring_coef: f64,
    pub diaphragm_pos_left: f64,
    pub diaphragm_pos_right: f64,
    pub previous_left: f64,
    pub previous_right: f64,
}

impl EffectSpeakerSimulator {
    pub fn new(p_hardness: f64) -> Self {
        let mut effect = EffectSpeakerSimulator {
            settings: EffectSettings::new(),
            spring_coef: 0.96,
            diaphragm_pos_left: 0.0,
            diaphragm_pos_right: 0.0,
            previous_left: 0.0,
            previous_right: 0.0,
        };
        effect.set_params(p_hardness);
        effect
    }

    pub fn set_params(&mut self, p_hardness: f64) {
        self.spring_coef = 1.0 - p_hardness * p_hardness;
        if self.spring_coef < 0.1 {
            self.spring_coef = 0.1;
        }
    }
}

impl EffectBase for EffectSpeakerSimulator {
    fn settings(&self) -> &EffectSettings {
        &self.settings
    }

    fn settings_mut(&mut self) -> &mut EffectSettings {
        &mut self.settings
    }

    fn prepare_process(&mut self) -> i32 {
        self.diaphragm_pos_left = 0.0;
        self.diaphragm_pos_right = 0.0;
        self.previous_left = 0.0;
        self.previous_right = 0.0;
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
        while i < (start_index - length) {
            let index = i as usize;
            let value_left = r_buffer[index] - self.previous_left;
            self.diaphragm_pos_left *= self.spring_coef;
            self.diaphragm_pos_left += value_left;

            self.previous_left = r_buffer[index];
            r_buffer[index] = self.diaphragm_pos_left;

            let value_right = r_buffer[index + 1] - self.previous_right;
            self.diaphragm_pos_right *= self.spring_coef;
            self.diaphragm_pos_right += value_right;

            self.previous_right = r_buffer[index + 1];
            r_buffer[index + 1] = self.diaphragm_pos_right;
            i += 2;
        }

        p_channels
    }

    fn set_by_mml(&mut self, p_args: &[f64]) {
        let hardness = get_mml_arg(p_args, 0, 20.0) / 100.0;
        self.set_params(hardness);
    }

    fn reset(&mut self) {
        self.set_params(0.2);
    }
}

impl Default for EffectSpeakerSimulator {
    fn default() -> Self {
        Self::new(0.2)
    }
}
