//! Port of `effector/filters/si_filter_band_pass.{h,cpp}`.

use crate::effector::effect_base::get_mml_arg;
use crate::effector::filters::base::FilterBase;
use crate::math;

pub struct BandPassFilter {
    pub base: FilterBase,
}

impl BandPassFilter {
    pub fn new(p_frequency: f64, p_band: f64) -> Self {
        let mut filter = BandPassFilter {
            base: FilterBase::new(),
        };
        filter.set_params(p_frequency, p_band);
        filter
    }

    pub fn set_params(&mut self, p_frequency: f64, p_band: f64) {
        let omg = p_frequency * 0.00014247585730565955;
        let cos = math::cos(omg);
        let sin = math::sin(omg);
        let ang = 0.34657359027997264 * p_band * omg / sin;
        let alp = sin * math::sinh(ang);
        let ia0 = 1.0 / (1.0 + alp);
        self.base.a1 = -2.0 * cos * ia0;
        self.base.a2 = (1.0 - alp) * ia0;
        self.base.b1 = 0.0;
        self.base.b0 = alp * ia0;
        self.base.b2 = -self.base.b0;
    }
}

impl BandPassFilter {
    pub fn set_params_by_mml(&mut self, p_args: &[f64]) {
        let frequency = get_mml_arg(p_args, 0, 3000.0);
        let band = get_mml_arg(p_args, 1, 1.0);
        self.set_params(frequency, band);
    }

    pub fn reset_params(&mut self) {
        self.set_params(3000.0, 1.0);
    }
}

crate::filter_effect!(BandPassFilter);

impl Default for BandPassFilter {
    fn default() -> Self {
        Self::new(3000.0, 1.0)
    }
}
