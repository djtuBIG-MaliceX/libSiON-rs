//! Port of `effector/filters/si_filter_low_pass.{h,cpp}`.

use crate::effector::effect_base::get_mml_arg;
use crate::effector::filters::base::FilterBase;
use crate::math;

pub struct LowPassFilter {
    pub base: FilterBase,
}

impl LowPassFilter {
    pub fn new(p_frequency: f64, p_band: f64) -> Self {
        let mut filter = LowPassFilter {
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
        self.base.b1 = (1.0 - cos) * ia0;
        self.base.b2 = self.base.b1 * 0.5;
        self.base.b0 = self.base.b1 * 0.5;
    }
}

impl LowPassFilter {
    pub fn set_params_by_mml(&mut self, p_args: &[f64]) {
        let frequency = get_mml_arg(p_args, 0, 800.0);
        let band = get_mml_arg(p_args, 1, 1.0);
        self.set_params(frequency, band);
    }

    pub fn reset_params(&mut self) {
        self.set_params(800.0, 1.0);
    }
}

crate::filter_effect!(LowPassFilter);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effector::effect_base::EffectBase;
    use crate::math;

    fn coefficients(frequency: f64, band: f64) -> (f64, f64, f64, f64, f64) {
        let omg = frequency * 0.00014247585730565955;
        let cos = math::cos(omg);
        let sin = math::sin(omg);
        let ang = 0.34657359027997264 * band * omg / sin;
        let alp = sin * math::sinh(ang);
        let ia0 = 1.0 / (1.0 + alp);
        let a1 = -2.0 * cos * ia0;
        let a2 = (1.0 - alp) * ia0;
        let b1 = (1.0 - cos) * ia0;
        let b2 = b1 * 0.5;
        let b0 = b1 * 0.5;
        (a1, a2, b0, b1, b2)
    }

    #[test]
    fn impule_response_matches_formula() {
        let mut filter = LowPassFilter::new(800.0, 1.0);
        let (a1, a2, b0, b1, _b2) = coefficients(800.0, 1.0);

        let b2 = b1 * 0.5;

        let mut buffer = vec![1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        filter.process(1, &mut buffer, 0, 4);

        let s0 = b0;
        let s1 = b1 - a1 * s0;
        let s2 = b2 - a1 * s1 - a2 * s0;

        assert_eq!(buffer[0], s0);
        assert_eq!(buffer[2], s1);
        assert_eq!(buffer[4], s2);
        assert_eq!(buffer[1], s0);
        assert_eq!(buffer[3], s1);
        assert_eq!(buffer[5], s2);
    }

    #[test]
    fn constant_stream_matches_recurrence_and_threshold() {
        let mut filter = LowPassFilter::new(800.0, 1.0);
        let (a1, a2, b0, b1, b2) = coefficients(800.0, 1.0);

        let mut expected = [0.0f64; 3];
        let mut in1 = 0.0;
        let mut in2 = 0.0;
        let mut out1 = 0.0;
        let mut out2 = 0.0;
        for k in 0..3 {
            let mut out = b0 * 0.5 + b1 * in1 + b2 * in2 - a1 * out1 - a2 * out2;
            out = out.clamp(-1.0, 1.0);
            in2 = in1;
            in1 = 0.5;
            out2 = out1;
            out1 = out;
            expected[k] = out;
        }

        let mut buffer = vec![0.5; 12];
        filter.process(1, &mut buffer, 0, 3);
        for k in 0..3 {
            assert_eq!(buffer[2 * k], expected[k]);
            assert_eq!(buffer[2 * k + 1], expected[k]);
        }

        let mut silent = vec![0.0; 12];
        filter.process(1, &mut silent, 0, 3);
        let mut out1 = expected[2];
        let mut out2 = expected[1];
        for k in 0..3 {
            let mut out = -a1 * out1 - a2 * out2;
            out = out.clamp(-1.0, 1.0);
            if out < crate::effector::filters::base::THRESHOLD {
                assert_eq!(silent[2 * k], 0.0);
            }
            out2 = out1;
            out1 = out;
        }
    }
}

impl Default for LowPassFilter {
    fn default() -> Self {
        Self::new(800.0, 1.0)
    }
}
