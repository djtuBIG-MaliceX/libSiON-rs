//! Port of `libSiON-cpp/src/chip/siopm_ref_table.{h,cpp}`.
//!
//! Process-singleton reference table (EG/PG/wave/LFO/filter tables). DSP math
//! is frozen: evaluation order, `int` truncation points and wrapping
//! arithmetic are reproduced verbatim. The C++ `SiOPMRefTable::_instance`
//! static becomes the thread_local [`instance`] below; construction is lazy
//! like `SiOPMRefTable::initialize()`.
//!
//! Deferred (see `docs/PENDING.md`): the `_pcm_voices` / `_stencil_pcm_voices`
//! members and the `get_pcm_data` / `get_global_pcm_voice` /
//! `set_global_pcm_voice` API depend on `SiMMLVoice`
//! (`sequencer/simml_voice.{h,cpp}`), which lands with the sequencer wave.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::wave::pcm_table::SiopmWavePcmTable;
use super::wave::sampler_data::SiopmWaveSamplerData;
use super::wave::sampler_table::SiopmWaveSamplerTable;
use super::wave::table::SiopmWaveTable;
use crate::random::RandomNumberGenerator;
use crate::sample_data::SampleData;
use crate::sion_enums::{
    PITCH_TABLE_APU_NOISE, PITCH_TABLE_GB_NOISE, PITCH_TABLE_MAX, PITCH_TABLE_OPM,
    PITCH_TABLE_OPM_NOISE, PITCH_TABLE_PCM, PITCH_TABLE_PSG, PITCH_TABLE_PSG_NOISE, PULSE_CUSTOM,
    PULSE_KNM_BUBBLE, PULSE_MA3_SAW, PULSE_MA3_SAW_SINE, PULSE_MA3_SINE, PULSE_MA3_SQUARE,
    PULSE_MA3_SQUARE_HALF, PULSE_MA3_SQUARE_QUART, PULSE_MA3_SQUARE_QUART_DOUBLE,
    PULSE_MA3_TRI, PULSE_MA3_TRI_SINE, PULSE_MA3_USER1, PULSE_MA3_USER2, PULSE_MA3_USER3,
    PULSE_NOISE, PULSE_NOISE_GB_SHORT, PULSE_NOISE_HIPASS, PULSE_NOISE_PINK, PULSE_NOISE_PULSE,
    PULSE_NOISE_SHORT, PULSE_NOISE_WHITE, PULSE_PC_NZ_16BIT, PULSE_PC_NZ_OPM, PULSE_PC_NZ_SHORT,
    PULSE_PCM, PULSE_PULSE, PULSE_PULSE_SPIKE,     PULSE_RAMP,     PULSE_SAW_DOWN, PULSE_SAW_UP, PULSE_SAW_VC6, PULSE_SINE, PULSE_SQUARE,
    PULSE_SYNC_HIGH, PULSE_SYNC_LOW, PULSE_TRIANGLE, PULSE_TRIANGLE_FC,
};

// C++ VelocityMode.
pub const VM_LINEAR: usize = 0; // linear scale
pub const VM_DR96DB: usize = 1; // log scale; dynamic range = 96dB; total level based.
pub const VM_DR64DB: usize = 2; // log scale; dynamic range = 64dB; fmp7 based.
pub const VM_DR48DB: usize = 3; // log scale; dynamic range = 48dB; PSG volume based.
pub const VM_DR32DB: usize = 4; // log scale; dynamic range = 32dB; based on N88 basic v command.
pub const VM_MAX: usize = 5;

// C++ LFOWaveShape.
const LFO_WAVE_SAW: usize = 0;
const LFO_WAVE_SQUARE: usize = 1;
/// Exposed for `chip/params` (`SiOPMChannelParams::initialize`).
pub const LFO_WAVE_TRIANGLE: usize = 2;
const LFO_WAVE_NOISE: usize = 3;
pub const LFO_WAVE_MAX: usize = 8; // Values 4-7 are pairs for the first 0-3. (now pub: wave-6a `initialize_lfo`)

/// C++ `SiOPMRefTable::calculate_log_table_index` — pure static, no instance.
pub fn calculate_log_table_index(p_number: f64) -> i32 {
    // Original code suggests that the incoming number must be between -1 and 1, but this isn't true in practice.
    // ////ERR_FAIL_COND_V(p_number < -1 || p_number > 1, LOG_TABLE_BOTTOM);

    const LOG_COEFFICIENT: f64 = 369.3299304675746; // 369.3299304675746 = 256/log(2)
    const LOG_THRESHOLD: f64 = 0.0001220703125; // 0.0001220703125 = 1/(2^13)

    if p_number < 0.0 {
        if p_number < -LOG_THRESHOLD {
            (((crate::math::ln(-p_number) * -LOG_COEFFICIENT + 0.5) as i32 + 1) << 1) + 1
        } else {
            SiopmRefTable::LOG_TABLE_BOTTOM
        }
    } else if p_number > LOG_THRESHOLD {
        ((crate::math::ln(p_number) * -LOG_COEFFICIENT + 0.5) as i32 + 1) << 1
    } else {
        SiopmRefTable::LOG_TABLE_BOTTOM
    }
}

thread_local! {
    static INSTANCE: RefCell<Option<Rc<RefCell<SiopmRefTable>>>> = const { RefCell::new(None) };
}

/// C++ `SiOPMRefTable::get_instance()` / `initialize()` combined: lazily
/// creates the singleton with the C++ default ctor arguments
/// `(3580000, 1789772.5, 44100)`.
pub fn instance() -> Rc<RefCell<SiopmRefTable>> {
    INSTANCE.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Some(Rc::new(RefCell::new(SiopmRefTable::new(
                3580000,
                1789772.5,
                44100,
            ))));
        }
        slot.as_ref().unwrap().clone()
    })
}

/// C++ `SiOPMRefTable::initialize()`.
pub fn initialize() {
    let _ = instance();
}

/// C++ `SiOPMRefTable::finalize()`.
pub fn finalize() {
    INSTANCE.with(|cell| *cell.borrow_mut() = None);
}

pub struct SiopmRefTable {
    // Wave samples.

    // Custom wave tables.
    custom_wave_tables: Vec<Option<Rc<RefCell<SiopmWaveTable>>>>,
    // Overriding custom wave tables.
    stencil_custom_wave_tables: Vec<Option<Rc<RefCell<SiopmWaveTable>>>>,
    /// C++ `_pcm_voices` (`Vec<Ref<SiMMLVoice>>`).
    pcm_voices: Vec<Option<Rc<RefCell<crate::sequencer::voice::SiMMLVoice>>>>,
    // Overriding PCM voices.
    stencil_pcm_voices: Vec<Option<Rc<RefCell<crate::sequencer::voice::SiMMLVoice>>>>,

    //

    // Wave tables.
    pub wave_tables: Vec<Option<Rc<RefCell<SiopmWaveTable>>>>,
    pub no_wave_table: Option<Rc<RefCell<SiopmWaveTable>>>,
    pub no_wave_table_opm: Option<Rc<RefCell<SiopmWaveTable>>>,
    // PG sampler table.
    pub sampler_tables: Vec<Rc<RefCell<SiopmWaveSamplerTable>>>,

    // All reference properties are made public to simplify code.
    // These are not expected to be written to externally. But that might happen.

    pub sampling_rate: i32,
    pub fm_clock: i32,
    pub psg_clock: f64,
    // (fm_clock/64/sampling_rate) << CLOCK_RATIO_BITS
    pub clock_ratio: i32,
    // 44100Hz=0, 22050Hz=1
    pub sample_rate_pitch_shift: i32,

    // int->double ratio on pulse data
    pub i2n: f64,

    // Envelope generator.

    // EG increment table. This table is based on MAME's opm emulation.
    pub eg_increment_tables: [[i32; 8]; 18],
    // EG increment table for attack. This table is based on fmgen (shift=0 means x0).
    pub eg_increment_tables_attack: [[i32; 8]; 18],
    // EG table selector. 128 = 64 rates + 32 ks-rates + 32 dummies for dr,sr=0
    pub eg_table_selector: [i32; 128],
    // EG timer step. 128 = 64 rates + 32 ks-rates + 32 dummies for dr,sr=0
    pub eg_timer_steps: [i32; 128],
    // EG table to calculate EG level tables.
    // [7][1 << ENV_BITS]
    pub eg_level_tables: [[i32; 1024]; 7],
    // EG table for SSG-type to EG level tables index. 10 = 8 standard + 2 extra.
    pub eg_ssg_table_index: [[[i32; 3]; 2]; 10],
    // EG sustain level table from 15 to 1024.
    pub eg_sustain_level_table: [i32; 16],
    // EG total level table from volume to tl.
    // [VM_MAX][TL_TABLE_SIZE]
    pub eg_total_level_tables: [[i32; 513]; VM_MAX],
    // EG conversion table from linear volume to total level.
    pub eg_linear_to_total_level_table: [i32; 129],

    // Panning volume table.
    pub pan_table: [f64; 129],

    // Low frequency oscillator.

    // LFO timer step. [LFO_TABLE_SIZE]
    pub lfo_timer_steps: [i32; 256],
    // LFO modulation table. [LFO_WAVE_MAX][LFO_TABLE_SIZE]
    pub lfo_wave_tables: [[i32; 256]; LFO_WAVE_MAX],
    // LFO modulation table for chorus. [LFO_TABLE_SIZE]
    pub lfo_chorus_tables: [i32; 256],

    // Filter.

    // FILTER cutoff.
    pub filter_cutoff_table: [f64; 129],
    // FILTER resonance.
    pub filter_feedback_table: [f64; 129],
    // FILTER envlope rate.
    pub filter_eg_rate: [i32; 64],

    // Pulse generator.

    // PG MIDI note number to FM key code. [NOTE_TABLE_SIZE]
    pub note_number_to_key_code: [i32; 128],

    // PG pitch table.
    pub pitch_table: Vec<Vec<i32>>,
    // PG pitch wave length (in samples) table. [PITCH_TABLE_SIZE]
    pub pitch_wave_length: [f64; 8192],
    // PG phase step shift filter.
    pub phase_step_shift_filter: [i32; PITCH_TABLE_MAX],
    // PG sound reference table.
    pub sound_reference: HashMap<String, Rc<RefCell<SampleData>>>,

