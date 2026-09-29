//! Port of `libSiON-cpp/src/chip/channels/siopm_operator.{h,cpp}`.
//!
//! `SiOPMOperator` → [`Operator`]: FM operator with the OPM-derived envelope
//! generator (`_shift_eg_state` / `tick_eg`), the phase generator
//! (`_update_pitch` / `_update_phase_step` / `tick_pulse_generator`) and the
//! key-scale / total-level math. All frozen bit shifts, table order and
//! evaluation order are reproduced verbatim; two's-complement wrap is applied
//! only where C++ `int` overflow is reachable (phase accumulation and the
//! `_phase_step *= _fine_multiple` product).
//!
//! The C++ `_sound_chip` raw pointer is replaced by a `&mut dyn
//! [`crate::chip::channels::ChipContext`]` argument on `initialize` and
//! `set_pipes` (wave-6b seam; the C++ ctor only stored the pointer).

use std::cell::RefCell;
use std::rc::Rc;

use super::{ChipContext, Pipe, PipeRc};
use crate::chip::params::operator_params::{self as op_params, OperatorParams};
use crate::chip::ref_table::{instance, SiopmRefTable};
use crate::chip::wave::pcm_data::SiopmWavePcmData;
use crate::chip::wave::table::SiopmWaveTable;
use crate::random::RandomNumberGenerator;
use crate::sion_enums::{PITCH_TABLE_PCM, PULSE_USER_CUSTOM, PULSE_USER_PCM};

/// C++ `SiOPMOperator::EGState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EgState {
    Attack = 0,
    Decay = 1,
    Sustain = 2,
    Release = 3,
    Off = 4,
}

/// C++ `SiOPMOperator::_eg_next_state_table[2][EG_MAX]`.
const EG_NEXT_STATE_TABLE: [[EgState; 5]; 2] = [
    // EG_ATTACK, EG_DECAY, EG_SUSTAIN, EG_RELEASE, EG_OFF
    [EgState::Decay, EgState::Sustain, EgState::Off, EgState::Off, EgState::Off], // normal
    [EgState::Decay, EgState::Sustain, EgState::Attack, EgState::Off, EgState::Off], // ssgev
];

/// C++ `SiOPMOperator`.
pub struct Operator {
    table: Rc<RefCell<SiopmRefTable>>,

    // FM module parameters.

    pub attack_rate: i32,
    pub decay_rate: i32,
    pub sustain_rate: i32,
    pub release_rate: i32,
    pub sustain_level: i32,
    pub total_level: i32,
    /// Key scaling rate = 5-ks [5,2].
    pub key_scaling_rate: i32,
    pub key_scaling_level: i32,
    /// Fine multiple [64,128,256,384,512...].
    pub fine_multiple: i32,
    /// Raw detune1 [0,7].
    pub detune1: i32,
    /// Raw detune2 [0,3].
    pub detune2: i32,
    /// Amp modulation shift [16,0].
    pub amplitude_modulation_shift: i32,
    /// Key code = oct << 4 + note [0,127].
    pub key_code: i32,

    /// Mute [0 / ENV_BOTTOM].
    pub mute: i32,
    /// SSG-type envelope control [0,17].
    pub ssg_type: i32,
    pub envelope_reset_on_attack: bool,

    // Pulse generator.

    pub pg_type: i32,
    pub pt_type: i32,
    pub wave_table: Vec<i32>,
    /// Phase shift (wave-table fixed bits).
    pub wave_fixed_bits: i32,
    /// Phase step shift.
    pub wave_phase_step_shift: i32,
    pub pitch_table: Vec<i32>,
    pub pitch_table_filter: i32,

    pub phase: i32,
    pub phase_step: i32,
    /// -1 means random, -2 means no phase reset.
    pub key_on_phase: i32,
    pub pitch_fixed: bool,

    /// Pitch index = note * 64 + key fraction.
    pub pitch_index: i32,
    /// Detune for pTSS. 1 halftone divides into 64 steps.
    pub pitch_index_shift: i32,
    /// Detune for pitch modulation.
    pub pitch_index_shift2: i32,
    /// Frequency modulation left-shift. 15 for FM, fb+6 for feedback.
    pub fm_shift: i32,

    // Envelope generator.

    pub eg_state: EgState,
    pub eg_timer: i32,
    /// Timer stepping by samples.
    pub eg_timer_step: i32,
    /// Counter rounded on 8.
    pub eg_counter: i32,
    /// Internal sustain level [0, ENV_BOTTOM].
    pub eg_sustain_level: i32,
    /// Internal total level (table index + 192).
    pub eg_total_level: i32,
    /// Internal total level offset by volume [-192,832].
    pub eg_tl_offset: i32,
    /// Internal key scaling rate = _kc >> _ks [0,32].
    pub eg_key_scale_rate: i32,
    /// Internal key scaling level right shift.
    pub eg_key_scale_level_rshift: i32,
    /// Envelope generator level [0,1024].
    pub eg_level: i32,
    /// Envelope generator output [0,1024<<3].
    pub eg_output: i32,
    /// SSG envelope control attack rate switch.
    pub eg_ssgec_attack_rate: i32,
    /// SSG envelope control state.
    pub eg_ssgec_state: i32,

    pub eg_increment_table: [i32; 8],
    pub eg_state_shift_level: i32,
    pub eg_state_table_index: usize,
    pub eg_level_table: [i32; 1024],

    // PCM wave.

    pub pcm_channel_num: i32,
    pub pcm_start_point: i32,
    pub pcm_end_point: i32,
    pub pcm_loop_point: i32,

    // Pipes.

