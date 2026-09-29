//! Port of `libSiON-cpp/src/chip/channels/siopm_channel_base.{h,cpp}`.
//!
//! Design (C++ ABC → Rust):
//! - [`ChannelBase`] — struct holding ALL shared state with the C++ base
//!   `.cpp`/.h-implemented bodies as inherent methods. Concrete channels
//!   (wave-6b) embed it as their first field.
//! - [`ChannelBaseTrait`] — the vtable: every C++ `virtual` becomes a trait
//!   method whose default body is the C++ base implementation (empty `{}` in
//!   the header where the base body is empty; the `.cpp` body otherwise).
//!   `void (SiOPMChannelBase::*)(int)` `_process_function` → [`process`]
//!   (base default = `_no_process`, matching the base ctor lambda).
//! - [`BaseChannel`] — C++ instantiable base stand-in: used for the pool
//!   terminator and as the shape concrete channels copy.
//!
//! C++ `_sound_chip` raw pointers become `&mut dyn ChipContext` arguments;
//! `SiOPMStream*` becomes `Rc<RefCell<dyn OutputStream>>`; pipe pointers
//! become shared [`PipeRc`] handles (list-global cursor, exactly like
//! `SinglyLinkedList<int>*`). The `_next`/`_prev` pool links are not state
//! of the channel in Rust — pool ordering lives in
//! [`manager`](super::manager) (VecDeque-based faithful reimplementation of
//! the circular free/used list).
//!
//! Virtuals the C++ base does not declare (the wave plan mentioned
//! `release` / `process_fm_feedback` / `get_eg_type` / `set_pitch_bend` /
//! `set_tempo`) do NOT exist in this C++ revision and are intentionally not
//! invented here; FM-only virtuals (`_set_lfo_state`) are added with the
//! concrete channels in wave-6b.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use super::{manager::ChannelType, ChipContext, OutputStream, PipeRc};
use crate::chip::params::channel_params::{ChannelParams, STREAM_SEND_SIZE};
use crate::chip::ref_table::{self, instance, SiopmRefTable};

/// C++ `SiOPMChannelBase::OutputMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputMode {
    /// Standard output.
    Standard = 0,
    /// Overwrite output pipe.
    Overwrite = 1,
    /// Add to output pipe.
    Add = 2,
}

/// C++ `SiOPMChannelBase::InputMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputMode {
    /// No input from pipe.
    Zero = 0,
    /// Input from pipe.
    Pipe = 1,
    /// Input from feedback.
    Feedback = 2,
}

/// C++ `SiOPMChannelBase::FilterType` (stored as `int` like the C++ field).
pub const FILTER_LP: i32 = 0; // Low pass.
pub const FILTER_BP: i32 = 1; // Band pass.
pub const FILTER_HP: i32 = 2; // High pass.

/// C++ private `SiOPMChannelBase::FilterState` (LP filter envelope states).
const FILTER_EG_ATTACK: usize = 0;
const FILTER_EG_DECAY1: usize = 1;
const FILTER_EG_DECAY2: usize = 2;
const FILTER_EG_SUSTAIN: usize = 3;
const FILTER_EG_RELEASE: usize = 4;
const FILTER_EG_OFF: usize = 5;

/// C++ `SiOPMRefTable::LFO_WAVE_MAX` re-export (made pub on the ref table in
/// this wave).
pub const LFO_WAVE_MAX: i32 = ref_table::LFO_WAVE_MAX as i32;

/// Shared state of every SiOPM channel — C++ `SiOPMChannelBase` members.
/// Public fields mirror the C++ `protected` access concrete channels use.
pub struct ChannelBase {
    table: Rc<RefCell<SiopmRefTable>>,

    pub is_free: bool,
    pub channel_type: ChannelType,

    pub is_note_on: bool,

    // Pipe buffer.

    pub buffer_index: i32,
    pub input_level: i32,
    pub ringmod_level: f64,
    pub input_mode: InputMode,
    pub output_mode: OutputMode,
    pub in_pipe: Option<PipeRc>,
    pub ring_pipe: Option<PipeRc>,
    pub base_pipe: Option<PipeRc>,
    pub out_pipe: Option<PipeRc>,

    // Volume and stream.

    pub streams: Vec<Option<Rc<RefCell<dyn OutputStream>>>>,
    pub volumes: Vec<f64>,
    pub is_idling: bool,
    pub pan: i32,
    pub has_effect_send: bool,
    pub mute: bool,
    pub velocity_table: [i32; SiopmRefTable::TL_TABLE_SIZE],
    pub expression_table: [i32; SiopmRefTable::TL_TABLE_SIZE],

    // Low pass filter.

    pub filter_on: bool,
    pub filter_type: i32,
    pub cutoff_frequency: i32,
    pub cutoff_offset: i32,
    pub resonance: f64,
    pub filter_variables: [f64; 3],
    pub filter_eg_residue: i32,
    pub filter_eg_step: i32,
    /// Phase shift.
    pub filter_eg_next: i32,
    /// Direction.
    pub filter_eg_cutoff_inc: i32,
    pub filter_eg_state: usize,
    /// Rate.
    pub filter_eg_time: [i32; 6],
    /// Level.
    pub filter_eg_cutoff: [i32; 6],

    // Low frequency oscillator (LFO).

    pub frequency_ratio: i32,
    /// Treated as a boolean flag.
    pub lfo_on: i32,
    pub lfo_timer: i32,
    pub lfo_timer_step: i32,
    pub lfo_timer_step_buffer: i32,
    pub lfo_phase: i32,
    pub lfo_wave_table: Vec<i32>,
    pub lfo_wave_shape: i32,
}

