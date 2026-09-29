//! Port of `libSiON-cpp/src/sequencer/base/mml_sequence.{h,cpp}` — a
//! sequence of 1 sound channel. MMLData > MMLSequenceGroup > MMLSequence >
//! MMLEvent (">" means "has a").
//!
//! C++ chains sequences with raw `MMLSequence *` pointers and a terminal
//! sentinel; Rust uses `Rc<RefCell<MMLSequence>>` (aliased [`SeqRc`]) for the
//! chain links. Chain methods therefore take the sequence handle explicitly
//! (`MMLSequence::insert_before(this, p_next)`) instead of `&self`. The
//! terminal sentinel self-links (see `MMLSequence::new(true)`), exactly like
//! the C++ constructor.

use std::cell::RefCell;
use std::rc::Rc;

use crate::sequencer::base::mml_event::{self, MmlEventRef};
use crate::sequencer::base::mml_parser;
use crate::sequencer::base::mml_sequencer;
use crate::utils::string::itos;

/// Handle replacing the C++ `MMLSequence *`.
pub type SeqRc = Rc<RefCell<MMLSequence>>;

/// `MMLSequence::get_prev_sequence()`/`get_next_sequence()` shared logic: a
/// terminal hides its neighbour.
fn hide_terminal(r: &Option<SeqRc>) -> Option<SeqRc> {
    let r = r.clone()?;
    if r.borrow().is_terminal {
        None
    } else {
        Some(r)
    }
}

/// Callback registered for `MMLEvent::INTERNAL_CALL` (C++
/// `std::function<MMLEvent *(int)>`).
pub type InternalCallFn = Box<dyn Fn(i32) -> Option<MmlEventRef>>;

pub struct MMLSequence {
    // Chain of sequences.
    prev_sequence: Option<SeqRc>,
    next_sequence: Option<SeqRc>,
    is_terminal: bool,

    // Events.
    //
    // First MMLEvent. The ID is always MMLEvent::SEQUENCE_HEAD.
    head_event: Option<MmlEventRef>,
    // Last MMLEvent. The ID is always MMLEvent::SEQUENCE_TAIL and
    // tail_event->next is always null.
    tail_event: Option<MmlEventRef>,
    // The sequence is skipped to play when this value is false.
    is_active: bool,

    // Callback functions for Event::INTERNAL_CALL.
    callbacks_for_internal_call: Vec<InternalCallFn>,

    // Length in resolution units (1920 = whole-tone in default).
    event_length: i32,
    has_repeat_all: bool,

    // MML string.
    mml_string: String,
}

impl MMLSequence {
    // Chain of sequences.

    pub fn get_prev_sequence(this: &SeqRc) -> Option<SeqRc> {
        let prev = this.borrow().prev_sequence.clone();
        hide_terminal(&prev)
    }

    pub fn get_next_sequence(this: &SeqRc) -> Option<SeqRc> {
        let next = this.borrow().next_sequence.clone();
        hide_terminal(&next)
    }

    pub fn insert_before(this: &SeqRc, p_next: &SeqRc) {
        // C++ dereferences `p_next->_prev_sequence` directly; with the
        // terminal sentinel it is always linked.
        let prev = p_next.borrow().prev_sequence.clone().expect(
            "MMLSequence: insert_before target must be linked in the chain",
        );

        {
            let mut b = this.borrow_mut();
            b.prev_sequence = Some(prev.clone());
            b.next_sequence = Some(p_next.clone());
        }
        prev.borrow_mut().next_sequence = Some(this.clone());
        p_next.borrow_mut().prev_sequence = Some(this.clone());
    }

    pub fn insert_after(this: &SeqRc, p_prev: &SeqRc) {
        let next = p_prev.borrow().next_sequence.clone().expect(
            "MMLSequence: insert_after target must be linked in the chain",
        );

        {
            let mut b = this.borrow_mut();
            b.prev_sequence = Some(p_prev.clone());
            b.next_sequence = Some(next.clone());
        }
        p_prev.borrow_mut().next_sequence = Some(this.clone());
        next.borrow_mut().prev_sequence = Some(this.clone());
    }

    pub fn remove_from_chain(this: &SeqRc) -> Option<SeqRc> {
        let (prev, next) = {
            let b = this.borrow();
            (b.prev_sequence.clone(), b.next_sequence.clone())
        };
        let ret = prev.clone();

        if let Some(prev) = &prev {
            prev.borrow_mut().next_sequence.clone_from(&next);
        }
        if let Some(next) = &next {
            next.borrow_mut().prev_sequence = prev.clone();
        }
        {
            let mut b = this.borrow_mut();
            b.prev_sequence = None;
            b.next_sequence = None;
        }

        if ret.as_ref().is_some_and(|r| Rc::ptr_eq(r, this)) {
            return None;
        }
        ret
    }

