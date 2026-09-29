//! `SiMMLTrack` (`libSiON-cpp/src/sequencer/simml_track.{h,cpp}`).
//!
//! One sequence being played: owns the sound module channel
//! ([`ChannelRc`], via [`manager`]), an [`MMLExecutor`], the module
//! [`SiMMLChannelSettings`] and every note-on/off envelope cursor. C++
//! `SinglyLinkedList<int>::Element *` envelope cursors become
//! [`EnvCursor`]s (index into the shared `Rc<RefCell<SinglyLinkedList>>`
//! owned by the referencing envelope table); the internal-cursor modulation
//! tables keep the list's own cursor (C++ `list->get()/next()/front()`).
//!
//! Ownership / virtual-dispatch mapping (mirrors earlier waves):
//! - C++ `_channel` / `_executor` raw pointers → an owned `Option<ChannelRc>`
//!   and an embedded [`MMLExecutor`].
//! - C++ `_sound_chip` (reached through the channel) → a `&mut dyn
//!   ChipContext` threaded through every playback method (wave-6b/7a
//!   convention).
//! - C++ `std::function` callbacks → `Option<Rc<dyn Fn...>>` so a track can
//!   hand clones of its trigger callbacks to the sequencer.
//! - C++ `SiMMLTrack::initialize()/finalize()` statics (the
//!   `_envelope_zero_table` singleton) → the `ZERO_LIST` thread-local below.
//!
//! Kept C++ quirks (reproduced, not fixed):
//! - `handle_pitch_bend` keeps the session-10 64-bit intermediate
//!   (`(i64)delta << FIXED_BITS * interval / max(term,1)`) — the upstream
//!   32-bit product wrapped and a zero `term` divided by zero. The C++
//!   `(int)` cast back to the field is an `as i32` truncation (identical
//!   wrap semantics to the C++ overflow).
//! - `_buffer_envelope` modulation walk uses the list's *internal* cursor
//!   (`get()` null at tail ⇒ modulation cleared); pitch/note/exp/filter
//!   cursors advance with `next_of` (nullptr-past-tail stops that envelope
//!   only). A null `_envelope_pitch/_envelope_note` read while
//!   `_envelope_pitch_active` is a C++ null deref; the port `expect`s.
//! - `set_note_immediately`/`handle_note_event`/`change_note_length`
//!   truncate `sample_length * quantize_ratio` with C++ `(int)` semantics;
//!   the amplitude envelope value is clamped `0..=128`.
//! - `SWEEP_FINESS` is declared in the C++ header and unused; kept with the
//!   same status.

use std::cell::RefCell;
use std::rc::Rc;

use crate::chip::channels::manager::{self, ChannelRc};
use crate::chip::channels::ChipContext;
use crate::chip::ref_table as chip_ref_table;
use crate::err_print;
use crate::math::clampi;
use crate::sequencer::base::beats_per_minute::BeatsPerMinute;
use crate::sequencer::base::mml_executor::MMLExecutor;
use crate::sequencer::base::mml_sequence::SeqRc;
use crate::sequencer::channel_settings::SiMMLChannelSettings;
use crate::sequencer::data::SiMMLData;
use crate::sequencer::envelope_table::SiMMLEnvelopeTable;
use crate::sequencer::ref_table as mml_ref_table;
use crate::utils::translator_util::SinglyLinkedList;

/// C++ `SiMMLTrack *` handle.
pub type TrackRc = Rc<RefCell<SiMMLTrack>>;
/// C++ `std::function<void(SiMMLTrack *)>`.
pub type TrackFn = Rc<dyn Fn(&mut SiMMLTrack)>;

/// C++ std::function<void(SiMMLTrack *, int, int)> (_callback_update_register).
pub type UpdateRegisterFn = Rc<dyn Fn(&mut SiMMLTrack, i32, i32)>;

// Mask bits for event_mask and @mask command (C++ MaskBits).
pub const NO_MASK: u32 = 0;
pub const MASK_VOLUME: u32 = 1;
pub const MASK_PAN: u32 = 2;
pub const MASK_QUANTIZE: u32 = 4;
pub const MASK_OPERATOR: u32 = 8;
pub const MASK_ENVELOPE: u32 = 16;
pub const MASK_MODULATE: u32 = 32;
pub const MASK_SLUR: u32 = 64;

// ProcessMode.
pub const PROCESS_NORMAL: i32 = 0;
pub const PROCESS_ENVELOPE: i32 = 2;

// EventTriggerType (stored as i32, like the C++ enums).
pub const NO_EVENTS: i32 = 0;
pub const EVENT_FRAME: i32 = 1;
pub const EVENT_STREAM: i32 = 2;
#[allow(dead_code)]
pub const EVENT_BOTH: i32 = EVENT_FRAME | EVENT_STREAM;

// Track ID filters (C++ TRACK_ID_FILTER / TRACK_TYPE_FILTER).
pub const TRACK_ID_FILTER: i32 = 0xffff;
pub const TRACK_TYPE_FILTER: i32 = 0xff0000;

// TrackTypeID.
pub const MML_TRACK: i32 = 0x10000;
#[allow(dead_code)] // Only referenced by the wave-8 driver.
pub const MIDI_TRACK: i32 = 0x20000;
pub const DRIVER_NOTE: i32 = 0x30000;
#[allow(dead_code)] // Only referenced by the wave-8 driver.
pub const DRIVER_SEQUENCE: i32 = 0x40000;
#[allow(dead_code)] // Only referenced by the wave-8 driver.
pub const DRIVER_BACKGROUND: i32 = 0x50000;
#[allow(dead_code)] // Only referenced by the wave-8 driver.
pub const USER_CONTROLLED: i32 = 0x60000;

// C++ private constants. `SWEEP_FINESS` is declared and unused in C++.
#[allow(dead_code)]
const SWEEP_FINESS: i32 = 128;
const FIXED_BITS: i32 = 16;
const SWEEP_MAX: i32 = 8192 << FIXED_BITS;

thread_local! {
    /// C++ `static SinglyLinkedList<int> *_envelope_zero_table` — a
    /// 1-element, self-looping zero list (`new SinglyLinkedList<int>(1, 0,
    /// true)`). `None` = nullptr (pre-`initialize`/post-`finalize`).
    static ZERO_TABLE: RefCell<Option<Rc<RefCell<SinglyLinkedList>>>> =
        const { RefCell::new(None) };
}

/// Cursor into a shared envelope list (C++ `SinglyLinkedList<int>::Element*`);
/// `index: None` == nullptr. `advance` follows `Element::next()` exactly,
/// including the tail loop installed by `SiMMLEnvelopeTable::set_data`.
#[derive(Clone)]
pub struct EnvCursor {
    list: Rc<RefCell<SinglyLinkedList>>,
    index: Option<usize>,
}

impl EnvCursor {
    /// The self-looping zero-table head cursor (C++ `_envelope_zero_table->get_front()`).
    fn zero() -> EnvCursor {
        EnvCursor {
            list: SiMMLTrack::zero_list(),
            index: Some(0),
        }
    }

    /// A nullptr cursor (C++ `nullptr` `Element*`).
    fn null() -> EnvCursor {
        EnvCursor {
            list: SiMMLTrack::zero_list(),
            index: None,
        }
    }

    fn is_null(&self) -> bool {
        self.index.is_none()
    }

    fn value(&self) -> i32 {
        // C++ dereferences the element pointer unguarded (UB when null);
        // the port panics with a message instead.
        let index = self.index.expect("SiMMLTrack: null envelope element");
        self.list.borrow().value_at(index)
    }

