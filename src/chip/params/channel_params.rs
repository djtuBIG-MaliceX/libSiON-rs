//! Port of `libSiON-cpp/src/chip/siopm_channel_params.{h,cpp}`.
//!
//! The C++ `List<Ref<SiOPMOperatorParams>>` is exactly `MAX_OPERATORS` long
//! at all times, so it becomes a `Vec<Rc<RefCell<OperatorParams>>>` of that
//! fixed length (shared, mutated via friend `TranslatorUtil` / wave-6
//! channels — hence `Rc<RefCell<_>>` per CONVENTIONS.md). `MMLSequence *`
//! becomes the ported `SeqRc` handle. `SiOPMSoundChip::STREAM_SEND_SIZE`
//! is re-exported here from its wave-6b home in `chip/sound_chip.rs`.

use std::cell::RefCell;
use std::rc::Rc;

use crate::chip::params::operator_params::OperatorParams;
use crate::chip::ref_table::{self, SiopmRefTable};
use crate::err_fail_cond;
use crate::err_fail_index;
use crate::err_fail_index_v;
use crate::sequencer::base::mml_sequence::{MMLSequence, SeqRc};
use crate::sion_enums::{PITCH_TABLE_OPM_NOISE, PULSE_NOISE_PULSE};
use crate::utils::string::{itos, rtos};

/// C++ `SiOPMChannelParams::MAX_OPERATORS`.
pub const MAX_OPERATORS: i32 = 4;
/// C++ `SiOPMSoundChip::STREAM_SEND_SIZE` (`siopm_sound_chip.h:43`).
/// Owned by `chip/sound_chip.rs` since wave-6b; re-exported so the
/// params-side public path stays unchanged.
pub use crate::chip::sound_chip::STREAM_SEND_SIZE;

/// C++ `SiOPMChannelParams`. Public fields mirror the C++ `friend` access
/// (`TranslatorUtil`) into the raw members.
pub struct ChannelParams {
    pub init_sequence: Option<SeqRc>,

    // This list is exactly MAX_OPERATORS at all times, use operator_count to
    // read only valid values.
    pub operator_params: Vec<Rc<RefCell<OperatorParams>>>,
    pub operator_count: i32,
    pub analog_like: bool,

    // Algorithm [0,15]
    pub algorithm: i32,
    // Feedback [0,7]
    pub feedback: i32,
    // Feedback connection [0,3]
    pub feedback_connection: i32,
    pub envelope_frequency_ratio: i32,
    pub lfo_wave_shape: i32,
    pub lfo_frequency_step: i32,

    pub amplitude_modulation_depth: i32,
    pub pitch_modulation_depth: i32,
    pub master_volumes: Vec<f64>,
    pub pan: i32,

    pub filter_type: i32,
    pub filter_cutoff: i32,
    pub filter_resonance: i32,
    pub filter_attack_rate: i32,
    pub filter_decay_rate1: i32,
    pub filter_decay_rate2: i32,
    pub filter_release_rate: i32,
    pub filter_decay_offset1: i32,
    pub filter_decay_offset2: i32,
    pub filter_sustain_offset: i32,
    pub filter_release_offset: i32,
}

impl ChannelParams {
    /// C++ `get_init_sequence()`.
    pub fn get_init_sequence(&self) -> Option<SeqRc> {
        self.init_sequence.clone()
    }

    /// C++ `get_operator_params(int)`.
    pub fn get_operator_params(&self, p_index: i32) -> Option<Rc<RefCell<OperatorParams>>> {
        err_fail_index_v!(p_index, "p_index", self.operator_count, "operator_count", None);

        Some(self.operator_params[p_index as usize].clone())
    }

    /// C++ `get_operator_count()`.
    pub fn get_operator_count(&self) -> i32 {
        self.operator_count
    }

