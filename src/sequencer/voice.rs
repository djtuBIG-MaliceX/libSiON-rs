//! `SiMMLVoice` (`libSiON-cpp/src/sequencer/simml_voice.{h,cpp}`).
//!
//! Base voice data object. C++ virtuals `reset`/`copy_from` are inherent
//! methods (no derived classes exist in this revision). `wave_data` stores
//! the C++ `Ref<SiOPMWaveBase>` upcast as `Rc<dyn Any>`; the concrete wave
//! classes (`SiopmWavePcmTable` / `SiopmWavePcmData` / `SiopmWaveSamplerTable`
//! / `SiopmWaveTable`) are recovered by `downcast_ref`, mirroring the
//! `Ref<X> x = wave_data` dynamic casts in the C++ queries.
//!
//! C++ quirk (ported as intended semantics, documented): the
//! `COPY_NOTE_ENVELOPE` macro in `copy_from` nulls the destination `Ref` and
//! then calls `->copy_from()` on the *nulled* pointer (UB; never exercised by
//! valid callers). The Rust port performs the obvious intent: allocate a new
//! [`SiMMLEnvelopeTable`] and copy the source into it.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use crate::chip::channels::ChipContext;
use crate::chip::params::channel_params::ChannelParams;
use crate::chip::wave::pcm_data::SiopmWavePcmData;
use crate::chip::wave::pcm_table::SiopmWavePcmTable;
use crate::chip::wave::sampler_data::SiopmWaveSamplerData;
use crate::chip::wave::sampler_table::SiopmWaveSamplerTable;
use crate::chip::wave::table::SiopmWaveTable;
use crate::sion_enums::{CHIP_SIOPM, MODULE_FM, MODULE_GENERIC_PG, MODULE_KS, MODULE_PCM};
use crate::sequencer::envelope_table::SiMMLEnvelopeTable;
use crate::sequencer::ref_table as mml_ref_table;
use crate::sequencer::track::SiMMLTrack;

pub struct SiMMLVoice {
    pub update_track_parameters: bool,
    pub update_volumes: bool,
    pub tone_num: i32,
    pub preferable_note: i32,
    pub default_gate_time: f64,
    pub default_gate_ticks: i32,
    pub default_key_on_delay_ticks: i32,
    pub note_shift: i32,
    pub portament: i32,
    pub release_sweep: i32,
    pub velocity: i32,
    pub expression: i32,
    pub velocity_mode: i32,
    pub velocity_shift: i32,
    pub expression_mode: i32,

    pub note_on_tone_envelope: Option<Rc<RefCell<SiMMLEnvelopeTable>>>,
    pub note_on_amplitude_envelope: Option<Rc<RefCell<SiMMLEnvelopeTable>>>,
    pub note_on_filter_envelope: Option<Rc<RefCell<SiMMLEnvelopeTable>>>,
    pub note_on_pitch_envelope: Option<Rc<RefCell<SiMMLEnvelopeTable>>>,
    pub note_on_note_envelope: Option<Rc<RefCell<SiMMLEnvelopeTable>>>,
    pub note_off_tone_envelope: Option<Rc<RefCell<SiMMLEnvelopeTable>>>,
    pub note_off_amplitude_envelope: Option<Rc<RefCell<SiMMLEnvelopeTable>>>,
    pub note_off_filter_envelope: Option<Rc<RefCell<SiMMLEnvelopeTable>>>,
    pub note_off_pitch_envelope: Option<Rc<RefCell<SiMMLEnvelopeTable>>>,
    pub note_off_note_envelope: Option<Rc<RefCell<SiMMLEnvelopeTable>>>,

    pub note_on_tone_envelope_step: i32,
    pub note_on_amplitude_envelope_step: i32,
    pub note_on_filter_envelope_step: i32,
    pub note_on_pitch_envelope_step: i32,
    pub note_on_note_envelope_step: i32,
    pub note_off_tone_envelope_step: i32,
    pub note_off_amplitude_envelope_step: i32,
    pub note_off_filter_envelope_step: i32,
    pub note_off_pitch_envelope_step: i32,
    pub note_off_note_envelope_step: i32,

    pub chip_type: i32,
    pub module_type: i32,
    pub channel_num: i32,
    pub pms_tension: i32,

