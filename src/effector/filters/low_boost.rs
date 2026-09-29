//! Port of `effector/filters/si_filter_low_boost.{h,cpp}`.

use crate::effector::effect_base::get_mml_arg;
use crate::effector::filters::base::FilterBase;
use crate::math;

pub struct LowBoostFilter {
    pub base: FilterBase,
}

impl LowBoostFilter {
    pub fn new(p_frequency: f64, p_slope: f64, p_gain: f64) -> Self {
        let mut filter = LowBoostFilter {
            base: FilterBase::new(),
        };
        filter.set_params(p_frequency, p_slope, p_gain);
        filter
    }

    pub fn set_params(&mut self, p_frequency: f64, p_slope: f64, p_gain: f64) {
        let slope = if p_slope < 1.0 { 1.0 } else { p_slope };
        let a = math::pow(10.0, p_gain * 0.025);
        let omg = p_frequency * 0.00014247585730565955;
        let cos = math::cos(omg);
        let sin = math::sin(omg);
        let alp = sin * 0.5 * math::sqrt((a + 1.0 / a) * (1.0 / slope - 1.0) + 2.0);
        let alpsa2 = alp * math::sqrt(a) * 2.0;
        let ia0 = 1.0 / ((a + 1.0) + (a - 1.0) * cos + alpsa2);
        self.base.a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos) * ia0;
        self.base.a2 = ((a + 1.0) + (a - 1.0) * cos - alpsa2) * ia0;
        self.base.b0 = ((a + 1.0) - (a - 1.0) * cos + alpsa2) * a * ia0;
        self.base.b1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos) * a * ia0;
        self.base.b2 = ((a + 1.0) - (a - 1.0) * cos - alpsa2) * a * ia0;
    }
}

impl LowBoostFilter {
    pub fn set_params_by_mml(&mut self, p_args: &[f64]) {
        let frequency = get_mml_arg(p_args, 0, 3000.0);
        let slope = get_mml_arg(p_args, 1, 1.0);
        let gain = get_mml_arg(p_args, 2, 6.0);
        self.set_params(frequency, slope, gain);
    }

    pub fn reset_params(&mut self) {
        self.set_params(3000.0, 1.0, 6.0);
    }
}

crate::filter_effect!(LowBoostFilter);

impl Default for LowBoostFilter {
    fn default() -> Self {
        Self::new(3000.0, 1.0, 6.0)
    }
}