    /// C++ `set_operator_count(int)`.
    pub fn set_operator_count(&mut self, p_value: i32) {
        err_fail_cond!(p_value > MAX_OPERATORS, "p_value > MAX_OPERATORS");

        self.operator_count = p_value;
    }

    /// C++ `is_analog_like()`.
    pub fn is_analog_like(&self) -> bool {
        self.analog_like
    }

    /// C++ `set_analog_like(bool)`.
    pub fn set_analog_like(&mut self, p_value: bool) {
        self.analog_like = p_value;
    }

    /// C++ `get_algorithm()`.
    pub fn get_algorithm(&self) -> i32 {
        self.algorithm
    }

    /// C++ `set_algorithm(int)`.
    pub fn set_algorithm(&mut self, p_value: i32) {
        self.algorithm = p_value;
    }

    /// C++ `get_feedback()`.
    pub fn get_feedback(&self) -> i32 {
        self.feedback
    }

    /// C++ `set_feedback(int)`.
    pub fn set_feedback(&mut self, p_value: i32) {
        self.feedback = p_value;
    }

    /// C++ `get_feedback_connection()`.
    pub fn get_feedback_connection(&self) -> i32 {
        self.feedback_connection
    }

    /// C++ `set_feedback_connection(int)`.
    pub fn set_feedback_connection(&mut self, p_value: i32) {
        self.feedback_connection = p_value;
    }

    /// C++ `get_envelope_frequency_ratio()`.
    pub fn get_envelope_frequency_ratio(&self) -> i32 {
        self.envelope_frequency_ratio
    }

    /// C++ `set_envelope_frequency_ratio(int)`.
    pub fn set_envelope_frequency_ratio(&mut self, p_value: i32) {
        self.envelope_frequency_ratio = p_value;
    }

    /// C++ `get_lfo_wave_shape()`.
    pub fn get_lfo_wave_shape(&self) -> i32 {
        self.lfo_wave_shape
    }

    /// C++ `set_lfo_wave_shape(int)`.
    pub fn set_lfo_wave_shape(&mut self, p_value: i32) {
        self.lfo_wave_shape = p_value;
    }

    /// C++ `get_lfo_frequency_step()`.
    pub fn get_lfo_frequency_step(&self) -> i32 {
        self.lfo_frequency_step
    }

    /// C++ `set_lfo_frequency_step(int)`.
    pub fn set_lfo_frequency_step(&mut self, p_value: i32) {
        self.lfo_frequency_step = p_value;
    }

    /// C++ `get_amplitude_modulation_depth()`.
    pub fn get_amplitude_modulation_depth(&self) -> i32 {
        self.amplitude_modulation_depth
    }

    /// C++ `set_amplitude_modulation_depth(int)`.
    pub fn set_amplitude_modulation_depth(&mut self, p_value: i32) {
        self.amplitude_modulation_depth = p_value;
    }

    /// C++ `has_amplitude_modulation()`.
    pub fn has_amplitude_modulation(&self) -> bool {
        self.amplitude_modulation_depth > 0
    }

    /// C++ `get_pitch_modulation_depth()`.
    pub fn get_pitch_modulation_depth(&self) -> i32 {
        self.pitch_modulation_depth
    }

    /// C++ `set_pitch_modulation_depth(int)`.
    pub fn set_pitch_modulation_depth(&mut self, p_value: i32) {
        self.pitch_modulation_depth = p_value;
    }

    /// C++ `has_pitch_modulation()`.
    pub fn has_pitch_modulation(&self) -> bool {
        self.pitch_modulation_depth > 0
    }

    /// C++ `get_master_volume(int)`.
    pub fn get_master_volume(&self, p_index: i32) -> f64 {
        err_fail_index_v!(
            p_index,
            "p_index",
            self.master_volumes.len() as i32,
            "master_volumes.size()",
            0.0
        );

        self.master_volumes[p_index as usize]
    }

