//! `SiMMLRefTable` (`libSiON-cpp/src/sequencer/simml_ref_table.{h,cpp}`).
//!
//! Sequencer reference singleton: channel-module settings map, master/stencil
//! envelope & voice registries, TSSCP rate maps and the OPLL/VRC7 preset
//! voices. The `ALGORITHM_*` tables owned by the C++ header live here (moved
//! from `utils/translator_util.rs` — wave-7).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::sion_enums::{
    CHIP_OPL, MODULE_APU, MODULE_FM, MODULE_GB, MODULE_GENERIC_PG, MODULE_KS, MODULE_MA3,
    MODULE_NOISE, MODULE_PSG, MODULE_SAMPLE, MODULE_SCC, MODULE_SID, MODULE_VRC6, MODULE_RAMP,
    MODULE_PCM, MODULE_PULSE,
    PULSE_CUSTOM, PULSE_MA3_SINE, PULSE_NOISE_GB_SHORT, PULSE_NOISE_PULSE, PULSE_NOISE_SHORT,
    PULSE_NOISE_WHITE, PULSE_PCM, PULSE_PULSE, PULSE_RAMP, PULSE_SAW_UP, PULSE_SAW_VC6,
    PULSE_SINE, PULSE_SQUARE, PULSE_TRIANGLE, PULSE_TRIANGLE_FC, PULSE_PC_NZ_16BIT,
    PITCH_TABLE_APU_NOISE, PITCH_TABLE_GB_NOISE, PITCH_TABLE_OPM_NOISE, PITCH_TABLE_PSG,
    PITCH_TABLE_PSG_NOISE,
};
use crate::sequencer::channel_settings::{SelectToneType, SiMMLChannelSettings};
use crate::sequencer::envelope_table::SiMMLEnvelopeTable;
use crate::sequencer::voice::SiMMLVoice;
use crate::err_fail_index;
use crate::err_print;
use crate::utils::string::itos;

pub const ENVELOPE_TABLE_MAX: usize = 512;
pub const VOICE_MAX: usize = 256;

/// C++ `SiMMLRefTable::algorithm_opm`.
pub const ALGORITHM_OPM: [[i32; 16]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 0, -1, -1, -1, -1, -1, -1, -1, -1],
    [0, 1, 1, 1, 1, 0, 1, 1, -1, -1, -1, -1, -1, -1, -1, -1],
    [0, 1, 2, 3, 3, 4, 3, 5, -1, -1, -1, -1, -1, -1, -1, -1],
    [0, 1, 2, 3, 4, 5, 6, 7, -1, -1, -1, -1, -1, -1, -1, -1],
];

/// C++ `SiMMLRefTable::algorithm_opl`.
pub const ALGORITHM_OPL: [[i32; 16]; 4] = [
    [0, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1],
    [0, 1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1],
    [0, 3, 2, 2, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1],
    [0, 4, 8, 9, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1],
];

/// C++ `SiMMLRefTable::algorithm_ma3`.
pub const ALGORITHM_MA3: [[i32; 16]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 0, -1, -1, -1, -1, -1, -1, -1, -1],
    [0, 1, 1, 1, 0, 1, 1, 1, -1, -1, -1, -1, -1, -1, -1, -1],
    [-1, -1, 5, 2, 0, 3, 2, 2, -1, -1, -1, -1, -1, -1, -1, -1],
    [-1, -1, 7, 2, 0, 4, 8, 9, -1, -1, -1, -1, -1, -1, -1, -1],
];

/// C++ `SiMMLRefTable::algorithm_opx` (LSB is the feedback-connection flag).
pub const ALGORITHM_OPX: [[i32; 16]; 4] = [
    [0, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1],
    [0, 16, 1, 2, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1],
    [0, 16, 1, 2, 3, 19, 5, 6, -1, -1, -1, -1, -1, -1, -1, -1],
    [0, 16, 1, 2, 3, 19, 4, 20, 8, 11, 6, 22, 5, 9, 12, 7],
];

/// C++ `SiMMLRefTable::algorithm_init`.
pub const ALGORITHM_INIT: [i32; 4] = [0, 1, 5, 7];

thread_local! {
    static INSTANCE: RefCell<Option<Rc<RefCell<SiMMLRefTable>>>> = const { RefCell::new(None) };
}

