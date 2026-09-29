//! `sion_voice.{h,cpp}` — `SiONVoice`, the user-facing voice class.
//!
//! C++ derived `SiONVoice : public SiMMLVoice`. The Rust port composes a
//! shared handle (`voice: Rc<RefCell<SiMMLVoice>>`): every consumer that
//! took `Ref<SiMMLVoice>` (ref tables, `update_track_voice`, the
//! translator's voice-setting pair) receives `voice.clone()`; `clone()`
//! replicates the C++ `copy_from(Ref<SiMMLVoice>::borrow(this)) + `_name`
//! carry-over.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::LazyLock;

use regex::Regex;

use crate::chip::ref_table as chip_ref_table;
use crate::chip::wave::pcm_data::SiopmWavePcmData;
use crate::chip::wave::pcm_table::SiopmWavePcmTable;
use crate::chip::wave::sampler_data::SiopmWaveSamplerData;
use crate::chip::wave::sampler_table::SiopmWaveSamplerTable;
use crate::chip::wave::table::SiopmWaveTable;
use crate::sample_data::SampleData;
use crate::sequencer::track::SiMMLTrack;
use crate::sequencer::voice::SiMMLVoice;
use crate::sion_enums as enums;
use crate::utils::translator_util::TranslatorUtil;

use crate::chip::channels::ChipContext;