    /// C++ `set_master_volume(int, double)`.
    pub fn set_master_volume(&mut self, p_index: i32, p_value: f64) {
        err_fail_index!(
            p_index,
            "p_index",
            self.master_volumes.len() as i32,
            "master_volumes.size()"
        );

        self.master_volumes[p_index as usize] = p_value;
    }

    /// C++ `get_pan()`.
    pub fn get_pan(&self) -> i32 {
        self.pan
    }

    /// C++ `set_pan(int)`.
    pub fn set_pan(&mut self, p_value: i32) {
        self.pan = p_value;
    }

    /// C++ `get_filter_type()`.
    pub fn get_filter_type(&self) -> i32 {
        self.filter_type
    }

    /// C++ `set_filter_type(int)`.
    pub fn set_filter_type(&mut self, p_value: i32) {
        self.filter_type = p_value;
    }

    /// C++ `get_filter_cutoff()`.
    pub fn get_filter_cutoff(&self) -> i32 {
        self.filter_cutoff
    }

    /// C++ `set_filter_cutoff(int)`.
    pub fn set_filter_cutoff(&mut self, p_value: i32) {
        self.filter_cutoff = p_value;
    }

    /// C++ `get_filter_resonance()`.
    pub fn get_filter_resonance(&self) -> i32 {
        self.filter_resonance
    }

    /// C++ `set_filter_resonance(int)`.
    pub fn set_filter_resonance(&mut self, p_value: i32) {
        self.filter_resonance = p_value;
    }

    /// C++ `get_filter_attack_rate()`.
    pub fn get_filter_attack_rate(&self) -> i32 {
        self.filter_attack_rate
    }

    /// C++ `set_filter_attack_rate(int)`.
    pub fn set_filter_attack_rate(&mut self, p_value: i32) {
        self.filter_attack_rate = p_value;
    }

    /// C++ `get_filter_decay_rate1()`.
    pub fn get_filter_decay_rate1(&self) -> i32 {
        self.filter_decay_rate1
    }

    /// C++ `set_filter_decay_rate1(int)`.
    pub fn set_filter_decay_rate1(&mut self, p_value: i32) {
        self.filter_decay_rate1 = p_value;
    }

    /// C++ `get_filter_decay_rate2()`.
    pub fn get_filter_decay_rate2(&self) -> i32 {
        self.filter_decay_rate2
    }

    /// C++ `set_filter_decay_rate2(int)`.
    pub fn set_filter_decay_rate2(&mut self, p_value: i32) {
        self.filter_decay_rate2 = p_value;
    }

    /// C++ `get_filter_release_rate()`.
    pub fn get_filter_release_rate(&self) -> i32 {
        self.filter_release_rate
    }

    /// C++ `set_filter_release_rate(int)`.
    pub fn set_filter_release_rate(&mut self, p_value: i32) {
        self.filter_release_rate = p_value;
    }

    /// C++ `get_filter_decay_offset1()`.
    pub fn get_filter_decay_offset1(&self) -> i32 {
        self.filter_decay_offset1
    }

    /// C++ `set_filter_decay_offset1(int)`.
    pub fn set_filter_decay_offset1(&mut self, p_value: i32) {
        self.filter_decay_offset1 = p_value;
    }

    /// C++ `get_filter_decay_offset2()`.
    pub fn get_filter_decay_offset2(&self) -> i32 {
        self.filter_decay_offset2
    }

    /// C++ `set_filter_decay_offset2(int)`.
    pub fn set_filter_decay_offset2(&mut self, p_value: i32) {
        self.filter_decay_offset2 = p_value;
    }

    /// C++ `get_filter_sustain_offset()`.
    pub fn get_filter_sustain_offset(&self) -> i32 {
        self.filter_sustain_offset
    }

    /// C++ `set_filter_sustain_offset(int)`.
    pub fn set_filter_sustain_offset(&mut self, p_value: i32) {
        self.filter_sustain_offset = p_value;
    }

