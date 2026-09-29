//! Port of `effector/effects/si_effect_distortion.{h,cpp}`.
//!
//! Pre-scaled soft clip followed by an optional internal low-pass biquad
//! (same coefficient formulas as `SiFilterLowPass`). Mono in, mono out
//! written to both channels.

use crate::effector::effect_base::{get_mml_arg, EffectBase, EffectSettings};
use crate::math::{self, clampf};

pub const THRESHOLD: f64 = 0.0000152587890625;

pub struct EffectDistortion {
    pub settings: EffectSettings,
    pub filter_enabled: bool,
    pub pre_scale: f64,
    pub limit: f64,
    pub a1: f64,
    pub a2: f64,
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub in1: f64,
    pub in2: f64,
    pub out1: f64,
    pub out2: f64,
}

impl EffectDistortion {
    pub fn new(p_pre_gain: f64, p_post_gain: f64, p_lpf_frequency: f64, p_lpf_slope: f64) -> Self {
        let mut effect = EffectDistortion {
            settings: EffectSettings::new(),
            filter_enabled: false,
            pre_scale: 0.0,
            limit: 0.0,
            a1: 0.0,
            a2: 0.0,
            b0: 0.0,
            b1: 0.0,
            b2: 0.0,
            in1: 0.0,
            in2: 0.0,
            out1: 0.0,
            out2: 0.0,
        };
        effect.set_params(p_pre_gain, p_post_gain, p_lpf_frequency, p_lpf_slope);
        effect
    }

    pub fn set_params(
        &mut self,
        p_pre_gain: f64,
        p_post_gain: f64,
        p_lpf_frequency: f64,
        p_lpf_slope: f64,
    ) {
        self.limit = math::pow(2.0, -p_post_gain / 6.0);
        self.pre_scale = math::pow(2.0, -p_pre_gain / 6.0) * self.limit;
        self.filter_enabled = p_lpf_frequency > 0.0;

        if self.filter_enabled {
            let omg = p_lpf_frequency * 0.00014247585730565955;
            let cos = math::cos(omg);
            let sin = math::sin(omg);
            let ang = 0.34657359027997264 * p_lpf_slope * omg / sin;
            let alp = sin * math::sinh(ang);
            let ia0 = 1.0 / (1.0 + alp);
            self.a1 = -2.0 * cos * ia0;
            self.a2 = (1.0 - alp) * ia0;
            self.b1 = (1.0 - cos) * ia0;
            self.b0 = self.b1 * 0.5;
            self.b2 = self.b1 * 0.5;
        }
    }
}

impl EffectBase for EffectDistortion {
    fn settings(&self) -> &EffectSettings {
        &self.settings
    }

    fn settings_mut(&mut self) -> &mut EffectSettings {
        &mut self.settings
    }

    fn prepare_process(&mut self) -> i32 {
        self.in1 = 0.0;
        self.in2 = 0.0;
        self.out1 = 0.0;
        self.out2 = 0.0;
        1
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

        if self.out1 < THRESHOLD {
            self.out1 = 0.0;
            self.out2 = 0.0;
        }

        let mut i = start_index;
        while i < (start_index + length) {
            let index = i as usize;
            let value = clampf(r_buffer[index] * self.pre_scale, -self.limit, self.limit);

            let mut output = value;
            if self.filter_enabled {
                output = self.b0 * value + self.b1 * self.in1 + self.b2 * self.in2
                    - self.a1 * self.out1
                    - self.a2 * self.out2;

                self.in2 = self.in1;
                self.in1 = value;

                self.out2 = self.out1;
                self.out1 = output;
            }

            r_buffer[index] = output;
            r_buffer[index + 1] = output;
            i += 2;
        }

        1
    }

    fn set_by_mml(&mut self, p_args: &[f64]) {
        let pre_gain = get_mml_arg(p_args, 0, -60.0);
        let post_gain = get_mml_arg(p_args, 1, 18.0);
        let lpf_frequency = get_mml_arg(p_args, 2, 2400.0);
        let lpf_slope = get_mml_arg(p_args, 3, 1.0);
        self.set_params(pre_gain, post_gain, lpf_frequency, lpf_slope);
    }

    fn reset(&mut self) {
        self.set_params(-60.0, 18.0, 2400.0, 1.0);
    }
}

impl Default for EffectDistortion {
    fn default() -> Self {
        Self::new(-60.0, 18.0, 2400.0, 1.0)
    }
}