    /// `elem = elem->next()`.
    fn advance(&mut self) {
        let current = match self.index {
            Some(index) => index,
            None => return,
        };
        self.index = self.list.borrow().next_of(current);
    }

    /// C++ `element == _envelope_zero_table->get_front()` pointer equality.
    fn points_to_zero_head(&self) -> bool {
        self.index == Some(0) && Rc::ptr_eq(&self.list, &SiMMLTrack::zero_list())
    }
}

pub struct SiMMLTrack {
    // Properties and data.
    channel: Option<ChannelRc>,
    pub executor: MMLExecutor,
    channel_settings: Option<Rc<RefCell<SiMMLChannelSettings>>>,
    mml_data: Option<Rc<RefCell<SiMMLData>>>,

    internal_track_id: i32,
    track_number: i32,
    channel_number: i32,

    process_mode: i32,
    track_start_delay: i32,
    track_stop_delay: i32,
    stop_with_reset: bool,
    is_disposable: bool,
    priority: i32,
    default_fps: i32,

    velocity_mode: i32,
    velocity_shift: i32,
    expression_mode: i32,
    velocity: i32,
    expression: i32,

    pitch_index: i32,
    pitch_bend: i32,
    pitch_shift: i32,
    voice_index: i32,
    note: i32,
    note_shift: i32,
    quantize_ratio: f64,
    quantize_count: i32,

    // Settings; index 1 = note_on, 0 = note_off (C++ `[2]` arrays).
    setting_process_mode: [i32; 2],
    setting_envelope_exp: [EnvCursor; 2],
    setting_envelope_voice: [EnvCursor; 2],
    setting_envelope_note: [EnvCursor; 2],
    setting_envelope_pitch: [EnvCursor; 2],
    setting_envelope_filter: [EnvCursor; 2],
    setting_exp_offset: [bool; 2],
    setting_pns_or: [bool; 2],
    setting_counter_exp: [i32; 2],
    setting_counter_voice: [i32; 2],
    setting_counter_note: [i32; 2],
    setting_counter_pitch: [i32; 2],
    setting_counter_filter: [i32; 2],
    table_envelope_mod_amp: [Option<Rc<RefCell<SinglyLinkedList>>>; 2],
    table_envelope_mod_pitch: [Option<Rc<RefCell<SinglyLinkedList>>>; 2],
    setting_sweep_step: [i32; 2],
    setting_sweep_end: [i32; 2],
    envelope_interval: i32,

    // Envelopes (live cursors; C++ `Element*` / list pointers).
    envelope_exp: EnvCursor,
    envelope_voice: EnvCursor,
    envelope_note: EnvCursor,
    envelope_pitch: EnvCursor,
    envelope_filter: EnvCursor,
    counter_exp: i32,
    max_counter_exp: i32,
    counter_voice: i32,
    max_counter_voice: i32,
    counter_note: i32,
    max_counter_note: i32,
    counter_pitch: i32,
    max_counter_pitch: i32,
    counter_filter: i32,
    max_counter_filter: i32,
    envelope_mod_amp: Option<Rc<RefCell<SinglyLinkedList>>>,
    envelope_mod_pitch: Option<Rc<RefCell<SinglyLinkedList>>>,
    sweep_step: i32,
    sweep_end: i32,
    sweep_pitch: i32,
    envelope_exp_offset: i32,
    envelope_pitch_active: bool,
    residue: i32,

    // Events.
    event_mask: u32,
    callback_before_note_on: Option<TrackFn>,
    callback_before_note_off: Option<TrackFn>,
    // `None` runs the C++ default member callback (`_channel->set_register`).
    callback_update_register: Option<UpdateRegisterFn>,
    event_trigger_on: Option<TrackFn>,
    event_trigger_off: Option<TrackFn>,
    event_trigger_id: i32,
    event_trigger_type_on: i32,
    event_trigger_type_off: i32,

    // Playback.
    key_on_counter: i32,
    key_on_length: i32,
    key_on_delay: i32,
    flag_no_key_on: bool,
}

impl SiMMLTrack {
    /// C++ `SiMMLTrack::initialize()`.
    pub fn initialize_statics() {
        ZERO_TABLE.with(|slot| {
            if slot.borrow().is_none() {
                *slot.borrow_mut() =
                    Some(Rc::new(RefCell::new(SinglyLinkedList::new_size_loop(1, 0, true))));
            }
        });
    }

    /// C++ `SiMMLTrack::finalize()`.
    pub fn finalize_statics() {
        ZERO_TABLE.with(|slot| *slot.borrow_mut() = None);
    }

    fn zero_list() -> Rc<RefCell<SinglyLinkedList>> {
        // C++ uses the singleton unguarded after `initialize()`; the port
        // lazily rebuilds it (behavior-identical for valid call order).
        SiMMLTrack::initialize_statics();
        ZERO_TABLE.with(|slot| slot.borrow().clone().unwrap())
    }

    // --- properties and data ---

    pub fn get_channel(&self) -> Option<&ChannelRc> {
        self.channel.as_ref()
    }
    pub fn set_channel(&mut self, p_channel: Option<ChannelRc>) {
        self.channel = p_channel;
    }

    fn chan(&self) -> &ChannelRc {
        // C++ dereferences `_channel` unguarded on these paths (valid once
        // `reset`/`initialize_tone` ran); the port expects.
        self.channel.as_ref().expect("SiMMLTrack: _channel is null")
    }

    pub fn get_executor(&self) -> &MMLExecutor {
        &self.executor
    }

    pub fn get_track_number(&self) -> i32 {
        self.track_number
    }
    pub fn set_track_number(&mut self, p_number: i32) {
        self.track_number = p_number;
    }

    pub fn get_internal_track_id(&self) -> i32 {
        self.internal_track_id
    }
    pub fn get_track_id(&self) -> i32 {
        self.internal_track_id & TRACK_ID_FILTER
    }
    pub fn get_track_type_id(&self) -> i32 {
        self.internal_track_id & TRACK_TYPE_FILTER
    }

    pub fn get_mml_data(&self) -> Option<Rc<RefCell<SiMMLData>>> {
        self.mml_data.clone()
    }

    pub fn get_bpm_settings(&self) -> Option<Rc<RefCell<BeatsPerMinute>>> {
        let data = self.mml_data.as_ref()?;
        if (self.internal_track_id & TRACK_TYPE_FILTER) != MML_TRACK {
            return data.borrow().base.get_bpm_settings();
        }
        None
    }

    pub fn get_channel_number(&self) -> i32 {
        self.channel_number
    }
    pub fn set_channel_number(&mut self, p_number: i32) {
        self.channel_number = p_number;
    }
    pub fn get_program_number(&self) -> i32 {
        self.voice_index
    }

    pub fn get_track_start_delay(&self) -> i32 {
        self.track_start_delay
    }
    pub fn get_track_stop_delay(&self) -> i32 {
        self.track_stop_delay
    }

    pub fn get_priority(&self) -> i32 {
        // Non-disposable and currently playing tracks always have top priority.
        if !self.is_disposable || self.is_playing_sequence() {
            return 0;
        }
        self.priority
    }

    pub fn is_active(&self) -> bool {
        if !self.is_disposable {
            return true;
        }
        self.executor.get_pointer().is_some() || !self.chan().borrow().is_idling()
    }

