//! Port of `effector/effects/si_effect_equalizer.{h,cpp}`.
//!
//! Three-band state-variable pipeline equalizer. Note the C++ pipeline
//! reads `sdm3` (delayed one frame) BEFORE shifting `sdm1/sdm2/sdm3` —
//! evaluation order preserved verbatim, including the denormal bias adds.

use crate::effector::effect_base::{get_mml_arg, EffectBase, EffectSettings};
use crate::math;

#[derive(Debug, Clone, Copy, Default)]
pub struct PipeChannel {
    pub f1p0: f64,
    pub f1p1: f64,
    pub f1p2: f64,
    pub f1p3: f64,
    pub f2p0: f64,
    pub f2p1: f64,
    pub f2p2: f64,
    pub f2p3: f64,
    pub sdm1: f64,
    pub sdm2: f64,
    pub sdm3: f64,
}

impl PipeChannel {
    pub fn clear(&mut self) {
        *self = PipeChannel::default();
    }
}

pub struct EffectEqualizer {
    pub settings: EffectSettings,
    pub left: PipeChannel,
    pub right: PipeChannel,
    pub low_frequency: f64,
    pub high_frequency: f64,
    pub low_gain: f64,
    pub mid_gain: f64,
    pub high_gain: f64,
}

impl EffectEqualizer {
    pub fn new(
        p_low_gain: f64,
        p_mid_gain: f64,
        p_high_gain: f64,
        p_low_frequency: f64,
        p_high_frequency: f64,
    ) -> Self {
        let mut effect = EffectEqualizer {
            settings: EffectSettings::new(),
            left: PipeChannel::default(),
            right: PipeChannel::default(),
            low_frequency: 0.0,
            high_frequency: 0.0,
            low_gain: 0.0,
            mid_gain: 0.0,
            high_gain: 0.0,
        };
        effect.set_params(p_low_gain, p_mid_gain, p_high_gain, p_low_frequency, p_high_frequency);
        effect
    }

    pub fn set_params(
        &mut self,
        p_low_gain: f64,
        p_mid_gain: f64,
        p_high_gain: f64,
        p_low_frequency: f64,
        p_high_frequency: f64,
    ) {
        self.low_gain = p_low_gain;
        self.mid_gain = p_mid_gain;
        self.high_gain = p_high_gain;
        self.low_frequency = 2.0 * math::sin(p_low_frequency * 0.00007123792865282977);
        self.high_frequency = 2.0 * math::sin(p_high_frequency * 0.00007123792865282977);
    }

    fn process_channel(&mut self, p_right: bool, p_value: f64) -> f64 {
        let low_frequency = self.low_frequency;
        let high_frequency = self.high_frequency;
        let low_gain = self.low_gain;
        let mid_gain = self.mid_gain;
        let high_gain = self.high_gain;
        let channel = if p_right { &mut self.right } else { &mut self.left };

        channel.f1p0 += (low_frequency * (p_value - channel.f1p0)) + 2.3283064370807974e-10;
        channel.f1p1 += low_frequency * (channel.f1p0 - channel.f1p1);
        channel.f1p2 += low_frequency * (channel.f1p1 - channel.f1p2);
        channel.f1p3 += low_frequency * (channel.f1p2 - channel.f1p3);

        channel.f2p0 += (high_frequency * (p_value - channel.f2p0)) + 2.3283064370807974e-10;
        channel.f2p1 += high_frequency * (channel.f2p0 - channel.f2p1);
        channel.f2p2 += high_frequency * (channel.f2p1 - channel.f2p2);
        channel.f2p3 += high_frequency * (channel.f2p2 - channel.f2p3);

        let value_low = channel.f1p3;
        let value_high = channel.sdm3 - channel.f2p3;
        let value_mid = channel.sdm3 - (value_high + value_low);

        channel.sdm3 = channel.sdm2;
        channel.sdm2 = channel.sdm1;
        channel.sdm1 = p_value;

        value_low * low_gain + value_mid * mid_gain + value_high * high_gain
    }

    fn process_mono(&mut self, r_buffer: &mut [f64], p_start_index: i32, p_length: i32) {
        let mut i = p_start_index;
        while i < (p_start_index + p_length) {
            let index = i as usize;
            let value = self.process_channel(false, r_buffer[index]);
            r_buffer[index] = value;
            r_buffer[index + 1] = value;
            i += 2;
        }
    }

    fn process_stereo(&mut self, r_buffer: &mut [f64], p_start_index: i32, p_length: i32) {
        let mut i = p_start_index;
        while i < (p_start_index + p_length) {
            let index = i as usize;
            let value_left = self.process_channel(false, r_buffer[index]);
            r_buffer[index] = value_left;

            let value_right = self.process_channel(true, r_buffer[index + 1]);
            r_buffer[index + 1] = value_right;
            i += 2;
        }
    }
}

impl EffectBase for EffectEqualizer {
    fn settings(&self) -> &EffectSettings {
        &self.settings
    }

    fn settings_mut(&mut self) -> &mut EffectSettings {
        &mut self.settings
    }

    fn prepare_process(&mut self) -> i32 {
        self.left.clear();
        self.right.clear();
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

        if p_channels == 1 {
            self.process_mono(r_buffer, start_index, length);
        } else {
            self.process_stereo(r_buffer, start_index, length);
        }

        p_channels
    }

    fn set_by_mml(&mut self, p_args: &[f64]) {
        let low_gain = get_mml_arg(p_args, 0, 100.0) / 100.0;
        let mid_gain = get_mml_arg(p_args, 1, 100.0) / 100.0;
        let high_gain = get_mml_arg(p_args, 2, 100.0) / 100.0;
        let low_frequency = get_mml_arg(p_args, 3, 880.0);
        let high_frequency = get_mml_arg(p_args, 4, 5000.0);
        self.set_params(low_gain, mid_gain, high_gain, low_frequency, high_frequency);
    }

    fn reset(&mut self) {
        self.set_params(1.0, 1.0, 1.0, 880.0, 5000.0);
    }
}

impl Default for EffectEqualizer {
    fn default() -> Self {
        Self::new(1.0, 1.0, 1.0, 880.0, 5000.0)
    }
}
