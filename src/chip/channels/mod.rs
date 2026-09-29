//! SiOPM channel core (wave-6a; concrete channels landed in wave-6b1).
//!
//! Module root for the port of `libSiON-cpp/src/chip/channels/`:
//! - [`operator`] — `chip/siopm_operator.{h,cpp}` (`SiOPMOperator`)
//! - [`channel_base`] — `chip/channels/siopm_channel_base.{h,cpp}`
//!   (`SiOPMChannelBase` ABC: struct `ChannelBase` + trait `ChannelBaseTrait`)
//! - [`manager`] — `chip/channels/siopm_channel_manager.{h,cpp}`
//!   (`SiOPMChannelManager`)
//! - [`channel_fm`] / [`channel_ks`] / [`channel_pcm`] / [`channel_sampler`]
//!   — the concrete channels (wave-6b1). `SiOPMChannelKS` inherits
//!   `SiOPMChannelFM`: per CONVENTIONS the hierarchy becomes
//!   [`channel_fm::ChannelFm`] with a `kind: FmKind::Ks(_)` variant, and
//!   [`channel_ks`] hosts the KS delay-line state, ctor and virtual bodies
//!   (dispatched by `match kind` in the shared `ChannelBaseTrait` impl).
//!
//! This root also hosts the two seams the C++ classes reach into but that
//! belong to later ports:
//! - [`Pipe`] — the ring-with-cursor stand-in for
//!   `SinglyLinkedList<int>` (`templates/singly_linked_list.h`) as used by
//!   `SiOPMSoundChip` (`get_pipe`/`get_zero_buffer`, all instances are ring
//!   lists). The cursor is list-global in C++, so it lives on the shared
//!   [`PipeRc`] handle, exactly like the C++ `Element *_cursor`.
//! - [`ChipContext`] / [`OutputStream`] — the seam traits standing in for
//!   `SiOPMSoundChip` and `SiOPMStream` (wave-6b
//!   `chip/sound_chip.rs` / `chip/stream.rs`). `SiopmSoundChip` will
//!   implement [`ChipContext`]; `SiopmStream` will implement
//!   [`OutputStream`].

pub mod channel_base;
pub mod channel_fm;
pub mod channel_ks;
pub mod channel_pcm;
pub mod channel_sampler;
pub mod manager;
pub mod operator;

#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::rc::Rc;

use super::params::operator_params::OperatorParams;

/// Ring buffer with a shared cursor; port of `SinglyLinkedList<int>` in its
/// ring configuration (`new SinglyLinkedList<int>(size, value, true)`). Every
/// pipe the chip hands out (`get_pipe`, `get_zero_buffer`) is a ring, so this
/// stand-in is ring-only; a cursor can never be "null" for a non-empty ring,
/// matching every live C++ code path.
pub struct Pipe {
    ring: Vec<i32>,
    cursor: usize,
}

impl Pipe {
    /// C++ `SinglyLinkedList<int>(p_size, p_default_value, true)` followed by
    /// `front()` (the ring ctor leaves the cursor on the first element).
    pub fn new(p_size: usize, p_default_value: i32) -> Self {
        Pipe {
            ring: vec![p_default_value; p_size],
            cursor: 0,
        }
    }

    /// C++ `size()`.
    pub fn size(&self) -> usize {
        self.ring.len()
    }

    /// Index of the current cursor element (C++ `get()` identity).
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Move the cursor to an absolute index (models `set(Element*)`).
    pub fn set_cursor(&mut self, p_index: usize) {
        self.cursor = p_index % self.ring.len();
    }

    /// C++ `front()`: reset the cursor to the first element.
    pub fn front(&mut self) {
        self.cursor = 0;
    }

    /// C++ `advance(int)`: follow the ring `p_distance` steps (negative
    /// distances move zero steps, like the C++ `for` loop).
    pub fn advance(&mut self, p_distance: i32) {
        if p_distance > 0 {
            self.cursor = (self.cursor + p_distance as usize) % self.ring.len();
        }
    }

    /// C++ `get()->value`.
    pub fn value(&self) -> i32 {
        self.ring[self.cursor]
    }

    /// C++ `get()->value = v`.
    pub fn set_value(&mut self, p_value: i32) {
        self.ring[self.cursor] = p_value;
    }

    /// C++ `Element::value = v` on the element at an absolute index, cursor
    /// not moved.
    pub fn set_value_at(&mut self, p_index: usize, p_value: i32) {
        let len = self.ring.len();
        self.ring[p_index % len] = p_value;
    }

    /// C++ `Element::value` of the element at an absolute index, cursor not
    /// moved (used for the read-only walks `SiOPMStream::write*` do through
    /// `Element::next()`).
    pub fn value_at(&self, p_index: usize) -> i32 {
        self.ring[p_index % self.ring.len()]
    }

    /// C++ index of `Element::next()` after an absolute index.
    pub fn next_index(&self, p_index: usize) -> usize {
        (p_index + 1) % self.ring.len()
    }