    pub fn is_playing_sequence(&self) -> bool {
        (self.internal_track_id & TRACK_TYPE_FILTER) != DRIVER_NOTE
            && self.executor.get_pointer().is_some()
    }

    pub fn is_finished(&self) -> bool {
        self.executor.get_pointer().is_none() && self.chan().borrow().is_idling()
    }

    pub fn is_disposable(&self) -> bool {
        self.is_disposable
    }
    pub fn set_disposable(&mut self) {
        self.is_disposable = true;
    }

    pub fn get_velocity_mode(&self) -> i32 {
        self.velocity_mode
    }
    pub fn get_velocity_shift(&self) -> i32 {
        self.velocity_shift
    }
    pub fn set_velocity_shift(&mut self, p_value: i32) {
        self.velocity_shift = p_value;
    }
    pub fn get_expression_mode(&self) -> i32 {
        self.expression_mode
    }
    pub fn get_velocity(&self) -> i32 {
        self.velocity
    }
    pub fn get_expression(&self) -> i32 {
        self.expression
    }
    pub fn get_pitch_bend(&self) -> i32 {
        self.pitch_bend
    }
    pub fn get_pitch_shift(&self) -> i32 {
        self.pitch_shift
    }
    pub fn set_pitch_shift(&mut self, p_value: i32) {
        self.pitch_shift = p_value;
    }
    pub fn get_module_type(&self) -> i32 {
        match &self.channel_settings {
            Some(settings) => settings.borrow().get_module_type(),
            None => crate::sion_enums::MODULE_PSG,
        }
    }
    pub fn is_pitch_sweeping(&self) -> bool {
        self.sweep_step != 0
    }
    pub fn get_pitch_sweep_target(&self) -> i32 {
        self.sweep_end >> FIXED_BITS
    }
    pub fn get_note(&self) -> i32 {
        self.note
    }
    pub fn get_note_shift(&self) -> i32 {
        self.note_shift
    }
    pub fn set_note_shift(&mut self, p_value: i32) {
        self.note_shift = p_value;
    }
    pub fn get_quantize_ratio(&self) -> f64 {
        self.quantize_ratio
    }
    pub fn set_quantize_ratio(&mut self, p_value: f64) {
        self.quantize_ratio = p_value;
    }
    pub fn get_quantize_count(&self) -> i32 {
        self.quantize_count
    }
    pub fn set_quantize_count(&mut self, p_value: i32) {
        self.quantize_count = p_value;
    }

    pub fn get_event_mask(&self) -> u32 {
        self.event_mask
    }
    pub fn set_event_mask(&mut self, p_mask: u32) {
        self.event_mask = p_mask;
    }
    pub fn get_event_trigger_id(&self) -> i32 {
        self.event_trigger_id
    }
    pub fn get_event_trigger_type_on(&self) -> i32 {
        self.event_trigger_type_on
    }
    pub fn get_event_trigger_type_off(&self) -> i32 {
        self.event_trigger_type_off
    }
    pub fn get_buffer_index(&self) -> i32 {
        self.chan().borrow().get_buffer_index()
    }

    pub fn get_channel_settings(&self) -> Option<Rc<RefCell<SiMMLChannelSettings>>> {
        self.channel_settings.clone()
    }

    /// C++ `set_channel_parameters(std::vector<int>)`.
    pub fn set_channel_parameters(
        &mut self,
        p_params: Vec<i32>,
        ctx: &mut dyn ChipContext,
    ) -> Option<SeqRc> {
        let mut sequence = None;
        if p_params[0] != i32::MIN {
            let settings = self.channel_settings.clone().expect("SiMMLTrack: settings null");
            sequence = settings.borrow().select_tone(self, p_params[0], ctx);
            self.voice_index = p_params[0];
        }
        self.chan().borrow_mut().set_parameters(p_params, ctx);
        sequence
    }

    // --- volume / velocity ---

    fn apply_volume_tables(&mut self) {
        let vm = self.velocity_mode as usize;
        let em = self.expression_mode as usize;
        let (velocity_table, expression_table) = {
            let table = chip_ref_table::instance();
            let table = table.borrow();
            (
                table.eg_total_level_tables[vm],
                table.eg_total_level_tables[em],
            )
        };
        self.chan()
            .borrow_mut()
            .set_volume_tables(&velocity_table, &expression_table);
    }

    pub fn set_velocity_mode(&mut self, p_mode: i32) {
        self.velocity_mode = if p_mode >= 0 && (p_mode as usize) < chip_ref_table::VM_MAX {
            p_mode
        } else {
            chip_ref_table::VM_LINEAR as i32
        };
        self.apply_volume_tables();
    }

    pub fn set_expression_mode(&mut self, p_mode: i32) {
        self.expression_mode = if p_mode >= 0 && (p_mode as usize) < chip_ref_table::VM_MAX {
            p_mode
        } else {
            chip_ref_table::VM_LINEAR as i32
        };
        self.apply_volume_tables();
    }

    pub fn set_velocity(&mut self, p_value: i32) {
        self.velocity = clampi(p_value, 0, 512);
        let (expression, velocity) = (self.expression, self.velocity);
        self.chan().borrow_mut().offset_volume(expression, velocity);
    }

    pub fn set_expression(&mut self, p_value: i32) {
        self.expression = clampi(p_value, 0, 128);
        let (expression, velocity) = (self.expression, self.velocity);
        self.chan().borrow_mut().offset_volume(expression, velocity);
    }

    pub fn get_output_level(&self) -> f64 {
        let volume = self.chan().borrow().get_master_volume();
        if volume == 0 {
            return self.velocity as f64 * self.expression as f64 * 0.0000152587890625;
        }
        volume as f64 * self.velocity as f64 * self.expression as f64 * 2.384185791015625e-7
    }

    pub fn set_pitch_bend(&mut self, p_value: i32) {
        self.pitch_bend = p_value;
        let pitch = self.pitch_index + self.pitch_bend;
        self.chan().borrow_mut().set_pitch(pitch);
    }

    pub fn set_note_immediately(
        &mut self,
        p_note: i32,
        p_sample_length: i32,
        p_slur: bool,
        ctx: &mut dyn ChipContext,
    ) {
        // Play with key off when quantize_ratio == 0 or p_sample_length != 0.
        if !p_slur && (self.quantize_ratio == 0.0 || p_sample_length > 0) {
            self.key_on_length = (p_sample_length as f64 * self.quantize_ratio) as i32
                - self.quantize_count
                - self.key_on_delay;
            if self.key_on_length < 1 {
                self.key_on_length = 1;
            }
        } else {
            self.key_on_length = 0;
        }

        self.mml_key_on(p_note, ctx);
        self.flag_no_key_on = p_slur;
    }

    // --- channel properties ---

    pub fn set_channel_module_type(
        &mut self,
        p_type: i32,
        p_channel_num: i32,
        p_tone_num: i32,
        ctx: &mut dyn ChipContext,
    ) {
        let settings = {
            let table = mml_ref_table::instance().expect("SiMMLRefTable not initialized");
            let table = table.borrow();
            // C++ `channel_settings_map[p_type]` (operator[] inserts a null
            // entry then crashes on the null deref below); the port expects
            // the key to exist.
            table
                .channel_settings_map
                .get(&p_type)
                .cloned()
                .expect("SiMMLTrack: channel_settings_map has no module entry")
        };
        self.channel_settings = Some(settings);

        let buffer_index = self.chan().borrow().get_buffer_index();
        let settings = self.channel_settings.clone().unwrap();
        self.voice_index = settings.borrow().initialize_tone(
            self,
            p_channel_num,
            buffer_index as usize,
            ctx,
        );
        if p_tone_num >= 0 {
            self.voice_index = p_tone_num;
            let settings = self.channel_settings.clone().unwrap();
            settings.borrow().select_tone(self, p_tone_num, ctx);
        }
    }

