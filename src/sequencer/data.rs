//! `SiMMLData` (`libSiON-cpp/src/sequencer/simml_data.{h,cpp}`).
//!
//! Compiled-song data: extends [`MMLData`] with the per-song envelope, wave,
//! sampler and voice banks, plus the ref-table stencil register/restore used
//! around playback of a compiled `MMLData`.

use std::cell::RefCell;
use std::rc::Rc;

use crate::chip::ref_table::{self as chip_ref_table, SiopmRefTable};
use crate::chip::wave::pcm_table::SiopmWavePcmTable;
use crate::chip::wave::sampler_table::SiopmWaveSamplerTable;
use crate::chip::wave::table::SiopmWaveTable;
use crate::err_fail_index;
use crate::err_print;
use crate::sequencer::base::mml_data::MMLData;
use crate::sequencer::envelope_table::SiMMLEnvelopeTable;
use crate::sequencer::ref_table::{self as mml_ref_table, ENVELOPE_TABLE_MAX, VOICE_MAX};
use crate::sequencer::voice::SiMMLVoice;

pub struct SiMMLData {
    pub base: MMLData,

    pub envelope_tables: Vec<Option<Rc<RefCell<SiMMLEnvelopeTable>>>>,
    pub wave_tables: Vec<Option<Rc<RefCell<SiopmWaveTable>>>>,
    pub sampler_tables: Vec<Option<Rc<RefCell<SiopmWaveSamplerTable>>>>,

    pub fm_voices: Vec<Option<Rc<RefCell<SiMMLVoice>>>>,
    pub pcm_voices: Vec<Option<Rc<RefCell<SiMMLVoice>>>>,
}

impl Default for SiMMLData {
    fn default() -> Self {
        SiMMLData::new()
    }
}

impl SiMMLData {
    /// C++ `clear_ref_stencils` (static).
    pub fn clear_ref_stencils() {
        let table = chip_ref_table::instance();
        let mut table = table.borrow_mut();
        table.clear_sampler_table_stencil(0);
        table.clear_sampler_table_stencil(1);
        table.clear_stencil_custom_wave_tables();
        table.clear_stencil_pcm_voices();
        drop(table);

        let mml = mml_ref_table::instance().expect("SiMMLRefTable not initialized");
        let mut mml = mml.borrow_mut();
        mml.clear_stencil_envelopes();
        mml.clear_stencil_voices();
    }

    /// C++ `register_ref_stencils`.
    pub fn register_ref_stencils(&self) {
        let table = chip_ref_table::instance();
        let mut table = table.borrow_mut();
        table.set_sampler_table_stencil(0, &self.sampler_tables[0]);
        table.set_sampler_table_stencil(1, &self.sampler_tables[1]);
        table.set_stencil_custom_wave_tables(self.wave_tables.clone());
        table.set_stencil_pcm_voices(self.pcm_voices.clone());
        drop(table);

        let mml = mml_ref_table::instance().expect("SiMMLRefTable not initialized");
        let mut mml = mml.borrow_mut();
        mml.set_stencil_envelopes(self.envelope_tables.clone());
        mml.set_stencil_voices(self.fm_voices.clone());
    }

    /// C++ `get_envelope_tables()` — the C++ returns the `vector<Ref<...>>`
    /// by value; nullable slots stay `None` here.
    pub fn get_envelope_tables(&self) -> Vec<Option<Rc<RefCell<SiMMLEnvelopeTable>>>> {
        self.envelope_tables.clone()
    }

    /// C++ `get_envelope_table`.
    pub fn get_envelope_table(&self, p_index: i32) -> Option<Rc<RefCell<SiMMLEnvelopeTable>>> {
        if p_index < 0 || p_index as usize >= ENVELOPE_TABLE_MAX {
            err_print!(
                "Index p_index = {} is out of bounds (SiMMLRefTable::ENVELOPE_TABLE_MAX = {}).",
                p_index,
                ENVELOPE_TABLE_MAX as i64
            );
            return None;
        }
        self.envelope_tables[p_index as usize].clone()
    }

    /// C++ `set_envelope_table`.
    pub fn set_envelope_table(&mut self, p_index: i32, p_envelope: Option<Rc<RefCell<SiMMLEnvelopeTable>>>) {
        err_fail_index!(
            p_index,
            "p_index",
            ENVELOPE_TABLE_MAX as i32,
            "SiMMLRefTable::ENVELOPE_TABLE_MAX"
        );
        self.envelope_tables[p_index as usize] = p_envelope;
    }

    /// C++ `get_wave_table`.
    pub fn get_wave_table(&self, p_index: i32) -> Option<Rc<RefCell<SiopmWaveTable>>> {
        let index = (p_index & (SiopmRefTable::WAVE_TABLE_MAX as i32 - 1)) as usize;
        self.wave_tables[index].clone()
    }

    /// C++ `set_wave_table`.
    pub fn set_wave_table(&mut self, p_index: i32, p_data: &[f64]) -> Rc<RefCell<SiopmWaveTable>> {
        let index = (p_index & (SiopmRefTable::WAVE_TABLE_MAX as i32 - 1)) as usize;
        let log_table: Vec<i32> = p_data
            .iter()
            .map(|&v| crate::chip::ref_table::calculate_log_table_index(v))
            .collect();
        let wave = Rc::new(RefCell::new(SiopmWaveTable::new(
            log_table,
            crate::sion_enums::PITCH_TABLE_OPM,
        )));
        self.wave_tables[index] = Some(wave.clone());
        wave
    }

