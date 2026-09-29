//! Port of `libSiON-cpp/src/chip/siopm_operator_params.{h,cpp}`.
//!
//! The C++ members are plain `int` / `bool` (no bitfields in this revision),
//! so they map to `i32` / `bool` per CONVENTIONS.md. The C++ class has no
//! static `initialize`/`finalize`; the `SiOPMRefTable` dependency lives only
//! in [`OperatorParams::set_pulse_generator_type`], which goes through
//! [`crate::chip::ref_table::instance`] (lazy-init, same as C++
//! `get_instance()` after `SiOPMRefTable::initialize()`).

use crate::chip::ref_table;
use crate::sion_enums::{PITCH_TABLE_OPM, PULSE_SINE};
use crate::utils::string::itos;

/// C++ `SiOPMOperatorParams::SSGEnvelopeControl`.
pub const SSG_DISABLED: i32 = 0; // Values 0-7 mean it's disabled.
pub const SSG_REPEAT_TO_ZERO: i32 = 8; // Repeats the ADSR envelope upon the volume reaching 0.
pub const SSG_IGNORE: i32 = 9; // As if SSG-EG was disabled, though it is enabled.
pub const SSG_REPEAT_SHUTTLE: i32 = 10; // Repeats the ADSR envelope forward and backward.
pub const SSG_ONCE_HOLD_HIGH: i32 = 11; // Uses the ADSR envelope, then stays at the maximum volume.
pub const SSG_REPEAT_TO_MAX: i32 = 12;
pub const SSG_INVERSE: i32 = 13;
pub const SSG_REPEAT_SHUTTLE_INVERSE: i32 = 14;
pub const SSG_ONCE_HOLD_LOW: i32 = 15;
pub const SSG_CONSTANT_HIGH: i32 = 16;
pub const SSG_CONSTANT_LOW: i32 = 17;
pub const SSG_MAX: i32 = 18;

/// C++ `SiOPMOperatorParams`. Public fields mirror the C++ `friend` access
/// (`SiOPMChannelParams`, `TranslatorUtil`) into the raw members.
pub struct OperatorParams {
    // Pulse generator type [0,511]
    pub pulse_generator_type: i32,
    // Pitch table type [0,7]
    pub pitch_table_type: i32,

    // Attack rate [0,63]
    pub attack_rate: i32,
    // Decay rate [0,63]
    pub decay_rate: i32,
    // Sustain rate [0,63]
    pub sustain_rate: i32,
    // Release rate [0,63]
    pub release_rate: i32,
    // Sustain level [0,15]
    pub sustain_level: i32,
    // Total level [0,127]
    pub total_level: i32,

    // Key scaling rate [0,3]
    pub key_scaling_rate: i32,
    // Key scaling level [0,3]
    pub key_scaling_level: i32,

    // Fine multiple [0,...]
    pub fine_multiple: i32,
    // Detune 1 [0,7]
    pub detune1: i32,
    // Detune 2 [0,...]
    pub detune2: i32,

    // Amp modulation shift [0,3]
    pub amplitude_modulation_shift: i32,
    // Initial phase [0,255]. 255 means no phase reset.
    pub initial_phase: i32,
    // 0 means pitch is not fixed.
    pub fixed_pitch: i32,

    pub mute: bool,
    // SSG-type envelope control [0,17].
    pub ssg_envelope_control: i32,
    // Frequency modulation level [0,7]. 5 means standard modulation.
    pub frequency_modulation_level: i32,
    pub envelope_reset_on_attack: bool,
}

impl OperatorParams {
    /// C++ `get_pulse_generator_type()`.
    pub fn get_pulse_generator_type(&self) -> i32 {
        self.pulse_generator_type
    }

    /// C++ `set_pulse_generator_type(int)`.
    pub fn set_pulse_generator_type(&mut self, p_type: i32) {
        self.pulse_generator_type = p_type & 511;
        let wave_table = ref_table::instance()
            .borrow()
            .get_wave_table(self.pulse_generator_type);
        if let Some(wave_table) = wave_table {
            self.pitch_table_type = wave_table.borrow().get_default_pitch_table_type();
        }
    }

    /// C++ `get_pitch_table_type()`.
    pub fn get_pitch_table_type(&self) -> i32 {
        self.pitch_table_type
    }

    /// C++ `set_pitch_table_type(SiONPitchTableType)`.
    pub fn set_pitch_table_type(&mut self, p_type: i32) {
        self.pitch_table_type = p_type;
    }

    /// C++ `get_attack_rate()`.
    pub fn get_attack_rate(&self) -> i32 {
        self.attack_rate
    }

    /// C++ `set_attack_rate(int)`.
    pub fn set_attack_rate(&mut self, p_value: i32) {
        self.attack_rate = p_value;
    }

    /// C++ `get_decay_rate()`.
    pub fn get_decay_rate(&self) -> i32 {
        self.decay_rate
    }

    /// C++ `set_decay_rate(int)`.
    pub fn set_decay_rate(&mut self, p_value: i32) {
        self.decay_rate = p_value;
    }

    /// C++ `get_sustain_rate()`.
    pub fn get_sustain_rate(&self) -> i32 {
        self.sustain_rate
    }

