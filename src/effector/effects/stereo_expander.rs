//! Port of `effector/effects/si_effect_stereo_expander.{h,cpp}`.
//!
//! Angle-matrix width/rotation/phase control with post-rotation gain
//! renormalization (only when the resulting center vector length exceeds
//! 0.01). `width == 0 && rotation == 0 && !invert` monoralizes with the
//! 1/sqrt(2) mid sum.

use crate::effector::effect_base::{get_mml_arg, EffectBase, EffectSettings};
use crate::math;

pub struct EffectStereoExpander {
    pub settings: EffectSettings,
    pub left_to_left: f64,
    pub right_to_left: f64,
    pub left_to_right: f64,
    pub right_to_right: f64,
    pub monoralize: bool,
}

impl EffectStereoExpander {
    pub fn new(p_stereo_width: f64, p_rotation: f64, p_phase_invert: bool) -> Self {
        let mut effect = EffectStereoExpander {
            settings: EffectSettings::new(),
            left_to_left: 0.0,
            right_to_left: 0.0,
            left_to_right: 0.0,
            right_to_right: 0.0,
            monoralize: false,
        };
        effect.set_params(p_stereo_width, p_rotation, p_phase_invert);
        effect
    }

    pub fn set_params(&mut self, p_stereo_width: f64, p_rotation: f64, p_phase_invert: bool) {
        self.monoralize = p_stereo_width == 0.0 && p_rotation == 0.0 && !p_phase_invert;

        let half_width = p_stereo_width * 0.7853981633974483;
        let center_angle = (p_rotation + 0.5) * 1.5707963267948965;
        let left_angle = center_angle - half_width;
        let right_angle = center_angle + half_width;
        let invert = if p_phase_invert { -1.0 } else { 1.0 };

        self.left_to_left = math::cos(left_angle);
        self.right_to_left = math::sin(left_angle);
        self.left_to_right = math::cos(right_angle) * invert;
        self.right_to_right = math::sin(right_angle) * invert;

        let x = self.left_to_left + self.left_to_right;
        let y = self.right_to_left + self.right_to_right;
        let mut l = math::sqrt(x * x + y * y);
        if l > 0.01 {
            l = 1.0 / l;
            self.left_to_left *= l;
            self.right_to_left *= l;
            self.left_to_right *= l;
            self.right_to_right *= l;
        }
    }
}

impl EffectBase for EffectStereoExpander {
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

        if self.monoralize {
            let mut i = start_index;
            while i < (start_index + length) {
                let index = i as usize;
                let mut value = r_buffer[index] + r_buffer[index + 1];
                value *= 0.7071067811865476;
                r_buffer[index] = value;
                r_buffer[index + 1] = value;
                i += 2;
            }
            return 1;
        }

        let mut i = start_index;
        while i < (start_index + length) {
            let index = i as usize;
            let value_left = r_buffer[index];
            let value_right = r_buffer[index + 1];
            r_buffer[index] = value_left * self.left_to_left + value_right * self.right_to_left;
            r_buffer[index + 1] =
                value_left * self.left_to_right + value_right * self.right_to_right;
            i += 2;
        }

        2
    }

    fn set_by_mml(&mut self, p_args: &[f64]) {
        let stereo_width = get_mml_arg(p_args, 0, 140.0) / 100.0;
        let rotation = get_mml_arg(p_args, 1, 0.0) / 100.0;
        let phase_invert = get_mml_arg(p_args, 2, 0.0) as i32;
        self.set_params(stereo_width, rotation, phase_invert != 0);
    }

    fn reset(&mut self) {
        self.set_params(1.4, 0.0, false);
    }
}

impl Default for EffectStereoExpander {
    fn default() -> Self {
        Self::new(1.4, 0.0, false)
    }
}