impl ChannelBase {
    /// C++ `SiOPMChannelBase(SiOPMSoundChip*)` (chip pointer → context
    /// arguments; the base ctor's `_process_function = _no_process` lambda
    /// is the trait default [`ChannelBaseTrait::process`]).
    pub fn new() -> Self {
        ChannelBase {
            table: instance(),
            is_free: true,
            channel_type: ChannelType::Max,
            is_note_on: false,
            buffer_index: 0,
            input_level: 0,
            ringmod_level: 0.0,
            input_mode: InputMode::Zero,
            output_mode: OutputMode::Standard,
            in_pipe: None,
            ring_pipe: None,
            base_pipe: None,
            out_pipe: None,
            streams: vec![None; STREAM_SEND_SIZE],
            volumes: vec![0.0; STREAM_SEND_SIZE],
            is_idling: true,
            pan: 64,
            has_effect_send: false,
            mute: false,
            velocity_table: [0; SiopmRefTable::TL_TABLE_SIZE],
            expression_table: [0; SiopmRefTable::TL_TABLE_SIZE],
            filter_on: false,
            filter_type: FILTER_LP,
            cutoff_frequency: 0,
            cutoff_offset: 0,
            resonance: 0.0,
            filter_variables: [0.0; 3],
            filter_eg_residue: 0,
            filter_eg_step: 0,
            filter_eg_next: 0,
            filter_eg_cutoff_inc: 0,
            filter_eg_state: 0,
            filter_eg_time: [0; 6],
            filter_eg_cutoff: [0; 6],
            frequency_ratio: 0,
            lfo_on: 0,
            lfo_timer: 0,
            lfo_timer_step: 0,
            lfo_timer_step_buffer: 0,
            lfo_phase: 0,
            lfo_wave_table: Vec::new(),
            lfo_wave_shape: 0,
        }
    }

    /// C++ `get_channel_type()`.
    pub fn get_channel_type(&self) -> ChannelType {
        self.channel_type
    }

    /// Shared ref-table handle (C++ `_table`, `protected`, used directly by
    /// the concrete channels for `log_table`, `filter_cutoff_table`,
    /// `pitch_wave_length`, `lfo_timer_steps`, `get_wave_table`,
    /// `get_pcm_data`, `sampler_tables`).
    pub(crate) fn table(&self) -> &Rc<RefCell<SiopmRefTable>> {
        &self.table
    }

    /// C++ `get_master_volume()` (the C++ `double * 128` int-conversion
    /// truncates toward zero like `as i32`).
    pub fn get_master_volume(&self) -> i32 {
        (self.volumes[0] * 128.0) as i32
    }

    /// C++ `set_master_volume(int)`.
    pub fn set_master_volume(&mut self, p_value: i32) {
        let value = crate::math::clampi(p_value, 0, 128);
        self.volumes[0] = value as f64 * 0.0078125; // 0.0078125 = 1/128
    }

    /// C++ `get_pan()` (external value is the -64..64 range).
    pub fn get_pan(&self) -> i32 {
        self.pan - 64
    }

    /// C++ `set_pan(int)`.
    pub fn set_pan(&mut self, p_value: i32) {
        self.pan = crate::math::clampi(p_value, -64, 64) + 64;
    }

    /// C++ `set_filter_type(int)`.
    pub fn set_filter_type(&mut self, p_type: i32) {
        self.filter_type = if p_type < 0 || p_type > 2 { 0 } else { p_type };
    }

    /// C++ `set_all_stream_send_levels(std::vector<int>)`.
    pub fn set_all_stream_send_levels(&mut self, p_levels: Vec<i32>) {
        for i in 0..STREAM_SEND_SIZE {
            let value = p_levels[i];
            self.volumes[i] = if value != i32::MIN {
                value as f64 * 0.0078125
            } else {
                0.0
            };
        }

        self.has_effect_send = false;
        for i in 1..STREAM_SEND_SIZE {
            if self.volumes[i] > 0.0 {
                self.has_effect_send = true;
            }
        }
    }

    /// C++ `set_stream_buffer(int p_stream_num, SiOPMStream *p_stream = nullptr)`.
    pub fn set_stream_buffer(
        &mut self,
        p_stream_num: usize,
        p_stream: Option<Rc<RefCell<dyn OutputStream>>>,
    ) {
        self.streams[p_stream_num] = p_stream;
    }

    /// C++ `set_stream_send(int, double)`.
    pub fn set_stream_send(&mut self, p_stream_num: usize, p_volume: f64) {
        self.volumes[p_stream_num] = p_volume;
        if p_stream_num == 0 {
            return;
        }

        if p_volume > 0.0 {
            self.has_effect_send = true;
        } else {
            self.has_effect_send = false;
            for i in 1..STREAM_SEND_SIZE {
                if self.volumes[i] > 0.0 {
                    self.has_effect_send = true;
                }
            }
        }
    }

    /// C++ `get_stream_send(int)`.
    pub fn get_stream_send(&mut self, p_stream_num: usize) -> f64 {
        self.volumes[p_stream_num]
    }

    /// C++ `initialize_lfo(int, std::vector<int>)`. The C++ acceptance test
    /// `p_waveform <= LFO_WAVE_MAX` overruns `lfo_wave_tables` when
    /// `p_waveform == LFO_WAVE_MAX` exactly (UB read of the adjacent row);
    /// the port keeps the accepted-shape quirk (`lfo_wave_shape == 8`) but
    /// clamps the table copy to the last valid row.
    pub fn initialize_lfo(&mut self, p_waveform: i32, p_custom_wave_table: Vec<i32>) {
        if p_waveform == -1 && p_custom_wave_table.len() == SiopmRefTable::LFO_TABLE_SIZE {
            self.lfo_wave_shape = -1;
            self.lfo_wave_table = p_custom_wave_table;
        } else {
            self.lfo_wave_shape = if p_waveform >= 0 && p_waveform <= LFO_WAVE_MAX {
                p_waveform
            } else {
                ref_table::LFO_WAVE_TRIANGLE as i32
            };
            let shape = std::cmp::min(self.lfo_wave_shape, ref_table::LFO_WAVE_MAX as i32 - 1) as usize;
            self.lfo_wave_table = self.table.borrow().lfo_wave_tables[shape].to_vec();
        }

        self.lfo_timer = 1;
        self.lfo_timer_step = 0;
        self.lfo_timer_step_buffer = 0;
        self.lfo_phase = 0;
    }

