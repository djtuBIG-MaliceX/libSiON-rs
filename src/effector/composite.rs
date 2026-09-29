//! Port of `effector/si_effect_composite.{h,cpp}`.
//!
//! `SiEffectComposite` is an `SiEffectBase` that hosts up to `SLOTS_MAX`
//! effect chains: slot 0 processes the main buffer in place, slots 1..8
//! each get a private send buffer (input `* send_level`, output mixed back
//! `+ * mix_level`). `Ref<SiEffectBase>` chain members map to
//! `Rc<RefCell<dyn EffectBase>>` per the shared/mutable ownership rule.

use std::cell::RefCell;
use std::rc::Rc;

use crate::effector::effect_base::{EffectBase, EffectSettings};

#[derive(Clone)]
pub struct SlottedEffect {
    pub effects: Vec<Rc<RefCell<dyn EffectBase>>>,
    pub buffer: Vec<f64>,
    pub send_level: f64,
    pub mix_level: f64,
}

impl SlottedEffect {
    pub fn new() -> SlottedEffect {
        SlottedEffect {
            effects: Vec::new(),
            buffer: Vec::new(),
            send_level: 1.0,
            mix_level: 1.0,
        }
    }
}

impl Default for SlottedEffect {
    fn default() -> Self {
        Self::new()
    }
}

pub const SLOTS_MAX: usize = 8;

pub struct EffectComposite {
    pub settings: EffectSettings,
    pub slots: [SlottedEffect; SLOTS_MAX],
}

impl EffectComposite {
    pub fn new() -> Self {
        EffectComposite {
            settings: EffectSettings::new(),
            slots: std::array::from_fn(|_| SlottedEffect::new()),
        }
    }

    pub fn set_slot_effects(&mut self, p_slot: usize, p_effects: Vec<Rc<RefCell<dyn EffectBase>>>) {
        crate::err_fail_index!(p_slot as i32, "p_slot", SLOTS_MAX as i32, "SLOTS_MAX");
        self.slots[p_slot].effects = p_effects;
    }

    pub fn set_slot_levels(&mut self, p_slot: usize, p_send_level: f64, p_mix_level: f64) {
        crate::err_fail_index!(p_slot as i32, "p_slot", SLOTS_MAX as i32, "SLOTS_MAX");
        self.slots[p_slot].send_level = p_send_level;
        self.slots[p_slot].mix_level = p_mix_level;
    }
}

impl Default for EffectComposite {
    fn default() -> Self {
        Self::new()
    }
}

impl EffectBase for EffectComposite {
    fn settings(&self) -> &EffectSettings {
        &self.settings
    }

    fn settings_mut(&mut self) -> &mut EffectSettings {
        &mut self.settings
    }

    fn prepare_process(&mut self) -> i32 {
        for i in 0..SLOTS_MAX {
            for effect in self.slots[i].effects.iter() {
                effect.borrow_mut().prepare_process();
            }
        }
        2
    }

    fn process(
        &mut self,
        p_channels: i32,
        r_buffer: &mut [f64],
        p_start_index: i32,
        p_length: i32,
    ) -> i32 {
        let start = p_start_index as usize;
        let end = (p_start_index + p_length) as usize;

        for i in 1..SLOTS_MAX {
            if self.slots[i].effects.is_empty() {
                continue;
            }
            let send_level = self.slots[i].send_level;
            if self.slots[i].buffer.len() < r_buffer.len() {
                self.slots[i].buffer.resize(r_buffer.len(), 0.0);
            }
            for j in start..end {
                self.slots[i].buffer[j] = r_buffer[j] * send_level;
            }
        }

        let send_level = self.slots[0].send_level;
        for j in start..end {
            r_buffer[j] *= send_level;
        }

        for i in 1..SLOTS_MAX {
            if self.slots[i].effects.is_empty() {
                continue;
            }
            let mut channel_num = p_channels;
            let effects = self.slots[i].effects.clone();
            for effect in effects.iter() {
                channel_num =
                    effect
                        .borrow_mut()
                        .process(channel_num, &mut self.slots[i].buffer, p_start_index, p_length);
            }
            let mix_level = self.slots[i].mix_level;
            for j in start..end {
                let value = self.slots[i].buffer[j];
                r_buffer[j] += value * mix_level;
            }
        }

        let mut out_channels = p_channels;
        if !self.slots[0].effects.is_empty() {
            let effects = self.slots[0].effects.clone();
            for effect in effects.iter() {
                out_channels =
                    effect
                        .borrow_mut()
                        .process(out_channels, r_buffer, p_start_index, p_length);
            }

            if self.slots[0].mix_level != 1.0 {
                let mix_level = self.slots[0].mix_level;
                for j in start..end {
                    r_buffer[j] *= mix_level;
                }
            }
        }

        out_channels
    }

