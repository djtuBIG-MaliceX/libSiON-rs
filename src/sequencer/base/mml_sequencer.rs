//! Port of `libSiON-cpp/src/sequencer/base/mml_sequencer.{h,cpp}` — the
//! base class that bridges MMLEvents, sound modules and sound systems.
//!
//! Design (C++ base-with-virtuals → Rust), mirroring the convention in
//! `chip/channels/channel_base.rs`:
//! - [`MMLSequencerBase`] struct holding ALL shared state, with every
//!   non-virtual C++ body (and the pure-state base default handlers) as
//!   inherent methods. Concrete subclasses (wave-7c `SiMMLSequencer`) embed
//!   it as their first field.
//! - [`MMLSequencerTrait`] — the vtable: every C++ `virtual` becomes a trait
//!   method whose default body is the C++ base implementation. The base
//!   default event handlers (`_default_on_*` / `_dummy_on_*`) are trait
//!   defaults because they call the `_on_*` virtuals; they must NOT be
//!   overridden (the C++ originals are private non-virtuals bound in the
//!   base constructor).
//!
//! C++ `std::function<MMLEvent *(MMLEvent *)>` listener slots become
//! [`MmlEventHandler`] enum slots: [`MmlEventHandler::Base`] re-selects one
//! of the base defaults (this replaces `_set_default_listener`'s
//! `MMLEvent *(MMLSequencer::*)(MMLEvent *)` parameter — the C++ PMFs were
//! base methods, so a 1:1 enum names them) and
//! [`MmlEventHandler::Custom`] carries an `Rc` callback that receives the
//! concrete instance as `&mut dyn MMLSequencerTrait` (downcast to `Any` for
//! subclass state). C++ `MMLExecutor *_current_executor` raw pointers
//! (global executor or a raw pointer to a track-owned executor, set by
//! `process_executor`) become [`CurrentExecutor`]. C++ `Ref<...>` members
//! become `Rc<RefCell<...>>`.
//!
//! C++ quirks reproduced verbatim: fixed-point truncation of the
//! `length * sample_per_tick + fraction` product once per rearm, the
//! `calculate_sample_delay` double/int round-trips, `prepare_compile`
//! dropping `mml_data` when `_on_before_compile` returns an empty string,
//! the initial-BPM `double`→`int` truncation in `_extract_global_sequence`,
//! and the `process_executor`/`execute_global_sequence` null-handler branch
//! that nulls the walk pointer without storing it back.

use std::any::Any;
use std::cell::{Ref, RefCell, RefMut};
use std::collections::HashMap;
use std::rc::Rc;

use crate::err_fail_cond_msg;
use crate::err_fail_cond_v;
use crate::sequencer::base::beats_per_minute::BeatsPerMinute;
use crate::sequencer::base::mml_data::MMLData;
use crate::sequencer::data::SiMMLData;
use crate::sequencer::base::mml_event::{self, MmlEventRef};
use crate::sequencer::base::mml_executor::MMLExecutor;
use crate::sequencer::base::mml_parser;
use crate::sequencer::base::mml_parser_settings::MMLParserSettings;
use crate::sequencer::base::mml_sequence::{MMLSequence, SeqRc};
use crate::sequencer::base::mml_sequence_group::MMLSequenceGroup;

/// C++ `MMLSequencer::FIXED_BITS`.
pub const FIXED_BITS: i32 = 8;
/// C++ `MMLSequencer::FIXED_FILTER`.
pub const FIXED_FILTER: i32 = (1 << FIXED_BITS) - 1;

/// Wave-7c data-handle seam. C++ threads one aliasing `Ref<MMLData>` and
/// re-downcasts it (`Ref<SiMMLData> x = mml_data;`); Rust cannot project
/// `Rc<RefCell<SiMMLData>>` to `Rc<RefCell<MMLData>>`, so every
/// sequencer-side data slot is this enum and base bodies go through
/// [`MmlDataHandle::mml`]. The wave-8 driver builds `Simml` handles
/// (`SiMMLData` is the real compiled-song object); base tests use `Base`.
#[derive(Clone)]
pub enum MmlDataHandle {
    Base(Rc<RefCell<MMLData>>),
    Simml(Rc<RefCell<SiMMLData>>),
}

impl MmlDataHandle {
    /// Base (`MMLData`) view of the song data.
    pub fn mml(&self) -> Ref<'_, MMLData> {
        match self {
            Self::Base(data) => data.borrow(),
            Self::Simml(data) => Ref::map(data.borrow(), |data| &data.base),
        }
    }

    /// Mutable base view of the song data.
    pub fn mml_mut(&self) -> RefMut<'_, MMLData> {
        match self {
            Self::Base(data) => data.borrow_mut(),
            Self::Simml(data) => RefMut::map(data.borrow_mut(), |data| &mut data.base),
        }
    }

    /// C++ `Ref<SiMMLData> simml_data = mml_data;` — `None` mirrors the
    /// null `Ref` the C++ `dynamic_pointer_cast` yields for base-only data.
    pub fn simml(&self) -> Option<Rc<RefCell<SiMMLData>>> {
        match self {
            Self::Simml(data) => Some(data.clone()),
            Self::Base(_) => None,
        }
    }
}

thread_local! {
    static TEMP_EXECUTOR: RefCell<Option<Rc<RefCell<MMLExecutor>>>> =
        const { RefCell::new(None) };
}

/// C++ `MMLSequencer::initialize()`.
pub fn initialize() {
    TEMP_EXECUTOR.with(|t| {
        if t.borrow().is_none() {
            *t.borrow_mut() = Some(Rc::new(RefCell::new(MMLExecutor::new())));
        }
    });
}

/// C++ `MMLSequencer::finalize()`.
pub fn finalize() {
    TEMP_EXECUTOR.with(|t| *t.borrow_mut() = None);
}

/// C++ `MMLSequencer::get_temp_executor()`.
pub fn temp_executor() -> Rc<RefCell<MMLExecutor>> {
    initialize();
    TEMP_EXECUTOR.with(|t| t.borrow().clone().unwrap())
}

/// C++ `_set_default_listener`'s member-function-pointer parameter: one
/// variant per private base default handler a subclass may re-register
/// (`SiMMLSequencer` selects from `NoProcess`, `DummyOnProcess`,
/// `DummyOnProcessEvent` and the `DefaultOn*` group).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MmlBaseHandler {
    NoProcess,
    DummyOnProcess,
    DummyOnProcessEvent,
    DefaultOnNoOperation,
    DefaultOnGlobalWait,
    DefaultOnProcess,
    DefaultOnRepeatAll,
    DefaultOnRepeatBegin,
    DefaultOnRepeatBreak,
    DefaultOnRepeatEnd,
    DefaultOnSequenceTail,
    DefaultOnTempo,
    DefaultOnTimer,
    DefaultOnInternalWait,
    DefaultOnInternalCall,
}

/// C++ `std::function<MMLEvent *(MMLEvent *)>` bound to the concrete
/// instance (wave-7c wraps its `_on_mml_*` methods this way;
/// `host.as_any_mut()` reaches subclass state).
pub type MmlCustomHandler =
    Rc<dyn Fn(&mut dyn MMLSequencerTrait, MmlEventRef) -> Option<MmlEventRef>>;

/// One `_event_handlers` slot.
#[derive(Clone)]
pub enum MmlEventHandler {
    /// Empty `std::function` — the pumps take their "this shouldn't happen"
    /// branch (walk pointer becomes null, nothing stored back).
    Empty,
    /// C++ `[this](MMLEvent *e) { return _no_process(e); }` (the initial
    /// value of every slot and the `TABLE_EVENT` registration).
    NoProcess,
    Base(MmlBaseHandler),
    Custom(MmlCustomHandler),
}

/// C++ `MMLExecutor *_current_executor`: either the owned global executor
/// or a raw pointer to a track-owned executor handed to
/// [`MMLSequencerTrait::process_executor`].
///
/// SAFETY contract (same as C++ raw pointers): the pointed executor must
/// outlive the `process_executor` call and must not be mutably borrowed
/// elsewhere (build the pointer in the raw domain, e.g. from
/// `Rc::as_ptr`, never keep it across another mutable borrow of the track).
#[derive(Clone, Copy)]
pub enum CurrentExecutor {
    Global,
    Track(*mut MMLExecutor),
}

