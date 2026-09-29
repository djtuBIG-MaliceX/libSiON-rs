//! Port of `effector/filters/si_filter_vowel.{h,cpp}`.
//!
//! Six-formant parallel band resonator driven by a time-sorted formant
//! event queue. The C++ static formant tables (`_alpha_table`, `_cos_table`,
//! `_gain_table`) are built once by `Formant::initialize()`; the Rust port
//! builds them lazily in a `thread_local` (same values, same first-touch
//! semantics).
//!
//! Faithfully-preserved C++ quirks:
//! - `_process_lfo_formant(Formant, FormantTap, double*)` takes the formant
//!   and tap BY VALUE, so tap-history writes are discarded — member taps
//!   stay zero forever after `prepare_process()`. The port copies the tap,
//!   mutates the copy, and drops it.
//! - `set_by_mml` calls `set_formant_band1` (index 0) for ALL six bands;
//!   only band 6's frequency/gain survive.
//! - `set_vowel_formants` computes two freq indices that are never used.

use std::cell::RefCell;

use crate::effector::effect_base::{get_mml_arg, EffectBase, EffectSettings};
use crate::math::{self, clampf, clampi};

const FORMANT_COUNT: usize = 6;
const BAND_TABLE_MAX: usize = 8;
const FREQ_TABLE_MAX: usize = 1024;
const GAIN_TABLE_MAX: usize = 128;

const BAND_LIST: [f64; BAND_TABLE_MAX] = [0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0];

struct FormantTables {
    alpha_table: [[f64; FREQ_TABLE_MAX]; BAND_TABLE_MAX],
    cos_table: [f64; FREQ_TABLE_MAX],
    gain_table: [f64; GAIN_TABLE_MAX],
}

impl FormantTables {
    fn initialize() -> FormantTables {
        let mut alpha_table = [[0.0; FREQ_TABLE_MAX]; BAND_TABLE_MAX];
        for i in 0..BAND_TABLE_MAX {
            let band = BAND_LIST[i];
            let mut frequency = 50.0;
            for j in 0..FREQ_TABLE_MAX {
                let omg = frequency * 0.00014247585730565955;
                let sin = math::sin(omg);
                let ang = 0.34657359027997264 * band * omg / sin;
                alpha_table[i][j] = sin * math::sinh(ang);
                frequency *= 1.0218971486541166;
            }
        }

        let mut cos_table = [0.0; FREQ_TABLE_MAX];
        {
            let mut frequency = 50.0;
            for j in 0..FREQ_TABLE_MAX {
                cos_table[j] = math::cos(frequency * 0.00014247585730565955);
                frequency *= 1.0218971486541166;
            }
        }

        let mut gain_table = [0.0; GAIN_TABLE_MAX];
        for i in 0..GAIN_TABLE_MAX {
            gain_table[i] = math::pow(10.0, (i as i32 - 32) as f64 * 0.025);
        }

        FormantTables {
            alpha_table,
            cos_table,
            gain_table,
        }
    }
}

thread_local! {
    static FORMANT_TABLES: RefCell<FormantTables> = RefCell::new(FormantTables::initialize());
}

#[derive(Debug, Clone, Copy)]
pub struct Formant {
    pub ab1: f64,
    pub a2: f64,
    pub b0: f64,
    pub b2: f64,
}

impl Formant {
    pub fn new() -> Formant {
        Formant {
            ab1: 0.0,
            a2: 0.0,
            b0: 1.0,
            b2: 0.0,
        }
    }

    pub fn calculate_freq_index(p_frequency: f64) -> i32 {
        let freq_index =
            (math::ln(p_frequency) * 1.4426950408889633 - 5.643856189774724) * 32.0;
        clampi(freq_index as i32, 0, 1023)
    }

    pub fn update(&mut self, p_freq_index: i32, p_gain: i32, p_band_index: i32) {
        FORMANT_TABLES.with(|tables| {
            let tables = tables.borrow();
            let gain_index = clampi(p_gain + 32, 0, 127);
            let alpha = tables.alpha_table[p_band_index as usize][p_freq_index as usize];
            let gain = tables.gain_table[gain_index as usize];

            let alpa = alpha * gain;
            let alpia = alpha / gain;
            let ia0 = 1.0 / (1.0 + alpia);

            self.ab1 = -2.0 * tables.cos_table[p_freq_index as usize] * ia0;
            self.a2 = (1.0 - alpia) * ia0;
            self.b0 = (1.0 + alpa) * ia0;
            self.b2 = (1.0 - alpa) * ia0;
        });
    }
}