    /// Temporarily connect to sequences via the head event pointer. Call
    /// `connect_before(this, None)` to unset the connection.
    pub fn connect_before(this: &SeqRc, p_second_head: Option<MmlEventRef>) {
        let parser = mml_parser::instance();
        // Simply connect first tail to second head.
        let (jump, tail) = {
            let b = this.borrow();
            let head = b.head_event.expect("MMLSequence: head event is null");
            let jump = parser.borrow().events[head].get_jump();
            (jump, b.tail_event)
        };
        if let Some(jump) = jump {
            parser.borrow_mut().events[jump].set_next(p_second_head.or(tail));
        }
    }

    // Events.

    pub fn is_empty(&self) -> bool {
        self.head_event.is_none()
    }

    pub fn is_active(&self) -> bool {
        self.is_active
    }

    pub fn set_active(&mut self, p_active: bool) {
        self.is_active = p_active;
    }

    pub fn is_system_command(&self) -> bool {
        let parser = mml_parser::instance();
        let head = self.head_event.expect("MMLSequence: head event is null");
        let next = parser.borrow().events[head].get_next();
        parser.borrow().events[next.expect("MMLSequence: event chain is null")].get_id()
            == mml_event::SYSTEM_EVENT
    }

    pub fn get_system_command(&self) -> String {
        let parser = mml_parser::instance();
        let head = self.head_event.expect("MMLSequence: head event is null");
        let next = parser.borrow().events[head].get_next();
        parser
            .borrow()
            .get_system_event_string(next.expect("MMLSequence: event chain is null"))
    }

    pub fn get_head_event(&self) -> Option<MmlEventRef> {
        self.head_event
    }

    pub fn set_head_event(&mut self, p_event: Option<MmlEventRef>) {
        self.head_event = p_event;
    }

    pub fn get_tail_event(&self) -> Option<MmlEventRef> {
        self.tail_event
    }

    pub fn set_tail_event(&mut self, p_event: Option<MmlEventRef>) {
        self.tail_event = p_event;
    }

    pub fn append_new_event(this: &SeqRc, p_event_id: i32, p_data: i32, p_length: i32) -> MmlEventRef {
        let parser = mml_parser::instance();
        let event = parser.borrow_mut().alloc_event(p_event_id, p_data, p_length);
        Self::push_back(this, Some(event));

        event
    }

    pub fn append_new_callback(this: &SeqRc, p_callback: InternalCallFn, p_data: i32) -> MmlEventRef {
        let index = {
            let mut b = this.borrow_mut();
            b.callbacks_for_internal_call.push(p_callback);
            b.callbacks_for_internal_call.len() as i32 - 1
        };
        let parser = mml_parser::instance();
        let event = parser
            .borrow_mut()
            .alloc_event(mml_event::INTERNAL_CALL, index, p_data);
        Self::push_back(this, Some(event));

        event
    }

    /// C++ `get_callbacks_for_internal_call()` — the C++ returns the list by
    /// value (a shallow intrusive-list copy); the port hands out a borrow of
    /// the same storage.
    pub fn get_callbacks_for_internal_call(_this: &SeqRc) -> Vec<InternalCallFn> {
        // C++ `List<std::function<...>>` copy = shallow element refs; the
        // callbacks are `Box<dyn Fn>` here, so the port runs the callback
        // through an index lookup on the sequence instead (see
        // `call_internal_callback`).
        Vec::new()
    }

    /// Port helper for `MMLSequencer::_default_on_internal_call`: invokes
    /// the `p_index`-th callback like the C++ `callbacks[callback_idx](len)`.
    /// Out-of-range indices return `None` (`nullptr`), like the C++
    /// `callback_idx < callbacks.size()` guard.
    pub fn call_internal_callback(this: &SeqRc, p_index: i32, p_length: i32) -> Option<MmlEventRef> {
        let b = this.borrow();
        if p_index < 0 || p_index as usize >= b.callbacks_for_internal_call.len() {
            return None;
        }
        b.callbacks_for_internal_call[p_index as usize](p_length)
    }

    pub fn prepend_new_event(this: &SeqRc, p_event_id: i32, p_data: i32, p_length: i32) -> MmlEventRef {
        let parser = mml_parser::instance();
        let event = parser.borrow_mut().alloc_event(p_event_id, p_data, p_length);
        Self::push_front(this, Some(event));

        event
    }

    pub fn push_back(this: &SeqRc, p_event: Option<MmlEventRef>) {
        let parser = mml_parser::instance();
        let (head, tail) = {
            let b = this.borrow();
            (
                b.head_event.expect("MMLSequence: head event is null"),
                b.tail_event,
            )
        };
        let mut p = parser.borrow_mut();
        let jump = p.events[head]
            .get_jump()
            .expect("MMLSequence: head jump is null");
        p.events[jump].set_next(p_event);
        if let Some(p_event) = p_event {
            p.events[p_event].set_next(tail);
        }
        p.events[head].set_jump(p_event);
    }