    /// Whole backing store (read/write without moving the cursor).
    pub fn ring_mut(&mut self) -> &mut [i32] {
        &mut self.ring
    }
}

/// Shared pipe handle (C++ `SinglyLinkedList<int>*` aliases owned by
/// `SiOPMSoundChip`; multiple channels share one list and its cursor).
pub type PipeRc = Rc<RefCell<Pipe>>;

/// Wave-6b seam for `SiOPMStream` (`chip/siopm_stream.{h,cpp}`). `SiopmStream`
/// implements this; channel code only ever reaches streams through it.
///
/// `p_start` replaces the C++ `SinglyLinkedList<int>::Element *p_data_start`:
/// the absolute cursor index of the shared [`Pipe`]; the walk follows
/// `next_index` for `p_length` samples without moving the pipe cursor.
pub trait OutputStream {
    /// C++ `write(Element*, int p_offset, int p_length, double p_volume,
    /// int p_pan)`.
    fn write(
        &mut self,
        p_data: &Pipe,
        p_start: usize,
        p_offset: i32,
        p_length: i32,
        p_volume: f64,
        p_pan: i32,
    );

    /// C++ `write_stereo(Element *left, Element *right, int p_offset,
    /// int p_length, double p_volume, int p_pan)`.
    fn write_stereo(
        &mut self,
        p_left: &Pipe,
        p_left_start: usize,
        p_right: &Pipe,
        p_right_start: usize,
        p_offset: i32,
        p_length: i32,
        p_volume: f64,
        p_pan: i32,
    );

    /// C++ `write_from_vector(std::vector<double>*, int p_start_data,
    /// int p_start_buffer, int p_length, double p_volume, int p_pan,
    /// int p_sample_channel_count)`.
    fn write_from_vector(
        &mut self,
        p_data: &[f64],
        p_start_data: i32,
        p_start_buffer: i32,
        p_length: i32,
        p_volume: f64,
        p_pan: i32,
        p_sample_channel_count: i32,
    );

    /// C++ `get_channel_count()` / `set_channel_count(int)`.
    fn get_channel_count(&self) -> i32;
    fn set_channel_count(&mut self, p_value: i32);

    /// C++ `get_buffer_ptr()` element access (interleaved stereo layout).
    fn get_buffer(&self) -> &[f64];
    fn get_buffer_mut(&mut self) -> &mut [f64];

    /// C++ `resize(int p_length)`.
    fn resize(&mut self, p_length: usize);

    /// C++ `clear()`.
    fn clear(&mut self);

    /// C++ `limit()` (`siopm_stream.h:32`; reached through this seam by
    /// `SiOPMSoundChip::end_process` `siopm_sound_chip.cpp:41`).
    fn limit(&mut self);

    /// C++ `quantize(int p_bitrate)` (`siopm_stream.h:33`,
    /// `siopm_sound_chip.cpp:43`).
    fn quantize(&mut self, p_bitrate: i32);
}

/// Wave-6b seam for `SiOPMSoundChip` (`chip/siopm_sound_chip.{h,cpp}`).
/// `SiopmSoundChip` implements this; [`channel_base::ChannelBaseTrait`] and
/// [`operator::Operator`] take `&mut dyn ChipContext` wherever the C++ code
/// dereferences its `_sound_chip` pointer.
pub trait ChipContext {
    /// C++ `get_zero_buffer()` (shared 1-element ring).
    fn get_zero_buffer(&self) -> PipeRc;

    /// C++ `get_pipe(int p_pipe_num, int p_index)`: returns the SAME shared
    /// list with its cursor rewound to `p_index`; `None` = nullptr
    /// (`ERR_FAIL_INDEX_V`).
    fn get_pipe(&mut self, p_pipe_num: i32, p_index: i32) -> Option<PipeRc>;

    /// C++ `get_buffer_length()`.
    fn get_buffer_length(&self) -> i32;

    /// C++ `get_init_operator_params()`.
    fn get_init_operator_params(&self) -> Rc<RefCell<OperatorParams>>;

    /// C++ `get_stream_slot(int)`; `None` = nullptr.
    fn get_stream_slot(&self, p_slot: usize) -> Option<Rc<RefCell<dyn OutputStream>>>;

    /// C++ `set_stream_slot(int, SiOPMStream*)`; `None` = nullptr.
    fn set_stream_slot(&mut self, p_slot: usize, p_stream: Option<Rc<RefCell<dyn OutputStream>>>);

    /// C++ `get_output_stream()`; `None` = nullptr.
    fn get_output_stream(&self) -> Option<Rc<RefCell<dyn OutputStream>>>;

    /// C++ `get_pcm_volume()` (`siopm_sound_chip.h:56`).
    fn get_pcm_volume(&self) -> f64;

    /// C++ `get_sampler_volume()` (`siopm_sound_chip.h:57`).
    fn get_sampler_volume(&self) -> f64;
}