/// C++ `get_instance()`; `None` = nullptr.
pub fn instance() -> Option<Rc<RefCell<SiMMLRefTable>>> {
    INSTANCE.with(|i| i.borrow().clone())
}

/// C++ `initialize()`.
pub fn initialize() {
    INSTANCE.with(|slot| {
        if slot.borrow().is_some() {
            return;
        }
        *slot.borrow_mut() = Some(Rc::new(RefCell::new(SiMMLRefTable::new())));
    });
}

/// C++ `finalize()`.
pub fn finalize() {
    INSTANCE.with(|slot| {
        *slot.borrow_mut() = None;
    });
}

pub struct SiMMLRefTable {
    master_envelopes: Vec<Option<Rc<RefCell<SiMMLEnvelopeTable>>>>,
    master_voices: Vec<Option<Rc<RefCell<SiMMLVoice>>>>,
    stencil_envelopes: Vec<Option<Rc<RefCell<SiMMLEnvelopeTable>>>>,
    stencil_voices: Vec<Option<Rc<RefCell<SiMMLVoice>>>>,

    pub channel_settings_map: HashMap<i32, Rc<RefCell<SiMMLChannelSettings>>>,

    pub tss_scmd_to_attack_rate: Vec<String>,
    pub tss_scmd_to_decay_rate: Vec<String>,
    pub tss_scmd_to_sustain_rate: Vec<String>,
    pub tss_scmd_to_release_rate: Vec<String>,

    pub preset_register_ym2413: [u32; 32],
    pub preset_register_vrc7: [u32; 32],
    pub preset_register_vrc7_drums: [u32; 6],
    pub preset_voice_ym2413: Vec<Rc<RefCell<SiMMLVoice>>>,
    pub preset_voice_vrc7: Vec<Rc<RefCell<SiMMLVoice>>>,
    pub preset_voice_vrc7_drums: Vec<Rc<RefCell<SiMMLVoice>>>,
}

fn fill_tss_log_table(r_table: &mut [String], p_start: i32, p_step: i32, p_v0: i32, p_v255: i32) {
    let mut value = p_start.wrapping_shl(16);
    let mut step = p_step.wrapping_shl(16);

    let mut i = 1usize;
    for j in 1..=8usize {
        let row_max = 1usize << j;
        while i < row_max {
            r_table[i] = itos((value >> 16) as i64);
            value = value.wrapping_add(step);
            i += 1;
        }
        step >>= 1;
    }

    r_table[0] = itos(p_v0 as i64);
    r_table[255] = itos(p_v255 as i64);
}

