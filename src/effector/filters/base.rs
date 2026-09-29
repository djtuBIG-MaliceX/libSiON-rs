//! Port of `effector/filters/si_filter_base.{h,cpp}`.
//!
//! `SiFilterBase` adds the biquad state (coefficients `a1,a2,b0,b1,b2` and
//! per-channel history `ChannelValues`) to `SiEffectBase` and provides the
//! shared direct-form I difference equation used by every concrete filter.
//! The Rust port embeds [`FilterBase`] in each concrete filter struct;
//! coefficient generation stays in the individual filter modules exactly as
//! in the C++ `set_params` bodies.

use crate::effector::effect_base::EffectSettings;
use crate::math::clampf;

/// `SiFilterBase::THRESHOLD`.
pub const THRESHOLD: f64 = 0.0000152587890625;

/// `SiFilterBase::ChannelValues`.
#[derive(Debug, Clone, Copy)]
pub struct ChannelValues {
    pub in1: f64,
    pub in2: f64,
    pub out1: f64,
    pub out2: f64,
}

impl ChannelValues {
    pub fn check_threshold(&mut self) {
        if self.out1 < THRESHOLD {
            self.out1 = 0.0;
            self.out2 = 0.0;
        }
    }

    pub fn clear(&mut self) {
        self.in1 = 0.0;
        self.in2 = 0.0;
        self.out1 = 0.0;
        self.out2 = 0.0;
    }
}

/// `SiFilterBase` state + shared biquad processing.
#[derive(Debug, Clone)]
pub struct FilterBase {
    pub settings: EffectSettings,
    pub a1: f64,
    pub a2: f64,
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub left: ChannelValues,
    pub right: ChannelValues,
}

impl FilterBase {
    pub fn new() -> Self {
        FilterBase {
            settings: EffectSettings::new(),
            a1: 0.0,
            a2: 0.0,
            b0: 0.0,
            b1: 0.0,
            b2: 0.0,
            left: ChannelValues {
                in1: 0.0,
                in2: 0.0,
                out1: 0.0,
                out2: 0.0,
            },
            right: ChannelValues {
                in1: 0.0,
                in2: 0.0,
                out1: 0.0,
                out2: 0.0,
            },
        }
    }

    fn process_channel(&mut self, p_right: bool, p_input: f64) -> f64 {
        let (a1, a2, b0, b1, b2) = (self.a1, self.a2, self.b0, self.b1, self.b2);
        let channel = if p_right { &mut self.right } else { &mut self.left };
        let mut output =
            b0 * p_input + b1 * channel.in1 + b2 * channel.in2 - a1 * channel.out1 - a2 * channel.out2;
        output = clampf(output, -1.0, 1.0);
        channel.in2 = channel.in1;
        channel.in1 = p_input;
        channel.out2 = channel.out1;
        channel.out1 = output;
        output
    }

    pub fn prepare_process(&mut self) -> i32 {
        self.left.clear();
        self.right.clear();
        2
    }

    pub fn process(
        &mut self,
        p_channels: i32,
        r_buffer: &mut [f64],
        p_start_index: i32,
        p_length: i32,
    ) -> i32 {
        let start_index = (p_start_index << 1) as usize;
        let length = (p_length << 1) as usize;
        self.left.check_threshold();
        self.right.check_threshold();
        if p_channels == 2 {
            let mut i = start_index;
            while i < start_index + length {
                let left = self.process_channel(false, r_buffer[i]);
                r_buffer[i] = left;
                let right = self.process_channel(true, r_buffer[i + 1]);
                r_buffer[i + 1] = right;
                i += 2;
            }
        } else {
            let mut i = start_index;
            while i < start_index + length {
                let value = self.process_channel(false, r_buffer[i]);
                r_buffer[i] = value;
                r_buffer[i + 1] = value;
                i += 2;
            }
        }
        p_channels
    }
}

impl Default for FilterBase {
    fn default() -> Self {
        Self::new()
    }
}

/// Generates the `EffectBase` impl for a concrete `FilterBase`-backed filter:
/// settings accessors, delegation of `prepare_process`/`process` to the
/// embedded base, and `set_by_mml`/`reset` forwarding to the inherent
/// `set_params_by_mml`/`reset_params` methods each filter implements.
#[macro_export]
macro_rules! filter_effect {
    ($name:ident) => {
        impl $crate::effector::effect_base::EffectBase for $name {
            fn settings(&self) -> &$crate::effector::effect_base::EffectSettings {
                &self.base.settings
            }
            fn settings_mut(&mut self) -> &mut $crate::effector::effect_base::EffectSettings {
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
                self.base.process(p_channels, r_buffer, p_start_index, p_length)
            }
            fn set_by_mml(&mut self, p_args: &[f64]) {
                self.set_params_by_mml(p_args);
            }
            fn reset(&mut self) {
                self.reset_params();
            }
        }
    };
}