    /// C++ `get_filter_release_offset()`.
    pub fn get_filter_release_offset(&self) -> i32 {
        self.filter_release_offset
    }

    /// C++ `set_filter_release_offset(int)`.
    pub fn set_filter_release_offset(&mut self, p_value: i32) {
        self.filter_release_offset = p_value;
    }

    /// C++ `has_filter()`.
    pub fn has_filter(&self) -> bool {
        self.filter_cutoff < 128 || self.filter_resonance > 0
    }

    /// C++ `has_filter_advanced()`.
    pub fn has_filter_advanced(&self) -> bool {
        self.filter_attack_rate > 0 || self.filter_release_rate > 0
    }

    /// C++ `get_lfo_frame()`.
    pub fn get_lfo_frame(&self) -> i32 {
        (SiopmRefTable::LFO_TIMER_INITIAL as f64 * 0.346938775510204
            / self.lfo_frequency_step as f64) as i32
    }

    /// C++ `set_lfo_frame(int)`.
    pub fn set_lfo_frame(&mut self, p_fps: i32) {
        self.lfo_frequency_step =
            (SiopmRefTable::LFO_TIMER_INITIAL as f64 / (p_fps as f64 * 2.882352941176471)) as i32;
    }

    /// C++ `set_by_opm_register(int, int, int)`.
    pub fn set_by_opm_register(&mut self, p_channel: i32, p_address: i32, p_data: i32) {
        if p_address < 0x20 {
            // Module parameter
            match p_address {
                15 => {
                    // NOIZE:7 FREQ:4-0 for channel#7
                    if p_channel == 7 && (p_data & 128) != 0 {
                        let mut op = self.operator_params[3].borrow_mut();
                        op.pulse_generator_type = PULSE_NOISE_PULSE;
                        op.pitch_table_type = PITCH_TABLE_OPM_NOISE;
                        op.fixed_pitch = ((p_data & 31) << 6) + 2048;
                    }
                }
                24 => {
                    // LFO FREQ:7-0 for all 8 channels
                    self.lfo_frequency_step =
                        ref_table::instance().borrow().lfo_timer_steps[p_data as usize];
                }
                25 => {
                    // A(0)/P(1):7 DEPTH:6-0 for all 8 channels
                    if (p_data & 128) != 0 {
                        self.pitch_modulation_depth = p_data & 127;
                    } else {
                        self.amplitude_modulation_depth = p_data & 127;
                    }
                }
                27 => {
                    // LFO WS:10 for all 8 channels
                    self.lfo_wave_shape = p_data & 3;
                }
                _ => {}
            }
        } else if p_channel == (p_address & 7) {
            if p_address < 0x40 {
                // Channel parameter
                match (p_address - 0x20) >> 3 {
                    0 => {
                        // L:7 R:6 FB:5-3 ALG:2-0
                        self.algorithm = p_data & 7;
                        self.feedback = (p_data >> 3) & 7;

                        let value = p_data >> 6;
                        self.master_volumes[0] = if value != 0 { 0.5 } else { 0.0 };
                        self.pan = if value == 1 {
                            128
                        } else if value == 2 {
                            0
                        } else {
                            64
                        };
                    }
                    1 => {} // KC:6-0
                    2 => {} // KF:6-0
                    3 => {} // PMS:6-4 AMS:10
                    _ => {}
                }
            } else {
                // Operator parameter
                let ops = [3, 1, 2, 0];
                let op_index = ops[((p_address >> 3) & 3) as usize];
                let op_params = self.operator_params[op_index].clone();

                match (p_address - 0x40) >> 5 {
                    0 => {
                        // DT1:6-4 MUL:3-0
                        let mut op = op_params.borrow_mut();
                        op.detune1 = (p_data >> 4) & 7;
                        op.set_multiple(p_data & 15);
                    }
                    1 => {
                        // TL:6-0
                        op_params.borrow_mut().total_level = p_data & 127;
                    }
                    2 => {
                        // KS:76 AR:4-0
                        let mut op = op_params.borrow_mut();
                        op.key_scaling_rate = (p_data >> 6) & 3;
                        op.attack_rate = (p_data & 31) << 1;
                    }
                    3 => {
                        // AMS:7 DR:4-0
                        let mut op = op_params.borrow_mut();
                        op.amplitude_modulation_shift = ((p_data >> 7) & 1) << 1;
                        op.decay_rate = (p_data & 31) << 1;
                    }
                    4 => {
                        // DT2:76 SR:4-0
                        let options = [0, 384, 500, 608];
                        let mut op = op_params.borrow_mut();
                        op.detune2 = options[((p_data >> 6) & 3) as usize];
                        op.sustain_rate = (p_data & 31) << 1;
                    }
                    5 => {
                        // SL:7-4 RR:3-0
                        let mut op = op_params.borrow_mut();
                        op.sustain_level = (p_data >> 4) & 15;
                        op.release_rate = (p_data & 15) << 2;
                    }
                    _ => {}
                }
            }
        }
    }