    /// C++ `set_lfo_cycle_time(double p_ms)`.
    pub fn set_lfo_cycle_time(&mut self, p_ms: f64) {
        self.lfo_timer = 0;
        // 0.17294117647058824 = 44100/(1000*255)
        let step = (SiopmRefTable::LFO_TIMER_INITIAL as f64 / (p_ms * 0.17294117647058824)) as i32;
        let pitch_shift = self.table.borrow().sample_rate_pitch_shift;
        self.lfo_timer_step = step << pitch_shift;
        self.lfo_timer_step_buffer = self.lfo_timer_step;
    }

    /// C++ `set_sv_filter(...)` (defaults are the C++ header defaults;
    /// [`set_sv_filter_default`] applies them).
    pub fn set_sv_filter(
        &mut self,
        p_cutoff: i32,
        p_resonance: i32,
        p_attack_rate: i32,
        p_decay_rate1: i32,
        p_decay_rate2: i32,
        p_release_rate: i32,
        p_decay_cutoff1: i32,
        p_decay_cutoff2: i32,
        p_sustain_cutoff: i32,
        p_release_cutoff: i32,
    ) {
        self.filter_eg_cutoff[FILTER_EG_ATTACK] = crate::math::clampi(p_cutoff, 0, 128);
        self.filter_eg_cutoff[FILTER_EG_DECAY1] = crate::math::clampi(p_decay_cutoff1, 0, 128);
        self.filter_eg_cutoff[FILTER_EG_DECAY2] = crate::math::clampi(p_decay_cutoff2, 0, 128);
        self.filter_eg_cutoff[FILTER_EG_SUSTAIN] = crate::math::clampi(p_sustain_cutoff, 0, 128);
        self.filter_eg_cutoff[FILTER_EG_RELEASE] = 0;
        self.filter_eg_cutoff[FILTER_EG_OFF] = crate::math::clampi(p_release_cutoff, 0, 128);

        let rates = {
            let borrow = self.table.borrow();
            [
                borrow.filter_eg_rate[(p_attack_rate & 63) as usize],
                borrow.filter_eg_rate[(p_decay_rate1 & 63) as usize],
                borrow.filter_eg_rate[(p_decay_rate2 & 63) as usize],
                borrow.filter_eg_rate[(p_release_rate & 63) as usize],
            ]
        };
        self.filter_eg_time[FILTER_EG_ATTACK] = rates[0];
        self.filter_eg_time[FILTER_EG_DECAY1] = rates[1];
        self.filter_eg_time[FILTER_EG_DECAY2] = rates[2];
        self.filter_eg_time[FILTER_EG_SUSTAIN] = i32::MAX;
        self.filter_eg_time[FILTER_EG_RELEASE] = rates[3];
        self.filter_eg_time[FILTER_EG_OFF] = i32::MAX;

        self.resonance =
            (1 << (9 - crate::math::clampi(p_resonance, 0, 9))) as f64 * 0.001953125; // 1/512
        self.filter_on =
            p_cutoff < 128 || p_resonance > 0 || p_attack_rate > 0 || p_release_rate > 0;
    }

    /// C++ `set_sv_filter()` with every header default
    /// (128, 0, 0, 0, 0, 0, 128, 128, 128, 128).
    pub fn set_sv_filter_default(&mut self) {
        self.set_sv_filter(128, 0, 0, 0, 0, 0, 128, 128, 128, 128);
    }

    /// C++ `offset_filter(int)`.
    pub fn offset_filter(&mut self, p_offset: i32) {
        self.cutoff_offset = p_offset - 128;
    }

    /// C++ `set_input(int p_level, int p_pipe_index)`.
    pub fn set_input(&mut self, p_level: i32, p_pipe_index: i32, ctx: &mut dyn ChipContext) {
        if p_level > 0 {
            self.in_pipe = ctx.get_pipe(p_pipe_index & 3, self.buffer_index);
            self.input_mode = InputMode::Pipe;
            self.input_level = p_level + 10;
        } else {
            self.in_pipe = Some(ctx.get_zero_buffer());
            self.input_mode = InputMode::Zero;
            self.input_level = 0;
        }
    }

    /// C++ `set_ring_modulation(int p_level, int p_pipe_index)`.
    pub fn set_ring_modulation(
        &mut self,
        p_level: i32,
        p_pipe_index: i32,
        ctx: &mut dyn ChipContext,
    ) {
        self.ringmod_level =
            p_level as f64 * 4.0 / ((1 << SiopmRefTable::LOG_VOLUME_BITS) as f64);
        self.ring_pipe = if p_level > 0 {
            ctx.get_pipe(p_pipe_index & 3, self.buffer_index)
        } else {
            None
        };
    }

    /// C++ `set_output(OutputMode, int p_pipe_index)`.
    pub fn set_output(
        &mut self,
        p_output_mode: OutputMode,
        p_pipe_index: i32,
        ctx: &mut dyn ChipContext,
    ) {
        let mut pipe_index = p_pipe_index & 3;
        if p_output_mode == OutputMode::Standard {
            pipe_index = 4; // pipe[4] is used.
        }

        self.output_mode = p_output_mode;
        self.out_pipe = ctx.get_pipe(pipe_index, self.buffer_index);
        self.base_pipe = if p_output_mode == OutputMode::Add {
            self.out_pipe.clone()
        } else {
            Some(ctx.get_zero_buffer())
        };
    }

    /// C++ `set_volume_tables(int (&)[TL_TABLE_SIZE], int (&)[TL_TABLE_SIZE])`
    /// (`COPY_TL_TABLE`).
    pub fn set_volume_tables(
        &mut self,
        p_velocity_table: &[i32; SiopmRefTable::TL_TABLE_SIZE],
        p_expression_table: &[i32; SiopmRefTable::TL_TABLE_SIZE],
    ) {
        self.velocity_table.copy_from_slice(p_velocity_table);
        self.expression_table.copy_from_slice(p_expression_table);
    }