/// Shared state of every sequencer — C++ `MMLSequencer` members. `pub`
/// fields mirror the C++ `protected` access concrete subclasses use.
pub struct MMLSequencerBase {
    parser_settings: Rc<RefCell<MMLParserSettings>>,
    pub sample_rate: i32,

    pub global_executor: MMLExecutor,
    pub current: Option<CurrentExecutor>,
    pub mml_data: Option<MmlDataHandle>,
    pub adjustible_bpm: Rc<RefCell<BeatsPerMinute>>,
    pub bpm: Rc<RefCell<BeatsPerMinute>>,

    pub global_buffer_index: i32,
    pub global_beat_16th: f64,
    /// Filter for the on-beat callback, 0 = 16th beat, 1 = 8th beat,
    /// 3 = 4th beat, 7 = 2nd beat, 15 = whole tone.
    pub on_beat_callback_filter: i32,

    // Events.
    next_user_defined_event_id: i32,
    user_defined_event_map: HashMap<String, i32>,
    event_command_letter_map: HashMap<i32, String>,
    event_handlers: Vec<MmlEventHandler>,
    event_global_flags: Vec<bool>,

    // Compilation and processing.
    // Leftover of buffer sample count in processing.
    process_buffer_sample_count: i32,
    // Leftover of buffer sample count in global sequence.
    global_buffer_sample_count: i32,
    // Executing buffer length in global sequence.
    global_execute_sample_count: i32,
    buffer_length: i32,
}

impl MMLSequencerBase {
    /// C++ `get_parser_settings()`.
    pub fn get_parser_settings(&self) -> Rc<RefCell<MMLParserSettings>> {
        self.parser_settings.clone()
    }

    /// C++ `get_sample_rate()`.
    pub fn get_sample_rate(&self) -> i32 {
        self.sample_rate
    }

    /// C++ `get_default_bpm()`.
    pub fn get_default_bpm(&self) -> f64 {
        self.parser_settings.borrow().default_bpm
    }

    /// C++ `set_default_bpm(double)`.
    pub fn set_default_bpm(&mut self, p_value: f64) {
        self.parser_settings.borrow_mut().default_bpm = p_value;
    }

    /// C++ `get_bpm()`.
    pub fn get_bpm(&self) -> f64 {
        self.bpm.borrow().get_bpm()
    }

    /// C++ `get_event_id(sion::String)`.
    pub fn get_event_id(&self, p_mml_command: &str) -> i32 {
        let event_id = mml_event::MMLEvent::get_id_from_mml(p_mml_command);
        if event_id != 0 {
            return event_id;
        }

        match self.user_defined_event_map.get(p_mml_command) {
            Some(id) => *id,
            None => 0,
        }
    }

    /// C++ `get_event_letter(int)`.
    pub fn get_event_letter(&self, p_event_id: i32) -> String {
        err_fail_cond_v!(
            !self.event_command_letter_map.contains_key(&p_event_id),
            "!_event_command_letter_map.has(p_event_id)",
            String::new()
        );

        self.event_command_letter_map[&p_event_id].clone()
    }

    /// C++ `_set_mml_event_listener(int, handler, bool)`. NOTE: an id
    /// outside `0..COMMAND_MAX` panics where the C++ indexed out of bounds.
    pub fn set_mml_event_listener(
        &mut self,
        p_event_id: i32,
        p_handler: MmlEventHandler,
        p_global: bool,
    ) {
        self.event_handlers[p_event_id as usize] = p_handler;
        self.event_global_flags[p_event_id as usize] = p_global;
    }

    /// C++ `_set_default_listener(int, PMF, bool)` — the enum names the
    /// base default the member-function pointer used to select.
    pub fn set_default_listener(&mut self, p_event_id: i32, p_handler: MmlBaseHandler, p_global: bool) {
        self.set_mml_event_listener(p_event_id, MmlEventHandler::Base(p_handler), p_global);
    }

    /// C++ `_create_mml_event_listener(sion::String, handler, bool)`.
    pub fn create_mml_event_listener(
        &mut self,
        p_letter: String,
        p_handler: MmlEventHandler,
        p_global: bool,
    ) -> i32 {
        let event_id = self.next_user_defined_event_id;
        self.next_user_defined_event_id += 1;

        self.user_defined_event_map.insert(p_letter.clone(), event_id);
        self.event_command_letter_map.insert(event_id, p_letter);
        self.set_mml_event_listener(event_id, p_handler, p_global);

        event_id
    }

    /// C++ `calculate_sample_count(int)`. NOTE: the C++ truncates the
    /// `int * double` product to `int` BEFORE the fixed-point shift.
    pub fn calculate_sample_count(&self, p_length: i32) -> i32 {
        (p_length as f64 * self.bpm.borrow().get_sample_per_tick()) as i32 >> FIXED_BITS
    }

    /// C++ `calculate_sample_length(double)`.
    pub fn calculate_sample_length(&self, p_beat_16th: f64) -> f64 {
        p_beat_16th * self.bpm.borrow().get_sample_per_beat_16th()
    }

    /// C++ `calculate_sample_delay(int, double, double)`. NOTE: the C++
    /// mixes `int beats` with the `double p_quant` through implicit
    /// conversions; the int↔double round-trips (truncation toward zero)
    /// are reproduced on the same expressions.
    pub fn calculate_sample_delay(
        &self,
        p_sample_offset: i32,
        p_beat_16th_offset: f64,
        p_quant: f64,
    ) -> f64 {
        let bpm = self.bpm.borrow();

        if p_quant == 0.0 {
            return p_sample_offset as f64 + p_beat_16th_offset * bpm.get_sample_per_beat_16th();
        }

        let mut beats = (p_sample_offset as f64 * bpm.get_beat_16th_per_sample()
            + self.global_beat_16th
            + p_beat_16th_offset
            + 0.9999847412109375) as i32; // = 65535/65536
        if p_quant != 1.0 {
            beats = (((beats as f64 + p_quant - 1.0) / p_quant) as i32 as f64 * p_quant) as i32;
        }

        (beats as f64 - self.global_beat_16th) * bpm.get_sample_per_beat_16th()
    }

    /// C++ `get_current_tick_count()`. NOTE: C++ evaluates
    /// `int - int * double` entirely in double and truncates once on
    /// return; the subtraction is NOT integer.
    pub fn get_current_tick_count(&self) -> i32 {
        (self.current_get_current_tick_count() as f64
            - self.current_get_residue() as f64 * self.bpm.borrow().get_tick_per_sample())
            as i32
    }

    // Current-executor accessors (`_current_executor` dereferences).

    pub(crate) fn current_get_pointer(&self) -> Option<MmlEventRef> {
        match self.current.expect("MMLSequencer: current executor is null") {
            CurrentExecutor::Global => self.global_executor.get_pointer(),
            CurrentExecutor::Track(executor) => unsafe { (*executor).get_pointer() },
        }
    }

    pub(crate) fn current_set_pointer(&mut self, p_event: Option<MmlEventRef>) {
        match self.current.expect("MMLSequencer: current executor is null") {
            CurrentExecutor::Global => self.global_executor.set_pointer(p_event),
            CurrentExecutor::Track(executor) => unsafe { (*executor).set_pointer(p_event) },
        }
    }

    pub(crate) fn current_get_residue(&self) -> i32 {
        match self.current.expect("MMLSequencer: current executor is null") {
            CurrentExecutor::Global => self.global_executor.get_residue_sample_count(),
            CurrentExecutor::Track(executor) => unsafe { (*executor).get_residue_sample_count() },
        }
    }

    pub(crate) fn current_set_residue(&mut self, p_value: i32) {
        match self.current.expect("MMLSequencer: current executor is null") {
            CurrentExecutor::Global => self.global_executor.set_residue_sample_count(p_value),
            CurrentExecutor::Track(executor) => unsafe {
                (*executor).set_residue_sample_count(p_value)
            },
        }
    }

    pub(crate) fn current_adjust_residue(&mut self, p_diff: i32) {
        match self.current.expect("MMLSequencer: current executor is null") {
            CurrentExecutor::Global => self.global_executor.adjust_residue_sample_count(p_diff),
            CurrentExecutor::Track(executor) => unsafe {
                (*executor).adjust_residue_sample_count(p_diff)
            },
        }
    }