    // Table for dt1 (from fmgen.cpp). [8][KEY_CODE_TABLE_SIZE]
    pub dt1_table: [[i32; 128]; 8],
    // Table for dt2 (from MAME's opm source).
    pub dt2_table: [i32; 4],
    // PG log table. (2 extra units of size for zero-filling)
    pub log_table: Vec<i32>,
}

impl SiopmRefTable {
    pub const ENV_BITS: i32 = 10; // Envelope output bit size.
    pub const ENV_TIMER_BITS: i32 = 24; // Envelope timer resolution bit size.
    pub const SAMPLING_TABLE_BITS: i32 = 10; // Sine wave table entries = 2 ^ SAMPLING_TABLE_BITS = 1024
    pub const HALF_TONE_BITS: i32 = 6; // Half tone resolution    = 2 ^ HALF_TONE_BITS      = 64
    pub const NOTE_BITS: i32 = 7; // Max note value          = 2 ^ NOTE_BITS           = 128
    pub const NOISE_TABLE_BITS: i32 = 15; // 32k noise
    pub const LOG_TABLE_RESOLUTION: i32 = 256; // Log table resolution    = LOG_TABLE_RESOLUTION for every 1/2 scaling.
    pub const LOG_VOLUME_BITS: i32 = 13; // _logTable[0] = 2^13 at maximum
    pub const LOG_TABLE_MAX_BITS: i32 = 16; // _logTable entries
    pub const FIXED_BITS: i32 = 16; // Internal fixed point 16.16
    pub const PCM_BITS: i32 = 20; // Maximum PCM sample length = 2 ^ PCM_BITS = 1048576
    pub const LFO_FIXED_BITS: i32 = 20; // Fixed point for lfo timer
    pub const CLOCK_RATIO_BITS: i32 = 10; // Bits for clock/64/[sampling rate]

    // -2044 < [noise amplitude] < 2040 -> NOISE_WAVE_OUTPUT=0.25
    pub const NOISE_WAVE_OUTPUT: f64 = 1.0;
    pub const SQUARE_WAVE_OUTPUT: f64 = 1.0;
    // Maximum output
    pub const OUTPUT_MAX: f64 = 0.5;

    // Shift number from input tl [0,127] to internal value [0,ENV_BOTTOM].
    pub const ENV_LSHIFT: i32 = Self::ENV_BITS - 7;
    // Envelope timer initial value.
    pub const ENV_TIMER_INITIAL: i32 = (2047 * 3) << Self::CLOCK_RATIO_BITS;
    // LFO timer initial value.
    pub const LFO_TIMER_INITIAL: i32 = 1 << Self::LFO_FIXED_BITS;
    // Internal phase is expressed by 10.16 fixed.
    pub const PHASE_BITS: i32 = Self::SAMPLING_TABLE_BITS + Self::FIXED_BITS;
    pub const PHASE_MAX: i32 = 1 << Self::PHASE_BITS;
    pub const PHASE_FILTER: i32 = Self::PHASE_MAX - 1;
    pub const PHASE_SIGN_RSHIFT: i32 = Self::PHASE_BITS - 1;
    pub const SAMPLING_TABLE_SIZE: usize = 1 << Self::SAMPLING_TABLE_BITS;
    pub const NOISE_TABLE_SIZE: usize = 1 << Self::NOISE_TABLE_BITS;
    pub const PITCH_TABLE_SIZE: usize = 1 << (Self::HALF_TONE_BITS + Self::NOTE_BITS);
    pub const NOTE_TABLE_SIZE: usize = 1 << Self::NOTE_BITS;
    pub const HALF_TONE_RESOLUTION: i32 = 1 << Self::HALF_TONE_BITS;
    // *2 posi&nega
    pub const LOG_TABLE_SIZE: usize =
        (Self::LOG_TABLE_MAX_BITS as usize) * (Self::LOG_TABLE_RESOLUTION as usize) * 2;
    // FIXED VALUE !!
    pub const LFO_TABLE_SIZE: usize = 256;
    pub const TL_TABLE_SIZE: usize = 513;
    // FIXED VALUE !!
    pub const KEY_CODE_TABLE_SIZE: usize = 128;
    // Bottom value of log table = 6656
    pub const LOG_TABLE_BOTTOM: i32 =
        Self::LOG_VOLUME_BITS * Self::LOG_TABLE_RESOLUTION * 2;
    // Minimum gain of envelope = 832
    pub const ENV_BOTTOM: i32 = (Self::LOG_VOLUME_BITS * Self::LOG_TABLE_RESOLUTION) >> 2;
    // Maximum gain of envelope = -192
    pub const ENV_TOP: i32 = Self::ENV_BOTTOM - (1 << Self::ENV_BITS);
    // Minimum gain of ssgec envelope = 128
    pub const ENV_BOTTOM_SSGEC: i32 = 1 << (Self::ENV_BITS - 3);

    // Maximum value of default pulse generator types.
    pub const DEFAULT_PG_MAX: usize = 256;
    // Maximum index from predefined pulse generator type values.
    pub const PG_FILTER: i32 = 511;

    // Custom wave table max.
    pub const WAVE_TABLE_MAX: usize = 128;
    // PCM data max.
    pub const PCM_DATA_MAX: usize = 128;
    // Sampler table max
    pub const SAMPLER_TABLE_MAX: usize = 4;
    // Sampler data max
    pub const SAMPLER_DATA_MAX: i32 = Self::NOTE_TABLE_SIZE as i32;

    /// C++ `SiOPMRefTable(int p_fm_clock = 3580000, double p_psg_clock =
    /// 1789772.5, int p_sampling_rate = 44100)`.
    pub fn new(p_fm_clock: i32, p_psg_clock: f64, p_sampling_rate: i32) -> Self {
        let mut table = SiopmRefTable {
            custom_wave_tables: Vec::new(),
            stencil_custom_wave_tables: Vec::new(),
            pcm_voices: Vec::new(),
            stencil_pcm_voices: Vec::new(),
            wave_tables: Vec::new(),
            no_wave_table: None,
            no_wave_table_opm: None,
            sampler_tables: Vec::new(),

            sampling_rate: 0,
            fm_clock: 0,
            psg_clock: 0.0,
            clock_ratio: 1,
            sample_rate_pitch_shift: 0,
            i2n: Self::OUTPUT_MAX / ((1 << Self::LOG_VOLUME_BITS) as f64),

            eg_increment_tables: [
                /* 0*/ [0, 1, 0, 1, 0, 1, 0, 1],
                /* 1*/ [0, 1, 0, 1, 1, 1, 0, 1],
                /* 2*/ [0, 1, 1, 1, 0, 1, 1, 1],
                /* 3*/ [0, 1, 1, 1, 1, 1, 1, 1],
                /* 4*/ [1, 1, 1, 1, 1, 1, 1, 1],
                /* 5*/ [1, 1, 1, 2, 1, 1, 1, 2],
                /* 6*/ [1, 2, 1, 2, 1, 2, 1, 2],
                /* 7*/ [1, 2, 2, 2, 1, 2, 2, 2],
                /* 8*/ [2, 2, 2, 2, 2, 2, 2, 2],
                /* 9*/ [2, 2, 2, 4, 2, 2, 2, 4],
                /*10*/ [2, 4, 2, 4, 2, 4, 2, 4],
                /*11*/ [2, 4, 4, 4, 2, 4, 4, 4],
                /*12*/ [4, 4, 4, 4, 4, 4, 4, 4],
                /*13*/ [4, 4, 4, 8, 4, 4, 4, 8],
                /*14*/ [4, 8, 4, 8, 4, 8, 4, 8],
                /*15*/ [4, 8, 8, 8, 4, 8, 8, 8],
                /*16*/ [8, 8, 8, 8, 8, 8, 8, 8],
                /*17*/ [0, 0, 0, 0, 0, 0, 0, 0],
            ],
            eg_increment_tables_attack: [
                /* 0*/ [0, 4, 0, 4, 0, 4, 0, 4],
                /* 1*/ [0, 4, 0, 4, 4, 4, 0, 4],
                /* 2*/ [0, 4, 4, 4, 0, 4, 4, 4],
                /* 3*/ [0, 4, 4, 4, 4, 4, 4, 4],
                /* 4*/ [4, 4, 4, 4, 4, 4, 4, 4],
                /* 5*/ [4, 4, 4, 3, 4, 4, 4, 3],
                /* 6*/ [4, 3, 4, 3, 4, 3, 4, 3],
                /* 7*/ [4, 3, 3, 3, 4, 3, 3, 3],
                /* 8*/ [3, 3, 3, 3, 3, 3, 3, 3],
                /* 9*/ [3, 3, 3, 2, 3, 3, 3, 2],
                /*10*/ [3, 2, 3, 2, 3, 2, 3, 2],
                /*11*/ [3, 2, 2, 2, 3, 2, 2, 2],
                /*12*/ [2, 2, 2, 2, 2, 2, 2, 2],
                /*13*/ [2, 2, 2, 1, 2, 2, 2, 1],
                /*14*/ [2, 8, 2, 1, 2, 1, 2, 1],
                /*15*/ [2, 1, 1, 1, 2, 1, 1, 1],
                /*16*/ [1, 1, 1, 1, 1, 1, 1, 1],
                /*17*/ [0, 0, 0, 0, 0, 0, 0, 0],
            ],
            eg_table_selector: [0; 128],
            eg_timer_steps: [0; 128],
            eg_level_tables: [[0; 1 << Self::ENV_BITS]; 7],
            // [w/ ar], [w/o ar]
            eg_ssg_table_index: [
                [[3, 3, 3], [1, 3, 3]], // ssgec=8
                [[1, 6, 6], [1, 6, 6]], // ssgec=9
                [[2, 1, 2], [1, 2, 1]], // ssgec=10
                [[2, 5, 5], [1, 5, 5]], // ssgec=11
                [[4, 4, 4], [2, 4, 4]], // ssgec=12
                [[2, 5, 5], [2, 5, 5]], // ssgec=13
                [[1, 2, 1], [2, 1, 2]], // ssgec=14
                [[1, 6, 6], [2, 6, 6]], // ssgec=15
                [[1, 1, 1], [1, 1, 1]], // ssgec=16
                [[2, 2, 2], [2, 2, 2]], // ssgec=17
            ],
            eg_sustain_level_table: [0; 16],
            eg_total_level_tables: [[0; Self::TL_TABLE_SIZE]; VM_MAX],
            eg_linear_to_total_level_table: [0; 129],

            pan_table: [0.0; 129],

            lfo_timer_steps: [0; Self::LFO_TABLE_SIZE],
            lfo_wave_tables: [[0; Self::LFO_TABLE_SIZE]; LFO_WAVE_MAX],
            lfo_chorus_tables: [0; Self::LFO_TABLE_SIZE],

            filter_cutoff_table: [0.0; 129],
            filter_feedback_table: [0.0; 129],
            filter_eg_rate: [0; 64],

            note_number_to_key_code: [0; Self::NOTE_TABLE_SIZE],

            pitch_table: Vec::new(),
            pitch_wave_length: [0.0; Self::PITCH_TABLE_SIZE],
            phase_step_shift_filter: [0; PITCH_TABLE_MAX],
            sound_reference: HashMap::new(),

            dt1_table: [[0; Self::KEY_CODE_TABLE_SIZE]; 8],
            dt2_table: [0, 384, 500, 608],
            // 16*256*2*3 = 24576
            log_table: vec![0; Self::LOG_TABLE_SIZE * 3],
        };

        table.set_constants(p_fm_clock, p_psg_clock, p_sampling_rate);

        table.create_eg_tables();
        table.create_pg_tables();
        table.create_wave_samples();
        table.create_lfo_tables();
        table.create_filter_tables();

        table
    }