    // Processing (base internals: `_reset_sv_filter_state`,
    // `_try_shift_sv_filter_state`, `_shift_sv_filter_state`, `_no_process`,
    // `_apply_ring_modulation`, `_apply_sv_filter`).

    pub(crate) fn reset_sv_filter_state(&mut self) {
        self.cutoff_frequency = self.filter_eg_cutoff[FILTER_EG_ATTACK];
    }

    pub(crate) fn try_shift_sv_filter_state(&mut self, p_state: usize) -> bool {
        if self.filter_eg_time[p_state] == 0 {
            return false;
        }

        self.filter_eg_state = p_state;
        self.filter_eg_step = self.filter_eg_time[p_state];
        self.filter_eg_next = self.filter_eg_cutoff[p_state + 1];
        self.filter_eg_cutoff_inc = if self.cutoff_frequency < self.filter_eg_next {
            1
        } else {
            -1
        };
        self.cutoff_frequency != self.filter_eg_next
    }

    pub(crate) fn shift_sv_filter_state(&mut self, p_state: usize) {
        let mut state = p_state;

        loop {
            match state {
                FILTER_EG_ATTACK | FILTER_EG_DECAY1 | FILTER_EG_DECAY2 => {
                    if self.try_shift_sv_filter_state(state) {
                        break;
                    }
                    // [[fallthrough]] -> next state
                    state += 1;
                }
                FILTER_EG_SUSTAIN => {
                    // Catch all.
                    self.filter_eg_state = FILTER_EG_SUSTAIN;
                    self.filter_eg_step = i32::MAX;
                    self.filter_eg_next = self.cutoff_frequency + 1;
                    self.filter_eg_cutoff_inc = 0;
                    break;
                }
                FILTER_EG_RELEASE => {
                    if self.try_shift_sv_filter_state(state) {
                        break;
                    }
                    // [[fallthrough]] -> EG_OFF
                    state += 1;
                }
                _ => {
                    // Catch all (EG_OFF / default).
                    self.filter_eg_state = FILTER_EG_OFF;
                    self.filter_eg_step = i32::MAX;
                    self.filter_eg_next = self.cutoff_frequency + 1;
                    self.filter_eg_cutoff_inc = 0;
                    break;
                }
            }
        }

        self.filter_eg_residue = self.filter_eg_step;
    }

    /// C++ `_no_process(int p_length)` — rotate the output/input/ring pipes.
    pub(crate) fn no_process(&mut self, ctx: &mut dyn ChipContext, p_length: i32) {
        // Rotate the output buffer.
        if self.output_mode == OutputMode::Standard {
            let pipe_index = (self.buffer_index + p_length) & (ctx.get_buffer_length() - 1);
            self.out_pipe = ctx.get_pipe(4, pipe_index);
        } else if let Some(out_pipe) = self.out_pipe.clone() {
            out_pipe.borrow_mut().advance(p_length);
            self.base_pipe = if self.output_mode == OutputMode::Add {
                Some(out_pipe)
            } else {
                Some(ctx.get_zero_buffer())
            };
        }

        // Rotate the input buffer when connected by @i.
        if self.input_mode == InputMode::Pipe {
            if let Some(in_pipe) = &self.in_pipe {
                in_pipe.borrow_mut().advance(p_length);
            }
        }

        // Rotate the ring buffer.
        if let Some(ring_pipe) = &self.ring_pipe {
            ring_pipe.borrow_mut().advance(p_length);
        }
    }

    /// C++ `_apply_ring_modulation(Element *p_buffer_start, int p_length)`.
    /// `p_start` is the absolute cursor index of `p_out_pipe` (C++
    /// `p_buffer_start`). When the ring pipe aliases the output pipe (same
    /// shared list, reachable via `@r` self-feed), C++ reads freshly written
    /// values through the aliasing pointers — reproduced under a single
    /// borrow in that case.
    pub(crate) fn apply_ring_modulation(
        &self,
        p_out_pipe: &PipeRc,
        p_start: usize,
        p_length: i32,
    ) {
        let ring = self.ring_pipe.as_ref().expect("ring pipe");
        let level = self.ringmod_level;

        if Rc::ptr_eq(p_out_pipe, ring) {
            let mut out = p_out_pipe.borrow_mut();
            let size = out.size();
            let mut target = p_start % size;
            let mut source = out.cursor();
            for _ in 0..p_length {
                let ring_value = out.value_at(source);
                let value = out.value_at(target);
                out.set_value_at(target, (value as f64 * (ring_value as f64 * level)) as i32);
                target = (target + 1) % size;
                source = (source + 1) % size;
            }
            out.set_cursor(source);
        } else {
            let size = p_out_pipe.borrow().size();
            let ring_size = ring.borrow().size();
            let mut target = p_start % size;
            let mut source = ring.borrow().cursor();
            for _ in 0..p_length {
                let ring_value = ring.borrow().value_at(source);
                let value = p_out_pipe.borrow().value_at(target);
                p_out_pipe
                    .borrow_mut()
                    .set_value_at(target, (value as f64 * (ring_value as f64 * level)) as i32);
                target = (target + 1) % size;
                source = (source + 1) % ring_size;
            }
            ring.borrow_mut().set_cursor(source);
        }
    }

