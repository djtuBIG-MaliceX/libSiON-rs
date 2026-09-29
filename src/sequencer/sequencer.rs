//! Port of `libSiON-cpp/src/sequencer/simml_sequencer.{h,cpp}` — the MML
//! sequencer that operates [`SiopmSoundChip`] (SiMMLSequencer → SiMMLTrack
//! → SiOPMChannelFM → SiOPMOperator).
//!
//! Design (wave-7c, coordinated across three agent passes):
//! - [`MMLSequencerBase`] is embedded first; the C++ vtable overrides live
//!   in [`MMLSequencerTrait`]. The base `prepare_compile`/`prepare_process`
//!   bodies are reached through [`BaseShim`] (Rust has no `super::` for
//!   trait defaults).
//! - C++ `SiOPMSoundChip *_sound_chip` (non-owning raw pointer) becomes a
//!   shared `Rc<RefCell<SiopmSoundChip>>` handed to the ctor; every track
//!   pump borrows it as the `&mut dyn ChipContext` the port threads through
//!   playback. No threading here: the wave-8 driver pumps this object
//!   from its render callback exactly like C++ `SiONDriver` did.
//! - Song data uses the wave-7c [`MmlDataHandle`] seam: the C++ threads one
//!   aliasing `Ref<MMLData>` and down-refs `Ref<SiMMLData> = mml_data`;
//!   the enum carries both views (see its docs in `base/mml_sequencer.rs`).
//! - C++ `std::function` callback slots become `Option<Rc<dyn Fn...>>`
//!   fields. The wave-8 driver owns the `SiONTrackEvent` queue — this C++
//!   class never built one (it only fires the plain callbacks), so no
//!   event vector is ported; `streaming_latency` is a driver-deferral
//!   field the wave-8 event pump reads when stamping NOTE_*_STREAM events
//!   (C++ pulled sample_rate/latency off the driver at that site).
//! - C++ `~SiMMLSequencer` (delete connector/tracks) is plain RAII here —
//!   no [`Drop`] impl.
//!
//! C++ quirks reproduced: `_initialize_track` passes a fresh *null*
//! `Ref<SiMMLData>` (port: `None`), `_on_table_parse` keeps
//! `set_envelope_table`'s index guard as a silent no-op before still
//! stamping the event, and fixed err-macro condition strings are kept
//! verbatim. The wave-7c C3 pass landed every `_on_mml_*` handler and the
//! listener-registration / defaults region (cpp 915-1744); no stubs
//! remain in this file.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::LazyLock;

use regex::{Captures, Regex};

use crate::chip::ref_table::{
    self as chip_ref_table, SiopmRefTable, VM_DR32DB, VM_DR48DB, VM_LINEAR, VM_MAX,
};
use crate::chip::channels::channel_base::OutputMode;
use crate::chip::channels::ChipContext;
use crate::chip::sound_chip::SiopmSoundChip;
use crate::chip::wave::pcm_table::SiopmWavePcmTable;
use crate::err_fail_cond_msg;
use crate::err_fail_cond_v_msg;
use crate::sequencer::base::mml_data::TCommandMode;
use crate::utils::string::{is_valid_float, itos, to_float, to_int};
use crate::utils::translator_util::TranslatorUtil;
use crate::sequencer::base::mml_event::{self, MmlEventRef};
use crate::sequencer::base::mml_executor::MMLExecutor;
use crate::sequencer::base::mml_executor_connector::MMLExecutorConnector;
use crate::sequencer::base::mml_parser;
use crate::sequencer::base::mml_sequence::MMLSequence;
use crate::sequencer::base::mml_sequence_group::MMLSequenceGroup;
use crate::sequencer::base::mml_sequencer::{
    MmlBaseHandler, MmlDataHandle, MmlEventHandler, MMLSequencerBase, MMLSequencerTrait,
};
use crate::sequencer::base::mml_system_command::MMLSystemCommand;
use crate::sequencer::data::SiMMLData;
use crate::sequencer::envelope_table::SiMMLEnvelopeTable;
use crate::sequencer::ref_table as mml_ref_table;
use crate::sequencer::track::{
    MML_TRACK, MASK_ENVELOPE, MASK_MODULATE, MASK_OPERATOR, MASK_PAN, MASK_QUANTIZE, MASK_SLUR,
    MASK_VOLUME, SiMMLTrack, TrackFn, TrackRc,
};
use crate::sion_enums::{MODULE_GENERIC_PG, MODULE_MAX, PULSE_SQUARE};

/// C++ `MAX_PARAM_COUNT`.
pub const MAX_PARAM_COUNT: usize = 16;

/// `RegExMatch::get_string(p_group)` — empty string when the group did
/// not participate in the match (PCRE2 `PCRE2_UNSET`).
trait CapsStr {
    fn group_str(&self, p_group: usize) -> String;
}

impl CapsStr for Captures<'_> {
    fn group_str(&self, p_group: usize) -> String {
        self.get(p_group)
            .map_or_else(String::new, |m| m.as_str().to_string())
    }
}
/// C++ `MACRO_SIZE`.
pub const MACRO_SIZE: usize = 26;
/// C++ `DEFAULT_MAX_TRACK_COUNT`.
pub const DEFAULT_MAX_TRACK_COUNT: i32 = 128;

/// C++ `std::function<bool(const Ref<SiMMLData>&, const
/// Ref<MMLSystemCommand>&)>` — return false to append the command to
/// `SiONData.system_commands`.
pub type SystemCommandParseFn =
    dyn Fn(&Rc<RefCell<SiMMLData>>, &Rc<RefCell<MMLSystemCommand>>) -> bool;

/// The SiMMLSequencer operates [`SiopmSoundChip`] by MML.
pub struct SiMMLSequencer {
    base: MMLSequencerBase,

    chip: Rc<RefCell<SiopmSoundChip>>,

    #[allow(dead_code)] // C++ `_connector` is only built/destroyed in this
    // file; `_parse_system_command_after` consumes it (wave-7c C2).
    connector: MMLExecutorConnector,

    title: String,

    // Tracks.
    free_tracks: Vec<TrackRc>,
    max_track_count: i32,
    tracks: Vec<TrackRc>,
    current_track: Option<TrackRc>,

    processed_sample_count: i32,
    is_sequence_finished: bool,

    // Compilation and processing.
    dummy_process: bool,
    bpm_change_enabled: bool,

    // Parser.
    #[allow(dead_code)] // filled by the ctor, consumed by macro expansion
    // (wave-7c C2, cpp 476-530).
    macro_strings: Vec<String>,
    #[allow(dead_code)] // consumed by macro expansion (wave-7c C2).
    macro_expand_dynamic: bool,
    internal_table_index: i32,

    // External callbacks.
    callback_event_note_on: Option<TrackFn>,
    callback_event_note_off: Option<TrackFn>,
    callback_tempo_changed: Option<Rc<dyn Fn(i32, bool)>>,
    callback_timer: Option<Rc<dyn Fn()>>,
    callback_beat: Option<Rc<dyn Fn(i32, i32)>>,
    #[allow(dead_code)] // Wired by the wave-8 driver, read by
    // `_try_process_command_callback` (C2).
    callback_parse_system_command: Option<Rc<SystemCommandParseFn>>,

    /// Driver-deferral seam (not a C++ member): wave-8 event pump reads
    /// this when constructing `SiONTrackEvent`s (C++ pulled
    /// `sample_rate`/`streaming_latency` off `SiONDriver` at that site).
    pub streaming_latency: f64,

    //
    envelope_event_id: i32,
}

/// Super-call shim: forwards the [`MMLSequencerTrait`] accessors to the
/// host but keeps the *base* default bodies of `prepare_compile` /
/// `prepare_process` (Rust has no trait `super::`; the base bodies'
/// `_on_before_compile` call is forwarded back to the host's override,
/// exactly like the C++ virtual dispatch).
struct BaseShim<'a> {
    host: &'a mut SiMMLSequencer,
}

impl MMLSequencerTrait for BaseShim<'_> {
    fn base(&self) -> &MMLSequencerBase {
        &self.host.base
    }

    fn base_mut(&mut self) -> &mut MMLSequencerBase {
        &mut self.host.base
    }

    fn as_any(&self) -> &dyn Any {
        self.host
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self.host
    }

    fn on_before_compile(&mut self, p_mml: String) -> String {
        self.host.on_before_compile(p_mml)
    }
}

#[allow(clippy::missing_panics_doc)] // the C++ deref-UB seams panic via
// `expect` instead (documented on each handler)
impl SiMMLSequencer {
    /// C++ `SiMMLSequencer(SiOPMSoundChip *)` — the C++ default argument
    /// (`nullptr`) was UB in `_reset_initial_operator_params`; the port
    /// makes the chip mandatory. The chip arrives as a shared `Rc` the
    /// wave-8 driver pumps (see module docs).
    pub fn new(p_chip: Rc<RefCell<SiopmSoundChip>>) -> Self {
        let mut sequencer = Self {
            base: MMLSequencerBase::new(),
            chip: p_chip,
            connector: MMLExecutorConnector::new(),
            title: String::new(),
            free_tracks: Vec::new(),
            max_track_count: DEFAULT_MAX_TRACK_COUNT,
            tracks: Vec::new(),
            current_track: None,
            processed_sample_count: 0,
            is_sequence_finished: true,
            dummy_process: false,
            bpm_change_enabled: false,
            macro_strings: vec![String::new(); MACRO_SIZE],
            macro_expand_dynamic: false,
            internal_table_index: 0,
            callback_event_note_on: None,
            callback_event_note_off: None,
            callback_tempo_changed: None,
            callback_timer: None,
            callback_beat: None,
            callback_parse_system_command: None,
            streaming_latency: 0.0,
            envelope_event_id: 0,
        };

        sequencer.register_event_listeners();
        sequencer.reset_initial_operator_params();
        sequencer.reset_parser_settings();

        sequencer
    }

    /// C++ `get_title()`.
    pub fn get_title(&self) -> String {
        self.title.clone()
    }

    /// C++ `get_effective_bpm()`.
    pub fn get_effective_bpm(&self) -> f64 {
        if self.is_ready_to_process() {
            self.base.get_bpm()
        } else {
            self.base.get_default_bpm()
        }
    }

    /// C++ `set_effective_bpm(double)`.
    pub fn set_effective_bpm(&mut self, p_value: f64) {
        self.base.set_default_bpm(p_value);

        err_fail_cond_msg!(
            self.is_ready_to_process() && !self.bpm_change_enabled,
            "is_ready_to_process() && !_bpm_change_enabled",
            "SiMMLSequencer: Cannot change BPM while rendering (SiONTrackEvent::NOTE_*_STREAM)."
        );
        self.set_bpm(p_value);
    }

    /// C++ `get_max_track_count()`.
    pub fn get_max_track_count(&self) -> i32 {
        self.max_track_count
    }

    /// C++ `set_max_track_count(int)`.
    pub fn set_max_track_count(&mut self, p_value: i32) {
        self.max_track_count = p_value;
    }

    /// C++ `get_tracks()`.
    pub fn get_tracks(&self) -> Vec<TrackRc> {
        self.tracks.clone()
    }

    /// C++ `get_current_track()`.
    pub fn get_current_track(&self) -> Option<TrackRc> {
        self.current_track.clone()
    }

    /// C++ `get_processed_sample_count()`.
    pub fn get_processed_sample_count(&self) -> i32 {
        self.processed_sample_count
    }

    /// C++ `is_sequence_finished()`.
    pub fn is_sequence_finished(&self) -> bool {
        self.is_sequence_finished
    }

    /// C++ `is_dummy_process()`.
    pub fn is_dummy_process(&self) -> bool {
        self.dummy_process
    }

    /// C++ `get_stream_writing_residue()` — current writing position in
    /// the streaming buffer, always less than the buffer length.
    pub fn get_stream_writing_residue(&self) -> i32 {
        self.base.global_buffer_index
    }

    /// C++ `set_note_on_callback`.
    pub fn set_note_on_callback(&mut self, p_func: Option<TrackFn>) {
        self.callback_event_note_on = p_func;
    }

    /// C++ `set_note_off_callback`.
    pub fn set_note_off_callback(&mut self, p_func: Option<TrackFn>) {
        self.callback_event_note_off = p_func;
    }

    /// C++ `set_tempo_changed_callback` — C++ fires `(int
    /// _global_buffer_index, bool _dummy_process)` (not `double`).
    pub fn set_tempo_changed_callback(&mut self, p_func: Option<Rc<dyn Fn(i32, bool)>>) {
        self.callback_tempo_changed = p_func;
    }

    /// C++ `set_timer_callback`.
    pub fn set_timer_callback(&mut self, p_func: Option<Rc<dyn Fn()>>) {
        self.callback_timer = p_func;
    }

    /// C++ `set_beat_callback`.
    pub fn set_beat_callback(&mut self, p_func: Option<Rc<dyn Fn(i32, i32)>>) {
        self.callback_beat = p_func;
    }

    /// C++ `set_beat_callback_filter(int)`.
    pub fn set_beat_callback_filter(&mut self, p_filter: i32) {
        self.base.on_beat_callback_filter = p_filter;
    }

    /// C++ `_free_all_tracks()`.
    pub fn free_all_tracks(&mut self) {
        for track in self.tracks.drain(..) {
            self.free_tracks.push(track);
        }
    }

    /// C++ `reset_all_tracks()`.
    pub fn reset_all_tracks(&mut self) {
        let settings = self.base.get_parser_settings();
        let (default_volume, default_quant_count, fine_volume) = {
            let settings = settings.borrow();
            (
                settings.default_volume,
                settings.default_quant_count,
                settings.default_fine_volume,
            )
        };
        let ratio = {
            let settings = settings.borrow();
            // C++ `(double)default_quant_ratio / max_quant_ratio` — one
            // cast, double division.
            settings.default_quant_ratio as f64 / settings.max_quant_ratio as f64
        };
        let quant_count = self.base.calculate_sample_count(default_quant_count);

        for track in self.tracks.clone() {
            {
                let chip = self.chip.clone();
                let mut chip = chip.borrow_mut();
                let mut track = track.borrow_mut();
                track.reset(0, &mut *chip);
                track.set_velocity(default_volume);
                track.set_quantize_ratio(ratio);
                track.set_quantize_count(quant_count);
            }
            let channel = track
                .borrow()
                .get_channel()
                .expect("SiMMLSequencer: track channel is null (C++ deref UB)")
                .clone();
            channel.borrow_mut().set_master_volume(fine_volume);
        }

        self.processed_sample_count = 0;
        self.is_sequence_finished = self.tracks.is_empty();
    }

    /// C++ `_initialize_track(SiMMLTrack *, int, bool)`. NOTE quirk: the
    /// C++ passes a freshly constructed null `Ref<SiMMLData>()` to
    /// `initialize` (the track gets no song data) — kept as `None`.
    pub fn initialize_track(&mut self, p_track: &TrackRc, p_internal_track_id: i32, p_disposable: bool) {
        p_track.borrow_mut().initialize(
            None,
            None,
            60,
            if p_internal_track_id >= 0 {
                p_internal_track_id
            } else {
                0
            },
            self.callback_event_note_on.clone(),
            self.callback_event_note_off.clone(),
            p_disposable,
        );

        let buffer_index = self.base.global_buffer_index;
        {
            let chip = self.chip.clone();
            let mut chip = chip.borrow_mut();
            p_track.borrow_mut().reset(buffer_index, &mut *chip);
        }

        let fine_volume = {
            let settings = self.base.get_parser_settings();
            settings.borrow().default_fine_volume
        };
        let channel = p_track
            .borrow()
            .get_channel()
            .expect("SiMMLSequencer: track channel is null (C++ deref UB)")
            .clone();
        channel.borrow_mut().set_master_volume(fine_volume);
    }