    pub is_final: bool,
    pub in_pipe: Option<PipeRc>,
    pub base_pipe: Option<PipeRc>,
    pub out_pipe: Option<PipeRc>,
    pub feed_pipe: PipeRc,
}

impl Operator {
    pub const PCM_WAVE_FIXED_BITS: i32 = 11;

    /// C++ `SiOPMOperator(SiOPMSoundChip*)` (chip pointer → `ChipContext`
    /// arguments on [`initialize`] / [`set_pipes`]).
    pub fn new() -> Self {
        let table = instance();
        let (eg_increment_table, eg_level_table) = {
            let borrow = table.borrow();
            (
                borrow.eg_increment_tables[17],
                borrow.eg_level_tables[0],
            )
        };

        Operator {
            table,
            attack_rate: 0,
            decay_rate: 0,
            sustain_rate: 0,
            release_rate: 0,
            sustain_level: 0,
            total_level: 0,
            key_scaling_rate: 0,
            key_scaling_level: 0,
            fine_multiple: 0,
            detune1: 0,
            detune2: 0,
            amplitude_modulation_shift: 0,
            key_code: 0,
            mute: 0,
            ssg_type: 0,
            envelope_reset_on_attack: false,
            pg_type: crate::sion_enums::PULSE_SINE,
            pt_type: crate::sion_enums::PITCH_TABLE_OPM,
            wave_table: Vec::new(),
            wave_fixed_bits: 0,
            wave_phase_step_shift: 0,
            pitch_table: Vec::new(),
            pitch_table_filter: 0,
            phase: 0,
            phase_step: 0,
            key_on_phase: 0,
            pitch_fixed: false,
            pitch_index: 0,
            pitch_index_shift: 0,
            pitch_index_shift2: 0,
            fm_shift: 0,
            eg_state: EgState::Off,
            eg_timer: 0,
            eg_timer_step: 0,
            eg_counter: 0,
            eg_sustain_level: 0,
            eg_total_level: 0,
            eg_tl_offset: 0,
            eg_key_scale_rate: 0,
            eg_key_scale_level_rshift: 0,
            eg_level: 0,
            eg_output: 0,
            eg_ssgec_attack_rate: 0,
            eg_ssgec_state: 0,
            eg_increment_table,
            eg_state_shift_level: 0,
            eg_state_table_index: 0,
            eg_level_table,
            pcm_channel_num: 0,
            pcm_start_point: 0,
            pcm_end_point: 0,
            pcm_loop_point: 0,
            is_final: false,
            in_pipe: None,
            base_pipe: None,
            out_pipe: None,
            feed_pipe: Rc::new(RefCell::new(Pipe::new(1, 0))),
        }
    }

    // FM module parameters.

    /// C++ `get_attack_rate()`.
    pub fn get_attack_rate(&self) -> i32 {
        self.attack_rate
    }

    /// C++ `set_attack_rate(int)`.
    pub fn set_attack_rate(&mut self, p_value: i32) {
        self.attack_rate = p_value & 63;

        if self.ssg_type == op_params::SSG_REPEAT_TO_ZERO
            || self.ssg_type == op_params::SSG_REPEAT_TO_MAX
        {
            self.eg_ssgec_attack_rate = if self.attack_rate >= 56 { 1 } else { 0 };
        } else {
            self.eg_ssgec_attack_rate = if self.attack_rate >= 60 { 1 } else { 0 };
        }
    }

    /// C++ `get_decay_rate()` / `set_decay_rate(int)`.
    pub fn get_decay_rate(&self) -> i32 {
        self.decay_rate
    }

    /// C++ `set_decay_rate(int)`.
    pub fn set_decay_rate(&mut self, p_value: i32) {
        self.decay_rate = p_value & 63;
    }

    /// C++ `get_sustain_rate()` / `set_sustain_rate(int)`.
    pub fn get_sustain_rate(&self) -> i32 {
        self.sustain_rate
    }

    /// C++ `set_sustain_rate(int)`.
    pub fn set_sustain_rate(&mut self, p_value: i32) {
        self.sustain_rate = p_value & 63;
    }

    /// C++ `get_release_rate()` / `set_release_rate(int)`.
    pub fn get_release_rate(&self) -> i32 {
        self.release_rate
    }

    /// C++ `set_release_rate(int)`.
    pub fn set_release_rate(&mut self, p_value: i32) {
        self.release_rate = p_value & 63;
    }

    /// C++ `get_sustain_level()`.
    pub fn get_sustain_level(&self) -> i32 {
        self.sustain_level
    }

    /// C++ `set_sustain_level(int)`. NOTE: the table lookup indexes with the
    /// RAW argument in C++ (`eg_sustain_level_table[p_value]`, not the
    /// `& 15` masked field); reproduced verbatim.
    pub fn set_sustain_level(&mut self, p_value: i32) {
        self.sustain_level = p_value & 15;
        self.eg_sustain_level = self.table.borrow().eg_sustain_level_table[p_value as usize];
    }

    /// C++ `_update_total_level()`.
    pub fn update_total_level(&mut self) {
        let (env_lshift, env_bottom, env_top) = (
            SiopmRefTable::ENV_LSHIFT,
            SiopmRefTable::ENV_BOTTOM,
            SiopmRefTable::ENV_TOP,
        );

        let mut eg_total_level = ((self.total_level + (self.key_code >> self.eg_key_scale_level_rshift))
            << env_lshift)
            + self.eg_tl_offset
            + self.mute;

        if eg_total_level > env_bottom {
            eg_total_level = env_bottom;
        }
        eg_total_level -= env_top; // Table index + 192.

        self.eg_total_level = eg_total_level;
        self.update_eg_output();
    }