    /// C++ `_apply_sv_filter(Element *p_buffer_start, int p_length,
    /// double (&r_variables)[3])`. `p_start` is the absolute cursor index of
    /// `p_out_pipe`. `r_variables` is the C++ reference parameter — the FM /
    /// KS path passes `&mut base.filter_variables` (callers copy the
    /// 3-element array out and back to side-step the self-aliasing the C++
    /// reference had); `SiOPMChannelPCM` passes its second stereo set.
    pub(crate) fn apply_sv_filter(
        &mut self,
        p_out_pipe: &PipeRc,
        p_start: usize,
        p_length: i32,
        r_variables: &mut [f64; 3],
    ) {
        let mut cutoff = crate::math::clampi(self.cutoff_frequency + self.cutoff_offset, 0, 128);
        let mut cutoff_value = self.table.borrow().filter_cutoff_table[cutoff as usize];
        // * _table->filter_feedback_table[out]; // This is commented out in original code.
        let mut feedback_value = self.resonance;

        // Previous setting.
        let mut step = self.filter_eg_residue;

        let size = p_out_pipe.borrow().size();
        let mut target = p_start % size;
        let mut length = p_length;
        while length >= step {
            // Process.
            for _ in 0..step {
                let mut value = p_out_pipe.borrow().value_at(target) as f64;
                value -= r_variables[0];
                value -= r_variables[1] * feedback_value;
                r_variables[2] = value;
                r_variables[1] += r_variables[2] * cutoff_value;
                r_variables[0] += r_variables[1] * cutoff_value;

                let out_value = r_variables[self.filter_type as usize] as i32;
                p_out_pipe.borrow_mut().set_value_at(target, out_value);
                target = (target + 1) % size;
            }
            length -= step;

            // Change cutoff and shift state.

            self.cutoff_frequency += self.filter_eg_cutoff_inc;
            cutoff = crate::math::clampi(self.cutoff_frequency + self.cutoff_offset, 0, 128);
            cutoff_value = self.table.borrow().filter_cutoff_table[cutoff as usize];
            feedback_value = self.resonance;

            if self.cutoff_frequency == self.filter_eg_next {
                self.shift_sv_filter_state(self.filter_eg_state + 1);
            }

            step = self.filter_eg_step;
        }

        // Process the remainder.
        for _ in 0..length {
            let mut value = p_out_pipe.borrow().value_at(target) as f64;
            value -= r_variables[0];
            value -= r_variables[1] * feedback_value;
            r_variables[2] = value;
            r_variables[1] += r_variables[2] * cutoff_value;
            r_variables[0] += r_variables[1] * cutoff_value;

            let out_value = r_variables[self.filter_type as usize] as i32;
            p_out_pipe.borrow_mut().set_value_at(target, out_value);
            target = (target + 1) % size;
        }

        // Next setting.
        self.filter_eg_residue = self.filter_eg_step - length;
    }

    /// C++ `SiOPMChannelBase::note_on()` base body.
    pub(crate) fn base_note_on(&mut self) {
        self.lfo_phase = 0; // Reset.
        if self.filter_on {
            self.reset_sv_filter_state();
            self.shift_sv_filter_state(FILTER_EG_ATTACK);
        }
        self.is_note_on = true;
    }

    /// C++ `SiOPMChannelBase::note_off()` base body.
    pub(crate) fn base_note_off(&mut self) {
        if self.filter_on {
            self.shift_sv_filter_state(FILTER_EG_RELEASE);
        }
        self.is_note_on = false;
    }

    /// C++ `SiOPMChannelBase::reset()` base body.
    pub(crate) fn base_reset(&mut self) {
        self.is_note_on = false;
        self.is_idling = true;
    }

    /// Volume/pipe-buffer half of C++ `initialize()` (everything before the
    /// virtual `initialize_lfo`/`set_input`/... calls, which the trait
    /// default performs in the C++ order). NOTE: the C++ `p_prev != this`
    /// guard is dropped — self-copy is an observation-identical no-op.
    pub(crate) fn init_volume_state(
        &mut self,
        p_prev: Option<&ChannelBase>,
        p_buffer_index: i32,
    ) {
        // Volume.
        if let Some(prev) = p_prev {
            for i in 0..STREAM_SEND_SIZE {
                self.volumes[i] = prev.volumes[i];
                self.streams[i] = prev.streams[i].clone();
            }

            self.pan = prev.pan;
            self.has_effect_send = prev.has_effect_send;
            self.mute = prev.mute;
            self.velocity_table.copy_from_slice(&prev.velocity_table);
            self.expression_table.copy_from_slice(&prev.expression_table);
        } else {
            self.volumes[0] = 0.5;
            self.streams[0] = None;
            for i in 1..STREAM_SEND_SIZE {
                self.volumes[i] = 0.0;
                self.streams[i] = None;
            }

            self.pan = 64;
            self.has_effect_send = false;
            self.mute = false;
            let linear = self.table.borrow().eg_total_level_tables[ref_table::VM_LINEAR];
            self.velocity_table.copy_from_slice(&linear);
            self.expression_table.copy_from_slice(&linear);
        }

        // Buffer index.
        self.is_note_on = false;
        self.is_idling = true;
        self.buffer_index = p_buffer_index;
    }

    /// C++ `_to_string()`.
    pub fn to_string_repr(&self) -> String {
        let mut params = String::new();

        params += &format!("feedback={}, ", self.input_level - 6);
        params += &format!("vol={:.5}, ", self.volumes[0]);
        params += &format!("pan={}", self.pan - 64);

        format!("SiOPMChannelBase: {params}")
    }
}

/// The C++ `SiOPMChannelBase` vtable. Every method mirrors one C++ `virtual`;
/// default bodies are the C++ base implementations. Concrete channels
/// (wave-6b) embed [`ChannelBase`] and override what their C++ class
/// overrides; to call the base body from an override use
/// `ChannelBaseTrait::<base>::method`-style qualified calls or the
/// `base_mut()`-forwarded inherent methods.
pub trait ChannelBaseTrait {
    /// Downcast helpers to the embedded [`ChannelBase`] (replaces direct
    /// base-class member access in C++ derived methods).
    fn base(&self) -> &ChannelBase;
    fn base_mut(&mut self) -> &mut ChannelBase;

    /// C++ `get_channel_params(const Ref<SiOPMChannelParams>&) const`.
    fn get_channel_params(&self, _r_params: &mut ChannelParams) {}

    /// C++ `set_channel_params(const Ref<SiOPMChannelParams>&, bool, bool = true)`.
    fn set_channel_params(
        &mut self,
        _p_params: &ChannelParams,
        _p_with_volume: bool,
        _p_with_modulation: bool,
        _ctx: &mut dyn ChipContext,
    ) {
    }