    pub(crate) fn current_get_decimal_fraction(&self) -> i32 {
        match self.current.expect("MMLSequencer: current executor is null") {
            CurrentExecutor::Global => self.global_executor.get_decimal_fraction_sample_count(),
            CurrentExecutor::Track(executor) => unsafe {
                (*executor).get_decimal_fraction_sample_count()
            },
        }
    }

    pub(crate) fn current_set_decimal_fraction(&mut self, p_value: i32) {
        match self.current.expect("MMLSequencer: current executor is null") {
            CurrentExecutor::Global => {
                self.global_executor.set_decimal_fraction_sample_count(p_value)
            }
            CurrentExecutor::Track(executor) => unsafe {
                (*executor).set_decimal_fraction_sample_count(p_value)
            },
        }
    }

    pub(crate) fn current_get_nop_event(&self) -> MmlEventRef {
        match self.current.expect("MMLSequencer: current executor is null") {
            CurrentExecutor::Global => self.global_executor.get_nop_event(),
            CurrentExecutor::Track(executor) => unsafe { (*executor).get_nop_event() },
        }
    }

    pub(crate) fn current_get_sequence(&self) -> Option<SeqRc> {
        match self.current.expect("MMLSequencer: current executor is null") {
            CurrentExecutor::Global => self.global_executor.get_sequence(),
            CurrentExecutor::Track(executor) => unsafe { (*executor).get_sequence() },
        }
    }

    pub(crate) fn current_get_current_tick_count(&self) -> i32 {
        match self.current.expect("MMLSequencer: current executor is null") {
            CurrentExecutor::Global => self.global_executor.get_current_tick_count(),
            CurrentExecutor::Track(executor) => unsafe { (*executor).get_current_tick_count() },
        }
    }

    pub(crate) fn current_publish_processing_event(
        &mut self,
        p_event: MmlEventRef,
    ) -> Option<MmlEventRef> {
        match self.current.expect("MMLSequencer: current executor is null") {
            CurrentExecutor::Global => self.global_executor.publish_processing_event(p_event),
            CurrentExecutor::Track(executor) => unsafe {
                (*executor).publish_processing_event(p_event)
            },
        }
    }

    pub(crate) fn current_on_repeat_all(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        match self.current.expect("MMLSequencer: current executor is null") {
            CurrentExecutor::Global => self.global_executor.on_repeat_all(p_event),
            CurrentExecutor::Track(executor) => unsafe { (*executor).on_repeat_all(p_event) },
        }
    }

    pub(crate) fn current_on_repeat_begin(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        match self.current.expect("MMLSequencer: current executor is null") {
            CurrentExecutor::Global => self.global_executor.on_repeat_begin(p_event),
            CurrentExecutor::Track(executor) => unsafe { (*executor).on_repeat_begin(p_event) },
        }
    }

    pub(crate) fn current_on_repeat_break(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        match self.current.expect("MMLSequencer: current executor is null") {
            CurrentExecutor::Global => self.global_executor.on_repeat_break(p_event),
            CurrentExecutor::Track(executor) => unsafe { (*executor).on_repeat_break(p_event) },
        }
    }

    pub(crate) fn current_on_repeat_end(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        match self.current.expect("MMLSequencer: current executor is null") {
            CurrentExecutor::Global => self.global_executor.on_repeat_end(p_event),
            CurrentExecutor::Track(executor) => unsafe { (*executor).on_repeat_end(p_event) },
        }
    }

    pub(crate) fn current_on_sequence_tail(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        match self.current.expect("MMLSequencer: current executor is null") {
            CurrentExecutor::Global => self.global_executor.on_sequence_tail(p_event),
            CurrentExecutor::Track(executor) => unsafe { (*executor).on_sequence_tail(p_event) },
        }
    }

    /// C++ `MMLSequencer()` — parser settings, handler table (all slots
    /// start at `_no_process`), default listener registration and the
    /// command-letter seed from the parser singleton.
    pub fn new() -> Self {
        let mut event_command_letter_map = HashMap::new();
        mml_parser::instance()
            .borrow()
            .get_command_letters(&mut event_command_letter_map);

        let mut event_handlers = vec![MmlEventHandler::NoProcess; mml_event::COMMAND_MAX];
        let mut event_global_flags = vec![false; mml_event::COMMAND_MAX];

        let mut register = |id: i32, handler: MmlBaseHandler, global: bool| {
            event_handlers[id as usize] = MmlEventHandler::Base(handler);
            event_global_flags[id as usize] = global;
        };
        register(mml_event::NO_OP, MmlBaseHandler::DefaultOnNoOperation, false);
        register(mml_event::PROCESS, MmlBaseHandler::DefaultOnProcess, false);
        register(mml_event::REPEAT_ALL, MmlBaseHandler::DefaultOnRepeatAll, false);
        register(mml_event::REPEAT_BEGIN, MmlBaseHandler::DefaultOnRepeatBegin, false);
        register(mml_event::REPEAT_BREAK, MmlBaseHandler::DefaultOnRepeatBreak, false);
        register(mml_event::REPEAT_END, MmlBaseHandler::DefaultOnRepeatEnd, false);
        register(mml_event::SEQUENCE_TAIL, MmlBaseHandler::DefaultOnSequenceTail, false);
        register(mml_event::GLOBAL_WAIT, MmlBaseHandler::DefaultOnGlobalWait, true);
        register(mml_event::TEMPO, MmlBaseHandler::DefaultOnTempo, true);
        register(mml_event::TIMER, MmlBaseHandler::DefaultOnTimer, true);
        register(mml_event::INTERNAL_WAIT, MmlBaseHandler::DefaultOnInternalWait, false);
        register(mml_event::INTERNAL_CALL, MmlBaseHandler::DefaultOnInternalCall, false);
        register(mml_event::TABLE_EVENT, MmlBaseHandler::NoProcess, true);

        let bpm = Rc::new(RefCell::new(BeatsPerMinute::new(120.0, 44100, 1920)));

        Self {
            parser_settings: Rc::new(RefCell::new(MMLParserSettings::default())),
            sample_rate: 44100,
            global_executor: MMLExecutor::new(),
            current: None,
            mml_data: None,
            adjustible_bpm: bpm.clone(),
            bpm,
            global_buffer_index: 0,
            global_beat_16th: 0.0,
            on_beat_callback_filter: 3,
            next_user_defined_event_id: mml_event::USER_DEFINED,
            user_defined_event_map: HashMap::new(),
            event_command_letter_map,
            event_handlers,
            event_global_flags,
            process_buffer_sample_count: 0,
            global_buffer_sample_count: 0,
            global_execute_sample_count: 0,
            buffer_length: 0,
        }
    }
}

impl Default for MMLSequencerBase {
    fn default() -> Self {
        Self::new()
    }
}

/// The C++ `MMLSequencer` vtable. The `_on_*` methods are the extension
/// points subclasses override; the pump methods and base default handlers
/// carry the C++ base bodies. To call a base body from an override use
/// `MMLSequencerTrait::<BaseSequencer>::method`-style qualified calls or
/// forward through `base_mut()`.
pub trait MMLSequencerTrait {
    /// Downcast helpers to the embedded [`MMLSequencerBase`] (replaces
    /// direct base-class member access in C++ derived methods).
    fn base(&self) -> &MMLSequencerBase;
    fn base_mut(&mut self) -> &mut MMLSequencerBase;

    /// Host cast for [`MmlCustomHandler`] callbacks (replaces the C++
    /// `this` captured by subclass listener lambdas).
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;

    // Extension points (`virtual ... {}` — empty C++ base bodies).

    /// C++ `_on_before_compile(sion::String)` — takes the MML string to
    /// parse and returns a new MML string; empty means the original string
    /// will be parsed (and compilation is skipped outright).
    fn on_before_compile(&mut self, _p_mml: String) -> String {
        String::new()
    }

    /// C++ `_on_after_compile(MMLSequenceGroup *)`.
    fn on_after_compile(&mut self, _p_group: &mut MMLSequenceGroup) {}