    /// C++ `get_total_level()`.
    pub fn get_total_level(&self) -> i32 {
        self.total_level
    }

    /// C++ `set_total_level(int)`.
    pub fn set_total_level(&mut self, p_value: i32) {
        self.total_level = crate::math::clampi(p_value, 0, 127);
        self.update_total_level();
    }

    /// C++ `offset_total_level(int)`.
    pub fn offset_total_level(&mut self, p_offset: i32) {
        self.eg_tl_offset = p_offset;
        self.update_total_level();
    }

    /// C++ `get_key_scaling_rate()`.
    pub fn get_key_scaling_rate(&self) -> i32 {
        5 - self.key_scaling_rate
    }

    /// C++ `set_key_scaling_rate(int)`.
    pub fn set_key_scaling_rate(&mut self, p_value: i32) {
        self.key_scaling_rate = 5 - (p_value & 3);
        self.eg_key_scale_rate = self.key_code >> self.key_scaling_rate;
    }

    /// C++ `get_key_scaling_level()`.
    pub fn get_key_scaling_level(&self) -> i32 {
        self.key_scaling_level
    }

    /// C++ `set_key_scaling_level(int, bool p_silent = false)` — the C++
    /// default argument becomes an explicit flag.
    pub fn set_key_scaling_level(&mut self, p_value: i32, p_silent: bool) {
        self.key_scaling_level = p_value & 3;
        // [0,1,2,3]->[8,4,3,2]
        self.eg_key_scale_level_rshift = if self.key_scaling_level == 0 {
            8
        } else {
            5 - self.key_scaling_level
        };

        if !p_silent {
            self.update_total_level();
        }
    }

    /// C++ `get_multiple()`.
    pub fn get_multiple(&self) -> i32 {
        self.fine_multiple >> 7
    }

    /// C++ `set_multiple(int)`.
    pub fn set_multiple(&mut self, p_value: i32) {
        let multiple = p_value & 15;
        self.fine_multiple = if multiple != 0 { multiple << 7 } else { 64 };
        self.update_pitch();
    }

    /// C++ `get_fine_multiple()` / `set_fine_multiple(int)`.
    pub fn get_fine_multiple(&self) -> i32 {
        self.fine_multiple
    }

    /// C++ `set_fine_multiple(int)`.
    pub fn set_fine_multiple(&mut self, p_value: i32) {
        self.fine_multiple = p_value;
        self.update_pitch();
    }

    /// C++ `get_detune1()` / `set_detune1(int)`.
    pub fn get_detune1(&self) -> i32 {
        self.detune1
    }

    /// C++ `set_detune1(int)`.
    pub fn set_detune1(&mut self, p_value: i32) {
        self.detune1 = p_value & 7;
        self.update_pitch();
    }

    /// C++ `get_detune2()` / `set_detune2(int)`.
    pub fn get_detune2(&self) -> i32 {
        self.detune2
    }

    /// C++ `set_detune2(int)`.
    pub fn set_detune2(&mut self, p_value: i32) {
        self.detune2 = p_value & 3;
        self.pitch_index_shift = self.table.borrow().dt2_table[self.detune2 as usize];
        self.update_pitch();
    }

    /// C++ `is_amplitude_modulation_enabled()`.
    pub fn is_amplitude_modulation_enabled(&self) -> bool {
        self.amplitude_modulation_shift != 16
    }

    /// C++ `set_amplitude_modulation_enabled(bool)`.
    pub fn set_amplitude_modulation_enabled(&mut self, p_enabled: bool) {
        self.amplitude_modulation_shift = if p_enabled { 2 } else { 16 };
    }

    /// C++ `get_amplitude_modulation_shift()`.
    pub fn get_amplitude_modulation_shift(&self) -> i32 {
        if self.amplitude_modulation_shift == 16 {
            0
        } else {
            3 - self.amplitude_modulation_shift
        }
    }

    /// C++ `set_amplitude_modulation_shift(int)`.
    pub fn set_amplitude_modulation_shift(&mut self, p_value: i32) {
        self.amplitude_modulation_shift = if p_value != 0 { 3 - p_value } else { 16 };
    }

    /// C++ `_update_key_code(int)`.
    pub fn update_key_code(&mut self, p_value: i32) {
        self.key_code = p_value;
        self.eg_key_scale_rate = self.key_code >> self.key_scaling_rate;
        self.update_total_level();
    }

    /// C++ `get_key_code()`.
    pub fn get_key_code(&self) -> i32 {
        self.key_code
    }

    /// C++ `set_key_code(int)`.
    pub fn set_key_code(&mut self, p_value: i32) {
        if self.pitch_fixed {
            return;
        }

        self.update_key_code(p_value & 127);
        self.pitch_index = ((self.key_code - (self.key_code >> 2)) << 6) | (self.pitch_index & 63);
        self.update_pitch();
    }

    /// C++ `is_mute()`.
    pub fn is_mute(&self) -> bool {
        self.mute != 0
    }

    /// C++ `set_mute(bool)`.
    pub fn set_mute(&mut self, p_mute: bool) {
        self.mute = if p_mute { SiopmRefTable::ENV_BOTTOM } else { 0 };
        self.update_total_level();
    }

    /// C++ `get_ssg_type()`.
    pub fn get_ssg_type(&self) -> i32 {
        self.ssg_type
    }

    /// C++ `set_ssg_type(int)`.
    pub fn set_ssg_type(&mut self, p_value: i32) {
        if p_value >= op_params::SSG_REPEAT_TO_ZERO {
            self.eg_state_table_index = 1;
            self.ssg_type = p_value;
            if self.ssg_type >= op_params::SSG_MAX {
                self.ssg_type = op_params::SSG_IGNORE;
            }
        } else {
            self.eg_state_table_index = 0;
            self.ssg_type = op_params::SSG_DISABLED;
        }
    }