    /// C++ `set_wave_data(const Ref<SiOPMWaveBase>&)` — `dyn Any` stands in
    /// for the C++ upcast; concrete channels downcast
    /// (`SiopmWaveTable` / `SiopmWavePcmData` / `SiopmWaveSamplerTable` /
    /// `SiopmWaveSamplerData`, wave-6b). `ctx` replaces the C++ `_sound_chip`
    /// member reached (via `_update_operator_count` → `_set_feedback`) by
    /// `SiOPMChannelFM::set_wave_data`.
    fn set_wave_data(&mut self, _p_wave_data: &dyn Any, _ctx: &mut dyn ChipContext) {}

    /// C++ `set_channel_number(int)`.
    fn set_channel_number(&mut self, _p_value: i32) {}

    /// C++ `set_register(int p_address, int p_data)` — `ctx` replaces the
    /// C++ `_sound_chip` member reached by `SiOPMChannelFM`'s register map.
    fn set_register(&mut self, _p_address: i32, _p_data: i32, _ctx: &mut dyn ChipContext) {}

    /// C++ `set_algorithm(int p_operator_count, bool p_analog_like, int p_algorithm)`
    /// — `ctx` replaces the C++ `_sound_chip` member used by `set_pipes`.
    fn set_algorithm(
        &mut self,
        _p_operator_count: i32,
        _p_analog_like: bool,
        _p_algorithm: i32,
        _ctx: &mut dyn ChipContext,
    ) {
    }

    /// C++ `set_feedback(int p_level, int p_connection)` — `ctx` replaces
    /// the C++ `_sound_chip->get_zero_buffer()`.
    fn set_feedback(&mut self, _p_level: i32, _p_connection: i32, _ctx: &mut dyn ChipContext) {}

    /// C++ `set_parameters(std::vector<int>)`.
    fn set_parameters(&mut self, _p_params: Vec<i32>, _ctx: &mut dyn ChipContext) {}

    /// C++ `set_types(int p_pg_type, SiONPitchTableType p_pt_type)`.
    fn set_types(&mut self, _p_pg_type: i32, _p_pt_type: i32, _ctx: &mut dyn ChipContext) {}

    /// C++ `set_all_attack_rate(int)`.
    fn set_all_attack_rate(&mut self, _p_value: i32) {}

    /// C++ `set_all_release_rate(int)`.
    fn set_all_release_rate(&mut self, _p_value: i32) {}

    /// C++ `get_master_volume()` / `set_master_volume(int)`.
    fn get_master_volume(&self) -> i32 {
        self.base().get_master_volume()
    }

    /// C++ `set_master_volume(int)`.
    fn set_master_volume(&mut self, p_value: i32) {
        self.base_mut().set_master_volume(p_value);
    }

    /// C++ `get_pan()` / `set_pan(int)`.
    fn get_pan(&self) -> i32 {
        self.base().get_pan()
    }

    /// C++ `set_pan(int)`.
    fn set_pan(&mut self, p_value: i32) {
        self.base_mut().set_pan(p_value);
    }

    /// C++ `is_mute()` / `set_mute(bool)`.
    fn is_mute(&self) -> bool {
        self.base().mute
    }

    /// C++ `set_mute(bool)`.
    fn set_mute(&mut self, p_value: bool) {
        self.base_mut().mute = p_value;
    }

    /// C++ `get_pitch()` / `set_pitch(int)`.
    fn get_pitch(&self) -> i32 {
        0
    }

    /// C++ `set_pitch(int)`.
    fn set_pitch(&mut self, _p_value: i32) {}

    /// C++ `set_active_operator_index(int)`.
    fn set_active_operator_index(&mut self, _p_value: i32) {}

    /// C++ `set_release_rate(int)`.
    fn set_release_rate(&mut self, _p_value: i32) {}

    /// C++ `set_total_level(int)`.
    fn set_total_level(&mut self, _p_value: i32) {}

    /// C++ `set_fine_multiple(int)`.
    fn set_fine_multiple(&mut self, _p_value: i32) {}

    /// C++ `set_phase(int)`.
    fn set_phase(&mut self, _p_value: i32) {}

    /// C++ `set_detune(int)`.
    fn set_detune(&mut self, _p_value: i32) {}

    /// C++ `set_fixed_pitch(int)`.
    fn set_fixed_pitch(&mut self, _p_value: i32) {}

    /// C++ `set_ssg_envelope_control(int)`.
    fn set_ssg_envelope_control(&mut self, _p_value: i32) {}

    /// C++ `set_envelope_reset(bool)`.
    fn set_envelope_reset(&mut self, _p_reset: bool) {}

    /// C++ `get_buffer_index()`.
    fn get_buffer_index(&self) -> i32 {
        self.base().buffer_index
    }

    /// C++ `is_note_on()`.
    fn is_note_on(&self) -> bool {
        self.base().is_note_on
    }

    /// C++ `is_idling()`.
    fn is_idling(&self) -> bool {
        self.base().is_idling
    }

    /// C++ `is_filter_active()`.
    fn is_filter_active(&self) -> bool {
        self.base().filter_on
    }

    /// C++ `get_filter_type()`.
    fn get_filter_type(&self) -> i32 {
        self.base().filter_type
    }

    /// C++ `set_filter_type(int)`.
    fn set_filter_type(&mut self, p_type: i32) {
        self.base_mut().set_filter_type(p_type);
    }

    /// C++ `set_all_stream_send_levels(std::vector<int>)`.
    fn set_all_stream_send_levels(&mut self, p_levels: Vec<i32>) {
        self.base_mut().set_all_stream_send_levels(p_levels);
    }

    /// C++ `set_stream_buffer(int, SiOPMStream* = nullptr)`.
    fn set_stream_buffer(
        &mut self,
        p_stream_num: usize,
        p_stream: Option<Rc<RefCell<dyn OutputStream>>>,
    ) {
        self.base_mut().set_stream_buffer(p_stream_num, p_stream);
    }

    /// C++ `set_stream_send(int, double)` / `get_stream_send(int)`.
    fn set_stream_send(&mut self, p_stream_num: usize, p_volume: f64) {
        self.base_mut().set_stream_send(p_stream_num, p_volume);
    }