    pub fn reset_volume_offset(&mut self) {
        let (expression, velocity) = (self.expression, self.velocity);
        self.chan().borrow_mut().offset_volume(expression, velocity);
    }

    pub fn get_master_volume(&self) -> i32 {
        self.chan().borrow().get_master_volume()
    }
    pub fn set_master_volume(&mut self, p_value: i32) {
        self.chan().borrow_mut().set_master_volume(p_value);
    }
    pub fn get_effect_send1(&mut self) -> f64 {
        self.chan().borrow_mut().get_stream_send(1)
    }
    pub fn get_effect_send2(&mut self) -> f64 {
        self.chan().borrow_mut().get_stream_send(2)
    }
    pub fn get_effect_send3(&mut self) -> f64 {
        self.chan().borrow_mut().get_stream_send(3)
    }
    pub fn get_effect_send4(&mut self) -> f64 {
        self.chan().borrow_mut().get_stream_send(4)
    }

    /// C++ `STREAM_SEND_SAN(m_value)`.
    fn stream_send_san(p_value: i32) -> f64 {
        if p_value < 0 {
            0.0
        } else if p_value > 128 {
            1.0
        } else {
            p_value as f64 * 0.0078125
        }
    }

    pub fn set_effect_send1(&mut self, p_value: i32) {
        let level = Self::stream_send_san(p_value);
        self.chan().borrow_mut().set_stream_send(1, level);
    }
    pub fn set_effect_send2(&mut self, p_value: i32) {
        let level = Self::stream_send_san(p_value);
        self.chan().borrow_mut().set_stream_send(2, level);
    }
    pub fn set_effect_send3(&mut self, p_value: i32) {
        let level = Self::stream_send_san(p_value);
        self.chan().borrow_mut().set_stream_send(3, level);
    }
    pub fn set_effect_send4(&mut self, p_value: i32) {
        let level = Self::stream_send_san(p_value);
        self.chan().borrow_mut().set_stream_send(4, level);
    }

    pub fn is_mute(&self) -> bool {
        self.chan().borrow().is_mute()
    }
    pub fn set_mute(&mut self, p_value: bool) {
        self.chan().borrow_mut().set_mute(p_value);
    }
    pub fn get_pan(&self) -> i32 {
        self.chan().borrow().get_pan()
    }
    pub fn set_pan(&mut self, p_value: i32) {
        self.chan().borrow_mut().set_pan(p_value);
    }

    // --- envelopes ---

    /// C++ `_make_modulation_table`. The C++ pre-allocates
    /// `p_delay + p_term + 1` zero elements and walks raw element pointers;
    /// every write stays inside the list, so the port appends the values
    /// directly. A C++ walk past the tail (unreachable when the counts are
    /// consistent) would write UB; the port skips such writes.
    fn make_modulation_table(
        p_depth: i32,
        p_end_depth: i32,
        p_delay: i32,
        p_term: i32,
    ) -> Rc<RefCell<SinglyLinkedList>> {
        let mut values: Vec<i32> = Vec::new();
        for _ in 0..p_delay {
            values.push(p_depth);
        }
        if p_term != 0 {
            let mut depth = p_depth.wrapping_shl(FIXED_BITS as u32);
            let step = (p_end_depth.wrapping_shl(FIXED_BITS as u32) - depth) / p_term;
            for _ in 0..p_term {
                values.push(depth >> FIXED_BITS);
                depth = depth.wrapping_add(step);
            }
        }
        values.push(p_end_depth);

        let mut list = SinglyLinkedList::new_empty();
        for value in values {
            list.append(value);
        }
        list.front();
        Rc::new(RefCell::new(list))
    }

    pub fn set_portament(&mut self, p_frame: i32) {
        self.setting_sweep_step[1] = p_frame;
        if p_frame != 0 {
            self.setting_pns_or[1] = true;
            self.enable_envelope_mode(1);
        } else {
            self.disable_envelope_mode(1);
        }
    }

    pub fn set_envelope_fps(&mut self, p_fps: i32) {
        let table = chip_ref_table::instance();
        self.envelope_interval = table.borrow().sampling_rate / p_fps;
    }

    pub fn set_release_sweep(&mut self, p_sweep: i32) {
        self.setting_sweep_step[0] = p_sweep << FIXED_BITS;
        self.setting_sweep_end[0] = if p_sweep < 0 { 0 } else { SWEEP_MAX };

        if p_sweep != 0 {
            self.setting_pns_or[0] = true;
            self.enable_envelope_mode(0);
        } else {
            self.disable_envelope_mode(0);
        }
    }

    pub fn set_modulation_envelope(
        &mut self,
        p_is_pitch_mod: bool,
        p_depth: i32,
        p_end_depth: i32,
        p_delay: i32,
        p_term: i32,
    ) {
        let table: &mut [Option<Rc<RefCell<SinglyLinkedList>>>; 2] = if p_is_pitch_mod {
            &mut self.table_envelope_mod_pitch
        } else {
            &mut self.table_envelope_mod_amp
        };

        // Releasing the previous table happens on assignment (C++ `delete`).
        if (p_depth >= 0 && p_depth < p_end_depth) || (p_depth < 0 && p_depth > p_end_depth) {
            table[1] = Some(Self::make_modulation_table(
                p_depth, p_end_depth, p_delay, p_term,
            ));
            self.enable_envelope_mode(1);
        } else {
            table[1] = None;
            if p_is_pitch_mod {
                self.chan().borrow_mut().set_pitch_modulation(p_depth);
            } else {
                self.chan().borrow_mut().set_amplitude_modulation(p_depth);
            }
            self.disable_envelope_mode(1);
        }
    }

    fn table_cursor(p_table: &Option<Rc<RefCell<SiMMLEnvelopeTable>>>) -> Option<EnvCursor> {
        let table = p_table.as_ref()?;
        let list = table.borrow().data.clone()?;
        let head = table.borrow().get_head()?;
        Some(EnvCursor {
            list,
            index: Some(head),
        })
    }

    pub fn set_tone_envelope(
        &mut self,
        p_note_on: usize,
        p_table: &Option<Rc<RefCell<SiMMLEnvelopeTable>>>,
        p_step: i32,
    ) {
        match Self::table_cursor(p_table) {
            Some(cursor) if p_step != 0 => {
                self.setting_envelope_voice[p_note_on] = cursor;
                self.setting_counter_voice[p_note_on] = p_step;
                self.enable_envelope_mode(p_note_on as i32);
            }
            _ => {
                self.setting_envelope_voice[p_note_on] = EnvCursor::null();
                self.disable_envelope_mode(p_note_on as i32);
            }
        }
    }

    pub fn set_amplitude_envelope(
        &mut self,
        p_note_on: usize,
        p_table: &Option<Rc<RefCell<SiMMLEnvelopeTable>>>,
        p_step: i32,
        p_offset: bool,
    ) {
        match Self::table_cursor(p_table) {
            Some(cursor) if p_step != 0 => {
                self.setting_envelope_exp[p_note_on] = cursor;
                self.setting_counter_exp[p_note_on] = p_step;
                self.setting_exp_offset[p_note_on] = p_offset;
                self.enable_envelope_mode(p_note_on as i32);
            }
            _ => {
                self.setting_envelope_exp[p_note_on] = EnvCursor::null();
                self.disable_envelope_mode(p_note_on as i32);
            }
        }
    }

