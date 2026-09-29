//! Port of `libSiON-cpp/src/sequencer/base/mml_executor.{h,cpp}` — an
//! MMLSequence plus executing pointer. Each track has one executor, and the
//! sequencer also has one for the global sequence.
//!
//! `MMLSequence *` becomes [`SeqRc`]; `MMLEvent *` becomes arena indices
//! (see [`crate::sequencer::base::mml_parser`]); the `SinglyLinkedList<int>`
//! repeat-counter stack becomes a [`VecDeque`] with strictly preserved
//! push/pop order.

use std::collections::VecDeque;

use crate::sequencer::base::mml_event::{self, MmlEventRef};
use crate::sequencer::base::mml_parser;
use crate::sequencer::base::mml_sequence::SeqRc;

pub struct MMLExecutor {
    sequence: Option<SeqRc>,
    pointer: Option<MmlEventRef>,

    repeat_end_counter: i32,
    repeat_point: Option<MmlEventRef>,

    nop_event: MmlEventRef,
    process_event: MmlEventRef,
    bend_from_event: MmlEventRef,
    bend_event: MmlEventRef,
    note_event: MmlEventRef,

    current_tick_count: i32,
    // Stack of counters.
    repeat_counters: VecDeque<i32>,
    residue_sample_count: i32,
    decimal_fraction_sample_count: i32,
}

impl MMLExecutor {
    pub fn get_nop_event(&self) -> MmlEventRef {
        self.nop_event
    }

    pub fn get_sequence(&self) -> Option<SeqRc> {
        self.sequence.clone()
    }

    pub fn get_pointer(&self) -> Option<MmlEventRef> {
        self.pointer
    }

    pub fn set_pointer(&mut self, p_event: Option<MmlEventRef>) {
        self.pointer = p_event;
    }

    pub fn get_current_event(&self) -> Option<MmlEventRef> {
        if self.pointer == Some(self.process_event) {
            let parser = mml_parser::instance();
            parser.borrow().events[self.process_event].get_jump()
        } else {
            self.pointer
        }
    }

    pub fn get_repeat_end_counter(&self) -> i32 {
        self.repeat_end_counter
    }

    /// Note that's awaiting "note on" execution, or -1.
    pub fn get_waiting_note(&self) -> i32 {
        if self.pointer == Some(self.note_event) {
            let parser = mml_parser::instance();
            parser.borrow().events[self.note_event].get_data()
        } else {
            -1
        }
    }

    pub fn get_current_tick_count(&self) -> i32 {
        self.current_tick_count
    }

    pub fn get_residue_sample_count(&self) -> i32 {
        self.residue_sample_count
    }

    pub fn set_residue_sample_count(&mut self, p_value: i32) {
        self.residue_sample_count = p_value;
    }

    pub fn adjust_residue_sample_count(&mut self, p_diff: i32) {
        // C++ int wrap-around.
        self.residue_sample_count = self.residue_sample_count.wrapping_add(p_diff);
    }

    pub fn get_decimal_fraction_sample_count(&self) -> i32 {
        self.decimal_fraction_sample_count
    }

    pub fn set_decimal_fraction_sample_count(&mut self, p_value: i32) {
        self.decimal_fraction_sample_count = p_value;
    }

    // Execution.

    pub fn reset_pointer(&mut self) {
        let Some(sequence) = &self.sequence else {
            return;
        };

        let head = {
            let s = sequence.borrow();
            s.get_head_event().expect("MMLExecutor: sequence head is null")
        };
        let parser = mml_parser::instance();
        self.pointer = parser.borrow().events[head].get_next();

        self.repeat_end_counter = 0;
        self.repeat_point = None;

        self.repeat_counters.clear();
        self.current_tick_count = 0;
        self.residue_sample_count = 0;
        self.decimal_fraction_sample_count = 0;
    }

    pub fn stop(&mut self) {
        if self.pointer.is_none() {
            return;
        }

        if self.pointer == Some(self.process_event) {
            let parser = mml_parser::instance();
            parser.borrow_mut().events[self.process_event].set_jump(Some(self.nop_event));
        } else {
            self.pointer = None;
        }
    }

    pub fn execute_single_note(&mut self, p_note: i32, p_tick_length: i32) {
        {
            let parser = mml_parser::instance();
            let mut p = parser.borrow_mut();
            p.events[self.note_event].set_next(None);
            p.events[self.note_event].set_data(p_note);
            p.events[self.note_event].set_length(p_tick_length);
        }
        self.pointer = Some(self.note_event);

        self.sequence = None;

        self.repeat_end_counter = 0;
        self.repeat_point = None;

        self.repeat_counters.clear();
        self.current_tick_count = 0;
    }

    pub fn bend_single_note(&mut self, p_to_note: i32, p_tick_length: i32) {
        if self.pointer != Some(self.note_event) || p_tick_length == 0 {
            return;
        }

        let parser = mml_parser::instance();
        let mut p = parser.borrow_mut();

        let mut tick_length = p_tick_length;
        let note_length = p.events[self.note_event].get_length();
        if note_length > 0 {
            // Bending cannot last longer than the note. But the trailing
            // note must play for at least 1 tick.
            if tick_length > note_length {
                tick_length = note_length - 1;
            }
            p.events[self.note_event].set_length(note_length - tick_length);
        }

        let note_data = p.events[self.note_event].get_data();
        p.events[self.bend_from_event].set_length(0);
        p.events[self.bend_from_event].set_data(note_data);
        p.events[self.bend_event].set_length(tick_length);
        p.events[self.note_event].set_data(p_to_note);
        self.pointer = Some(self.bend_from_event);
    }