    /// C++ `_on_process(int, MMLEvent *)`.
    fn on_process(&mut self, _p_length: i32, _p_event: Option<MmlEventRef>) {}

    /// C++ `_on_timer_interruption()`.
    fn on_timer_interruption(&mut self) {}

    /// C++ `_on_beat(int, int)`.
    fn on_beat(&mut self, _p_delay_samples: i32, _p_beat_counter: i32) {}

    /// C++ `_on_table_parse(MMLEvent *, sion::String)`.
    fn on_table_parse(&mut self, _p_prev: MmlEventRef, _p_table: String) {}

    /// C++ `_on_tempo_changed(double)`.
    fn on_tempo_changed(&mut self, _p_tempo_ratio: f64) {}

    // Event handlers (private non-virtual base defaults — do not override;
    // they reach subclass behavior only through the `_on_*` virtuals).

    /// C++ `_no_process(MMLEvent *)`.
    fn no_process(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        parser.borrow().events[p_event].get_next()
    }

    /// C++ `_dummy_on_process(MMLEvent *)` — advances the clock without
    /// sounding anything (dummy/measure mode).
    fn dummy_on_process(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        if self.base().current_get_residue() == 0 {
            let length = {
                let parser = mml_parser::instance();
                parser.borrow().events[p_event].get_length()
            };
            let fraction = self.base().current_get_decimal_fraction();
            let sample_per_tick = self.base().bpm.borrow().get_sample_per_tick();
            // C++ truncates the whole `length * sample_per_tick + fraction`
            // double product once, then splits the fixed-point word.
            let sample_count_fixed = (length as f64 * sample_per_tick + fraction as f64) as i32;
            self.base_mut()
                .current_set_residue(sample_count_fixed >> FIXED_BITS);
            self.base_mut()
                .current_set_decimal_fraction(sample_count_fixed & FIXED_FILTER);
        }

        let residue = self.base().current_get_residue();
        let buffer = self.base().process_buffer_sample_count;
        if residue <= buffer {
            self.base_mut().process_buffer_sample_count = buffer - residue;
            self.base_mut().current_set_residue(0);
            let parser = mml_parser::instance();
            let jump = parser.borrow().events[p_event]
                .get_jump()
                .expect("MMLSequencer: process jump is null");
            parser.borrow().events[jump].get_next()
        } else {
            self.base_mut().current_adjust_residue(-buffer);
            self.base_mut().process_buffer_sample_count = 0;
            Some(p_event)
        }
    }

    /// C++ `_dummy_on_process_event(MMLEvent *)`.
    fn dummy_on_process_event(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        self.base_mut().current_publish_processing_event(p_event)
    }

    /// C++ `_default_on_no_operation(MMLEvent *)` — `MMLEvent::NO_OP`.
    /// NOTE: C++ does not subtract from `_process_buffer_sample_count`
    /// here, which is what ends the `process_executor` loop via the null
    /// event branch.
    fn default_on_no_operation(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let length = self.base().process_buffer_sample_count;
        self.on_process(length, Some(p_event));
        self.base_mut().current_adjust_residue(-length);
        Some(p_event)
    }

    /// C++ `_default_on_global_wait(MMLEvent *)` — `MMLEvent::GLOBAL_WAIT`.
    fn default_on_global_wait(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        if self.base().current_get_residue() == 0 {
            let length = {
                let parser = mml_parser::instance();
                parser.borrow().events[p_event].get_length()
            };
            let fraction = self.base().current_get_decimal_fraction();
            let sample_per_tick = self.base().bpm.borrow().get_sample_per_tick();
            // C++ single truncation of the fixed-point product (see
            // `dummy_on_process`).
            let sample_count_fixed = (length as f64 * sample_per_tick + fraction as f64) as i32;
            self.base_mut()
                .current_set_residue(sample_count_fixed >> FIXED_BITS);
            self.base_mut()
                .current_set_decimal_fraction(sample_count_fixed & FIXED_FILTER);
        }

        let residue = self.base().current_get_residue();
        let buffer = self.base().global_buffer_sample_count;
        if residue <= buffer {
            self.base_mut().global_execute_sample_count = residue;
            self.base_mut().global_buffer_sample_count = buffer - residue;
            self.base_mut().current_set_residue(0);
            let parser = mml_parser::instance();
            parser.borrow().events[p_event].get_next()
        } else {
            self.base_mut().global_execute_sample_count = buffer;
            self.base_mut().current_adjust_residue(-buffer);
            self.base_mut().global_buffer_sample_count = 0;
            Some(p_event)
        }
    }

    /// C++ `_default_on_process(MMLEvent *)` — `MMLEvent::PROCESS`.
    fn default_on_process(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        if self.base().current_get_residue() == 0 {
            let length = {
                let parser = mml_parser::instance();
                parser.borrow().events[p_event].get_length()
            };
            let fraction = self.base().current_get_decimal_fraction();
            let sample_per_tick = self.base().bpm.borrow().get_sample_per_tick();
            // C++ single truncation of the fixed-point product (see
            // `dummy_on_process`).
            let sample_count_fixed = (length as f64 * sample_per_tick + fraction as f64) as i32;
            self.base_mut()
                .current_set_residue(sample_count_fixed >> FIXED_BITS);
            self.base_mut()
                .current_set_decimal_fraction(sample_count_fixed & FIXED_FILTER);
        }

        let parser = mml_parser::instance();
        let jump = parser.borrow().events[p_event]
            .get_jump()
            .expect("MMLSequencer: process jump is null");

        let residue = self.base().current_get_residue();
        let buffer = self.base().process_buffer_sample_count;
        if residue <= buffer {
            self.on_process(residue, Some(jump));
            self.base_mut().process_buffer_sample_count = buffer - residue;
            self.base_mut().current_set_residue(0);
            parser.borrow().events[jump].get_next()
        } else {
            self.on_process(buffer, Some(jump));
            self.base_mut().current_adjust_residue(-buffer);
            self.base_mut().process_buffer_sample_count = 0;
            Some(p_event)
        }
    }

    /// C++ `_default_on_repeat_all(MMLEvent *)`.
    fn default_on_repeat_all(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        self.base_mut().current_on_repeat_all(p_event)
    }

    /// C++ `_default_on_repeat_begin(MMLEvent *)`.
    fn default_on_repeat_begin(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        self.base_mut().current_on_repeat_begin(p_event)
    }

    /// C++ `_default_on_repeat_break(MMLEvent *)`.
    fn default_on_repeat_break(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        self.base_mut().current_on_repeat_break(p_event)
    }

    /// C++ `_default_on_repeat_end(MMLEvent *)`.
    fn default_on_repeat_end(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        self.base_mut().current_on_repeat_end(p_event)
    }

    /// C++ `_default_on_sequence_tail(MMLEvent *)`.
    fn default_on_sequence_tail(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        self.base_mut().current_on_sequence_tail(p_event)
    }

    /// C++ `_default_on_tempo(MMLEvent *)` — `MMLEvent::TEMPO`.
    fn default_on_tempo(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let data = {
            let parser = mml_parser::instance();
            parser.borrow().events[p_event].get_data()
        };
        let mml_data = self.base().mml_data.clone();
        let bpm = match &mml_data {
            Some(handle) => handle.mml().get_bpm_from_tcommand(data),
            None => data as f64,
        };
        self.set_bpm(bpm);

        let parser = mml_parser::instance();
        parser.borrow().events[p_event].get_next()
    }

    /// C++ `_default_on_timer(MMLEvent *)` — `MMLEvent::TIMER`.
    fn default_on_timer(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        self.on_timer_interruption();
        let parser = mml_parser::instance();
        parser.borrow().events[p_event].get_next()
    }

    /// C++ `_default_on_internal_wait(MMLEvent *)`.
    fn default_on_internal_wait(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        self.base_mut().current_publish_processing_event(p_event)
    }

    /// C++ `_default_on_internal_call(MMLEvent *)` — the C++ copies the
    /// callback list and checks `cb` for null; the Rust port invokes
    /// through [`MMLSequence::call_internal_callback`] (out-of-range
    /// indices behave like the C++ size guard, boxed callbacks are never
    /// null).
    fn default_on_internal_call(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (data, length) = {
            let p = parser.borrow();
            (p.events[p_event].get_data(), p.events[p_event].get_length())
        };

        let sequence = self.base().current_get_sequence();
        let next = sequence
            .as_ref()
            .and_then(|seq| MMLSequence::call_internal_callback(seq, data, length));

        match next {
            Some(next) => Some(next),
            None => parser.borrow().events[p_event].get_next(),
        }
    }