    /// C++ `is_ready_to_process()`.
    pub fn is_ready_to_process(&self) -> bool {
        !self.tracks.is_empty()
    }

    /// C++ `is_finished()`.
    pub fn is_finished(&self) -> bool {
        if !self.is_sequence_finished {
            return false;
        }
        for track in &self.tracks {
            if !track.borrow().is_finished() {
                return false;
            }
        }
        true
    }

    /// C++ `stop_sequence()`.
    pub fn stop_sequence(&mut self) {
        self.is_sequence_finished = true;
    }

    /// C++ `process_dummy(int)`.
    pub fn process_dummy(&mut self, p_sample_count: i32) {
        let buffer_count = p_sample_count / self.chip.borrow().get_buffer_length();
        if buffer_count == 0 {
            return;
        }

        // Temporary enable dummy mode and register events.
        self.dummy_process = true;
        self.register_dummy_process_events();

        // Process things.
        for _ in 0..buffer_count {
            self.process();
        }

        // Set everything back to normal.
        self.dummy_process = false;
        self.register_process_events();
    }

    /// C++ `_find_lowest_priority_track()` — scans backwards and keeps the
    /// last (lowest-index) track whose priority ties the running maximum,
    /// exactly like the C++ `>=` compare; returns `None` when the winning
    /// priority is 0 (C++ `nullptr`).
    pub fn find_lowest_priority_track(&mut self) -> Option<TrackRc> {
        let mut index = 0usize;
        let mut max_priority = 0i32;

        for i in (0..self.tracks.len()).rev() {
            let priority = self.tracks[i].borrow().get_priority();
            if priority >= max_priority {
                index = i;
                max_priority = priority;
            }
        }

        if max_priority == 0 {
            return None;
        }
        Some(self.tracks[index].clone())
    }

    /// C++ `find_active_track(int, int)` — `-1` delay matches any active
    /// track with the id; otherwise the start-delay difference must lie in
    /// `(-8, 8)`.
    pub fn find_active_track(&mut self, p_internal_track_id: i32, p_delay: i32) -> Option<TrackRc> {
        for track in self.tracks.clone() {
            let matched = {
                let borrowed = track.borrow();
                if borrowed.get_internal_track_id() != p_internal_track_id || !borrowed.is_active()
                {
                    continue;
                }
                if p_delay == -1 {
                    true
                } else {
                    let diff = borrowed.get_track_start_delay() - p_delay;
                    (-8..8).contains(&diff)
                }
            };

            if matched {
                return Some(track);
            }
        }

        None
    }

    /// C++ `create_controllable_track(int, bool)`. NOTE quirk preserved:
    /// the `_tracks.size() < _max_track_count` compare mixes `size_t` and
    /// `int` (C++ converts the `int` side to unsigned), reproduced with
    /// `as usize`.
    pub fn create_controllable_track(
        &mut self,
        p_internal_track_id: i32,
        p_disposable: bool,
    ) -> Option<TrackRc> {
        for i in (0..self.tracks.len()).rev() {
            let track = self.tracks[i].clone();
            if !track.borrow().is_active() {
                self.initialize_track(&track, p_internal_track_id, p_disposable);
                return Some(track);
            }
        }

        let track = if self.tracks.len() < self.max_track_count as usize {
            let track = match self.free_tracks.pop() {
                Some(track) => track,
                None => Rc::new(RefCell::new(SiMMLTrack::new())),
            };
            let number = self.tracks.len() as i32;
            track.borrow_mut().set_track_number(number);
            self.tracks.push(track.clone());
            track
        } else {
            self.find_lowest_priority_track()?
        };

        self.initialize_track(&track, p_internal_track_id, p_disposable);
        Some(track)
    }