    pub fn set_filter_envelope(
        &mut self,
        p_note_on: usize,
        p_table: &Option<Rc<RefCell<SiMMLEnvelopeTable>>>,
        p_step: i32,
    ) {
        match Self::table_cursor(p_table) {
            Some(cursor) if p_step != 0 => {
                self.setting_envelope_filter[p_note_on] = cursor;
                self.setting_counter_filter[p_note_on] = p_step;
                self.enable_envelope_mode(p_note_on as i32);
            }
            _ => {
                self.setting_envelope_filter[p_note_on] = EnvCursor::null();
                self.disable_envelope_mode(p_note_on as i32);
            }
        }
    }

    pub fn set_pitch_envelope(
        &mut self,
        p_note_on: usize,
        p_table: &Option<Rc<RefCell<SiMMLEnvelopeTable>>>,
        p_step: i32,
    ) {
        match Self::table_cursor(p_table) {
            Some(cursor) if p_step != 0 => {
                self.setting_envelope_pitch[p_note_on] = cursor;
                self.setting_counter_pitch[p_note_on] = p_step;
                self.setting_pns_or[p_note_on] = true;
                self.enable_envelope_mode(p_note_on as i32);
            }
            _ => {
                self.setting_envelope_pitch[p_note_on] = EnvCursor::zero();
                self.disable_envelope_mode(p_note_on as i32);
            }
        }
    }

    pub fn set_note_envelope(
        &mut self,
        p_note_on: usize,
        p_table: &Option<Rc<RefCell<SiMMLEnvelopeTable>>>,
        p_step: i32,
    ) {
        match Self::table_cursor(p_table) {
            Some(cursor) if p_step != 0 => {
                self.setting_envelope_note[p_note_on] = cursor;
                self.setting_counter_note[p_note_on] = p_step;
                self.setting_pns_or[p_note_on] = true;
                self.enable_envelope_mode(p_note_on as i32);
            }
            _ => {
                self.setting_envelope_note[p_note_on] = EnvCursor::zero();
                self.disable_envelope_mode(p_note_on as i32);
            }
        }
    }

    // --- events ---

    pub fn set_update_register_callback(
        &mut self,
        p_func: Option<UpdateRegisterFn>,
    ) {
        self.callback_update_register = p_func;
    }

    /// C++ `call_update_register` — `None` callback runs the C++ default
    /// member handler (`_channel->set_register`).
    pub fn call_update_register(&mut self, p_address: i32, p_data: i32, ctx: &mut dyn ChipContext) {
        if let Some(callback) = self.callback_update_register.clone() {
            callback(self, p_address, p_data);
        } else {
            self.chan().borrow_mut().set_register(p_address, p_data, ctx);
        }
    }

    pub fn set_note_on_callback(&mut self, p_func: Option<TrackFn>) {
        self.callback_before_note_on = p_func;
    }
    pub fn set_note_off_callback(&mut self, p_func: Option<TrackFn>) {
        self.callback_before_note_off = p_func;
    }

    pub fn set_event_trigger_callbacks(
        &mut self,
        p_id: i32,
        p_note_on_type: i32,
        p_note_off_type: i32,
    ) {
        self.event_trigger_id = p_id;
        self.event_trigger_type_on = p_note_on_type;
        self.event_trigger_type_off = p_note_off_type;

        self.callback_before_note_on = if self.event_trigger_type_on != NO_EVENTS {
            self.event_trigger_on.clone()
        } else {
            None
        };
        self.callback_before_note_off = if self.event_trigger_type_off != NO_EVENTS {
            self.event_trigger_off.clone()
        } else {
            None
        };
    }

    pub fn trigger_note_callback(&mut self, p_note_on: bool) {
        if p_note_on {
            if let Some(callback) = self.callback_before_note_on.clone() {
                callback(self);
            }
        } else if let Some(callback) = self.callback_before_note_off.clone() {
            callback(self);
        }
    }

    pub fn trigger_note_on_event(&mut self, p_id: i32, p_trigger_type: i32) {
        if p_trigger_type == NO_EVENTS {
            return;
        }

        // Remember current settings, swap in the event's, run, restore.
        let current_id = self.event_trigger_id;
        let current_type = self.event_trigger_type_on;

        self.event_trigger_id = p_id;
        self.event_trigger_type_on = p_trigger_type;
        if let Some(callback) = self.event_trigger_on.clone() {
            callback(self);
        }

        self.event_trigger_id = current_id;
        self.event_trigger_type_on = current_type;
    }

    // --- MML handlers ---

    pub fn handle_rest_event(&mut self) {
        self.flag_no_key_on = false;
    }

    pub fn handle_note_event(&mut self, p_note: i32, p_length: i32, ctx: &mut dyn ChipContext) {
        self.key_on_length = (p_length as f64 * self.quantize_ratio) as i32
            - self.quantize_count
            - self.key_on_delay;
        if self.key_on_length < 1 {
            self.key_on_length = 1;
        }
        self.mml_key_on(p_note, ctx);
    }

    pub fn handle_slur(&mut self) {
        self.flag_no_key_on = true;
        self.key_on_counter = 0;
    }

    pub fn handle_slur_weak(&mut self) {
        self.key_on_counter = 0;
    }

    pub fn handle_pitch_bend(&mut self, p_next_note: i32, p_term: i32) {
        let start_pitch = self.chan().borrow().get_pitch();

        let mut end_pitch_base = (p_next_note + self.note_shift) << 6;
        if end_pitch_base == 0 {
            end_pitch_base = start_pitch & 63;
        }
        let end_pitch = end_pitch_base + self.pitch_shift;

        self.handle_slur();
        if start_pitch == end_pitch {
            return;
        }

        // 64-bit intermediate + `max(term, 1)` guard (session-10 fix kept
        // verbatim; see the doc comment above).
        let term = std::cmp::max(p_term, 1);
        self.sweep_step = ((((end_pitch - start_pitch) as i64) << FIXED_BITS)
            * self.envelope_interval as i64
            / term as i64) as i32;
        self.sweep_end = end_pitch << FIXED_BITS;
        self.sweep_pitch = start_pitch << FIXED_BITS;
        self.envelope_pitch_active = true;
        self.envelope_note = self.setting_envelope_note[1].clone();
        self.envelope_pitch = self.setting_envelope_pitch[1].clone();

        self.process_mode = PROCESS_ENVELOPE;
    }

    pub fn handle_velocity(&mut self, p_value: i32) {
        let velocity = p_value << self.velocity_shift;
        self.set_velocity(velocity);
    }

    pub fn handle_velocity_shift(&mut self, p_value: i32) {
        let velocity = self.velocity + (p_value << self.velocity_shift);
        self.set_velocity(velocity);
    }

    // --- playback (private) ---

    fn enable_envelope_mode(&mut self, p_note_on: i32) {
        self.setting_process_mode[p_note_on as usize] = PROCESS_ENVELOPE;
    }

    fn disable_envelope_mode(&mut self, p_note_on: i32) {
        let index = p_note_on as usize;
        if self.setting_sweep_step[index] == 0
            && self.setting_envelope_pitch[index].points_to_zero_head()
            && self.setting_envelope_note[index].points_to_zero_head()
        {
            self.setting_pns_or[index] = false;
        }

        if !self.setting_pns_or[index]
            && self.table_envelope_mod_amp[index].is_none()
            && self.table_envelope_mod_pitch[index].is_none()
            && self.setting_envelope_exp[index].is_null()
            && self.setting_envelope_filter[index].is_null()
            && self.setting_envelope_voice[index].is_null()
        {
            self.setting_process_mode[index] = PROCESS_NORMAL;
        }
    }