    // Compilation and processing.

    /// C++ `prepare_compile(const Ref<MMLData>&, sion::String)` — returns
    /// false if compilation is not needed. NOTE: C++ drops `mml_data` when
    /// `_on_before_compile` returns an empty string (the original input is
    /// NOT parsed); reproduced.
    fn prepare_compile(&mut self, p_data: Option<MmlDataHandle>, p_mml: String) -> bool {
        self.base_mut().mml_data = p_data;
        let mml_data = match &self.base().mml_data {
            Some(data) => data.clone(),
            None => return false,
        };

        mml_data.mml_mut().clear();

        let parser = mml_parser::instance();
        {
            let base = self.base();
            let mut parser = parser.borrow_mut();
            parser.set_user_defined_event_map(base.user_defined_event_map.clone());
            parser.set_global_event_flags(base.event_global_flags.clone());
        }

        let mml_string = self.on_before_compile(p_mml);
        if mml_string.is_empty() {
            self.base_mut().mml_data = None;
            return false;
        }

        let settings = self.base().get_parser_settings();
        parser.borrow_mut().prepare_parse(settings, mml_string);
        true
    }

    /// C++ `compile(int p_interval = 1000)` — returns compilation
    /// progress [0-1].
    fn compile(&mut self, p_interval: i32) -> f64 {
        let mml_data = match &self.base().mml_data {
            Some(data) => data.clone(),
            None => return 1.0,
        };

        let parser = mml_parser::instance();
        let event = match parser.borrow_mut().parse(p_interval) {
            Some(event) => event,
            // If there is no event, then the parsing process is still
            // going.
            None => return parser.borrow().get_parse_progress(),
        };

        // Create the main sequence group.
        let mut unhandled = mml_data
            .mml_mut()
            .get_sequence_group()
            .populate_sequences(event);
        while let Some(event) = unhandled {
            unhandled = parser.borrow_mut().free_event(event);
        }

        self.extract_global_sequence();
        let mut data = mml_data.mml_mut();
        self.on_after_compile(data.get_sequence_group());

        1.0
    }

    /// C++ `prepare_process(const Ref<MMLData>&, int, int)`.
    fn prepare_process(
        &mut self,
        p_data: Option<MmlDataHandle>,
        p_sample_rate: i32,
        p_buffer_length: i32,
    ) {
        err_fail_cond_msg!(
            p_sample_rate != 22050 && p_sample_rate != 44100,
            "(p_sample_rate != 22050 && p_sample_rate != 44100)",
            "MMLSequencer: Sampling rate can only be 22050 or 44100."
        );

        self.base_mut().mml_data = p_data;
        self.base_mut().sample_rate = p_sample_rate;
        self.base_mut().buffer_length = p_buffer_length;

        let mml_data = self.base().mml_data.clone();
        let data_bpm = match &mml_data {
            Some(data) => data.mml().get_bpm(),
            None => 0.0,
        };

        match &mml_data {
            Some(data) if data_bpm > 0.0 => {
                self.base_mut()
                    .adjustible_bpm
                    .borrow_mut()
                    .update(data_bpm, p_sample_rate);
                let sequence = data.mml().get_global_sequence();
                self.base_mut().global_executor.initialize(Some(sequence));
            }
            _ => {
                let default_bpm = self.base().get_default_bpm();
                self.base_mut()
                    .adjustible_bpm
                    .borrow_mut()
                    .update(default_bpm, p_sample_rate);
                self.base_mut().global_executor.initialize(None);
            }
        }

        let adjustible = self.base().adjustible_bpm.clone();
        self.base_mut().bpm = adjustible;
        self.base_mut().global_buffer_index = 0;
        self.base_mut().global_beat_16th = 0.0;
    }

    /// C++ `process()`.
    fn process(&mut self) {}

    // Compilation and processing handlers.

    /// C++ `_extract_global_sequence()` — walks every sequence with the
    /// temp executor, cuts global events (and parses table events) into a
    /// position-sorted global sequence. NOTE: dropping the first sequence
    /// (`has_no_event`) anchors the walk on the terminal, whose
    /// `get_next_sequence` hides itself exactly like the C++ — the loop
    /// then ends and later sequences keep their global events. NOTE: the
    /// initial BPM from the first zero-length `TEMPO` is a `double` cut to
    /// `int`. NOTE: out-of-range event ids read the global flag as false
    /// (the C++ indexed out of bounds).
    fn extract_global_sequence(&mut self) {
        let mml_data = self
            .base()
            .mml_data
            .clone()
            .expect("MMLSequencer: extract requires mml_data");
        let parser = mml_parser::instance();
        let temp_executor = temp_executor();

        let mut global_list: Vec<MmlEventRef> = Vec::new();

        let mut sequence = mml_data
            .mml_mut()
            .get_sequence_group()
            .get_head_sequence();
        while let Some(current) = sequence {
            let head_event = current
                .borrow()
                .get_head_event()
                .expect("MMLSequence: head event is null");
            let mut count = parser.borrow().events[head_event].get_data();
            if count == 0 {
                sequence = MMLSequence::get_next_sequence(&current);
                continue;
            }

            temp_executor.borrow_mut().initialize(Some(current.clone()));

            let mut prev = head_event;
            let mut event = parser.borrow().events[prev].get_next();
            let mut position = 0i32;
            let mut has_no_event = true;

            // Calculate position and pick global events.
            while let Some(ev) = event {
                if count <= 0 && !has_no_event {
                    break;
                }

                let id = parser.borrow().events[ev].get_id();
                if self
                    .base()
                    .event_global_flags
                    .get(id as usize)
                    .copied()
                    .unwrap_or(false)
                {
                    if id == mml_event::TABLE_EVENT {
                        // It's a table event, parse it in its own special
                        // way. Once consumed, table events are
                        // automatically removed from the chain.
                        self.parse_table_event(prev);
                    } else {
                        // And here it's a global event. Cut from the chain
                        // to further process below.
                        if parser.borrow().events[head_event].get_jump() == Some(ev) {
                            parser.borrow_mut().events[head_event].set_jump(Some(prev));
                        }

                        let ev_next = parser.borrow().events[ev].get_next();
                        parser.borrow_mut().events[prev].set_next(ev_next);
                        parser.borrow_mut().events[ev].set_next(None);
                        parser.borrow_mut().events[ev].set_length(position);
                        global_list.push(ev);
                    }

                    event = parser.borrow().events[prev].get_next();
                    count -= 1;
                    continue;
                }

                let ev_length = parser.borrow().events[ev].get_length();
                if ev_length != 0 {
                    // Note or rest.
                    position += ev_length;
                    if id != mml_event::REST {
                        has_no_event = false;
                    }
                    prev = ev;
                    event = parser.borrow().events[ev].get_next();
                    continue;
                }

                // Everything else.
                prev = ev;
                event = match id {
                    mml_event::REPEAT_BEGIN => temp_executor.borrow_mut().on_repeat_begin(ev),
                    mml_event::REPEAT_BREAK => {
                        let next = temp_executor.borrow_mut().on_repeat_break(ev);
                        if parser.borrow().events[prev].get_next() != next {
                            let jump = parser.borrow().events[prev]
                                .get_jump()
                                .expect("MMLSequencer: repeat break jump is null");
                            prev = parser.borrow().events[jump]
                                .get_jump()
                                .expect("MMLSequencer: repeat break jump is null");
                        }
                        next
                    }
                    mml_event::REPEAT_END => {
                        let next = temp_executor.borrow_mut().on_repeat_end(ev);
                        if parser.borrow().events[prev].get_next() != next {
                            prev = parser.borrow().events[prev]
                                .get_jump()
                                .expect("MMLSequencer: repeat end jump is null");
                        }
                        next
                    }
                    mml_event::REPEAT_ALL => temp_executor.borrow_mut().on_repeat_all(ev),
                    mml_event::SEQUENCE_TAIL => None,
                    _ => {
                        has_no_event = true;
                        parser.borrow().events[ev].get_next()
                    }
                };
            }

            // If there is no event (except for rest) in the sequence, skip
            // this sequence.
            let mut anchor = current.clone();
            if has_no_event {
                anchor = MMLSequence::remove_from_chain(&current)
                    .expect("MMLSequencer: chain head is not a terminal");
            }
            sequence = MMLSequence::get_next_sequence(&anchor);
        }

        global_list.sort_by(|a, b| {
            let parser = mml_parser::instance();
            let parser = parser.borrow();
            parser.events[*a]
                .get_length()
                .cmp(&parser.events[*b].get_length())
        });

        // Create global sequence.
        let global_sequence = mml_data.mml().get_global_sequence();
        let mut position = 0i32;
        let mut initial_bpm = 0i32;

        for global_event in global_list {
            let (length, id, data) = {
                let parser = parser.borrow();
                (
                    parser.events[global_event].get_length(),
                    parser.events[global_event].get_id(),
                    parser.events[global_event].get_data(),
                )
            };

            if length == 0 && id == mml_event::TEMPO {
                // First tempo command is default BPM.
                initial_bpm = mml_data.mml().get_bpm_from_tcommand(data) as i32;

                parser.borrow_mut().free_event(global_event); // Free after consumption.
            } else {
                let count = length - position;
                position = length;
                parser.borrow_mut().events[global_event].set_length(0);

                if count > 0 {
                    MMLSequence::append_new_event(&global_sequence, mml_event::GLOBAL_WAIT, 0, count);
                }
                MMLSequence::push_back(&global_sequence, Some(global_event));
            }
        }

        if initial_bpm > 0 {
            let resolution = self.base().get_parser_settings().borrow().resolution;
            let bpm = Rc::new(RefCell::new(BeatsPerMinute::new(
                initial_bpm as f64,
                44100,
                resolution,
            )));
            mml_data.mml_mut().set_bpm_settings(Some(bpm));
        }
    }

