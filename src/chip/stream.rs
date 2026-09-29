//! Port of `libSiON-cpp/src/chip/siopm_stream.{h,cpp}` (`SiOPMStream`).
//!
//! The chip's interleaved-stereo accumulation buffer. Wave-6a's
//! [`OutputStream`] trait (`chip/channels/mod.rs`) is the seam channels
//! reach streams through; `limit`/`quantize` were added to that trait here
//! (wave-6b2) because C++ `SiOPMSoundChip::end_process` calls them through
//! the concrete `SiOPMStream*`.
//!
//! Pan/volume math is frozen against the C++ exactly, including its quirks:
//! `write_stereo` scales the stereo pair by the RAW `p_volume` (no `i2n`)
//! while `write` uses `p_volume * i2n`; `write_from_vector` never applies
//! `i2n` at all and the mono-source stereo fan-out uses the literal
//! `0.707` factor.

use crate::chip::channels::{OutputStream, Pipe};
use crate::chip::ref_table;

/// C++ `SiOPMStream`.
pub struct SiopmStream {
    channels: i32,
    buffer: Vec<f64>,
}

impl SiopmStream {
    /// C++ `SiOPMStream()` (members default to `channels = 2`, empty buffer).
    pub fn new() -> Self {
        SiopmStream {
            channels: 2,
            buffer: Vec::new(),
        }
    }

    /// C++ `resize(int p_length)`. `std::vector::resize` keeps the leading
    /// elements and value-initializes the new slots to 0 (the C++
    /// "TODO zeroed" slots are in fact zeroed); `Vec::resize(n, 0.0)` is the
    /// exact match.
    pub fn resize(&mut self, p_length: usize) {
        self.buffer.resize(p_length, 0.0);
    }

    /// C++ `clear()`: zero every slot (does NOT shrink).
    pub fn clear(&mut self) {
        for value in self.buffer.iter_mut() {
            *value = 0.0;
        }
    }

    /// C++ `limit()`: clamp buffered signals between -1 and 1.
    pub fn limit(&mut self) {
        for value in self.buffer.iter_mut() {
            *value = value.clamp(-1.0, 1.0);
        }
    }

    /// C++ `quantize(int p_bitrate)`. C++ `(int)(x * r)` truncates toward
    /// zero (UB out of `int` range); Rust `as i32` truncates identically in
    /// range (`debug_assert` free like the C++ release build).
    pub fn quantize(&mut self, p_bitrate: i32) {
        let r = (1i32 << p_bitrate) as f64;
        let ir = 2.0 / r;
        for value in self.buffer.iter_mut() {
            let n = (*value * r) as i32;
            *value = (n >> 1) as f64 * ir;
        }
    }

    /// `SiOPMRefTable::get_instance()->i2n` (snapshot, per CONVENTIONS).
    fn i2n() -> f64 {
        ref_table::instance().borrow().i2n
    }

    /// `SiOPMRefTable::get_instance()->pan_table[128 - p_pan] / [p_pan]`
    /// (snapshot). Out-of-range pan is OOB UB in C++ (every live caller
    /// clamps pan to 0..=128); the Rust port panics instead.
    fn pan_pair(p_pan: i32) -> (f64, f64) {
        let table = ref_table::instance();
        let table = table.borrow();
        (
            table.pan_table[(128 - p_pan) as usize],
            table.pan_table[p_pan as usize],
        )
    }
}

impl Default for SiopmStream {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputStream for SiopmStream {
    /// C++ `write(Element *p_data_start, int p_offset, int p_length,
    /// double p_volume, int p_pan)`. `p_start` replaces the element pointer:
    /// walk the ring with `next_index`, never move the pipe cursor.
    fn write(
        &mut self,
        p_data: &Pipe,
        p_start: usize,
        p_offset: i32,
        p_length: i32,
        p_volume: f64,
        p_pan: i32,
    ) {
        let volume = p_volume * Self::i2n();
        let buffer_size = (p_offset + p_length) << 1;

        if self.channels == 2 {
            // stereo
            let (pan_left, pan_right) = Self::pan_pair(p_pan);
            let volume_left = pan_left * volume;
            let volume_right = pan_right * volume;
            let mut current = p_start;
            let mut i = (p_offset << 1) as usize;
            while (i as i32) < buffer_size {
                let value = p_data.value_at(current) as f64;
                self.buffer[i] += value * volume_left;
                i += 1;
                self.buffer[i] += value * volume_right;
                i += 1;

                current = p_data.next_index(current);
            }
        } else if self.channels == 1 {
            // mono
            let mut current = p_start;
            let mut i = (p_offset << 1) as usize;
            while (i as i32) < buffer_size {
                let value = p_data.value_at(current) as f64;
                self.buffer[i] += value * volume;
                i += 1;
                self.buffer[i] += value * volume;
                i += 1;

                current = p_data.next_index(current);
            }
        }
    }

