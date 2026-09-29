//! Port of `libSiON-cpp/src/chip/wave/siopm_wave_pcm_table.{h,cpp}`.

use std::cell::RefCell;
use std::rc::Rc;

use super::base::SiopmWaveBase;
use super::pcm_data::SiopmWavePcmData;
use crate::chip::ref_table::SiopmRefTable;
use crate::sion_enums::MODULE_PCM;

#[derive(Clone, Debug)]
pub struct SiopmWavePcmTable {
    base: SiopmWaveBase,

    // PCM wave data assign table for each note.
    note_data_map: Vec<Option<Rc<RefCell<SiopmWavePcmData>>>>,
    note_volume_map: Vec<f64>,
    note_pan_map: Vec<i32>,
}

impl SiopmWavePcmTable {
    /// C++ `SiOPMWavePCMTable::SiOPMWavePCMTable()`.
    pub fn new() -> Self {
        let mut table = SiopmWavePcmTable {
            base: SiopmWaveBase::new(MODULE_PCM),
            note_data_map: Vec::new(),
            note_volume_map: Vec::new(),
            note_pan_map: Vec::new(),
        };
        table
            .note_data_map
            .resize(SiopmRefTable::NOTE_TABLE_SIZE, None); // TODO zeroed
        table
            .note_volume_map
            .resize(SiopmRefTable::NOTE_TABLE_SIZE, 0.0); // TODO zeroed
        table
            .note_pan_map
            .resize(SiopmRefTable::NOTE_TABLE_SIZE, 0); // TODO zeroed

        table.clear();
        table
    }

    /// C++ `SiOPMWaveBase::get_module_type` (inherited).
    pub fn get_module_type(&self) -> i32 {
        self.base.get_module_type()
    }

    /// C++ `SiOPMWavePCMTable::get_note_data`.
    pub fn get_note_data(&self, p_note: i32) -> Option<Rc<RefCell<SiopmWavePcmData>>> {
        // ERR_FAIL_INDEX_V_MSG(p_note, _note_data_map.size(), nullptr, vformat(...));
        if p_note < 0 || p_note as usize >= self.note_data_map.len() {
            crate::error::err_print_body(
                &format!(
                    "Index p_note = {} is out of bounds (_note_data_map.size() = {}). Returning: nullptr\nSiOPMWavePCMData: Trying to access note data for a note that doesn't exist ({}).",
                    p_note,
                    self.note_data_map.len(),
                    p_note
                ),
                false,
            );
            return None;
        }
        self.note_data_map[p_note as usize].clone()
    }

    /// C++ `SiOPMWavePCMTable::get_note_volume`.
    pub fn get_note_volume(&self, p_note: i32) -> f64 {
        if p_note < 0 || p_note as usize >= self.note_volume_map.len() {
            crate::error::err_print_body(
                &format!(
                    "Index p_note = {} is out of bounds (_note_volume_map.size() = {}). Returning: 0\nSiOPMWavePCMData: Trying to access note volume for a note that doesn't exist ({}).",
                    p_note,
                    self.note_volume_map.len(),
                    p_note
                ),
                false,
            );
            return 0.0;
        }
        self.note_volume_map[p_note as usize]
    }

    /// C++ `SiOPMWavePCMTable::get_note_pan`.
    pub fn get_note_pan(&self, p_note: i32) -> i32 {
        if p_note < 0 || p_note as usize >= self.note_pan_map.len() {
            crate::error::err_print_body(
                &format!(
                    "Index p_note = {} is out of bounds (_note_pan_map.size() = {}). Returning: 0\nSiOPMWavePCMData: Trying to access note pan for a note that doesn't exist ({}).",
                    p_note,
                    self.note_pan_map.len(),
                    p_note
                ),
                false,
            );
            return 0;
        }
        self.note_pan_map[p_note as usize]
    }