    //

    pub fn reset_all_user_tables(&mut self) {
        for i in 0..Self::WAVE_TABLE_MAX {
            self.custom_wave_tables[i] = None;
        }

        for i in 0..Self::PCM_DATA_MAX {
            if let Some(voice) = self.pcm_voices[i].take() {
                if let Some(wave) = &voice.borrow().wave_data {
                    if let Some(pcm_table) =
                        wave.downcast_ref::<Rc<RefCell<SiopmWavePcmTable>>>()
                    {
                        pcm_table.borrow_mut().clear();
                    }
                }
            }
        }

        self.stencil_custom_wave_tables.clear();
        self.stencil_pcm_voices.clear();
    }

    pub fn register_wave_table(
        &mut self,
        p_index: i32,
        p_table: &Option<Rc<RefCell<SiopmWaveTable>>>,
    ) {
        let index = (p_index & (Self::WAVE_TABLE_MAX as i32 - 1)) as usize;
        self.custom_wave_tables[index] = p_table.clone();

        // MA-3 waveforms support up to 3 user defined values. If this table is one of the first
        // 3 custom tables, use it.
        if index < 3 {
            // User defined waves are at offsets 15,23,31.
            self.wave_tables[(PULSE_MA3_SINE + 15 + index as i32 * 8) as usize] =
                p_table.clone();
        }
    }

    pub fn register_sampler_data(
        &mut self,
        p_index: i32,
        p_data: Option<&Rc<RefCell<SampleData>>>,
        p_ignore_note_off: bool,
        p_pan: i32,
        p_src_channel_count: i32,
        p_channel_count: i32,
    ) -> Rc<RefCell<SiopmWaveSamplerData>> {
        let nil = SampleData::Nil;
        let data_ref = match p_data {
            Some(d) => d.borrow().clone(),
            None => nil,
        };
        let sampler_data = Rc::new(RefCell::new(SiopmWaveSamplerData::new(
            &data_ref,
            p_ignore_note_off,
            p_pan,
            p_src_channel_count,
            p_channel_count,
        )));

        let bank = ((p_index >> Self::NOTE_BITS) & (Self::SAMPLER_TABLE_MAX as i32 - 1)) as usize;
        self.sampler_tables[bank]
            .borrow_mut()
            .set_sample(&Some(sampler_data.clone()), p_index & (Self::SAMPLER_DATA_MAX - 1), -1);

        sampler_data
    }

    pub fn get_wave_table(&self, p_index: i32) -> Option<Rc<RefCell<SiopmWaveTable>>> {
        if p_index < PULSE_CUSTOM {
            if p_index < 0 || p_index as usize >= self.wave_tables.len() {
                // ERR_FAIL_INDEX_V(p_index, wave_tables.size(), no_wave_table);
                crate::error::err_print_body(
                    &format!(
                        "Index p_index = {} is out of bounds (wave_tables.size() = {}).",
                        p_index,
                        self.wave_tables.len()
                    ),
                    false,
                );
                return self.no_wave_table.clone();
            }
            return self.wave_tables[p_index as usize].clone();
        }
        if p_index < PULSE_PCM {
            let table_index = (p_index - PULSE_CUSTOM) as usize;

            if table_index < self.stencil_custom_wave_tables.len()
                && self.stencil_custom_wave_tables[table_index].is_some()
            {
                return self.stencil_custom_wave_tables[table_index].clone();
            }

            if table_index < self.custom_wave_tables.len()
                && self.custom_wave_tables[table_index].is_some()
            {
                return self.custom_wave_tables[table_index].clone();
            }

            return self.no_wave_table_opm.clone();
        }

        self.no_wave_table.clone()
    }

    /// C++ `get_pcm_data`.
    pub fn get_pcm_data(&self, p_index: i32) -> Option<Rc<RefCell<SiopmWavePcmTable>>> {
        let table_index = (p_index & (Self::PCM_DATA_MAX as i32 - 1)) as usize;

        if table_index < self.stencil_pcm_voices.len() && self.stencil_pcm_voices[table_index].is_some()
        {
            let voice = self.stencil_pcm_voices[table_index].as_ref().unwrap().borrow();
            return voice
                .wave_data
                .as_ref()
                .and_then(|w| w.downcast_ref::<Rc<RefCell<SiopmWavePcmTable>>>())
                .cloned();
        }

        if table_index < self.pcm_voices.len() && self.pcm_voices[table_index].is_some() {
            let voice = self.pcm_voices[table_index].as_ref().unwrap().borrow();
            return voice
                .wave_data
                .as_ref()
                .and_then(|w| w.downcast_ref::<Rc<RefCell<SiopmWavePcmTable>>>())
                .cloned();
        }

        None
    }

    /// C++ `get_global_pcm_voice`.
    pub fn get_global_pcm_voice(
        &mut self,
        p_index: i32,
    ) -> Rc<RefCell<crate::sequencer::voice::SiMMLVoice>> {
        use crate::sequencer::voice::SiMMLVoice;
        let index = (p_index & (Self::PCM_DATA_MAX as i32 - 1)) as usize;
        if self.pcm_voices[index].is_none() {
            self.pcm_voices[index] = Some(SiMMLVoice::create_blank_pcm_voice(index as i32));
        }
        self.pcm_voices[index].clone().unwrap()
    }

    /// C++ `set_global_pcm_voice`.
    pub fn set_global_pcm_voice(
        &mut self,
        p_index: i32,
        p_from_voice: &Rc<RefCell<crate::sequencer::voice::SiMMLVoice>>,
    ) -> Rc<RefCell<crate::sequencer::voice::SiMMLVoice>> {
        use crate::sequencer::voice::SiMMLVoice;
        let index = (p_index & (Self::PCM_DATA_MAX as i32 - 1)) as usize;
        if self.pcm_voices[index].is_none() {
            self.pcm_voices[index] = Some(Rc::new(RefCell::new(SiMMLVoice::new())));
        }
        let voice = self.pcm_voices[index].clone().unwrap();
        voice.borrow_mut().copy_from(p_from_voice);
        voice
    }

    /// C++ `set_stencil_pcm_voices`.
    pub fn set_stencil_pcm_voices(
        &mut self,
        p_voices: Vec<Option<Rc<RefCell<crate::sequencer::voice::SiMMLVoice>>>>,
    ) {
        self.stencil_pcm_voices = p_voices;
    }

    /// C++ `clear_stencil_pcm_voices`.
    pub fn clear_stencil_pcm_voices(&mut self) {
        self.stencil_pcm_voices.clear();
    }

    pub fn set_sampler_table_stencil(
        &mut self,
        p_index: i32,
        p_table: &Option<Rc<RefCell<SiopmWaveSamplerTable>>>,
    ) {
        // ERR_FAIL_INDEX(p_index, sampler_tables.size());
        if p_index < 0 || p_index as usize >= self.sampler_tables.len() {
            crate::error::err_print_body(
                &format!(
                    "Index p_index = {} is out of bounds (sampler_tables.size() = {}).",
                    p_index,
                    self.sampler_tables.len()
                ),
                false,
            );
            return;
        }

        self.sampler_tables[p_index as usize]
            .borrow_mut()
            .set_stencil(p_table.clone());
    }

    pub fn clear_sampler_table_stencil(&mut self, p_index: i32) {
        // ERR_FAIL_INDEX(p_index, sampler_tables.size());
        if p_index < 0 || p_index as usize >= self.sampler_tables.len() {
            crate::error::err_print_body(
                &format!(
                    "Index p_index = {} is out of bounds (sampler_tables.size() = {}).",
                    p_index,
                    self.sampler_tables.len()
                ),
                false,
            );
            return;
        }

        self.sampler_tables[p_index as usize]
            .borrow_mut()
            .set_stencil(None);
    }

    pub fn set_stencil_custom_wave_tables(
        &mut self,
        p_tables: Vec<Option<Rc<RefCell<SiopmWaveTable>>>>,
    ) {
        self.stencil_custom_wave_tables = p_tables;
    }

    pub fn clear_stencil_custom_wave_tables(&mut self) {
        self.stencil_custom_wave_tables = Vec::new();
    }

    //

    // NOTE: parameters come from the ctor; the C++ signature carries no defaults.
    fn set_constants(&mut self, p_fm_clock: i32, p_psg_clock: f64, p_sampling_rate: i32) {
        // ERR_FAIL_COND_MSG(...): custom message is the FIRST ERROR: line (golden order).
        if p_sampling_rate != 44100 && p_sampling_rate != 22050 {
            crate::error::err_print_body(
                &format!(
                    "SiOPMRefTable: Invalid sampling rate '{}', only 44100 and 22050 are allowed.\nCondition \"(p_sampling_rate != 44100 && p_sampling_rate != 22050)\" is true.",
                    p_sampling_rate
                ),
                false,
            );
            return;
        }

        self.fm_clock = p_fm_clock;
        self.psg_clock = p_psg_clock;
        self.sampling_rate = p_sampling_rate;
        self.sample_rate_pitch_shift = if self.sampling_rate == 44100 { 0 } else { 1 };
        self.clock_ratio =
            ((self.fm_clock / 64) << Self::CLOCK_RATIO_BITS) / self.sampling_rate;
    }