impl Default for Formant {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FormantTap {
    pub in0: f64,
    pub in1: f64,
    pub out0: f64,
    pub out1: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct FormantEvent {
    pub frequency1: i32,
    pub gain1: i32,
    pub frequency2: i32,
    pub gain2: i32,
    pub output_level: f64,
    pub time: i32,
}

pub struct VowelFilter {
    pub settings: EffectSettings,
    pub output_level: f64,
    pub formants: Vec<Formant>,
    pub taps: [FormantTap; FORMANT_COUNT],
    pub event_queue: Vec<FormantEvent>,
}

impl VowelFilter {
    pub fn new() -> Self {
        let mut filter = VowelFilter {
            settings: EffectSettings::new(),
            output_level: 1.0,
            formants: vec![Formant::new(); FORMANT_COUNT],
            taps: [FormantTap::default(); FORMANT_COUNT],
            event_queue: Vec::new(),
        };
        filter.set_formant_band(0, 800.0, 36, 3);
        filter.set_formant_band(1, 1300.0, 24, 3);
        filter.set_formant_band(2, 2200.0, 12, 3);
        filter.set_formant_band(3, 3500.0, 9, 3);
        filter.set_formant_band(4, 4500.0, 6, 3);
        filter.set_formant_band(5, 5500.0, 3, 3);
        filter
    }

    pub fn set_vowel_formants(
        &mut self,
        p_output_level: f64,
        p_frequency1: f64,
        p_gain1: i32,
        p_frequency2: f64,
        p_gain2: i32,
        p_delay: i32,
    ) {
        let event = FormantEvent {
            frequency1: p_frequency1 as i32,
            gain1: p_gain1,
            frequency2: p_frequency2 as i32,
            gain2: p_gain2,
            output_level: p_output_level,
            time: p_delay,
        };
        let pos = self
            .event_queue
            .partition_point(|event| event.time <= p_delay);
        self.event_queue.insert(pos, event);
    }

    pub fn set_formant_band(
        &mut self,
        p_index: i32,
        p_frequency: f64,
        p_gain: i32,
        p_band_index: i32,
    ) {
        crate::err_fail_index!(
            p_index,
            "p_index",
            self.formants.len() as i32,
            "_formants.size()"
        );
        let freq_index = Formant::calculate_freq_index(p_frequency);
        self.formants[p_index as usize].update(freq_index, p_gain, p_band_index);
    }

    pub fn set_formant_band1(&mut self, p_frequency: f64, p_gain: i32, p_band_index: i32) {
        self.set_formant_band(0, p_frequency, p_gain, p_band_index);
    }

    pub fn set_formant_band2(&mut self, p_frequency: f64, p_gain: i32, p_band_index: i32) {
        self.set_formant_band(1, p_frequency, p_gain, p_band_index);
    }

    pub fn set_formant_band3(&mut self, p_frequency: f64, p_gain: i32, p_band_index: i32) {
        self.set_formant_band(2, p_frequency, p_gain, p_band_index);
    }

    pub fn set_formant_band4(&mut self, p_frequency: f64, p_gain: i32, p_band_index: i32) {
        self.set_formant_band(3, p_frequency, p_gain, p_band_index);
    }

    pub fn set_formant_band5(&mut self, p_frequency: f64, p_gain: i32, p_band_index: i32) {
        self.set_formant_band(4, p_frequency, p_gain, p_band_index);
    }

    pub fn set_formant_band6(&mut self, p_frequency: f64, p_gain: i32, p_band_index: i32) {
        self.set_formant_band(5, p_frequency, p_gain, p_band_index);
    }

    fn update_event(&mut self, p_time: i32) -> i32 {
        while let Some(front) = self.event_queue.first() {
            if front.time != 0 {
                break;
            }
            let event = self.event_queue.remove(0);
            self.formants[0].update(event.frequency1, event.gain1, 3);
            self.formants[1].update(event.frequency2, event.gain2, 2);
            self.output_level = event.output_level;
        }

        if let Some(front) = self.event_queue.first() {
            let delta = if p_time < front.time {
                p_time
            } else {
                front.time
            };
            for event in self.event_queue.iter_mut() {
                event.time -= delta;
            }
            delta
        } else {
            p_time
        }
    }