    /// C++ `is_envelope_reset_on_attack()` /
    /// `set_envelope_reset_on_attack(bool)`.
    pub fn is_envelope_reset_on_attack(&self) -> bool {
        self.envelope_reset_on_attack
    }

    /// C++ `set_envelope_reset_on_attack(bool)`.
    pub fn set_envelope_reset_on_attack(&mut self, p_reset: bool) {
        self.envelope_reset_on_attack = p_reset;
    }

    // Pulse generator.

    /// C++ `_update_pitch()`.
    pub fn update_pitch(&mut self) {
        let index =
            (self.pitch_index + self.pitch_index_shift + self.pitch_index_shift2) & self.pitch_table_filter;
        let step = self.pitch_table[index as usize] >> self.wave_phase_step_shift;
        self.update_phase_step(step);
    }

    /// C++ `_update_phase_step(int)`. `+= dt1` / `*= fine_multiple` use
    /// wrapping arithmetic (the product is reachable above `i32::MAX` — C++
    /// relies on two's-complement wrap there).
    pub fn update_phase_step(&mut self, p_step: i32) {
        let (dt, pitch_shift) = {
            let borrow = self.table.borrow();
            (
                borrow.dt1_table[self.detune1 as usize][self.key_code as usize],
                borrow.sample_rate_pitch_shift,
            )
        };

        let mut phase_step = p_step;
        phase_step = phase_step.wrapping_add(dt);
        phase_step = phase_step.wrapping_mul(self.fine_multiple);
        self.phase_step = phase_step >> (7 - pitch_shift); // 44kHz:1/128, 22kHz:1/256
    }

    /// C++ `get_pulse_generator_type()`.
    pub fn get_pulse_generator_type(&self) -> i32 {
        self.pg_type
    }

    /// C++ `set_pulse_generator_type(int)`.
    pub fn set_pulse_generator_type(&mut self, p_type: i32) {
        self.pg_type = p_type & SiopmRefTable::PG_FILTER;

        let wave_table = self.table.borrow().get_wave_table(self.pg_type);
        // C++ dereferences the returned Ref unconditionally; a missing table
        // is a hard failure there. The Rust port prints and leaves the
        // previous wave intact (no valid caller path exists).
        let Some(wave_table) = wave_table else {
            crate::error::err_print_body("Parameter \"wave_table\" is null.", false);
            return;
        };
        let borrow = wave_table.borrow();
        self.wave_table = borrow.get_wavelet();
        self.wave_fixed_bits = borrow.get_fixed_bits();
    }

    /// C++ `get_pitch_table_type()`.
    pub fn get_pitch_table_type(&self) -> i32 {
        self.pt_type
    }

    /// C++ `set_pitch_table_type(SiONPitchTableType)`.
    pub fn set_pitch_table_type(&mut self, p_type: i32) {
        self.pt_type = p_type;

        let (pitch_table, shift_filter) = {
            let borrow = self.table.borrow();
            (
                borrow.pitch_table[p_type as usize].clone(),
                borrow.phase_step_shift_filter[p_type as usize],
            )
        };

        self.wave_phase_step_shift =
            (SiopmRefTable::PHASE_BITS - self.wave_fixed_bits) & shift_filter;
        self.pitch_table = pitch_table;
        self.pitch_table_filter = self.pitch_table.len() as i32 - 1;
    }

    /// C++ `get_wave_value(int)` / `get_wave_fixed_bits()`.
    pub fn get_wave_value(&self, p_index: i32) -> i32 {
        crate::err_fail_index_v!(p_index, "p_index", self.wave_table.len() as i32, "_wave_table.size()", -1);
        self.wave_table[p_index as usize]
    }

    /// C++ `get_wave_fixed_bits()`.
    pub fn get_wave_fixed_bits(&self) -> i32 {
        self.wave_fixed_bits
    }

    /// C++ `get_phase()` / `set_phase(int)` / `adjust_phase(int)`.
    pub fn get_phase(&self) -> i32 {
        self.phase
    }

    /// C++ `set_phase(int)`.
    pub fn set_phase(&mut self, p_value: i32) {
        self.phase = p_value;
    }

    /// C++ `adjust_phase(int)`.
    pub fn adjust_phase(&mut self, p_diff: i32) {
        self.phase = self.phase.wrapping_add(p_diff);
    }

    /// C++ `get_key_on_phase_raw()`.
    pub fn get_key_on_phase_raw(&self) -> i32 {
        self.key_on_phase
    }

    /// C++ `get_pitch_index()` / `set_pitch_index(int)`.
    pub fn get_pitch_index(&self) -> i32 {
        self.pitch_index
    }

    /// C++ `set_pitch_index(int)`.
    pub fn set_pitch_index(&mut self, p_value: i32) {
        if self.pitch_fixed {
            return;
        }

        self.pitch_index = p_value;
        let key_code = {
            let borrow = self.table.borrow();
            borrow.note_number_to_key_code[((p_value >> 6) & 127) as usize]
        };
        self.update_key_code(key_code);
        self.update_pitch();
    }

    /// C++ `is_pitch_fixed()`.
    pub fn is_pitch_fixed(&self) -> bool {
        self.pitch_fixed
    }