    fn create_eg_tables(&mut self) {
        // Table selector & timer steps for rates.
        {
            let mut i = 0;
            while i < 44 {
                // rate = 0-43
                // C++ `1<<(i>>2)` never exceeds 1<<10 for i < 44; clock_ratio
                // product stays in i32 for supported clocks, but keep the
                // documented wrap semantics (two's complement `as i32`).
                let steps = (((1i32 << (i >> 2)) as f64) * (self.clock_ratio as f64)) as i32;
                self.eg_timer_steps[i] = steps;
                self.eg_table_selector[i] = (i & 3) as i32;
                i += 1;
            }
            while i < 48 {
                // rate = 44-47
                self.eg_timer_steps[i] = (2047.0 * (self.clock_ratio as f64)) as i32;
                self.eg_table_selector[i] = (i & 3) as i32;
                i += 1;
            }
            while i < 60 {
                // rate = 48-59
                self.eg_timer_steps[i] = (2047.0 * (self.clock_ratio as f64)) as i32;
                self.eg_table_selector[i] = (i - 44) as i32;
                i += 1;
            }
            while i < 96 {
                // rate = 60-95 (rate=60-95 are same as rate=63(maximum))
                self.eg_timer_steps[i] = (2047.0 * (self.clock_ratio as f64)) as i32;
                self.eg_table_selector[i] = 16;
                i += 1;
            }
            while i < 128 {
                // rate = 96-127 (dummies for ar,dr,sr=0)
                self.eg_timer_steps[i] = 0;
                self.eg_table_selector[i] = 17;
                i += 1;
            }
        }

        // Level tables for SSG envelope.
        {
            let table_size = 1 << Self::ENV_BITS;
            let half_size = table_size >> 2;

            let mut i = 0usize;
            while i < half_size {
                self.eg_level_tables[0][i] = i as i32; // normal table
                self.eg_level_tables[1][i] = (i as i32) << 2; // ssg positive
                self.eg_level_tables[2][i] = 512 - ((i as i32) << 2); // ssg negative
                self.eg_level_tables[3][i] = 512 + ((i as i32) << 2); // ssg positive + offset
                self.eg_level_tables[4][i] = 1024 - ((i as i32) << 2); // ssg negative + offset
                self.eg_level_tables[5][i] = 0; // ssg fixed at max
                self.eg_level_tables[6][i] = 1024; // ssg fixed at min
                i += 1;
            }
            while i < table_size {
                self.eg_level_tables[0][i] = i as i32; // normal table
                self.eg_level_tables[1][i] = 1024; // ssg positive
                self.eg_level_tables[2][i] = 0; // ssg negative
                self.eg_level_tables[3][i] = 1024; // ssg positive + offset
                self.eg_level_tables[4][i] = 512; // ssg negative + offset
                self.eg_level_tables[5][i] = 0; // ssg fixed at max
                self.eg_level_tables[6][i] = 1024; // ssg fixed at min
                i += 1;
            }
        }

        // Sustain and total level tables.
        {
            for i in 0..15 {
                self.eg_sustain_level_table[i] = (i as i32) << 5;
            }
            // sl(15) -> sl(1023)
            self.eg_sustain_level_table[15] = 31 << 5;

            // v(0-256) -> total_level(832,0). Translate linear volume to log scale gain.
            self.eg_total_level_tables[VM_LINEAR][0] = Self::ENV_BOTTOM;
            self.eg_total_level_tables[VM_DR96DB][0] = Self::ENV_BOTTOM;
            self.eg_total_level_tables[VM_DR64DB][0] = Self::ENV_BOTTOM;
            self.eg_total_level_tables[VM_DR48DB][0] = Self::ENV_BOTTOM;
            self.eg_total_level_tables[VM_DR32DB][0] = Self::ENV_BOTTOM;
            for i in 1..257i32 {
                // 0.00390625 = 1/256
                self.eg_total_level_tables[VM_LINEAR][i as usize] =
                    calculate_log_table_index(i as f64 * 0.00390625)
                        >> (Self::LOG_VOLUME_BITS - Self::ENV_BITS);
                self.eg_total_level_tables[VM_DR96DB][i as usize] = (256 - i) * 4; //  (n/2)<<ENV_LSHIFT
                self.eg_total_level_tables[VM_DR64DB][i as usize] =
                    ((256 - i) as f64 * 2.6666666666666667) as i32; // ((n/2)<<ENV_LSHIFT)*2/3
                self.eg_total_level_tables[VM_DR48DB][i as usize] = (256 - i) * 2; // ((n/2)<<ENV_LSHIFT)*1/2
                self.eg_total_level_tables[VM_DR32DB][i as usize] =
                    ((256 - i) as f64 * 1.333333333333333) as i32; // ((n/2)<<ENV_LSHIFT)*1/3
            }
            // v(257-448) -> total_level (0,-192). Distortion.
            for i in 1..193 {
                let j = i + 256;
                self.eg_total_level_tables[VM_LINEAR][j] = -(i as i32);
                self.eg_total_level_tables[VM_DR96DB][j] = -(i as i32);
                self.eg_total_level_tables[VM_DR64DB][j] = -(i as i32);
                self.eg_total_level_tables[VM_DR48DB][j] = -(i as i32);
                self.eg_total_level_tables[VM_DR32DB][j] = -(i as i32);
            }
            // v(449-512) -> total_level=-192. Distortion.
            for i in 1..65 {
                let j = i + 448;
                self.eg_total_level_tables[VM_LINEAR][j] = Self::ENV_TOP;
                self.eg_total_level_tables[VM_DR96DB][j] = Self::ENV_TOP;
                self.eg_total_level_tables[VM_DR64DB][j] = Self::ENV_TOP;
                self.eg_total_level_tables[VM_DR48DB][j] = Self::ENV_TOP;
                self.eg_total_level_tables[VM_DR32DB][j] = Self::ENV_TOP;
            }

            for i in 0..129 {
                // 0.0078125 = 1/128
                self.eg_linear_to_total_level_table[i] = calculate_log_table_index(
                    i as f64 * 0.0078125,
                ) >> (Self::LOG_VOLUME_BITS - Self::ENV_BITS + Self::ENV_LSHIFT);
            }
        }

        // Panning volume table.
        for i in 0..129 {
            self.pan_table[i] = crate::math::sin(i as f64 * 0.01227184630308513); // 0.01227184630308513 = PI*0.5/128
        }
    }

