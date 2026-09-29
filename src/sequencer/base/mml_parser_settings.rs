//! Port of `libSiON-cpp/src/sequencer/base/mml_parser_settings.{h,cpp}` —
//! parse-time settings for the MML parser.

#[derive(Clone, Debug)]
pub struct MMLParserSettings {
    /// Offset from MML notes to MIDI note numbers. Calculated from the default
    /// octave.
    mml_to_note_number: i32,
    default_octave: i32,

    /// Resolution of the note length. `resolution/4` is a length of a beat.
    pub resolution: i32,
    pub default_bpm: f64,

    /// Default value of the l command.
    pub default_l_value: i32,

    /// Minimum ratio of the q command.
    pub min_quant_ratio: i32,
    /// Maximum ratio of the q command.
    pub max_quant_ratio: i32,
    /// Default value of the q command.
    pub default_quant_ratio: i32,
    /// Minimum value of the @q command.
    pub min_quant_count: i32,
    /// Maximum value of the @q command.
    pub max_quant_count: i32,
    /// Default value of the @q command.
    pub default_quant_count: i32,

    /// Maximum value of the v command.
    pub max_volume: i32,
    /// Default value of the v command.
    pub default_volume: i32,
    /// Maximum value of the @v command.
    pub max_fine_volume: i32,
    /// Default value of the @v command.
    pub default_fine_volume: i32,

    /// Minimum value of the o command.
    pub min_octave: i32,
    /// Maximum value of the o command.
    pub max_octave: i32,

    /// Polarization of the ( and ) command. 1=x68k/-1=pc98.
    pub volume_polarization: i32,
    /// Polarization of the < and > command. 1=x68k/-1=pc98.
    pub octave_polarization: i32,
}

impl MMLParserSettings {
    pub fn get_mml_to_note_offset(&self) -> i32 {
        self.mml_to_note_number
    }

    /// Default value of length in MML event.
    pub fn get_default_length(&self) -> i32 {
        self.resolution / self.default_l_value
    }

    pub fn get_default_octave(&self) -> i32 {
        self.default_octave
    }

    pub fn set_default_octave(&mut self, p_value: i32) {
        self.default_octave = p_value;
        self.mml_to_note_number = 60 - self.default_octave * 12;

        let octave_limit = ((128 - self.mml_to_note_number) / 12) - 1;
        if self.max_octave > octave_limit {
            self.max_octave = octave_limit;
        }
    }
}

impl Default for MMLParserSettings {
    /// `MMLParserSettings()` — note that the original method takes an
    /// initialization object; nothing uses the feature, it was removed
    /// upstream, so `Default` matches the C++ constructor exactly.
    fn default() -> Self {
        let mut s = Self {
            mml_to_note_number: 0,
            default_octave: 0,
            resolution: 1920,
            default_bpm: 120.0,
            default_l_value: 4,
            min_quant_ratio: 0,
            max_quant_ratio: 8,
            default_quant_ratio: 10,
            min_quant_count: -192,
            max_quant_count: 192,
            default_quant_count: 0,
            max_volume: 15,
            default_volume: 10,
            max_fine_volume: 127,
            default_fine_volume: 127,
            min_octave: 0,
            max_octave: 9,
            volume_polarization: 1,
            octave_polarization: 1,
        };
        s.set_default_octave(5);
        s
    }
}
