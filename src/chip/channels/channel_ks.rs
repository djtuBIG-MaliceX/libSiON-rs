//! Port of `libSiON-cpp/src/chip/channels/siopm_channel_ks.{h,cpp}`
//! (`SiOPMChannelKS`).
//!
//! Per the wave-6a design note, `SiOPMChannelKS : SiOPMChannelFM` is modeled
//! as [`ChannelFm`] with [`FmKind::Ks`]: the KS state lives in [`KsState`]
//! (embedded in the kind), the C++ KS virtual bodies are the `ks_*`
//! inherent methods on `ChannelFm` below (dispatched by the `match kind`
//! sites in `channel_fm.rs`), and [`new_channel_ks`] is the manager-factory
//! ctor (`new SiOPMChannelKS(_sound_chip)`,
//! `siopm_channel_manager.cpp:84-86`).

use std::cell::RefCell;
use std::rc::Rc;

use super::channel_base::ChannelBaseTrait;
use super::channel_fm::ChannelFm;
use super::manager::ChannelRc;
use super::operator::Operator;
use super::{ChipContext, PipeRc};
use crate::chip::ref_table::SiopmRefTable;
use crate::err_fail_index;
use crate::math::fmod;
use crate::sion_enums::{PITCH_TABLE_PCM, PULSE_NOISE_PINK};

/// C++ `SiOPMChannelKS::KS_BUFFER_SIZE` (5394 = sampling count of MIDI note
/// number=0, rounded up).
pub const KS_BUFFER_SIZE: usize = 5400;

/// C++ `SiOPMChannelKS::KSSeedType` (stored raw like the C++ enum member —
/// arbitrary `int` casts of it are legal in C++ and fall into the
/// `default:` branch, which pins the value back to `Default`).
pub const KS_SEED_DEFAULT: i32 = 0;
pub const KS_SEED_FM: i32 = 1;
pub const KS_SEED_PCM: i32 = 2;

/// Discriminant for the `SiOPMChannelFM` / `SiOPMChannelKS` pair folded into
/// one Rust type (CONVENTIONS class-hierarchy rule).
pub enum FmKind {
    /// Plain `SiOPMChannelFM`.
    Fm,
    /// `SiOPMChannelKS` state ([`KsState`]).
    Ks(KsState),
}

/// KS delay-line state (C++ `SiOPMChannelKS` members beyond FM).
pub struct KsState {
    pub seed_type: i32,
    pub seed_index: i32,
    pub delay_buffer: Vec<i32>,
    pub delay_buffer_index: f64,
    pub ks_pitch_index: i32,
    pub ks_decay_lpf: f64,
    pub ks_decay: f64,
    pub ks_mute_decay_lpf: f64,
    pub ks_mute_decay: f64,
    pub output: f64,
    pub decay_lpf: f64,
    pub decay: f64,
    pub expression: f64,
}

impl KsState {
    /// C++ member initializers (`siopm_channel_ks.h:32-47`) plus the ctor's
    /// `_ks_delay_buffer.resize(KS_BUFFER_SIZE)` (zero-filled).
    pub fn new() -> Self {
        KsState {
            seed_type: KS_SEED_DEFAULT,
            seed_index: 0,
            delay_buffer: vec![0; KS_BUFFER_SIZE],
            delay_buffer_index: 0.0,
            ks_pitch_index: 0,
            ks_decay_lpf: 0.875,
            ks_decay: 0.98,
            ks_mute_decay_lpf: 0.5,
            ks_mute_decay: 0.75,
            output: 0.0,
            decay_lpf: 0.5,
            decay: 0.75,
            expression: 1.0,
        }
    }

    /// C++ `SiOPMChannelKS::note_on()` head (the FM `note_on` operator loop
    /// and base tail run in the `ChannelBaseTrait::note_on` dispatch that
    /// calls this; `p_operators` mirrors the FM loop's receiver).
    pub fn note_on_pre(&mut self, _p_operators: &mut Vec<Option<Rc<RefCell<Operator>>>>) {
        self.output = 0.0;

        for value in self.delay_buffer.iter_mut() {
            // C++ `int *= 0.3` — double product truncated back to int.
            *value = (*value as f64 * 0.3) as i32;
        }

        self.decay_lpf = self.ks_decay_lpf;
        self.decay = self.ks_decay;
    }

