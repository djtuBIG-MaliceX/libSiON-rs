use std::cell::RefCell;
use std::rc::Rc;

/// Replacement for the compat `SampleData` Variant (src/compat/sion_audio.h):
/// the union type accepted as wave/sampler/PCM input.
#[derive(Clone, Debug)]
pub enum SampleData {
    Nil,
    /// `PackedInt32Array` wave data.
    Int32Array(Rc<RefCell<Vec<i32>>>),
    /// `PackedFloat32Array` wave data.
    Float32Array(Rc<RefCell<Vec<f32>>>),
    /// `Ref<AudioStreamWAV>` — decoded sample frames + format.
    Wave(Rc<RefCell<AudioStreamWav>>),
}

impl Default for SampleData {
    fn default() -> Self {
        SampleData::Nil
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WavFormat {
    #[default]
    Unknown,
    Pcm8,
    Pcm16,
    ImaAdpcm,
    Float32,
}

#[derive(Clone, Debug, Default)]
pub struct AudioStreamWav {
    pub format: WavFormat,
    /// Raw sample bytes, as Godot's `PackedByteArray` `data` carried them
    /// (PCM8 = one int8 per sample, PCM16 = little-endian int16 pairs).
    pub data: Vec<u8>,
    pub stereo: bool,
    pub loop_start: i32,
    pub loop_end: i32,
}

impl AudioStreamWav {
    /// Ordinal matching the C++ `AudioStreamWAV::Format` enum values used in
    /// error messages (`%d` expansion): FORMAT_NONE=0, FORMAT_8_BITS=1,
    /// FORMAT_16_BITS=2, FORMAT_IMA_ADPCM=3. `Float32` has no C++ counterpart.
    pub fn cpp_ord(&self) -> i32 {
        match self.format {
            WavFormat::Unknown => 0,
            WavFormat::Pcm8 => 1,
            WavFormat::Pcm16 => 2,
            WavFormat::ImaAdpcm => 3,
            WavFormat::Float32 => 4,
        }
    }

    /// C++ `PackedByteArray::decode_s8` (compat/sion_audio.h).
    pub fn decode_s8(&self, p_offset: usize) -> i32 {
        self.data[p_offset] as i8 as i32
    }

    /// C++ `PackedByteArray::decode_s16`, little-endian (compat/sion_audio.h).
    pub fn decode_s16(&self, p_offset: usize) -> i32 {
        let raw = (self.data[p_offset] as u16) | ((self.data[p_offset + 1] as u16) << 8);
        raw as i16 as i32
    }
}

impl SampleData {
    pub fn from_floats(v: Vec<f32>) -> Self {
        SampleData::Float32Array(Rc::new(RefCell::new(v)))
    }
    pub fn from_int32s(v: Vec<i32>) -> Self {
        SampleData::Int32Array(Rc::new(RefCell::new(v)))
    }
    pub fn from_wave(w: AudioStreamWav) -> Self {
        SampleData::Wave(Rc::new(RefCell::new(w)))
    }

    /// C++ `get_type() != NIL`.
    pub fn is_valid(&self) -> bool {
        !matches!(self, SampleData::Nil)
    }
}