    pub fn prepare_buffer(&mut self, p_buffer_length: i32, ctx: &mut dyn ChipContext) -> i32 {
        match self.mml_data.clone() {
            Some(data) => data.borrow().register_ref_stencils(),
            None => SiMMLData::clear_ref_stencils(),
        }

        // No delay.
        if self.track_start_delay == 0 {
            return p_buffer_length;
        }
        // Wait for the starting sound.
        if p_buffer_length <= self.track_start_delay {
            self.track_start_delay -= p_buffer_length;
            return 0;
        }
        // Start sound in this frame.
        let length = p_buffer_length - self.track_start_delay;
        let delay = self.track_start_delay;
        self.chan().borrow_mut().buffer_no_process(delay, ctx);
        self.track_start_delay = 0;
        self.priority += 1;
        length
    }

    /// Edition-2024 `if let` scrutinee temps span the whole block, so the
    /// modulation-table read borrows are hoisted out of the `if let` before
    /// `next()`'s mut borrow (same trap as the wave-7b executor-pointer fix).
    fn buffer_envelope(&mut self, p_length: i32, p_step: i32, ctx: &mut dyn ChipContext) -> i32 {
        let mut remaining_length = p_length;
        let mut current_step = p_step;

        while remaining_length >= current_step {
            if current_step > 0 {
                self.chan().borrow_mut().buffer(current_step, ctx);
            }

            if !self.envelope_exp.is_null() && self.counter_exp == 1 {
                self.counter_exp -= 1;
                let expression =
                    clampi(self.envelope_exp_offset + self.envelope_exp.value(), 0, 128);
                let velocity = self.velocity;
                self.chan().borrow_mut().offset_volume(expression, velocity);
                self.envelope_exp.advance();
                self.counter_exp = self.max_counter_exp;
            }

            if self.envelope_pitch_active {
                let pitch = self.envelope_pitch.value()
                    + (self.envelope_note.value() << 6)
                    + (self.sweep_pitch >> FIXED_BITS);
                self.chan().borrow_mut().set_pitch(pitch);

                if self.counter_pitch == 1 {
                    self.counter_pitch -= 1;
                    self.envelope_pitch.advance();
                    self.counter_pitch = self.max_counter_pitch;
                }
                if self.counter_note == 1 {
                    self.counter_note -= 1;
                    self.envelope_note.advance();
                    self.counter_note = self.max_counter_note;
                }

                self.sweep_pitch += self.sweep_step;
                if (self.sweep_step > 0 && self.sweep_pitch > self.sweep_end)
                    || (self.sweep_step <= 0 && self.sweep_pitch < self.sweep_end)
                {
                    self.sweep_pitch = self.sweep_end;
                    self.sweep_step = 0;
                }
            }

            if !self.envelope_filter.is_null() && self.counter_filter == 1 {
                self.counter_filter -= 1;
                let cutoff = self.envelope_filter.value();
                self.chan().borrow_mut().offset_filter(cutoff);
                self.envelope_filter.advance();
                self.counter_filter = self.max_counter_filter;
            }

            if !self.envelope_voice.is_null() && self.counter_voice == 1 {
                self.counter_voice -= 1;
                let voice_index = self.envelope_voice.value();
                let settings = self.channel_settings.clone().expect("SiMMLTrack: settings null");
                settings.borrow().select_tone(self, voice_index, ctx);
                self.envelope_voice.advance();
                self.counter_voice = self.max_counter_voice;
            }

            // Modulation tables walk the list's own cursor (`get()` null
            // past the non-looped tail clears that modulation, quirk kept).
            if let Some(list) = self.envelope_mod_amp.clone() {
                let value = list.borrow().get();
                if let Some(value) = value {
                    self.chan().borrow_mut().set_amplitude_modulation(value);
                    list.borrow_mut().next();
                } else {
                    self.envelope_mod_amp = None;
                }
            }
            if let Some(list) = self.envelope_mod_pitch.clone() {
                let value = list.borrow().get();
                if let Some(value) = value {
                    self.chan().borrow_mut().set_pitch_modulation(value);
                    list.borrow_mut().next();
                } else {
                    self.envelope_mod_pitch = None;
                }
            }

            remaining_length -= current_step;
            current_step = self.envelope_interval;
        }

        // Process the remainder.
        if remaining_length > 0 {
            self.chan().borrow_mut().buffer(remaining_length, ctx);
        }

        self.envelope_interval - remaining_length
    }

    fn process_buffer(&mut self, p_length: i32, ctx: &mut dyn ChipContext) {
        match self.process_mode {
            PROCESS_ENVELOPE => {
                let residue = self.residue;
                self.residue = self.buffer_envelope(p_length, residue, ctx);
            }
            _ => {
                self.chan().borrow_mut().buffer(p_length, ctx);
            }
        }
    }

    pub fn buffer(&mut self, p_length: i32, ctx: &mut dyn ChipContext) {
        let mut length = p_length;

        // Check if the track is stopping.
        let mut track_stopped = false;
        let mut track_stop_resume = 0;

        if self.track_stop_delay > 0 {
            if self.track_stop_delay > length {
                self.track_stop_delay -= length;
            } else {
                track_stop_resume = length - self.track_stop_delay;
                track_stopped = true;
                length = self.track_stop_delay;
                self.track_stop_delay = 0;
            }
        }

        // Buffering.
        if self.key_on_counter == 0 {
            // No status change.
            self.process_buffer(length, ctx);
        } else if self.key_on_counter > length {
            // Decrement the counter.
            self.process_buffer(length, ctx);
            self.key_on_counter -= length;
        } else {
            // Process -> Toggle key -> Process again.
            length -= self.key_on_counter;
            let counter = self.key_on_counter;
            self.process_buffer(counter, ctx);
            self.toggle_key(ctx);
            if length > 0 {
                self.process_buffer(length, ctx);
            }
        }

        if track_stopped {
            if self.executor.get_pointer().is_some() {
                self.executor.stop();
                if self.stop_with_reset {
                    self.key_off_now();
                    self.note = -1;
                    self.chan().borrow_mut().reset();
                }
            } else if self.chan().borrow().is_note_on() {
                self.key_off_now();
                self.note = -1;
                if self.stop_with_reset {
                    self.chan().borrow_mut().reset();
                }
            }

            if track_stop_resume > 0 {
                self.process_buffer(track_stop_resume, ctx);
            }
        }
    }

    fn toggle_key(&mut self, ctx: &mut dyn ChipContext) {
        if self.chan().borrow().is_note_on() {
            self.key_off_now();
        } else {
            self.key_on_now(ctx);
        }
    }

