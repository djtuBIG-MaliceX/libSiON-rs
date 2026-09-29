//! Port of `effector/filters/si_controllable_filter_base.{h,cpp}`.
//!
//! `SiControllableFilterBase` extends `SiEffectBase` (NOT `SiFilterBase`):
//! it owns a cutoff/resonance envelope cursor pair walking
//! `SiMMLEnvelopeTable` data, converts each chunk's cutoff index through the
//! chip `filter_cutoff_table`/`filter_feedback_table`, and delegates the
//! per-chunk sample loop to the derived class `_process_lfo` override.
//! The Rust port embeds [`ControllableFilterBase`] in the two concrete
//! controllable filters and passes the override as a closure to
//! [`ControllableFilterBase::process`].
//!
//! Envelope cursors are modeled as [`EnvelopeCursor`] (an index into a
//! shared `Vec<i32>` snapshot of the table data); `None` is the C++
//! `nullptr`. The `SiMMLRefTable` envelope-table lookup tail of
//! `set_params` is not portable yet (wave-7) — see `docs/PENDING.md`.

use std::cell::RefCell;
use std::rc::Rc;

use crate::chip::ref_table;
use crate::effector::effect_base::{get_mml_arg, EffectSettings};
use crate::math::clampf;

/// Cursor into an envelope table's values; `None` where the C++ code holds a
/// null `SinglyLinkedList<int>::Element*`.
#[derive(Debug, Clone)]
pub struct EnvelopeCursor {
    pub list: Rc<RefCell<crate::utils::translator_util::SinglyLinkedList>>,
    pub index: usize,
}

impl EnvelopeCursor {
    pub fn value(&self) -> i32 {
        self.list.borrow().value_at(self.index)
    }

    /// Advances like `_ptr = _ptr->next()` (follows the list loop); returns
    /// `false` at end of list (the C++ cursor becomes `nullptr`).
    pub fn advance(&mut self) -> bool {
        match self.list.borrow().next_of(self.index) {
            Some(next) => {
                self.index = next;
                true
            }
            None => false,
        }
    }
}

/// The protected `_p0/_p1` biquad state shared by both controllable filters.
#[derive(Debug, Clone, Copy, Default)]
pub struct ControllableLfo {
    pub p0_right: f64,
    pub p1_right: f64,
    pub p0_left: f64,
    pub p1_left: f64,
}

/// C++ `table->get_head()` + cursor construction via `SiMMLRefTable::get_envelope_table`.
fn env_table_head(p_index: i32) -> Option<EnvelopeCursor> {
    let table = crate::sequencer::ref_table::instance()?
        .borrow()
        .get_envelope_table(p_index)?;
    let head = table.borrow().get_head()?;
    Some(EnvelopeCursor {
        list: table.borrow().data.clone()?,
        index: head,
    })
}
#[derive(Debug, Clone)]
pub struct ControllableFilterBase {
    pub settings: EffectSettings,
    pub lfo: ControllableLfo,
    pub cutoff_ptr: Option<EnvelopeCursor>,
    pub resonance_ptr: Option<EnvelopeCursor>,
    pub cutoff_index: i32,
    pub resonance: f64,
    pub lfo_step: i32,
    pub lfo_residue_step: i32,
}

impl ControllableFilterBase {
    pub fn new() -> Self {
        ControllableFilterBase {
            settings: EffectSettings::new(),
            lfo: ControllableLfo::default(),
            cutoff_ptr: None,
            resonance_ptr: None,
            cutoff_index: 0,
            resonance: 0.0,
            lfo_step: 0,
            lfo_residue_step: 0,
        }
    }