fn dump_ym2413_register(p_voice: &Rc<RefCell<SiMMLVoice>>, p_u0: u32, p_u1: u32) {
    let channel_params = p_voice.borrow().channel_params.clone();

    p_voice.borrow_mut().chip_type = CHIP_OPL;
    p_voice.borrow_mut().channel_num = 0;
    p_voice.borrow_mut().tone_num = -1;
    p_voice.borrow_mut().module_type = MODULE_FM;

    {
        let mut cp = channel_params.borrow_mut();
        cp.set_envelope_frequency_ratio(133);
        cp.set_operator_count(2);
        cp.set_algorithm(0);
    }
    let op0 = channel_params.borrow().get_operator_params(0).unwrap();
    let op1 = channel_params.borrow().get_operator_params(1).unwrap();

    op0.borrow_mut()
        .set_amplitude_modulation_shift((((p_u0 >> 31) & 1) as i32) << 1);
    op1.borrow_mut()
        .set_amplitude_modulation_shift((((p_u0 >> 23) & 1) as i32) << 1);
    op0.borrow_mut()
        .set_key_scaling_rate((((p_u0 >> 28) & 1) as i32) << 1);
    op1.borrow_mut()
        .set_key_scaling_rate((((p_u0 >> 20) & 1) as i32) << 1);

    let mut i = ((p_u0 >> 24) & 15) as i32;
    op0.borrow_mut().set_multiple(if i == 11 || i == 13 { i - 1 } else if i == 14 { i + 1 } else { i });
    i = ((p_u0 >> 16) & 15) as i32;
    op1.borrow_mut().set_multiple(if i == 11 || i == 13 { i - 1 } else if i == 14 { i + 1 } else { i });

    op0.borrow_mut()
        .set_key_scaling_level(((p_u0 >> 14) & 3) as i32);
    op1.borrow_mut()
        .set_key_scaling_level(((p_u0 >> 6) & 3) as i32);

    channel_params
        .borrow_mut()
        .set_feedback((p_u0 & 7) as i32);

    op0.borrow_mut()
        .set_pulse_generator_type(PULSE_MA3_SINE + ((p_u0 >> 3) & 1) as i32);
    op1.borrow_mut()
        .set_pulse_generator_type(PULSE_MA3_SINE + ((p_u0 >> 4) & 1) as i32);

    op0.borrow_mut()
        .set_attack_rate((((p_u1 >> 28) & 15) as i32) << 2);
    op1.borrow_mut()
        .set_attack_rate((((p_u1 >> 20) & 15) as i32) << 2);
    op0.borrow_mut()
        .set_decay_rate((((p_u1 >> 24) & 15) as i32) << 2);
    op1.borrow_mut()
        .set_decay_rate((((p_u1 >> 16) & 15) as i32) << 2);
    op0.borrow_mut()
        .set_sustain_level(((p_u1 >> 12) & 15) as i32);
    op1.borrow_mut()
        .set_sustain_level(((p_u1 >> 4) & 15) as i32);
    op0.borrow_mut()
        .set_release_rate((((p_u1 >> 8) & 15) as i32) << 2);
    op1.borrow_mut()
        .set_release_rate(((p_u1 & 15) as i32) << 2);
    let sr0 = op0.borrow().get_release_rate();
    op0.borrow_mut()
        .set_sustain_rate(if ((p_u0 >> 29) & 1) != 0 { 0 } else { sr0 });
    let sr1 = op1.borrow().get_release_rate();
    op1.borrow_mut()
        .set_sustain_rate(if ((p_u0 >> 21) & 1) != 0 { 0 } else { sr1 });
    op0.borrow_mut().set_total_level(((p_u0 >> 8) & 63) as i32);
    op1.borrow_mut().set_total_level(0);
}

fn setup_default_voices(register_map: &[u32]) -> Vec<Rc<RefCell<SiMMLVoice>>> {
    let count = register_map.len() / 2;
    let mut voices = Vec::with_capacity(count);
    for i in 0..count {
        let voice = Rc::new(RefCell::new(SiMMLVoice::new()));
        dump_ym2413_register(&voice, register_map[2 * i], register_map[2 * i + 1]);
        voices.push(voice);
    }
    voices
}

impl SiMMLRefTable {
    /// C++ `reset_all_user_tables`.
    pub fn reset_all_user_tables(&mut self) {
        for slot in self.master_envelopes.iter_mut() {
            if slot.is_some() {
                *slot = None;
            }
        }
        for slot in self.master_voices.iter_mut() {
            *slot = None;
        }
    }

    /// C++ `register_master_envelope_table`.
    pub fn register_master_envelope_table(
        &mut self,
        p_index: i32,
        p_table: Option<Rc<RefCell<SiMMLEnvelopeTable>>>,
    ) {
        err_fail_index!(p_index, "p_index", ENVELOPE_TABLE_MAX as i32, "ENVELOPE_TABLE_MAX");
        self.master_envelopes[p_index as usize] = p_table;
    }

    /// C++ `register_master_voice`.
    pub fn register_master_voice(
        &mut self,
        p_index: i32,
        p_voice: Option<Rc<RefCell<SiMMLVoice>>>,
    ) {
        err_fail_index!(p_index, "p_index", VOICE_MAX as i32, "VOICE_MAX");
        self.master_voices[p_index as usize] = p_voice;
    }

    /// C++ `set_stencil_envelopes`.
    pub fn set_stencil_envelopes(&mut self, p_tables: Vec<Option<Rc<RefCell<SiMMLEnvelopeTable>>>>) {
        self.stencil_envelopes = p_tables;
    }

    /// C++ `clear_stencil_envelopes`.
    pub fn clear_stencil_envelopes(&mut self) {
        self.stencil_envelopes = Vec::new();
    }