    /// C++ `set_fixed_pitch_index(int)` (setting to 0 disables the fixed
    /// pitch flag).
    pub fn set_fixed_pitch_index(&mut self, p_value: i32) {
        if p_value > 0 {
            self.pitch_index = p_value;

            let key_code = {
                let borrow = self.table.borrow();
                borrow.note_number_to_key_code[((self.pitch_index >> 6) & 127) as usize]
            };
            self.update_key_code(key_code);
            self.update_pitch();
            self.pitch_fixed = true;
        } else {
            self.pitch_fixed = false;
        }
    }

    /// C++ `get_ptss_detune()` / `set_ptss_detune(int)`.
    pub fn get_ptss_detune(&self) -> i32 {
        self.pitch_index_shift
    }

    /// C++ `set_ptss_detune(int)`.
    pub fn set_ptss_detune(&mut self, p_value: i32) {
        self.detune2 = 0;
        self.pitch_index_shift = p_value;
        self.update_pitch();
    }

    /// C++ `get_pm_detune()` / `set_pm_detune(int)`.
    pub fn get_pm_detune(&self) -> i32 {
        self.pitch_index_shift2
    }

    /// C++ `set_pm_detune(int)`.
    pub fn set_pm_detune(&mut self, p_value: i32) {
        self.pitch_index_shift2 = p_value;
        self.update_pitch();
    }

    /// C++ `get_fm_shift()`.
    pub fn get_fm_shift(&self) -> i32 {
        self.fm_shift
    }

    /// C++ `get_key_on_phase()` (255 = no phase reset, -1 = random).
    pub fn get_key_on_phase(&self) -> i32 {
        if self.key_on_phase >= 0 {
            self.key_on_phase >> (SiopmRefTable::PHASE_BITS - 8)
        } else if self.key_on_phase == -1 {
            -1
        } else {
            255
        }
    }

    /// C++ `set_key_on_phase(int)`.
    pub fn set_key_on_phase(&mut self, p_phase: i32) {
        if p_phase == 255 {
            self.key_on_phase = -2;
        } else if p_phase == -1 {
            self.key_on_phase = -1;
        } else {
            self.key_on_phase = (p_phase & 255) << (SiopmRefTable::PHASE_BITS - 8);
        }
    }

    /// C++ `get_fm_level()` / `set_fm_level(int)`.
    pub fn get_fm_level(&self) -> i32 {
        if self.fm_shift > 10 {
            self.fm_shift - 10
        } else {
            0
        }
    }

    /// C++ `set_fm_level(int)`.
    pub fn set_fm_level(&mut self, p_level: i32) {
        self.fm_shift = if p_level != 0 { p_level + 10 } else { 0 };
    }

    /// C++ `get_key_fraction()` / `set_key_fraction(int)`.
    pub fn get_key_fraction(&self) -> i32 {
        self.pitch_index & 63
    }

    /// C++ `set_key_fraction(int)`.
    pub fn set_key_fraction(&mut self, p_value: i32) {
        self.pitch_index = (self.pitch_index & 0xffc0) | (p_value & 63);
        self.update_pitch();
    }

    /// C++ `set_fnumber(int)` (F-Number for OPNA; naive implementation).
    pub fn set_fnumber(&mut self, p_value: i32) {
        self.update_key_code((p_value >> 7) & 127);
        self.detune2 = 0;
        self.pitch_index = 0;
        self.pitch_index_shift = 0;
        self.update_phase_step((p_value & 2047) << ((p_value >> 11) & 7));
    }

    /// C++ `tick_pulse_generator(int p_extra = 0)` — the phase accumulator
    /// wraps like the C++ `int`.
    pub fn tick_pulse_generator(&mut self, p_extra: i32) {
        self.phase = self.phase.wrapping_add(self.phase_step.wrapping_add(p_extra));
    }

    // Envelope generator.

    /// C++ `get_eg_state()` / `set_eg_state(EGState)`.
    pub fn get_eg_state(&self) -> EgState {
        self.eg_state
    }

    /// C++ `set_eg_state(EGState)`.
    pub fn set_eg_state(&mut self, p_state: EgState) {
        self.shift_eg_state(p_state);
    }

