//! Port of `libSiON-cpp/src/chip/wave/siopm_wave_sampler_table.{h,cpp}`.

use std::cell::RefCell;
use std::rc::Rc;

use super::base::SiopmWaveBase;
use super::sampler_data::SiopmWaveSamplerData;
use crate::chip::ref_table::SiopmRefTable;
use crate::sion_enums::MODULE_SAMPLE;

#[derive(Clone, Debug)]
pub struct SiopmWaveSamplerTable {
    base: SiopmWaveBase,

    // Stencil table; search sample in stencil table before seaching in this instance's own table.
    stencil: Option<Rc<RefCell<SiopmWaveSamplerTable>>>,
    table: Vec<Option<Rc<RefCell<SiopmWaveSamplerData>>>>,
}

impl SiopmWaveSamplerTable {
    /// C++ `SiOPMWaveSamplerTable::SiOPMWaveSamplerTable()`.
    pub fn new() -> Self {
        let mut table = SiopmWaveSamplerTable {
            base: SiopmWaveBase::new(MODULE_SAMPLE),
            stencil: None,
            table: Vec::new(),
        };
        table
            .table
            .resize(SiopmRefTable::SAMPLER_DATA_MAX as usize, None); // TODO zeroed

        table.clear();
        table
    }

    /// C++ `SiOPMWaveBase::get_module_type` (inherited).
    pub fn get_module_type(&self) -> i32 {
        self.base.get_module_type()
    }

    /// C++ `SiOPMWaveSamplerTable::get_stencil`.
    pub fn get_stencil(&self) -> Option<Rc<RefCell<SiopmWaveSamplerTable>>> {
        self.stencil.clone()
    }

    /// C++ `SiOPMWaveSamplerTable::set_stencil`.
    pub fn set_stencil(&mut self, p_table: Option<Rc<RefCell<SiopmWaveSamplerTable>>>) {
        self.stencil = p_table;
    }

    /// C++ `SiOPMWaveSamplerTable::get_sample`.
    ///
    /// NOTE: faithfully reproduces the original's quirks: the stencil probe
    /// condition is `_stencil->_table.size() < p_sample_number` (inverted vs.
    /// the commented-out intent), `_stencil` is dereferenced without a null
    /// check, and the index is used unchecked — an out-of-range index panics
    /// here exactly like C++ `std::vector` UB.
    pub fn get_sample(&self, p_sample_number: usize) -> Option<Rc<RefCell<SiopmWaveSamplerData>>> {
        // if (_stencil.is_valid() && _stencil->_table[p_sample_number].is_valid()) {
        if self.stencil.as_ref().unwrap().borrow().table.len() < p_sample_number {
            return self.stencil.as_ref().unwrap().borrow().table[p_sample_number].clone();
        }

        self.table[p_sample_number].clone()
    }

    /// C++ `SiOPMWaveSamplerTable::set_sample(p_sample, p_key_range_from = 0, p_key_range_to = -1)`.
    pub fn set_sample(
        &mut self,
        p_sample: &Option<Rc<RefCell<SiopmWaveSamplerData>>>,
        p_key_range_from: i32,
        p_key_range_to: i32,
    ) {
        let key_from = 0.max(p_key_range_from);
        let mut key_to = (SiopmRefTable::SAMPLER_DATA_MAX - 1).min(p_key_range_to);

        if key_to == -1 {
            key_to = key_from;
        }

        // ERR_FAIL_COND_MSG(key_from > (SiOPMRefTable::SAMPLER_DATA_MAX - 1), vformat(...));
        if key_from > (SiopmRefTable::SAMPLER_DATA_MAX - 1) {
            crate::error::err_print_body(
                &format!(
                    "Condition \"key_from > (SiOPMRefTable::SAMPLER_DATA_MAX - 1)\" is true.\nSiOPMWaveSamplerTable: Invalid sample key range, left boundary cannot be greater than {} but {} was given.",
                    (SiopmRefTable::SAMPLER_DATA_MAX - 1),
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
                    "Condition \"key_to < 0\" is true.\nSiOPMWaveSamplerTable: Invalid sample key range, right boundary cannot be less than 0 (except -1) but {} was given.",
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
                    "Condition \"key_to < key_from\" is true.\nSiOPMWaveSamplerTable: Invalid sample key range, left boundary cannot be greater than right boundary ({} > {}).",
                    key_from, key_to
                ),
                false,
            );
            return;
        }

        let mut i = key_from;
        while i <= key_to {
            self.table[i as usize] = p_sample.clone();
            i += 1;
        }
    }

    /// C++ `SiOPMWaveSamplerTable::clear`.
    pub fn clear(&mut self) {
        for i in 0..SiopmRefTable::SAMPLER_DATA_MAX as usize {
            self.table[i] = None;
        }
    }
}

impl Default for SiopmWaveSamplerTable {
    fn default() -> Self {
        Self::new()
    }
}