    /// C++ `set_bpm(double)`.
    fn set_bpm(&mut self, p_value: f64) {
        let (old_value, sample_rate) = {
            let base = self.base();
            (base.adjustible_bpm.borrow().get_bpm(), base.sample_rate)
        };

        if self
            .base()
            .adjustible_bpm
            .borrow_mut()
            .update(p_value, sample_rate)
        {
            self.on_tempo_changed(old_value / p_value);
        }
    }

    // Must be called between prepare_process() and process().

    /// C++ `set_global_sequence(MMLSequence *)`.
    fn set_global_sequence(&mut self, p_sequence: Option<SeqRc>) {
        self.base_mut().global_executor.initialize(p_sequence);
    }

    /// C++ `start_global_sequence()`.
    fn start_global_sequence(&mut self) {
        let buffer_length = self.base().buffer_length;
        let base = self.base_mut();
        base.global_buffer_sample_count = buffer_length;
        base.global_execute_sample_count = 0;
        base.global_buffer_index = 0;
    }

    /// C++ `execute_global_sequence()` — returns the executed sample
    /// count. NOTE: a handler that neither advances the pointer nor sets
    /// `_global_execute_sample_count` repeats forever, exactly like the
    /// C++ do-while (callers restart via `start_global_sequence`).
    fn execute_global_sequence(&mut self) -> i32
    where
        Self: Sized,
    {
        self.base_mut().current = Some(CurrentExecutor::Global);

        let mut event = self.base().current_get_pointer();
        self.base_mut().global_execute_sample_count = 0;

        loop {
            match event {
                Some(ev) => {
                    // Update global execute sample count in some event
                    // handlers.
                    let id = {
                        let parser = mml_parser::instance();
                        parser.borrow().events[ev].get_id()
                    };
                    match dispatch_event(self, id, ev) {
                        Dispatch::Handled(next) => {
                            event = next;
                            self.base_mut().current_set_pointer(next);
                        }
                        // This shouldn't happen unless something is very
                        // wrong with our callables.
                        Dispatch::Missing => event = None,
                    }
                }
                None => {
                    let buffer = self.base().global_buffer_sample_count;
                    self.base_mut().global_execute_sample_count = buffer;
                    self.base_mut().global_buffer_sample_count = 0;
                }
            }

            if self.base().global_execute_sample_count != 0 {
                break;
            }
        }

        self.base().global_execute_sample_count
    }

    /// C++ `check_global_sequence_end()`.
    fn check_global_sequence_end(&mut self) -> bool {
        let prev_beat = self.base().global_beat_16th;
        let mut floor_prev_beat = prev_beat as i32;

        {
            let base = self.base_mut();
            base.global_buffer_index += base.global_execute_sample_count;
            let beat_16th_per_sample = base.bpm.borrow().get_beat_16th_per_sample();
            base.global_beat_16th += base.global_execute_sample_count as f64 * beat_16th_per_sample;
        }

        if prev_beat == 0.0 {
            self.on_beat(0, 0);
        } else {
            let floor_curr_beat = self.base().global_beat_16th as i32;
            while floor_prev_beat < floor_curr_beat {
                floor_prev_beat += 1;

                if (floor_prev_beat & self.base().on_beat_callback_filter) == 0 {
                    // C++ computed the double delay and truncated on the
                    // `int` parameter.
                    let delay = (floor_prev_beat as f64 - prev_beat)
                        * self.base().bpm.borrow().get_sample_per_beat_16th();
                    self.on_beat(delay as i32, floor_prev_beat);
                }
            }
        }

        if self.base().global_buffer_sample_count == 0 {
            self.base_mut().global_buffer_index = 0;
            return true;
        }
        false
    }

    /// C++ `process_executor(MMLExecutor *, int)` — process audio by one
    /// executor, returns true if the sequence has ended. See
    /// [`CurrentExecutor`] for the raw-pointer contract.
    fn process_executor(
        &mut self,
        p_executor: *mut MMLExecutor,
        p_buffer_sample_count: i32,
    ) -> bool
    where
        Self: Sized,
    {
        self.base_mut().current = Some(CurrentExecutor::Track(p_executor));
        self.base_mut().process_buffer_sample_count = p_buffer_sample_count;

        let mut event = self.base().current_get_pointer();
        while self.base().process_buffer_sample_count > 0 {
            match event {
                Some(ev) => {
                    // Update process buffer sample count in some event
                    // handlers.
                    let id = {
                        let parser = mml_parser::instance();
                        parser.borrow().events[ev].get_id()
                    };
                    match dispatch_event(self, id, ev) {
                        Dispatch::Handled(next) => {
                            event = next;
                            self.base_mut().current_set_pointer(next);
                        }
                        // This shouldn't happen unless something is very
                        // wrong with our callables. NOTE: C++ drops the
                        // null without storing it back into the executor.
                        Dispatch::Missing => event = None,
                    }
                }
                None => {
                    let nop = self.base().current_get_nop_event();
                    let _ = dispatch_event(self, mml_event::NO_OP, nop);
                    return true;
                }
            }
        }

        false
    }

    /// C++ `parse_table_event(MMLEvent *)` — consumes the table event
    /// after `p_prev` through [`on_table_parse`](Self::on_table_parse).
    fn parse_table_event(&mut self, p_prev: MmlEventRef) {
        let parser = mml_parser::instance();
        let table_event = parser.borrow().events[p_prev]
            .get_next()
            .expect("MMLSequencer: table event is null");
        let table = parser.borrow().get_system_event_string(table_event);

        self.on_table_parse(p_prev, table);

        let next = parser.borrow().events[table_event].get_next();
        parser.borrow_mut().events[p_prev].set_next(next);
        parser.borrow_mut().free_event(table_event); // Free after consumption.
    }
}

/// Outcome of one handler-table lookup.
enum Dispatch {
    Handled(Option<MmlEventRef>),
    Missing,
}