    fn reset(&mut self) {
        for i in 0..SLOTS_MAX {
            self.slots[i].effects.clear();
            self.slots[i].buffer.clear();
            self.slots[i].send_level = 1.0;
            self.slots[i].mix_level = 1.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    struct Recorder {
        settings: EffectSettings,
        log: Rc<RefCell<Vec<&'static str>>>,
        tag: &'static str,
    }

    impl EffectBase for Recorder {
        fn settings(&self) -> &EffectSettings {
            &self.settings
        }

        fn settings_mut(&mut self) -> &mut EffectSettings {
            &mut self.settings
        }

        fn process(
            &mut self,
            p_channels: i32,
            r_buffer: &mut [f64],
            p_start_index: i32,
            p_length: i32,
        ) -> i32 {
            self.log.borrow_mut().push(self.tag);
            for j in (p_start_index as usize)..((p_start_index + p_length) as usize) {
                r_buffer[j] = 1.0;
            }
            p_channels
        }

        fn prepare_process(&mut self) -> i32 {
            self.log.borrow_mut().push(self.tag);
            2
        }
    }

    fn recorder(
        tag: &'static str,
        log: &Rc<RefCell<Vec<&'static str>>>,
    ) -> Rc<RefCell<dyn EffectBase>> {
        Rc::new(RefCell::new(Recorder {
            settings: EffectSettings::new(),
            log: log.clone(),
            tag,
        }))
    }

    #[test]
    fn slot_chain_refcounts_and_orders() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let a = recorder("a", &log);
        let b = recorder("b", &log);

        let a_count_in = Rc::strong_count(&a);
        let mut composite = EffectComposite::new();
        composite.set_slot_effects(1, vec![a.clone(), b.clone()]);

        assert_eq!(composite.slots[1].effects.len(), 2);
        assert_eq!(Rc::strong_count(&a), a_count_in + 1);

        composite.prepare_process();
        assert_eq!(*log.borrow(), vec!["a", "b"]);

        let mut buffer = vec![10.0, 10.0];
        composite.set_slot_levels(1, 2.0, 0.5);
        composite.process(2, &mut buffer, 0, 1);

        // slot1 buffer = 10*2, chain overwrites with 1.0, mixed back at 0.5.
        assert_eq!(buffer[0], 10.0 * 1.0 + 1.0 * 0.5);
        assert_eq!(*log.borrow(), vec!["a", "b", "a", "b"]);

        composite.reset();
        assert!(composite.slots[1].effects.is_empty());
        assert_eq!(Rc::strong_count(&a), a_count_in);
        assert!(composite.slots[1].buffer.is_empty());
        assert_eq!(composite.slots[1].send_level, 1.0);
        assert_eq!(composite.slots[1].mix_level, 1.0);
    }

    #[test]
    fn main_slot_processes_after_send_slots() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut composite = EffectComposite::new();
        composite.set_slot_effects(0, vec![recorder("main", &log)]);
        composite.set_slot_effects(1, vec![recorder("side", &log)]);

        let mut buffer = vec![4.0, 4.0];
        composite.process(2, &mut buffer, 0, 1);

        assert_eq!(*log.borrow(), vec!["side", "main"]);
        assert_eq!(buffer[0], 1.0);
    }
}
