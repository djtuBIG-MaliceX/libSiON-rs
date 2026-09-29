//! Port of `libSiON-cpp/src/sequencer/base/mml_executor_connector.{h,cpp}`
//! — used for #FM connection.
//!
//! The `MECElement` tree nodes live in a process-wide arena because the C++
//! `_free_list` is a *static* pool shared by every connector instance.
//!
//! TODO(port): `SiOPMChannelBase::OUTPUT_*` comes from
//! `chip/channels/siopm_channel_base.{h,cpp}` (wave owned by another
//! worker). The numeric values are hard-coded below; see
//! `docs/PENDING-SEQ.md`.

use std::cell::RefCell;

use crate::sequencer::base::mml_event;
use crate::sequencer::base::mml_parser;
use crate::sequencer::base::mml_sequence::{MMLSequence, SeqRc};
use crate::sequencer::base::mml_sequence_group::MMLSequenceGroup;
use crate::utils::string;

// `SiOPMChannelBase::OutputMode` values.
const OUTPUT_STANDARD: i32 = 0; // Standard output.
const OUTPUT_OVERWRITE: i32 = 1; // Overwrite output pipe.
const OUTPUT_ADD: i32 = 2; // Add to output pipe.

// Golden-parity error macros; see the note in `mml_parser.rs` and
// `docs/PENDING-SEQ.md`.

/// C++ `ERR_FAIL_COND_V_MSG(cond, retval, msg)`.
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

/// C++ `ERR_FAIL_INDEX_MSG(index, size, msg)` — the C++ macro never
/// mentions the return value in the message.
macro_rules! cpp_err_fail_index_msg {
    ($idx:expr, $i_text:expr, $size:expr, $size_text:expr, $msg:expr) => {
        if $idx < 0 || $idx >= $size {
            $crate::error::err_print_body(
                &format!(
                    "{}\nIndex {} = {} is out of bounds ({} = {}).",
                    $msg, $i_text, $idx, $size_text, $size
                ),
                false,
            );
            return;
        }
    };
}

/// `MECElement` — arena node.
struct MecElement {
    number: i32,
    modulation: i32,

    parent: Option<usize>,
    next: Option<usize>,
    first_child: Option<usize>,
}

impl MecElement {
    /// `MECElement::initialize(p_number)`.
    fn initialize(&mut self, p_number: i32) {
        self.number = p_number;
        self.modulation = 3;

        self.parent = None;
        self.next = None;
        self.first_child = None;
    }
}

/// `static List<MECElement *> _free_list` plus the arena backing it.
struct MecPool {
    elements: Vec<MecElement>,
    free_list: Vec<usize>,
}

thread_local! {
    static MEC_POOL: RefCell<MecPool> = const { RefCell::new(MecPool { elements: Vec::new(), free_list: Vec::new() }) };
}

impl MecPool {
    /// `_free_element()` — depth-first release of the child/next subtree,
    /// then the node itself, matching the C++ push order.
    fn free_element(&mut self, p_element: usize) {
        let (first_child, next) = {
            let e = &self.elements[p_element];
            (e.first_child, e.next)
        };
        if let Some(first_child) = first_child {
            self.free_element(first_child);
        }
        if let Some(next) = next {
            self.free_element(next);
        }

        self.free_list.push(p_element);
    }

    /// `_alloc_element()`.
    fn alloc_element(&mut self, p_number: i32) -> usize {
        let element = match self.free_list.pop() {
            Some(element) => element,
            None => {
                self.elements.push(MecElement {
                    number: 0,
                    modulation: 3,
                    parent: None,
                    next: None,
                    first_child: None,
                });
                self.elements.len() - 1
            }
        };

        self.elements[element].initialize(p_number);
        element
    }
}

pub struct MMLExecutorConnector {
    first_element: Option<usize>,
    executor_count: i32,
    sequence_count: i32,

    // C++ held these as raw pointers valid only during `connect()`; the raw
    // pointer member keeps that lifetime model (single-threaded use only).
    connecting_sequence_group: Option<*mut MMLSequenceGroup>,
    connecting_sequence: Option<SeqRc>,
    connecting_sequence_list: Vec<SeqRc>,
}

impl MMLExecutorConnector {
    pub fn get_executor_count(&self) -> i32 {
        self.executor_count
    }