    /// C++ `_expand_macro(sion::String, uint32_t)` — replaces every bare
    /// `A`-`Z` (optionally `(shift)`-annotated) with the macro body,
    /// iterating matches backwards so in-place splices keep earlier
    /// indices valid. The circular-reference guard is the *fixed* version
    /// the C++ comment describes (flags threaded through recursion).
    /// NOTE: the C++ `ERR_FAIL_COND_V_MSG` stringifies the return
    /// expression (`Returning: p_macro`) — the ported print mirrors that.
    pub fn expand_macro(&mut self, p_macro: String, p_macro_flags: u32) -> String {
        static RE_MACRO: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"([A-Z])(\(([\-\d]+)\))?").expect("valid regex"));

        if p_macro.is_empty() {
            return String::new();
        }

        let mut expanded_macro = p_macro.clone();

        type OwnedMacroMatch = (usize, usize, u8, Option<String>, Option<String>);
        let owned: Vec<OwnedMacroMatch> = {
            let snapshot = expanded_macro.clone();
            RE_MACRO
                .captures_iter(&snapshot)
                .map(|caps| {
                    let full = caps.get(0).expect("whole match");
                    let letter = caps.get(1).expect("command group").as_str().as_bytes()[0];
                    let shift_opt = caps
                        .get(3)
                        .map(|m| m.as_str().to_string())
                        .filter(|s| !s.is_empty());
                    (
                        full.start(),
                        full.end(),
                        letter,
                        caps.get(2).map(|m| m.as_str().to_string()),
                        shift_opt,
                    )
                })
                .collect()
        };

        for (start, end, letter, shift_group, shift_value) in owned.into_iter().rev() {
            let index = (letter - b'A') as usize;

            let mut expanded_macro_flags = p_macro_flags;
            let flag = 1u32 << index;
            if expanded_macro_flags & flag != 0 {
                crate::error::err_print_body(
                    &format!(
                        "SiMMLSequencer: Failed to expand a macro due to a circular reference, '{}'.\nCondition \"expanded_macro_flags & flag\" is true. Returning: p_macro",
                        &expanded_macro[start..end]
                    ),
                    false,
                );
                return p_macro;
            }
            expanded_macro_flags |= flag;

            let mut replacement = String::new();
            if !self.macro_strings[index].is_empty() {
                let macro_string = self.macro_strings[index].clone();
                replacement = if self.macro_expand_dynamic {
                    self.expand_macro(macro_string, expanded_macro_flags)
                } else {
                    macro_string
                };

                if shift_group.is_some() {
                    let note_shift = shift_value.map_or(0, |v| to_int(&v) as i32);
                    replacement = format!(
                        "!@ns{}{}!@ns{}",
                        itos(note_shift as i64),
                        replacement,
                        itos(-(note_shift as i64))
                    );
                }
            }

            expanded_macro = format!(
                "{}{}{}",
                &expanded_macro[..start],
                replacement,
                &expanded_macro[end..]
            );
        }

        expanded_macro
    }

    /// C++ `_reset_parser_parameters()`.
    pub fn reset_parser_parameters(&mut self) {
        self.internal_table_index = 511;
        self.title = String::new();

        {
            let settings = self.base.get_parser_settings();
            let mut settings = settings.borrow_mut();
            settings.octave_polarization = 1;
            settings.volume_polarization = 1;
            settings.default_quant_ratio = 6;
            settings.max_quant_ratio = 8;
        }

        self.macro_expand_dynamic = false;
        mml_parser::instance().borrow_mut().set_key_signature("C");

        for slot in self.macro_strings.iter_mut() {
            *slot = String::new();
        }
    }

    /// C++ `_parse_command_init_sequence(const Ref<SiOPMChannelParams>&,
    /// sion::String)` — compiles the postfix into the params' init
    /// sequence, rejecting processing events and `%`/`@`. NOTE: the C++
    /// dereferences a null init sequence unchanged (the C++ ctor
    /// invariant: `ChannelParams::new` always allocates one — `expect`
    /// marks the would-be deref UB).
    pub fn parse_command_init_sequence(
        &mut self,
        p_params: &Rc<RefCell<crate::chip::params::channel_params::ChannelParams>>,
        p_postfix: String,
    ) {
        let sequence = p_params
            .borrow()
            .get_init_sequence()
            .expect("SiMMLSequencer: init sequence is null (C++ deref UB)");

        let settings = self.base.get_parser_settings();
        let parser = mml_parser::instance();
        parser.borrow_mut().prepare_parse(settings, p_postfix.clone());
        let event = match parser.borrow_mut().parse(0) {
            Some(event) => event,
            None => return,
        };
        if parser.borrow().events[event].get_next().is_none() {
            return;
        }
        MMLSequence::cutout(&sequence, event);

        let mut prev = sequence
            .borrow()
            .get_head_event()
            .expect("SiMMLSequencer: head event is null (C++ deref UB)");
        loop {
            let next = match parser.borrow().events[prev].get_next() {
                Some(next) => next,
                None => break,
            };

            let length = parser.borrow().events[next].get_length();
            err_fail_cond_msg!(
                length != 0,
                "next->get_length() != 0",
                format!(
                    "SiMMLSequencer: Initializing sequence cannot contain processing events, '{}'.",
                    p_postfix
                )
            );

            let id = parser.borrow().events[next].get_id();
            err_fail_cond_msg!(
                id == mml_event::MOD_TYPE || id == mml_event::MOD_PARAM,
                "next->get_id() == MMLEvent::MOD_TYPE || next->get_id() == MMLEvent::MOD_PARAM",
                format!(
                    "SiMMLSequencer: Initializing sequence cannot contain '%' or '@', '{}'.",
                    p_postfix
                )
            );

            if id == mml_event::TABLE_EVENT {
                // Parse table events and keep the pointer.
                self.parse_table_event(prev);
            } else {
                // Move to the next event in every other case.
                prev = next;
            }
        }
    }

    /// C++ `_parse_tmode_command(sion::String)`. NOTE: the C++ never
    /// checks the search result for null — a non-matching string was a
    /// deref UB, mirrored by the `expect`.
    pub fn parse_tmode_command(&mut self, p_mml: String) {
        static RE_TCOMMAND: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"(unit|timerb|fps)=?([\d.]*)").expect("valid regex"));

        let res = RE_TCOMMAND
            .captures(&p_mml)
            .expect("SiMMLSequencer: tcommand match is null (C++ deref UB)");

        let value_string = res.get(2).map_or("", |m| m.as_str());
        let value = if is_valid_float(value_string) {
            to_float(value_string)
        } else {
            0.0
        };

        let name = res.get(1).map_or("", |m| m.as_str());
        let handle = self
            .base
            .mml_data
            .clone()
            .expect("SiMMLSequencer: tcommand without mml_data (C++ deref UB)");

        if name == "unit" {
            let mut data = handle.mml_mut();
            data.set_tcommand_mode(TCommandMode::TCOMMAND_BPM);
            data.set_tcommand_resolution(if value > 0.0 { 1.0 / value } else { 1.0 });
        } else if name == "timerb" {
            let mut data = handle.mml_mut();
            data.set_tcommand_mode(TCommandMode::TCOMMAND_TIMERB);
            data.set_tcommand_resolution((if value > 0.0 { value } else { 4000.0 }) * 1.220703125);
        } else if name == "fps" {
            let mut data = handle.mml_mut();
            data.set_tcommand_mode(TCommandMode::TCOMMAND_FRAME);
            data.set_tcommand_resolution(if value > 0.0 { value * 60.0 } else { 3600.0 });
        }
    }

    /// C++ `_parse_vmode_command(sion::String)` — every
    /// `search_all` alternative starts with a literal, so `captures_iter`
    /// matches the PCRE2 global scan exactly.
    pub fn parse_vmode_command(&mut self, p_mml: String) {
        static RE_VCOMMAND: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(n88|mdx|psg|mck|tss|%[xv])(\d*)(\s*,?\s*(\d?))").expect("valid regex")
        });

        let handle = self
            .base
            .mml_data
            .clone()
            .expect("SiMMLSequencer: vmode without mml_data (C++ deref UB)");

        for res in RE_VCOMMAND.captures_iter(&p_mml) {
            let name = res.get(1).map_or("", |m| m.as_str());

            if name == "%v" {
                let mode = to_int(res.get(2).map_or("", |m| m.as_str())) as i32;
                let mut data = handle.mml_mut();
                data.set_default_velocity_mode(if mode >= 0 && (mode as usize) < VM_MAX {
                    mode
                } else {
                    0
                });

                let mut shift = 4i32;
                let shift_string = res.get(4).map_or("", |m| m.as_str());
                if !shift_string.is_empty() {
                    shift = to_int(shift_string) as i32;
                }
                data.set_default_velocity_shift(if (0..8).contains(&shift) { shift } else { 0 });
            } else if name == "%x" {
                let mode = to_int(res.get(2).map_or("", |m| m.as_str())) as i32;
                let mut data = handle.mml_mut();
                data.set_default_expression_mode(if mode >= 0 && (mode as usize) < VM_MAX {
                    mode
                } else {
                    0
                });
            } else if name == "n88" || name == "mdx" {
                let mut data = handle.mml_mut();
                data.set_default_velocity_mode(VM_DR32DB as i32);
                data.set_default_expression_mode(VM_DR48DB as i32);
            } else if name == "psg" {
                let mut data = handle.mml_mut();
                data.set_default_velocity_mode(VM_DR48DB as i32);
                data.set_default_expression_mode(VM_DR48DB as i32);
            } else {
                // mck/tss
                let mut data = handle.mml_mut();
                data.set_default_velocity_mode(VM_LINEAR as i32);
                data.set_default_expression_mode(VM_LINEAR as i32);
            }
        }
    }

    /// C++ `_try_set_sampler_wave(int, sion::String)` — the bank/index
    /// split uses the C++ `>> NOTE_BITS` / mask pair; a missing sound
    /// reference short-circuits to `false` (caller falls back to the
    /// user callback).
    pub fn try_set_sampler_wave(&mut self, p_index: i32, p_mml: String) -> bool {
        let ref_table = chip_ref_table::instance();
        if ref_table.borrow().sound_reference.is_empty() {
            return false;
        }

        let bank =
            (p_index >> SiopmRefTable::NOTE_BITS) & (SiopmRefTable::SAMPLER_TABLE_MAX as i32 - 1);
        let index = p_index & (SiopmRefTable::NOTE_TABLE_SIZE as i32 - 1);

        let handle = self
            .base
            .mml_data
            .clone()
            .expect("SiMMLSequencer: sampler wave without mml_data (C++ deref UB)");
        let simml_data = handle
            .simml()
            .expect("SiMMLSequencer: sampler wave on base-only data (C++ downcast null deref)");
        let table = simml_data
            .borrow()
            .get_sampler_table(bank)
            .expect("SiMMLSequencer: sampler table is null (C++ deref UB)");

        TranslatorUtil::parse_sampler_wave(
            &table,
            index,
            &p_mml,
            &ref_table.borrow().sound_reference,
        )
    }

    /// C++ `_try_set_pcm_wave(int, sion::String)` — a voice whose
    /// `wave_data` is not a PCM table behaves like the C++ null
    /// `Ref<SiOPMWavePCMTable>` downcast result.
    pub fn try_set_pcm_wave(&mut self, p_index: i32, p_mml: String) -> bool {
        let ref_table = chip_ref_table::instance();
        if ref_table.borrow().sound_reference.is_empty() {
            return false;
        }

        let handle = self
            .base
            .mml_data
            .clone()
            .expect("SiMMLSequencer: pcm wave without mml_data (C++ deref UB)");
        let simml_data = handle
            .simml()
            .expect("SiMMLSequencer: pcm wave on base-only data (C++ downcast null deref)");
        let voice = simml_data.borrow_mut().get_pcm_voice(p_index);

        let table = match voice.borrow().wave_data.as_ref() {
            Some(wave) => match wave
                .downcast_ref::<Rc<RefCell<SiopmWavePcmTable>>>()
            {
                Some(table) => table.clone(),
                None => return false,
            },
            None => return false,
        };

        TranslatorUtil::parse_pcm_wave(&table, &p_mml, &ref_table.borrow().sound_reference)
    }

    /// C++ `_try_set_pcm_voice(int, sion::String, sion::String)`. NOTE
    /// quirk: the C++ `voice.is_null()` guard is dead —
    /// `SiMMLData::get_pcm_voice` always creates a blank voice — so the
    /// port (whose getter cannot return null) simply carries on.
    pub fn try_set_pcm_voice(&mut self, p_index: i32, p_mml: String, p_postfix: String) -> bool {
        let ref_table = chip_ref_table::instance();
        if ref_table.borrow().sound_reference.is_empty() {
            return false;
        }

        let handle = self
            .base
            .mml_data
            .clone()
            .expect("SiMMLSequencer: pcm voice without mml_data (C++ deref UB)");
        let simml_data = handle
            .simml()
            .expect("SiMMLSequencer: pcm voice on base-only data (C++ downcast null deref)");
        let voice = simml_data.borrow_mut().get_pcm_voice(p_index);
        let envelopes = simml_data.borrow().get_envelope_tables();

        TranslatorUtil::parse_pcm_voice(&voice, &p_mml, &p_postfix, envelopes)
    }

    /// C++ `_try_process_command_callback(sion::String, int, sion::String,
    /// sion::String)` — hands the parsed command to the user hook; an
    /// unhandled command is appended to `MMLData::system_commands`.
    pub fn try_process_command_callback(
        &mut self,
        p_command: String,
        p_number: i32,
        p_content: String,
        p_postfix: String,
    ) {
        let command_obj = Rc::new(RefCell::new(MMLSystemCommand {
            command: p_command,
            number: p_number,
            content: p_content,
            postfix: p_postfix,
        }));

        let handle = self
            .base
            .mml_data
            .clone()
            .expect("SiMMLSequencer: command callback without mml_data (C++ deref UB)");

        if self.callback_parse_system_command.is_some() {
            let simml_data = handle
                .simml()
                .expect("SiMMLSequencer: command callback on base-only data (C++ downcast null deref)");
            let callback = self.callback_parse_system_command.clone().expect("checked");
            if callback(&simml_data, &command_obj) {
                return;
            }
        }

        // Wasn't parsed, add it to the list.
        handle.mml_mut().add_system_command(command_obj);
    }

    /// C++ `_parse_system_command_before(sion::String, sion::String)` —
    /// returns `true` when the command is consumed (or intentionally
    /// swallowed); `false` defers `#FM` to `_parse_system_command_after`.
    /// NOTE: the `PARSE_TONE_PARAMS` macro's `voice->channel_params` read
    /// after a possibly-null `initialize_voice` was C++ deref UB —
    /// `expect` marks it.
    pub fn parse_system_command_before(&mut self, p_command: String, p_param: String) -> bool {
        static RE_PARAM: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"(?s)\s*(\d*)\s*(\{(.*?)\})?(.*)").expect("valid regex"));

        let res = RE_PARAM
            .captures(&p_param)
            .expect("SiMMLSequencer: param match is null (C++ deref UB)");

        let number = to_int(res.get(1).map_or("", |m| m.as_str())) as i32;
        let has_content = res.get(2).is_some();
        let content = res.get(3).map_or_else(String::new, |m| m.as_str().to_string());
        let postfix = res.get(4).map_or_else(String::new, |m| m.as_str().to_string());

        // Tone settings.
        let tone_parser: Option<fn(&mut crate::chip::params::channel_params::ChannelParams, &str)> =
            match p_command.as_str() {
                "#@" => Some(TranslatorUtil::parse_siopm_params),
                "#OPM@" => Some(TranslatorUtil::parse_opm_params),
                "#OPN@" => Some(TranslatorUtil::parse_opn_params),
                "#OPL@" => Some(TranslatorUtil::parse_opl_params),
                "#OPX@" => Some(TranslatorUtil::parse_opx_params),
                "#MA@" => Some(TranslatorUtil::parse_ma3_params),
                "#AL@" => Some(TranslatorUtil::parse_al_params),
                _ => None,
            };

        if let Some(parse_params) = tone_parser {
            let handle = self
                .base
                .mml_data
                .clone()
                .expect("SiMMLSequencer: tone command without mml_data (C++ deref UB)");
            let simml_data = handle
                .simml()
                .expect("SiMMLSequencer: tone command on base-only data (C++ downcast null deref)");
            let voice = simml_data
                .borrow_mut()
                .initialize_voice(number)
                .expect("SiMMLSequencer: voice is null (C++ deref UB)");
            let params = voice.borrow().channel_params.clone();
            parse_params(&mut params.borrow_mut(), &content);
            if !postfix.is_empty() {
                self.parse_command_init_sequence(&params, postfix);
            }
            return true;
        }

        // Parser settings.
        if p_command == "#TITLE" {
            let handle = self.mml_handle();
            handle
                .mml_mut()
                .set_title(if has_content { content } else { postfix });
            return true;
        }
        if p_command == "#FPS" {
            let fps = if number > 0 {
                number
            } else if has_content {
                to_int(&content) as i32
            } else {
                60
            };
            let handle = self.mml_handle();
            handle.mml_mut().set_default_fps(fps);
            return true;
        }
        if p_command == "#SIGN" {
            mml_parser::instance()
                .borrow_mut()
                .set_key_signature(if has_content { &content } else { &postfix });
            return true;
        }
        if p_command == "#MACRO" {
            let data = if has_content { content } else { postfix };
            if data == "dynamic" {
                self.macro_expand_dynamic = true;
            } else if data == "static" {
                self.macro_expand_dynamic = false;
            } else {
                crate::error::err_print_body(
                    &format!(
                        "SiMMLSequencer: Invalid parameter '{}' for command '{}'.\nMethod/function failed. Returning: true",
                        data, p_command
                    ),
                    false,
                );
            }
            return true;
        }
        if p_command == "#QUANT" {
            if number > 0 {
                let settings = self.base.get_parser_settings();
                let mut settings = settings.borrow_mut();
                settings.max_quant_ratio = number;
                settings.default_quant_ratio = (number as f64 * 0.75) as i32;
            }
            return true;
        }
        if p_command == "#TMODE" {
            self.parse_tmode_command(content);
            return true;
        }
        if p_command == "#VMODE" {
            self.parse_vmode_command(content);
            return true;
        }
        if p_command == "#REV" {
            // Reverse
            let data = if has_content { content } else { postfix };
            if data.is_empty() {
                let settings = self.base.get_parser_settings();
                let mut settings = settings.borrow_mut();
                settings.octave_polarization = -1;
                settings.volume_polarization = -1;
            } else if data == "octave" {
                let settings = self.base.get_parser_settings();
                settings.borrow_mut().octave_polarization = -1;
            } else if data == "volume" {
                let settings = self.base.get_parser_settings();
                settings.borrow_mut().volume_polarization = -1;
            } else {
                crate::error::err_print_body(
                    &format!(
                        "SiMMLSequencer: Invalid parameter '{}' for command '{}'.\nMethod/function failed. Returning: true",
                        data, p_command
                    ),
                    false,
                );
            }
            return true;
        }

        // Tables.
        if p_command == "#TABLE" {
            err_fail_cond_v_msg!(
                !(0..=254).contains(&number),
                "(number < 0 || number > 254)",
                true,
                format!(
                    "SiMMLSequencer: Parameter '{}' for command '{}' is outside of valid range ({} : {}).",
                    number, p_command, 0, 254
                )
            );

            let mut env_table = SiMMLEnvelopeTable::default();
            env_table.parse_mml(&content, &postfix, 65536);
            err_fail_cond_v_msg!(
                env_table.data.is_none(),
                "!env_table->get_data()",
                true,
                format!(
                    "SiMMLSequencer: Invalid parameter '{}' for command '{}'.",
                    content, p_command
                )
            );

            let handle = self.mml_handle();
            let simml_data = handle
                .simml()
                .expect("SiMMLSequencer: table command on base-only data (C++ downcast null deref)");
            simml_data.borrow_mut().set_envelope_table(
                number,
                Some(Rc::new(RefCell::new(env_table))),
            );
            return true;
        }
        if p_command == "#WAV" {
            err_fail_cond_v_msg!(
                !(0..=255).contains(&number),
                "(number < 0 || number > 255)",
                true,
                format!(
                    "SiMMLSequencer: Parameter '{}' for command '{}' is outside of valid range ({} : {}).",
                    number, p_command, 0, 255
                )
            );

            let handle = self.mml_handle();
            let simml_data = handle
                .simml()
                .expect("SiMMLSequencer: wav command on base-only data (C++ downcast null deref)");
            let mut wave_data: Vec<f64> = Vec::new();
            TranslatorUtil::parse_wav(&content, &postfix, &mut wave_data);
            simml_data.borrow_mut().set_wave_table(number, &wave_data);
            return true;
        }
        if p_command == "#WAVB" {
            err_fail_cond_v_msg!(
                !(0..=255).contains(&number),
                "(number < 0 || number > 255)",
                true,
                format!(
                    "SiMMLSequencer: Parameter '{}' for command '{}' is outside of valid range ({} : {}).",
                    number, p_command, 0, 255
                )
            );

            let handle = self.mml_handle();
            let simml_data = handle
                .simml()
                .expect("SiMMLSequencer: wav command on base-only data (C++ downcast null deref)");
            let mut wave_data: Vec<f64> = Vec::new();
            TranslatorUtil::parse_wavb(if has_content { &content } else { &postfix }, &mut wave_data);
            simml_data.borrow_mut().set_wave_table(number, &wave_data);
            return true;
        }

        // PCM voices.
        if p_command == "#SAMPLER" {
            err_fail_cond_v_msg!(
                !(0..=255).contains(&number),
                "(number < 0 || number > 255)",
                true,
                format!(
                    "SiMMLSequencer: Parameter '{}' for command '{}' is outside of valid range ({} : {}).",
                    number, p_command, 0, 255
                )
            );

            if !self.try_set_sampler_wave(number, content.clone()) {
                self.try_process_command_callback(p_command, number, content, postfix);
            }
            return true;
        }
        if p_command == "#PCMWAVE" {
            err_fail_cond_v_msg!(
                !(0..=255).contains(&number),
                "(number < 0 || number > 255)",
                true,
                format!(
                    "SiMMLSequencer: Parameter '{}' for command '{}' is outside of valid range ({} : {}).",
                    number, p_command, 0, 255
                )
            );

            if !self.try_set_pcm_wave(number, content.clone()) {
                self.try_process_command_callback(p_command, number, content, postfix);
            }
            return true;
        }
        if p_command == "#PCMVOICE" {
            err_fail_cond_v_msg!(
                !(0..=255).contains(&number),
                "(number < 0 || number > 255)",
                true,
                format!(
                    "SiMMLSequencer: Parameter '{}' for command '{}' is outside of valid range ({} : {}).",
                    number, p_command, 0, 255
                )
            );

            if !self.try_set_pcm_voice(number, content.clone(), postfix.clone()) {
                self.try_process_command_callback(p_command, number, content, postfix);
            }
            return true;
        }

        // Commands to be handled after parsing.
        if p_command == "#FM" {
            return false;
        }

        // Known but unsupported commands.
        if p_command == "#WAVEXP" || p_command == "#PCMB" || p_command == "#PCMC" {
            crate::error::err_print_body(
                &format!(
                    "SiMMLSequencer: Command '{}' is not supported at this time.",
                    p_command
                ),
                true,
            );
            return true;
        }

        // User defined commands, probably.
        self.try_process_command_callback(p_command, number, content, postfix);
        true
    }

    /// Clone of the base song-data handle; C++ dereferenced the
    /// (possibly null) `mml_data` `Ref` at these sites.
    fn mml_handle(&self) -> MmlDataHandle {
        self.base
            .mml_data
            .clone()
            .expect("SiMMLSequencer: system command without mml_data (C++ deref UB)")
    }

    /// C++ `_parse_system_command_after(MMLSequenceGroup *, MMLSequence
    /// *)` — the `#FM` executor-connector splice. NOTE: C++ still calls
    /// `sequence->get_next_sequence()` after `connect()`; a null connect
    /// result was deref UB, mirrored by the `expect`. The
    /// `res->get_string(1) != "FM"` / empty-body guards return `None`
    /// exactly like C++'s `ERR_FAIL_COND_V_MSG(..., nullptr, ...)`.
    pub fn parse_system_command_after(
        &mut self,
        p_seq_group: &mut MMLSequenceGroup,
        p_command_seq: &crate::sequencer::base::mml_sequence::SeqRc,
    ) -> Option<crate::sequencer::base::mml_sequence::SeqRc> {
        static RE_COMMAND: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"#(FM)[{ \t\r\n]*([^}]*)").expect("valid regex"));

        let command = p_command_seq.borrow().get_system_command();
        let res = RE_COMMAND.captures(&command);

        // Remove it from the chain to skip.
        let mut sequence = MMLSequence::remove_from_chain(p_command_seq);

        // Parse the command.
        if let Some(res) = res {
            let letter = res.get(1).map_or("", |m| m.as_str());
            if letter != "FM" {
                crate::error::err_print_body(
                    &format!(
                        "SiMMLSequencer: Invalid system command letter, '{}'.\nCondition \"res->get_string(1) != \"FM\"\" is true. Returning: nullptr",
                        command
                    ),
                    false,
                );
                return None;
            }

            let formula = res.get(2).map_or("", |m| m.as_str());
            if formula.is_empty() {
                crate::error::err_print_body(
                    &format!(
                        "SiMMLSequencer: Invalid system command syntax, '{}'.\nCondition \"res->get_string(2).empty()\" is true. Returning: nullptr",
                        command
                    ),
                    false,
                );
                return None;
            }

            self.connector.parse(formula);
            sequence = self
                .connector
                .connect(p_seq_group, sequence.expect("SiMMLSequencer: sequence is null (C++ deref UB)"));
        }

        MMLSequence::get_next_sequence(
            &sequence.expect("SiMMLSequencer: sequence is null (C++ deref UB)"),
        )
    }

    /// Body of C++ `GET_EV_PARAMS(m_count)` — a `MAX_PARAM_COUNT`-wide
    /// zero-filled vector filled by `get_parameters` (missing trailing
    /// values become `INT32_MIN`, entries past `m_count` stay `0`);
    /// returns the `(next_event, ev_params)` the macros introduce.
    fn get_ev_params(p_event: MmlEventRef, p_count: i32) -> (MmlEventRef, Vec<i32>) {
        let mut ev_params = vec![0; MAX_PARAM_COUNT];
        let next_event = mml_event::MMLEvent::get_parameters(p_event, &mut ev_params, p_count);
        (next_event, ev_params)
    }

    /// Body of C++ `BIND_EV_PARAM` — the default expression is evaluated
    /// only when the slot holds the `INT32_MIN` sentinel.
    fn bind_ev_param(p_params: &[i32], p_index: usize, p_default: i32) -> i32 {
        if p_params[p_index] != i32::MIN {
            p_params[p_index]
        } else {
            p_default
        }
    }

    /// Body of C++ `BIND_EV_PARAM_RANGE` — in-range (`>= min && < max`)
    /// wins, the default otherwise.
    fn bind_ev_param_range(
        p_params: &[i32],
        p_index: usize,
        p_min: i32,
        p_max: i32,
        p_default: i32,
    ) -> i32 {
        let value = p_params[p_index];
        if value >= p_min && value < p_max {
            value
        } else {
            p_default
        }
    }

    /// C++ `_current_track` dereference (valid while `process()` pumps a
    /// track; null outside the pump == C++ deref UB).
    fn current_track_ref(&self) -> TrackRc {
        self.current_track
            .clone()
            .expect("SiMMLSequencer: _current_track is null (C++ deref UB)")
    }

    /// C++ `_on_mml_rest(MMLEvent *)`.
    pub fn on_mml_rest(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let track = self.current_track_ref();
        track.borrow_mut().handle_rest_event();
        self.base.current_publish_processing_event(p_event)
    }

    /// C++ `_on_mml_note(MMLEvent *)`.
    pub fn on_mml_note(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (data, length) = {
            let p = parser.borrow();
            (p.events[p_event].get_data(), p.events[p_event].get_length())
        };
        let sample_count = self.base.calculate_sample_count(length);

        let track = self.current_track_ref();
        let chip = self.chip.clone();
        let mut chip = chip.borrow_mut();
        track
            .borrow_mut()
            .handle_note_event(data, sample_count, &mut *chip);
        self.base.current_publish_processing_event(p_event)
    }

    /// C++ `_on_mml_driver_note_on(MMLEvent *)` — `p_slur` stays at the
    /// C++ default (`false`).
    pub fn on_mml_driver_note_on(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (data, length) = {
            let p = parser.borrow();
            (p.events[p_event].get_data(), p.events[p_event].get_length())
        };
        let sample_count = self.base.calculate_sample_count(length);

        let track = self.current_track_ref();
        let chip = self.chip.clone();
        let mut chip = chip.borrow_mut();
        track
            .borrow_mut()
            .set_note_immediately(data, sample_count, false, &mut *chip);
        self.base.current_publish_processing_event(p_event)
    }

    /// C++ `_on_mml_slur(MMLEvent *)`.
    pub fn on_mml_slur(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let length = parser.borrow().events[p_event].get_length();
        let sample_count = self.base.calculate_sample_count(length);

        let track = self.current_track_ref();
        {
            let mut track = track.borrow_mut();
            if track.get_event_mask() & MASK_SLUR != 0 {
                track.change_note_length(sample_count);
            } else {
                track.handle_slur();
            }
        }
        self.base.current_publish_processing_event(p_event)
    }

    /// C++ `_on_mml_slur_weak(MMLEvent *)`.
    pub fn on_mml_slur_weak(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let length = parser.borrow().events[p_event].get_length();
        let sample_count = self.base.calculate_sample_count(length);

        let track = self.current_track_ref();
        {
            let mut track = track.borrow_mut();
            if track.get_event_mask() & MASK_SLUR != 0 {
                track.change_note_length(sample_count);
            } else {
                track.handle_slur_weak();
            }
        }
        self.base.current_publish_processing_event(p_event)
    }

    /// C++ `_on_mml_pitch_bend(MMLEvent *)` — NOTE: the bend targets the
    /// NEXT event (`p_event->get_next()`), not `p_event`.
    pub fn on_mml_pitch_bend(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let track = self.current_track_ref();

        if track.borrow().get_event_mask() & MASK_SLUR != 0 {
            let length = parser.borrow().events[p_event].get_length();
            let sample_count = self.base.calculate_sample_count(length);
            track.borrow_mut().change_note_length(sample_count);
        } else {
            let next = parser.borrow().events[p_event].get_next();
            let next_is_note = match next {
                Some(next) => parser.borrow().events[next].get_id() == mml_event::NOTE,
                None => false,
            };
            if !next_is_note {
                return next; // Check the next note.
            }

            let length = parser.borrow().events[p_event].get_length();
            let term = self.base.calculate_sample_count(length);
            let next_data = parser.borrow().events[next.expect("checked above")].get_data();
            track.borrow_mut().handle_pitch_bend(next_data, term);
        }

        self.base.current_publish_processing_event(p_event)
    }

    /// C++ `_on_mml_quant_ratio(MMLEvent *)`.
    pub fn on_mml_quant_ratio(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let data = parser.borrow().events[p_event].get_data();

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_QUANTIZE != 0 {
            return parser.borrow().events[p_event].get_next(); // Check the mask.
        }

        let max_quant_ratio = self.base.get_parser_settings().borrow().max_quant_ratio;
        // C++ `(double)data / max_quant_ratio` — double division.
        track
            .borrow_mut()
            .set_quantize_ratio(data as f64 / max_quant_ratio as f64);
        parser.borrow().events[p_event].get_next()
    }

    /// C++ `_on_mml_quant_count(MMLEvent *)`. NOTE: the C++ scales both
    /// values by the INTEGER `resolution / max_quant_count`.
    pub fn on_mml_quant_count(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 2);
        let mut quant_count = Self::bind_ev_param(&ev_params, 0, 0);
        let mut key_delay = Self::bind_ev_param(&ev_params, 1, 0);

        let (resolution, max_quant_count) = {
            let settings = self.base.get_parser_settings();
            let settings = settings.borrow();
            (settings.resolution, settings.max_quant_count)
        };
        quant_count *= resolution / max_quant_count;
        key_delay *= resolution / max_quant_count;

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_QUANTIZE != 0 {
            return parser.borrow().events[next_event].get_next(); // Check the mask.
        }

        let quant_count = self.base.calculate_sample_count(quant_count);
        let key_delay = self.base.calculate_sample_count(key_delay);
        let mut track = track.borrow_mut();
        track.set_quantize_count(quant_count);
        track.set_key_on_delay(key_delay);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_event_mask(MMLEvent *)`.
    pub fn on_mml_event_mask(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let data = parser.borrow().events[p_event].get_data();

        let track = self.current_track_ref();
        track
            .borrow_mut()
            .set_event_mask(if data == i32::MIN { 0 } else { data } as u32);
        parser.borrow().events[p_event].get_next()
    }

    /// C++ `_on_mml_detune(MMLEvent *)`.
    pub fn on_mml_detune(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let data = parser.borrow().events[p_event].get_data();

        let track = self.current_track_ref();
        track
            .borrow_mut()
            .set_pitch_shift(if data == i32::MIN { 0 } else { data });
        parser.borrow().events[p_event].get_next()
    }

    /// C++ `_on_mml_key_transition(MMLEvent *)`.
    pub fn on_mml_key_transition(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let data = parser.borrow().events[p_event].get_data();

        let track = self.current_track_ref();
        track
            .borrow_mut()
            .set_note_shift(if data == i32::MIN { 0 } else { data });
        parser.borrow().events[p_event].get_next()
    }

    /// C++ `_on_mml_relative_detune(MMLEvent *)`.
    pub fn on_mml_relative_detune(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let data = parser.borrow().events[p_event].get_data();

        let track = self.current_track_ref();
        let shift =
            track.borrow().get_pitch_shift() + if data == i32::MIN { 0 } else { data };
        track.borrow_mut().set_pitch_shift(shift);
        parser.borrow().events[p_event].get_next()
    }

    /// C++ `_on_mml_envelope_fps(MMLEvent *)`.
    pub fn on_mml_envelope_fps(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let data = parser.borrow().events[p_event].get_data();

        let mut frame = if data == i32::MIN || data == 0 { 60 } else { data };
        if frame > 1000 {
            frame = 1000;
        }

        let track = self.current_track_ref();
        track.borrow_mut().set_envelope_fps(frame);
        parser.borrow().events[p_event].get_next()
    }

    /// C++ `_on_mml_tone_envelope(MMLEvent *)`. NOTE: the envelope index
    /// is clamped to `[-1, 255)` (`-1` = no table), the step default is 1.
    pub fn on_mml_tone_envelope(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 2);
        let idx = Self::bind_ev_param_range(&ev_params, 0, 0, 255, -1);
        let step = Self::bind_ev_param(&ev_params, 1, 1);

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_ENVELOPE != 0 {
            return parser.borrow().events[next_event].get_next(); // Check the mask.
        }

        let env_table = if idx >= 0 {
            mml_ref_table::instance()
                .expect("SiMMLRefTable not initialized")
                .borrow()
                .get_envelope_table(idx)
        } else {
            None
        };
        track.borrow_mut().set_tone_envelope(1, &env_table, step);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_amplitude_envelope(MMLEvent *)`.
    pub fn on_mml_amplitude_envelope(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 2);
        let idx = Self::bind_ev_param_range(&ev_params, 0, 0, 255, -1);
        let step = Self::bind_ev_param(&ev_params, 1, 1);

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_ENVELOPE != 0 {
            return parser.borrow().events[next_event].get_next(); // Check the mask.
        }

        let env_table = if idx >= 0 {
            mml_ref_table::instance()
                .expect("SiMMLRefTable not initialized")
                .borrow()
                .get_envelope_table(idx)
        } else {
            None
        };
        track.borrow_mut().set_amplitude_envelope(1, &env_table, step, false);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_amplitude_envelope_tsscp(MMLEvent *)` — `!na` runs
    /// the amplitude envelope with the `true` offset flag.
    pub fn on_mml_amplitude_envelope_tsscp(
        &mut self,
        p_event: MmlEventRef,
    ) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 2);
        let idx = Self::bind_ev_param_range(&ev_params, 0, 0, 255, -1);
        let step = Self::bind_ev_param(&ev_params, 1, 1);

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_ENVELOPE != 0 {
            return parser.borrow().events[next_event].get_next(); // Check the mask.
        }

        let env_table = if idx >= 0 {
            mml_ref_table::instance()
                .expect("SiMMLRefTable not initialized")
                .borrow()
                .get_envelope_table(idx)
        } else {
            None
        };
        track.borrow_mut().set_amplitude_envelope(1, &env_table, step, true);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_pitch_envelope(MMLEvent *)`.
    pub fn on_mml_pitch_envelope(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 2);
        let idx = Self::bind_ev_param_range(&ev_params, 0, 0, 255, -1);
        let step = Self::bind_ev_param(&ev_params, 1, 1);

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_ENVELOPE != 0 {
            return parser.borrow().events[next_event].get_next(); // Check the mask.
        }

        let env_table = if idx >= 0 {
            mml_ref_table::instance()
                .expect("SiMMLRefTable not initialized")
                .borrow()
                .get_envelope_table(idx)
        } else {
            None
        };
        track.borrow_mut().set_pitch_envelope(1, &env_table, step);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_note_envelope(MMLEvent *)`.
    pub fn on_mml_note_envelope(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 2);
        let idx = Self::bind_ev_param_range(&ev_params, 0, 0, 255, -1);
        let step = Self::bind_ev_param(&ev_params, 1, 1);

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_ENVELOPE != 0 {
            return parser.borrow().events[next_event].get_next(); // Check the mask.
        }

        let env_table = if idx >= 0 {
            mml_ref_table::instance()
                .expect("SiMMLRefTable not initialized")
                .borrow()
                .get_envelope_table(idx)
        } else {
            None
        };
        track.borrow_mut().set_note_envelope(1, &env_table, step);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_filter_envelope(MMLEvent *)`.
    pub fn on_mml_filter_envelope(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 2);
        let idx = Self::bind_ev_param_range(&ev_params, 0, 0, 255, -1);
        let step = Self::bind_ev_param(&ev_params, 1, 1);

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_ENVELOPE != 0 {
            return parser.borrow().events[next_event].get_next(); // Check the mask.
        }

        let env_table = if idx >= 0 {
            mml_ref_table::instance()
                .expect("SiMMLRefTable not initialized")
                .borrow()
                .get_envelope_table(idx)
        } else {
            None
        };
        track.borrow_mut().set_filter_envelope(1, &env_table, step);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_tone_release_envelope(MMLEvent *)` — the `note_on`
    /// slot is `0` (release) for the whole `_` family.
    pub fn on_mml_tone_release_envelope(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 2);
        let idx = Self::bind_ev_param_range(&ev_params, 0, 0, 255, -1);
        let step = Self::bind_ev_param(&ev_params, 1, 1);

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_ENVELOPE != 0 {
            return parser.borrow().events[next_event].get_next(); // Check the mask.
        }

        let env_table = if idx >= 0 {
            mml_ref_table::instance()
                .expect("SiMMLRefTable not initialized")
                .borrow()
                .get_envelope_table(idx)
        } else {
            None
        };
        track.borrow_mut().set_tone_envelope(0, &env_table, step);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_amplitude_release_envelope(MMLEvent *)`.
    pub fn on_mml_amplitude_release_envelope(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 2);
        let idx = Self::bind_ev_param_range(&ev_params, 0, 0, 255, -1);
        let step = Self::bind_ev_param(&ev_params, 1, 1);

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_ENVELOPE != 0 {
            return parser.borrow().events[next_event].get_next(); // Check the mask.
        }

        let env_table = if idx >= 0 {
            mml_ref_table::instance()
                .expect("SiMMLRefTable not initialized")
                .borrow()
                .get_envelope_table(idx)
        } else {
            None
        };
        track
            .borrow_mut()
            .set_amplitude_envelope(0, &env_table, step, false);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_pitch_release_envelope(MMLEvent *)`.
    pub fn on_mml_pitch_release_envelope(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 2);
        let idx = Self::bind_ev_param_range(&ev_params, 0, 0, 255, -1);
        let step = Self::bind_ev_param(&ev_params, 1, 1);

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_ENVELOPE != 0 {
            return parser.borrow().events[next_event].get_next(); // Check the mask.
        }

        let env_table = if idx >= 0 {
            mml_ref_table::instance()
                .expect("SiMMLRefTable not initialized")
                .borrow()
                .get_envelope_table(idx)
        } else {
            None
        };
        track.borrow_mut().set_pitch_envelope(0, &env_table, step);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_note_release_envelope(MMLEvent *)`.
    pub fn on_mml_note_release_envelope(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 2);
        let idx = Self::bind_ev_param_range(&ev_params, 0, 0, 255, -1);
        let step = Self::bind_ev_param(&ev_params, 1, 1);

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_ENVELOPE != 0 {
            return parser.borrow().events[next_event].get_next(); // Check the mask.
        }

        let env_table = if idx >= 0 {
            mml_ref_table::instance()
                .expect("SiMMLRefTable not initialized")
                .borrow()
                .get_envelope_table(idx)
        } else {
            None
        };
        track.borrow_mut().set_note_envelope(0, &env_table, step);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_filter_release_envelope(MMLEvent *)`.
    pub fn on_mml_filter_release_envelope(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 2);
        let idx = Self::bind_ev_param_range(&ev_params, 0, 0, 255, -1);
        let step = Self::bind_ev_param(&ev_params, 1, 1);

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_ENVELOPE != 0 {
            return parser.borrow().events[next_event].get_next(); // Check the mask.
        }

        let env_table = if idx >= 0 {
            mml_ref_table::instance()
                .expect("SiMMLRefTable not initialized")
                .borrow()
                .get_envelope_table(idx)
        } else {
            None
        };
        track.borrow_mut().set_filter_envelope(0, &env_table, step);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_filter(MMLEvent *)` — the ten `@f` parameters with
    /// their C++ defaults.
    pub fn on_mml_filter(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 10);
        let cut = Self::bind_ev_param(&ev_params, 0, 128);
        let res = Self::bind_ev_param(&ev_params, 1, 0);
        let ar = Self::bind_ev_param(&ev_params, 2, 0);
        let dr1 = Self::bind_ev_param(&ev_params, 3, 0);
        let dr2 = Self::bind_ev_param(&ev_params, 4, 0);
        let rr = Self::bind_ev_param(&ev_params, 5, 0);
        let dc1 = Self::bind_ev_param(&ev_params, 6, 128);
        let dc2 = Self::bind_ev_param(&ev_params, 7, 64);
        let sc = Self::bind_ev_param(&ev_params, 8, 32);
        let rc = Self::bind_ev_param(&ev_params, 9, 128);

        let track = self.current_track_ref();
        let channel = track
            .borrow()
            .get_channel()
            .expect("SiMMLSequencer: channel is null (C++ deref UB)")
            .clone();
        channel
            .borrow_mut()
            .set_sv_filter(cut, res, ar, dr1, dr2, rr, dc1, dc2, sc, rc);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_filter_mode(MMLEvent *)`.
    pub fn on_mml_filter_mode(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let data = parser.borrow().events[p_event].get_data();

        let track = self.current_track_ref();
        let channel = track
            .borrow()
            .get_channel()
            .expect("SiMMLSequencer: channel is null (C++ deref UB)")
            .clone();
        channel.borrow_mut().set_filter_type(data);
        parser.borrow().events[p_event].get_next()
    }

    /// C++ `_on_mml_lf_oscillator(MMLEvent *)`. NOTE: the custom-table
    /// branch reads the RAW `ev_params[1]` (an `INT32_MIN` sentinel would
    /// hit the table's index guard), `cycle_time` scales by the INTEGER
    /// `1000/60`, and the result feeds the `double` setter by widening.
    pub fn on_mml_lf_oscillator(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 2);
        let mut cycle_time = Self::bind_ev_param(&ev_params, 0, 20); // One third of a second.
        let waveform = Self::bind_ev_param(
            &ev_params,
            1,
            chip_ref_table::LFO_WAVE_TRIANGLE as i32,
        );

        cycle_time *= 1000 / 60; // Convert to ms.

        let track = self.current_track_ref();
        let channel = track
            .borrow()
            .get_channel()
            .expect("SiMMLSequencer: channel is null (C++ deref UB)")
            .clone();

        if waveform > 7 && waveform < 255 {
            let ev_table = mml_ref_table::instance()
                .expect("SiMMLRefTable not initialized")
                .borrow()
                .get_envelope_table(ev_params[1]);
            match ev_table {
                Some(ev_table) => {
                    let mut table_vector = Vec::new();
                    ev_table
                        .borrow_mut()
                        .to_vector(256, &mut table_vector, 0, 255);
                    channel.borrow_mut().initialize_lfo(-1, table_vector);
                }
                None => {
                    channel
                        .borrow_mut()
                        .initialize_lfo(chip_ref_table::LFO_WAVE_TRIANGLE as i32, Vec::new());
                }
            }
        } else {
            channel.borrow_mut().initialize_lfo(waveform, Vec::new());
        }

        channel.borrow_mut().set_lfo_cycle_time(cycle_time as f64);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_pitch_modulation(MMLEvent *)`.
    pub fn on_mml_pitch_modulation(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 4);
        let depth = Self::bind_ev_param(&ev_params, 0, 0);
        let end_depth = Self::bind_ev_param(&ev_params, 1, 0);
        let delay = Self::bind_ev_param(&ev_params, 2, 0);
        let term = Self::bind_ev_param(&ev_params, 3, 0);

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_MODULATE != 0 {
            return parser.borrow().events[next_event].get_next(); // Check the mask.
        }

        track
            .borrow_mut()
            .set_modulation_envelope(true, depth, end_depth, delay, term);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_amplitude_modulation(MMLEvent *)`.
    pub fn on_mml_amplitude_modulation(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 4);
        let depth = Self::bind_ev_param(&ev_params, 0, 0);
        let end_depth = Self::bind_ev_param(&ev_params, 1, 0);
        let delay = Self::bind_ev_param(&ev_params, 2, 0);
        let term = Self::bind_ev_param(&ev_params, 3, 0);

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_MODULATE != 0 {
            return parser.borrow().events[next_event].get_next(); // Check the mask.
        }

        track
            .borrow_mut()
            .set_modulation_envelope(false, depth, end_depth, delay, term);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_portament(MMLEvent *)`.
    pub fn on_mml_portament(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let data = parser.borrow().events[p_event].get_data();

        let track = self.current_track_ref();
        track
            .borrow_mut()
            .set_portament(if data == i32::MIN { 0 } else { data });
        parser.borrow().events[p_event].get_next()
    }

    /// C++ `_on_mml_volume(MMLEvent *)` — `data` is the raw velocity
    /// (`data << 3 = 16->128` per the C++ comment).
    pub fn on_mml_volume(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let data = parser.borrow().events[p_event].get_data();

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_VOLUME != 0 {
            return parser.borrow().events[p_event].get_next(); // Check the mask.
        }

        track.borrow_mut().handle_velocity(data);
        parser.borrow().events[p_event].get_next()
    }

    /// C++ `_on_mml_volume_shift(MMLEvent *)` — shifted raw velocity
    /// (`data << 3 = 16->128` per the C++ comment).
    pub fn on_mml_volume_shift(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let data = parser.borrow().events[p_event].get_data();

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_VOLUME != 0 {
            return parser.borrow().events[p_event].get_next(); // Check the mask.
        }

        track.borrow_mut().handle_velocity_shift(data);
        parser.borrow().events[p_event].get_next()
    }

    /// C++ `_on_mml_volume_setting(MMLEvent *)` — `%v`.
    pub fn on_mml_volume_setting(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) =
            Self::get_ev_params(p_event, SiopmSoundChip::STREAM_SEND_SIZE as i32);
        let velocity_mode = Self::bind_ev_param(&ev_params, 0, 0);
        let velocity_shift = Self::bind_ev_param(&ev_params, 1, 4);

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_VOLUME != 0 {
            return parser.borrow().events[next_event].get_next(); // Check the mask.
        }

        let mut track = track.borrow_mut();
        track.set_velocity_mode(velocity_mode);
        track.set_velocity_shift(velocity_shift);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_expression(MMLEvent *)`.
    pub fn on_mml_expression(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let data = parser.borrow().events[p_event].get_data();

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_VOLUME != 0 {
            return parser.borrow().events[p_event].get_next(); // Check the mask.
        }

        track
            .borrow_mut()
            .set_expression(if data == i32::MIN { 128 } else { data });
        parser.borrow().events[p_event].get_next()
    }

    /// C++ `_on_mml_expression_setting(MMLEvent *)` — `%x`.
    pub fn on_mml_expression_setting(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let data = parser.borrow().events[p_event].get_data();

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_VOLUME != 0 {
            return parser.borrow().events[p_event].get_next(); // Check the mask.
        }

        track
            .borrow_mut()
            .set_expression_mode(if data == i32::MIN { 0 } else { data });
        parser.borrow().events[p_event].get_next()
    }

    /// C++ `_on_mml_master_volume(MMLEvent *)` — `@v`/`FINE_VOLUME`.
    /// NOTE: like the C++ the 16-slot `ev_params` vector (only
    /// `STREAM_SEND_SIZE` slots parsed) is handed to the channel verbatim.
    pub fn on_mml_master_volume(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) =
            Self::get_ev_params(p_event, SiopmSoundChip::STREAM_SEND_SIZE as i32);

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_VOLUME != 0 {
            return parser.borrow().events[next_event].get_next(); // Check the mask.
        }

        let channel = track
            .borrow()
            .get_channel()
            .expect("SiMMLSequencer: channel is null (C++ deref UB)")
            .clone();
        channel
            .borrow_mut()
            .set_all_stream_send_levels(ev_params);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_pan(MMLEvent *)`. NOTE: the sentinel means centre `0`;
    /// a real value is widened `(data << 4) - 64`.
    pub fn on_mml_pan(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let data = parser.borrow().events[p_event].get_data();

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_PAN != 0 {
            return parser.borrow().events[p_event].get_next(); // Check the mask.
        }

        let pan = if data == i32::MIN {
            0
        } else {
            (data << 4) - 64
        };
        let channel = track
            .borrow()
            .get_channel()
            .expect("SiMMLSequencer: channel is null (C++ deref UB)")
            .clone();
        channel.borrow_mut().set_pan(pan);
        parser.borrow().events[p_event].get_next()
    }

    /// C++ `_on_mml_fine_pan(MMLEvent *)` — `@p`, already `0..127`.
    pub fn on_mml_fine_pan(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let data = parser.borrow().events[p_event].get_data();

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_PAN != 0 {
            return parser.borrow().events[p_event].get_next(); // Check the mask.
        }

        let channel = track
            .borrow()
            .get_channel()
            .expect("SiMMLSequencer: channel is null (C++ deref UB)")
            .clone();
        channel
            .borrow_mut()
            .set_pan(if data == i32::MIN { 0 } else { data });
        parser.borrow().events[p_event].get_next()
    }

    /// C++ `_on_mml_input(MMLEvent *)` — `@i`.
    pub fn on_mml_input(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 2);
        let level = Self::bind_ev_param(&ev_params, 0, 5);
        let index = Self::bind_ev_param(&ev_params, 1, 0);

        let track = self.current_track_ref();
        let channel = track
            .borrow()
            .get_channel()
            .expect("SiMMLSequencer: channel is null (C++ deref UB)")
            .clone();
        let chip = self.chip.clone();
        let mut chip = chip.borrow_mut();
        channel.borrow_mut().set_input(level, index, &mut *chip);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_output(MMLEvent *)` — `@o`. NOTE: the C++ cast
    /// `(OutputMode)mode` is unchecked; `set_output` only ever compares
    /// against `Standard`/`Add`, so the port maps `0->Standard`,
    /// `2->Add` and every other value behaves like `Overwrite`.
    pub fn on_mml_output(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 2);
        let mode = Self::bind_ev_param(&ev_params, 0, 2);
        let index = Self::bind_ev_param(&ev_params, 1, 0);

        let track = self.current_track_ref();
        let channel = track
            .borrow()
            .get_channel()
            .expect("SiMMLSequencer: channel is null (C++ deref UB)")
            .clone();
        let chip = self.chip.clone();
        let mut chip = chip.borrow_mut();
        let output_mode = match mode {
            0 => OutputMode::Standard,
            2 => OutputMode::Add,
            _ => OutputMode::Overwrite,
        };
        channel
            .borrow_mut()
            .set_output(output_mode, index, &mut *chip);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_ring_modulation(MMLEvent *)` — `@r`.
    pub fn on_mml_ring_modulation(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 2);
        let level = Self::bind_ev_param(&ev_params, 0, 4);
        let index = Self::bind_ev_param(&ev_params, 1, 0);

        let track = self.current_track_ref();
        let channel = track
            .borrow()
            .get_channel()
            .expect("SiMMLSequencer: channel is null (C++ deref UB)")
            .clone();
        let chip = self.chip.clone();
        let mut chip = chip.borrow_mut();
        channel
            .borrow_mut()
            .set_ring_modulation(level, index, &mut *chip);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_module_type(MMLEvent *)` — `%m,n`. NOTE: the C++
    /// in-range check (`0..MODULE_MAX`) keeps `MODULE_GENERIC_PG` as the
    /// default; `channel_num` keeps its `INT32_MIN` sentinel (auto
    /// channel); `p_tone_num` is the C++ default (`INT32_MIN`).
    pub fn on_mml_module_type(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 2);
        let r#type = Self::bind_ev_param_range(
            &ev_params,
            0,
            0,
            MODULE_MAX,
            MODULE_GENERIC_PG,
        );
        let channel_num = Self::bind_ev_param(&ev_params, 1, i32::MIN);

        let track = self.current_track_ref();
        let chip = self.chip.clone();
        let mut chip = chip.borrow_mut();
        track
            .borrow_mut()
            .set_channel_module_type(r#type, channel_num, i32::MIN, &mut *chip);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_event_trigger(MMLEvent *)` — `%t`.
    pub fn on_mml_event_trigger(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 3);
        let id = Self::bind_ev_param(&ev_params, 0, 0);
        let type_on = Self::bind_ev_param(&ev_params, 1, 1);
        let type_off = Self::bind_ev_param(&ev_params, 2, 1);

        let track = self.current_track_ref();
        track
            .borrow_mut()
            .set_event_trigger_callbacks(id, type_on, type_off);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_dispatch_event(MMLEvent *)` — `%e`.
    pub fn on_mml_dispatch_event(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 2);
        let id = Self::bind_ev_param(&ev_params, 0, 0);
        let type_on = Self::bind_ev_param(&ev_params, 1, 1);

        let track = self.current_track_ref();
        track.borrow_mut().trigger_note_on_event(id, type_on);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_clock(MMLEvent *)` — `@clock`.
    pub fn on_mml_clock(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let data = parser.borrow().events[p_event].get_data();

        let track = self.current_track_ref();
        let channel = track
            .borrow()
            .get_channel()
            .expect("SiMMLSequencer: channel is null (C++ deref UB)")
            .clone();
        channel
            .borrow_mut()
            .set_frequency_ratio(if data == i32::MIN { 100 } else { data });
        parser.borrow().events[p_event].get_next()
    }

    /// C++ `_on_mml_algorithm(MMLEvent *)` — `@al`. NOTE: the default
    /// algorithm reads `ALGORITHM_INIT[op_count]` lazily (only when the
    /// slot is the sentinel), exactly like the C++ ternary; an
    /// out-of-range `op_count` is OOB UB in C++ and panics here.
    pub fn on_mml_algorithm(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 2);
        let op_count = Self::bind_ev_param(&ev_params, 0, 0);
        let algorithm = if ev_params[1] != i32::MIN {
            ev_params[1]
        } else {
            mml_ref_table::ALGORITHM_INIT[op_count as usize]
        };

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_OPERATOR != 0 {
            return parser.borrow().events[next_event].get_next(); // Check the mask.
        }

        let channel = track
            .borrow()
            .get_channel()
            .expect("SiMMLSequencer: channel is null (C++ deref UB)")
            .clone();
        let chip = self.chip.clone();
        let mut chip = chip.borrow_mut();
        channel
            .borrow_mut()
            .set_algorithm(op_count, false, algorithm, &mut *chip);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_operator_parameter(MMLEvent *)` — `@` (`MOD_PARAM`).
    /// NOTE: a returned init sequence is spliced in with
    /// `connect_before` and the walk resumes at its SECOND event.
    pub fn on_mml_operator_parameter(
        &mut self,
        p_event: MmlEventRef,
    ) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, MAX_PARAM_COUNT as i32);

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_OPERATOR != 0 {
            return parser.borrow().events[next_event].get_next(); // Check the mask.
        }

        let chip = self.chip.clone();
        let mut chip = chip.borrow_mut();
        let sequence = track.borrow_mut().set_channel_parameters(ev_params, &mut *chip);
        drop(chip);

        if let Some(sequence) = sequence {
            let next = parser.borrow().events[next_event].get_next();
            MMLSequence::connect_before(&sequence, next);
            let head = sequence
                .borrow()
                .get_head_event()
                .expect("SiMMLSequencer: head event is null (C++ deref UB)");
            return parser.borrow().events[head].get_next();
        }

        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_feedback(MMLEvent *)` — `@fb`.
    pub fn on_mml_feedback(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 2);
        let level = Self::bind_ev_param(&ev_params, 0, 0);
        let connection = Self::bind_ev_param(&ev_params, 1, 0);

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_OPERATOR != 0 {
            return parser.borrow().events[next_event].get_next(); // Check the mask.
        }

        let channel = track
            .borrow()
            .get_channel()
            .expect("SiMMLSequencer: channel is null (C++ deref UB)")
            .clone();
        let chip = self.chip.clone();
        let mut chip = chip.borrow_mut();
        channel
            .borrow_mut()
            .set_feedback(level, connection, &mut *chip);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_slot_index(MMLEvent *)` — `i`; sentinel means slot 4.
    pub fn on_mml_slot_index(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let data = parser.borrow().events[p_event].get_data();

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_OPERATOR != 0 {
            return parser.borrow().events[p_event].get_next(); // Check the mask.
        }

        let channel = track
            .borrow()
            .get_channel()
            .expect("SiMMLSequencer: channel is null (C++ deref UB)")
            .clone();
        channel
            .borrow_mut()
            .set_active_operator_index(if data == i32::MIN { 4 } else { data });
        parser.borrow().events[p_event].get_next()
    }

    /// C++ `_on_mml_operator_release_rate(MMLEvent *)` — `@rr`. NOTE:
    /// the sweep is set even when the release-rate slot is the sentinel.
    pub fn on_mml_operator_release_rate(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 2);
        let release_rate = Self::bind_ev_param(&ev_params, 0, i32::MIN);
        let release_sweep = Self::bind_ev_param(&ev_params, 1, 0);

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_OPERATOR != 0 {
            return parser.borrow().events[next_event].get_next(); // Check the mask.
        }

        if release_rate != i32::MIN {
            let channel = track
                .borrow()
                .get_channel()
                .expect("SiMMLSequencer: channel is null (C++ deref UB)")
                .clone();
            channel.borrow_mut().set_release_rate(release_rate);
        }

        track.borrow_mut().set_release_sweep(release_sweep);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_operator_total_level(MMLEvent *)` — `@tl`.
    pub fn on_mml_operator_total_level(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let data = parser.borrow().events[p_event].get_data();

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_OPERATOR != 0 {
            return parser.borrow().events[p_event].get_next(); // Check the mask.
        }

        let channel = track
            .borrow()
            .get_channel()
            .expect("SiMMLSequencer: channel is null (C++ deref UB)")
            .clone();
        channel
            .borrow_mut()
            .set_total_level(if data == i32::MIN { 0 } else { data });
        parser.borrow().events[p_event].get_next()
    }

    /// C++ `_on_mml_operator_multiple(MMLEvent *)` — `@ml`,
    /// `(base << 7) + offset` into the fine multiple.
    pub fn on_mml_operator_multiple(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 2);
        let value_base = Self::bind_ev_param(&ev_params, 0, 0);
        let value_offset = Self::bind_ev_param(&ev_params, 1, 0);

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_OPERATOR != 0 {
            return parser.borrow().events[next_event].get_next(); // Check the mask.
        }

        let channel = track
            .borrow()
            .get_channel()
            .expect("SiMMLSequencer: channel is null (C++ deref UB)")
            .clone();
        channel
            .borrow_mut()
            .set_fine_multiple((value_base << 7) + value_offset);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_operator_detune(MMLEvent *)` — `@dt`.
    pub fn on_mml_operator_detune(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let data = parser.borrow().events[p_event].get_data();

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_OPERATOR != 0 {
            return parser.borrow().events[p_event].get_next(); // Check the mask.
        }

        let channel = track
            .borrow()
            .get_channel()
            .expect("SiMMLSequencer: channel is null (C++ deref UB)")
            .clone();
        channel
            .borrow_mut()
            .set_detune(if data == i32::MIN { 0 } else { data });
        parser.borrow().events[p_event].get_next()
    }

    /// C++ `_on_mml_operator_phase(MMLEvent *)` — `@ph`.
    pub fn on_mml_operator_phase(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let data = parser.borrow().events[p_event].get_data();

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_OPERATOR != 0 {
            return parser.borrow().events[p_event].get_next(); // Check the mask.
        }

        let channel = track
            .borrow()
            .get_channel()
            .expect("SiMMLSequencer: channel is null (C++ deref UB)")
            .clone();
        channel
            .borrow_mut()
            .set_phase(if data == i32::MIN { 0 } else { data });
        parser.borrow().events[p_event].get_next()
    }

    /// C++ `_on_mml_operator_fixed_note(MMLEvent *)` — `@fx`,
    /// `(base << 6) + offset` into the fixed pitch.
    pub fn on_mml_operator_fixed_note(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 2);
        let value_base = Self::bind_ev_param(&ev_params, 0, 0);
        let value_offset = Self::bind_ev_param(&ev_params, 1, 0);

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_OPERATOR != 0 {
            return parser.borrow().events[next_event].get_next(); // Check the mask.
        }

        let channel = track
            .borrow()
            .get_channel()
            .expect("SiMMLSequencer: channel is null (C++ deref UB)")
            .clone();
        channel
            .borrow_mut()
            .set_fixed_pitch((value_base << 6) + value_offset);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_operator_ssg_envelope(MMLEvent *)` — `@se`.
    pub fn on_mml_operator_ssg_envelope(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let data = parser.borrow().events[p_event].get_data();

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_OPERATOR != 0 {
            return parser.borrow().events[p_event].get_next(); // Check the mask.
        }

        let channel = track
            .borrow()
            .get_channel()
            .expect("SiMMLSequencer: channel is null (C++ deref UB)")
            .clone();
        channel
            .borrow_mut()
            .set_ssg_envelope_control(if data == i32::MIN { 0 } else { data });
        parser.borrow().events[p_event].get_next()
    }

    /// C++ `_on_mml_operator_envelope_reset(MMLEvent *)` — `@er`; only
    /// the exact value `1` requests the reset.
    pub fn on_mml_operator_envelope_reset(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let data = parser.borrow().events[p_event].get_data();

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_OPERATOR != 0 {
            return parser.borrow().events[p_event].get_next(); // Check the mask.
        }

        let channel = track
            .borrow()
            .get_channel()
            .expect("SiMMLSequencer: channel is null (C++ deref UB)")
            .clone();
        channel.borrow_mut().set_envelope_reset(data == 1);
        parser.borrow().events[p_event].get_next()
    }

    /// C++ `_on_mml_sustain(MMLEvent *)` — `s`. NOTE: like
    /// `@rr` the sweep is applied even when the rate slot is the
    /// sentinel.
    pub fn on_mml_sustain(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 2);
        let release_rate = Self::bind_ev_param(&ev_params, 0, i32::MIN);
        let release_sweep = Self::bind_ev_param(&ev_params, 1, 0);

        let track = self.current_track_ref();
        if track.borrow().get_event_mask() & MASK_OPERATOR != 0 {
            return parser.borrow().events[next_event].get_next(); // Check the mask.
        }

        if release_rate != i32::MIN {
            let channel = track
                .borrow()
                .get_channel()
                .expect("SiMMLSequencer: channel is null (C++ deref UB)")
                .clone();
            channel.borrow_mut().set_all_release_rate(release_rate);
        }

        track.borrow_mut().set_release_sweep(release_sweep);
        parser.borrow().events[next_event].get_next()
    }

    /// C++ `_on_mml_register_update(MMLEvent *)` — REGISTER event; both
    /// parameters are passed through RAW (no `BIND_EV_PARAM` guard, so
    /// `INT32_MIN` sentinels reach the callback too).
    pub fn on_mml_register_update(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let parser = mml_parser::instance();
        let (next_event, ev_params) = Self::get_ev_params(p_event, 2);

        let track = self.current_track_ref();
        let chip = self.chip.clone();
        let mut chip = chip.borrow_mut();
        track
            .borrow_mut()
            .call_update_register(ev_params[0], ev_params[1], &mut *chip);
        parser.borrow().events[next_event].get_next()
    }


    /// Binds one `SiMMLSequencer` `_on_mml_*` method into the base
    /// listener table as [`MmlEventHandler::Custom`] — the port of the
    /// C++ `[this](MMLEvent *p_event) { return _on_mml_x(p_event); }`
    /// lambda (the base pumps hand the concrete host through
    /// [`MMLSequencerTrait::as_any_mut`]).
    fn custom(p_handler: fn(&mut Self, MmlEventRef) -> Option<MmlEventRef>) -> MmlEventHandler {
        MmlEventHandler::Custom(Rc::new(move |host, event| {
            p_handler(
                host.as_any_mut()
                    .downcast_mut::<Self>()
                    .expect("SiMMLSequencer: listener bound to a foreign host"),
                event,
            )
        }))
    }

    /// C++ `_register_process_events()` — the sounding handlers for the
    /// slots that `process_dummy` swaps out (re-run when dummy mode
    /// ends).
    pub fn register_process_events(&mut self) {
        self.base
            .set_default_listener(mml_event::NO_OP, MmlBaseHandler::DefaultOnNoOperation, false);
        self.base
            .set_default_listener(mml_event::PROCESS, MmlBaseHandler::DefaultOnProcess, false);
        self.base
            .set_mml_event_listener(mml_event::REST, Self::custom(Self::on_mml_rest), false);
        self.base
            .set_mml_event_listener(mml_event::NOTE, Self::custom(Self::on_mml_note), false);
        self.base
            .set_mml_event_listener(mml_event::SLUR, Self::custom(Self::on_mml_slur), false);
        self.base
            .set_mml_event_listener(mml_event::SLUR_WEAK, Self::custom(Self::on_mml_slur_weak), false);
        self.base
            .set_mml_event_listener(mml_event::PITCHBEND, Self::custom(Self::on_mml_pitch_bend), false);
    }

    /// C++ `_register_dummy_process_events()` — the measure-mode swap
    /// (`process_dummy`): the clock keeps running, nothing sounds.
    pub fn register_dummy_process_events(&mut self) {
        self.base
            .set_default_listener(mml_event::NO_OP, MmlBaseHandler::NoProcess, false);
        self.base
            .set_default_listener(mml_event::PROCESS, MmlBaseHandler::DummyOnProcess, false);
        for event_id in [
            mml_event::REST,
            mml_event::NOTE,
            mml_event::SLUR,
            mml_event::SLUR_WEAK,
            mml_event::PITCHBEND,
        ] {
            self.base
                .set_default_listener(event_id, MmlBaseHandler::DummyOnProcessEvent, false);
        }
    }

    /// C++ `_register_event_listeners()` — the whole user-command table.
    /// Sets `envelope_event_id` (read by `on_table_parse`) and ends by
    /// arming the swappable playback slots via
    /// [`Self::register_process_events`]. NOTE: every listener defaults
    /// to `p_global == false` exactly like the C++ call sites.
    pub fn register_event_listeners(&mut self) {
        // Pitch.
        self.base
            .create_mml_event_listener("k".to_string(), Self::custom(Self::on_mml_detune), false);
        self.base.create_mml_event_listener(
            "kt".to_string(),
            Self::custom(Self::on_mml_key_transition),
            false,
        );
        self.base.create_mml_event_listener(
            "!@kr".to_string(),
            Self::custom(Self::on_mml_relative_detune),
            false,
        );

        // Track settings.
        self.base.create_mml_event_listener(
            "@mask".to_string(),
            Self::custom(Self::on_mml_event_mask),
            false,
        );
        self.base.set_mml_event_listener(
            mml_event::QUANT_RATIO,
            Self::custom(Self::on_mml_quant_ratio),
            false,
        );
        self.base.set_mml_event_listener(
            mml_event::QUANT_COUNT,
            Self::custom(Self::on_mml_quant_count),
            false,
        );

        // Volume.
        self.base
            .create_mml_event_listener("p".to_string(), Self::custom(Self::on_mml_pan), false);
        self.base
            .create_mml_event_listener("@p".to_string(), Self::custom(Self::on_mml_fine_pan), false);
        self.base
            .create_mml_event_listener("@f".to_string(), Self::custom(Self::on_mml_filter), false);
        self.base
            .create_mml_event_listener("x".to_string(), Self::custom(Self::on_mml_expression), false);
        self.base.set_mml_event_listener(
            mml_event::VOLUME,
            Self::custom(Self::on_mml_volume),
            false,
        );
        self.base.set_mml_event_listener(
            mml_event::VOLUME_SHIFT,
            Self::custom(Self::on_mml_volume_shift),
            false,
        );
        self.base.set_mml_event_listener(
            mml_event::FINE_VOLUME,
            Self::custom(Self::on_mml_master_volume),
            false,
        );
        self.base.create_mml_event_listener(
            "%v".to_string(),
            Self::custom(Self::on_mml_volume_setting),
            false,
        );
        self.base.create_mml_event_listener(
            "%x".to_string(),
            Self::custom(Self::on_mml_expression_setting),
            false,
        );
        self.base.create_mml_event_listener(
            "%f".to_string(),
            Self::custom(Self::on_mml_filter_mode),
            false,
        );

        // Channel settings.
        self.base.create_mml_event_listener(
            "@clock".to_string(),
            Self::custom(Self::on_mml_clock),
            false,
        );
        self.base
            .create_mml_event_listener("@al".to_string(), Self::custom(Self::on_mml_algorithm), false);
        self.base
            .create_mml_event_listener("@fb".to_string(), Self::custom(Self::on_mml_feedback), false);
        self.base.create_mml_event_listener(
            "@r".to_string(),
            Self::custom(Self::on_mml_ring_modulation),
            false,
        );
        self.base.set_mml_event_listener(
            mml_event::MOD_TYPE,
            Self::custom(Self::on_mml_module_type),
            false,
        );
        self.base.set_mml_event_listener(
            mml_event::INPUT_PIPE,
            Self::custom(Self::on_mml_input),
            false,
        );
        self.base.set_mml_event_listener(
            mml_event::OUTPUT_PIPE,
            Self::custom(Self::on_mml_output),
            false,
        );
        self.base.create_mml_event_listener(
            "%t".to_string(),
            Self::custom(Self::on_mml_event_trigger),
            false,
        );
        self.base.create_mml_event_listener(
            "%e".to_string(),
            Self::custom(Self::on_mml_dispatch_event),
            false,
        );

        // Operator settings.
        self.base
            .create_mml_event_listener("i".to_string(), Self::custom(Self::on_mml_slot_index), false);
        self.base.create_mml_event_listener(
            "@rr".to_string(),
            Self::custom(Self::on_mml_operator_release_rate),
            false,
        );
        self.base.create_mml_event_listener(
            "@tl".to_string(),
            Self::custom(Self::on_mml_operator_total_level),
            false,
        );
        self.base.create_mml_event_listener(
            "@ml".to_string(),
            Self::custom(Self::on_mml_operator_multiple),
            false,
        );
        self.base.create_mml_event_listener(
            "@dt".to_string(),
            Self::custom(Self::on_mml_operator_detune),
            false,
        );
        self.base.create_mml_event_listener(
            "@ph".to_string(),
            Self::custom(Self::on_mml_operator_phase),
            false,
        );
        self.base.create_mml_event_listener(
            "@fx".to_string(),
            Self::custom(Self::on_mml_operator_fixed_note),
            false,
        );
        self.base.create_mml_event_listener(
            "@se".to_string(),
            Self::custom(Self::on_mml_operator_ssg_envelope),
            false,
        );
        self.base.create_mml_event_listener(
            "@er".to_string(),
            Self::custom(Self::on_mml_operator_envelope_reset),
            false,
        );
        self.base.set_mml_event_listener(
            mml_event::MOD_PARAM,
            Self::custom(Self::on_mml_operator_parameter),
            false,
        );
        self.base
            .create_mml_event_listener("s".to_string(), Self::custom(Self::on_mml_sustain), false);

        // Modulation.
        self.base.create_mml_event_listener(
            "@lfo".to_string(),
            Self::custom(Self::on_mml_lf_oscillator),
            false,
        );
        self.base
            .create_mml_event_listener("mp".to_string(), Self::custom(Self::on_mml_pitch_modulation), false);
        self.base.create_mml_event_listener(
            "ma".to_string(),
            Self::custom(Self::on_mml_amplitude_modulation),
            false,
        );

        // Envelope.
        self.base.create_mml_event_listener(
            "@fps".to_string(),
            Self::custom(Self::on_mml_envelope_fps),
            false,
        );
        self.envelope_event_id = self.base.create_mml_event_listener(
            "@@".to_string(),
            Self::custom(Self::on_mml_tone_envelope),
            false,
        );
        self.base.create_mml_event_listener(
            "na".to_string(),
            Self::custom(Self::on_mml_amplitude_envelope),
            false,
        );
        self.base
            .create_mml_event_listener("np".to_string(), Self::custom(Self::on_mml_pitch_envelope), false);
        self.base
            .create_mml_event_listener("nt".to_string(), Self::custom(Self::on_mml_note_envelope), false);
        self.base.create_mml_event_listener(
            "nf".to_string(),
            Self::custom(Self::on_mml_filter_envelope),
            false,
        );
        self.base.create_mml_event_listener(
            "_@@".to_string(),
            Self::custom(Self::on_mml_tone_release_envelope),
            false,
        );
        self.base.create_mml_event_listener(
            "_na".to_string(),
            Self::custom(Self::on_mml_amplitude_release_envelope),
            false,
        );
        self.base.create_mml_event_listener(
            "_np".to_string(),
            Self::custom(Self::on_mml_pitch_release_envelope),
            false,
        );
        self.base.create_mml_event_listener(
            "_nt".to_string(),
            Self::custom(Self::on_mml_note_release_envelope),
            false,
        );
        self.base.create_mml_event_listener(
            "_nf".to_string(),
            Self::custom(Self::on_mml_filter_release_envelope),
            false,
        );
        self.base.create_mml_event_listener(
            "!na".to_string(),
            Self::custom(Self::on_mml_amplitude_envelope_tsscp),
            false,
        );
        self.base
            .create_mml_event_listener("po".to_string(), Self::custom(Self::on_mml_portament), false);

        // These can be swapped for dummy processing.
        self.register_process_events();

        self.base.set_mml_event_listener(
            mml_event::DRIVER_NOTE,
            Self::custom(Self::on_mml_driver_note_on),
            false,
        );
        self.base.set_mml_event_listener(
            mml_event::REGISTER,
            Self::custom(Self::on_mml_register_update),
            false,
        );
    }

    /// C++ `_reset_initial_operator_params()` — the chip's shared
    /// initial operator template (the C++ ctor made the chip mandatory;
    /// the C++ default-`nullptr` UB is not reproduced — the port's ctor
    /// requires the chip).
    pub fn reset_initial_operator_params(&mut self) {
        let op_params = self.chip.borrow().get_init_operator_params();

        let mut op_params = op_params.borrow_mut();
        op_params.set_attack_rate(63);
        op_params.set_decay_rate(0);
        op_params.set_sustain_rate(0);
        op_params.set_release_rate(28);
        op_params.set_sustain_level(0);
        op_params.set_total_level(0);
        op_params.set_key_scaling_rate(0);
        op_params.set_key_scaling_level(0);
        op_params.set_fine_multiple(128);
        op_params.set_detune1(0);
        op_params.set_detune2(0);
        op_params.set_amplitude_modulation_shift(1);
        op_params.set_initial_phase(0);
        op_params.set_fixed_pitch(0);
        op_params.set_frequency_modulation_level(5);
        op_params.set_pulse_generator_type(PULSE_SQUARE);
    }

    /// C++ `_reset_parser_settings()`.
    pub fn reset_parser_settings(&mut self) {
        let settings = self.base.get_parser_settings();

        let mut settings = settings.borrow_mut();
        settings.default_bpm = 120.0;
        settings.default_l_value = 4;
        settings.default_quant_ratio = 6;
        settings.max_quant_ratio = 8;
        settings.set_default_octave(5);
        settings.max_volume = 512;
        settings.default_volume = 256;
        settings.max_fine_volume = 128;
        settings.default_fine_volume = 64;
    }
}