    pub channel_params: Rc<RefCell<ChannelParams>>,
    pub wave_data: Option<Rc<dyn Any>>,

    pub pitch_shift: i32,
    pub amplitude_modulation_depth: i32,
    pub amplitude_modulation_depth_end: i32,
    pub amplitude_modulation_delay: i32,
    pub amplitude_modulation_term: i32,
    pub pitch_modulation_depth: i32,
    pub pitch_modulation_depth_end: i32,
    pub pitch_modulation_delay: i32,
    pub pitch_modulation_term: i32,
}

impl SiMMLVoice {
    fn wave_is<T: Any>(&self) -> bool {
        self.wave_data
            .as_ref()
            .is_some_and(|w| w.is::<Rc<RefCell<T>>>())
    }

    /// C++ `is_fm_voice()`.
    pub fn is_fm_voice(&self) -> bool {
        self.module_type == MODULE_FM
    }

    /// C++ `is_pcm_voice()`.
    pub fn is_pcm_voice(&self) -> bool {
        self.wave_is::<SiopmWavePcmTable>() || self.wave_is::<SiopmWavePcmData>()
    }

    /// C++ `is_sampler_voice()`.
    pub fn is_sampler_voice(&self) -> bool {
        self.wave_is::<SiopmWaveSamplerTable>()
    }

    /// C++ `is_wave_table_voice()`.
    pub fn is_wave_table_voice(&self) -> bool {
        self.wave_is::<SiopmWaveTable>()
    }

    /// C++ `is_suitable_for_fm_voice()`. The instance must be initialized
    /// (C++ dereferences `get_instance()` unguarded here).
    pub fn is_suitable_for_fm_voice(&self) -> bool {
        if self.update_track_parameters {
            return true;
        }
        let table = mml_ref_table::instance()
            .expect("SiMMLRefTable::get_instance(): uninitialized (SiMMLVoice::is_suitable_for_fm_voice)");
        let rt = table.borrow();
        rt.is_suitable_for_fm_voice(self.module_type) && self.wave_data.is_none()
    }

    /// C++ `set_module_type`.
    pub fn set_module_type(&mut self, p_module_type: i32, p_channel_num: i32, p_tone_num: i32) {
        self.module_type = p_module_type;
        self.channel_num = p_channel_num;
        self.tone_num = p_tone_num;

        let mut pg_type: i32 = -1;
        if let Some(table) = mml_ref_table::instance() {
            let rt = table.borrow();
            pg_type = rt.get_pulse_generator_type(self.module_type, self.channel_num, self.tone_num);
        }
        if pg_type != -1 {
            let op = self.channel_params.borrow().get_operator_params(0);
            if let Some(op) = op {
                op.borrow_mut().set_pulse_generator_type(pg_type);
            }
        }
    }

    /// C++ `should_update_track_parameters()`.
    pub fn should_update_track_parameters(&self) -> bool {
        self.update_track_parameters
    }

    /// C++ `wave_data->get_module_type()` — the virtual base accessor,
    /// recovered by downcasting the `Rc<dyn Any>` payload (every concrete
    /// wave class stores one).
    fn wave_module_type(p_wave_data: &Rc<dyn Any>) -> i32 {
        if let Some(wave) = p_wave_data.downcast_ref::<Rc<RefCell<SiopmWavePcmData>>>() {
            return wave.borrow().get_module_type();
        }
        if let Some(wave) = p_wave_data.downcast_ref::<Rc<RefCell<SiopmWavePcmTable>>>() {
            return wave.borrow().get_module_type();
        }
        if let Some(wave) = p_wave_data.downcast_ref::<Rc<RefCell<SiopmWaveSamplerTable>>>() {
            return wave.borrow().get_module_type();
        }
        if let Some(wave) = p_wave_data.downcast_ref::<Rc<RefCell<SiopmWaveSamplerData>>>() {
            return wave.borrow().get_module_type();
        }
        if let Some(wave) = p_wave_data.downcast_ref::<Rc<RefCell<SiopmWaveTable>>>() {
            return wave.borrow().get_module_type();
        }
        panic!("SiMMLVoice: unknown wave_data payload");
    }