    pub fn get_sequence_count(&self) -> i32 {
        self.sequence_count
    }

    /// `_connect()`.
    fn connect_recursive(&mut self, p_element: usize, p_first_oscillator: bool, p_out_pipe: i32) {
        // Modulator before carrior.
        let (first_child, modulation, number, next) = MEC_POOL
            .with(|pool| {
                let pool = pool.borrow();
                let e = &pool.elements[p_element];
                (e.first_child, e.modulation, e.number, e.next)
            });
        let mut in_pipe = 0;
        if let Some(first_child) = first_child {
            in_pipe = p_out_pipe + i32::from(!p_first_oscillator);
            self.connect_recursive(first_child, true, in_pipe);
        }

        // Preprocess and assign sequence.
        let group_ptr = self
            .connecting_sequence_group
            .expect("MMLExecutorConnector: connecting sequence group is null");
        // SAFETY: single-threaded; the group outlives `connect()` in C++ too.
        let prep_sequence = unsafe { (*group_ptr).create_new_sequence() };
        MMLSequence::initialize(&prep_sequence);

        // Out pipe.
        if p_out_pipe != -1 {
            let mode = if p_first_oscillator { OUTPUT_OVERWRITE } else { OUTPUT_ADD };
            MMLSequence::append_new_event(&prep_sequence, mml_event::OUTPUT_PIPE, mode, 0);
            MMLSequence::append_new_event(&prep_sequence, mml_event::PARAMETER, p_out_pipe, 0);
        } else {
            MMLSequence::append_new_event(&prep_sequence, mml_event::OUTPUT_PIPE, OUTPUT_STANDARD, 0);
            MMLSequence::append_new_event(&prep_sequence, mml_event::PARAMETER, 0, 0);
        }

        // In pipe.
        if first_child.is_some() {
            MMLSequence::append_new_event(&prep_sequence, mml_event::INPUT_PIPE, modulation, 0);
            MMLSequence::append_new_event(&prep_sequence, mml_event::PARAMETER, in_pipe, 0);
        } else {
            MMLSequence::append_new_event(&prep_sequence, mml_event::INPUT_PIPE, 0, 0);
            MMLSequence::append_new_event(&prep_sequence, mml_event::PARAMETER, 0, 0);
        }

        let target = self.connecting_sequence_list[number as usize].clone();
        let head_next = {
            let head = target
                .borrow()
                .get_head_event()
                .expect("MMLExecutorConnector: sequence head is null");
            let parser = mml_parser::instance();
            parser.borrow().events[head].get_next()
        };
        MMLSequence::connect_before(&prep_sequence, head_next);
        let connecting = self
            .connecting_sequence
            .clone()
            .expect("MMLExecutorConnector: connecting sequence is null");
        MMLSequence::insert_after(&prep_sequence, &connecting);
        self.connecting_sequence = Some(prep_sequence);

        // Move to the next oscillator.
        if let Some(next) = next {
            self.connect_recursive(next, false, p_out_pipe);
        }
    }

    pub fn connect(
        &mut self,
        p_seq_group: &mut MMLSequenceGroup,
        p_sequence: SeqRc,
    ) -> Option<SeqRc> {
        self.connecting_sequence_group = Some(p_seq_group as *mut _);
        self.connecting_sequence = Some(p_sequence);
        self.connecting_sequence_list.clear();

        for _ in 0..self.sequence_count {
            let connecting = self
                .connecting_sequence
                .clone()
                .expect("MMLExecutorConnector: connecting sequence is null");
            let next = MMLSequence::get_next_sequence(&connecting);
            cpp_err_fail_cond_v_msg!(
                next.is_none(),
                "!_connecting_sequence->get_next_sequence()",
                None,
                "nullptr",
                "MMLExecutorConnector: Not enough sequences to connect."
            );

            let next = next.expect("checked above");
            self.connecting_sequence_list.push(next.clone());
            MMLSequence::remove_from_chain(&next);
        }

        let first = self
            .first_element
            .expect("MMLExecutorConnector: no formula was parsed");
        self.connect_recursive(first, false, -1);
        self.connecting_sequence.clone()
    }