    /// C++ `get_stream_send(int)`.
    fn get_stream_send(&mut self, p_stream_num: usize) -> f64 {
        self.base_mut().get_stream_send(p_stream_num)
    }

    /// C++ `offset_volume(int p_expression, int p_velocity)`.
    fn offset_volume(&mut self, _p_expression: i32, _p_velocity: i32) {}

    /// C++ `set_frequency_ratio(int)`.
    fn set_frequency_ratio(&mut self, p_ratio: i32) {
        self.base_mut().frequency_ratio = p_ratio;
    }

    /// C++ `initialize_lfo(int, std::vector<int> = {})`.
    fn initialize_lfo(&mut self, p_waveform: i32, p_custom_wave_table: Vec<i32>) {
        self.base_mut().initialize_lfo(p_waveform, p_custom_wave_table);
    }

    /// C++ `set_lfo_cycle_time(double p_ms)`.
    fn set_lfo_cycle_time(&mut self, p_ms: f64) {
        self.base_mut().set_lfo_cycle_time(p_ms);
    }

    /// C++ `set_amplitude_modulation(int)`.
    fn set_amplitude_modulation(&mut self, _p_depth: i32) {}

    /// C++ `set_pitch_modulation(int)`.
    fn set_pitch_modulation(&mut self, _p_depth: i32) {}

    /// C++ `activate_filter(bool)`.
    fn activate_filter(&mut self, p_active: bool) {
        self.base_mut().filter_on = p_active;
    }

    /// C++ `set_sv_filter(int ... x10)` — C++ header defaults; call
    /// [`ChannelBase::set_sv_filter_default`] for the no-argument form.
    fn set_sv_filter(
        &mut self,
        p_cutoff: i32,
        p_resonance: i32,
        p_attack_rate: i32,
        p_decay_rate1: i32,
        p_decay_rate2: i32,
        p_release_rate: i32,
        p_decay_cutoff1: i32,
        p_decay_cutoff2: i32,
        p_sustain_cutoff: i32,
        p_release_cutoff: i32,
    ) {
        self.base_mut().set_sv_filter(
            p_cutoff,
            p_resonance,
            p_attack_rate,
            p_decay_rate1,
            p_decay_rate2,
            p_release_rate,
            p_decay_cutoff1,
            p_decay_cutoff2,
            p_sustain_cutoff,
            p_release_cutoff,
        );
    }

    /// C++ `offset_filter(int)`.
    fn offset_filter(&mut self, p_offset: i32) {
        self.base_mut().offset_filter(p_offset);
    }

    /// C++ `set_input(int p_level, int p_pipe_index)`.
    fn set_input(&mut self, p_level: i32, p_pipe_index: i32, ctx: &mut dyn ChipContext) {
        self.base_mut().set_input(p_level, p_pipe_index, ctx);
    }

    /// C++ `set_ring_modulation(int p_level, int p_pipe_index)`.
    fn set_ring_modulation(&mut self, p_level: i32, p_pipe_index: i32, ctx: &mut dyn ChipContext) {
        self.base_mut().set_ring_modulation(p_level, p_pipe_index, ctx);
    }

    /// C++ `set_output(OutputMode, int p_pipe_index)`.
    fn set_output(
        &mut self,
        p_output_mode: OutputMode,
        p_pipe_index: i32,
        ctx: &mut dyn ChipContext,
    ) {
        self.base_mut().set_output(p_output_mode, p_pipe_index, ctx);
    }

    /// C++ `set_volume_tables(int (&)[], int (&)[])`.
    fn set_volume_tables(
        &mut self,
        p_velocity_table: &[i32; SiopmRefTable::TL_TABLE_SIZE],
        p_expression_table: &[i32; SiopmRefTable::TL_TABLE_SIZE],
    ) {
        self.base_mut()
            .set_volume_tables(p_velocity_table, p_expression_table);
    }

    /// C++ `note_on()`.
    fn note_on(&mut self) {
        self.base_mut().base_note_on();
    }

    /// C++ `note_off()`.
    fn note_off(&mut self) {
        self.base_mut().base_note_off();
    }

    /// C++ `reset_channel_buffer_status()`.
    fn reset_channel_buffer_status(&mut self) {
        self.base_mut().buffer_index = 0;
    }

    /// C++ `_process_function` target — the per-block sample generator.
    /// Base default = `_no_process` (the base ctor lambda).
    fn process(&mut self, p_length: i32, ctx: &mut dyn ChipContext) {
        self.base_mut().no_process(ctx, p_length);
    }

    /// C++ `buffer(int p_length)` — vtable entry; defaults to
    /// [`ChannelBaseTrait::buffer_base`] (`SiOPMChannelFM` dispatches
    /// through the thunk, `SiOPMChannelKS` / `SiOPMChannelPCM` /
    /// `SiOPMChannelSampler` replace it outright).
    fn buffer(&mut self, p_length: i32, ctx: &mut dyn ChipContext) {
        self.buffer_base(p_length, ctx);
    }