    fn create_pg_tables(&mut self) {
        // MIDI Note Number -> Key Code table
        {
            let mut i = 0;
            let mut j = 0;
            while j < Self::NOTE_TABLE_SIZE {
                if i < 16 {
                    self.note_number_to_key_code[j] = i;
                } else if i < Self::KEY_CODE_TABLE_SIZE as i32 {
                    self.note_number_to_key_code[j] = i - 16;
                } else {
                    self.note_number_to_key_code[j] = Self::KEY_CODE_TABLE_SIZE as i32 - 1;
                }
                i += 1;
                j = (i - (i >> 2)) as usize;
            }
        }

        self.pitch_table = vec![Vec::new(); PITCH_TABLE_MAX];

        // Pitch table.
        {
            let table_step = (Self::HALF_TONE_RESOLUTION * 12) as usize; // 12 = 1 octave
            let table_size = Self::PITCH_TABLE_SIZE;
            let pitch_delta = 1.0 / table_step as f64;

            // Wave length table.
            {
                let mut pitch_value = 0.0f64;
                let pitch_coef =
                    self.sampling_rate as f64 / 8.175798915643707; // = 5393.968278209282@44.1kHz sampling count @ MIDI note number = 0

                for i in 0..table_step {
                    let mut value = crate::math::pow(2.0, -pitch_value) * pitch_coef;

                    let mut j = i;
                    while j < table_size {
                        self.pitch_wave_length[j] = value;
                        value *= 0.5;
                        j += table_step;
                    }

                    pitch_value += pitch_delta;
                }
            }

            // Phase step tables.

            // OPM
            {
                let mut table = vec![0; table_size];

                let mut pitch_value = 0.0f64;
                // dphase @ MIDI note number = 0
                let pitch_coef = 8.175798915643707 * Self::PHASE_MAX as f64
                    / self.sampling_rate as f64;

                for i in 0..table_step {
                    let mut value = crate::math::pow(2.0, pitch_value) * pitch_coef;

                    let mut j = i;
                    while j < table_size {
                        table[j] = value as i32;
                        value *= 2.0;
                        j += table_step;
                    }

                    pitch_value += pitch_delta;
                }

                self.pitch_table[PITCH_TABLE_OPM as usize] = table;
                self.phase_step_shift_filter[PITCH_TABLE_OPM as usize] = 0;
            }

            // PCM
            {
                let mut table = vec![0; table_size];

                let mut pitch_value = 0.0f64;
                // dphase = pitchTablePCM[pitchIndex] >> (table_size (= PHASE_BITS - waveTable.fixedBits))
                // dphase @ MIDI note number = 0/ o0c=0.01858136117191752 -> o5a=1
                let pitch_coef = 0.01858136117191752 * Self::PHASE_MAX as f64;

                for i in 0..table_step {
                    let mut value = crate::math::pow(2.0, pitch_value) * pitch_coef;

                    let mut j = i;
                    while j < table_size {
                        table[j] = value as i32;
                        value *= 2.0;
                        j += table_step;
                    }

                    pitch_value += pitch_delta;
                }

                self.pitch_table[PITCH_TABLE_PCM as usize] = table;
                // 0xffffffff as C++ int
                self.phase_step_shift_filter[PITCH_TABLE_PCM as usize] = -1;
            }

            // PSG (table_size = 16)
            {
                let mut table = vec![0; table_size];

                let mut pitch_value = 0.0f64;
                let pitch_coef =
                    self.psg_clock * ((Self::PHASE_MAX >> 4) as f64) / self.sampling_rate as f64;

                for i in 0..table_step {
                    // 8.175798915643707 = [frequency @ MIDI note number = 0]
                    // 130.8127826502993 = 8.175798915643707 * 16
                    let mut value =
                        self.psg_clock / (crate::math::pow(2.0, pitch_value) * 130.8127826502993);

                    let mut j = i;
                    while j < table_size {
                        // Register value.
                        let reg_value = ((value + 0.5) as i32).min(4096); // Cap at 4096.

                        table[j] = (pitch_coef / reg_value as f64) as i32;
                        value *= 0.5;
                        j += table_step;
                    }

                    pitch_value += pitch_delta;
                }

                self.pitch_table[PITCH_TABLE_PSG as usize] = table;
                self.phase_step_shift_filter[PITCH_TABLE_PSG as usize] = 0;
            }
        }

        // Noise period tables.
        {
            // OPM noise period table.
            {
                let table_size = (32 << Self::HALF_TONE_BITS) as usize;
                let mut table = vec![0; table_size];

                // noise_phase_shift = pitchTable[PITCH_TABLE_OPM_NOISE][noiseFreq] >> (PHASE_BITS - waveTable.fixedBits).
                // C++ computes PHASE_MAX * clock_ratio in int and wraps;
                // reproduce the i32 truncation before the double conversion.
                let pitch_coef =
                    ((Self::PHASE_MAX as i64).wrapping_mul(self.clock_ratio as i64) as i32) as f64; // clock_ratio = ((clock/64)/rate) << CLOCK_RATIO_BITS

                let mut value = 0i32;
                for i in 0..31 {
                    value = ((pitch_coef / ((32 - i) as f64 * 0.5)) as i32) >> Self::CLOCK_RATIO_BITS;

                    for j in 0..Self::HALF_TONE_RESOLUTION as usize {
                        table[(i << Self::HALF_TONE_BITS as usize) + j] = value;
                    }
                }
                let mut i = 31i32 << Self::HALF_TONE_BITS;
                while i < table_size as i32 {
                    table[i as usize] = value;
                    i += 1;
                }

                self.pitch_table[PITCH_TABLE_OPM_NOISE as usize] = table;
                self.phase_step_shift_filter[PITCH_TABLE_OPM_NOISE as usize] = -1;
            }

            // PSG noise period table.
            {
                let table_size = (32 << Self::HALF_TONE_BITS) as usize;
                let mut table = vec![0; table_size];

                // noise_phase_shift = ((1<<PHASE_BIT)  /  ((nf/(clock/16))[sec]  /  (1/44100)[sec])) >> (PHASE_BIT - waveTable.fixedBits)
                let pitch_coef =
                    Self::PHASE_MAX as f64 * self.fm_clock as f64 / (self.sampling_rate * 16) as f64;

                for i in 0..32 {
                    // i == 0 divides by zero: C++ produced inf -> (int) UB
                    // (INT_MIN on x64); Rust `as i32` saturates to i32::MIN,
                    // matching MSVC's cvttsd2si behavior.
                    let value = pitch_coef / i as f64;

                    for j in 0..Self::HALF_TONE_RESOLUTION as usize {
                        table[(i << Self::HALF_TONE_BITS as usize) + j] = value as i32;
                    }
                }

                self.pitch_table[PITCH_TABLE_PSG_NOISE as usize] = table;
                self.phase_step_shift_filter[PITCH_TABLE_PSG_NOISE as usize] = -1;
            }

            // APU noise period table.
            {
                const REF_VALUES: [i32; 16] =
                    [4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068];

                let table_size = (16 << Self::HALF_TONE_BITS) as usize;
                let mut table = vec![0; table_size];

                // noise_phase_shift = ((1<<PHASE_BIT)  /  ((nf/clock)[sec]  /  (1/44100)[sec])) >> (PHASE_BIT - waveTable.fixedBits)
                let pitch_coef = Self::PHASE_MAX as f64 * self.psg_clock / self.sampling_rate as f64;

                for i in 0..16 {
                    let value = pitch_coef / REF_VALUES[i] as f64;

                    for j in 0..Self::HALF_TONE_RESOLUTION as usize {
                        table[(i << Self::HALF_TONE_BITS as usize) + j] = value as i32;
                    }
                }

                self.pitch_table[PITCH_TABLE_APU_NOISE as usize] = table;
                self.phase_step_shift_filter[PITCH_TABLE_APU_NOISE as usize] = -1;
            }

            // Gameboy noise period.
            {
                const REF_VALUES: [i32; 64] = [
                    2, 4, 8, 12, 16, 20, 24, 28, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192,
                    224, 256, 320, 384, 448, 512, 640, 768, 896, 1024, 1280, 1536, 1792, 2048,
                    2560, 3072, 3584, 4096, 5120, 6144, 7168, 8192, 10240, 12288, 14336, 16384,
                    20480, 24576, 28672, 32768, 40960, 49152, 57344, 65536, 81920, 98304, 114688,
                    131072, 163840, 196608, 229376, 262144, 327680, 393216, 458752,
                ];

                let table_size = (64 << Self::HALF_TONE_BITS) as usize;
                let mut table = vec![0; table_size];

                // noise_phase_shift = ((1<<PHASE_BIT)  /  ((nf/clock)[sec]  /  (1/44100)[sec])) >> (PHASE_BIT - waveTable.fixedBits)
                let pitch_coef =
                    Self::PHASE_MAX as f64 * 1048576.0 / self.sampling_rate as f64; // gb clock = 1048576

                for i in 0..64 {
                    let value = pitch_coef / REF_VALUES[i] as f64;

                    for j in 0..Self::HALF_TONE_RESOLUTION as usize {
                        table[(i << Self::HALF_TONE_BITS as usize) + j] = value as i32;
                    }
                }

                self.pitch_table[PITCH_TABLE_GB_NOISE as usize] = table;
                self.phase_step_shift_filter[PITCH_TABLE_GB_NOISE as usize] = -1;
            }
        }

        // dt1 table.
        {
            // dt1 table from X68Sound.dll
            const REF_VALUES: [[i32; 32]; 4] = [
                [
                    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                    0, 0, 0, 0, 0, 0,
                ],
                [
                    0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 3, 3, 3, 4, 4, 4, 5, 5, 6,
                    6, 7, 8, 8, 8, 8,
                ],
                [
                    1, 1, 1, 1, 2, 2, 2, 2, 2, 3, 3, 3, 4, 4, 4, 5, 5, 6, 6, 7, 8, 8, 9, 10, 11,
                    12, 13, 14, 16, 16, 16, 16,
                ],
                [
                    2, 2, 2, 2, 2, 3, 3, 3, 4, 4, 4, 5, 5, 6, 6, 7, 8, 8, 9, 10, 11, 12, 13, 14,
                    16, 17, 19, 20, 22, 22, 22, 22,
                ],
            ];

            for i in 0..4 {
                for j in 0..Self::KEY_CODE_TABLE_SIZE {
                    // C++: `int value = ((int)(ref_values[i][j >> 2]) * 64 *
                    // clock_ratio) >> CLOCK_RATIO_BITS;` — the cast applies to
                    // the table value only; the product is i32 (wrapping).
                    let value = (REF_VALUES[i][j >> 2].wrapping_mul(64)
                        .wrapping_mul(self.clock_ratio))
                        >> Self::CLOCK_RATIO_BITS;

                    self.dt1_table[i][j] = value;
                    self.dt1_table[i + 4][j] = -value;
                }
            }
        }

        // Log table.
        {
            {
                let table_start = (-Self::ENV_TOP) << 3; // Start at -ENV_TOP.
                let table_step = table_start + Self::LOG_TABLE_RESOLUTION * 2; // * 2 (positive & negative)
                let table_size = Self::LOG_TABLE_SIZE as i32;

                let pitch_delta = 1.0 / Self::LOG_TABLE_RESOLUTION as f64;
                let mut pitch_value = pitch_delta;

                let mut i = table_start;
                while i < table_step {
                    // v=2^(LOG_VOLUME_BITS-1/256) at maximum (i=2)
                    let mut value =
                        crate::math::pow(2.0, Self::LOG_VOLUME_BITS as f64 - pitch_value);

                    let mut j = i;
                    while j < table_size {
                        let reg_value = value as i32;

                        self.log_table[j as usize] = reg_value;
                        self.log_table[j as usize + 1] = -reg_value;
                        value *= 0.5;
                        j += Self::LOG_TABLE_RESOLUTION * 2;
                    }

                    pitch_value += pitch_delta;
                    i += 2;
                }
            }

            // Saturation area.
            {
                let table_size = ((-Self::ENV_TOP) << 3) as usize;
                let value = self.log_table[table_size];

                for i in (0..table_size).step_by(2) {
                    self.log_table[i] = value;
                    self.log_table[i + 1] = -value;
                }
            }

            // Zero fill area.
            {
                let table_size = Self::LOG_TABLE_SIZE * 3;

                for i in Self::LOG_TABLE_SIZE..table_size {
                    self.log_table[i] = 0;
                }
            }
        }
    }