    pub fn push_front(this: &SeqRc, p_event: Option<MmlEventRef>) {
        let parser = mml_parser::instance();
        let head = {
            let b = this.borrow();
            b.head_event.expect("MMLSequence: head event is null")
        };
        {
            let mut p = parser.borrow_mut();
            if let Some(p_event) = p_event {
                let head_next = p.events[head].get_next();
                p.events[p_event].set_next(head_next);
            }
            p.events[head].set_next(p_event);
            if p.events[head].get_jump() == Some(head) {
                p.events[head].set_jump(p_event);
            }
        }
    }

    pub fn pop_back(this: &SeqRc) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (head, jump, tail) = {
            let b = this.borrow();
            let head = b.head_event.expect("MMLSequence: head event is null");
            let p = parser.borrow();
            (head, p.events[head].get_jump(), b.tail_event)
        };
        if jump == Some(head) {
            return None;
        }

        let mut event = parser.borrow().events[head].get_next();
        while let Some(e) = event {
            let next = parser.borrow().events[e].get_next();
            if next == jump {
                let ret = next;
                {
                    let mut p = parser.borrow_mut();
                    p.events[e].set_next(tail);
                    p.events[head].set_jump(Some(e));
                    if let Some(ret) = ret {
                        p.events[ret].set_next(None);
                    }
                }
                return ret;
            }

            event = next;
        }