    /// C++ `_key_on()` (process-time key-on; the public `key_on` only
    /// re-arms the executor).
    fn key_on_now(&mut self, ctx: &mut dyn ChipContext) {
        if let Some(callback) = self.callback_before_note_on.clone() {
            callback(self);
        }

        // Update pitch.
        let old_pitch = self.chan().borrow().get_pitch();
        self.pitch_index = ((self.note + self.note_shift) << 6) + self.pitch_shift;
        let pitch = self.pitch_index + self.pitch_bend;
        self.chan().borrow_mut().set_pitch(pitch);

        if self.flag_no_key_on {
            // Portament.
            if self.setting_sweep_step[1] > 0 {
                self.chan().borrow_mut().set_pitch(old_pitch);
                self.sweep_step = ((self.pitch_index - old_pitch) << FIXED_BITS)
                    / self.setting_sweep_step[1];
                self.sweep_end = self.pitch_index << FIXED_BITS;
                self.sweep_pitch = old_pitch << FIXED_BITS;
            } else {
                let current = self.chan().borrow().get_pitch();
                self.sweep_pitch = current << FIXED_BITS;
            }

            // Try to set envelope off.
            self.disable_envelope_mode(1);
        } else {
            // Reset previous envelope.
            if self.process_mode == PROCESS_ENVELOPE {
                let (expression, velocity) = (self.expression, self.velocity);
                self.chan().borrow_mut().offset_volume(expression, velocity);
                let voice_index = self.voice_index;
                let settings = self.channel_settings.clone().expect("SiMMLTrack: settings null");
                settings.borrow().select_tone(self, voice_index, ctx);
                self.chan().borrow_mut().offset_filter(128);
            }

            // Previous note off.
            if self.chan().borrow().is_note_on() {
                if let Some(callback) = self.callback_before_note_off.clone() {
                    callback(self);
                }
                self.chan().borrow_mut().note_off();
            }

            self.update_process(1);
            self.chan().borrow_mut().note_on();
        }

        self.flag_no_key_on = false;
        self.key_on_counter = self.key_on_length;
    }

    /// C++ `_key_off()`.
    fn key_off_now(&mut self) {
        if let Some(callback) = self.callback_before_note_off.clone() {
            callback(self);
        }

        self.chan().borrow_mut().note_off();
        self.key_on_counter = 0;
        self.update_process(0);

        // Lower the priority.
        self.priority += 32;
    }

    fn update_process(&mut self, p_key_on: i32) {
        let key = p_key_on as usize;
        self.process_mode = self.setting_process_mode[key];
        if self.process_mode != PROCESS_ENVELOPE {
            return;
        }

        // Set envelope tables.
        self.envelope_exp = self.setting_envelope_exp[key].clone();
        self.envelope_voice = self.setting_envelope_voice[key].clone();
        self.envelope_note = self.setting_envelope_note[key].clone();
        self.envelope_pitch = self.setting_envelope_pitch[key].clone();
        self.envelope_filter = self.setting_envelope_filter[key].clone();

        // Set envelope counters.
        self.max_counter_exp = self.setting_counter_exp[key];
        self.max_counter_voice = self.setting_counter_voice[key];
        self.max_counter_note = self.setting_counter_note[key];
        self.max_counter_pitch = self.setting_counter_pitch[key];
        self.max_counter_filter = self.setting_counter_filter[key];

        self.counter_exp = 1;
        self.counter_voice = 1;
        self.counter_note = 1;
        self.counter_pitch = 1;
        self.counter_filter = 1;

        // Set modulation envelopes. (Cursor reset matches the original.)
        self.envelope_mod_amp = self.table_envelope_mod_amp[key].clone();
        if let Some(list) = &self.envelope_mod_amp {
            list.borrow_mut().front();
        }
        self.envelope_mod_pitch = self.table_envelope_mod_pitch[key].clone();
        if let Some(list) = &self.envelope_mod_pitch {
            list.borrow_mut().front();
        }

        // Set sweep.
        self.sweep_step = if p_key_on == 1 { 0 } else { self.setting_sweep_step[key] };
        self.sweep_end = if p_key_on == 1 { 0 } else { self.setting_sweep_end[key] };
        let current = self.chan().borrow().get_pitch();
        self.sweep_pitch = current << FIXED_BITS;

        // Set pitch values.
        self.envelope_exp_offset = if self.setting_exp_offset[key] { self.expression } else { 0 };
        self.envelope_pitch_active = self.setting_pns_or[key];

        // Activate filter.
        if !self.chan().borrow().is_filter_active() {
            let active = !self.envelope_filter.is_null();
            self.chan().borrow_mut().activate_filter(active);
        }

        // Reset index.
        self.residue = 0;
    }

    fn mml_key_on(&mut self, p_note: i32, ctx: &mut dyn ChipContext) {
        self.note = p_note;
        self.track_start_delay = 0;

        if self.key_on_delay != 0 {
            self.key_off_now();
            self.key_on_counter = self.key_on_delay;
        } else {
            self.key_on_now(ctx);
        }
    }

    // --- playback (public) ---

    pub fn key_on(&mut self, p_note: i32, p_tick_length: i32, p_sample_delay: i32) {
        self.track_start_delay = p_sample_delay;
        self.executor.execute_single_note(p_note, p_tick_length);
    }

    pub fn key_off(&mut self, p_sample_delay: i32, p_with_reset: bool, _ctx: &mut dyn ChipContext) {
        self.stop_with_reset = p_with_reset;

        if p_sample_delay != 0 {
            self.track_stop_delay = p_sample_delay;
        } else {
            self.key_off_now();
            self.note = -1;
            if self.stop_with_reset {
                self.chan().borrow_mut().reset();
            }
        }
    }

    pub fn bend_note(&mut self, p_to_note: i32, p_tick_length: i32) {
        self.executor.bend_single_note(p_to_note, p_tick_length);
    }

    pub fn sequence_on(
        &mut self,
        p_data: Option<Rc<RefCell<SiMMLData>>>,
        p_sequence: Option<SeqRc>,
        p_sample_length: i32,
        p_sample_delay: i32,
    ) {
        if p_sequence.is_none() {
            err_print!("Parameter \"p_sequence\" is null.");
            return;
        }

        self.mml_data = p_data;
        self.track_start_delay = p_sample_delay;
        self.track_stop_delay = p_sample_length;
        self.executor.initialize(p_sequence);
    }

    pub fn sequence_off(
        &mut self,
        p_sample_delay: i32,
        p_with_reset: bool,
        _ctx: &mut dyn ChipContext,
    ) {
        self.stop_with_reset = p_with_reset;

        if p_sample_delay != 0 {
            self.track_stop_delay = p_sample_delay;
        } else {
            self.executor.clear();
            if self.stop_with_reset {
                self.chan().borrow_mut().reset();
            }
        }
    }

    pub fn limit_key_length(&mut self, p_stop_delay: i32) {
        let length = p_stop_delay - self.track_start_delay;
        if length < self.key_on_length {
            self.key_on_length = length;
            self.key_on_counter = self.key_on_length;
        }
    }

    pub fn change_note_length(&mut self, p_length: i32) {
        self.key_on_counter = (p_length as f64 * self.quantize_ratio) as i32
            - self.quantize_count
            - self.key_on_delay;
        if self.key_on_counter < 1 {
            self.key_on_counter = 1;
        }
    }

    pub fn set_key_on_delay(&mut self, p_delay: i32) {
        self.key_on_delay = p_delay;
    }

    // --- init / reset ---