    /// C++ `set_stencil_voices`.
    pub fn set_stencil_voices(&mut self, p_tables: Vec<Option<Rc<RefCell<SiMMLVoice>>>>) {
        self.stencil_voices = p_tables;
    }

    /// C++ `clear_stencil_voices`.
    pub fn clear_stencil_voices(&mut self) {
        self.stencil_voices = Vec::new();
    }

    /// C++ `get_envelope_table`.
    pub fn get_envelope_table(&self, p_index: i32) -> Option<Rc<RefCell<SiMMLEnvelopeTable>>> {
        if p_index < 0 || p_index as usize >= ENVELOPE_TABLE_MAX {
            err_print!(
                "Index p_index = {} is out of bounds (ENVELOPE_TABLE_MAX = {}).",
                p_index,
                ENVELOPE_TABLE_MAX as i64
            );
            return None;
        }
        let p_index = p_index as usize;
        if p_index < self.stencil_envelopes.len() && self.stencil_envelopes[p_index].is_some() {
            return self.stencil_envelopes[p_index].clone();
        }
        self.master_envelopes[p_index].clone()
    }

    /// C++ `get_voice`.
    pub fn get_voice(&self, p_index: i32) -> Option<Rc<RefCell<SiMMLVoice>>> {
        if p_index < 0 || p_index as usize >= VOICE_MAX {
            err_print!(
                "Index p_index = {} is out of bounds (VOICE_MAX = {}).",
                p_index,
                VOICE_MAX as i64
            );
            return None;
        }
        let p_index = p_index as usize;
        if p_index < self.stencil_voices.len() && self.stencil_voices[p_index].is_some() {
            return self.stencil_voices[p_index].clone();
        }
        self.master_voices[p_index].clone()
    }

    /// C++ `get_pulse_generator_type`.
    pub fn get_pulse_generator_type(
        &self,
        p_module_type: i32,
        p_channel_num: i32,
        p_tone_num: i32,
    ) -> i32 {
        let settings = match self.channel_settings_map.get(&p_module_type) {
            Some(s) => s.clone(),
            None => {
                err_print!("Condition \"!channel_settings\" is true.");
                return -1;
            }
        };
        let settings = settings.borrow();

        if !settings.is_select_tone_type(SelectToneType::SelectToneNormal) {
            return -1;
        }

        let mut tone_num = p_tone_num;

        let voice_index_table = settings.get_voice_index_table();
        if tone_num == -1 && p_channel_num >= 0 && (p_channel_num as usize) < voice_index_table.len() {
            tone_num = voice_index_table[p_channel_num as usize];
        }

        let pg_type_list = settings.get_pg_type_list();
        if tone_num < 0 || (tone_num as usize) >= pg_type_list.len() {
            tone_num = settings.get_initial_voice_index();
        }

        pg_type_list[tone_num as usize]
    }

    /// C++ `is_suitable_for_fm_voice`.
    pub fn is_suitable_for_fm_voice(&self, p_module_type: i32) -> bool {
        match self.channel_settings_map.get(&p_module_type) {
            Some(s) => s.borrow().is_suitable_for_fm_voice(),
            None => {
                err_print!("Condition \"!channel_settings\" is true.");
                false
            }
        }
    }