    /// C++ `SiOPMChannelKS::note_off()` (full body — KS releases via the
    /// decay envelope, not the operator EG).
    pub fn note_off(&mut self) {
        self.decay_lpf = self.ks_mute_decay_lpf;
        self.decay = self.ks_mute_decay;
    }
}

/// C++ `SiOPMChannelKS(SiOPMSoundChip*)`. The FM ctor runs first — exactly
/// like a C++ base ctor, its `initialize(nullptr, 0)` virtuals resolve to
/// the FM bodies (`kind` is still `Fm`); then the KS part switches the kind
/// (the C++ vtable flip) and resizes the delay buffer.
pub fn new_channel_ks(ctx: &mut dyn ChipContext) -> ChannelRc {
    let mut channel = ChannelFm::new(ctx);
    channel.kind = FmKind::Ks(KsState::new());
    let channel: ChannelRc = Rc::new(RefCell::new(channel));
    channel
}

impl ChannelFm {
    /// C++ `set_karplus_strong_params(p_attack_rate = 48, p_decay_rate = 48,
    /// p_total_level = 0, p_fixed_pitch = 0, p_wave_shape = -1,
    /// p_tension = 8)` — Rust has no default args; pass the C++ defaults
    /// explicitly (`-1` selects `PULSE_NOISE_PINK`).
    pub fn set_karplus_strong_params(
        &mut self,
        p_attack_rate: i32,
        p_decay_rate: i32,
        p_total_level: i32,
        p_fixed_pitch: i32,
        p_wave_shape: i32,
        p_tension: i32,
        ctx: &mut dyn ChipContext,
    ) {
        let wave_shape = if p_wave_shape == -1 {
            PULSE_NOISE_PINK
        } else {
            p_wave_shape
        };

        if let FmKind::Ks(ks) = &mut self.kind {
            ks.seed_type = KS_SEED_DEFAULT;
        }

        self.set_algorithm(1, false, 0, ctx);
        self.set_feedback(0, 0, ctx);
        self.set_params_by_value(
            p_attack_rate,
            p_decay_rate,
            0,
            63,
            15,
            p_total_level,
            0,
            0,
            1,
            0,
            0,
            0,
            0,
            p_fixed_pitch,
        );

        let op = self.active_op();
        op.borrow_mut().set_pulse_generator_type(wave_shape);
        let pg_type = op.borrow().get_pulse_generator_type();
        let table = self.base.table().clone();
        match table.borrow().get_wave_table(pg_type) {
            Some(wave_table) => {
                let pt_type = wave_table.borrow().get_default_pitch_table_type();
                op.borrow_mut().set_pitch_table_type(pt_type);
            }
            None => crate::error::err_print_body("Parameter \"wave_table\" is null.", false),
        }

        // C++ `set_all_release_rate(p_tension)` — the KS override body:
        self.ks_set_decay_lpf(p_tension);
    }