    /// C++ `_shift_eg_state(EGState)` — the full EG state machine, including
    /// the `[[fallthrough]]` chains (Attack→Decay→Sustain, Release→Off).
    pub fn shift_eg_state(&mut self, p_state: EgState) {
        let (
            eg_level_tables,
            eg_increment_tables,
            eg_increment_tables_attack,
            eg_table_selector,
            eg_timer_steps,
            eg_ssg_table_index,
        ) = {
            let borrow = self.table.borrow();
            (
                borrow.eg_level_tables,
                borrow.eg_increment_tables,
                borrow.eg_increment_tables_attack,
                borrow.eg_table_selector,
                borrow.eg_timer_steps,
                borrow.eg_ssg_table_index,
            )
        };
        let env_bottom = SiopmRefTable::ENV_BOTTOM;
        let env_bottom_ssgec = SiopmRefTable::ENV_BOTTOM_SSGEC;

        let mut state = p_state;
        loop {
            match state {
                EgState::Attack => {
                    self.eg_ssgec_state += 1;
                    if self.eg_ssgec_state == 3 {
                        self.eg_ssgec_state = 1;
                    }

                    if self.attack_rate + self.eg_key_scale_rate < 62 {
                        if self.envelope_reset_on_attack {
                            self.eg_level = env_bottom;
                        }
                        self.eg_state = EgState::Attack;
                        self.eg_level_table = eg_level_tables[0];

                        let index = if self.attack_rate != 0 {
                            self.attack_rate + self.eg_key_scale_rate
                        } else {
                            96
                        };
                        self.eg_increment_table =
                            eg_increment_tables_attack[eg_table_selector[index as usize] as usize];
                        self.eg_timer_step = eg_timer_steps[index as usize];
                        break;
                    }
                    // [[fallthrough]] -> EG_DECAY
                    state = EgState::Decay;
                }

                EgState::Decay => {
                    if self.eg_sustain_level != 0 {
                        self.eg_state = EgState::Decay;

                        if self.ssg_type > op_params::SSG_REPEAT_TO_ZERO {
                            self.eg_level = 0;

                            self.eg_state_shift_level = self.eg_sustain_level >> 2;
                            if self.eg_state_shift_level > env_bottom_ssgec {
                                self.eg_state_shift_level = env_bottom_ssgec;
                            }

                            let normalized_ssg_type =
                                (self.ssg_type - op_params::SSG_REPEAT_TO_ZERO) as usize;
                            let level_index = eg_ssg_table_index[normalized_ssg_type]
                                [self.eg_ssgec_attack_rate as usize][self.eg_ssgec_state as usize];
                            self.eg_level_table = eg_level_tables[level_index as usize];
                        } else {
                            self.eg_level = 0;
                            self.eg_state_shift_level = self.eg_sustain_level;
                            self.eg_level_table = eg_level_tables[0];
                        }

                        let index = if self.decay_rate != 0 {
                            self.decay_rate + self.eg_key_scale_rate
                        } else {
                            96
                        };
                        self.eg_increment_table =
                            eg_increment_tables[eg_table_selector[index as usize] as usize];
                        self.eg_timer_step = eg_timer_steps[index as usize];
                        break;
                    }
                    // [[fallthrough]] -> EG_SUSTAIN
                    state = EgState::Sustain;
                }

                EgState::Sustain => {
                    self.eg_state = EgState::Sustain;

                    if self.ssg_type >= op_params::SSG_REPEAT_TO_ZERO {
                        self.eg_level = self.eg_sustain_level >> 2;
                        self.eg_state_shift_level = env_bottom_ssgec;

                        let normalized_ssg_type =
                            (self.ssg_type - op_params::SSG_REPEAT_TO_ZERO) as usize;
                        let level_index = eg_ssg_table_index[normalized_ssg_type]
                            [self.eg_ssgec_attack_rate as usize][self.eg_ssgec_state as usize];
                        self.eg_level_table = eg_level_tables[level_index as usize];
                    } else {
                        self.eg_level = self.eg_sustain_level;
                        self.eg_state_shift_level = env_bottom;
                        self.eg_level_table = eg_level_tables[0];
                    }

                    let index = if self.sustain_rate != 0 {
                        self.sustain_rate + self.eg_key_scale_rate
                    } else {
                        96
                    };
                    self.eg_increment_table =
                        eg_increment_tables[eg_table_selector[index as usize] as usize];
                    self.eg_timer_step = eg_timer_steps[index as usize];
                    break;
                }

                EgState::Release => {
                    if self.eg_level < env_bottom {
                        self.eg_state = EgState::Release;
                        self.eg_state_shift_level = env_bottom;

                        if self.ssg_type >= op_params::SSG_REPEAT_TO_ZERO {
                            self.eg_level_table = eg_level_tables[1];
                        } else {
                            self.eg_level_table = eg_level_tables[0];
                        }

                        let index = self.release_rate + self.eg_key_scale_rate;
                        self.eg_increment_table =
                            eg_increment_tables[eg_table_selector[index as usize] as usize];
                        self.eg_timer_step = eg_timer_steps[index as usize];
                        break;
                    }
                    // [[fallthrough]] -> EG_OFF
                    state = EgState::Off;
                }

                EgState::Off => {
                    self.eg_state = EgState::Off;
                    self.eg_level = env_bottom;
                    self.eg_state_shift_level = env_bottom + 1;
                    self.eg_level_table = eg_level_tables[0];

                    self.eg_increment_table = eg_increment_tables[17]; // 17 = all zero
                    self.eg_timer_step = eg_timer_steps[96]; // 96 = all zero
                    break;
                }
            }
        }
    }

    /// C++ `get_eg_output()`.
    pub fn get_eg_output(&self) -> i32 {
        self.eg_output
    }

    /// C++ `tick_eg(int p_timer_initial)`.
    pub fn tick_eg(&mut self, p_timer_initial: i32) {
        self.eg_timer -= self.eg_timer_step;
        if self.eg_timer >= 0 {
            return;
        }

        if self.eg_state == EgState::Attack {
            let offset = self.eg_increment_table[self.eg_counter as usize];
            if offset > 0 {
                self.eg_level -= 1 + (self.eg_level >> offset);
                if self.eg_level <= 0 {
                    let next = EG_NEXT_STATE_TABLE[self.eg_state_table_index][self.eg_state as usize];
                    self.shift_eg_state(next);
                }
            }
        } else {
            self.eg_level += self.eg_increment_table[self.eg_counter as usize];
            if self.eg_level >= self.eg_state_shift_level {
                let next = EG_NEXT_STATE_TABLE[self.eg_state_table_index][self.eg_state as usize];
                self.shift_eg_state(next);
            }
        }

        self.update_eg_output();
        self.eg_counter = (self.eg_counter + 1) & 7;

        self.eg_timer += p_timer_initial;
    }

    /// C++ `update_eg_output()`.
    pub fn update_eg_output(&mut self) {
        self.eg_output = (self.eg_level_table[self.eg_level as usize] + self.eg_total_level) << 3;
    }

    /// C++ `update_eg_output_from(SiOPMOperator*)`.
    pub fn update_eg_output_from(&mut self, p_other: &Operator) {
        self.eg_output =
            (p_other.eg_level_table[p_other.eg_level as usize] + self.eg_total_level) << 3;
    }

    // PCM wave.