    /// C++ `set_sustain_rate(int)`.
    pub fn set_sustain_rate(&mut self, p_value: i32) {
        self.sustain_rate = p_value;
    }

    /// C++ `get_release_rate()`.
    pub fn get_release_rate(&self) -> i32 {
        self.release_rate
    }

    /// C++ `set_release_rate(int)`.
    pub fn set_release_rate(&mut self, p_value: i32) {
        self.release_rate = p_value;
    }

    /// C++ `get_sustain_level()`.
    pub fn get_sustain_level(&self) -> i32 {
        self.sustain_level
    }

    /// C++ `set_sustain_level(int)`.
    pub fn set_sustain_level(&mut self, p_value: i32) {
        self.sustain_level = p_value;
    }

    /// C++ `get_total_level()`.
    pub fn get_total_level(&self) -> i32 {
        self.total_level
    }

    /// C++ `set_total_level(int)`.
    pub fn set_total_level(&mut self, p_value: i32) {
        self.total_level = p_value;
    }

    /// C++ `get_key_scaling_rate()`.
    pub fn get_key_scaling_rate(&self) -> i32 {
        self.key_scaling_rate
    }

    /// C++ `set_key_scaling_rate(int)`.
    pub fn set_key_scaling_rate(&mut self, p_value: i32) {
        self.key_scaling_rate = p_value;
    }

    /// C++ `get_key_scaling_level()`.
    pub fn get_key_scaling_level(&self) -> i32 {
        self.key_scaling_level
    }

    /// C++ `set_key_scaling_level(int)`.
    pub fn set_key_scaling_level(&mut self, p_value: i32) {
        self.key_scaling_level = p_value;
    }

    /// C++ `get_fine_multiple()`.
    pub fn get_fine_multiple(&self) -> i32 {
        self.fine_multiple
    }

    /// C++ `set_fine_multiple(int)`.
    pub fn set_fine_multiple(&mut self, p_value: i32) {
        self.fine_multiple = p_value;
    }

    /// C++ `get_multiple()` — Multiple [0,15].
    pub fn get_multiple(&self) -> i32 {
        (self.fine_multiple >> 7) & 15
    }

    /// C++ `set_multiple(int)`.
    pub fn set_multiple(&mut self, p_value: i32) {
        self.fine_multiple = if p_value == 0 {
            64
        } else {
            p_value.wrapping_shl(7)
        };
    }

    /// C++ `get_detune1()`.
    pub fn get_detune1(&self) -> i32 {
        self.detune1
    }

    /// C++ `set_detune1(int)`.
    pub fn set_detune1(&mut self, p_value: i32) {
        self.detune1 = p_value;
    }

    /// C++ `get_detune2()`.
    pub fn get_detune2(&self) -> i32 {
        self.detune2
    }

    /// C++ `set_detune2(int)`.
    pub fn set_detune2(&mut self, p_value: i32) {
        self.detune2 = p_value;
    }

    /// C++ `get_amplitude_modulation_shift()`.
    pub fn get_amplitude_modulation_shift(&self) -> i32 {
        self.amplitude_modulation_shift
    }

    /// C++ `set_amplitude_modulation_shift(int)`.
    pub fn set_amplitude_modulation_shift(&mut self, p_value: i32) {
        self.amplitude_modulation_shift = p_value;
    }

    /// C++ `get_initial_phase()`.
    pub fn get_initial_phase(&self) -> i32 {
        self.initial_phase
    }

    /// C++ `set_initial_phase(int)`.
    pub fn set_initial_phase(&mut self, p_value: i32) {
        self.initial_phase = p_value;
    }

    /// C++ `get_fixed_pitch()`.
    pub fn get_fixed_pitch(&self) -> i32 {
        self.fixed_pitch
    }

    /// C++ `set_fixed_pitch(int)`.
    pub fn set_fixed_pitch(&mut self, p_value: i32) {
        self.fixed_pitch = p_value;
    }

    /// C++ `is_mute()`.
    pub fn is_mute(&self) -> bool {
        self.mute
    }

    /// C++ `set_mute(bool)`.
    pub fn set_mute(&mut self, p_mute: bool) {
        self.mute = p_mute;
    }

    /// C++ `get_ssg_envelope_control()`.
    pub fn get_ssg_envelope_control(&self) -> i32 {
        self.ssg_envelope_control
    }

    /// C++ `set_ssg_envelope_control(int)`.
    pub fn set_ssg_envelope_control(&mut self, p_value: i32) {
        if p_value >= SSG_MAX {
            self.ssg_envelope_control = SSG_IGNORE;
        } else if p_value < SSG_REPEAT_TO_ZERO {
            self.ssg_envelope_control = SSG_DISABLED;
        } else {
            self.ssg_envelope_control = p_value;
        }
    }

    /// C++ `get_frequency_modulation_level()`.
    pub fn get_frequency_modulation_level(&self) -> i32 {
        self.frequency_modulation_level
    }

    /// C++ `set_frequency_modulation_level(int)`.
    pub fn set_frequency_modulation_level(&mut self, p_value: i32) {
        self.frequency_modulation_level = p_value;
    }