    pub fn parse(&mut self, p_formula: &str) {
        self.clear();

        let mut last_elem: Option<usize> = None;

        let re_formula = regex::Regex::new(r"(\()?([a-zA-Z])([0-7])?(\)+)?")
            .expect("MMLExecutorConnector: static formula regex");
        // `search_all` from `compat/sion_regex.cpp`; this pattern always
        // consumes at least the [a-zA-Z] group, so no empty matches occur.
        let mut offset = 0usize;
        while offset <= p_formula.len() {
            let Some(caps) = re_formula.captures_at(p_formula, offset) else {
                break;
            };
            let whole = caps.get(0).expect("whole match");
            offset = if whole.start() == whole.end() {
                whole.end() + 1
            } else {
                whole.end()
            };

            let group = |i: usize| -> String {
                caps.get(i).map(|m| m.as_str().to_string()).unwrap_or_default()
            };

            // We want to have a 0-based index for letters from A to Z.
            let osc_key = group(2);
            let osc_idx = string::unicode_at(&string::to_lower(&osc_key), 0) - i32::from(b'a');
            cpp_err_fail_index_msg!(
                osc_idx,
                "osc_idx",
                26,
                "26",
                format!(
                    "MMLExecutorConnector: Invalid oscillator key '{osc_key}' in formula: '{p_formula}'"
                )
            );

            if self.sequence_count <= osc_idx {
                self.sequence_count = osc_idx + 1;
            }
            self.executor_count += 1;

            let mut elem = MEC_POOL
                .with(|pool| pool.borrow_mut().alloc_element(osc_idx));

            let modulation_src = group(3);
            MEC_POOL.with(|pool| {
                let mut pool = pool.borrow_mut();
                if !modulation_src.is_empty() {
                    pool.elements[elem].modulation = string::to_int(&modulation_src) as i32;
                } else {
                    pool.elements[elem].modulation = 5;
                }
            });

            // Modulation start "(".
            if !group(1).is_empty() {
                let Some(last_elem) = last_elem else {
                    crate::error::err_print_body(
                        &format!(
                            "MMLExecutorConnector: Invalid modulation start '(' in formula: '{p_formula}'\nCondition \"!last_elem\" is true."
                        ),
                        false,
                    );
                    return;
                };
                MEC_POOL.with(|pool| {
                    let mut pool = pool.borrow_mut();
                    pool.elements[last_elem].first_child = Some(elem);
                    pool.elements[elem].parent = Some(last_elem);
                });
            } else if let Some(last_elem) = last_elem {
                MEC_POOL.with(|pool| {
                    let mut pool = pool.borrow_mut();
                    let parent = pool.elements[last_elem].parent;
                    pool.elements[last_elem].next = Some(elem);
                    pool.elements[elem].parent = parent;
                });
            } else {
                self.first_element = Some(elem);
            }

            // Modulation end ")+".
            let end_string = group(4);
            if !end_string.is_empty() {
                for _ in 0..end_string.chars().count() {
                    let parent = MEC_POOL.with(|pool| pool.borrow().elements[elem].parent);
                    let Some(parent) = parent else {
                        crate::error::err_print_body(
                            &format!(
                                "MMLExecutorConnector: Invalid modulation end ')' in formula: '{p_formula}'\nCondition \"!elem->parent\" is true."
                            ),
                            false,
                        );
                        return;
                    };
                    elem = parent;
                }
            }

            last_elem = Some(elem);
        }

        let invalid = match last_elem {
            None => true,
            Some(last_elem) => MEC_POOL
                .with(|pool| pool.borrow().elements[last_elem].parent)
                .is_some(),
        };
        if invalid {
            crate::error::err_print_body(
                &format!(
                    "MMLExecutorConnector: Invalid formula: '{p_formula}'\nCondition \"!last_elem || last_elem->parent\" is true."
                ),
                false,
            );
            return;
        }
    }

    pub fn clear(&mut self) {
        if let Some(first_element) = self.first_element.take() {
            MEC_POOL.with(|pool| pool.borrow_mut().free_element(first_element));
        }

        self.executor_count = 0;
        self.sequence_count = 0;
    }

    /// `MMLExecutorConnector()`.
    pub fn new() -> Self {
        Self {
            first_element: None,
            executor_count: 0,
            sequence_count: 0,
            connecting_sequence_group: None,
            connecting_sequence: None,
            connecting_sequence_list: Vec::new(),
        }
    }
}

impl Default for MMLExecutorConnector {
    fn default() -> Self {
        Self::new()
    }
}