    /// C++ `SiOPMChannelBase::buffer(int p_length)` body — the thunk derived
    /// channels call via `ChannelBaseTrait::buffer_base(self, ...)` after
    /// their own dispatch (a direct `buffer` call from their override would
    /// re-enter the override, which the C++ vtable never did).
    fn buffer_base(&mut self, p_length: i32, ctx: &mut dyn ChipContext) {
        if self.base().is_idling {
            self.buffer_no_process(p_length, ctx);
            return;
        }

        // Preserve the start of the output pipe.
        let out_pipe = self.base().out_pipe.clone().expect("out_pipe");
        let start = out_pipe.borrow().cursor();

        // Update the output pipe for the provided length.
        self.process(p_length, ctx);

        if self.base().ring_pipe.is_some() {
            self.base().apply_ring_modulation(&out_pipe, start, p_length);
        }
        if self.base().filter_on {
            let base = self.base_mut();
            let mut variables = base.filter_variables;
            base.apply_sv_filter(&out_pipe, start, p_length, &mut variables);
            base.filter_variables = variables;
        }

        if self.base().output_mode == OutputMode::Standard && !self.base().mute {
            let buffer_index = self.base().buffer_index;
            let pan = self.base().pan;
            let volumes = self.base().volumes.clone();
            let streams = self.base().streams.clone();

            if self.base().has_effect_send {
                for i in 0..STREAM_SEND_SIZE {
                    if volumes[i] > 0.0 {
                        let stream = match &streams[i] {
                            Some(stream) => Some(stream.clone()),
                            None => ctx.get_stream_slot(i),
                        };
                        if let Some(stream) = stream {
                            stream.borrow_mut().write(
                                &out_pipe.borrow(),
                                start,
                                buffer_index,
                                p_length,
                                volumes[i],
                                pan,
                            );
                        }
                    }
                }
            } else {
                // C++ writes unconditionally through the (chip-guaranteed
                // non-null) output stream; a missing stream is a port-level
                // hard error, reported and skipped.
                let stream = match &streams[0] {
                    Some(stream) => Some(stream.clone()),
                    None => ctx.get_output_stream(),
                };
                match stream {
                    Some(stream) => stream.borrow_mut().write(
                        &out_pipe.borrow(),
                        start,
                        buffer_index,
                        p_length,
                        volumes[0],
                        pan,
                    ),
                    None => crate::error::err_print_body("Parameter \"stream\" is null.", false),
                }
            }
        }

        self.base_mut().buffer_index += p_length;
    }

    /// C++ `buffer_no_process(int p_length)`.
    fn buffer_no_process(&mut self, p_length: i32, ctx: &mut dyn ChipContext) {
        let base = self.base_mut();
        base.no_process(ctx, p_length);
        base.buffer_index += p_length;
    }

    /// C++ `initialize(SiOPMChannelBase *p_prev, int p_buffer_index)`.
    /// The shared base body (volume half + virtual tail). Concrete channel
    /// overrides of [`ChannelBaseTrait::initialize`] run their own part and
    /// then call this via the default thunk — exactly how the C++ derived
    /// `initialize()` calls `SiOPMChannelBase::initialize()` with the
    /// virtual tail still dispatching to the concrete class.
    fn initialize_base(
        &mut self,
        p_prev: Option<&dyn ChannelBaseTrait>,
        p_buffer_index: i32,
        ctx: &mut dyn ChipContext,
    ) {
        let prev_base = p_prev.map(|prev| prev.base());
        self.base_mut().init_volume_state(prev_base, p_buffer_index);

        // LFO.
        self.initialize_lfo(ref_table::LFO_WAVE_TRIANGLE as i32, Vec::new());
        self.set_lfo_cycle_time(333.0);
        self.set_frequency_ratio(100);

        // Connection.
        self.set_input(0, 0, ctx);
        self.set_ring_modulation(0, 0, ctx);
        self.set_output(OutputMode::Standard, 0, ctx);

        // LP filter.
        {
            let base = self.base_mut();
            base.filter_variables = [0.0; 3];
            base.cutoff_offset = 0;
            base.filter_type = FILTER_LP;
        }

        self.set_sv_filter(128, 0, 0, 0, 0, 0, 128, 128, 128, 128);
        self.base_mut().shift_sv_filter_state(FILTER_EG_OFF);
    }

    /// C++ `initialize(SiOPMChannelBase *p_prev, int p_buffer_index)` —
    /// base-class vtable entry; defaults to [`ChannelBaseTrait::initialize_base`].
    fn initialize(
        &mut self,
        p_prev: Option<&dyn ChannelBaseTrait>,
        p_buffer_index: i32,
        ctx: &mut dyn ChipContext,
    ) {
        self.initialize_base(p_prev, p_buffer_index, ctx);
    }

    /// C++ `initialize(p_prev, p_buffer_index)` where `p_prev` aliases
    /// `self` — the re-init path of
    /// `SiMMLChannelSettings::initialize_tone`
    /// (`channel->initialize(channel, buffer_index)`,
    /// `simml_channel_settings.cpp:37`). The C++ base body assigns every
    /// volume-state field from the aliased pointer (a self-copy no-op), so
    /// the port snapshots that state and restores it around
    /// `initialize(None, ..)`, which reproduces the observable behavior
    /// (self-copy + the `is_note_on = false; is_idling = true;
    /// buffer_index` tail) with defined Rust borrows.
    fn initialize_self(&mut self, p_buffer_index: i32, ctx: &mut dyn ChipContext) {
        let base = self.base_mut();
        let volumes = base.volumes.clone();
        let streams = base.streams.clone();
        let pan = base.pan;
        let has_effect_send = base.has_effect_send;
        let mute = base.mute;
        let velocity_table = base.velocity_table;
        let expression_table = base.expression_table;

        self.initialize(None, p_buffer_index, ctx);

        let base = self.base_mut();
        base.volumes = volumes;
        base.streams = streams;
        base.pan = pan;
        base.has_effect_send = has_effect_send;
        base.mute = mute;
        base.velocity_table = velocity_table;
        base.expression_table = expression_table;
    }

    /// C++ `reset()`.
    fn reset(&mut self) {
        self.base_mut().base_reset();
    }
}

/// C++ `SiOPMChannelBase` itself as an instantiable channel: the pool
/// terminator (`_terminator = new SiOPMChannelBase(chip)`) and the base
/// stand-in concrete channels embed alongside.
pub struct BaseChannel {
    base: ChannelBase,
}

impl BaseChannel {
    /// C++ `SiOPMChannelBase(SiOPMSoundChip*)` — see
    /// [`ChannelManager::create_channel`][super::manager::create_channel]
    /// for the terminator construction site.
    pub fn new() -> Self {
        BaseChannel {
            base: ChannelBase::new(),
        }
    }
}

impl ChannelBaseTrait for BaseChannel {
    fn base(&self) -> &ChannelBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut ChannelBase {
        &mut self.base
    }
}