    /// C++ ctor.
    fn new() -> Self {
        let mut channel_settings_map: HashMap<i32, Rc<RefCell<SiMMLChannelSettings>>> =
            HashMap::new();
        {
            channel_settings_map.insert(
                MODULE_PSG,
                Rc::new(RefCell::new(SiMMLChannelSettings::new(MODULE_PSG, PULSE_SQUARE, 3, 1, 4))),
            );
            channel_settings_map.insert(
                MODULE_APU,
                Rc::new(RefCell::new(SiMMLChannelSettings::new(MODULE_APU, PULSE_PULSE, 11, 2, 4))),
            );
            channel_settings_map.insert(
                MODULE_NOISE,
                Rc::new(RefCell::new(SiMMLChannelSettings::new(MODULE_NOISE,
                    PULSE_NOISE_WHITE,
                    16,
                    1,
                    16,
                ))),
            );
            channel_settings_map.insert(
                MODULE_MA3,
                Rc::new(RefCell::new(SiMMLChannelSettings::new(MODULE_MA3,
                    PULSE_MA3_SINE,
                    32,
                    1,
                    32,
                ))),
            );
            channel_settings_map.insert(
                MODULE_SCC,
                Rc::new(RefCell::new(SiMMLChannelSettings::new(MODULE_SCC,
                    PULSE_CUSTOM,
                    256,
                    1,
                    256,
                ))),
            );
            channel_settings_map.insert(
                MODULE_GENERIC_PG,
                Rc::new(RefCell::new(SiMMLChannelSettings::new(MODULE_GENERIC_PG, PULSE_SINE, 512, 1, 512))),
            );
            channel_settings_map.insert(
                MODULE_FM,
                Rc::new(RefCell::new(SiMMLChannelSettings::new(MODULE_FM, PULSE_SINE, 1, 1, 1))),
            );
            channel_settings_map.insert(
                MODULE_PCM,
                Rc::new(RefCell::new(SiMMLChannelSettings::new(MODULE_PCM, PULSE_PCM, 128, 1, 128))),
            );
            channel_settings_map.insert(
                MODULE_PULSE,
                Rc::new(RefCell::new(SiMMLChannelSettings::new(MODULE_PULSE, PULSE_PULSE, 32, 1, 32))),
            );
            channel_settings_map.insert(
                MODULE_RAMP,
                Rc::new(RefCell::new(SiMMLChannelSettings::new(MODULE_RAMP, PULSE_RAMP, 128, 1, 128))),
            );
            channel_settings_map.insert(
                MODULE_SAMPLE,
                Rc::new(RefCell::new(SiMMLChannelSettings::new(MODULE_SAMPLE, PULSE_SINE, 4, 1, 4))),
            );
            channel_settings_map.insert(
                MODULE_KS,
                Rc::new(RefCell::new(SiMMLChannelSettings::new(MODULE_KS, PULSE_SINE, 3, 1, 3))),
            );
            channel_settings_map.insert(
                MODULE_GB,
                Rc::new(RefCell::new(SiMMLChannelSettings::new(MODULE_GB, PULSE_PULSE, 11, 2, 4))),
            );
            channel_settings_map.insert(
                MODULE_VRC6,
                Rc::new(RefCell::new(SiMMLChannelSettings::new(MODULE_VRC6, PULSE_PULSE, 9, 1, 3))),
            );
            channel_settings_map.insert(
                MODULE_SID,
                Rc::new(RefCell::new(SiMMLChannelSettings::new(MODULE_SID, PULSE_PULSE, 12, 1, 3))),
            );

            {
                let cs = channel_settings_map[&MODULE_PSG].clone();
                let mut cs = cs.borrow_mut();
                cs.set_pg_type(0, PULSE_SQUARE);
                cs.set_pg_type(1, PULSE_NOISE_PULSE);
                cs.set_pg_type(2, PULSE_PC_NZ_16BIT);
                cs.set_pt_type(0, PITCH_TABLE_PSG);
                cs.set_pt_type(1, PITCH_TABLE_PSG_NOISE);
                cs.set_pt_type(2, PITCH_TABLE_PSG);
                cs.set_voice_index(0, 0);
                cs.set_voice_index(1, 0);
                cs.set_voice_index(2, 0);
                cs.set_voice_index(3, 1);
            }

            {
                let cs = channel_settings_map[&MODULE_APU].clone();
                let mut cs = cs.borrow_mut();
                cs.set_pg_type(8, PULSE_TRIANGLE_FC);
                cs.set_pg_type(9, PULSE_NOISE_PULSE);
                cs.set_pg_type(10, PULSE_NOISE_SHORT);
                for i in 0..9 {
                    cs.set_pt_type(i, PITCH_TABLE_PSG);
                }
                for i in 9..11 {
                    cs.set_pt_type(i, PITCH_TABLE_APU_NOISE);
                }
                cs.set_initial_voice_index(1);
                cs.set_voice_index(0, 4);
                cs.set_voice_index(1, 4);
                cs.set_voice_index(2, 8);
                cs.set_voice_index(3, 9);
            }

            {
                let cs = channel_settings_map[&MODULE_GB].clone();
                let mut cs = cs.borrow_mut();
                cs.set_pg_type(8, PULSE_CUSTOM);
                cs.set_pg_type(9, PULSE_NOISE_PULSE);
                cs.set_pg_type(10, PULSE_NOISE_GB_SHORT);
                for i in 0..9 {
                    cs.set_pt_type(i, PITCH_TABLE_PSG);
                }
                for i in 9..11 {
                    cs.set_pt_type(i, PITCH_TABLE_GB_NOISE);
                }
                cs.set_initial_voice_index(1);
                cs.set_voice_index(0, 4);
                cs.set_voice_index(1, 4);
                cs.set_voice_index(2, 8);
                cs.set_voice_index(3, 9);
            }

            {
                let cs = channel_settings_map[&MODULE_VRC6].clone();
                let mut cs = cs.borrow_mut();
                cs.set_pg_type(8, PULSE_SAW_VC6);
                cs.set_pt_type(8, PITCH_TABLE_PSG);
                cs.set_initial_voice_index(1);
                cs.set_voice_index(0, 7);
                cs.set_voice_index(1, 7);
                cs.set_voice_index(2, 8);
            }

            {
                let cs = channel_settings_map[&MODULE_SID].clone();
                let mut cs = cs.borrow_mut();
                cs.set_pg_type(8, PULSE_TRIANGLE);
                cs.set_pg_type(9, PULSE_SAW_UP);
                cs.set_pg_type(10, PULSE_SAW_VC6);
                cs.set_pg_type(11, PULSE_NOISE_PULSE);
                for i in 0..11 {
                    cs.set_pt_type(i, PITCH_TABLE_PSG);
                }
                cs.set_pt_type(11, PITCH_TABLE_OPM_NOISE);
                cs.set_initial_voice_index(1);
                cs.set_voice_index(0, 7);
                cs.set_voice_index(1, 7);
                cs.set_voice_index(2, 7);
            }

            {
                let cs = channel_settings_map[&MODULE_FM].clone();
                let mut cs = cs.borrow_mut();
                cs.set_select_tone_type(SelectToneType::SelectToneFm);
                cs.set_suitable_for_fm_voice(false);
            }

            {
                let cs = channel_settings_map[&MODULE_PCM].clone();
                let mut cs = cs.borrow_mut();
                cs.set_channel_type(crate::chip::channels::manager::ChannelType::Pcm);
                cs.set_suitable_for_fm_voice(false);
            }

            {
                let cs = channel_settings_map[&MODULE_SAMPLE].clone();
                let mut cs = cs.borrow_mut();
                cs.set_channel_type(crate::chip::channels::manager::ChannelType::Sampler);
                cs.set_suitable_for_fm_voice(false);
            }

            {
                let cs = channel_settings_map[&MODULE_KS].clone();
                let mut cs = cs.borrow_mut();
                cs.set_channel_type(crate::chip::channels::manager::ChannelType::Ks);
                cs.set_suitable_for_fm_voice(false);
            }
        }

        let preset_register_ym2413: [u32; 32] = [
            0x00000000, 0x00000000, 0x61611e17, 0xf07f0717, 0x13410f0d, 0xced24313, 0x03019904,
            0xffc30373, 0x21611b07, 0xaf634028, 0x22211e06, 0xf0760828, 0x31221605, 0x90710018,
            0x21611d07, 0x82811017, 0x23212d16, 0xc0700707, 0x61211b06, 0x64651818, 0x61610c18,
            0x85a07907, 0x23218711, 0xf0a400f7, 0x97e12807, 0xfff302f8, 0x61100c05, 0xf2c440c8,
            0x01015603, 0xb4b22358, 0x61418903, 0xf1f4f013,
        ];
        let preset_register_vrc7: [u32; 32] = [
            0x00000000, 0x00000000, 0x3301090e, 0x94904001, 0x13410f0d, 0xced34313, 0x01121b06,
            0xffd20032, 0x61611b07, 0xaf632028, 0x22211e06, 0xf0760828, 0x66211500, 0x939420f8,
            0x21611c07, 0x82811017, 0x2321201f, 0xc0710747, 0x25312605, 0x644118f8, 0x17212807,
            0xff8302f8, 0x97812507, 0xcfc80214, 0x2121540f, 0x807f0707, 0x01015603, 0xd3b24358,
            0x31210c03, 0x82c04007, 0x21010c03, 0xd4d34084,
        ];
        let preset_register_vrc7_drums: [u32; 6] = [
            0x04212800, 0xdff8fff8, 0x23220000, 0xd8f8f8f8, 0x25180000, 0xf8daf855,
        ];

        let preset_voice_ym2413 = setup_default_voices(&preset_register_ym2413);
        let preset_voice_vrc7 = setup_default_voices(&preset_register_vrc7);
        let preset_voice_vrc7_drums = setup_default_voices(&preset_register_vrc7_drums);

        let mut tss_scmd_to_attack_rate = vec![String::new(); 256];
        let mut tss_scmd_to_decay_rate = vec![String::new(); 256];
        let mut tss_scmd_to_sustain_rate = vec![String::new(); 256];
        let mut tss_scmd_to_release_rate = vec![String::new(); 256];
        fill_tss_log_table(&mut tss_scmd_to_attack_rate, 41, -4, 63, 9);
        fill_tss_log_table(&mut tss_scmd_to_decay_rate, 52, -4, 0, 20);
        fill_tss_log_table(&mut tss_scmd_to_sustain_rate, 9, 5, 0, 63);
        fill_tss_log_table(&mut tss_scmd_to_release_rate, 12, 4, 63, 63);

        SiMMLRefTable {
            master_envelopes: vec![None; ENVELOPE_TABLE_MAX],
            master_voices: vec![None; VOICE_MAX],
            stencil_envelopes: Vec::new(),
            stencil_voices: Vec::new(),
            channel_settings_map,
            tss_scmd_to_attack_rate,
            tss_scmd_to_decay_rate,
            tss_scmd_to_sustain_rate,
            tss_scmd_to_release_rate,
            preset_register_ym2413,
            preset_register_vrc7,
            preset_register_vrc7_drums,
            preset_voice_ym2413,
            preset_voice_vrc7,
            preset_voice_vrc7_drums,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chip::ref_table as chip_ref_table;
    use crate::chip::channels::manager::ChannelType;

    fn init() {
        chip_ref_table::initialize();
        initialize();
    }

    #[test]
    fn algorithm_tables_relocated() {
        init();
        assert_eq!(ALGORITHM_OPX[3][7], 20);
        assert_eq!(ALGORITHM_MA3[2][2], 5);
        assert_eq!(ALGORITHM_OPL[3][2], 8);
        assert_eq!(ALGORITHM_OPM[3][6], 6);
    }

    #[test]
    fn channel_type_map_and_lookup() {
        init();
        let rt = instance().unwrap();
        let rt = rt.borrow();
        assert_eq!(
            rt.channel_settings_map[&MODULE_PSG]
                .borrow()
                .get_channel_type(),
            ChannelType::Fm
        );
        assert_eq!(
            rt.channel_settings_map[&MODULE_PCM]
                .borrow()
                .get_channel_type(),
            ChannelType::Pcm
        );
        // PSG channel 1 -> voice_index_table[1] = 0 -> pg PULSE_SQUARE.
        assert_eq!(rt.get_pulse_generator_type(MODULE_PSG, 1, -1), PULSE_SQUARE);
        // PSG channel 3 -> voice_index_table[3] = 1 -> PULSE_NOISE_PULSE.
        assert_eq!(
            rt.get_pulse_generator_type(MODULE_PSG, 3, -1),
            PULSE_NOISE_PULSE
        );
        // FM selects tone by voice, not pg -> -1.
        assert_eq!(rt.get_pulse_generator_type(MODULE_FM, 0, -1), -1);
        assert!(!rt.is_suitable_for_fm_voice(MODULE_FM));
        assert!(rt.is_suitable_for_fm_voice(MODULE_GENERIC_PG));
    }

    #[test]
    fn voice_registry_stencil_over_master() {
        init();
        let rt = instance().unwrap();
        let voice = SiMMLVoice::create_blank_pcm_voice(0);
        {
            let mut b = rt.borrow_mut();
            b.register_master_voice(7, Some(voice.clone()));
            assert!(Rc::ptr_eq(
                &b.get_voice(7).unwrap(),
                &voice
            ));
            let out_of_range = b.get_voice(VOICE_MAX as i32);
            assert!(out_of_range.is_none());
            b.reset_all_user_tables();
            assert!(b.get_voice(7).is_none());
        }
    }
}