    /// C++ `get_sampler_table`.
    pub fn get_sampler_table(&self, p_index: i32) -> Option<Rc<RefCell<SiopmWaveSamplerTable>>> {
        if p_index < 0 || p_index as usize >= SiopmRefTable::SAMPLER_TABLE_MAX {
            err_print!(
                "Index p_index = {} is out of bounds (SiOPMRefTable::SAMPLER_TABLE_MAX = {}).",
                p_index,
                SiopmRefTable::SAMPLER_TABLE_MAX as i64
            );
            return None;
        }
        self.sampler_tables[p_index as usize].clone()
    }

    /// C++ `set_sampler_table`.
    pub fn set_sampler_table(
        &mut self,
        p_index: i32,
        p_sampler: Option<Rc<RefCell<SiopmWaveSamplerTable>>>,
    ) {
        err_fail_index!(
            p_index,
            "p_index",
            SiopmRefTable::SAMPLER_TABLE_MAX as i32,
            "SiOPMRefTable::SAMPLER_TABLE_MAX"
        );
        self.sampler_tables[p_index as usize] = p_sampler;
    }

    /// C++ `initialize_voice`.
    pub fn initialize_voice(&mut self, p_index: i32) -> Option<Rc<RefCell<SiMMLVoice>>> {
        if p_index < 0 || p_index as usize >= VOICE_MAX {
            err_print!(
                "Index p_index = {} is out of bounds (SiMMLRefTable::VOICE_MAX = {}).",
                p_index,
                VOICE_MAX as i64
            );
            return None;
        }
        let voice = Rc::new(RefCell::new(SiMMLVoice::new()));
        self.fm_voices[p_index as usize] = Some(voice.clone());
        Some(voice)
    }

    /// C++ `set_voice`.
    pub fn set_voice(&mut self, p_index: i32, p_voice: &Rc<RefCell<SiMMLVoice>>) {
        err_fail_index!(p_index, "p_index", VOICE_MAX as i32, "SiMMLRefTable::VOICE_MAX");
        if !p_voice.borrow().is_suitable_for_fm_voice() {
            err_print!("SiMMLData: Cannot set voice data which is not suitable for FM voices.");
            return;
        }
        self.fm_voices[p_index as usize] = Some(p_voice.clone());
    }

    /// C++ `get_pcm_voice`.
    pub fn get_pcm_voice(&mut self, p_index: i32) -> Rc<RefCell<SiMMLVoice>> {
        let index = (p_index & (SiopmRefTable::PCM_DATA_MAX as i32 - 1)) as usize;
        if self.pcm_voices[index].is_none() {
            self.pcm_voices[index] = Some(SiMMLVoice::create_blank_pcm_voice(index as i32));
        }
        self.pcm_voices[index].clone().unwrap()
    }

    /// C++ `clear()`.
    pub fn clear(&mut self) {
        self.base.clear();

        for slot in self.envelope_tables.iter_mut() {
            *slot = None;
        }
        for slot in self.fm_voices.iter_mut() {
            *slot = None;
        }
        for slot in self.wave_tables.iter_mut() {
            *slot = None;
        }
        for slot in self.pcm_voices.iter_mut() {
            if let Some(voice) = slot.take()
                && let Some(wave) = &voice.borrow().wave_data
                && let Some(pcm_table) = wave.downcast_ref::<Rc<RefCell<SiopmWavePcmTable>>>()
            {
                pcm_table.borrow_mut().clear();
            }
        }
        for slot in self.sampler_tables.iter_mut() {
            *slot = None;
        }
    }

    /// C++ ctor.
    pub fn new() -> Self {
        let sampler_tables = (0..SiopmRefTable::SAMPLER_TABLE_MAX)
            .map(|_| Some(Rc::new(RefCell::new(SiopmWaveSamplerTable::new()))))
            .collect();
        SiMMLData {
            base: MMLData::new(),
            envelope_tables: vec![None; ENVELOPE_TABLE_MAX],
            wave_tables: vec![None; SiopmRefTable::WAVE_TABLE_MAX],
            sampler_tables,
            fm_voices: vec![None; VOICE_MAX],
            pcm_voices: vec![None; SiopmRefTable::PCM_DATA_MAX],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sequencer::ref_table as sequencer_ref_table;

    #[test]
    fn pcm_voice_lazily_blank_and_cleared() {
        chip_ref_table::initialize();
        sequencer_ref_table::initialize();
        let mut data = SiMMLData::new();
        let voice = data.get_pcm_voice(5);
        assert!(voice.borrow().is_pcm_voice());
        data.clear();
        assert!(data.pcm_voices[5].is_none());
        assert!(data.get_pcm_voice(5 + SiopmRefTable::PCM_DATA_MAX as i32)
            .borrow()
            .is_pcm_voice());
    }

    #[test]
    fn set_wave_table_builds_log_table() {
        chip_ref_table::initialize();
        sequencer_ref_table::initialize();
        let mut data = SiMMLData::new();
        let wave = data.set_wave_table(0x100 + 3, &[1.0, 0.5]);
        let wt = data.get_wave_table(3).unwrap();
        assert!(Rc::ptr_eq(&wave, &wt));
    }
}