    pub fn reset(&mut self, p_buffer_index: i32, ctx: &mut dyn ChipContext) {
        // Channel module settings.
        let settings = {
            let table = mml_ref_table::instance().expect("SiMMLRefTable not initialized");
            table
                .borrow()
                .channel_settings_map
                .get(&crate::sion_enums::MODULE_PSG)
                .cloned()
                .expect("SiMMLTrack: MODULE_PSG channel settings missing")
        };
        self.channel_settings = Some(settings);
        self.channel_number = 0;

        // Initialize channel.
        if let Some(data) = self.mml_data.clone() {
            let data = data.borrow();
            self.velocity_shift = data.base.get_default_velocity_shift();
            self.velocity_mode = data.base.get_default_velocity_mode();
            self.expression_mode = data.base.get_default_expression_mode();
        } else {
            self.velocity_shift = 4;
            self.velocity_mode = chip_ref_table::VM_LINEAR as i32;
            self.expression_mode = chip_ref_table::VM_LINEAR as i32;
        }

        self.velocity = 256;
        self.expression = 128;
        self.pitch_bend = 0;
        self.note = -1;

        if let Some(old_channel) = self.channel.take() {
            manager::delete_channel(&old_channel);
        }
        let settings = self.channel_settings.clone().unwrap();
        // This sets the channel.
        self.voice_index =
            settings.borrow().initialize_tone(self, i32::MIN, p_buffer_index as usize, ctx);

        self.apply_volume_tables();

        // Initialize parameters.
        self.note_shift = 0;
        self.pitch_shift = 0;
        self.quantize_ratio = 1.0;
        self.quantize_count = 0;

        self.event_mask = NO_MASK;

        self.key_on_counter = 0;
        self.key_on_length = 0;
        self.key_on_delay = 0;
        self.flag_no_key_on = false;

        self.process_mode = PROCESS_NORMAL;
        self.track_start_delay = 0;
        self.track_stop_delay = 0;
        self.stop_with_reset = false;
        self.priority = 0;

        self.pitch_index = 0;
        self.sweep_pitch = 0;

        self.envelope_pitch_active = false;
        self.envelope_exp_offset = 0;
        let fps = self.default_fps;
        self.set_envelope_fps(fps);

        self.callback_before_note_on = None;
        self.callback_before_note_off = None;
        // `None` == C++ default member callback (`_default_update_register`).
        self.callback_update_register = None;

        self.residue = 0;

        self.envelope_exp = EnvCursor::null();
        self.envelope_voice = EnvCursor::null();
        self.envelope_note = EnvCursor::zero();
        self.envelope_pitch = EnvCursor::zero();
        self.envelope_filter = EnvCursor::null();
        self.envelope_mod_amp = None;
        self.envelope_mod_pitch = None;

        // Reset envelope tables.
        for i in 0..2 {
            self.setting_process_mode[i] = PROCESS_NORMAL;

            self.setting_envelope_exp[i] = EnvCursor::null();
            self.setting_envelope_voice[i] = EnvCursor::null();
            self.setting_envelope_note[i] = EnvCursor::zero();
            self.setting_envelope_pitch[i] = EnvCursor::zero();
            self.setting_envelope_filter[i] = EnvCursor::null();

            self.setting_pns_or[i] = false;
            self.setting_exp_offset[i] = false;

            self.setting_counter_exp[i] = 1;
            self.setting_counter_voice[i] = 1;
            self.setting_counter_note[i] = 1;
            self.setting_counter_pitch[i] = 1;
            self.setting_counter_filter[i] = 1;

            self.setting_sweep_step[i] = 0;
            self.setting_sweep_end[i] = 0;

            self.table_envelope_mod_amp[i] = None;
            self.table_envelope_mod_pitch[i] = None;
        }

        // Reset executor.
        self.executor.reset_pointer();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn initialize(
        &mut self,
        p_data: Option<Rc<RefCell<SiMMLData>>>,
        p_sequence: Option<SeqRc>,
        p_fps: i32,
        p_internal_track_id: i32,
        p_event_trigger_on: Option<TrackFn>,
        p_event_trigger_off: Option<TrackFn>,
        p_disposable: bool,
    ) {
        self.mml_data = p_data;

        self.default_fps = p_fps;
        self.internal_track_id = p_internal_track_id;
        self.is_disposable = p_disposable;

        self.event_trigger_on = p_event_trigger_on;
        self.event_trigger_off = p_event_trigger_off;
        self.event_trigger_id = -1;
        self.event_trigger_type_on = NO_EVENTS;
        self.event_trigger_type_off = NO_EVENTS;

        self.executor.initialize(p_sequence);
    }

    /// `SiMMLTrack()` — the C++ ctor only builds the executor and sizes the
    /// `[2]` tables; the C++ leaves envelope cursors null, so every cursor
    /// starts as the nullptr cursor here (`reset` then installs the real
    /// zero-table cursors before playback).
    pub fn new() -> Self {
        SiMMLTrack::initialize_statics();
        let null = EnvCursor::null();
        let zeros = [null.clone(), null.clone()];

        SiMMLTrack {
            channel: None,
            executor: MMLExecutor::new(),
            channel_settings: None,
            mml_data: None,

            internal_track_id: 0,
            track_number: 0,
            channel_number: 0,

            process_mode: PROCESS_NORMAL,
            track_start_delay: 0,
            track_stop_delay: 0,
            stop_with_reset: false,
            is_disposable: false,
            priority: 0,
            default_fps: 0,

            velocity_mode: 0,
            velocity_shift: 0,
            expression_mode: 0,
            velocity: 0,
            expression: 0,

            pitch_index: 0,
            pitch_bend: 0,
            pitch_shift: 0,
            voice_index: 0,
            note: 0,
            note_shift: 0,
            quantize_ratio: 0.0,
            quantize_count: 0,

            setting_process_mode: [PROCESS_NORMAL; 2],
            setting_envelope_exp: zeros.clone(),
            setting_envelope_voice: zeros.clone(),
            setting_envelope_note: zeros.clone(),
            setting_envelope_pitch: zeros.clone(),
            setting_envelope_filter: zeros.clone(),
            setting_exp_offset: [false; 2],
            setting_pns_or: [false; 2],
            setting_counter_exp: [1; 2],
            setting_counter_voice: [1; 2],
            setting_counter_note: [1; 2],
            setting_counter_pitch: [1; 2],
            setting_counter_filter: [1; 2],
            table_envelope_mod_amp: [None, None],
            table_envelope_mod_pitch: [None, None],
            setting_sweep_step: [0; 2],
            setting_sweep_end: [0; 2],
            envelope_interval: 0,

            envelope_exp: EnvCursor::null(),
            envelope_voice: EnvCursor::null(),
            envelope_note: EnvCursor::null(),
            envelope_pitch: EnvCursor::null(),
            envelope_filter: EnvCursor::null(),
            counter_exp: 0,
            max_counter_exp: 0,
            counter_voice: 0,
            max_counter_voice: 0,
            counter_note: 0,
            max_counter_note: 0,
            counter_pitch: 0,
            max_counter_pitch: 0,
            counter_filter: 0,
            max_counter_filter: 0,
            envelope_mod_amp: None,
            envelope_mod_pitch: None,
            sweep_step: 0,
            sweep_end: 0,
            sweep_pitch: 0,
            envelope_exp_offset: 0,
            envelope_pitch_active: false,
            residue: 0,

            event_mask: NO_MASK,
            callback_before_note_on: None,
            callback_before_note_off: None,
            callback_update_register: None,
            event_trigger_on: None,
            event_trigger_off: None,
            event_trigger_id: -1,
            event_trigger_type_on: NO_EVENTS,
            event_trigger_type_off: NO_EVENTS,

            key_on_counter: 0,
            key_on_length: 0,
            key_on_delay: 0,
            flag_no_key_on: false,
        }
    }
}

impl Default for SiMMLTrack {
    fn default() -> Self {
        SiMMLTrack::new()
    }
}