    /// C++ `initialize()`.
    pub fn initialize(&mut self) {
        self.operator_count = 1;

        self.algorithm = 0;
        self.feedback = 0;
        self.feedback_connection = 0;

        self.lfo_wave_shape = ref_table::LFO_WAVE_TRIANGLE as i32;
        self.lfo_frequency_step = 12126; // 12126 = 30 frames / 100 fratio

        self.amplitude_modulation_depth = 0;
        self.pitch_modulation_depth = 0;
        self.envelope_frequency_ratio = 100;

        for i in 1..STREAM_SEND_SIZE {
            self.master_volumes[i] = 0.0;
        }
        self.master_volumes[0] = 0.5;
        self.pan = 64;

        self.filter_type = 0;
        self.filter_cutoff = 128;
        self.filter_resonance = 0;
        self.filter_attack_rate = 0;
        self.filter_decay_rate1 = 0;
        self.filter_decay_rate2 = 0;
        self.filter_release_rate = 0;
        self.filter_decay_offset1 = 128;
        self.filter_decay_offset2 = 64;
        self.filter_sustain_offset = 32;
        self.filter_release_offset = 128;

        for op in &self.operator_params {
            op.borrow_mut().initialize();
        }

        if let Some(init_sequence) = &self.init_sequence {
            MMLSequence::clear(init_sequence);
        }
    }

    /// C++ `copy_from(const Ref<SiOPMChannelParams> &)`.
    pub fn copy_from(&mut self, p_params: &ChannelParams) {
        self.operator_count = p_params.operator_count;

        self.algorithm = p_params.algorithm;
        self.feedback = p_params.feedback;
        self.feedback_connection = p_params.feedback_connection;

        self.lfo_wave_shape = p_params.lfo_wave_shape;
        self.lfo_frequency_step = p_params.lfo_frequency_step;

        self.amplitude_modulation_depth = p_params.amplitude_modulation_depth;
        self.pitch_modulation_depth = p_params.pitch_modulation_depth;
        self.envelope_frequency_ratio = p_params.envelope_frequency_ratio;

        for i in 1..STREAM_SEND_SIZE {
            self.master_volumes[i] = p_params.master_volumes[i];
        }
        self.pan = p_params.pan;

        self.filter_type = p_params.filter_type;
        self.filter_cutoff = p_params.filter_cutoff;
        self.filter_resonance = p_params.filter_resonance;
        self.filter_attack_rate = p_params.filter_attack_rate;
        self.filter_decay_rate1 = p_params.filter_decay_rate1;
        self.filter_decay_rate2 = p_params.filter_decay_rate2;
        self.filter_release_rate = p_params.filter_release_rate;
        self.filter_decay_offset1 = p_params.filter_decay_offset1;
        self.filter_decay_offset2 = p_params.filter_decay_offset2;
        self.filter_sustain_offset = p_params.filter_sustain_offset;
        self.filter_release_offset = p_params.filter_release_offset;

        for i in 0..MAX_OPERATORS as usize {
            let source = p_params.operator_params[i].borrow();
            self.operator_params[i].borrow_mut().copy_from(&source);
        }

        if let Some(init_sequence) = &self.init_sequence {
            MMLSequence::clear(init_sequence);
        }
    }