    // `approx_constant`: DSP-frozen literals stay verbatim (CONVENTIONS.md).
    #[allow(clippy::approx_constant)]
    fn create_wave_samples(&mut self) {
        // Prepare tables.
        {
            let table_size = calculate_log_table_index(1.0);

            let no_wave_table_wave = vec![0; table_size as usize];
            self.no_wave_table = Some(Rc::new(RefCell::new(SiopmWaveTable::new(
                no_wave_table_wave,
                PITCH_TABLE_PCM,
            ))));

            let no_wave_table_opm_wave = vec![0; table_size as usize];
            self.no_wave_table_opm = Some(Rc::new(RefCell::new(SiopmWaveTable::new(
                no_wave_table_opm_wave,
                PITCH_TABLE_OPM,
            ))));

            self.wave_tables = vec![self.no_wave_table.clone(); Self::DEFAULT_PG_MAX];
            self.sampler_tables = (0..Self::SAMPLER_TABLE_MAX)
                .map(|_| {
                    let mut sampler = SiopmWaveSamplerTable::new();
                    sampler.clear();
                    Rc::new(RefCell::new(sampler))
                })
                .collect();

            self.custom_wave_tables = vec![None; Self::WAVE_TABLE_MAX];
            self.pcm_voices = vec![None; Self::PCM_DATA_MAX];
        }

        // Sine wave tables.
        {
            let table_step = Self::SAMPLING_TABLE_SIZE >> 1;
            let table_size = Self::SAMPLING_TABLE_SIZE;

            let mut table = vec![0; table_size];

            let value_delta = 6.283185307179586 / table_size as f64;
            let mut value_base = value_delta * 0.5;

            for i in 0..table_step {
                let value = calculate_log_table_index(crate::math::sin(value_base));

                table[i] = value; // positive
                table[i + table_step] = value + 1; // negative

                value_base += value_delta;
            }

            self.wave_tables[PULSE_SINE as usize] =
                Some(Rc::new(RefCell::new(SiopmWaveTable::new(table, PITCH_TABLE_OPM))));
        }

        // Saw wave tables.
        {
            {
                let table_step = Self::SAMPLING_TABLE_SIZE >> 1;
                let table_size = Self::SAMPLING_TABLE_SIZE;

                let mut table1 = vec![0; table_size];
                let mut table2 = vec![0; table_size];

                let value_delta = 1.0 / table_step as f64;
                let mut value_base = value_delta * 0.5;

                for i in 0..table_step {
                    let value = calculate_log_table_index(value_base);

                    table1[i] = value; // positive
                    table1[table_size - i - 1] = value + 1; // negative
                    table2[table_step - i - 1] = value; // positive
                    table2[table_step + i] = value + 1; // negative

                    value_base += value_delta;
                }

                self.wave_tables[PULSE_SAW_UP as usize] =
                    Some(Rc::new(RefCell::new(SiopmWaveTable::new(table1, PITCH_TABLE_OPM))));
                self.wave_tables[PULSE_SAW_DOWN as usize] =
                    Some(Rc::new(RefCell::new(SiopmWaveTable::new(table2, PITCH_TABLE_OPM))));
            }

            {
                let table_size = 32;

                let mut table = vec![0; table_size];

                let value_delta = 0.0625;
                let mut value_base = -0.96875;

                for i in 0..table_size {
                    table[i] = calculate_log_table_index(value_base);

                    value_base += value_delta;
                }

                self.wave_tables[PULSE_SAW_VC6 as usize] =
                    Some(Rc::new(RefCell::new(SiopmWaveTable::new(table, PITCH_TABLE_OPM))));
            }
        }

        // Triangle wave tables.
        {
            // Triangle wave.
            {
                let table_step = Self::SAMPLING_TABLE_SIZE >> 2;
                let table_offset = Self::SAMPLING_TABLE_SIZE >> 1;
                let table_size = Self::SAMPLING_TABLE_SIZE;

                let mut table = vec![0; table_size];

                let value_delta = 1.0 / table_step as f64;
                let mut value_base = value_delta * 0.5;

                for i in 0..table_step {
                    let value = calculate_log_table_index(value_base);

                    table[i] = value; // positive
                    table[table_offset - i - 1] = value; // positive
                    table[table_offset + i] = value + 1; // negative
                    table[table_size - i - 1] = value + 1; // negative

                    value_base += value_delta;
                }

                self.wave_tables[PULSE_TRIANGLE as usize] =
                    Some(Rc::new(RefCell::new(SiopmWaveTable::new(table, PITCH_TABLE_OPM))));
            }

            // FC triangle wave.
            {
                let mut table = vec![0; 32];

                let value_delta = 0.125;
                let mut value_base = 0.125;

                table[0] = Self::LOG_TABLE_BOTTOM;
                table[15] = Self::LOG_TABLE_BOTTOM;
                table[23] = 3;
                table[24] = 3;

                for i in 1..8 {
                    let value = calculate_log_table_index(value_base);

                    table[i] = value;
                    table[15 - i] = value;
                    table[15 + i] = value + 1;
                    table[32 - i] = value + 1;

                    value_base += value_delta;
                }

                self.wave_tables[PULSE_TRIANGLE_FC as usize] =
                    Some(Rc::new(RefCell::new(SiopmWaveTable::new(table, PITCH_TABLE_OPM))));
            }
        }

        // Square wave tables.
        {
            // 50% square wave.
            let value = calculate_log_table_index(Self::SQUARE_WAVE_OUTPUT);
            let table = vec![value, value + 1];

            self.wave_tables[PULSE_SQUARE as usize] =
                Some(Rc::new(RefCell::new(SiopmWaveTable::new(table, PITCH_TABLE_OPM))));
        }

        // Pulse wave tables.
        {
            let base_table = self.wave_tables[PULSE_SQUARE as usize]
                .as_ref()
                .unwrap()
                .borrow()
                .get_wavelet();

            // Pulse wave.
            // NOTE: The resolution of duty ratio is twice than pAPU. [pAPU pulse wave table] = waveTables[PULSE_PULSE+duty*2].
            {
                for j in 0..16 {
                    let mut table = vec![0; 16];

                    for i in 0..16 {
                        table[i] = if i < j { base_table[0] } else { base_table[1] };
                    }

                    self.wave_tables[(PULSE_PULSE + j as i32) as usize] =
                        Some(Rc::new(RefCell::new(SiopmWaveTable::new(table, PITCH_TABLE_OPM))));
                }
            }

            // Spike pulse.
            {
                let value = calculate_log_table_index(0.0);

                for j in 0..16 {
                    let mut table = vec![0; 32];

                    let mut i = 0;
                    let table_step = j << 1;
                    while i < table_step {
                        table[i] = if i < j { base_table[0] } else { base_table[1] };
                        i += 1;
                    }
                    while i < 32 {
                        table[i] = value;
                        i += 1;
                    }

                    self.wave_tables[(PULSE_PULSE_SPIKE + j as i32) as usize] =
                        Some(Rc::new(RefCell::new(SiopmWaveTable::new(table, PITCH_TABLE_OPM))));
                }
            }
        }

        // Konami bubble system wave tables.
        {
            const REF_TABLE: [i32; 32] = [
                -80, -112, -16, 96, 64, 16, 64, 96, 32, -16, 64, 112, 80, 0, 32, 48, -16, -96, 0,
                80, 16, -64, -48, -16, -96, -128, -80, 0, -48, -112, -80, -32,
            ];

            let mut table = vec![0; 32];

            for i in 0..32 {
                table[i] = calculate_log_table_index(REF_TABLE[i] as f64 / 128.0);
            }

            self.wave_tables[PULSE_KNM_BUBBLE as usize] =
                Some(Rc::new(RefCell::new(SiopmWaveTable::new(table, PITCH_TABLE_OPM))));
        }

        // Pseudo sync wave tables.
        {
            let mut table1 = vec![0; Self::SAMPLING_TABLE_SIZE];
            let mut table2 = vec![0; Self::SAMPLING_TABLE_SIZE];

            let table_step = Self::SAMPLING_TABLE_SIZE;
            let value_delta = 1.0 / table_step as f64;
            let mut value_base = value_delta * 0.5;

            for i in 0..table_step {
                let value = calculate_log_table_index(value_base);

                table1[i] = value + 1; // negative
                table2[i] = value; // positive

                value_base += value_delta;
            }

            self.wave_tables[PULSE_SYNC_LOW as usize] =
                Some(Rc::new(RefCell::new(SiopmWaveTable::new(table1, PITCH_TABLE_OPM))));
            self.wave_tables[PULSE_SYNC_HIGH as usize] =
                Some(Rc::new(RefCell::new(SiopmWaveTable::new(table2, PITCH_TABLE_OPM))));
        }

        // Noise tables.
        {
            // White noise, pulse noise.
            // NOTE: Naive implementation. Details are shown in MAME or VirtuaNes source.
            {
                let mut table1 = vec![0; Self::NOISE_TABLE_SIZE];
                let mut table2 = vec![0; Self::NOISE_TABLE_SIZE];

                let table_step = Self::NOISE_TABLE_SIZE;

                let value_coef = Self::NOISE_WAVE_OUTPUT / 32768.0;
                let mut value_base = 1i32; // 15bit LFSR
                let value = calculate_log_table_index(Self::NOISE_WAVE_OUTPUT);

                for i in 0..table_step {
                    value_base = (((value_base << 13) ^ (value_base << 14)) & 0x4000)
                        | (value_base >> 1);

                    table1[i] = calculate_log_table_index(
                        (value_base & 0x7fff) as f64 * value_coef * 2.0 - 1.0,
                    );
                    table2[i] = if value_base & 1 != 0 { value } else { value + 1 };
                }

                self.wave_tables[PULSE_NOISE_WHITE as usize] = Some(Rc::new(RefCell::new(
                    SiopmWaveTable::new(table1, PITCH_TABLE_PCM),
                )));
                self.wave_tables[PULSE_NOISE_PULSE as usize] = Some(Rc::new(RefCell::new(
                    SiopmWaveTable::new(table2.clone(), PITCH_TABLE_PCM),
                )));
                self.wave_tables[PULSE_PC_NZ_OPM as usize] = Some(Rc::new(RefCell::new(
                    SiopmWaveTable::new(table2, PITCH_TABLE_OPM_NOISE),
                )));
                self.wave_tables[PULSE_NOISE as usize] =
                    self.wave_tables[PULSE_NOISE_WHITE as usize].clone();
            }

            // FC short noise.
            //NOTE: Naive implementation. 93*11=1023 approx.-> 1024.
            {
                let mut table = vec![0; Self::SAMPLING_TABLE_SIZE];

                let table_step = Self::SAMPLING_TABLE_SIZE;
                let mut value_base = 1i32; // 15bit LFSR
                let value = calculate_log_table_index(Self::NOISE_WAVE_OUTPUT);

                for i in 0..table_step {
                    value_base = (((value_base << 8) ^ (value_base << 14)) & 0x4000)
                        | (value_base >> 1);

                    table[i] = if value_base & 1 != 0 { value } else { value + 1 };
                }

                self.wave_tables[PULSE_NOISE_SHORT as usize] = Some(Rc::new(RefCell::new(
                    SiopmWaveTable::new(table, PITCH_TABLE_PCM),
                )));
            }

            // GB short noise.
            {
                let mut table = vec![0; 128];

                let mut value_base = 0xffffi32; // 16bit LFSR (wrapping like C++ int in practice)
                let mut value_offset = 0i32;
                let value = calculate_log_table_index(Self::NOISE_WAVE_OUTPUT);

                for i in 0..128 {
                    value_base = value_base.wrapping_add(
                        value_base
                            .wrapping_add(((value_base >> 6) ^ (value_base >> 5)) & 1),
                    );
                    value_offset ^= value_base & 1;

                    table[i] = if value_offset & 1 != 0 { value } else { value + 1 };
                }

                self.wave_tables[PULSE_NOISE_GB_SHORT as usize] = Some(Rc::new(RefCell::new(
                    SiopmWaveTable::new(table, PITCH_TABLE_PCM),
                )));
            }

            // Periodic noise.
            {
                let mut table = vec![0; 16];

                table[0] = calculate_log_table_index(Self::SQUARE_WAVE_OUTPUT);
                for item in table.iter_mut().take(16).skip(1) {
                    *item = Self::LOG_TABLE_BOTTOM;
                }

                self.wave_tables[PULSE_PC_NZ_16BIT as usize] = Some(Rc::new(RefCell::new(
                    SiopmWaveTable::new(table, PITCH_TABLE_OPM),
                )));
            }

            // High-passed white noise.
            {
                let base_table = self.wave_tables[PULSE_NOISE_WHITE as usize]
                    .as_ref()
                    .unwrap()
                    .borrow()
                    .get_wavelet();
                let mut table = vec![0; Self::NOISE_TABLE_SIZE];

                let table_step = Self::NOISE_TABLE_SIZE;

                let value_offset = (-Self::ENV_TOP) << 3;
                let value_coef = 16.0 / ((1 << Self::LOG_VOLUME_BITS) as f64);
                let mut log_value1 = base_table[0] + value_offset;
                let mut log_value2 = base_table[Self::NOISE_TABLE_SIZE - 1] + value_offset;
                let mut value = (self.log_table[log_value1 as usize] as f64
                    - self.log_table[log_value2 as usize] as f64)
                    * 0.0625;

                table[0] = calculate_log_table_index(value * value_coef);
                for i in 1..table_step {
                    log_value1 = base_table[i] + value_offset;
                    log_value2 = base_table[i - 1] + value_offset;
                    value = (value
                        + self.log_table[log_value1 as usize] as f64
                        - self.log_table[log_value2 as usize] as f64)
                        * 0.0625;

                    table[i] = calculate_log_table_index(value * value_coef);
                }

                self.wave_tables[PULSE_NOISE_HIPASS as usize] = Some(Rc::new(RefCell::new(
                    SiopmWaveTable::new(table, PITCH_TABLE_PCM),
                )));
            }

            // Pink noise.
            {
                let base_table = self.wave_tables[PULSE_NOISE_WHITE as usize]
                    .as_ref()
                    .unwrap()
                    .borrow()
                    .get_wavelet();
                let mut table = vec![0; Self::NOISE_TABLE_SIZE];

                let table_step = Self::NOISE_TABLE_SIZE;

                let value_offset = (-Self::ENV_TOP) << 3;
                let value_coef = 0.125 / ((1 << Self::LOG_VOLUME_BITS) as f64);
                let mut b0 = 0.0f64;
                let mut b1 = 0.0f64;
                let mut b2 = 0.0f64;

                for i in 0..table_step {
                    let log_value = base_table[i] + value_offset;
                    let value_base = self.log_table[log_value as usize] as f64;

                    b0 = 0.99765 * b0 + value_base * 0.0990460;
                    b1 = 0.96300 * b1 + value_base * 0.2965164;
                    b2 = 0.57000 * b2 + value_base * 1.0526913;

                    table[i] = calculate_log_table_index(
                        (b0 + b1 + b2 + value_base * 0.1848) * value_coef,
                    );
                }

                self.wave_tables[PULSE_NOISE_PINK as usize] = Some(Rc::new(RefCell::new(
                    SiopmWaveTable::new(table, PITCH_TABLE_PCM),
                )));
            }

            // Pitch-controllable noise.
            {
                let base_table = self.wave_tables[PULSE_NOISE_SHORT as usize]
                    .as_ref()
                    .unwrap()
                    .borrow()
                    .get_wavelet();
                let mut table = vec![0; Self::SAMPLING_TABLE_SIZE];

                for j in 0..Self::SAMPLING_TABLE_SIZE {
                    let mut i = j * 11;
                    let table_step = (i + 11).min(Self::SAMPLING_TABLE_SIZE);

                    while i < table_step {
                        table[i] = base_table[j];
                        i += 1;
                    }
                }

                self.wave_tables[PULSE_PC_NZ_SHORT as usize] = Some(Rc::new(RefCell::new(
                    SiopmWaveTable::new(table, PITCH_TABLE_OPM),
                )));
            }
        }

        // Ramp wave tables.
        {
            let table_size = Self::SAMPLING_TABLE_SIZE;
            let table_offset = Self::SAMPLING_TABLE_SIZE >> 1;
            let table_shift = Self::SAMPLING_TABLE_SIZE >> 2;

            let mut prev = 0;
            for j in 1..60 {
                let mut curr = table_shift >> (j >> 3);
                curr -= (curr * (j & 7)) >> 4;

                if prev == curr {
                    let t = self.wave_tables[(PULSE_RAMP + 65 - j as i32) as usize].clone();
                    self.wave_tables[(PULSE_RAMP + 64 - j as i32) as usize] = t;
                    let t = self.wave_tables[(PULSE_RAMP + 63 + j as i32) as usize].clone();
                    self.wave_tables[(PULSE_RAMP + 64 + j as i32) as usize] = t;
                    continue;
                }
                prev = curr;

                let mut table1 = vec![0; Self::SAMPLING_TABLE_SIZE];
                let mut table2 = vec![0; Self::SAMPLING_TABLE_SIZE];

                let table_step = table_offset - curr;

                let mut value_delta = 1.0 / table_step as f64;
                let mut value_base = value_delta * 0.5;

                let mut i = 0;
                while i < table_step {
                    let value = calculate_log_table_index(value_base);

                    table1[i] = value; // positive
                    table1[table_size - i - 1] = value + 1; // negative
                    table2[table_offset + i] = value + 1; // negative
                    table2[table_offset - i - 1] = value; // positive

                    value_base += value_delta;
                    i += 1;
                }

                value_delta = 1.0 / (table_offset - table_step) as f64;

                while i < table_offset {
                    let value = calculate_log_table_index(value_base);

                    table1[i] = value; // positive
                    table1[table_size - i - 1] = value + 1; // negative
                    table2[table_offset + i] = value + 1; // negative
                    table2[table_offset - i - 1] = value; // positive

                    value_base -= value_delta;
                    i += 1;
                }

                self.wave_tables[(PULSE_RAMP + 64 - j as i32) as usize] =
                    Some(Rc::new(RefCell::new(SiopmWaveTable::new(table1, PITCH_TABLE_OPM))));
                self.wave_tables[(PULSE_RAMP + 64 + j as i32) as usize] =
                    Some(Rc::new(RefCell::new(SiopmWaveTable::new(table2, PITCH_TABLE_OPM))));
            }

            for j in 0..5 {
                let t = self.wave_tables[PULSE_SAW_UP as usize].clone();
                self.wave_tables[(PULSE_RAMP + j) as usize] = t;
            }
            for j in 124..128 {
                let t = self.wave_tables[PULSE_SAW_DOWN as usize].clone();
                self.wave_tables[(PULSE_RAMP + j) as usize] = t;
            }

            let t = self.wave_tables[PULSE_TRIANGLE as usize].clone();
            self.wave_tables[(PULSE_RAMP + 64) as usize] = t;
        }

        // MA3 wave tables.
        {
            // 0-5 - sine waves.
            let sine = self.wave_tables[PULSE_SINE as usize].clone();
            self.create_ma3_waveset(PULSE_MA3_SINE, &sine);

            // 6 - square wave.
            let t = self.wave_tables[PULSE_SQUARE as usize].clone();
            self.wave_tables[PULSE_MA3_SQUARE as usize] = t;

            // 7 - downwards saw wave with sine flattening. Best name I can come up with, not sure if there is a more common description.
            {
                let mut table1 = vec![0; Self::SAMPLING_TABLE_SIZE];

                let table_step = Self::SAMPLING_TABLE_SIZE >> 2;
                let table_offset = Self::SAMPLING_TABLE_SIZE >> 1;
                let table_size = Self::SAMPLING_TABLE_SIZE;

                let value_delta = 6.283185307179586 / Self::SAMPLING_TABLE_SIZE as f64;
                let mut value_base = value_delta * 0.5;

                for i in 0..table_step {
                    let value = calculate_log_table_index(1.0 - crate::math::sin(value_base));

                    table1[i] = value; // positive
                    table1[i + table_step] = Self::LOG_TABLE_BOTTOM;
                    table1[i + table_offset] = Self::LOG_TABLE_BOTTOM;
                    table1[table_size - i - 1] = value + 1; // negative

                    value_base += value_delta;
                }

                self.wave_tables[PULSE_MA3_SAW_SINE as usize] = Some(Rc::new(RefCell::new(
                    SiopmWaveTable::new(table1, PITCH_TABLE_OPM),
                )));
            }

            // 8-13 - triangle modulated sine
            {
                let base_table = self.wave_tables[PULSE_SINE as usize]
                    .as_ref()
                    .unwrap()
                    .borrow()
                    .get_wavelet();
                let mut table = vec![0; Self::SAMPLING_TABLE_SIZE];

                // `j` oscillates negative in mid blocks (C++ int arithmetic);
                // `i + j` always stays within the table.
                let mut j = 0isize;
                for i in 0..Self::SAMPLING_TABLE_SIZE {
                    table[i] = base_table[(i as isize + j) as usize];
                    j += 1 - ((((i as i32) >> (Self::SAMPLING_TABLE_BITS - 3)) + 1) & 2) as isize; // triangle wave
                }

                self.create_ma3_waveset(
                    PULSE_MA3_TRI_SINE,
                    &Some(Rc::new(RefCell::new(SiopmWaveTable::new(
                        table,
                        PITCH_TABLE_OPM,
                    )))),
                );
            }

            // 14 - half square
            {
                let value = calculate_log_table_index(1.0);
                let table = vec![value, Self::LOG_TABLE_BOTTOM];

                self.wave_tables[PULSE_MA3_SQUARE_HALF as usize] = Some(Rc::new(RefCell::new(
                    SiopmWaveTable::new(table, PITCH_TABLE_OPM),
                )));
            }

            // 16-21 - triangle waves
            let triangle = self.wave_tables[PULSE_TRIANGLE as usize].clone();
            self.create_ma3_waveset(PULSE_MA3_TRI, &triangle);

            // 22 - quarter square doubled.
            {
                let value = calculate_log_table_index(1.0);
                let table = vec![
                    value,
                    Self::LOG_TABLE_BOTTOM,
                    value,
                    Self::LOG_TABLE_BOTTOM,
                ];

                self.wave_tables[PULSE_MA3_SQUARE_QUART_DOUBLE as usize] = Some(Rc::new(
                    RefCell::new(SiopmWaveTable::new(table, PITCH_TABLE_OPM)),
                ));
            }

            // 24-29 - upwards saw waves.
            let saw_up = self.wave_tables[PULSE_SAW_UP as usize].clone();
            self.create_ma3_waveset(PULSE_MA3_SAW, &saw_up);

            // 30 - quarter square wave.
            {
                let value = calculate_log_table_index(1.0);
                let table = vec![
                    value,
                    Self::LOG_TABLE_BOTTOM,
                    Self::LOG_TABLE_BOTTOM,
                    Self::LOG_TABLE_BOTTOM,
                ];

                self.wave_tables[PULSE_MA3_SQUARE_QUART as usize] = Some(Rc::new(RefCell::new(
                    SiopmWaveTable::new(table, PITCH_TABLE_OPM),
                )));
            }

            // 15,23,31 - user defined waves.
            self.wave_tables[PULSE_MA3_USER1 as usize] = self.no_wave_table.clone();
            self.wave_tables[PULSE_MA3_USER2 as usize] = self.no_wave_table.clone();
            self.wave_tables[PULSE_MA3_USER3 as usize] = self.no_wave_table.clone();
        }
    }