/// Shared body of the two event pumps: clone the slot out of the table
/// (releases the borrow before the handler runs) and invoke it.
fn dispatch_event<T: MMLSequencerTrait>(
    seq: &mut T,
    event_id: i32,
    event: MmlEventRef,
) -> Dispatch {
    let slot = match seq.base().event_handlers.get(event_id as usize) {
        Some(slot) => slot.clone(),
        None => return Dispatch::Missing,
    };

    match slot {
        MmlEventHandler::Empty => Dispatch::Missing,
        MmlEventHandler::NoProcess => Dispatch::Handled(seq.no_process(event)),
        MmlEventHandler::Base(handler) => Dispatch::Handled(match handler {
            MmlBaseHandler::NoProcess => seq.no_process(event),
            MmlBaseHandler::DummyOnProcess => seq.dummy_on_process(event),
            MmlBaseHandler::DummyOnProcessEvent => seq.dummy_on_process_event(event),
            MmlBaseHandler::DefaultOnNoOperation => seq.default_on_no_operation(event),
            MmlBaseHandler::DefaultOnGlobalWait => seq.default_on_global_wait(event),
            MmlBaseHandler::DefaultOnProcess => seq.default_on_process(event),
            MmlBaseHandler::DefaultOnRepeatAll => seq.default_on_repeat_all(event),
            MmlBaseHandler::DefaultOnRepeatBegin => seq.default_on_repeat_begin(event),
            MmlBaseHandler::DefaultOnRepeatBreak => seq.default_on_repeat_break(event),
            MmlBaseHandler::DefaultOnRepeatEnd => seq.default_on_repeat_end(event),
            MmlBaseHandler::DefaultOnSequenceTail => seq.default_on_sequence_tail(event),
            MmlBaseHandler::DefaultOnTempo => seq.default_on_tempo(event),
            MmlBaseHandler::DefaultOnTimer => seq.default_on_timer(event),
            MmlBaseHandler::DefaultOnInternalWait => seq.default_on_internal_wait(event),
            MmlBaseHandler::DefaultOnInternalCall => seq.default_on_internal_call(event),
        }),
        MmlEventHandler::Custom(cb) => {
            let host: &mut dyn MMLSequencerTrait = seq;
            Dispatch::Handled(cb(host, event))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error;

    struct Probe {
        base: MMLSequencerBase,
        prelude: RefCell<Option<String>>,
        processes: RefCell<Vec<(i32, Option<MmlEventRef>)>>,
        beats: RefCell<Vec<(i32, i32)>>,
        timers: RefCell<i32>,
        tempos: RefCell<Vec<f64>>,
        compiled_groups: RefCell<i32>,
    }

    impl Probe {
        fn new() -> Self {
            Self {
                base: MMLSequencerBase::new(),
                prelude: RefCell::new(None),
                processes: RefCell::new(Vec::new()),
                beats: RefCell::new(Vec::new()),
                timers: RefCell::new(0),
                tempos: RefCell::new(Vec::new()),
                compiled_groups: RefCell::new(0),
            }
        }
    }

    impl MMLSequencerTrait for Probe {
        fn base(&self) -> &MMLSequencerBase {
            &self.base
        }

        fn base_mut(&mut self) -> &mut MMLSequencerBase {
            &mut self.base
        }

        fn as_any(&self) -> &dyn Any {
            self
        }

        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }

        fn on_before_compile(&mut self, p_mml: String) -> String {
            self.prelude.borrow().clone().unwrap_or(p_mml)
        }

        fn on_after_compile(&mut self, _p_group: &mut MMLSequenceGroup) {
            *self.compiled_groups.borrow_mut() += 1;
        }

        fn on_process(&mut self, p_length: i32, p_event: Option<MmlEventRef>) {
            self.processes.borrow_mut().push((p_length, p_event));
        }

        fn on_beat(&mut self, p_delay_samples: i32, p_beat_counter: i32) {
            self.beats.borrow_mut().push((p_delay_samples, p_beat_counter));
        }

        fn on_timer_interruption(&mut self) {
            *self.timers.borrow_mut() += 1;
        }

        fn on_tempo_changed(&mut self, p_tempo_ratio: f64) {
            self.tempos.borrow_mut().push(p_tempo_ratio);
        }
    }

    fn make_data() -> Rc<RefCell<MMLData>> {
        mml_parser::initialize();
        let data = Rc::new(RefCell::new(MMLData::new()));
        data.borrow_mut().clear();
        data
    }

    fn make_sequence(events: &[(i32, i32, i32)]) -> SeqRc {
        let sequence = MMLSequence::new(false);
        MMLSequence::initialize(&sequence);
        for (id, data, length) in events {
            MMLSequence::append_new_event(&sequence, *id, *data, *length);
        }
        let head = sequence.borrow().get_head_event().unwrap();
        let parser = mml_parser::instance();
        parser.borrow_mut().events[head].set_data(events.len() as i32);
        sequence
    }

    fn global_event_ids(data: &Rc<RefCell<MMLData>>) -> Vec<i32> {
        let sequence = data.borrow().get_global_sequence();
        let head = sequence.borrow().get_head_event().unwrap();
        let parser = mml_parser::instance();
        let mut ids = Vec::new();
        let mut event = parser.borrow().events[head].get_next();
        while let Some(current) = event {
            let (id, next) = {
                let p = parser.borrow();
                (p.events[current].get_id(), p.events[current].get_next())
            };
            if id == mml_event::SEQUENCE_TAIL {
                break;
            }
            ids.push(id);
            event = next;
        }
        ids
    }

    #[test]
    fn constants_and_temp_executor() {
        assert_eq!(FIXED_BITS, 8);
        assert_eq!(FIXED_FILTER, 255);
        let first = temp_executor();
        let second = temp_executor();
        assert!(Rc::ptr_eq(&first, &second));
    }

    #[test]
    fn bpm_defaults_and_tempo_callback() {
        let mut probe = Probe::new();
        assert!((probe.base.get_default_bpm() - 120.0).abs() < f64::EPSILON);
        probe.base.set_default_bpm(130.0);
        assert!((probe.base.get_default_bpm() - 130.0).abs() < f64::EPSILON);

        probe.set_bpm(150.0);
        assert!((probe.base.get_bpm() - 150.0).abs() < f64::EPSILON);
        assert_eq!(*probe.tempos.borrow(), vec![120.0 / 150.0]);

        probe.set_bpm(150.0);
        assert_eq!(probe.tempos.borrow().len(), 1);
    }

    #[test]
    fn interval_math() {
        let probe = Probe::new();
        let bpm = probe.base.bpm.borrow();
        let sample_per_tick = bpm.get_sample_per_tick();
        let sample_per_beat = bpm.get_sample_per_beat_16th();

        let expected = (1920f64 * sample_per_tick) as i32 >> FIXED_BITS;
        assert_eq!(probe.base.calculate_sample_count(1920), expected);
        assert!(expected > 0);
        assert!((probe.base.calculate_sample_length(4.0) - 4.0 * sample_per_beat).abs() < f64::EPSILON);

        assert!((probe.base.calculate_sample_delay(0, 0.5, 0.0) - 0.5 * sample_per_beat).abs() < 1e-9);
        assert!((probe.base.calculate_sample_delay(0, 0.5, 1.0) - sample_per_beat).abs() < 1e-9);
        assert!((probe.base.calculate_sample_delay(0, 0.5, 4.0) - 4.0 * sample_per_beat).abs() < 1e-9);
    }

    #[test]
    fn listener_registration_and_error_text() {
        let mut probe = Probe::new();
        assert_eq!(probe.base.get_event_id("c"), mml_event::NOTE);
        assert_eq!(probe.base.get_event_id("@q"), mml_event::QUANT_COUNT);
        assert_eq!(probe.base.get_event_id("?"), 0);
        assert_eq!(probe.base.get_event_letter(mml_event::NOTE), "c");

        let id = probe
            .base
            .create_mml_event_listener("zz".to_string(), MmlEventHandler::Empty, true);
        assert_eq!(id, mml_event::USER_DEFINED);
        assert!(probe.base.event_global_flags[id as usize]);
        assert_eq!(probe.base.get_event_id("zz"), id);
        assert_eq!(probe.base.get_event_letter(id), "zz");

        let id2 = probe
            .base
            .create_mml_event_listener("yy".to_string(), MmlEventHandler::Empty, false);
        assert_eq!(id2, mml_event::USER_DEFINED + 1);
        assert!(!probe.base.event_global_flags[id2 as usize]);

        probe
            .base
            .set_default_listener(mml_event::TIMER, MmlBaseHandler::NoProcess, false);
        assert!(matches!(
            probe.base.event_handlers[mml_event::TIMER as usize],
            MmlEventHandler::Base(MmlBaseHandler::NoProcess)
        ));

        // C++ ERR_FAIL_COND_V wording in the sink, plus the
        // ERR_FAIL_COND_MSG wording of prepare_process, both in one test so
        // the process-wide error sink is not raced by parallel tests.
        let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = lines.clone();
        error::set_error_output(Some(Box::new(move |line| {
            sink.lock().unwrap().push(line.to_string());
        })));
        let letter = probe.base.get_event_letter(12345);
        probe.prepare_process(None, 8000, 512);
        error::set_error_output(None);

        assert_eq!(letter, "");
        let lines = lines.lock().unwrap();
        assert!(lines
            .iter()
            .any(|line| line.contains("Condition \"!_event_command_letter_map.has(p_event_id)\"")));
        assert!(lines.iter().any(|line| {
            line.contains("ERROR: MMLSequencer: Sampling rate can only be 22050 or 44100.")
        }));
        assert_eq!(probe.base.get_sample_rate(), 44100);
    }

    #[test]
    fn process_executor_pump_custom_and_nop() {
        let mut probe = Probe::new();
        let sequence = make_sequence(&[(mml_event::NOTE, 60, 100)]);
        let mut executor = MMLExecutor::new();
        executor.initialize(Some(sequence));
        let nop = executor.get_nop_event();

        let hits = Rc::new(RefCell::new(0i32));
        let counter = hits.clone();
        probe.base.event_handlers[mml_event::NOTE as usize] =
            MmlEventHandler::Custom(Rc::new(move |host: &mut dyn MMLSequencerTrait, event| {
                *counter.borrow_mut() += 1;
                // Subclass seam: the concrete instance is reachable again.
                let downcast = host.as_any_mut().downcast_mut::<Probe>();
                assert!(downcast.is_some());
                mml_parser::instance().borrow().events[event].get_next()
            }));

        let raw = &mut executor as *mut MMLExecutor;
        let finished = probe.process_executor(raw, 1000);

        assert!(finished);
        assert_eq!(*hits.borrow(), 1);
        assert_eq!(executor.get_pointer(), None);
        assert_eq!(*probe.processes.borrow(), vec![(1000, Some(nop))]);
        // C++ quirk: the NO_OP handler never decrements the buffer count;
        // the loop is ended by the null-event branch instead.
        assert_eq!(probe.base.process_buffer_sample_count, 1000);
    }

    #[test]
    fn global_sequence_pump_waits_beats_and_timer() {
        let mut probe = Probe::new();
        let data = make_data();
        probe.prepare_process(Some(MmlDataHandle::Base(data.clone())), 44100, 200000);
        assert!(Rc::ptr_eq(&probe.base.bpm, &probe.base.adjustible_bpm));

        let global = data.borrow().get_global_sequence();
        MMLSequence::append_new_event(&global, mml_event::GLOBAL_WAIT, 0, 1920);
        MMLSequence::append_new_event(&global, mml_event::TIMER, 0, 0);
        probe.set_global_sequence(Some(global));
        probe.start_global_sequence();

        let sample_per_tick = probe.base.bpm.borrow().get_sample_per_tick();
        let residue = (1920f64 * sample_per_tick) as i32 >> FIXED_BITS;
        assert_eq!(residue, 88200);

        let first = probe.execute_global_sequence();
        assert_eq!(first, residue);
        assert_eq!(probe.base.global_buffer_sample_count, 200000 - residue);
        assert!(!probe.check_global_sequence_end());
        assert_eq!(*probe.beats.borrow(), vec![(0, 0)]);

        let second = probe.execute_global_sequence();
        assert_eq!(second, 200000 - residue);
        assert_eq!(*probe.timers.borrow(), 1);

        assert!(probe.check_global_sequence_end());
        assert_eq!(probe.base.global_buffer_index, 0);
        let beats = probe.beats.borrow();
        // Sixteen beats advanced: the floor walk fires at 20, 24, 28, 32
        // and 36 with the default (4th beat) filter.
        assert_eq!(beats.len(), 6);
        assert_eq!(beats[1], (22050, 20));
        assert_eq!(beats[5].1, 36);
    }

    #[test]
    fn global_tempo_event_updates_bpm_and_notifies() {
        let mut probe = Probe::new();
        let data = make_data();
        let global = data.borrow().get_global_sequence();
        MMLSequence::append_new_event(&global, mml_event::TEMPO, 150, 0);

        probe.prepare_process(Some(MmlDataHandle::Base(data.clone())), 44100, 1000);
        probe.start_global_sequence();
        let executed = probe.execute_global_sequence();

        assert!(executed > 0);
        assert!((probe.base.get_bpm() - 150.0).abs() < f64::EPSILON);
        assert_eq!(*probe.tempos.borrow(), vec![120.0 / 150.0]);
    }

    #[test]
    fn extract_global_sequence_moves_globals_and_sets_initial_bpm() {
        let mut probe = Probe::new();
        let data = make_data();
        probe.base_mut().mml_data = Some(MmlDataHandle::Base(data.clone()));

        let sequence = {
            let mut data = data.borrow_mut();
            let group = data.get_sequence_group();
            group.append_new_sequence()
        };
        MMLSequence::initialize(&sequence);
        MMLSequence::append_new_event(&sequence, mml_event::TEMPO, 150, 0);
        MMLSequence::append_new_event(&sequence, mml_event::NOTE, 60, 100);
        MMLSequence::append_new_event(&sequence, mml_event::TIMER, 0, 0);
        MMLSequence::append_new_event(&sequence, mml_event::REST, 0, 200);
        let head = sequence.borrow().get_head_event().unwrap();
        mml_parser::instance().borrow_mut().events[head].set_data(4);

        probe.extract_global_sequence();

        assert_eq!(data.borrow_mut().get_sequence_group().get_sequence_count(), 1);
        // The first zero-length TEMPO became the data BPM setting instead
        // of entering the global sequence.
        assert!((data.borrow().get_bpm() - 150.0).abs() < f64::EPSILON);
        assert_eq!(
            global_event_ids(&data),
            vec![mml_event::GLOBAL_WAIT, mml_event::TIMER]
        );
    }

    #[test]
    fn extract_global_sequence_drops_rest_only_sequence() {
        let mut probe = Probe::new();
        let data = make_data();
        probe.base_mut().mml_data = Some(MmlDataHandle::Base(data.clone()));

        let sequence = {
            let mut data = data.borrow_mut();
            let group = data.get_sequence_group();
            group.append_new_sequence()
        };
        MMLSequence::initialize(&sequence);
        MMLSequence::append_new_event(&sequence, mml_event::REST, 0, 200);
        let head = sequence.borrow().get_head_event().unwrap();
        mml_parser::instance().borrow_mut().events[head].set_data(1);

        probe.extract_global_sequence();

        assert!(data
            .borrow_mut()
            .get_sequence_group()
            .get_head_sequence()
            .is_none());
        assert!(global_event_ids(&data).is_empty());
    }

    #[test]
    fn prepare_compile_drops_data_on_empty_preprocess() {
        let mut probe = Probe::new();
        let data = make_data();
        *probe.prelude.borrow_mut() = Some(String::new());

        assert!(!probe.prepare_compile(Some(MmlDataHandle::Base(data.clone())), "c".to_string()));
        assert!(probe.base.mml_data.is_none());
    }

    #[test]
    fn compile_builds_sequences_through_the_parser() {
        let mut probe = Probe::new();
        let data = make_data();

        assert!(probe.prepare_compile(Some(MmlDataHandle::Base(data.clone())), "o4;c;d;".to_string()));
        let mut progress = 0.0;
        for _ in 0..1000 {
            progress = probe.compile(100000);
            if progress >= 1.0 {
                break;
            }
        }
        assert_eq!(progress, 1.0);
        assert_eq!(probe.compiled_groups.borrow().clone(), 1);
        assert!(data.borrow_mut().get_sequence_group().get_sequence_count() >= 1);
        assert!(probe.base.get_event_id("c") == mml_event::NOTE);
    }
}