    /// C++ `_to_string()`.
    pub fn to_string_repr(&self) -> String {
        let mut params = String::new();

        params += "ops=";
        params += &itos(self.operator_count as i64);
        params += ", ";
        params += "alg=";
        params += &itos(self.algorithm as i64);
        params += ", ";
        params += "feedback=(";
        params += &itos(self.feedback as i64);
        params += ", ";
        params += &itos(self.feedback_connection as i64);
        params += "), ";
        params += "fratio=";
        params += &itos(self.envelope_frequency_ratio as i64);
        params += ", ";

        let lfo_frequency =
            SiopmRefTable::LFO_TIMER_INITIAL as f64 * 0.005782313 / self.lfo_frequency_step as f64;
        params += "lfo=(";
        params += &itos(self.lfo_wave_shape as i64);
        params += ", ";
        params += &rtos(lfo_frequency);
        params += "), ";

        params += "amp=";
        params += &itos(self.amplitude_modulation_depth as i64);
        params += ", ";
        params += "pitch=";
        params += &itos(self.pitch_modulation_depth as i64);
        params += ", ";
        params += "vol=";
        params += &rtos(self.master_volumes[0]);
        params += ", ";
        params += "pan=";
        params += &itos((self.pan - 64) as i64);
        params += ", ";

        params += "filter=(";
        params += &itos(self.filter_type as i64);
        params += ", ";
        params += &itos(self.filter_cutoff as i64);
        params += ", ";
        params += &itos(self.filter_resonance as i64);
        params += "), ";
        params += "frate=(";
        params += &itos(self.filter_attack_rate as i64);
        params += ", ";
        params += &itos(self.filter_decay_rate1 as i64);
        params += ", ";
        params += &itos(self.filter_decay_rate2 as i64);
        params += ", ";
        params += &itos(self.filter_release_rate as i64);
        params += "), ";
        params += "foffset=(";
        params += &itos(self.filter_decay_offset1 as i64);
        params += ", ";
        params += &itos(self.filter_decay_offset2 as i64);
        params += ", ";
        params += &itos(self.filter_sustain_offset as i64);
        params += ", ";
        params += &itos(self.filter_release_offset as i64);
        params += ")";

        format!("SiOPMChannelParams: {params}")
    }

    /// C++ `SiOPMChannelParams()`.
    pub fn new() -> Self {
        let mut params = ChannelParams {
            init_sequence: Some(MMLSequence::new(false)),

            operator_params: (0..MAX_OPERATORS)
                .map(|_| Rc::new(RefCell::new(OperatorParams::new())))
                .collect(),
            operator_count: 0,
            analog_like: false,

            algorithm: 0,
            feedback: 0,
            feedback_connection: 0,
            envelope_frequency_ratio: 100,
            lfo_wave_shape: 0,
            lfo_frequency_step: 0,

            amplitude_modulation_depth: 0,
            pitch_modulation_depth: 0,
            master_volumes: vec![0.0; STREAM_SEND_SIZE],
            pan: 0,

            filter_type: 0,
            filter_cutoff: 0,
            filter_resonance: 0,
            filter_attack_rate: 0,
            filter_decay_rate1: 0,
            filter_decay_rate2: 0,
            filter_release_rate: 0,
            filter_decay_offset1: 0,
            filter_decay_offset2: 0,
            filter_sustain_offset: 0,
            filter_release_offset: 0,
        };
        params.initialize();
        params
    }
}