    #[allow(clippy::approx_constant)]
    fn create_ma3_waveset(&mut self, p_index: i32, p_table: &Option<Rc<RefCell<SiopmWaveTable>>>) {
        // MA-3 waveforms contain 4 sets with the same basic premise. We take the base one,
        // then modify it via the same transforms to get 5 variations.

        // 0 - Full wave.
        self.wave_tables[p_index as usize] = p_table.clone();
        let basic_waveform = p_table.as_ref().unwrap().borrow().get_wavelet();

        // 1 - Half wave.
        {
            let mut table = vec![0; Self::SAMPLING_TABLE_SIZE];
            let table_offset = Self::SAMPLING_TABLE_SIZE >> 1;
            for i in 0..table_offset {
                table[i] = basic_waveform[i];
                table[i + table_offset] = Self::LOG_TABLE_BOTTOM;
            }
            self.wave_tables[(p_index + 1) as usize] =
                Some(Rc::new(RefCell::new(SiopmWaveTable::new(table, PITCH_TABLE_OPM))));
        }

        // 2 - Half wave doubled.
        {
            let mut table = vec![0; Self::SAMPLING_TABLE_SIZE];
            let table_offset = Self::SAMPLING_TABLE_SIZE >> 1;
            for i in 0..table_offset {
                table[i] = basic_waveform[i];
                table[i + table_offset] = basic_waveform[i];
            }
            self.wave_tables[(p_index + 2) as usize] =
                Some(Rc::new(RefCell::new(SiopmWaveTable::new(table, PITCH_TABLE_OPM))));
        }

        // 3 - Quarter wave doubled.
        {
            let mut table = vec![0; Self::SAMPLING_TABLE_SIZE];
            let table_offset = Self::SAMPLING_TABLE_SIZE >> 2;
            for i in 0..table_offset {
                table[i] = basic_waveform[i];
                table[i + table_offset] = Self::LOG_TABLE_BOTTOM;
                table[i + table_offset * 2] = basic_waveform[i];
                table[i + table_offset * 3] = Self::LOG_TABLE_BOTTOM;
            }
            self.wave_tables[(p_index + 3) as usize] =
                Some(Rc::new(RefCell::new(SiopmWaveTable::new(table, PITCH_TABLE_OPM))));
        }

        // 4 - Sped up wave.
        {
            let mut table = vec![0; Self::SAMPLING_TABLE_SIZE];
            let table_offset = Self::SAMPLING_TABLE_SIZE >> 1;
            for i in 0..table_offset {
                table[i] = basic_waveform[i << 1];
                table[i + table_offset] = Self::LOG_TABLE_BOTTOM;
            }
            self.wave_tables[(p_index + 4) as usize] =
                Some(Rc::new(RefCell::new(SiopmWaveTable::new(table, PITCH_TABLE_OPM))));
        }

        // 5 - Sped up half wave doubled.
        {
            let mut table = vec![0; Self::SAMPLING_TABLE_SIZE];
            let table_offset = Self::SAMPLING_TABLE_SIZE >> 2;
            for i in 0..table_offset {
                table[i] = basic_waveform[i << 1];
                table[i + table_offset] = table[i];
                table[i + table_offset * 2] = Self::LOG_TABLE_BOTTOM;
                table[i + table_offset * 3] = Self::LOG_TABLE_BOTTOM;
            }
            self.wave_tables[(p_index + 5) as usize] =
                Some(Rc::new(RefCell::new(SiopmWaveTable::new(table, PITCH_TABLE_OPM))));
        }
    }