    /// C++ `SiOPMChannelKS::set_parameters(std::vector<int>)`.
    pub(crate) fn ks_set_parameters(&mut self, p_params: &[i32], ctx: &mut dyn ChipContext) {
        let FmKind::Ks(ks) = &mut self.kind else {
            return;
        };
        ks.seed_type = if p_params[0] == i32::MIN {
            KS_SEED_DEFAULT
        } else {
            p_params[0]
        };
        ks.seed_index = if p_params[1] == i32::MIN { 0 } else { p_params[1] };
        let (seed_type, seed_index) = (ks.seed_type, ks.seed_index);

        match seed_type {
            KS_SEED_FM => {
                err_fail_index!(
                    seed_index,
                    "_ks_seed_index",
                    256, // SiMMLRefTable::VOICE_MAX (simml_ref_table.h:42)
                    "SiMMLRefTable::VOICE_MAX"
                );
                if let Some(rt) = crate::sequencer::ref_table::instance() {
                    let voice = rt.borrow().get_voice(seed_index);
                    if let Some(voice) = voice {
                        let params = voice.borrow().channel_params.clone();
                        let params = params.borrow();
                        self.set_channel_params(&params, false, true, ctx);
                    }
                }
            }
            KS_SEED_PCM => {
                err_fail_index!(
                    seed_index,
                    "_ks_seed_index",
                    SiopmRefTable::PCM_DATA_MAX as i32,
                    "SiOPMRefTable::PCM_DATA_MAX"
                );
                let pcm_table = self.base.table().clone().borrow().get_pcm_data(seed_index);
                if let Some(pcm_table) = pcm_table {
                    self.set_wave_data(&pcm_table, ctx);
                }
            }
            _ => {
                // C++ `default:` pins any value (including out-of-enum int
                // casts) back to KS_SEED_DEFAULT.
                ks.seed_type = KS_SEED_DEFAULT;
                self.set_params_by_value(
                    p_params[1],
                    p_params[2],
                    0,
                    63,
                    15,
                    p_params[3],
                    0,
                    0,
                    1,
                    0,
                    0,
                    0,
                    0,
                    p_params[4],
                );

                let pg_type = if p_params[5] == i32::MIN {
                    PULSE_NOISE_PINK
                } else {
                    p_params[5]
                };
                let op = self.active_op();
                op.borrow_mut().set_pulse_generator_type(pg_type);
                let pg_type = op.borrow().get_pulse_generator_type();
                let table = self.base.table().clone();
                let wave_table = table.borrow().get_wave_table(pg_type);
                if let Some(wave_table) = wave_table {
                    let pt_type = wave_table.borrow().get_default_pitch_table_type();
                    op.borrow_mut().set_pitch_table_type(pt_type);
                } else {
                    crate::error::err_print_body("Parameter \"wave_table\" is null.", false);
                }
            }
        }
    }

    /// C++ `SiOPMChannelKS::set_types(int, SiONPitchTableType)` — the pitch
    /// table type argument is ignored by KS (`p_pt_type` unused in C++).
    pub(crate) fn ks_set_types(&mut self, p_pg_type: i32) {
        if let FmKind::Ks(ks) = &mut self.kind {
            ks.seed_type = p_pg_type;
            ks.seed_index = 0;
        }
    }

    /// C++ `SiOPMChannelKS::set_all_release_rate` / `set_release_rate`
    /// (identical bodies).
    pub(crate) fn ks_set_decay_lpf(&mut self, p_value: i32) {
        if let FmKind::Ks(ks) = &mut self.kind {
            ks.ks_decay_lpf = 1.0 - p_value as f64 * 0.015625; // 1/64
        }
    }

    /// C++ `SiOPMChannelKS::initialize` head.
    pub(crate) fn ks_initialize_pre(&mut self) {
        if let FmKind::Ks(ks) = &mut self.kind {
            ks.delay_buffer_index = 0.0;
            ks.ks_pitch_index = 0;

            ks.ks_decay_lpf = 0.875;
            ks.ks_decay = 0.98;
            ks.ks_mute_decay_lpf = 0.5;
            ks.ks_mute_decay = 0.75;

            ks.output = 0.0;
            ks.decay_lpf = ks.ks_mute_decay_lpf;
            ks.decay = ks.ks_mute_decay;
            ks.expression = 1.0;
        }
    }

    /// C++ `SiOPMChannelKS::initialize` tail (after the FM/base body).
    pub(crate) fn ks_initialize_post(&mut self, _ctx: &mut dyn ChipContext) {
        if let FmKind::Ks(ks) = &mut self.kind {
            ks.seed_type = KS_SEED_DEFAULT;
            ks.seed_index = 0;
        }

        self.set_params_by_value(48, 48, 0, 63, 15, 0, 0, 0, 1, 0, 0, 0, -1, 0);
        let op = self.active_op();
        op.borrow_mut().set_pulse_generator_type(PULSE_NOISE_PINK);
        op.borrow_mut().set_pitch_table_type(PITCH_TABLE_PCM);
    }