    #[allow(unused_assignments)]
    fn process_lfo_formant(p_formant: &Formant, mut p_tap: FormantTap, r_input: &mut f64) -> f64 {
        let fab1 = p_formant.ab1;
        let fa2 = p_formant.a2;
        let fb0 = p_formant.b0;
        let fb2 = p_formant.b2;

        let output =
            fb0 * (*r_input) + fab1 * p_tap.in0 + fb2 * p_tap.in1 - fab1 * p_tap.out0 - fa2 * p_tap.out1;
        p_tap.in1 = p_tap.in0;
        p_tap.in0 = *r_input;
        p_tap.out1 = p_tap.out0;
        p_tap.out0 = output;
        *r_input = output;

        output
    }

    fn process_lfo(&mut self, r_buffer: &mut [f64], p_start_index: i32, p_length: i32) {
        let start_index = (p_start_index << 1) as usize;
        let length = (p_length << 1) as usize;

        let mut i = start_index;
        while i < start_index + length {
            let mut input = r_buffer[i];

            Self::process_lfo_formant(&self.formants[0], self.taps[0], &mut input);
            Self::process_lfo_formant(&self.formants[1], self.taps[1], &mut input);
            Self::process_lfo_formant(&self.formants[2], self.taps[2], &mut input);
            Self::process_lfo_formant(&self.formants[3], self.taps[3], &mut input);
            Self::process_lfo_formant(&self.formants[4], self.taps[4], &mut input);

            let mut output = Self::process_lfo_formant(&self.formants[5], self.taps[5], &mut input);
            output = clampf(output * self.output_level, -1.0, 1.0);

            r_buffer[i] = output;
            r_buffer[i + 1] = output;
            i += 2;
        }
    }
}

impl Default for VowelFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl EffectBase for VowelFilter {
    fn settings(&self) -> &EffectSettings {
        &self.settings
    }

    fn settings_mut(&mut self) -> &mut EffectSettings {
        &mut self.settings
    }

    fn prepare_process(&mut self) -> i32 {
        self.taps = [FormantTap::default(); FORMANT_COUNT];
        1
    }

    fn process(
        &mut self,
        _p_channels: i32,
        r_buffer: &mut [f64],
        p_start_index: i32,
        p_length: i32,
    ) -> i32 {
        let mut length = p_length;
        let mut i = p_start_index;
        while i < (p_start_index + p_length) {
            let step = self.update_event(length);
            self.process_lfo(r_buffer, i, step);
            i += step;
            length -= step;
        }
        1
    }

    fn set_by_mml(&mut self, p_args: &[f64]) {
        self.output_level = get_mml_arg(p_args, 0, 100.0) / 100.0;

        let frequency1 = get_mml_arg(p_args, 1, 800.0);
        let gain1 = get_mml_arg(p_args, 2, 36.0) as i32;
        self.set_formant_band1(frequency1, gain1, 3);

        let frequency2 = get_mml_arg(p_args, 3, 1300.0);
        let gain2 = get_mml_arg(p_args, 4, 24.0) as i32;
        self.set_formant_band1(frequency2, gain2, 3);

        let frequency3 = get_mml_arg(p_args, 5, 2200.0);
        let gain3 = get_mml_arg(p_args, 6, 12.0) as i32;
        self.set_formant_band1(frequency3, gain3, 3);

        let frequency4 = get_mml_arg(p_args, 7, 3500.0);
        let gain4 = get_mml_arg(p_args, 8, 9.0) as i32;
        self.set_formant_band1(frequency4, gain4, 3);

        let frequency5 = get_mml_arg(p_args, 9, 4500.0);
        let gain5 = get_mml_arg(p_args, 10, 6.0) as i32;
        self.set_formant_band1(frequency5, gain5, 3);

        let frequency6 = get_mml_arg(p_args, 11, 5500.0);
        let gain6 = get_mml_arg(p_args, 12, 3.0) as i32;
        self.set_formant_band1(frequency6, gain6, 3);
    }
}