    fn create_lfo_tables(&mut self) {
        // Timer steps.
        // This calculation is hybrid between fmgen and x68sound.dll, and extend as 20bit fixed dicimal.
        for i in 0..Self::LFO_TABLE_SIZE {
            let t = (16 + (i & 15)) as i32; // linear interpolation for 4LSBs
            let s = 15 - (i >> 4); // log-scale shift for 4HSBs
            // `* clock_ratio` overflows i32 for the slowest rates — C++ wraps
            // on two's complement; match it.
            self.lfo_timer_steps[i] = t.wrapping_shl((Self::LFO_FIXED_BITS - 4) as u32)
                .wrapping_mul(self.clock_ratio)
                / (8i32 << s)
                >> Self::CLOCK_RATIO_BITS;
        }

        // Wave tables.
        {
            // Saw wave.
            for i in 0..Self::LFO_TABLE_SIZE {
                self.lfo_wave_tables[LFO_WAVE_SAW][i] = 255 - i as i32;
                self.lfo_wave_tables[LFO_WAVE_SAW + 4][i] = i as i32;
            }

            // Pulse wave.
            for i in 0..Self::LFO_TABLE_SIZE {
                let value = if i < 128 { 255 } else { 0 };
                self.lfo_wave_tables[LFO_WAVE_SQUARE][i] = value;
                self.lfo_wave_tables[LFO_WAVE_SQUARE + 4][i] = 255 - value;
            }

            // Triangle wave.
            for i in 0..64 {
                // Quarter of LFO_TABLE_SIZE.
                let t = (i << 1) as i32;
                self.lfo_wave_tables[LFO_WAVE_TRIANGLE][i] = t + 128;
                self.lfo_wave_tables[LFO_WAVE_TRIANGLE][127 - i] = t + 128;
                self.lfo_wave_tables[LFO_WAVE_TRIANGLE][128 + i] = 126 - t;
                self.lfo_wave_tables[LFO_WAVE_TRIANGLE][255 - i] = 126 - t;
            }
            for i in 0..Self::LFO_TABLE_SIZE {
                self.lfo_wave_tables[LFO_WAVE_TRIANGLE + 4][i] =
                    255 - self.lfo_wave_tables[LFO_WAVE_TRIANGLE][i];
            }

            // Noise wave.
            let mut rng = RandomNumberGenerator::new();
            for i in 0..Self::LFO_TABLE_SIZE {
                let value = rng.randi_range(0, 255);
                self.lfo_wave_tables[LFO_WAVE_NOISE][i] = value;
                self.lfo_wave_tables[LFO_WAVE_NOISE + 4][i] = 255 - value;
            }
        }

        // Chorus tables.
        for i in 0..Self::LFO_TABLE_SIZE {
            let d = i as i32 - 128;
            self.lfo_chorus_tables[i] = d * d;
        }
    }

    fn create_filter_tables(&mut self) {
        for i in 0..128 {
            self.filter_cutoff_table[i] = i as f64 * i as f64 * 0.00006103515625; // 0.00006103515625 = 1/(128*128)
            self.filter_feedback_table[i] =
                1.0 + 1.0 / (1.0 - self.filter_cutoff_table[i]); // ???
        }
        self.filter_cutoff_table[128] = 1.0;
        // Original code assigns the value to itself here. Probably meant this instead.
        self.filter_feedback_table[128] = self.filter_cutoff_table[128];

        self.filter_eg_rate[0] = 0;
        for i in 1..60 {
            let shift = (1 << (14 - (i >> 2))) as f64;
            let liner = ((i & 3) as f64) * 0.125 + 0.5;
            // 2.36514 = 3 / ((fm_clock/64)/sampling_rate)
            self.filter_eg_rate[i] = (2.36514 * shift * liner + 0.5) as i32;
        }
        for i in 60..64 {
            self.filter_eg_rate[i] = 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_table_index_known_values() {
        // 1.0 -> index 2 (first entry pair beyond the saturation area start).
        assert_eq!(calculate_log_table_index(1.0), 2);
        // Exact powers of two step by LOG_TABLE_RESOLUTION*2 per octave.
        assert_eq!(
            calculate_log_table_index(0.5) - calculate_log_table_index(1.0),
            SiopmRefTable::LOG_TABLE_RESOLUTION * 2
        );
        // Below threshold clamps to the bottom.
        assert_eq!(
            calculate_log_table_index(0.0),
            SiopmRefTable::LOG_TABLE_BOTTOM
        );
        assert_eq!(
            calculate_log_table_index(-1.0),
            calculate_log_table_index(1.0) + 1
        );
    }

    #[test]
    fn singleton_tables_sane() {
        initialize();
        let t = instance();
        let t = t.borrow();

        // Sine wavelet: half positive / negative halves, first negative pair at the half.
        let sine = t.wave_tables[crate::sion_enums::PULSE_SINE as usize]
            .as_ref()
            .unwrap()
            .borrow();
        let w = sine.wavelet_slice();
        assert_eq!(w.len(), SiopmRefTable::SAMPLING_TABLE_SIZE);
        assert!(w[0] < SiopmRefTable::LOG_TABLE_BOTTOM);
        assert_eq!(w[SiopmRefTable::SAMPLING_TABLE_SIZE / 2], w[0] + 1);
        assert_eq!(sine.get_fixed_bits(), SiopmRefTable::PHASE_BITS - 10);

        // Log table: first written entry is `2^(13 - 1/256)` truncated; the
        // saturation area replicates it over [0, 1536). Zero fill starts at
        // LOG_TABLE_SIZE.
        assert_eq!(t.log_table[0], 8169);
        assert_eq!(t.log_table[1536], 8169);
        assert_eq!(t.log_table[SiopmRefTable::LOG_TABLE_SIZE], 0);
        assert_eq!(t.log_table[1537], -8169);

        // Wave tables all populated (no holes).
        for (i, wt) in t.wave_tables.iter().enumerate() {
            assert!(wt.is_some(), "wave_tables[{i}] empty");
        }

        // Pitch table OPM: o5a-ish index sanity (MIDI 69, exact half-tone grid).
        let opm = &t.pitch_table[crate::sion_enums::PITCH_TABLE_OPM as usize];
        assert_eq!(opm.len(), SiopmRefTable::PITCH_TABLE_SIZE);
        assert!(opm[69 * 64] > opm[68 * 64]);
    }
}