#[allow(clippy::missing_panics_doc)] // the C++ deref-UB seams panic via
// `expect` instead (documented on each handler)
impl MMLSequencerTrait for SiMMLSequencer {
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

    /// C++ `_on_before_compile(sion::String)` — comment stripping,
    /// trailing-`;` guarantee, macro-definition/system-command pass and
    /// the `![..|..!]..` repeat expansion. NOTE quirks reproduced: the
    /// repeat splice skips ONE character past the match end (C++
    /// `substr(get_end() + 1)` on exclusive ends), a repeat count of `0`
    /// underflows through C++ `repeat(uint64_t)` (ported as the same
    /// `as usize` wrap -> allocation panic), and `#END` truncates the
    /// stream.
    fn on_before_compile(&mut self, p_mml: String) -> String {
        static RE_COMMENTS: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(?s)/\*.*?\*/|//.*?[\r\n]+").expect("valid regex")
        });
        static RE_SEQUENCE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(?s)[ \t\r\n]*(#([A-Z@\-]+)(\+=|=)?)?([^;{]*(\{.*?\})?[^;]*);")
                .expect("valid regex")
        });
        static RE_MACRO_ID: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"([A-Z])?(-([A-Z])?)?").expect("valid regex"));
        static RE_REPEAT: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(?s)!\[(\d*)(.*?)(!\|(.*?))?\](\d*)").expect("valid regex")
        });

        self.reset_parser_parameters();

        let mut mml = p_mml + "\n";

        // Remove comments.
        mml = crate::utils::string::literal_replace_all(&RE_COMMENTS, &mml, "");

        // Ensure the string ends with a semicolon.
        let mut i = mml.len();
        let mut last_char;
        loop {
            if i == 0 {
                return String::new();
            }
            i -= 1;
            last_char = mml.as_bytes()[i];
            if last_char != b' ' && last_char != b'\t' && last_char != b'\r' && last_char != b'\n' {
                break;
            }
        }
        mml = mml[..i + 1].to_string();
        if last_char != b';' {
            mml += ";";
        }

        // Expand macros.
        let mut expanded_mml = String::new();

        let owned: Vec<Vec<String>> = RE_SEQUENCE
            .captures_iter(&mml)
            .map(|caps| (0..=4).map(|g| caps.group_str(g)).collect())
            .collect();

        for res in owned {
            // Normal sequence.
            if res[1].is_empty() {
                expanded_mml += &self.expand_macro(res[4].clone(), 0);
                expanded_mml += ";";
                continue;
            }

            // System command.
            if res[3].is_empty() {
                if res[2] == "END" {
                    break; // The #END command.
                }
                if !self.parse_system_command_before(res[1].clone(), res[4].clone()) {
                    // If parsing returned false, we'll try it again after
                    // compiling MML.
                    expanded_mml += &res[0];
                }
                continue;
            }

            // Macro definition.
            let macro_id = res[2].clone();
            let concat = res[3] == "+=";

            // Parse macro IDs.
            for mid_res in RE_MACRO_ID.captures_iter(&macro_id) {
                if mid_res.get(0).is_none_or(|m| m.as_str().is_empty()) {
                    continue; // Regex can have empty matches.
                }

                let mut start_id = 0usize;
                let g1 = mid_res.group_str(1);
                if !g1.is_empty() {
                    start_id = (g1.as_bytes()[0] - b'A') as usize;
                }

                let mut end_id = start_id;
                let g2 = mid_res.group_str(2);
                if !g2.is_empty() {
                    let g3 = mid_res.group_str(3);
                    if !g3.is_empty() {
                        end_id = (g3.as_bytes()[0] - b'A') as usize;
                    } else {
                        end_id = MACRO_SIZE - 1;
                    }
                }

                let value = if self.macro_expand_dynamic {
                    res[4].clone()
                } else {
                    self.expand_macro(res[4].clone(), 0)
                };

                for k in start_id..=end_id {
                    if concat {
                        self.macro_strings[k] += &value;
                    } else {
                        self.macro_strings[k] = value.clone();
                    }
                }
            }
        }

        // Expand repeat.
        let owned: Vec<(usize, usize, Vec<String>)> = RE_REPEAT
            .captures_iter(&expanded_mml)
            .map(|caps| {
                let full = caps.get(0).expect("whole match");
                (
                    full.start(),
                    full.end(),
                    (1..=5).map(|g| caps.group_str(g)).collect(),
                )
            })
            .collect();

        // Iterate backwards so we can do in-place replacements without
        // disturbing indices.
        for (start, end, res) in owned.into_iter().rev() {
            let mut repeat_count = 1i32;
            if !res[0].is_empty() {
                repeat_count = to_int(&res[0]) as i32 - 1;
            } else if !res[4].is_empty() {
                repeat_count = to_int(&res[4]) as i32 - 1;
            }

            if repeat_count > 256 {
                repeat_count = 256;
            }

            let mut rep = res[1].clone();
            if !res[2].is_empty() {
                rep += &res[3];
            }

            let replacement = rep.repeat(repeat_count as usize) + &res[1];

            // Take the rest of the string (around the match) and insert
            // the replaced substring.
            let suffix = expanded_mml.get(end + 1..).unwrap_or("");
            expanded_mml =
                format!("{}{}{}", &expanded_mml[..start], replacement, suffix);
        }

        expanded_mml
    }

    /// C++ `_on_after_compile(MMLSequenceGroup *)`. NOTE: the base
    /// `compile()` keeps a mutable borrow of `mml_data` across this call —
    /// `_parse_system_command_after` must not touch `mml_data` (the C++
    /// original does not; it only drives `_connector` and the chain).
    fn on_after_compile(&mut self, p_group: &mut MMLSequenceGroup) {
        let mut sequence = p_group.get_head_sequence();
        while let Some(current) = sequence {
            sequence = if current.borrow().is_system_command() {
                self.parse_system_command_after(p_group, &current)
            } else {
                MMLSequence::get_next_sequence(&current)
            };
        }
    }

    /// C++ `_on_process(int, MMLEvent *)`.
    fn on_process(&mut self, p_length: i32, _p_event: Option<MmlEventRef>) {
        let track = self
            .current_track
            .clone()
            .expect("SiMMLSequencer: _current_track is null (C++ deref UB)");
        let chip = self.chip.clone();
        let mut chip = chip.borrow_mut();
        track.borrow_mut().buffer(p_length, &mut *chip);
    }

    /// C++ `_on_timer_interruption()`.
    fn on_timer_interruption(&mut self) {
        if self.dummy_process {
            return;
        }
        if let Some(callback) = &self.callback_timer {
            callback();
        }
    }

    /// C++ `_on_beat(int, int)`.
    fn on_beat(&mut self, p_delay_samples: i32, p_beat_counter: i32) {
        if self.dummy_process {
            return;
        }
        if let Some(callback) = &self.callback_beat {
            callback(p_delay_samples, p_beat_counter);
        }
    }

    /// C++ `_on_table_parse(MMLEvent *, sion::String)`. NOTE quirk kept:
    /// `SiMMLData::set_envelope_table`'s index guard returns early on a
    /// bad index but this caller still stamps the event data and
    /// decrements the counter.
    fn on_table_parse(&mut self, p_prev: MmlEventRef, p_table: String) {
        static RE_TABLE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"(?s)\{([^}]*)\}(.*)").expect("valid regex"));

        let id = mml_parser::instance().borrow().events[p_prev].get_id();
        err_fail_cond_msg!(
            id < self.envelope_event_id || id > self.envelope_event_id + 10,
            "p_prev->get_id() < _envelope_event_id || p_prev->get_id() > _envelope_event_id + 10",
            "SiMMLSequencer : Internal table is available only for envelope commands."
        );

        let res = RE_TABLE.captures(&p_table);
        err_fail_cond_msg!(res.is_none(), "!res.is_valid()", "SiMMLSequencer: Invalid table format.");
        let res = res.expect("table search matched");

        let data = res.get(1).map_or_else(String::new, |m| m.as_str().to_string());
        let postfix = res.get(2).map_or_else(String::new, |m| m.as_str().to_string());

        let handle = self
            .base
            .mml_data
            .clone()
            .expect("SiMMLSequencer: table parse without mml_data (C++ deref UB)");
        let simml_data = handle
            .simml()
            .expect("SiMMLSequencer: table parse on base-only data (C++ downcast null deref)");

        let mut env_table = SiMMLEnvelopeTable::default();
        env_table.parse_mml(&data, &postfix, 65536);
        err_fail_cond_msg!(
            env_table.data.is_none(),
            "!env_table->get_data()",
            format!("SiMMLSequencer: Invalid table parameter '{}' in the {{..}} command.", data)
        );

        let index = self.internal_table_index;
        simml_data
            .borrow_mut()
            .set_envelope_table(index, Some(Rc::new(RefCell::new(env_table))));

        mml_parser::instance().borrow_mut().events[p_prev].set_data(index);
        self.internal_table_index -= 1;
    }

    /// C++ `_on_tempo_changed(double)`.
    fn on_tempo_changed(&mut self, p_tempo_ratio: f64) {
        for track in self.tracks.clone() {
            let mut track = track.borrow_mut();
            if track.get_bpm_settings().is_none() {
                track.executor.on_tempo_changed(p_tempo_ratio);
            }
        }

        if let Some(callback) = &self.callback_tempo_changed {
            let buffer_index = self.base.global_buffer_index;
            let dummy = self.dummy_process;
            callback(buffer_index, dummy);
        }
    }

    /// C++ `prepare_compile(const Ref<MMLData>&, sion::String)`.
    fn prepare_compile(&mut self, p_data: Option<MmlDataHandle>, p_mml: String) -> bool {
        self.free_all_tracks();
        BaseShim { host: self }.prepare_compile(p_data, p_mml)
    }

    /// C++ `prepare_process(const Ref<MMLData>&, int, int)`.
    fn prepare_process(&mut self, p_data: Option<MmlDataHandle>, p_sample_rate: i32, p_buffer_length: i32) {
        self.free_all_tracks();
        self.processed_sample_count = 0;
        self.bpm_change_enabled = true;

        BaseShim { host: self }.prepare_process(p_data, p_sample_rate, p_buffer_length);

        if let Some(handle) = self.base.mml_data.clone() {
            let simml_data = handle
                .simml()
                .expect("SiMMLSequencer: prepare_process requires SiMMLData (C++ downcast null deref)");
            let mut index = 0i32;

            let mut sequence = handle.mml_mut().get_sequence_group().get_head_sequence();
            while let Some(current) = sequence {
                if current.borrow().is_active() {
                    let track = match self.free_tracks.pop() {
                        Some(track) => track,
                        None => Rc::new(RefCell::new(SiMMLTrack::new())),
                    };

                    let internal_track_id = index | MML_TRACK;
                    track.borrow_mut().initialize(
                        Some(simml_data.clone()),
                        Some(current.clone()),
                        handle.mml().get_default_fps(),
                        internal_track_id,
                        self.callback_event_note_on.clone(),
                        self.callback_event_note_off.clone(),
                        true,
                    );
                    track.borrow_mut().set_track_number(index);
                    self.tracks.push(track);

                    index += 1;
                }

                sequence = MMLSequence::get_next_sequence(&current);
            }
        }

        self.reset_all_tracks();
    }

    /// C++ `process()` — buffer one chip block: reset channel statuses,
    /// pump the global sequence, then per track `prepare_buffer` +
    /// `process_executor`. The executor raw pointer follows the
    /// [`CurrentExecutor`] contract (the `Rc` in `self.tracks` keeps the
    /// `MMLExecutor` alive across the pump).
    fn process(&mut self) {
        // Prepare for buffering.
        for track in self.tracks.clone() {
            let channel = track
                .borrow()
                .get_channel()
                .expect("SiMMLSequencer: track channel is null (C++ deref UB)")
                .clone();
            channel.borrow_mut().reset_channel_buffer_status();
        }

        // Buffering.
        let mut finished = true;
        self.start_global_sequence();

        loop {
            let buffering_length = self.execute_global_sequence();
            self.bpm_change_enabled = false;

            for track in self.tracks.clone() {
                self.current_track = Some(track.clone());
                let length = {
                    let chip = self.chip.clone();
                    let mut chip = chip.borrow_mut();
                    track.borrow_mut().prepare_buffer(buffering_length, &mut *chip)
                };

                let bpm = track.borrow().get_bpm_settings();
                self.base.bpm = bpm.unwrap_or_else(|| self.base.adjustible_bpm.clone());

                // NOTE: the `borrow_mut` guard is scoped to a block —
                // edition-2024 temporary lifetimes would otherwise keep
                // the `RefCell` borrowed across `process_executor`,
                // breaking handlers that re-borrow the track. The raw
                // pointer stays valid: it targets the `MMLExecutor`
                // inside the (still alive, unborrowed) track `Rc`.
                let executor: *mut MMLExecutor = {
                    let mut borrowed = track.borrow_mut();
                    std::ptr::addr_of_mut!(borrowed.executor)
                };
                finished = self.process_executor(executor, length) && finished;
            }

            self.bpm_change_enabled = true;

            if self.check_global_sequence_end() {
                break;
            }
        }

        self.base.bpm = self.base.adjustible_bpm.clone();
        self.current_track = None;
        self.processed_sample_count += self.chip.borrow().get_buffer_length();

        self.is_sequence_finished = finished;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sequencer::base::mml_data::MMLData;

    fn bare_chip() -> Rc<RefCell<SiopmSoundChip>> {
        Rc::new(RefCell::new(SiopmSoundChip::new()))
    }

    fn bare_sequencer() -> SiMMLSequencer {
        // Ctor smoke (wave-7c C3 landed): `new` registers the full
        // listener table and resets the operator/parser defaults.
        SiMMLSequencer::new(bare_chip())
    }

    #[test]
    fn mml_data_handle_projects_both_views() {
        let simml = Rc::new(RefCell::new(SiMMLData::new()));
        let handle = MmlDataHandle::Simml(simml.clone());
        assert_eq!(handle.mml().get_default_fps(), 60);
        assert!(handle.simml().is_some());

        let base_handle = MmlDataHandle::Base(Rc::new(RefCell::new(MMLData::new())));
        assert_eq!(base_handle.mml().get_default_fps(), 60);
        assert!(base_handle.simml().is_none());
    }

    #[test]
    fn lifecycle_state_defaults() {
        let mut seq = bare_sequencer();
        assert!(!seq.is_ready_to_process());
        assert!(seq.is_finished());
        assert!(seq.is_sequence_finished());
        assert_eq!(seq.get_effective_bpm(), seq.base.get_default_bpm());

        seq.stop_sequence();
        assert!(seq.is_sequence_finished());

        seq.set_beat_callback_filter(7);
        assert_eq!(seq.base.on_beat_callback_filter, 7);
        assert_eq!(seq.get_max_track_count(), DEFAULT_MAX_TRACK_COUNT);

        seq.set_effective_bpm(150.0);
        assert!(!seq.bpm_change_enabled);
        assert_eq!(seq.base.get_default_bpm(), 150.0);
    }

    fn live_sequencer() -> (SiMMLSequencer, Rc<RefCell<SiMMLData>>) {
        let mut seq = bare_sequencer();
        let data = Rc::new(RefCell::new(SiMMLData::new()));
        seq.base.mml_data = Some(MmlDataHandle::Simml(data.clone()));
        (seq, data)
    }

    #[test]
    fn expand_macro_static_dynamic_and_shift() {
        let mut seq = bare_sequencer();
        seq.macro_strings[0] = "cd".to_string();
        seq.macro_strings[1] = "A e".to_string();

        // Static mode: macro bodies splice in verbatim (no recursion).
        assert_eq!(seq.expand_macro("A B".to_string(), 0), "cd A e");

        // Dynamic mode: bodies expand recursively at call time.
        seq.macro_expand_dynamic = true;
        assert_eq!(seq.expand_macro("A B".to_string(), 0), "cd cd e");
        seq.macro_expand_dynamic = false;

        // Note-shift wrapper.
        seq.macro_strings[6] = "g".to_string();
        assert_eq!(seq.expand_macro("G(2)".to_string(), 0), "!@ns2g!@ns-2");

        // Circular reference guard returns the ORIGINAL string.
        seq.macro_strings[0] = "A".to_string();
        assert_eq!(seq.expand_macro("A".to_string(), 0), "A");
    }

    #[test]
    fn system_command_parser_settings_branches() {
        let (mut seq, data) = live_sequencer();
        let base = MmlDataHandle::Simml(data.clone());

        assert!(seq.parse_system_command_before("#TITLE".to_string(), "MySong".to_string()));
        assert_eq!(base.mml().get_title(), "MySong");
        assert!(seq.parse_system_command_before("#TITLE".to_string(), "{Other}".to_string()));
        assert_eq!(base.mml().get_title(), "Other");

        assert!(seq.parse_system_command_before("#FPS".to_string(), "120".to_string()));
        assert_eq!(base.mml().get_default_fps(), 120);
        assert!(seq.parse_system_command_before("#FPS".to_string(), String::new()));
        assert_eq!(base.mml().get_default_fps(), 60);

        assert!(seq.parse_system_command_before("#QUANT".to_string(), "16".to_string()));
        {
            let settings = seq.base.get_parser_settings();
            let settings = settings.borrow();
            assert_eq!(settings.max_quant_ratio, 16);
            assert_eq!(settings.default_quant_ratio, 12);
        }

        assert!(seq.parse_system_command_before("#REV".to_string(), String::new()));
        {
            let settings = seq.base.get_parser_settings();
            let settings = settings.borrow();
            assert_eq!(settings.octave_polarization, -1);
            assert_eq!(settings.volume_polarization, -1);
        }

        assert!(seq.parse_system_command_before("#MACRO".to_string(), "dynamic".to_string()));
        assert!(seq.macro_expand_dynamic);
        assert!(seq.parse_system_command_before("#MACRO".to_string(), "bogus".to_string()));
        assert!(seq.macro_expand_dynamic);
        assert!(seq.parse_system_command_before("#MACRO".to_string(), "static".to_string()));
        assert!(!seq.macro_expand_dynamic);

        // #FM defers to the after pass.
        assert!(!seq.parse_system_command_before("#FM".to_string(), "x".to_string()));

        // TMODE unit=240 -> resolution 1/240 on the BPM tcommand.
        assert!(seq.parse_system_command_before("#TMODE".to_string(), "{unit=240}".to_string()));
        assert!((base.mml().get_bpm_from_tcommand(120) - 0.5).abs() < 1e-12);

        // VMODE n88 -> velocity DR32DB(4), expression DR48DB(3).
        assert!(seq.parse_system_command_before("#VMODE".to_string(), "{n88}".to_string()));
        assert_eq!(base.mml().get_default_velocity_mode(), 4);
        assert_eq!(base.mml().get_default_expression_mode(), 3);
    }

    #[test]
    fn system_command_tables_and_user_commands() {
        let (mut seq, data) = live_sequencer();

        assert!(seq.parse_system_command_before("#TABLE".to_string(), "1{0 12}x".to_string()));
        assert!(data.borrow().get_envelope_table(1).is_some());

        // Out-of-range guard: consumed, nothing stored.
        assert!(seq.parse_system_command_before("#TABLE".to_string(), "255{0 12}".to_string()));
        assert!(data.borrow().get_envelope_table(255).is_none());

        assert!(seq.parse_system_command_before("#WAV".to_string(), "3{0,64,127}".to_string()));
        assert!(data.borrow().get_wave_table(3).is_some());

        // Unknown commands land in system_commands with the split fields.
        assert!(seq.parse_system_command_before("#FOO".to_string(), "3{a}b".to_string()));
        let commands = MmlDataHandle::Simml(data.clone()).mml().get_system_commands();
        assert_eq!(commands.len(), 1);
        let command = commands[0].borrow();
        assert_eq!(command.command, "#FOO");
        assert_eq!(command.number, 3);
        assert_eq!(command.content, "a");
        assert_eq!(command.postfix, "b");
    }

    #[test]
    fn system_command_tone_initializes_voice_sequence() {
        let (mut seq, data) = live_sequencer();

        // Full 1-operator `#@` parameter list: AL, FB, FC + 15 operator
        // values (ChannelParams::new() starts with operator_count == 1).
        let zeros = "0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0";
        assert!(seq.parse_system_command_before("#@".to_string(), format!("5{{{zeros}}}c8")));

        let voice = data.borrow().fm_voices[5].clone();
        assert!(voice.is_some());
        let voice = voice.expect("checked");
        let init_sequence = voice.borrow().channel_params.borrow().get_init_sequence();
        assert!(init_sequence.is_some());
        let init_sequence = init_sequence.expect("checked");
        assert!(init_sequence.borrow().get_head_event().is_some());
    }

    #[test]
    fn on_before_compile_expands_macros_and_comments() {
        let (mut seq, _data) = live_sequencer();

        let out = seq.on_before_compile("#A=c d;\n// comment\nA e;\n".to_string());
        assert_eq!(out, "c d e;");

        // Missing trailing semicolon is appended.
        let out = seq.on_before_compile("ab".to_string());
        assert_eq!(out, "ab;");

        // #END truncates the stream.
        let out = seq.on_before_compile("#A=1;#END;c;".to_string());
        assert_eq!(out, "");
    }

    #[test]
    fn on_before_compile_repeat_skips_one_char_past_match() {
        let (mut seq, _data) = live_sequencer();

        // C++ quirks: the lazy body stops at the first position where the
        // rest matches (here `"de!"` — the `!\|` branch stays empty), and
        // the splice at `get_end() + 1` on the exclusive end eats the
        // character right after `]` (the terminating semicolon).
        let out = seq.on_before_compile("c![2de!];".to_string());
        assert_eq!(out, "cde!de!");
    }

    #[test]
    fn process_pumps_empty_chip_block() {
        let (mut seq, data) = live_sequencer();
        seq.chip.borrow_mut().initialize(32, 16, 1024);
        seq.prepare_process(Some(MmlDataHandle::Simml(data)), 44100, 1024);

        seq.process();
        assert_eq!(seq.get_processed_sample_count(), 1024);
        assert!(seq.is_sequence_finished());
        assert!(seq.current_track.is_none());
    }

    #[test]
    fn table_parse_stamps_and_decrements_index() {
        let (mut seq, data) = live_sequencer();
        seq.reset_parser_parameters();
        seq.internal_table_index = 5;

        let parser = mml_parser::instance();
        let sequence = MMLSequence::new(false);
        MMLSequence::initialize(&sequence);
        let event = MMLSequence::append_new_event(&sequence, mml_event::TABLE_EVENT, 0, 0);
        seq.envelope_event_id = mml_event::TABLE_EVENT;

        seq.on_table_parse(event, "{0,64,127}".to_string());

        assert!(data.borrow().get_envelope_table(5).is_some());
        assert_eq!(parser.borrow().events[event].get_data(), 5);
        assert_eq!(seq.internal_table_index, 4);
    }

    fn compiled_chip() -> Rc<RefCell<SiopmSoundChip>> {
        let chip = bare_chip();
        chip.borrow_mut().initialize(32, 16, 1024);
        chip
    }

    fn compile_to_completion(seq: &mut SiMMLSequencer, data: &Rc<RefCell<SiMMLData>>, p_mml: &str) {
        assert!(seq.prepare_compile(Some(MmlDataHandle::Simml(data.clone())), p_mml.to_string()));
        let mut progress = 0.0;
        for _ in 0..1000 {
            progress = seq.compile(100000);
            if progress >= 1.0 {
                break;
            }
        }
        assert_eq!(progress, 1.0);
    }

    #[test]
    fn ctor_wires_listener_table_and_defaults() {
        let seq = bare_sequencer();

        // The `_create_mml_event_listener` table filled by the ctor;
        // `@@` must sit at `envelope_event_id` for `on_table_parse`.
        assert!(seq.envelope_event_id >= mml_event::USER_DEFINED);
        assert_eq!(seq.base.get_event_letter(seq.envelope_event_id), "@@");
        for letter in [
            "k", "kt", "!@kr", "@mask", "p", "@p", "@f", "x", "%v", "%x", "%f", "@clock", "@al",
            "@fb", "@r", "%t", "%e", "i", "@rr", "@tl", "@ml", "@dt", "@ph", "@fx", "@se", "@er",
            "s", "@lfo", "mp", "ma", "@fps", "na", "np", "nt", "nf", "_@@", "_na", "_np", "_nt",
            "_nf", "!na", "po",
        ] {
            assert_ne!(seq.base.get_event_id(letter), 0, "missing listener: {letter}");
        }

        // Parser defaults per C++ `_reset_parser_settings`.
        let settings = seq.base.get_parser_settings();
        let settings = settings.borrow();
        assert_eq!(settings.default_l_value, 4);
        assert_eq!(settings.default_quant_ratio, 6);
        assert_eq!(settings.max_quant_ratio, 8);
        assert_eq!(settings.max_volume, 512);
        assert_eq!(settings.default_volume, 256);
        assert_eq!(settings.max_fine_volume, 128);
        assert_eq!(settings.default_fine_volume, 64);
        drop(settings);

        // Operator template per C++ `_reset_initial_operator_params`.
        let params = seq.chip.borrow().get_init_operator_params();
        let params = params.borrow();
        assert_eq!(params.get_attack_rate(), 63);
        assert_eq!(params.get_release_rate(), 28);
        assert_eq!(params.get_fine_multiple(), 128);
        assert_eq!(params.get_pulse_generator_type(), PULSE_SQUARE);
    }

    #[test]
    fn pumped_psg_tune_finishes_with_non_silent_output() {
        mml_ref_table::initialize();
        let chip = compiled_chip();
        let mut seq = SiMMLSequencer::new(chip.clone());
        let data = Rc::new(RefCell::new(SiMMLData::new()));

        compile_to_completion(&mut seq, &data, "%0,0 v15 c8 d e o1 r;");

        seq.prepare_process(Some(MmlDataHandle::Simml(data)), 44100, 1024);
        assert_eq!(seq.get_tracks().len(), 1);

        let mut peak = 0.0f64;
        let mut pumps = 0;
        while pumps < 2000 && !seq.is_finished() {
            seq.process();
            pumps += 1;
            chip.borrow().with_output_buffer(|buffer| {
                for sample in buffer {
                    peak = peak.max(sample.abs());
                }
            });
        }

        assert!(seq.is_finished(), "sequence did not finish in {pumps} pumps");
        assert!(seq.is_sequence_finished());
        assert!(peak > 0.001, "expected audible output, peak {peak}");
    }

    #[test]
    fn event_dispatch_command_fires_track_callback() {
        mml_ref_table::initialize();
        let chip = compiled_chip();
        let mut seq = SiMMLSequencer::new(chip.clone());
        let data = Rc::new(RefCell::new(SiMMLData::new()));

        let fired = Rc::new(RefCell::new(0i32));
        let counter = fired.clone();
        seq.set_note_on_callback(Some(Rc::new(move |_track: &mut SiMMLTrack| {
            *counter.borrow_mut() += 1;
        })));

        // `%e1` dispatches the track's event-trigger callback (id 1,
        // type default `EVENT_FRAME`); only `%e` may fire here since
        // `event_trigger_type_on` stays `NO_EVENTS` for plain notes.
        compile_to_completion(&mut seq, &data, "%0,0 v15 o4 c8 %e1 r;");

        seq.prepare_process(Some(MmlDataHandle::Simml(data)), 44100, 1024);
        assert_eq!(seq.get_tracks().len(), 1);

        let mut pumps = 0;
        while pumps < 2000 && !seq.is_finished() {
            seq.process();
            pumps += 1;
        }

        assert!(seq.is_finished(), "sequence did not finish in {pumps} pumps");
        assert_eq!(*fired.borrow(), 1, "%e must fire exactly one dispatch");
    }

}