    /// C++ `update_track_voice(SiMMLTrack *)` — drives the track onto this
    /// voice's module and copies every track-level setting. `ctx` replaces
    /// the C++ `_sound_chip` reached through the channel setters; the
    /// `set_channel_module_type` tone argument keeps the C++ `INT32_MIN`
    /// defaults for the FM/KS/wave branches (the `p_tone_num >= 0` gate in
    /// the track skips them). Quirk kept: the KS branch calls
    /// `set_channel_params(channel_params, false)`, so the defaulted
    /// `p_with_modulation` stays `true` there while `SELECT_TONE_FM`'s own
    /// call passes `false, false`.
    pub fn update_track_voice(&self, track: &mut SiMMLTrack, ctx: &mut dyn ChipContext) {
        match self.module_type {
            MODULE_FM => {
                // Registered FM voice (%6).
                track.set_channel_module_type(MODULE_FM, self.channel_num, i32::MIN, ctx);
            }
            MODULE_KS => {
                // PMS Guitar (%11).
                track.set_channel_module_type(MODULE_KS, 1, i32::MIN, ctx);
                let params = self.channel_params.borrow();
                track
                    .get_channel()
                    .cloned()
                    .expect("SiMMLVoice: _channel is null")
                    .borrow_mut()
                    .set_channel_params(&params, false, true, ctx);
                track
                    .get_channel()
                    .cloned()
                    .expect("SiMMLVoice: _channel is null")
                    .borrow_mut()
                    .set_all_release_rate(self.pms_tension);
                if self.is_pcm_voice() {
                    // `is_pcm_voice()` only holds when a wave payload exists.
                    let wave_data = self
                        .wave_data
                        .as_ref()
                        .expect("SiMMLVoice: pcm voice without wave_data");
                    track
                        .get_channel()
                        .cloned()
                        .expect("SiMMLVoice: _channel is null")
                        .borrow_mut()
                        .set_wave_data(&**wave_data, ctx);
                }
            }
            _ => {
                // Other sound modules.
                if let Some(wave_data) = &self.wave_data {
                    let module_type = Self::wave_module_type(wave_data);
                    track.set_channel_module_type(module_type, -1, i32::MIN, ctx);
                    let params = self.channel_params.borrow();
                    track
                        .get_channel()
                        .cloned()
                        .expect("SiMMLVoice: _channel is null")
                        .borrow_mut()
                        .set_channel_params(&params, self.update_volumes, true, ctx);
                    track
                        .get_channel()
                        .cloned()
                        .expect("SiMMLVoice: _channel is null")
                        .borrow_mut()
                        .set_wave_data(&**wave_data, ctx);
                } else {
                    track.set_channel_module_type(self.module_type, self.channel_num, self.tone_num, ctx);
                    let params = self.channel_params.borrow();
                    track
                        .get_channel()
                        .cloned()
                        .expect("SiMMLVoice: _channel is null")
                        .borrow_mut()
                        .set_channel_params(&params, self.update_volumes, true, ctx);
                }
            }
        }

        if !self.default_gate_time.is_nan() {
            track.set_quantize_ratio(self.default_gate_time);
        }

        track.set_pitch_shift(self.pitch_shift);
        track.set_note_shift(self.note_shift);
        track.set_velocity_shift(self.velocity_shift);

        track.set_velocity_mode(self.velocity_mode);
        track.set_expression_mode(self.expression_mode);
        if self.update_volumes {
            track.set_velocity(self.velocity);
            track.set_expression(self.expression);
        }

        track.set_portament(self.portament);
        track.set_release_sweep(self.release_sweep);
        track.set_modulation_envelope(
            false,
            self.amplitude_modulation_depth,
            self.amplitude_modulation_depth_end,
            self.amplitude_modulation_delay,
            self.amplitude_modulation_term,
        );
        track.set_modulation_envelope(
            true,
            self.pitch_modulation_depth,
            self.pitch_modulation_depth_end,
            self.pitch_modulation_delay,
            self.pitch_modulation_term,
        );

        track.set_tone_envelope(1, &self.note_on_tone_envelope, self.note_on_tone_envelope_step);
        track.set_amplitude_envelope(
            1,
            &self.note_on_amplitude_envelope,
            self.note_on_amplitude_envelope_step,
            false,
        );
        track.set_filter_envelope(
            1,
            &self.note_on_filter_envelope,
            self.note_on_filter_envelope_step,
        );
        track.set_pitch_envelope(
            1,
            &self.note_on_pitch_envelope,
            self.note_on_pitch_envelope_step,
        );
        track.set_note_envelope(1, &self.note_on_note_envelope, self.note_on_note_envelope_step);
        track.set_tone_envelope(0, &self.note_off_tone_envelope, self.note_off_tone_envelope_step);
        track.set_amplitude_envelope(
            0,
            &self.note_off_amplitude_envelope,
            self.note_off_amplitude_envelope_step,
            false,
        );
        track.set_filter_envelope(
            0,
            &self.note_off_filter_envelope,
            self.note_off_filter_envelope_step,
        );
        track.set_pitch_envelope(
            0,
            &self.note_off_pitch_envelope,
            self.note_off_pitch_envelope_step,
        );
        track.set_note_envelope(0, &self.note_off_note_envelope, self.note_off_note_envelope_step);
    }