static RE_VOICE_MML: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)(#[A-Z]*@)\s*(\d+)\s*\{(.*?)\}(.*?);").unwrap());
static RE_VOICE_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^.*?(//\s*(.+?))?[\n\r]").unwrap());

pub struct SiONVoice {
    pub voice: Rc<RefCell<SiMMLVoice>>,
    name: String,
}

impl SiONVoice {
    /// C++ ctor (and the `create()` static factory — identical in Rust).
    pub fn new(
        p_module_type: i32,
        p_channel_num: i32,
        p_attack_rate: i32,
        p_release_rate: i32,
        p_pitch_shift: i32,
        p_connection_type: i32,
        p_wave_shape2: i32,
        p_pitch_shift2: i32,
    ) -> Self {
        let voice = Rc::new(RefCell::new(SiMMLVoice::new()));
        {
            let mut v = voice.borrow_mut();
            v.update_track_parameters = true;
            // C++ default `p_tone_num = -1`.
            v.set_module_type(p_module_type, p_channel_num, -1);
            v.channel_params
                .borrow()
                .get_operator_params(0)
                .expect("operator params")
                .borrow_mut()
                .set_attack_rate(p_attack_rate);
            v.channel_params
                .borrow()
                .get_operator_params(0)
                .expect("operator params")
                .borrow_mut()
                .set_release_rate(p_release_rate);
            v.pitch_shift = p_pitch_shift;

            if p_connection_type >= 0 {
                v.channel_params.borrow_mut().set_operator_count(2);
                v.channel_params.borrow_mut().set_analog_like(true);
                v.channel_params.borrow_mut().set_algorithm(if p_connection_type <= 2 {
                    p_connection_type
                } else {
                    0
                });
                v.channel_params
                    .borrow()
                    .get_operator_params(0)
                    .expect("operator params")
                    .borrow_mut()
                    .set_pulse_generator_type(p_channel_num);
                v.channel_params
                    .borrow()
                    .get_operator_params(1)
                    .expect("operator params")
                    .borrow_mut()
                    .set_pulse_generator_type(p_wave_shape2);
                v.channel_params
                    .borrow()
                    .get_operator_params(1)
                    .expect("operator params")
                    .borrow_mut()
                    .set_detune2(p_pitch_shift2);
            }
        }
        SiONVoice { voice, name: String::new() }
    }

    /// C++ `create(...)` static factory defaults.
    pub fn create(
        p_module_type: i32,
        p_channel_num: i32,
        p_attack_rate: i32,
        p_release_rate: i32,
        p_pitch_shift: i32,
        p_connection_type: i32,
        p_wave_shape2: i32,
        p_pitch_shift2: i32,
    ) -> Self {
        Self::new(
            p_module_type,
            p_channel_num,
            p_attack_rate,
            p_release_rate,
            p_pitch_shift,
            p_connection_type,
            p_wave_shape2,
            p_pitch_shift2,
        )
    }

    /// C++ `SiONVoice()` default ctor (`(SiONModuleType)5` = generic PG).
    pub fn default_voice() -> Self {
        Self::new(enums::MODULE_GENERIC_PG, 0, 63, 63, 0, -1, 0, 0)
    }

    pub fn get_name(&self) -> String {
        self.name.clone()
    }

    pub fn set_name(&mut self, p_name: String) {
        self.name = p_name;
    }

    pub fn set_update_volumes(&self, p_value: bool) {
        self.voice.borrow_mut().update_volumes = p_value;
    }

    pub fn set_update_track_parameters(&self, p_value: bool) {
        self.voice.borrow_mut().update_track_parameters = p_value;
    }

    pub fn is_suitable_for_fm_voice(&self) -> bool {
        self.voice.borrow().is_suitable_for_fm_voice()
    }

    /// C++ `update_track_voice(SiMMLTrack*)` — forwarded to the base.
    pub fn update_track_voice(&self, track: &mut SiMMLTrack, ctx: &mut dyn ChipContext) {
        self.voice.borrow().update_track_voice(track, ctx);
    }

    fn set_params_by_translator<F>(&mut self, p_args: Vec<i32>, f: F, p_chip_type: i32)
    where
        F: FnOnce(&mut crate::chip::params::channel_params::ChannelParams, Vec<i32>),
    {
        {
            let params = self.voice.borrow().channel_params.clone();
            f(&mut params.borrow_mut(), p_args);
        }
        self.voice.borrow_mut().chip_type = p_chip_type;
    }

    pub fn set_params(&mut self, p_args: Vec<i32>) {
        self.set_params_by_translator(p_args, TranslatorUtil::set_siopm_params, enums::CHIP_SIOPM);
    }

    pub fn set_params_opl(&mut self, p_args: Vec<i32>) {
        self.set_params_by_translator(p_args, TranslatorUtil::set_opl_params, enums::CHIP_OPL);
    }

    pub fn set_params_opm(&mut self, p_args: Vec<i32>) {
        self.set_params_by_translator(p_args, TranslatorUtil::set_opm_params, enums::CHIP_OPM);
    }

    pub fn set_params_opn(&mut self, p_args: Vec<i32>) {
        self.set_params_by_translator(p_args, TranslatorUtil::set_opn_params, enums::CHIP_OPN);
    }

    pub fn set_params_opx(&mut self, p_args: Vec<i32>) {
        self.set_params_by_translator(p_args, TranslatorUtil::set_opx_params, enums::CHIP_OPX);
    }

    pub fn set_params_ma3(&mut self, p_args: Vec<i32>) {
        self.set_params_by_translator(p_args, TranslatorUtil::set_ma3_params, enums::CHIP_MA3);
    }

    pub fn set_params_al(&mut self, p_args: Vec<i32>) {
        self.set_params_by_translator(
            p_args,
            TranslatorUtil::set_al_params,
            enums::CHIP_ANALOG_LIKE,
        );
    }

    pub fn get_params(&self) -> Vec<i32> {
        let params = self.voice.borrow().channel_params.clone();
        TranslatorUtil::get_siopm_params(&params.borrow())
    }

    pub fn get_params_opl(&self) -> Vec<i32> {
        let params = self.voice.borrow().channel_params.clone();
        TranslatorUtil::get_opl_params(&params.borrow())
    }

    pub fn get_params_opm(&self) -> Vec<i32> {
        let params = self.voice.borrow().channel_params.clone();
        TranslatorUtil::get_opm_params(&params.borrow())
    }

    pub fn get_params_opn(&self) -> Vec<i32> {
        let params = self.voice.borrow().channel_params.clone();
        TranslatorUtil::get_opn_params(&params.borrow())
    }

    pub fn get_params_opx(&self) -> Vec<i32> {
        let params = self.voice.borrow().channel_params.clone();
        TranslatorUtil::get_opx_params(&params.borrow())
    }

    pub fn get_params_ma3(&self) -> Vec<i32> {
        let params = self.voice.borrow().channel_params.clone();
        TranslatorUtil::get_ma3_params(&params.borrow())
    }

    /// C++ quirk kept: `get_params_al` calls `get_ma3_params`
    /// (`sion_voice.cpp:100-103` — upstream copy-paste bug).
    pub fn get_params_al(&self) -> Vec<i32> {
        let params = self.voice.borrow().channel_params.clone();
        TranslatorUtil::get_ma3_params(&params.borrow())
    }

    /// C++ `get_mml(p_index, p_chip_type, p_append_postfix)`.
    pub fn get_mml(&self, p_index: i32, p_chip_type: i32, p_append_postfix: bool) -> String {
        let mut kind = p_chip_type;
        let chip_type = self.voice.borrow().chip_type;
        if kind == enums::CHIP_AUTO {
            kind = chip_type;
        }

        let mml: String = match kind {
            enums::CHIP_SIOPM => format!(
                "#@{}{}",
                p_index,
                self.params_as_mml(TranslatorUtil::get_siopm_params_as_mml)
            ),
            enums::CHIP_OPL => format!(
                "#OPL@{}{}",
                p_index,
                self.params_as_mml(TranslatorUtil::get_opl_params_as_mml)
            ),
            enums::CHIP_OPM => format!(
                "#OPM@{}{}",
                p_index,
                self.params_as_mml(TranslatorUtil::get_opm_params_as_mml)
            ),
            enums::CHIP_OPN => format!(
                "#OPN@{}{}",
                p_index,
                self.params_as_mml(TranslatorUtil::get_opn_params_as_mml)
            ),
            enums::CHIP_OPX => format!(
                "#OPX@{}{}",
                p_index,
                self.params_as_mml(TranslatorUtil::get_opx_params_as_mml)
            ),
            enums::CHIP_MA3 => format!(
                "#MA@{}{}",
                p_index,
                self.params_as_mml(TranslatorUtil::get_ma3_params_as_mml)
            ),
            enums::CHIP_ANALOG_LIKE => format!(
                "#AL@{}{}",
                p_index,
                self.params_as_mml(TranslatorUtil::get_al_params_as_mml)
            ),
            other => {
                crate::err_print!(
                    "SiONVoice: Chip type {} is unsupported for MML strings.",
                    other
                );
                return String::new();
            }
        };

        if p_append_postfix {
            let postfix = TranslatorUtil::get_voice_setting_as_mml(&self.voice);
            if !postfix.is_empty() {
                return format!("{}\n{};", mml, postfix);
            }
        }
        format!("{};", mml)
    }

    fn params_as_mml<F>(&self, f: F) -> String
    where
        F: FnOnce(
            &crate::chip::params::channel_params::ChannelParams,
            &str,
            &str,
            &str,
        ) -> String,
    {
        let params = self.voice.borrow().channel_params.clone();
        f(&params.borrow(), " ", "\n", &self.name)
    }

    /// C++ `set_by_mml(p_mml)` — returns the voice index, `-1` on failure.
    pub fn set_by_mml(&mut self, p_mml: &str) -> i32 {
        self.reset();

        let res = match RE_VOICE_MML.captures(p_mml) {
            Some(res) => res,
            None => return -1,
        };
        let command = res
            .get(1)
            .map(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        let data = res.get(3).map(|m| m.as_str()).unwrap_or("").to_string();

        let kind = if command == "#@" {
            enums::CHIP_SIOPM
        } else if command == "#OPL@" {
            enums::CHIP_OPL
        } else if command == "#OPM@" {
            enums::CHIP_OPM
        } else if command == "#OPN@" {
            enums::CHIP_OPN
        } else if command == "#OPX@" {
            enums::CHIP_OPX
        } else if command == "#MA@" {
            enums::CHIP_MA3
        } else if command == "#AL@" {
            enums::CHIP_ANALOG_LIKE
        } else {
            return -1;
        };

        {
            let params = self.voice.borrow().channel_params.clone();
            let mut params = params.borrow_mut();
            match kind {
                enums::CHIP_SIOPM => TranslatorUtil::parse_siopm_params(&mut params, &data),
                enums::CHIP_OPL => TranslatorUtil::parse_opl_params(&mut params, &data),
                enums::CHIP_OPM => TranslatorUtil::parse_opm_params(&mut params, &data),
                enums::CHIP_OPN => TranslatorUtil::parse_opn_params(&mut params, &data),
                enums::CHIP_OPX => TranslatorUtil::parse_opx_params(&mut params, &data),
                enums::CHIP_MA3 => TranslatorUtil::parse_ma3_params(&mut params, &data),
                _ => TranslatorUtil::parse_al_params(&mut params, &data),
            }
        }
        self.voice.borrow_mut().chip_type = kind;

        let postfix = res.get(4).map(|m| m.as_str()).unwrap_or("").to_string();
        let voice_index = res
            .get(2)
            .map(|m| crate::utils::string::to_int(m.as_str()))
            .unwrap_or(-1) as i32;
        TranslatorUtil::parse_voice_setting(&self.voice, &postfix, Vec::new());

        match RE_VOICE_NAME.captures(&data) {
            Some(name_res) => {
                self.name = name_res
                    .get(2)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default();
            }
            None => self.name = String::new(),
        }

        voice_index
    }

    /// C++ `set_wave_table(std::vector<double> *)`.
    pub fn set_wave_table(&mut self, p_data: &[f64]) -> Rc<RefCell<SiopmWaveTable>> {
        let table: Vec<i32> = p_data
            .iter()
            .map(|value| chip_ref_table::calculate_log_table_index(*value))
            .collect();
        let wave_table = Rc::new(RefCell::new(SiopmWaveTable::new(
            table,
            crate::sion_enums::PITCH_TABLE_OPM,
        )));
        {
            let mut v = self.voice.borrow_mut();
            v.module_type = enums::MODULE_SCC;
            v.wave_data = Some(Rc::new(wave_table.clone()) as Rc<dyn Any>);
        }
        wave_table
    }

    /// C++ `set_pcm_voice`.
    pub fn set_pcm_voice(
        &mut self,
        p_data: &Rc<RefCell<SampleData>>,
        p_sampling_note: i32,
        p_src_channel_count: i32,
        p_channel_count: i32,
    ) -> Rc<RefCell<SiopmWavePcmData>> {
        let pcm_data = Rc::new(RefCell::new(SiopmWavePcmData::new(
            &p_data.borrow(),
            p_sampling_note * 64,
            p_src_channel_count,
            p_channel_count,
        )));
        {
            let mut v = self.voice.borrow_mut();
            v.module_type = enums::MODULE_PCM;
            v.wave_data = Some(Rc::new(pcm_data.clone()) as Rc<dyn Any>);
        }
        pcm_data
    }

    /// C++ `set_pcm_wave`.
    pub fn set_pcm_wave(
        &mut self,
        p_index: i32,
        p_data: &Rc<RefCell<SampleData>>,
        p_sampling_note: i32,
        p_key_range_from: i32,
        p_key_range_to: i32,
        p_src_channel_count: i32,
        p_channel_count: i32,
    ) -> Rc<RefCell<SiopmWavePcmData>> {
        {
            let mut v = self.voice.borrow_mut();
            if v.module_type != enums::MODULE_PCM || v.channel_num != p_index {
                v.wave_data = None;
            }
            v.module_type = enums::MODULE_PCM;
            v.channel_num = p_index;

            let mut table = v
                .wave_data
                .as_ref()
                .and_then(|wave| wave.downcast_ref::<Rc<RefCell<SiopmWavePcmTable>>>())
                .cloned();
            if table.is_none() {
                let fresh = Rc::new(RefCell::new(SiopmWavePcmTable::new()));
                v.wave_data = Some(Rc::new(fresh.clone()) as Rc<dyn Any>);
                table = Some(fresh);
            }
            let table = table.unwrap();
            drop(v);

            let pcm_data = Rc::new(RefCell::new(SiopmWavePcmData::new(
                &p_data.borrow(),
                p_sampling_note * 64,
                p_src_channel_count,
                p_channel_count,
            )));
            table.borrow_mut().set_key_range_data(
                &Some(pcm_data.clone()),
                p_key_range_from,
                p_key_range_to,
            );
            return pcm_data;
        }
    }

    /// C++ `set_sampler_voice`.
    pub fn set_sampler_voice(
        &mut self,
        p_data: &Rc<RefCell<SampleData>>,
        p_ignore_note_off: bool,
        p_channel_count: i32,
    ) -> Rc<RefCell<SiopmWaveSamplerData>> {
        let sampler_data = Rc::new(RefCell::new(SiopmWaveSamplerData::new(
            &p_data.borrow(),
            p_ignore_note_off,
            0,
            2,
            p_channel_count,
        )));
        {
            let mut v = self.voice.borrow_mut();
            v.module_type = enums::MODULE_SAMPLE;
            v.wave_data = Some(Rc::new(sampler_data.clone()) as Rc<dyn Any>);
        }
        sampler_data
    }

    /// C++ `set_sampler_wave`.
    pub fn set_sampler_wave(
        &mut self,
        p_index: i32,
        p_data: &Rc<RefCell<SampleData>>,
        p_ignore_note_off: bool,
        p_pan: i32,
        p_src_channel_count: i32,
        p_channel_count: i32,
    ) -> Rc<RefCell<SiopmWaveSamplerData>> {
        let mut v = self.voice.borrow_mut();
        v.module_type = enums::MODULE_SAMPLE;

        let mut table = v
            .wave_data
            .as_ref()
            .and_then(|wave| wave.downcast_ref::<Rc<RefCell<SiopmWaveSamplerTable>>>())
            .cloned();
        if table.is_none() {
            let fresh = Rc::new(RefCell::new(SiopmWaveSamplerTable::new()));
            v.wave_data = Some(Rc::new(fresh.clone()) as Rc<dyn Any>);
            table = Some(fresh);
        }
        drop(v);

        let table = table.unwrap();
        let sampler_data = Rc::new(RefCell::new(SiopmWaveSamplerData::new(
            &p_data.borrow(),
            p_ignore_note_off,
            p_pan,
            p_src_channel_count,
            p_channel_count,
        )));
        table.borrow_mut().set_sample(
            &Some(sampler_data.clone()),
            p_index & (chip_ref_table::SiopmRefTable::NOTE_TABLE_SIZE as i32 - 1),
            -1,
        );
        sampler_data
    }

    /// C++ `set_sampler_table`.
    pub fn set_sampler_table(&mut self, p_table: &Rc<RefCell<SiopmWaveSamplerTable>>) {
        let mut v = self.voice.borrow_mut();
        v.module_type = enums::MODULE_SAMPLE;
        v.wave_data = Some(Rc::new(p_table.clone()) as Rc<dyn Any>);
    }

    /// C++ `set_pms_guitar`.
    pub fn set_pms_guitar(
        &mut self,
        p_attack_rate: i32,
        p_decay_rate: i32,
        p_total_level: i32,
        p_fixed_pitch: i32,
        p_wave_shape: i32,
        p_tension: i32,
    ) {
        {
            let mut v = self.voice.borrow_mut();
            v.module_type = enums::MODULE_KS;
            v.channel_num = 1;
        }
        self.set_params(vec![
            1,
            0,
            0,
            p_wave_shape,
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
        ]);
        {
            let mut v = self.voice.borrow_mut();
            v.pms_tension = p_tension;
            v.chip_type = enums::CHIP_PMS_GUITAR;
        }
    }

    /// C++ `set_analog_like`.
    pub fn set_analog_like(
        &mut self,
        p_connection_type: i32,
        p_wave_shape1: i32,
        p_wave_shape2: i32,
        p_balance: i32,
        p_pitch_difference: i32,
    ) {
        let params = self.voice.borrow().channel_params.clone();
        let table = chip_ref_table::instance();
        let level_table = &table.borrow().eg_linear_to_total_level_table;
        {
            let mut params = params.borrow_mut();
            params.set_operator_count(2);
            params.set_analog_like(true);
            params.set_algorithm(if (0..=3).contains(&p_connection_type) {
                p_connection_type
            } else {
                0
            });
            let op0 = params
                .get_operator_params(0)
                .expect("operator params")
                .clone();
            let op1 = params
                .get_operator_params(1)
                .expect("operator params")
                .clone();
            op0.borrow_mut().set_pulse_generator_type(p_wave_shape1);
            op1.borrow_mut().set_pulse_generator_type(p_wave_shape2);

            let balance = p_balance.clamp(-64, 64);
            op0.borrow_mut().set_total_level(level_table[(64 - balance) as usize]);
            op1.borrow_mut().set_total_level(level_table[(balance + 64) as usize]);

            op0.borrow_mut().set_detune2(0);
            op1.borrow_mut().set_detune2(p_pitch_difference);
        }
        self.voice.borrow_mut().chip_type = enums::CHIP_ANALOG_LIKE;
    }

    /// C++ `set_envelope`.
    pub fn set_envelope(
        &mut self,
        p_attack_rate: i32,
        p_decay_rate: i32,
        p_sustain_rate: i32,
        p_release_rate: i32,
        p_sustain_level: i32,
        p_total_level: i32,
    ) {
        let params = self.voice.borrow().channel_params.clone();
        let count = params.borrow().get_operator_count();
        for i in 0..count {
            let op = params
                .borrow()
                .get_operator_params(i)
                .expect("operator params")
                .clone();
            let mut op = op.borrow_mut();
            op.set_attack_rate(p_attack_rate);
            op.set_decay_rate(p_decay_rate);
            op.set_sustain_rate(p_sustain_rate);
            op.set_release_rate(p_release_rate);
            op.set_sustain_level(p_sustain_level);
            op.set_total_level(p_total_level);
        }
    }

    /// C++ `set_filter_envelope`.
    #[allow(clippy::too_many_arguments)]
    pub fn set_filter_envelope(
        &mut self,
        p_filter_type: i32,
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
        let params = self.voice.borrow().channel_params.clone();
        let mut params = params.borrow_mut();
        params.set_filter_type(p_filter_type);
        params.set_filter_cutoff(p_cutoff);
        params.set_filter_resonance(p_resonance);
        params.set_filter_attack_rate(p_attack_rate);
        params.set_filter_decay_rate1(p_decay_rate1);
        params.set_filter_decay_rate2(p_decay_rate2);
        params.set_filter_release_rate(p_release_rate);
        params.set_filter_decay_offset1(p_decay_cutoff1);
        params.set_filter_decay_offset2(p_decay_cutoff2);
        params.set_filter_sustain_offset(p_sustain_cutoff);
        params.set_filter_release_offset(p_release_cutoff);
    }

    /// C++ `set_amplitude_modulation`.
    pub fn set_amplitude_modulation(
        &mut self,
        p_depth: i32,
        p_end_depth: i32,
        p_delay: i32,
        p_term: i32,
    ) {
        let params = self.voice.borrow().channel_params.clone();
        {
            let mut v = self.voice.borrow_mut();
            v.amplitude_modulation_depth = p_depth;
            v.amplitude_modulation_depth_end = p_end_depth;
            v.amplitude_modulation_delay = p_delay;
            v.amplitude_modulation_term = p_term;
        }
        params.borrow_mut().set_amplitude_modulation_depth(p_depth);
    }

    /// C++ `set_pitch_modulation`.
    pub fn set_pitch_modulation(&mut self, p_depth: i32, p_end_depth: i32, p_delay: i32, p_term: i32) {
        let params = self.voice.borrow().channel_params.clone();
        {
            let mut v = self.voice.borrow_mut();
            v.pitch_modulation_depth = p_depth;
            v.pitch_modulation_depth_end = p_end_depth;
            v.pitch_modulation_delay = p_delay;
            v.pitch_modulation_term = p_term;
        }
        params.borrow_mut().set_pitch_modulation_depth(p_depth);
    }

    /// C++ `clone()` — base `copy_from` + `_name` carry-over.
    pub fn clone_voice(&self) -> SiONVoice {
        let new_voice = Rc::new(RefCell::new(SiMMLVoice::new()));
        new_voice.borrow_mut().copy_from(&self.voice);
        SiONVoice {
            voice: new_voice,
            name: self.name.clone(),
        }
    }

    /// C++ `reset()` override (base reset + name clear + flag re-set).
    pub fn reset(&mut self) {
        self.voice.borrow_mut().reset();
        self.name = String::new();
        self.voice.borrow_mut().update_track_parameters = true;
    }
}