    /// C++ `SiOPMChannelKS::_apply_karplus_strong(Element *p_buffer_start,
    /// int p_length)` — `p_start` is the absolute cursor index of
    /// `p_out_pipe` (replaces the C++ `Element*`).
    fn ks_apply_karplus_strong(&mut self, p_out_pipe: &PipeRc, p_start: usize, p_length: i32) {
        let ope0 = self.op(0);
        let table = self.base.table().clone();
        let detune = ope0.borrow().get_ptss_detune();

        let FmKind::Ks(ks) = &mut self.kind else {
            return;
        };

        let pitch_idx_max = (SiopmRefTable::PITCH_TABLE_SIZE - 1) as i32;
        let mut pitch_idx = ks.ks_pitch_index + detune + self.pitch_modulation_output_level;
        pitch_idx = pitch_idx.clamp(0, pitch_idx_max);
        let mut wave_length_max = table.borrow().pitch_wave_length[pitch_idx as usize];

        let mut target = p_start % p_out_pipe.borrow().size();

        for _ in 0..p_length {
            // Update LFO.
            self.base.lfo_timer -= self.base.lfo_timer_step;
            if self.base.lfo_timer < 0 {
                self.base.lfo_phase = (self.base.lfo_phase + 1) & 255;

                let value_base = self.base.lfo_wave_table[self.base.lfo_phase as usize];
                self.pitch_modulation_output_level =
                    (((value_base << 1) - 255) * self.pitch_modulation_depth) >> 8;

                pitch_idx = ks.ks_pitch_index + detune + self.pitch_modulation_output_level;
                pitch_idx = pitch_idx.clamp(0, pitch_idx_max);
                wave_length_max = table.borrow().pitch_wave_length[pitch_idx as usize];

                self.base.lfo_timer += self.lfo_timer_initial;
            }

            // Update KS.
            ks.delay_buffer_index += 1.0;
            if ks.delay_buffer_index >= wave_length_max {
                ks.delay_buffer_index = fmod(ks.delay_buffer_index, wave_length_max);
            }
            let buffer_index = ks.delay_buffer_index as usize;

            ks.output *= ks.decay;
            ks.output += (ks.delay_buffer[buffer_index] as f64 - ks.output) * ks.decay_lpf
                + p_out_pipe.borrow().value_at(target) as f64;

            ks.delay_buffer[buffer_index] = ks.output as i32;
            p_out_pipe.borrow_mut().set_value_at(target, ks.output as i32);
            target = p_out_pipe.borrow().next_index(target);
        }
    }

    /// C++ `SiOPMChannelKS::buffer(int p_length)` — a carbon copy of
    /// `SiOPMChannelBase::buffer` (per the C++ comment) with the KS stage
    /// inserted and `_expression` folded into the stream volumes.
    pub(crate) fn ks_buffer(&mut self, p_length: i32, ctx: &mut dyn ChipContext) {
        if self.base.is_idling {
            ChannelBaseTrait::buffer_no_process(self, p_length, ctx);
            return;
        }

        // Preserve the start of the output pipe.
        let out_pipe = self.base.out_pipe.clone().expect("out_pipe");
        let start = out_pipe.borrow().cursor();

        // Update the output pipe for the provided length.
        self.process(p_length, ctx);

        if self.base.ring_pipe.is_some() {
            self.base.apply_ring_modulation(&out_pipe, start, p_length);
        }

        self.ks_apply_karplus_strong(&out_pipe, start, p_length);

        if self.base.filter_on {
            let base = self.base_mut();
            let mut variables = base.filter_variables;
            base.apply_sv_filter(&out_pipe, start, p_length, &mut variables);
            base.filter_variables = variables;
        }

        if self.base.output_mode == super::channel_base::OutputMode::Standard && !self.base.mute {
            let expression = match &self.kind {
                FmKind::Ks(ks) => ks.expression,
                FmKind::Fm => 1.0,
            };
            let buffer_index = self.base.buffer_index;
            let pan = self.base.pan;
            let volumes = self.base.volumes.clone();
            let streams = self.base.streams.clone();

            if self.base.has_effect_send {
                for i in 0..crate::chip::params::channel_params::STREAM_SEND_SIZE {
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
                                volumes[i] * expression,
                                pan,
                            );
                        }
                    }
                }
            } else {
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
                        volumes[0] * expression,
                        pan,
                    ),
                    None => crate::error::err_print_body("Parameter \"stream\" is null.", false),
                }
            }
        }

        self.base.buffer_index += p_length;
    }
}