    /// C++ `SiOPMWavePCMTable::set_key_range_data(p_pcm_data, p_key_range_from = 0, p_key_range_to = 127)`.
    pub fn set_key_range_data(
        &mut self,
        p_pcm_data: &Option<Rc<RefCell<SiopmWavePcmData>>>,
        p_key_range_from: i32,
        p_key_range_to: i32,
    ) {
        let key_from = 0.max(p_key_range_from);
        let mut key_to = (SiopmRefTable::NOTE_TABLE_SIZE as i32 - 1).min(p_key_range_to);

        if key_to == -1 {
            key_to = key_from;
        }

        // ERR_FAIL_COND_MSG(key_from > (SiOPMRefTable::NOTE_TABLE_SIZE - 1), vformat(...));
        if key_from > (SiopmRefTable::NOTE_TABLE_SIZE as i32 - 1) {
            crate::error::err_print_body(
                &format!(
                    "Condition \"key_from > (SiOPMRefTable::NOTE_TABLE_SIZE - 1)\" is true.\nSiOPMWavePCMTable: Invalid sample key range, left boundary cannot be greater than {} but {} was given.",
                    (SiopmRefTable::NOTE_TABLE_SIZE as i32 - 1),
                    key_from
                ),
                false,
            );
            return;
        }
        // ERR_FAIL_COND_MSG(key_to < 0, vformat(...));
        if key_to < 0 {
            crate::error::err_print_body(
                &format!(
                    "Condition \"key_to < 0\" is true.\nSiOPMWavePCMTable: Invalid sample key range, right boundary cannot be less than 0 (except -1) but {} was given.",
                    key_to
                ),
                false,
            );
            return;
        }
        // ERR_FAIL_COND_MSG(key_to < key_from, vformat(...));
        if key_to < key_from {
            crate::error::err_print_body(
                &format!(
                    "Condition \"key_to < key_from\" is true.\nSiOPMWavePCMTable: Invalid sample key range, left boundary cannot be greater than right boundary ({} > {}).",
                    key_from, key_to
                ),
                false,
            );
            return;
        }

        let mut i = key_from;
        while i <= key_to {
            self.note_data_map[i as usize] = p_pcm_data.clone();
            i += 1;
        }
    }

    /// C++ `SiOPMWavePCMTable::set_key_scale_volume(p_center_note = 64, p_key_range = 0, p_volume_range = 0)`.
    pub fn set_key_scale_volume(
        &mut self,
        p_center_note: i32,
        p_key_range: f64,
        p_volume_range: f64,
    ) {
        let volume_range = p_volume_range * 0.0078125;

        let min_range = p_center_note - (p_key_range * 0.5) as i32;
        let max_range = p_center_note + (p_key_range * 0.5) as i32;
        let delta_value = if p_key_range == 0.0 {
            volume_range
        } else {
            volume_range / p_key_range
        };

        if volume_range > 0.0 {
            let mut value = 1.0 - volume_range;

            let mut i = 0i32;
            while i < min_range {
                self.note_volume_map[i as usize] = value;
                i += 1;
            }
            while i < max_range {
                self.note_volume_map[i as usize] = value;
                value += delta_value;
                i += 1;
            }
            while i < SiopmRefTable::NOTE_TABLE_SIZE as i32 {
                self.note_volume_map[i as usize] = 1.0;
                i += 1;
            }
        } else {
            let mut value = 1.0;

            let mut i = 0i32;
            while i < min_range {
                self.note_volume_map[i as usize] = 1.0;
                i += 1;
            }
            while i < max_range {
                self.note_volume_map[i as usize] = value;
                value += delta_value;
                i += 1;
            }

            value = 1.0 + volume_range;
            while i < SiopmRefTable::NOTE_TABLE_SIZE as i32 {
                self.note_volume_map[i as usize] = value;
                i += 1;
            }
        }
    }

    /// C++ `SiOPMWavePCMTable::set_key_scale_pan(p_center_note = 64, p_key_range = 0, p_pan_width = 0)`.
    pub fn set_key_scale_pan(&mut self, p_center_note: i32, p_key_range: f64, p_pan_width: f64) {
        let min_range = p_center_note - (p_key_range * 0.5) as i32;
        let max_range = p_center_note + (p_key_range * 0.5) as i32;
        let delta_value = if p_key_range == 0.0 {
            p_pan_width
        } else {
            p_pan_width / p_key_range
        };
        let mut value = -p_pan_width * 0.5;

        let mut i = 0i32;
        while i < min_range {
            self.note_pan_map[i as usize] = value as i32;
            i += 1;
        }
        while i < max_range {
            self.note_pan_map[i as usize] = value as i32;
            value += delta_value;
            i += 1;
        }

        value = p_pan_width * 0.5;
        while i < SiopmRefTable::NOTE_TABLE_SIZE as i32 {
            self.note_pan_map[i as usize] = value as i32;
            i += 1;
        }
    }

    /// C++ `SiOPMWavePCMTable::clear`.
    pub fn clear(&mut self) {
        for i in 0..SiopmRefTable::NOTE_TABLE_SIZE {
            self.note_data_map[i] = None;
            self.note_volume_map[i] = 1.0;
            self.note_pan_map[i] = 0;
        }
    }
}

impl Default for SiopmWavePcmTable {
    fn default() -> Self {
        Self::new()
    }
}