impl Default for ChannelParams {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chip::params::operator_params::{
        OperatorParams, SSG_DISABLED, SSG_IGNORE, SSG_MAX, SSG_REPEAT_SHUTTLE,
    };
    use crate::chip::ref_table;
    use crate::sion_enums::{PITCH_TABLE_OPM, PULSE_SINE};

    #[test]
    fn initialize_defaults_match_cpp() {
        let params = ChannelParams::new();

        assert_eq!(params.operator_count, 1);
        assert_eq!(params.operator_params.len(), MAX_OPERATORS as usize);
        assert_eq!(params.algorithm, 0);
        assert_eq!(params.envelope_frequency_ratio, 100);
        assert_eq!(params.lfo_wave_shape, ref_table::LFO_WAVE_TRIANGLE as i32);
        assert_eq!(params.lfo_frequency_step, 12126);
        assert_eq!(params.master_volumes[0], 0.5);
        assert_eq!(&params.master_volumes[1..], &[0.0; STREAM_SEND_SIZE - 1]);
        assert_eq!(params.pan, 64);
        assert_eq!(params.filter_cutoff, 128);
        assert_eq!(
            (
                params.filter_decay_offset1,
                params.filter_decay_offset2,
                params.filter_sustain_offset,
                params.filter_release_offset
            ),
            (128, 64, 32, 128)
        );

        let op = params.operator_params[0].borrow();
        assert_eq!(op.pulse_generator_type, PULSE_SINE);
        assert_eq!(op.pitch_table_type, PITCH_TABLE_OPM);
        assert_eq!(op.attack_rate, 63);
        assert_eq!(op.release_rate, 63);
        assert_eq!(op.key_scaling_rate, 1);
        assert_eq!(op.fine_multiple, 128);
        assert_eq!(op.frequency_modulation_level, 5);
    }

    #[test]
    fn lfo_frame_default_is_30() {
        // translator_util compares `get_lfo_frame() != 30` against the fresh
        // default step 12126 (= 30 frames / 100 fratio).
        let params = ChannelParams::new();
        assert_eq!(params.get_lfo_frame(), 30);
    }

    #[test]
    fn opm_register_and_operator_accessors_round_trip() {
        let mut params = ChannelParams::new();

        // ALG/feedback/L/R write (channel 0, address 0x20):
        // p_data = ALG 7 | FB 3<<3 | value 1<<6 -> pan 128, vol 0.5.
        params.set_by_opm_register(0, 0x20, 7 | (3 << 3) | (1 << 6));
        assert_eq!(params.algorithm, 7);
        assert_eq!(params.feedback, 3);
        assert_eq!(params.pan, 128);
        assert_eq!(params.master_volumes[0], 0.5);

        // DT1/MUL write for operator #0 selects op index 3 (ops map {3,1,2,0}):
        // p_data at 0x40, channel 0 -> (0x40>>3)&3 = 0 -> ops[0] = 3.
        params.set_by_opm_register(0, 0x40, (5 << 4) | 3);
        let op = params.operator_params[3].borrow();
        assert_eq!(op.detune1, 5);
        assert_eq!(op.get_multiple(), 3);
        assert_eq!(op.fine_multiple, 3 * 128);
        drop(op);

        // Range-clamped setter round-trips.
        let mut op = OperatorParams::new();
        op.set_ssg_envelope_control(SSG_MAX + 1);
        assert_eq!(op.get_ssg_envelope_control(), SSG_IGNORE);
        op.set_ssg_envelope_control(5);
        assert_eq!(op.get_ssg_envelope_control(), SSG_DISABLED);
        op.set_ssg_envelope_control(SSG_REPEAT_SHUTTLE);
        assert_eq!(op.get_ssg_envelope_control(), SSG_REPEAT_SHUTTLE);
        op.set_multiple(0);
        assert_eq!(op.fine_multiple, 64);
        assert_eq!(op.get_multiple(), 0);
    }
}