    pub fn publish_processing_event(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let length = parser.borrow().events[p_event].get_length();
        if length > 0 {
            self.current_tick_count += length;

            let mut p = parser.borrow_mut();
            p.events[self.process_event].set_length(length);
            p.events[self.process_event].set_jump(Some(p_event));

            return Some(self.process_event);
        }

        parser.borrow().events[p_event].get_next()
    }

    // Handlers.

    pub fn on_tempo_changed(&mut self, p_changing_ratio: f64) {
        let mut ratio = p_changing_ratio;
        if self.residue_sample_count < 0 {
            ratio = 1.0 / ratio;
        }

        self.residue_sample_count = (self.residue_sample_count as f64 * ratio) as i32;
        self.decimal_fraction_sample_count =
            (self.decimal_fraction_sample_count as f64 * ratio) as i32;
    }

    pub fn on_repeat_all(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let next = parser.borrow().events[p_event].get_next();

        self.repeat_point = next;

        next
    }

    pub fn on_repeat_begin(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (data, next) = {
            let p = parser.borrow();
            (p.events[p_event].get_data(), p.events[p_event].get_next())
        };
        self.repeat_counters.push_front(data);

        next
    }

    pub fn on_repeat_break(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        if self.repeat_counters.front().expect("MMLExecutor: repeat counters stack is empty") == &1
        {
            self.repeat_counters.pop_front();

            // Jump back to the repeat start, then to the repeat end, and
            // exit the loop.
            let parser = mml_parser::instance();
            let p = parser.borrow();
            let jump1 = p.events[p_event].get_jump().expect("MMLExecutor: repeat break jump is null");
            let jump2 = p.events[jump1].get_jump().expect("MMLExecutor: repeat break jump is null");
            return p.events[jump2].get_next();
        }

        let parser = mml_parser::instance();
        parser.borrow().events[p_event].get_next()
    }

    pub fn on_repeat_end(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        {
            let top = self
                .repeat_counters
                .front_mut()
                .expect("MMLExecutor: repeat counters stack is empty");
            // C++ int wrap-around.
            *top = top.wrapping_sub(1);
        }
        if self.repeat_counters.front().expect("stack") == &0 {
            self.repeat_counters.pop_front();

            // Exit the repeat loop.
            let parser = mml_parser::instance();
            return parser.borrow().events[p_event].get_next();
        }

        // Jump back to the repeat start.
        let parser = mml_parser::instance();
        let p = parser.borrow();
        let jump = p.events[p_event].get_jump().expect("MMLExecutor: repeat end jump is null");
        p.events[jump].get_next()
    }

    pub fn on_sequence_tail(&mut self, _p_event: MmlEventRef) -> Option<MmlEventRef> {
        self.repeat_end_counter += 1;

        self.repeat_point
    }

    //

    pub fn initialize(&mut self, p_sequence: Option<SeqRc>) {
        self.clear();

        if let Some(p_sequence) = p_sequence {
            let head = {
                let s = p_sequence.borrow();
                s.get_head_event().expect("MMLExecutor: sequence head is null")
            };
            let parser = mml_parser::instance();
            let next = parser.borrow().events[head].get_next();
            self.sequence = Some(p_sequence);
            self.pointer = next;
        }
    }

    pub fn clear(&mut self) {
        self.sequence = None;
        self.pointer = None;

        self.repeat_end_counter = 0;
        self.repeat_point = None;

        self.repeat_counters.clear();
        self.current_tick_count = 0;
        self.residue_sample_count = 0;
        self.decimal_fraction_sample_count = 0;
    }

    /// `MMLExecutor()`.
    pub fn new() -> Self {
        let parser = mml_parser::instance();
        let mut p = parser.borrow_mut();
        let nop_event = p.alloc_event(mml_event::NO_OP, 0, 0);
        let process_event = p.alloc_event(mml_event::PROCESS, 0, 0);
        let note_event = p.alloc_event(mml_event::DRIVER_NOTE, 0, 0);
        let bend_from_event = p.alloc_event(mml_event::NOTE, 0, 0);
        let bend_event = p.alloc_event(mml_event::PITCHBEND, 0, 0);

        p.events[bend_from_event].set_next(Some(bend_event));
        p.events[bend_event].set_next(Some(note_event));
        drop(p);

        Self {
            sequence: None,
            pointer: None,
            repeat_end_counter: 0,
            repeat_point: None,
            nop_event,
            process_event,
            bend_from_event,
            bend_event,
            note_event,
            current_tick_count: 0,
            repeat_counters: VecDeque::new(),
            residue_sample_count: 0,
            decimal_fraction_sample_count: 0,
        }
    }

    /// `~MMLExecutor()` — Rust has no deterministic destructor hook for the
    /// shared parser arena; the driver wave must call this before dropping
    /// (see `docs/PENDING-SEQ.md`).
    pub fn finalize(&mut self) {
        let parser = mml_parser::instance();
        parser.borrow_mut().free_event(self.nop_event);
        parser.borrow_mut().free_event(self.process_event);
        parser.borrow_mut().free_event(self.note_event);
        parser.borrow_mut().free_event(self.bend_from_event);
        parser.borrow_mut().free_event(self.bend_event);
    }
}

impl Default for MMLExecutor {
    fn default() -> Self {
        Self::new()
    }
}
