//! Port of `effector/filters/si_filter_peak.{h,cpp}`.

use crate::effector::effect_base::get_mml_arg;
use crate::effector::filters::base::FilterBase;
use crate::math;

pub struct PeakFilter {
    pub base: FilterBase,
}

impl PeakFilter {
    pub fn new(p_frequency: f64, p_band: f64, p_gain: f64) -> Self {
        let mut filter = PeakFilter {
            base: FilterBase::new(),
        };
        filter.set_params(p_frequency, p_band, p_gain);
        filter
    }

    pub fn set_params(&mut self, p_frequency: f64, p_band: f64, p_gain: f64) {
        let a = math::pow(10.0, p_gain * 0.025);
        let omg = p_frequency * 0.00014247585730565955;
        let cos = math::cos(omg);
        let sin = math::sin(omg);
        let ang = 0.34657359027997264 * p_band * omg / sin;
        let alp = sin * math::sinh(ang);
        let alpa = alp * a;
        let alpia = alp / a;
        let ia0 = 1.0 / (1.0 + alpia);
        self.base.a1 = -2.0 * cos * ia0;
        self.base.b1 = self.base.a1;
        self.base.a2 = (1.0 - alpia) * ia0;
        self.base.b0 = (1.0 + alpa) * ia0;
        self.base.b2 = (1.0 - alpa) * ia0;
    }
}

impl PeakFilter {
    pub fn set_params_by_mml(&mut self, p_args: &[f64]) {
        let frequency = get_mml_arg(p_args, 0, 3000.0);
        let band = get_mml_arg(p_args, 1, 1.0);
        let gain = get_mml_arg(p_args, 2, 6.0);
        self.set_params(frequency, band, gain);
    }

    pub fn reset_params(&mut self) {
        self.set_params(3000.0, 1.0, 6.0);
    }
}

crate::filter_effect!(PeakFilter);

impl Default for PeakFilter {
    fn default() -> Self {
        Self::new(3000.0, 1.0, 6.0)
    }
}
