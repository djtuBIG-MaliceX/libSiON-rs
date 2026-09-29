//! `sion_data.{h,cpp}` — `SiONData`, the user-facing `SiMMLData` wrapper.
//!
//! C++ derived `SiONData : public SiMMLData`; Rust has no inheritance, so
//! the port composes a shared `Rc<RefCell<SiMMLData>>` handle (`data`).
//! Everything that consumed a `Ref<SiONData>` down- or up-cast to the base —
//! drivers and sequencers take `data.clone()` (`MmlDataHandle::Simml` /
//! `TrackRc::sequence_on`) and stay behavior-identical.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use crate::chip::ref_table::SiopmRefTable;
use crate::chip::wave::pcm_data::SiopmWavePcmData;
use crate::chip::wave::pcm_table::SiopmWavePcmTable;
use crate::chip::wave::sampler_data::SiopmWaveSamplerData;
use crate::chip::wave::sampler_table::SiopmWaveSamplerTable;
use crate::sample_data::SampleData;
use crate::sequencer::data::SiMMLData;

use crate::core::voice::SiONVoice;

pub struct SiONData {
    pub data: Rc<RefCell<SiMMLData>>,
}

impl SiONData {
    /// C++ `SiONData()` ctor.
    pub fn new() -> Self {
        SiONData {
            data: Rc::new(RefCell::new(SiMMLData::new())),
        }
    }

    /// C++ `set_pcm_wave`. Returns `None` when the slot's voice carries no
    /// PCM table (the C++ `pcm_table.is_valid()` false path).
    pub fn set_pcm_wave(
        &self,
        p_index: i32,
        p_data: &Rc<RefCell<SampleData>>,
        p_sampling_note: f64,
        p_key_range_from: i32,
        p_key_range_to: i32,
        p_src_channel_count: i32,
        p_channel_count: i32,
    ) -> Option<Rc<RefCell<SiopmWavePcmData>>> {
        let voice = self.data.borrow_mut().get_pcm_voice(p_index);
        let pcm_table = voice
            .borrow()
            .wave_data
            .as_ref()
            .and_then(|wave| wave.downcast_ref::<Rc<RefCell<SiopmWavePcmTable>>>())
            .cloned();

        match pcm_table {
            Some(table) => {
                let pcm_data = Rc::new(RefCell::new(SiopmWavePcmData::new(
                    &p_data.borrow(),
                    (p_sampling_note * 64.0) as i32,
                    p_src_channel_count,
                    p_channel_count,
                )));
                table.borrow_mut().set_key_range_data(
                    &Some(pcm_data.clone()),
                    p_key_range_from,
                    p_key_range_to,
                );
                Some(pcm_data)
            }
            None => None,
        }
    }

    /// C++ `set_pcm_voice` — direct `_pcm_voices` store (size masked, the
    /// C++ comment "expected to be power of 2" kept: `& (len-1)`).
    pub fn set_pcm_voice(&self, p_index: i32, p_voice: &SiONVoice) {
        let mut data = self.data.borrow_mut();
        let index = (p_index & (data.pcm_voices.len() as i32 - 1)) as usize;
        data.pcm_voices[index] = Some(p_voice.voice.clone());
    }

    /// C++ `set_sampler_wave`. A null bank table is C++ deref UB; the port
    /// panics (only reachable after `set_sampler_table(bank, None)`).
    pub fn set_sampler_wave(
        &self,
        p_index: i32,
        p_data: &Rc<RefCell<SampleData>>,
        p_ignore_note_off: bool,
        p_pan: i32,
        p_src_channel_count: i32,
        p_channel_count: i32,
    ) -> Rc<RefCell<SiopmWaveSamplerData>> {
        let bank =
            (p_index >> SiopmRefTable::NOTE_BITS) & (SiopmRefTable::SAMPLER_TABLE_MAX as i32 - 1);
        let sampler_data = Rc::new(RefCell::new(SiopmWaveSamplerData::new(
            &p_data.borrow(),
            p_ignore_note_off,
            p_pan,
            p_src_channel_count,
            p_channel_count,
        )));
        let table = self
            .data
            .borrow()
            .sampler_tables[bank as usize]
            .clone()
            .expect("SiONData: sampler bank table is null (C++ deref UB)");
        table.borrow_mut().set_sample(
            &Some(sampler_data.clone()),
            p_index & (SiopmRefTable::NOTE_TABLE_SIZE as i32 - 1),
            -1,
        );
        sampler_data
    }

    /// C++ `set_sampler_table` — direct masked store.
    pub fn set_sampler_table(&self, p_bank: i32, p_table: Option<Rc<RefCell<SiopmWaveSamplerTable>>>) {
        let mut data = self.data.borrow_mut();
        let index = (p_bank & (data.sampler_tables.len() as i32 - 1)) as usize;
        data.sampler_tables[index] = p_table;
    }

    /// `SiMMLVoice::set_wave_data` pass-through used by the sampler-table
    /// flow (wave payload: `Rc::new(Rc<RefCell<T>>) as Rc<dyn Any>`).
    pub fn wave_payload<T: Any>(table: Rc<RefCell<T>>) -> Rc<dyn Any> {
        Rc::new(table) as Rc<dyn Any>
    }
}

impl Default for SiONData {
    fn default() -> Self {
        Self::new()
    }
}