    /// C++ `get_pcm_channel_num()` / `get_pcm_start_point()` /
    /// `get_pcm_end_point()` / `get_pcm_loop_point()`.
    pub fn get_pcm_channel_num(&self) -> i32 {
        self.pcm_channel_num
    }

    /// C++ `get_pcm_start_point()`.
    pub fn get_pcm_start_point(&self) -> i32 {
        self.pcm_start_point
    }

    /// C++ `get_pcm_end_point()`.
    pub fn get_pcm_end_point(&self) -> i32 {
        self.pcm_end_point
    }

    /// C++ `get_pcm_loop_point()`.
    pub fn get_pcm_loop_point(&self) -> i32 {
        self.pcm_loop_point
    }

    // Pipes.

    /// C++ `is_final()`.
    pub fn is_final(&self) -> bool {
        self.is_final
    }

    /// C++ `get_in_pipe()` / `get_base_pipe()` / `get_out_pipe()` /
    /// `get_feed_pipe()`.
    pub fn get_in_pipe(&self) -> Option<&PipeRc> {
        self.in_pipe.as_ref()
    }

    /// C++ `get_base_pipe()` (`set_base_pipe` assigns directly).
    pub fn get_base_pipe(&self) -> Option<&PipeRc> {
        self.base_pipe.as_ref()
    }

    /// C++ `get_out_pipe()`.
    pub fn get_out_pipe(&self) -> Option<&PipeRc> {
        self.out_pipe.as_ref()
    }

    /// C++ `get_feed_pipe()`.
    pub fn get_feed_pipe(&self) -> &PipeRc {
        &self.feed_pipe
    }

    /// C++ `set_base_pipe(SinglyLinkedList<int>*)`.
    pub fn set_base_pipe(&mut self, p_pipe: PipeRc) {
        self.base_pipe = Some(p_pipe);
    }

    /// C++ `set_pipes(out, in = nullptr, final = false)`.
    pub fn set_pipes(
        &mut self,
        ctx: &mut dyn ChipContext,
        p_out_pipe: PipeRc,
        p_in_pipe: Option<PipeRc>,
        p_final: bool,
    ) {
        self.is_final = p_final;
        self.fm_shift = 15;

        let base_pipe = match p_in_pipe.as_ref() {
            Some(in_pipe) if Rc::ptr_eq(in_pipe, &p_out_pipe) => ctx.get_zero_buffer(),
            _ => p_out_pipe.clone(),
        };

        self.out_pipe = Some(p_out_pipe);
        self.in_pipe = Some(p_in_pipe.unwrap_or_else(|| ctx.get_zero_buffer()));
        self.base_pipe = Some(base_pipe);
    }

    /// C++ `set_operator_params(const Ref<SiOPMOperatorParams>&)`. Some code
    /// is duplicated from the respective setters to avoid their side
    /// effects, exactly as in C++. Modify with care.
    pub fn set_operator_params(&mut self, p_params: &OperatorParams) {
        self.set_pulse_generator_type(p_params.get_pulse_generator_type());
        self.set_pitch_table_type(p_params.get_pitch_table_type());

        self.set_key_on_phase(p_params.get_initial_phase());

        self.set_attack_rate(p_params.get_attack_rate());
        self.set_decay_rate(p_params.get_decay_rate());
        self.set_sustain_rate(p_params.get_sustain_rate());
        self.set_release_rate(p_params.get_release_rate());

        self.set_key_scaling_rate(p_params.get_key_scaling_rate());
        self.set_key_scaling_level(p_params.get_key_scaling_level(), true);

        self.set_amplitude_modulation_shift(p_params.get_amplitude_modulation_shift());

        self.fine_multiple = p_params.get_fine_multiple();
        self.fm_shift = (p_params.get_frequency_modulation_level() & 7) + 10;
        self.detune1 = p_params.get_detune1() & 7;
        self.pitch_index_shift = p_params.get_detune2();

        self.mute = if p_params.is_mute() {
            SiopmRefTable::ENV_BOTTOM
        } else {
            0
        };
        self.set_ssg_type(p_params.get_ssg_envelope_control());
        self.envelope_reset_on_attack = p_params.is_envelope_reset_on_attack();

        if p_params.get_fixed_pitch() > 0 {
            self.pitch_index = p_params.get_fixed_pitch();

            let key_code = {
                let borrow = self.table.borrow();
                borrow.note_number_to_key_code[((self.pitch_index >> 6) & 127) as usize]
            };
            self.update_key_code(key_code);
            self.pitch_fixed = true;
        } else {
            self.pitch_fixed = false;
        }

        self.set_sustain_level(p_params.get_sustain_level() & 15);
        self.set_total_level(p_params.get_total_level());

        self.update_pitch();
    }

    /// C++ `get_operator_params(Ref<SiOPMOperatorParams>)`.
    pub fn get_operator_params(&self, r_params: &mut OperatorParams) {
        let params = r_params;
        params.set_pulse_generator_type(self.pg_type);
        params.set_pitch_table_type(self.pt_type);

        params.set_attack_rate(self.attack_rate);
        params.set_decay_rate(self.decay_rate);
        params.set_sustain_rate(self.sustain_rate);
        params.set_release_rate(self.release_rate);
        params.set_sustain_level(self.sustain_level);
        params.set_total_level(self.total_level);

        params.set_key_scaling_rate(self.get_key_scaling_rate());
        params.set_key_scaling_level(self.key_scaling_level);
        params.set_fine_multiple(self.get_fine_multiple());
        params.set_detune1(self.detune1);
        params.set_detune2(self.get_ptss_detune());
        params.set_amplitude_modulation_shift(self.get_amplitude_modulation_shift());

        params.set_ssg_envelope_control(self.get_ssg_type());
        params.set_envelope_reset_on_attack(self.is_envelope_reset_on_attack());

        params.set_initial_phase(self.get_key_on_phase());
        params.set_frequency_modulation_level(self.get_fm_level());
    }