    /// C++ `write_stereo(...)` — note the C++ quirk: the stereo branch
    /// scales by the raw `p_volume` (NOT the `i2n`-scaled `volume`) while
    /// the mono branch uses `volume * 0.5`.
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
    ) {
        let volume = p_volume * Self::i2n();
        let buffer_size = (p_offset + p_length) << 1;

        if self.channels == 2 {
            // stereo
            let (pan_left, pan_right) = Self::pan_pair(p_pan);
            let volume_left = pan_left * p_volume;
            let volume_right = pan_right * p_volume;

            let mut current_left = p_left_start;
            let mut current_right = p_right_start;
            let mut i = (p_offset << 1) as usize;
            while (i as i32) < buffer_size {
                self.buffer[i] += p_left.value_at(current_left) as f64 * volume_left;
                i += 1;
                self.buffer[i] += p_right.value_at(current_right) as f64 * volume_right;
                i += 1;

                current_left = p_left.next_index(current_left);
                current_right = p_right.next_index(current_right);
            }
        } else if self.channels == 1 {
            // mono
            let volume = volume * 0.5;

            let mut current_left = p_left_start;
            let mut current_right = p_right_start;
            let mut i = (p_offset << 1) as usize;
            while (i as i32) < buffer_size {
                let sum = p_left.value_at(current_left) as f64 + p_right.value_at(current_right) as f64;
                self.buffer[i] += sum * volume;
                i += 1;
                self.buffer[i] += sum * volume;
                i += 1;

                current_left = p_left.next_index(current_left);
                current_right = p_right.next_index(current_right);
            }
        }
    }

    /// C++ `write_from_vector(std::vector<double> *p_data, int p_start_data,
    /// int p_start_buffer, int p_length, double p_volume, int p_pan,
    /// int p_sample_channel_count)`. No `i2n` scaling (levels arrive
    /// normalized). C++ self-send aliasing (`p_data` = this buffer,
    /// reachable when a slot stream sends to its own slot index) reads and
    /// writes in place; the Rust borrow check panics on that pathological
    /// configuration — no wave-6b caller produces it.
    fn write_from_vector(
        &mut self,
        p_data: &[f64],
        p_start_data: i32,
        p_start_buffer: i32,
        p_length: i32,
        p_volume: f64,
        p_pan: i32,
        p_sample_channel_count: i32,
    ) {
        let volume = p_volume;

        if self.channels == 2 {
            if p_sample_channel_count == 2 {
                // stereo data to stereo buffer
                let (pan_left, pan_right) = Self::pan_pair(p_pan);
                let volume_left = pan_left * volume;
                let volume_right = pan_right * volume;
                let buffer_size = (p_start_data + p_length) << 1;

                let mut j = (p_start_data << 1) as usize;
                let mut i = (p_start_buffer << 1) as usize;
                while (j as i32) < buffer_size {
                    self.buffer[i] += p_data[j] * volume_left;
                    j += 1;
                    i += 1;
                    self.buffer[i] += p_data[j] * volume_right;
                    j += 1;
                    i += 1;
                }
            } else {
                // mono data to stereo buffer
                let (pan_left, pan_right) = Self::pan_pair(p_pan);
                let volume_left = pan_left * volume * 0.707;
                let volume_right = pan_right * volume * 0.707;
                let buffer_size = p_start_data + p_length;

                let mut j = p_start_data as usize;
                let mut i = (p_start_buffer << 1) as usize;
                while (j as i32) < buffer_size {
                    self.buffer[i] += p_data[j] * volume_left;
                    i += 1;
                    self.buffer[i] += p_data[j] * volume_right;
                    i += 1;
                    j += 1;
                }
            }
        } else if self.channels == 1 {
            if p_sample_channel_count == 2 {
                // stereo data to mono buffer
                let volume = volume * 0.5;
                let buffer_size = (p_start_data + p_length) << 1;

                let mut j = (p_start_data << 1) as usize;
                let mut i = (p_start_buffer << 1) as usize;
                while (j as i32) < buffer_size {
                    let sum = p_data[j] + p_data[j + 1];
                    self.buffer[i] += sum * volume;
                    i += 1;
                    self.buffer[i] += sum * volume;
                    i += 1;
                    j += 2;
                }
            } else {
                // mono data to mono buffer
                let buffer_size = p_start_data + p_length;

                let mut j = p_start_data as usize;
                let mut i = (p_start_buffer << 1) as usize;
                while (j as i32) < buffer_size {
                    self.buffer[i] += p_data[j] * volume;
                    i += 1;
                    self.buffer[i] += p_data[j] * volume;
                    i += 1;
                    j += 1;
                }
            }
        }
    }

    fn get_channel_count(&self) -> i32 {
        self.channels
    }

    fn set_channel_count(&mut self, p_value: i32) {
        self.channels = p_value;
    }

    fn get_buffer(&self) -> &[f64] {
        &self.buffer
    }

    fn get_buffer_mut(&mut self) -> &mut [f64] {
        &mut self.buffer
    }

    fn resize(&mut self, p_length: usize) {
        Self::resize(self, p_length)
    }

    fn clear(&mut self) {
        Self::clear(self)
    }

    fn limit(&mut self) {
        Self::limit(self)
    }

    fn quantize(&mut self, p_bitrate: i32) {
        Self::quantize(self, p_bitrate)
    }
}