    /// C++ `has_amplitude_modulation()`.
    pub fn has_amplitude_modulation(&self) -> bool {
        self.amplitude_modulation_depth > 0 || self.amplitude_modulation_depth_end > 0
    }

    /// C++ `has_pitch_modulation()`.
    pub fn has_pitch_modulation(&self) -> bool {
        self.pitch_modulation_depth > 0 || self.pitch_modulation_depth_end > 0
    }

    /// C++ `create_blank_pcm_voice`.
    pub fn create_blank_pcm_voice(p_channel_num: i32) -> Rc<RefCell<SiMMLVoice>> {
        let voice = Rc::new(RefCell::new(SiMMLVoice::new()));
        {
            let mut v = voice.borrow_mut();
            v.module_type = MODULE_PCM;
            v.channel_num = p_channel_num;
            v.wave_data = Some(Rc::new(Rc::new(RefCell::new(
                SiopmWavePcmTable::new(),
            ))) as Rc<dyn Any>);
        }
        voice
    }

    /// C++ `reset()`.
    pub fn reset(&mut self) {
        self.update_track_parameters = false;
        self.update_volumes = false;

        self.chip_type = CHIP_SIOPM;
        self.module_type = MODULE_GENERIC_PG;
        self.channel_num = -1;
        self.tone_num = -1;
        self.preferable_note = -1;

        self.channel_params.borrow_mut().initialize();
        self.wave_data = None;
        self.pms_tension = 8;

        self.default_gate_time = f64::NAN;
        self.default_gate_ticks = -1;
        self.default_key_on_delay_ticks = -1;
        self.pitch_shift = 0;
        self.note_shift = 0;
        self.portament = 0;
        self.release_sweep = 0;

        self.velocity = 256;
        self.expression = 128;
        self.velocity_mode = 0;
        self.velocity_shift = 4;
        self.expression_mode = 0;

        self.amplitude_modulation_depth = 0;
        self.amplitude_modulation_depth_end = 0;
        self.amplitude_modulation_delay = 0;
        self.amplitude_modulation_term = 0;
        self.pitch_modulation_depth = 0;
        self.pitch_modulation_depth_end = 0;
        self.pitch_modulation_delay = 0;
        self.pitch_modulation_term = 0;

        self.note_on_tone_envelope = None;
        self.note_on_amplitude_envelope = None;
        self.note_on_filter_envelope = None;
        self.note_on_pitch_envelope = None;
        self.note_on_note_envelope = None;
        self.note_off_tone_envelope = None;
        self.note_off_amplitude_envelope = None;
        self.note_off_filter_envelope = None;
        self.note_off_pitch_envelope = None;
        self.note_off_note_envelope = None;

        self.note_on_tone_envelope_step = 1;
        self.note_on_amplitude_envelope_step = 1;
        self.note_on_filter_envelope_step = 1;
        self.note_on_pitch_envelope_step = 1;
        self.note_on_note_envelope_step = 1;
        self.note_off_tone_envelope_step = 1;
        self.note_off_amplitude_envelope_step = 1;
        self.note_off_filter_envelope_step = 1;
        self.note_off_pitch_envelope_step = 1;
        self.note_off_note_envelope_step = 1;
    }

