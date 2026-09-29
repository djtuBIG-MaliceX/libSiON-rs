//! Port of `libSiON-cpp/src/chip/wave/siopm_wave_table.{h,cpp}`.

use std::cell::RefCell;
use std::rc::Rc;

use super::base::SiopmWaveBase;
use crate::chip::ref_table::SiopmRefTable;
use crate::sion_enums::{MODULE_SCC, PITCH_TABLE_OPM};

#[derive(Clone, Debug)]
pub struct SiopmWaveTable {
    base: SiopmWaveBase,
    wavelet: Vec<i32>,
    fixed_bits: i32,
    // SiONPitchTableType
    default_pitch_table_type: i32,
}

impl SiopmWaveTable {
    /// C++ `SiOPMWaveTable(std::vector<int> p_wavelet = {}, SiONPitchTableType
    /// p_default_pitch_table_type = PITCH_TABLE_OPM)`.
    pub fn new(p_wavelet: Vec<i32>, p_default_pitch_table_type: i32) -> Self {
        let mut table = SiopmWaveTable {
            base: SiopmWaveBase::new(MODULE_SCC),
            wavelet: Vec::new(),
            fixed_bits: 0,
            default_pitch_table_type: PITCH_TABLE_OPM,
        };
        table.initialize(p_wavelet, p_default_pitch_table_type);
        table
    }

    /// C++ `SiOPMWaveTable::get_wavelet`.
    pub fn get_wavelet(&self) -> Vec<i32> {
        self.wavelet.clone()
    }

    /// C++ `SiOPMWaveTable::get_wavelet` used hot; borrow-free accessor for
    /// internal indexing (same data, avoids the C++ value-copy where the
    /// caller only reads).
    pub fn wavelet_slice(&self) -> &[i32] {
        &self.wavelet
    }

    /// C++ `SiOPMWaveTable::get_fixed_bits`.
    pub fn get_fixed_bits(&self) -> i32 {
        self.fixed_bits
    }

    /// C++ `SiOPMWaveTable::get_default_pitch_table_type`.
    pub fn get_default_pitch_table_type(&self) -> i32 {
        self.default_pitch_table_type
    }

    /// C++ `SiOPMWaveBase::get_module_type` (inherited).
    pub fn get_module_type(&self) -> i32 {
        self.base.get_module_type()
    }

    /// C++ `SiOPMWaveTable::initialize`.
    pub fn initialize(&mut self, p_wavelet: Vec<i32>, p_default_pitch_table_type: i32) {
        self.wavelet = p_wavelet;
        self.default_pitch_table_type = p_default_pitch_table_type;

        let mut bits = 0i32;
        let mut len = (self.wavelet.len() >> 1) as i32;
        while len != 0 {
            bits += 1;
            len >>= 1;
        }
        self.fixed_bits = SiopmRefTable::PHASE_BITS - bits;
    }

    /// C++ `SiOPMWaveTable::copy_from`.
    pub fn copy_from(&mut self, p_source: &Rc<RefCell<SiopmWaveTable>>) {
        let source = p_source.borrow();
        self.fixed_bits = source.fixed_bits;
        self.default_pitch_table_type = source.default_pitch_table_type;
        self.wavelet.clear();

        let wavelet_size = source.wavelet.len();
        for i in 0..wavelet_size {
            self.wavelet.push(source.wavelet[i]);
        }
    }
}