    /// C++ `is_envelope_reset_on_attack()`.
    pub fn is_envelope_reset_on_attack(&self) -> bool {
        self.envelope_reset_on_attack
    }

    /// C++ `set_envelope_reset_on_attack(bool)`.
    pub fn set_envelope_reset_on_attack(&mut self, p_reset: bool) {
        self.envelope_reset_on_attack = p_reset;
    }

    /// C++ `initialize()`.
    pub fn initialize(&mut self) {
        self.pulse_generator_type = PULSE_SINE;
        self.pitch_table_type = PITCH_TABLE_OPM;

        self.attack_rate = 63;
        self.decay_rate = 0;
        self.sustain_rate = 0;
        self.release_rate = 63;
        self.sustain_level = 0;
        self.total_level = 0;

        self.key_scaling_rate = 1;
        self.key_scaling_level = 0;

        self.fine_multiple = 128;
        self.detune1 = 0;
        self.detune2 = 0;

        self.amplitude_modulation_shift = 0;
        self.initial_phase = 0;
        self.fixed_pitch = 0;

        self.mute = false;
        self.ssg_envelope_control = SSG_DISABLED;
        self.frequency_modulation_level = 5;
        self.envelope_reset_on_attack = false;
    }

    /// C++ `copy_from(const Ref<SiOPMOperatorParams> &)`.
    pub fn copy_from(&mut self, p_params: &OperatorParams) {
        self.pulse_generator_type = p_params.pulse_generator_type;
        self.pitch_table_type = p_params.pitch_table_type;

        self.attack_rate = p_params.attack_rate;
        self.decay_rate = p_params.decay_rate;
        self.sustain_rate = p_params.sustain_rate;
        self.release_rate = p_params.release_rate;
        self.sustain_level = p_params.sustain_level;
        self.total_level = p_params.total_level;

        self.key_scaling_rate = p_params.key_scaling_rate;
        self.key_scaling_level = p_params.key_scaling_level;

        self.fine_multiple = p_params.fine_multiple;
        self.detune1 = p_params.detune1;
        self.detune2 = p_params.detune2;

        self.amplitude_modulation_shift = p_params.amplitude_modulation_shift;
        self.initial_phase = p_params.initial_phase;
        self.fixed_pitch = p_params.fixed_pitch;

        self.mute = p_params.mute;
        self.ssg_envelope_control = p_params.ssg_envelope_control;
        self.frequency_modulation_level = p_params.frequency_modulation_level;
        self.envelope_reset_on_attack = p_params.envelope_reset_on_attack;
    }

    /// C++ `_to_string()`.
    pub fn to_string_repr(&self) -> String {
        let mut params = String::new();

        params += "pg=";
        params += &itos(self.pulse_generator_type as i64);
        params += ", ";
        params += "pt=";
        params += &itos(self.pitch_table_type as i64);
        params += ", ";

        params += "ar=";
        params += &itos(self.attack_rate as i64);
        params += ", ";
        params += "dr=";
        params += &itos(self.decay_rate as i64);
        params += ", ";
        params += "sr=";
        params += &itos(self.sustain_rate as i64);
        params += ", ";
        params += "rr=";
        params += &itos(self.release_rate as i64);
        params += ", ";
        params += "sl=";
        params += &itos(self.sustain_level as i64);
        params += ", ";
        params += "tl=";
        params += &itos(self.total_level as i64);
        params += ", ";

        params += "keyscale=(";
        params += &itos(self.key_scaling_rate as i64);
        params += ", ";
        params += &itos(self.key_scaling_level as i64);
        params += "), ";
        params += "fmul=";
        params += &itos(self.fine_multiple as i64);
        params += ", ";
        params += "detune=(";
        params += &itos(self.detune1 as i64);
        params += ", ";
        params += &itos(self.detune2 as i64);
        params += "), ";

        params += "amp=";
        params += &itos(self.amplitude_modulation_shift as i64);
        params += ", ";
        params += "phase=";
        params += &itos(self.initial_phase as i64);
        params += ", ";
        params += "note=";
        params += &itos(self.fixed_pitch as i64);
        params += ", ";

        params += "ssgec=";
        params += &itos(self.ssg_envelope_control as i64);
        params += ", ";
        params += "mute=";
        params += if self.mute { "yes" } else { "no" };
        params += ", ";
        params += "reset=";
        params += if self.envelope_reset_on_attack {
            "yes"
        } else {
            "no"
        };

        format!("SiOPMOperatorParams: {params}")
    }

    /// C++ `SiOPMOperatorParams()`.
    pub fn new() -> Self {
        let mut params = OperatorParams {
            pulse_generator_type: PULSE_SINE,
            pitch_table_type: PITCH_TABLE_OPM,

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
            initial_phase: 0,
            fixed_pitch: 0,

            mute: false,
            ssg_envelope_control: 0,
            frequency_modulation_level: 5,
            envelope_reset_on_attack: false,
        };
        params.initialize();
        params
    }
}

impl Default for OperatorParams {
    fn default() -> Self {
        Self::new()
    }
}