    /// C++ `copy_from`.
    pub fn copy_from(&mut self, p_source: &Rc<RefCell<SiMMLVoice>>) {
        let source = p_source.borrow();

        self.chip_type = source.chip_type;
        self.update_track_parameters = source.update_track_parameters;
        self.update_volumes = source.update_volumes;
        self.module_type = source.module_type;
        self.channel_num = source.channel_num;
        self.tone_num = source.tone_num;
        self.preferable_note = source.preferable_note;

        self.channel_params
            .borrow_mut()
            .copy_from(&source.channel_params.borrow());
        self.wave_data = source.wave_data.clone();
        self.pms_tension = source.pms_tension;

        self.default_gate_time = source.default_gate_time;
        self.default_gate_ticks = source.default_gate_ticks;
        self.default_key_on_delay_ticks = source.default_key_on_delay_ticks;
        self.pitch_shift = source.pitch_shift;
        self.note_shift = source.note_shift;
        self.portament = source.portament;
        self.release_sweep = source.release_sweep;

        self.velocity = source.velocity;
        self.expression = source.expression;
        self.velocity_mode = source.velocity_mode;
        self.velocity_shift = source.velocity_shift;
        self.expression_mode = source.expression_mode;

        self.amplitude_modulation_depth = source.amplitude_modulation_depth;
        self.amplitude_modulation_depth_end = source.amplitude_modulation_depth_end;
        self.amplitude_modulation_delay = source.amplitude_modulation_delay;
        self.amplitude_modulation_term = source.amplitude_modulation_term;
        self.pitch_modulation_depth = source.pitch_modulation_depth;
        self.pitch_modulation_depth_end = source.pitch_modulation_depth_end;
        self.pitch_modulation_delay = source.pitch_modulation_delay;
        self.pitch_modulation_term = source.pitch_modulation_term;

        macro_rules! copy_note_envelope {
            ($field:ident) => {
                self.$field = None;
                if let Some(src) = &source.$field {
                    let fresh = Rc::new(RefCell::new(SiMMLEnvelopeTable::default()));
                    fresh.borrow_mut().copy_from(src);
                    self.$field = Some(fresh);
                }
            };
        }

        copy_note_envelope!(note_on_tone_envelope);
        copy_note_envelope!(note_on_amplitude_envelope);
        copy_note_envelope!(note_on_filter_envelope);
        copy_note_envelope!(note_on_pitch_envelope);
        copy_note_envelope!(note_on_note_envelope);
        copy_note_envelope!(note_off_tone_envelope);
        copy_note_envelope!(note_off_amplitude_envelope);
        copy_note_envelope!(note_off_filter_envelope);
        copy_note_envelope!(note_off_pitch_envelope);
        copy_note_envelope!(note_off_note_envelope);

        self.note_on_tone_envelope_step = source.note_on_tone_envelope_step;
        self.note_on_amplitude_envelope_step = source.note_on_amplitude_envelope_step;
        self.note_on_filter_envelope_step = source.note_on_filter_envelope_step;
        self.note_on_pitch_envelope_step = source.note_on_pitch_envelope_step;
        self.note_on_note_envelope_step = source.note_on_note_envelope_step;
        self.note_off_tone_envelope_step = source.note_off_tone_envelope_step;
        self.note_off_amplitude_envelope_step = source.note_off_amplitude_envelope_step;
        self.note_off_filter_envelope_step = source.note_off_filter_envelope_step;
        self.note_off_pitch_envelope_step = source.note_off_pitch_envelope_step;
        self.note_off_note_envelope_step = source.note_off_note_envelope_step;
    }