    pub fn set_params(&mut self, p_cutoff: i32, p_resonance: i32, p_fps: f64) {
        self.cutoff_ptr = None;
        if p_cutoff >= 0 && p_cutoff < 255 {
            if let Some(cursor) = env_table_head(p_cutoff) {
                self.cutoff_ptr = Some(cursor);
            }
        }

        self.resonance_ptr = None;
        if p_resonance >= 0 && p_resonance < 255 {
            if let Some(cursor) = env_table_head(p_resonance) {
                self.resonance_ptr = Some(cursor);
            }
        }

        self.cutoff_index = match &self.cutoff_ptr {
            Some(cursor) => cursor.value(),
            None => 128,
        };
        self.resonance = match &self.resonance_ptr {
            Some(cursor) => cursor.value() as f64 * 0.007751937984496124, // 0.007751937984496124 = 1/129
            None => 0.0,
        };

        self.lfo_step = (44100.0 / p_fps) as i32;
        if self.lfo_step <= 44 {
            self.lfo_step = 44;
        }
        self.lfo_residue_step = self.lfo_step << 1;
    }

    pub fn set_params_manually(&mut self, p_cutoff: f64, p_resonance: f64) {
        self.lfo_step = 2048;
        self.lfo_residue_step = 4096;
        self.set_cutoff(p_cutoff);
        self.set_resonance(p_resonance);
    }

    pub fn get_cutoff(&self) -> f64 {
        self.cutoff_index as f64 * 0.0078125
    }

    pub fn set_cutoff(&mut self, p_value: f64) {
        self.cutoff_index = clampf(p_value * 128.0, 0.0, 128.0) as i32;
    }

    pub fn get_resonance(&self) -> f64 {
        self.resonance
    }

    pub fn set_resonance(&mut self, p_value: f64) {
        self.resonance = clampf(p_value, 0.0, 1.0);
    }

    pub fn set_params_by_mml(&mut self, p_args: &[f64]) {
        let cutoff = get_mml_arg(p_args, 0, 255.0) as i32;
        let resonance = get_mml_arg(p_args, 1, 255.0) as i32;
        let fps = get_mml_arg(p_args, 2, 20.0);
        self.set_params(cutoff, resonance, fps);
    }

    pub fn reset_params(&mut self) {
        self.set_params(255, 255, 20.0);
    }

    pub fn prepare_process(&mut self) -> i32 {
        self.lfo_residue_step = 0;
        self.lfo.p0_left = 0.0;
        self.lfo.p1_left = 0.0;
        self.lfo.p0_right = 0.0;
        self.lfo.p1_right = 0.0;
        2
    }

    fn lookup_cutoff_feedback(&self) -> (f64, f64) {
        let table = ref_table::instance();
        let table = table.borrow();
        let cutoff = table.filter_cutoff_table[self.cutoff_index as usize];
        let feedback = self.resonance * table.filter_feedback_table[self.cutoff_index as usize];
        (cutoff, feedback)
    }

    pub fn process<F>(
        &mut self,
        p_channels: i32,
        r_buffer: &mut [f64],
        p_start_index: i32,
        p_length: i32,
        mut p_process_lfo: F,
    ) -> i32
    where
        F: FnMut(&mut ControllableLfo, f64, f64, &mut [f64], i32, i32),
    {
        let start_index = p_start_index << 1;
        let length = p_length << 1;

        let mut step = self.lfo_residue_step;
        let max = start_index + length;
        let mut i = start_index;
        while i < (max - step) {
            let (cutoff, feedback) = self.lookup_cutoff_feedback();
            p_process_lfo(&mut self.lfo, cutoff, feedback, r_buffer, i, step);

            if let Some(cursor) = &mut self.cutoff_ptr {
                if cursor.advance() {
                    self.cutoff_index = cursor.value();
                } else {
                    self.cutoff_ptr = None;
                    self.cutoff_index = 128;
                }
            }

            if let Some(cursor) = &mut self.resonance_ptr {
                if cursor.advance() {
                    self.resonance = cursor.value() as f64 * 0.007751937984496124;
                } else {
                    self.resonance_ptr = None;
                    self.resonance = 0.0;
                }
            }

            i += step;
            step = self.lfo_step << 1;
        }

        let (cutoff, feedback) = self.lookup_cutoff_feedback();
        p_process_lfo(&mut self.lfo, cutoff, feedback, r_buffer, i, max - i);
        self.lfo_residue_step = step - (max - i);

        p_channels
    }
}

impl Default for ControllableFilterBase {
    fn default() -> Self {
        Self::new()
    }
}


