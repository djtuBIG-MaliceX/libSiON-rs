//! Port of `libSiON-cpp/src/utils/fader_util.{h,cpp}`.
//!
//! The C++ `std::function<void(double)>` callback becomes a boxed `Fn(f64)`;
//! the C++ ctor defaults are folded into [`FaderUtil::new`]'s explicit
//! parameters (no default arguments in Rust).

/// C++ `FaderUtil`.
pub struct FaderUtil {
    end: f64,
    step: f64,
    counter: i32,
    value: f64,

    callback: Option<Box<dyn Fn(f64)>>,
}

impl FaderUtil {
    /// C++ `is_active()`.
    pub fn is_active(&self) -> bool {
        self.counter > 0
    }

    /// C++ `is_incrementing()`.
    pub fn is_incrementing(&self) -> bool {
        self.step > 0.0
    }

    /// C++ `get_value()`.
    pub fn get_value(&self) -> f64 {
        self.value
    }

    /// C++ `set_callback(const std::function<void(double)> &)`.
    pub fn set_callback(&mut self, p_callback: Option<Box<dyn Fn(f64)>>) {
        self.callback = p_callback;
    }

    /// C++ `execute()` — returns true when the end value has been reached.
    pub fn execute(&mut self) -> bool {
        if self.counter > 0 {
            self.value += self.step;
            self.counter -= 1;

            if self.counter == 0 {
                self.value = self.end; // Ensure there is no imprecision.
                if let Some(callback) = &self.callback {
                    callback(self.value);
                }

                return true;
            } else if let Some(callback) = &self.callback {
                callback(self.value);
            }
        }

        false
    }

    /// C++ `stop()`.
    pub fn stop(&mut self) {
        self.counter = 0;
    }

    /// C++ `set_fade(p_value_from = 0, p_value_to = 1, p_frames = 60)`.
    pub fn set_fade(&mut self, p_value_from: f64, p_value_to: f64, p_frames: i32) {
        self.value = p_value_from;

        if p_frames == 0 || self.callback.is_none() {
            self.counter = 0;
            return;
        }

        self.end = p_value_to;
        self.step = (p_value_to - p_value_from) / p_frames as f64;
        self.counter = p_frames;
        if let Some(callback) = &self.callback {
            callback(self.value);
        }
    }

    /// Port seam for `SiONDriver` (state lives on the owner, not in a
    /// captured callback): like [`set_fade`](Self::set_fade) but computes
    /// the ramp even with no callback stored. The owner replicates the
    /// C++ `set_fade` callback firing itself.
    pub fn compute_fade(&mut self, p_value_from: f64, p_value_to: f64, p_frames: i32) {
        self.value = p_value_from;
        if p_frames == 0 {
            self.counter = 0;
            return;
        }
        self.end = p_value_to;
        self.step = (p_value_to - p_value_from) / p_frames as f64;
        self.counter = p_frames;
    }

    /// Port seam: [`execute`](Self::execute) without the callback —
    /// returns `(completed, Some(new_value))` when a step was taken,
    /// `(false, None)` when idle (identical control flow).
    pub fn execute_compute(&mut self) -> (bool, Option<f64>) {
        if self.counter > 0 {
            self.value += self.step;
            self.counter -= 1;

            if self.counter == 0 {
                self.value = self.end; // Ensure there is no imprecision.
                return (true, Some(self.value));
            }
            return (false, Some(self.value));
        }
        (false, None)
    }

    /// C++ `FaderUtil(p_callback = nullptr, p_value_from = 0, p_value_to = 1, p_frames = 60)`.
    pub fn new(
        p_callback: Option<Box<dyn Fn(f64)>>,
        p_value_from: f64,
        p_value_to: f64,
        p_frames: i32,
    ) -> Self {
        let mut fader = FaderUtil {
            end: 0.0,
            step: 0.0,
            counter: 0,
            value: 0.0,
            callback: None,
        };
        fader.set_callback(p_callback);
        fader.set_fade(p_value_from, p_value_to, p_frames);
        fader
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn linear_ramp_lands_exactly_on_the_end_value() {
        let seen = Rc::new(RefCell::new(Vec::<f64>::new()));
        let sink = seen.clone();

        let mut fader = FaderUtil::new(
            Some(Box::new(move |v| sink.borrow_mut().push(v))),
            0.0,
            1.0,
            60,
        );
        assert!(fader.is_active());
        assert!(fader.is_incrementing());
        assert_eq!(fader.get_value(), 0.0);

        // set_fade fires the callback once with the start value.
        assert_eq!(&*seen.borrow(), &[0.0]);

        let mut finished_at = -1i32;
        for i in 0..65 {
            if fader.execute() {
                finished_at = i;
                break;
            }
        }

        assert_eq!(finished_at, 59);
        assert_eq!(fader.get_value(), 1.0); // Exact end value, no imprecision.
        assert!(!fader.is_active());
        assert!(!fader.execute()); // Inactive fades do nothing.

        // 60 ramp callbacks plus the initial one.
        assert_eq!(seen.borrow().len(), 61);

        // Monotonic linear ramp invariant over the stepping range.
        let values = seen.borrow();
        for i in 1..values.len() {
            assert!(values[i] >= values[i - 1]);
        }
    }

    #[test]
    fn stop_and_frameless_fade_short_circuit() {
        let seen = Rc::new(RefCell::new(0u32));
        let sink = seen.clone();

        let mut fader = FaderUtil::new(
            Some(Box::new(move |_v| *sink.borrow_mut() += 1)),
            0.0,
            1.0,
            10,
        );
        fader.stop();
        assert!(!fader.is_active());
        assert!(!fader.execute());
        assert_eq!(*seen.borrow(), 1); // Only the initial set_fade callback.

        // p_frames == 0 kills the fade and fires no callback.
        fader.set_fade(1.0, 0.0, 0);
        assert_eq!(*seen.borrow(), 1);
        assert_eq!(fader.get_value(), 1.0);

        // No callback at all: the fade is dropped.
        let mut silent = FaderUtil::new(None, 0.0, 1.0, 30);
        assert!(!silent.is_active());
        silent.set_fade(0.0, 0.5, 30);
        assert!(!silent.is_active());
    }
}