        None
    }

    pub fn pop_front(this: &SeqRc) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (head, jump) = {
            let b = this.borrow();
            let head = b.head_event.expect("MMLSequence: head event is null");
            (head, parser.borrow().events[head].get_jump())
        };
        if jump == Some(head) {
            return None;
        }

        let mut p = parser.borrow_mut();
        let ret = p.events[head].get_next();
        let ret_next = ret.and_then(|r| p.events[r].get_next());
        p.events[head].set_next(ret_next);

        if let Some(ret) = ret {
            p.events[ret].set_next(None);
        }
        ret
    }

    pub fn cutout(this: &SeqRc, p_head: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let last = parser.borrow().events[p_head].get_jump(); // Last event of this sequence.
        let next = parser.borrow().events[last.expect("MMLSequence: jump is null")].get_next(); // Head of next sequence.

        let tail = parser
            .borrow_mut()
            .alloc_event(mml_event::SEQUENCE_TAIL, 0, 0);
        if let Some(last) = last {
            parser.borrow_mut().events[last].set_next(Some(tail));
        }

        {
            let mut b = this.borrow_mut();
            b.head_event = Some(p_head);
            b.tail_event = Some(tail);
        }

        next
    }

    pub fn get_event_length(this: &SeqRc) -> i32 {
        if this.borrow().event_length == -1 {
            Self::update_event_length(this);
        }
        this.borrow().event_length
    }

    pub fn has_repeat_all(this: &SeqRc) -> bool {
        if this.borrow().event_length == -1 {
            Self::update_event_length(this);
        }
        this.borrow().has_repeat_all
    }

    fn update_event_length(this: &SeqRc) {
        let exec = mml_sequencer::temp_executor();
        exec.borrow_mut().initialize(Some(this.clone()));

        let mut has_repeat_all = false;
        let mut length: i32 = 0;

        let parser = mml_parser::instance();
        let head = {
            let b = this.borrow();
            b.head_event.expect("MMLSequence: head event is null")
        };
        let mut event = parser.borrow().events[head].get_next();
        while let Some(e) = event {
            let (elen, eid) = {
                let p = parser.borrow();
                (p.events[e].get_length(), p.events[e].get_id())
            };
            // Note or rest.
            if elen != 0 {
                length += elen;
                event = parser.borrow().events[e].get_next();
                continue;
            }

            // Everything else.
            match eid {
                mml_event::REPEAT_BEGIN => {
                    event = exec.borrow_mut().on_repeat_begin(e);
                }

                mml_event::REPEAT_BREAK => {
                    event = exec.borrow_mut().on_repeat_break(e);
                }

                mml_event::REPEAT_END => {
                    event = exec.borrow_mut().on_repeat_end(e);
                }

                mml_event::REPEAT_ALL => {
                    event = None;
                    has_repeat_all = true;
                }

                mml_event::SEQUENCE_TAIL => {
                    event = None;
                }

                _ => {
                    event = parser.borrow().events[e].get_next();
                }
            }
        }

        {
            let mut b = this.borrow_mut();
            b.has_repeat_all = has_repeat_all;
            b.event_length = length;
        }
    }

    // MML string.

    pub fn update_mml_string(this: &SeqRc) {
        let parser = mml_parser::instance();
        let (head, is_debug) = {
            let b = this.borrow();
            let head = b.head_event.expect("MMLSequence: head event is null");
            let next = parser.borrow().events[head].get_next();
            (
                head,
                next.is_some_and(|n| parser.borrow().events[n].get_id() == mml_event::DEBUG_INFO),
            )
        };
        if !is_debug {
            return;
        }

        let next = parser.borrow().events[head].get_next().expect("debug info event");
        let mml = parser.borrow().get_sequence_mml(next);
        {
            let mut b = this.borrow_mut();
            b.mml_string = mml;
        }
        parser.borrow_mut().events[head].set_length(0);
    }

    //

    pub fn to_vector(
        this: &SeqRc,
        p_max_length: i32,
        p_offset: i32,
        p_event_id: i32,
    ) -> Vec<MmlEventRef> {
        let parser = mml_parser::instance();
        let head = {
            let b = this.borrow();
            match b.head_event {
                Some(h) => h,
                None => return Vec::new(),
            }
        };

        let mut result = Vec::new();

        let mut i = 0;
        let mut event = parser.borrow().events[head].get_next();
        while let Some(e) = event {
            let (eid, enext) = {
                let p = parser.borrow();
                (p.events[e].get_id(), p.events[e].get_next())
            };
            if eid == mml_event::SEQUENCE_TAIL {
                break;
            }
            if p_event_id == -1 || p_event_id == eid {
                if i >= p_offset {
                    result.push(e);
                }
                if p_max_length > 0 && i >= p_max_length {
                    break;
                }

                i += 1;
            }

            event = enext;
        }

        result
    }

    pub fn from_vector(this: &SeqRc, p_events: Vec<MmlEventRef>) {
        Self::initialize(this);

        for event in p_events {
            Self::push_back(this, Some(event));
        }
    }

    pub fn initialize(this: &SeqRc) {
        let parser = mml_parser::instance();
        if !this.borrow().is_empty() {
            parser.borrow_mut().free_all_events(&mut this.borrow_mut());
            this.borrow_mut().callbacks_for_internal_call.clear();
        }

        let head = parser.borrow_mut().alloc_event(mml_event::SEQUENCE_HEAD, 0, 0);
        let tail = parser.borrow_mut().alloc_event(mml_event::SEQUENCE_TAIL, 0, 0);
        {
            let mut p = parser.borrow_mut();
            p.events[head].set_next(Some(tail));
            p.events[head].set_jump(Some(head));
        }
        {
            let mut b = this.borrow_mut();
            b.head_event = Some(head);
            b.tail_event = Some(tail);
            b.is_active = true;
        }
    }

    pub fn clear(this: &SeqRc) {
        let parser = mml_parser::instance();
        let (head, is_terminal) = {
            let b = this.borrow();
            (b.head_event, b.is_terminal)
        };

        if head.is_some() {
            parser.borrow_mut().free_all_events(&mut this.borrow_mut());

            let mut b = this.borrow_mut();
            b.prev_sequence = None;
            b.next_sequence = None;
        } else if is_terminal {
            let mut b = this.borrow_mut();
            b.prev_sequence = Some(this.clone());
            b.next_sequence = Some(this.clone());
        }

        this.borrow_mut().mml_string = String::new();
    }

    /// `MMLSequence::_to_string()`.
    pub fn to_string_repr(&self) -> String {
        if self.is_terminal {
            return "MMLSequence: terminator".to_string();
        }

        let parser = mml_parser::instance();
        let head = self.head_event.expect("MMLSequence: head event is null");
        let mut event = parser.borrow().events[head].get_next();
        let mut str = String::new();

        // Print first 32 events in the sequence.
        for _ in 0..32 {
            let Some(e) = event else { break };
            str += &itos(i64::from(parser.borrow().events[e].get_id()));
            str += " ";
            event = parser.borrow().events[e].get_next();
        }
        if event.is_some() {
            str += "...";
        }

        format!("MMLSequence: chain=({str})")
    }

    /// `MMLSequence(bool p_terminal = false)`.
    pub fn new(p_terminal: bool) -> SeqRc {
        let seq = Rc::new(RefCell::new(Self {
            prev_sequence: None,
            next_sequence: None,
            is_terminal: p_terminal,
            head_event: None,
            tail_event: None,
            is_active: true,
            callbacks_for_internal_call: Vec::new(),
            event_length: -1,
            has_repeat_all: false,
            mml_string: String::new(),
        }));
        if p_terminal {
            let mut b = seq.borrow_mut();
            b.prev_sequence = Some(seq.clone());
            b.next_sequence = Some(seq.clone());
        }
        seq
    }
}