    /// C++ ctor (`channel_params.instantiate(); reset();`).
    pub fn new() -> Self {
        let mut voice = SiMMLVoice {
            update_track_parameters: false,
            update_volumes: false,
            tone_num: -1,
            preferable_note: -1,
            default_gate_time: f64::NAN,
            default_gate_ticks: -1,
            default_key_on_delay_ticks: -1,
            note_shift: 0,
            portament: 0,
            release_sweep: 0,
            velocity: 256,
            expression: 128,
            velocity_mode: 0,
            velocity_shift: 4,
            expression_mode: 0,
            note_on_tone_envelope: None,
            note_on_amplitude_envelope: None,
            note_on_filter_envelope: None,
            note_on_pitch_envelope: None,
            note_on_note_envelope: None,
            note_off_tone_envelope: None,
            note_off_amplitude_envelope: None,
            note_off_filter_envelope: None,
            note_off_pitch_envelope: None,
            note_off_note_envelope: None,
            note_on_tone_envelope_step: 1,
            note_on_amplitude_envelope_step: 1,
            note_on_filter_envelope_step: 1,
            note_on_pitch_envelope_step: 1,
            note_on_note_envelope_step: 1,
            note_off_tone_envelope_step: 1,
            note_off_amplitude_envelope_step: 1,
            note_off_filter_envelope_step: 1,
            note_off_pitch_envelope_step: 1,
            note_off_note_envelope_step: 1,
            chip_type: CHIP_SIOPM,
            module_type: MODULE_GENERIC_PG,
            channel_num: -1,
            pms_tension: 8,
            channel_params: Rc::new(RefCell::new(ChannelParams::new())),
            wave_data: None,
            pitch_shift: 0,
            amplitude_modulation_depth: 0,
            amplitude_modulation_depth_end: 0,
            amplitude_modulation_delay: 0,
            amplitude_modulation_term: 0,
            pitch_modulation_depth: 0,
            pitch_modulation_depth_end: 0,
            pitch_modulation_delay: 0,
            pitch_modulation_term: 0,
        };
        voice.reset();
        voice
    }
}

impl Default for SiMMLVoice {
    fn default() -> Self {
        SiMMLVoice::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chip::ref_table as chip_ref_table;

    #[test]
    fn ctor_defaults_match_cpp() {
        chip_ref_table::initialize();
        let voice = SiMMLVoice::new();
        assert_eq!(voice.chip_type, CHIP_SIOPM);
        assert_eq!(voice.module_type, MODULE_GENERIC_PG);
        assert_eq!(voice.channel_num, -1);
        assert_eq!(voice.tone_num, -1);
        assert_eq!(voice.velocity, 256);
        assert_eq!(voice.expression, 128);
        assert_eq!(voice.velocity_shift, 4);
        assert_eq!(voice.pms_tension, 8);
        assert_eq!(voice.default_gate_ticks, -1);
        assert!(voice.default_gate_time.is_nan());
        assert!(voice.wave_data.is_none());
    }

    #[test]
    fn blank_pcm_voice_queries() {
        chip_ref_table::initialize();
        let voice = SiMMLVoice::create_blank_pcm_voice(3);
        let v = voice.borrow();
        assert_eq!(v.module_type, MODULE_PCM);
        assert_eq!(v.channel_num, 3);
        assert!(v.wave_data.is_some());
        assert!(v.is_pcm_voice());
        assert!(!v.is_sampler_voice());
        assert!(!v.is_wave_table_voice());
    }

    #[test]
    fn copy_from_deep_copies_envelopes_shares_wave() {
        chip_ref_table::initialize();
        let src = Rc::new(RefCell::new(SiMMLVoice::new()));
        {
            let mut s = src.borrow_mut();
            s.velocity = 99;
            s.note_on_tone_envelope = Some(Rc::new(RefCell::new(SiMMLEnvelopeTable::new(
                vec![1, 2, 3],
                -1,
            ))));
            s.wave_data = Some(Rc::new(Rc::new(RefCell::new(simple_table()))) as Rc<dyn Any>);
        }
        let mut dst = SiMMLVoice::new();
        dst.copy_from(&src);
        assert_eq!(dst.velocity, 99);
        // Envelope deep-copied: own Rc, same values.
        let src_env = src.borrow().note_on_tone_envelope.clone().unwrap();
        let dst_env = dst.note_on_tone_envelope.clone().unwrap();
        assert!(!Rc::ptr_eq(&src_env, &dst_env));
        assert_eq!(dst_env.borrow().get_head(), Some(0));
        // Wave data shared by reference (C++ Ref assignment).
        assert!(dst.wave_data.is_some());
        assert!(dst.is_wave_table_voice());
    }

    fn simple_table() -> SiopmWaveTable {
        SiopmWaveTable::new(vec![0; 256], crate::sion_enums::PITCH_TABLE_OPM)
    }
}
