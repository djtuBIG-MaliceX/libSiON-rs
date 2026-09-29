//! Port of `effector/filters/si_controllable_filter_high_pass.{h,cpp}`.

use crate::effector::effect_base::{EffectBase, EffectSettings};
use crate::effector::filters::controllable_base::{ControllableFilterBase, ControllableLfo};

pub struct ControllableHighPass {
    pub base: ControllableFilterBase,
}

impl ControllableHighPass {
    pub fn new(p_cutoff: f64, p_resonance: f64) -> Self {
        let mut filter = ControllableHighPass {
            base: ControllableFilterBase::new(),
        };
        filter.base.reset_params();
        filter.base.set_params_manually(p_cutoff, p_resonance);
        filter
    }

    fn process_lfo(
        lfo: &mut ControllableLfo,
        cutoff: f64,
        feedback: f64,
        r_buffer: &mut [f64],
        p_start_index: i32,
        p_length: i32,
    ) {
        let mut i = p_start_index;
        while i < (p_start_index + p_length) {
            let index = i as usize;
            let value_left = r_buffer[index];
            lfo.p0_left += cutoff * (value_left - lfo.p0_left + feedback * (lfo.p0_left - lfo.p1_left));
            lfo.p1_left += cutoff * (lfo.p0_left - lfo.p1_left);
            r_buffer[index] = value_left - lfo.p0_left;
            i += 1;

            let index = i as usize;
            let value_right = r_buffer[index];
            lfo.p0_right += cutoff * (value_right - lfo.p0_right + feedback * (lfo.p0_right - lfo.p1_right));
            lfo.p1_right += cutoff * (lfo.p0_right - lfo.p1_right);
            r_buffer[index] = value_right - lfo.p0_right;
            i += 1;
        }
    }
}

impl EffectBase for ControllableHighPass {
    fn settings(&self) -> &EffectSettings {
        &self.base.settings
    }

    fn settings_mut(&mut self) -> &mut EffectSettings {
        &mut self.base.settings
    }

    fn prepare_process(&mut self) -> i32 {
        self.base.prepare_process()
    }

    fn process(
        &mut self,
        p_channels: i32,
        r_buffer: &mut [f64],
        p_start_index: i32,
        p_length: i32,
    ) -> i32 {
        self.base
            .process(p_channels, r_buffer, p_start_index, p_length, Self::process_lfo)
    }

    fn set_by_mml(&mut self, p_args: &[f64]) {
        self.base.set_params_by_mml(p_args);
    }

    fn reset(&mut self) {
        self.base.reset_params();
    }
}

impl Default for ControllableHighPass {
    fn default() -> Self {
        Self::new(1.0, 0.0)
    }
}