    /// C++ `set_wave_table(const Ref<SiOPMWaveTable>&)`.
    pub fn set_wave_table(&mut self, p_wave_table: &Rc<RefCell<SiopmWaveTable>>) {
        self.pg_type = PULSE_USER_CUSTOM;

        let (pt_type, wave_table, wave_fixed_bits) = {
            let borrow = p_wave_table.borrow();
            (
                borrow.get_default_pitch_table_type(),
                borrow.get_wavelet(),
                borrow.get_fixed_bits(),
            )
        };
        self.pt_type = pt_type;
        self.wave_table = wave_table;
        self.wave_fixed_bits = wave_fixed_bits;
    }

    /// C++ `set_pcm_data(const Ref<SiOPMWavePCMData>&)`.
    pub fn set_pcm_data(&mut self, p_pcm_data: Option<&Rc<RefCell<SiopmWavePcmData>>>) {
        let mut valid = false;
        if let Some(pcm_data) = p_pcm_data {
            let borrow = pcm_data.borrow();
            if !borrow.get_wavelet().is_empty() {
                valid = true;
                self.pg_type = PULSE_USER_PCM;
                self.pt_type = PITCH_TABLE_PCM;

                self.wave_table = borrow.get_wavelet();
                self.wave_fixed_bits = Self::PCM_WAVE_FIXED_BITS;

                self.pcm_channel_num = borrow.get_channel_count();
                self.pcm_start_point = borrow.get_start_point();
                self.pcm_end_point = borrow.get_end_point();
                self.pcm_loop_point = borrow.get_loop_point();
            }
        }

        if valid {
            self.key_on_phase = self.pcm_start_point.wrapping_shl(Self::PCM_WAVE_FIXED_BITS as u32);
        } else {
            // Quick initialization for SiOPMChannelPCM (the C++ double
            // assignment to _pcm_loop_point is reproduced verbatim).
            self.pcm_end_point = 0;
            self.pcm_loop_point = 0;
            self.pcm_loop_point = -1;
        }
    }

    /// C++ `note_on()`.
    pub fn note_on(&mut self) {
        if self.key_on_phase >= 0 {
            self.phase = self.key_on_phase;
        } else if self.key_on_phase == -1 {
            let mut rng = RandomNumberGenerator::new();
            self.phase = rng.randi_range(0, SiopmRefTable::PHASE_MAX);
        }

        self.eg_ssgec_state = -1;
        self.shift_eg_state(EgState::Attack);
        self.update_eg_output();
    }

    /// C++ `note_off()`.
    pub fn note_off(&mut self) {
        self.shift_eg_state(EgState::Release);
        self.update_eg_output();
    }

    /// C++ `initialize()` (chip access via the [`ChipContext`] seam).
    pub fn initialize(&mut self, ctx: &mut dyn ChipContext) {
        // Reset operator connections.
        self.is_final = true;
        let zero = ctx.get_zero_buffer();
        self.in_pipe = Some(zero.clone());
        self.base_pipe = Some(zero);
        self.feed_pipe.borrow_mut().set_value(0);

        // Reset all parameters.
        let init_params = ctx.get_init_operator_params();
        {
            let borrow = init_params.borrow();
            self.set_operator_params(&borrow);
        }

        // Reset some other parameters.
        self.eg_tl_offset = 0;
        self.pitch_index_shift2 = 0;

        self.pcm_channel_num = 0;
        self.pcm_start_point = 0;
        self.pcm_end_point = 0;
        self.pcm_loop_point = -1;

        // Reset PG and EG states.
        self.reset();
    }

    /// C++ `reset()`.
    pub fn reset(&mut self) {
        self.shift_eg_state(EgState::Off);
        self.update_eg_output();
        self.eg_timer = SiopmRefTable::ENV_TIMER_INITIAL;
        self.eg_counter = 0;
        self.eg_ssgec_state = 0;

        self.phase = 0;
    }

    /// C++ `_to_string()`.
    pub fn to_string_repr(&self) -> String {
        let mut params = String::new();

        params += &format!("pg={}, ", self.pg_type);
        params += &format!("pt={}, ", self.pt_type);

        params += &format!("ar={}, ", self.attack_rate);
        params += &format!("dr={}, ", self.decay_rate);
        params += &format!("sr={}, ", self.sustain_rate);
        params += &format!("rr={}, ", self.release_rate);
        params += &format!("sl={}, ", self.sustain_level);
        params += &format!("tl={}, ", self.total_level);

        params += &format!(
            "keyscale=({}, {}), ",
            self.get_key_scaling_rate(),
            self.get_key_scaling_level()
        );
        params += &format!("fmul={}, ", self.get_fine_multiple());
        params += &format!(
            "detune=({}, {}), ",
            self.get_detune1(),
            self.get_ptss_detune()
        );

        params += &format!("amp={}, ", self.get_amplitude_modulation_shift());
        params += &format!("phase={}, ", self.get_key_on_phase());
        params += &format!(
            "note={}, ",
            if self.is_pitch_fixed() { "yes" } else { "no" }
        );

        params += &format!("ssgec={}, ", self.ssg_type);
        params += &format!("mute={}, ", self.mute);
        params += &format!(
            "reset={}",
            if self.envelope_reset_on_attack { "yes" } else { "no" }
        );

        format!("SiOPMOperator: {params}")
    }
}
