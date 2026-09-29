//! Port of `libSiON-cpp/src/sequencer/base/mml_sequence_group.{h,cpp}` — a
//! group of MMLSequences. MMLData > MMLSequenceGroup > MMLSequence >
//! MMLEvent (">" means "has a").
//!
//! Godot `List<MMLSequence *>` iteration order (insertion order) is kept by
//! `Vec` / `VecDeque`. The sequences are `Rc<RefCell<...>>` handles, so
//! [`MMLSequence::clear`] keeps them reusable from the free list.

use std::collections::VecDeque;

use crate::sequencer::base::mml_event::MmlEventRef;
use crate::sequencer::base::mml_sequence::{MMLSequence, SeqRc};

// Golden-parity error macros; see the note in `mml_parser.rs` and
// `docs/PENDING-SEQ.md`.

/// C++ `ERR_FAIL_COND_V_MSG(cond, retval, msg)` — `$retval_text` is the
/// literal stringification (`#m_retval`) of the C++ return expression.
macro_rules! cpp_err_fail_cond_v_msg {
    ($cond:expr, $cond_text:expr, $retval:expr, $retval_text:expr, $msg:expr) => {
        if $cond {
            $crate::error::err_print_body(
                &format!(
                    "{}\nCondition \"{}\" is true. Returning: {}",
                    $msg, $cond_text, $retval_text
                ),
                false,
            );
            return $retval;
        }
    };
}

/// C++ `ERR_FAIL_INDEX_V(index, size, retval)` — the C++ macro never
/// mentions the return value in the message.
macro_rules! cpp_err_fail_index_v {
    ($idx:expr, $i_text:expr, $size:expr, $size_text:expr, $retval:expr) => {
        if $idx < 0 || $idx >= $size {
            $crate::error::err_print_body(
                &format!(
                    "Index {} = {} is out of bounds ({} = {}).",
                    $i_text, $idx, $size_text, $size
                ),
                false,
            );
            return $retval;
        }
    };
}

pub struct MMLSequenceGroup {
    free_list: VecDeque<SeqRc>,

    // Terminator.
    sequences: Vec<SeqRc>,
    term: SeqRc,
}

impl MMLSequenceGroup {
    pub fn create_new_sequence(&mut self) -> SeqRc {
        let sequence = match self.free_list.pop_front() {
            Some(sequence) => sequence,
            None => MMLSequence::new(false),
        };

        self.sequences.push(sequence.clone());
        sequence
    }

    pub fn append_new_sequence(&mut self) -> SeqRc {
        let sequence = self.create_new_sequence();
        MMLSequence::insert_before(&sequence, &self.term);
        sequence.borrow_mut().set_active(false);
        sequence
    }

    pub fn populate_sequences(&mut self, p_head_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = crate::sequencer::base::mml_parser::instance();
        let mut event = Some(p_head_event);
        loop {
            let Some(current) = event else { break };
            let jump = parser.borrow().events[current].get_jump();
            if jump.is_none() {
                break;
            }

            cpp_err_fail_cond_v_msg!(
                parser.borrow().events[current].get_id() != crate::sequencer::base::mml_event::SEQUENCE_HEAD,
                "event->get_id() != MMLEvent::SEQUENCE_HEAD",
                event,
                "event",
                format!(
                    "MMLSequenceGroup: Invalid event in the head event sequence ({}).",
                    parser.borrow().events[current].as_text()
                )
            );

            let sequence = self.append_new_sequence();
            event = MMLSequence::cutout(&sequence, current);
            MMLSequence::update_mml_string(&sequence);
            sequence.borrow_mut().set_active(true);
        }

        if let Some(current) = event {
            // This can happen normally, as we always add an extra head after
            // finishing previous sequence. But anything else is a problem
            // with data or a bug.
            cpp_err_fail_cond_v_msg!(
                parser.borrow().events[current].get_id() != crate::sequencer::base::mml_event::SEQUENCE_HEAD,
                "event->get_id() != MMLEvent::SEQUENCE_HEAD",
                event,
                "event",
                format!(
                    "MMLSequenceGroup: Invalid events at the end of the sequence (starting with {}).",
                    parser.borrow().events[current].as_text()
                )
            );
            let next = parser.borrow().events[current].get_next();
            cpp_err_fail_cond_v_msg!(
                next.is_some(),
                "event->get_next()",
                event,
                "event",
                format!(
                    "MMLSequenceGroup: Invalid events at the end of the sequence (starting with {}).",
                    next.map(|n| parser.borrow().events[n].as_text())
                        .unwrap_or_default()
                )
            );
        }

        // Return the remainder, if any, so the caller can decide what to do
        // with it.
        event
    }

    /// C++ `get_head_sequence()` — wrapped in an [`MMLSequence::Ref`]-shaped
    /// handle for [`super::mml_data::MMLData`].
    pub fn get_head_sequence(&self) -> Option<SeqRc> {
        MMLSequence::get_next_sequence(&self.term)
    }

    pub fn get_sequence(&self, p_index: i32) -> Option<SeqRc> {
        cpp_err_fail_index_v!(
            p_index,
            "p_index",
            self.sequences.len() as i32,
            "_sequences.size()",
            None
        );

        Some(self.sequences[p_index as usize].clone())
    }

    pub fn get_sequence_count(&self) -> i32 {
        self.sequences.len() as i32
    }

    //

    pub fn get_tick_count(&mut self) -> i32 {
        let mut tick_count: i32 = 0;

        let sequences = self.sequences.clone();
        for sequence in &sequences {
            let length = MMLSequence::get_event_length(sequence);
            if length > tick_count {
                tick_count = length;
            }
        }

        tick_count
    }

    pub fn has_repeat_all(&mut self) -> bool {
        let sequences = self.sequences.clone();
        for sequence in &sequences {
            if MMLSequence::has_repeat_all(sequence) {
                return true;
            }
        }
        false
    }

    //

    pub fn clear(&mut self) {
        let sequences = self.sequences.clone();
        for sequence in &sequences {
            MMLSequence::clear(sequence);
            self.free_list.push_back(sequence.clone());
        }
        self.sequences.clear();
        MMLSequence::clear(&self.term);
    }

    /// `MMLSequenceGroup()` — the constructor only builds the terminal.
    pub fn new() -> Self {
        Self {
            free_list: VecDeque::new(),
            sequences: Vec::new(),
            term: MMLSequence::new(true),
        }
    }
}

impl Default for MMLSequenceGroup {
    fn default() -> Self {
        Self::new()
    }
}
