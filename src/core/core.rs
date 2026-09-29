//! `sion_core.{h,cpp}` — standalone `sion::initialize()/finalize()`.
//!
//! C++ `initialize()` also ran `SinglyLinkedList<int/double>::initialize_pool`;
//! the Rust port has no element pools (`SinglyLinkedList` / `Pipe` are
//! heap-`Vec` based), so those two steps have no Rust counterpart.
//! C++ `finalize()`'s `SiOPMChannelFM::finalize_pool` is covered by
//! `SiopmSoundChip::Drop` → `manager::finalize()` in this port.

use crate::chip::ref_table as chip_ref_table;
use crate::sequencer::base::{mml_parser, mml_sequencer};
use crate::sequencer::ref_table as mml_ref_table;
use crate::sequencer::track::SiMMLTrack;

/// Must be called before using any SiON functionality.
///
/// C++ `sion::initialize()`.
pub fn initialize() {
    mml_parser::initialize();
    mml_sequencer::initialize();
    chip_ref_table::initialize();
    mml_ref_table::initialize();
    SiMMLTrack::initialize_statics();
}

/// Releases the static state built by [`initialize`].
///
/// C++ `sion::finalize()`. Release all driver/data objects BEFORE calling
/// (the parser arena dies here; freeing sequences afterwards is UB in C++
/// and panics on the Rust arena indices).
pub fn finalize() {
    SiMMLTrack::finalize_statics();
    mml_ref_table::finalize();
    chip_ref_table::finalize();
    mml_sequencer::finalize();
    mml_parser::finalize();
}
