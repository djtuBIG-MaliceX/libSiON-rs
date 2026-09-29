//! Port of `libSiON-cpp/src/utils/sion_voice_preset_util.{h,cpp}`.
//!
//! C++ `SiONVoicePresetUtil` is a *generator* of ~650 hardcoded voice
//! presets (default:16, valsound:258, GM:128x2, GMdrum:60x2), not a
//! serializer — there is no byte packing, file I/O or checksum logic anywhere
//! in the C++ file. Every `_generate_*` category builder and `_create_*`
//! helper constructs `Ref<SiONVoice>` objects and stores them in
//! `List`/`HashMap` registries keyed by preset name.
//!
//! Wave-9 pass A landed the class surface live: the fields, the
//! `generate_voices` entry point, the `_generate_voices` dispatcher,
//! `_generate_default_voices`, all seven `_create_*_voice` helpers,
//! `_begin_category`, `_register_voice`, `_register_wave_table`,
//! `get_voice_preset_keys` and `get_voice_preset`. The five big category
//! builders (`_generate_valsound_voices`, `_generate_midi_voices`,
//! `_generate_mididrum_voices`, `_generate_wave_table_voices`,
//! `_generate_single_drum_voices`) are `unimplemented!()` region markers
//! pending wave-9 pass B; see `docs/PENDING.md`.
//!
//! Fidelity notes:
//! - C++ `_begin_category` assigns Godot value-semantics `List` copies into
//!   both `_current_category` and `_category_map`, so later `_register_voice`
//!   `push_back` only ever grows `_current_category` and the map entries stay
//!   empty — reproduced with `Vec::clone` (quirk kept, not fixed).
//! - `get_voice_preset_keys` iterates the `HashMap`; C++ `HashMap` order is
//!   already unspecified, so Rust `HashMap` order is kept (no deterministic
//!   re-sort invented).
//! - The C++ dtor only clears the four containers (all of which die with the
//!   object), so no Rust `Drop` is implemented — RAII makes it a no-op.
//! - C++ `get_operator_params(0)` null-derefs on a missing operator; the Rust
//!   port panics via `.expect("operator params")`, matching `core/voice.rs`.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::chip::wave::table::SiopmWaveTable;
use crate::core::voice::SiONVoice;
use crate::err_fail_cond;
use crate::sion_enums as enums;

/// C++ `ERR_FAIL_COND_V_MSG(cond, nullptr, msg)` — compat prints the
/// *stringified* `nullptr` (per `sion_errors.h` `#m_retval`), so the printed
/// text and the returned `Option::None` are decoupled (same helper pattern as
/// `mml_parser.rs` / `mml_sequence_group.rs`).
macro_rules! cpp_err_fail_cond_v_msg {
    ($cond:expr, $cond_text:expr, $retval:expr, $retval_text:expr, $msg:expr) => {
        if $cond {
            $crate::error::err_print_body(
                &format!(
                    "{}\nCondition \"{}\" is true. Returning: {}",
                    $msg, $cond_text, $retval_text
                ),
                false,
            );
            return $retval;
        }
    };
}

/// C++ `SiONVoicePresetUtil::GeneratorFlags::INCLUDE_DEFAULT`.
pub const INCLUDE_DEFAULT: u32 = 1;

/// C++ `SiONVoicePresetUtil::GeneratorFlags::INCLUDE_VALSOUND`.
pub const INCLUDE_VALSOUND: u32 = 2;

/// C++ `SiONVoicePresetUtil::GeneratorFlags::INCLUDE_MIDI`.
pub const INCLUDE_MIDI: u32 = 4;

/// C++ `SiONVoicePresetUtil::GeneratorFlags::INCLUDE_MIDIDRUM`.
pub const INCLUDE_MIDIDRUM: u32 = 8;

/// C++ `SiONVoicePresetUtil::GeneratorFlags::INCLUDE_WAVETABLE`.
pub const INCLUDE_WAVETABLE: u32 = 16;

/// C++ `SiONVoicePresetUtil::GeneratorFlags::INCLUDE_SINGLE_DRUM`.
pub const INCLUDE_SINGLE_DRUM: u32 = 32;

/// C++ `SiONVoicePresetUtil::GeneratorFlags::INCLUDE_ALL`.
pub const INCLUDE_ALL: u32 = 0xffff;

/// C++ `SiONVoicePresetUtil` — voice-preset generator and registry.
#[derive(Default)]
pub struct SiONVoicePresetUtil {
    pub current_category: Vec<Rc<RefCell<SiONVoice>>>,
    pub category_map: HashMap<String, Vec<Rc<RefCell<SiONVoice>>>>,
    pub voice_map: HashMap<String, Rc<RefCell<SiONVoice>>>,
    pub wave_tables: Vec<Rc<RefCell<SiopmWaveTable>>>,
}

impl SiONVoicePresetUtil {
    /// C++ `_generate_voices(p_flags)` — per-flag category dispatch.
    pub fn _generate_voices(&mut self, p_flags: u32) {
        if p_flags & INCLUDE_DEFAULT != 0 {
            self._generate_default_voices();
        }
        if p_flags & INCLUDE_VALSOUND != 0 {
            self._generate_valsound_voices();
        }
        if p_flags & INCLUDE_MIDI != 0 {
            self._generate_midi_voices();
        }
        if p_flags & INCLUDE_MIDIDRUM != 0 {
            self._generate_mididrum_voices();
        }
        if p_flags & INCLUDE_WAVETABLE != 0 {
            self._generate_wave_table_voices();
        }
        if p_flags & INCLUDE_SINGLE_DRUM != 0 {
            self._generate_single_drum_voices();
        }
    }

    /// C++ `_generate_default_voices` — the 16 default voices.
    pub fn _generate_default_voices(&mut self) {
        self._begin_category("default");
        self._create_basic_voice("sine", "Sine Wave", 0);
        self._create_basic_voice("saw", "Saw Wave", 1);
        self._create_basic_voice("triangle8", "8-bit Triangle Wave", 3);
        self._create_basic_voice("triangle", "Triangle Wave", 4);
        self._create_basic_voice("square", "Square Wave", 5);
        self._create_basic_voice("noise", "White Noise", 6);
        self._create_basic_voice("snoise", "93-bit Noise", 25);
        self._create_basic_voice("konami", "Konami Wave Sample", 7);
        self._create_basic_voice("ma1", "MA-3 Wave Sample", 33);
        self._create_basic_voice("beep", "Pulse Wave Sample", 81);
        self._create_basic_voice("ramp", "Ramp Wave Sample", 160);

        self._create_percussive_voice("bassdrumm", "Bass Drum (1 op)", 0, 63, 28, -128, 128, 0);
        self._create_percussive_voice("snare", "Snare Drum (1 op)", 17, 63, 36, 0, 96, 1);
        self._create_percussive_voice("closedhh", "Closed Hi-Hat (1 op)", 19, 63, 40, 0, 128, 0);
        self._create_percussive_voice("openedhh", "Opened Hi-Hat (1 op)", 19, 63, 28, 0, 128, 0);
        self._create_percussive_voice("crash", "Crash Symbal (1 op)", 16, 48, 24, 0, 128, 0);

        self._create_analog_voice("dualsaw", "Dual Saw", 0, 1, 1, 0, 8);
        self._create_analog_voice("dualsquare", "Dual Square", 0, 5, 5, 0, 8);
    }

    /// C++ `_generate_valsound_voices` — cpp lines 65-349 (wave-9 pass B).
    fn _generate_valsound_voices(&mut self) {
        self._begin_category("valsound.bass");
        self._create_opn_voice("valsound.bass1", "Analog Bass #2 (+FBsynth)", vec![6, 7, 31, 0, 0, 12, 1, 18, 1, 1, 0, 0, 31, 0, 1, 12, 1, 4, 1, 2, 0, 0, 31, 0, 0, 9, 0, 3, 0, 1, 7, 0, 31, 0, 0, 9, 0, 3, 0, 1, 3, 0]);
        self._create_opn_voice("valsound.bass2", "Analog Bass", vec![5, 6, 31, 0, 0, 0, 0, 41, 0, 1, 0, 0, 20, 0, 0, 10, 0, 0, 0, 1, 1, 0, 24, 0, 0, 8, 0, 0, 0, 1, 2, 0, 20, 0, 0, 10, 0, 0, 0, 1, 3, 0]);
        self._create_opn_voice("valsound.bass3", "Analog Bass #2 (q2)", vec![6, 4, 21, 5, 0, 0, 2, 35, 0, 0, 0, 0, 26, 10, 0, 11, 1, 0, 0, 1, 0, 0, 27, 0, 0, 11, 0, 0, 0, 1, 3, 0, 27, 14, 0, 11, 1, 0, 0, 1, 7, 0]);
        self._create_opn_voice("valsound.bass4", "Chopper Bass 0", vec![0, 5, 28, 14, 15, 15, 4, 25, 2, 10, 1, 0, 31, 14, 10, 9, 3, 34, 2, 0, 2, 0, 31, 14, 9, 9, 2, 23, 0, 0, 3, 0, 31, 6, 5, 11, 2, 0, 0, 0, 7, 0]);
        self._create_opn_voice("valsound.bass5", "Chopper Bass 1", vec![0, 5, 28, 14, 15, 15, 4, 30, 1, 14, 1, 0, 31, 14, 10, 9, 2, 35, 1, 3, 2, 0, 31, 14, 9, 9, 2, 25, 0, 0, 3, 0, 31, 6, 5, 11, 1, 0, 0, 0, 7, 0]);
        self._create_opn_voice("valsound.bass6", "Chopper Bass 2 (cut)", vec![0, 4, 31, 15, 28, 5, 2, 28, 3, 15, 6, 0, 31, 10, 15, 4, 4, 41, 3, 4, 6, 0, 31, 8, 3, 5, 1, 21, 2, 0, 6, 0, 31, 2, 2, 5, 15, 0, 2, 0, 6, 0]);
        self._create_opn_voice("valsound.bass7", "Chopper Bass 3", vec![0, 5, 31, 18, 2, 13, 9, 28, 2, 13, 1, 0, 31, 10, 15, 4, 4, 41, 3, 1, 2, 0, 31, 8, 3, 5, 1, 21, 2, 0, 3, 0, 31, 2, 2, 12, 15, 0, 2, 0, 7, 0]);
        self._create_opn_voice("valsound.bass8", "Elec.Chopper Bass +4", vec![0, 5, 31, 18, 2, 13, 9, 28, 2, 13, 1, 0, 31, 10, 15, 4, 4, 41, 3, 1, 2, 0, 31, 8, 3, 5, 1, 21, 2, 0, 3, 0, 31, 2, 2, 12, 15, 0, 2, 1, 7, 0]);
        self._create_opn_voice("valsound.bass9", "Effect Bass 1", vec![4, 3, 23, 5, 4, 7, 2, 0, 0, 1, 3, 0, 30, 2, 2, 8, 2, 0, 1, 7, 3, 0, 24, 5, 4, 7, 2, 0, 0, 1, 7, 0, 31, 2, 2, 8, 2, 0, 1, 10, 7, 0]);
        self._create_opn_voice("valsound.bass10", "Effect Bass 2 (to UP)", vec![4, 3, 3, 6, 5, 15, 2, 0, 0, 1, 3, 0, 7, 4, 3, 15, 2, 0, 1, 7, 3, 0, 3, 6, 5, 15, 2, 0, 0, 1, 7, 0, 7, 4, 3, 15, 2, 0, 1, 10, 7, 0]);
        self._create_opn_voice("valsound.bass11", "Effect Bass 3", vec![4, 3, 22, 5, 6, 0, 0, 9, 0, 1, 3, 0, 19, 3, 4, 7, 1, 0, 0, 7, 3, 0, 23, 0, 0, 0, 0, 19, 0, 1, 7, 0, 20, 2, 0, 7, 1, 0, 0, 1, 7, 0]);
        self._create_opn_voice("valsound.bass12", "Mohaaa", vec![0, 5, 7, 0, 0, 15, 0, 21, 0, 1, 0, 0, 6, 0, 0, 15, 0, 18, 0, 2, 0, 0, 8, 0, 0, 15, 0, 23, 0, 1, 0, 0, 18, 0, 0, 15, 0, 0, 0, 2, 0, 0]);
        self._create_opn_voice("valsound.bass13", "Effect FB Bass #5", vec![0, 7, 31, 6, 2, 15, 3, 20, 0, 1, 3, 0, 31, 6, 2, 15, 6, 14, 2, 2, 0, 0, 6, 6, 2, 15, 1, 8, 0, 1, 7, 0, 31, 5, 1, 15, 2, 0, 0, 2, 0, 0]);
        self._create_opn_voice("valsound.bass14", "Magical Bass", vec![0, 7, 31, 8, 0, 6, 10, 38, 1, 1, 3, 0, 28, 18, 5, 6, 13, 47, 1, 10, 7, 0, 31, 7, 7, 6, 8, 23, 2, 0, 2, 0, 28, 9, 6, 8, 1, 0, 2, 0, 0, 0]);
        self._create_opn_voice("valsound.bass15", "E.Bass #6", vec![0, 7, 31, 15, 0, 10, 5, 35, 1, 14, 3, 0, 31, 14, 7, 7, 4, 41, 1, 4, 7, 0, 31, 14, 3, 0, 2, 18, 1, 0, 3, 0, 31, 12, 8, 8, 1, 0, 0, 0, 7, 0]);
        self._create_opn_voice("valsound.bass16", "E.Bass #7", vec![3, 7, 31, 15, 0, 10, 5, 29, 1, 10, 7, 0, 31, 13, 7, 7, 4, 46, 1, 4, 7, 0, 31, 14, 5, 0, 2, 19, 1, 0, 3, 0, 31, 12, 4, 8, 1, 0, 0, 0, 0, 0]);
        self._create_opn_voice("valsound.bass17", "E.Bass 70", vec![2, 5, 31, 8, 0, 0, 3, 34, 0, 0, 3, 0, 31, 14, 6, 9, 2, 42, 0, 8, 0, 0, 31, 16, 3, 0, 2, 20, 0, 0, 7, 0, 31, 12, 5, 8, 2, 0, 0, 1, 0, 0]);
        self._create_opn_voice("valsound.bass18", "VAL006 Bass (like Euro)", vec![0, 4, 31, 7, 7, 11, 2, 25, 3, 6, 0, 0, 31, 6, 6, 11, 1, 55, 3, 4, 7, 0, 31, 9, 6, 11, 1, 18, 2, 0, 3, 0, 31, 6, 8, 11, 15, 0, 2, 1, 0, 0]);
        self._create_opn_voice("valsound.bass19", "E.Bass x2", vec![2, 7, 31, 14, 8, 3, 1, 33, 0, 0, 1, 0, 31, 17, 8, 9, 5, 30, 0, 14, 2, 0, 31, 15, 8, 5, 5, 35, 0, 4, 3, 0, 31, 15, 8, 6, 1, 0, 0, 1, 7, 0]);
        self._create_opn_voice("valsound.bass20", "E.Bass x4", vec![2, 7, 31, 14, 3, 0, 2, 33, 1, 3, 3, 0, 23, 16, 4, 12, 3, 30, 2, 10, 0, 0, 31, 13, 3, 11, 3, 27, 1, 0, 7, 0, 31, 7, 5, 9, 1, 0, 3, 1, 0, 0]);
        self._create_opn_voice("valsound.bass21", "Metal Pick Bass x5", vec![3, 7, 31, 14, 0, 6, 13, 51, 2, 13, 0, 0, 31, 13, 0, 6, 13, 21, 0, 1, 0, 0, 31, 9, 0, 6, 13, 23, 0, 0, 0, 0, 31, 9, 0, 7, 13, 0, 0, 0, 0, 0]);
        self._create_opn_voice("valsound.bass22", "Groove Bass 1", vec![5, 3, 31, 0, 0, 0, 0, 38, 0, 0, 0, 0, 21, 0, 0, 13, 0, 5, 0, 0, 0, 0, 21, 0, 0, 13, 0, 3, 0, 1, 0, 0, 21, 0, 0, 13, 0, 3, 0, 1, 0, 0]);
        self._create_opn_voice("valsound.bass23", "Analog Bass Groove #2", vec![6, 5, 31, 0, 0, 0, 0, 41, 0, 2, 0, 0, 31, 0, 0, 10, 0, 2, 0, 1, 0, 0, 31, 0, 0, 10, 0, 1, 0, 1, 7, 0, 31, 0, 0, 10, 0, 1, 0, 1, 3, 0]);
        self._create_opn_voice("valsound.bass24", "Harmonics #1", vec![6, 6, 31, 12, 9, 0, 2, 45, 1, 2, 3, 0, 31, 10, 8, 6, 1, 0, 1, 1, 7, 0, 31, 13, 8, 5, 2, 0, 1, 2, 7, 0, 31, 16, 12, 5, 10, 0, 1, 0, 3, 0]);
        self._create_opn_voice("valsound.bass25", "Low Bass x1", vec![5, 3, 31, 0, 9, 15, 0, 25, 2, 0, 0, 0, 31, 15, 6, 8, 1, 0, 1, 0, 1, 0, 31, 15, 6, 8, 1, 0, 1, 1, 2, 0, 31, 15, 0, 7, 2, 0, 1, 1, 3, 0]);
        self._create_opn_voice("valsound.bass26", "Low Bass x2 (little FB)", vec![5, 6, 21, 0, 9, 0, 0, 24, 2, 0, 0, 0, 21, 15, 6, 8, 1, 0, 1, 0, 1, 0, 21, 15, 6, 8, 1, 0, 1, 1, 2, 0, 27, 15, 0, 7, 2, 0, 1, 1, 3, 0]);
        self._create_opn_voice("valsound.bass27", "Low Bass x1 (rezzo.)", vec![5, 3, 31, 0, 9, 15, 0, 30, 2, 0, 0, 0, 31, 15, 6, 12, 1, 2, 1, 0, 1, 0, 31, 15, 6, 12, 1, 2, 1, 1, 2, 0, 31, 15, 10, 12, 2, 2, 1, 4, 3, 0]);
        self._create_opn_voice("valsound.bass28", "Low Bass Picked", vec![5, 7, 31, 5, 0, 0, 11, 33, 1, 0, 0, 0, 30, 12, 4, 9, 1, 0, 1, 0, 0, 0, 27, 14, 8, 9, 3, 0, 1, 1, 0, 0, 27, 14, 7, 12, 15, 6, 1, 5, 0, 0]);
        self._create_opn_voice("valsound.bass29", "Metal Bass", vec![0, 5, 20, 10, 9, 15, 1, 22, 0, 0, 7, 0, 17, 9, 0, 0, 2, 22, 0, 1, 7, 0, 21, 9, 0, 0, 1, 18, 0, 0, 3, 0, 18, 8, 0, 8, 1, 0, 0, 1, 3, 0]);
        self._create_opn_voice("valsound.bass30", "E.N. Bass 1", vec![3, 7, 27, 14, 0, 4, 4, 25, 0, 7, 0, 0, 31, 12, 0, 4, 3, 45, 0, 2, 0, 0, 31, 19, 0, 4, 5, 15, 0, 0, 0, 0, 31, 12, 6, 7, 1, 0, 0, 0, 0, 0]);
        self._create_opn_voice("valsound.bass31", "PSG Bass 1", vec![5, 7, 31, 14, 0, 0, 0, 22, 0, 0, 0, 0, 31, 14, 3, 8, 5, 0, 0, 1, 3, 0, 31, 14, 3, 8, 3, 0, 0, 0, 0, 0, 31, 16, 3, 8, 3, 0, 0, 1, 7, 0]);
        self._create_opn_voice("valsound.bass32", "PSG Bass 2", vec![5, 7, 31, 14, 0, 0, 0, 22, 0, 1, 0, 0, 31, 14, 3, 8, 5, 0, 0, 2, 3, 0, 31, 14, 3, 8, 3, 0, 0, 0, 0, 0, 31, 16, 3, 8, 3, 0, 0, 1, 7, 0]);
        self._create_opn_voice("valsound.bass33", "Rezonance-type Bass #1", vec![2, 0, 24, 19, 2, 13, 10, 33, 1, 3, 3, 0, 26, 16, 5, 14, 6, 28, 1, 0, 0, 0, 15, 14, 6, 8, 5, 14, 1, 0, 0, 0, 31, 7, 5, 9, 2, 0, 0, 2, 7, 0]);
        self._create_opn_voice("valsound.bass34", "Slap Bass", vec![2, 2, 31, 10, 7, 8, 2, 33, 0, 0, 7, 0, 21, 8, 8, 7, 5, 23, 3, 7, 7, 0, 31, 5, 6, 7, 1, 37, 0, 0, 3, 0, 31, 8, 6, 7, 5, 0, 0, 1, 7, 0]);
        self._create_opn_voice("valsound.bass35", "Slap Bass 1", vec![2, 7, 31, 14, 7, 8, 2, 33, 0, 0, 7, 0, 21, 15, 6, 7, 4, 18, 2, 6, 7, 0, 31, 5, 6, 7, 1, 40, 0, 0, 3, 0, 31, 12, 7, 7, 5, 0, 0, 1, 3, 0]);
        self._create_opn_voice("valsound.bass36", "Slap Bass 2 (1+)", vec![2, 7, 31, 14, 7, 8, 2, 33, 0, 0, 7, 0, 21, 15, 6, 7, 4, 28, 2, 7, 7, 0, 31, 5, 6, 7, 1, 40, 0, 0, 3, 0, 31, 12, 7, 7, 5, 0, 0, 1, 3, 0]);
        self._create_opn_voice("valsound.bass37", "Slap Bass #3", vec![2, 7, 31, 14, 7, 0, 5, 32, 0, 3, 7, 0, 31, 16, 1, 12, 4, 35, 0, 10, 0, 0, 31, 11, 2, 0, 3, 23, 1, 0, 3, 0, 31, 12, 5, 7, 1, 0, 0, 0, 0, 0]);
        self._create_opn_voice("valsound.bass38", "Slap Bass (pull)", vec![2, 2, 31, 10, 7, 8, 2, 33, 0, 0, 7, 0, 21, 8, 8, 9, 5, 23, 3, 10, 7, 0, 31, 5, 6, 10, 1, 37, 0, 0, 3, 0, 31, 16, 6, 11, 1, 0, 0, 1, 7, 0]);
        self._create_opn_voice("valsound.bass39", "Slap Bass (mute)", vec![2, 2, 31, 18, 7, 11, 12, 33, 0, 0, 7, 0, 21, 11, 8, 11, 15, 23, 3, 7, 7, 0, 31, 15, 6, 11, 11, 37, 0, 0, 3, 0, 31, 15, 6, 13, 11, 0, 0, 1, 7, 0]);
        self._create_opn_voice("valsound.bass40", "Slap Bass (pick)", vec![2, 2, 31, 10, 7, 8, 2, 33, 0, 0, 7, 0, 21, 9, 8, 7, 5, 23, 3, 7, 7, 0, 31, 5, 6, 8, 1, 37, 0, 0, 3, 0, 31, 11, 6, 10, 5, 0, 0, 1, 7, 0]);
        self._create_opn_voice("valsound.bass41", "Super Bass #2", vec![2, 2, 24, 18, 2, 13, 9, 12, 2, 3, 3, 0, 26, 16, 5, 14, 9, 24, 1, 1, 0, 0, 31, 12, 2, 8, 3, 22, 1, 0, 7, 0, 31, 7, 5, 9, 2, 0, 0, 1, 0, 0]);
        self._create_opn_voice("valsound.bass42", "SP Bass #3 (soft)", vec![2, 3, 24, 18, 2, 13, 9, 25, 2, 3, 3, 0, 26, 16, 5, 14, 9, 24, 1, 1, 0, 0, 31, 12, 2, 8, 3, 32, 1, 0, 7, 0, 31, 7, 5, 9, 2, 0, 0, 1, 0, 0]);
        self._create_opn_voice("valsound.bass43", "SP Bass #4 (soft*2)", vec![2, 1, 24, 18, 2, 13, 10, 28, 2, 3, 3, 0, 26, 16, 5, 14, 6, 24, 1, 0, 0, 0, 31, 12, 2, 8, 3, 30, 1, 0, 7, 0, 31, 7, 5, 9, 2, 0, 0, 1, 0, 0]);
        self._create_opn_voice("valsound.bass44", "SP Bass #5 (attack)", vec![0, 5, 19, 18, 2, 15, 10, 30, 2, 0, 3, 0, 31, 16, 5, 14, 5, 24, 1, 0, 0, 0, 31, 12, 2, 8, 3, 30, 1, 0, 7, 0, 31, 10, 7, 9, 2, 0, 0, 1, 0, 0]);
        self._create_opn_voice("valsound.bass45", "SP Bass #6 (rezz+pipebass)", vec![2, 3, 24, 18, 2, 13, 9, 35, 2, 12, 3, 0, 26, 16, 5, 14, 9, 25, 1, 2, 0, 0, 31, 12, 2, 8, 3, 32, 1, 0, 7, 0, 31, 7, 5, 9, 2, 0, 0, 1, 0, 0]);
        self._create_opn_voice("valsound.bass46", "Synth Bass 1", vec![4, 3, 30, 0, 0, 0, 0, 23, 0, 1, 3, 0, 27, 4, 0, 7, 1, 0, 0, 1, 3, 0, 30, 0, 0, 0, 0, 18, 0, 1, 7, 0, 25, 4, 0, 7, 1, 0, 0, 1, 7, 0]);
        self._create_opn_voice("valsound.bass47", "Synth Bass 2 (myon)", vec![5, 7, 14, 0, 9, 12, 0, 26, 2, 0, 0, 0, 14, 15, 6, 8, 1, 0, 1, 0, 1, 0, 20, 15, 6, 8, 1, 0, 1, 1, 2, 0, 18, 15, 0, 12, 2, 0, 1, 1, 3, 0]);
        self._create_opn_voice("valsound.bass48", "Synth Bass #3 (cho!)", vec![3, 7, 31, 11, 9, 0, 4, 32, 1, 1, 3, 0, 31, 15, 7, 8, 5, 41, 1, 8, 7, 0, 26, 18, 7, 10, 6, 4, 1, 0, 3, 0, 31, 9, 6, 7, 1, 0, 0, 0, 7, 0]);
        self._create_opn_voice("valsound.bass49", "Synth Wind Bass #4", vec![2, 7, 31, 13, 9, 0, 4, 32, 1, 0, 3, 0, 31, 15, 7, 8, 4, 21, 1, 1, 0, 0, 26, 18, 7, 8, 3, 21, 1, 1, 7, 0, 31, 9, 6, 7, 1, 0, 0, 0, 0, 0]);
        self._create_opn_voice("valsound.bass50", "Synth Bass #5 (q2)", vec![4, 4, 20, 0, 0, 8, 7, 17, 0, 0, 3, 0, 18, 7, 4, 11, 0, 0, 0, 1, 3, 0, 18, 0, 0, 9, 0, 22, 0, 1, 7, 0, 15, 0, 0, 11, 1, 0, 0, 1, 7, 0]);
        self._create_opn_voice("valsound.bass51", "Old Wood Bass", vec![5, 7, 31, 15, 0, 13, 2, 28, 1, 0, 0, 0, 31, 10, 1, 12, 1, 4, 0, 2, 0, 0, 25, 10, 1, 12, 1, 4, 0, 1, 0, 0, 31, 10, 1, 12, 1, 4, 0, 2, 0, 0]);
        self._create_opn_voice("valsound.bass52", "Wood Bass (bright)", vec![2, 7, 31, 13, 3, 10, 2, 32, 1, 0, 7, 0, 31, 12, 4, 10, 3, 20, 1, 0, 0, 0, 31, 17, 0, 10, 7, 15, 1, 2, 3, 0, 31, 6, 1, 10, 5, 0, 1, 1, 0, 0]);
        self._create_opn_voice("valsound.bass53", "Wood Bass x2 (bow)", vec![4, 5, 31, 11, 5, 0, 3, 25, 1, 0, 3, 0, 31, 10, 9, 8, 4, 0, 1, 1, 3, 0, 23, 12, 5, 0, 4, 14, 1, 0, 7, 0, 31, 12, 9, 7, 5, 0, 1, 2, 7, 0]);
        self._create_opn_voice("valsound.bass54", "Wood Bass 3 (muted1)", vec![5, 5, 31, 15, 0, 15, 2, 38, 1, 0, 0, 0, 31, 10, 1, 12, 1, 4, 0, 2, 0, 0, 25, 10, 1, 12, 1, 2, 0, 1, 0, 0, 31, 10, 1, 12, 1, 4, 0, 1, 0, 0]);

        self._begin_category("valsound.bell");
        self._create_opn_voice("valsound.bell1", "Calm Bell", vec![4, 3, 31, 12, 0, 10, 5, 38, 0, 6, 3, 0, 31, 8, 4, 6, 11, 4, 0, 2, 3, 0, 31, 12, 4, 6, 2, 40, 1, 6, 7, 0, 31, 6, 4, 6, 11, 0, 0, 2, 7, 0]);
        self._create_opn_voice("valsound.bell2", "China Bell Double", vec![4, 7, 21, 15, 8, 0, 3, 27, 1, 8, 3, 0, 31, 13, 5, 6, 4, 0, 1, 4, 3, 0, 21, 15, 8, 0, 3, 25, 1, 6, 7, 0, 31, 13, 5, 6, 4, 0, 1, 3, 7, 0]);
        self._create_opn_voice("valsound.bell3", "Church Bell 2", vec![4, 0, 26, 3, 0, 2, 15, 35, 2, 4, 3, 0, 31, 6, 0, 3, 15, 7, 1, 11, 0, 0, 31, 6, 0, 1, 14, 41, 2, 6, 7, 0, 31, 7, 0, 3, 15, 0, 0, 11, 7, 0]);
        self._create_opn_voice("valsound.bell4", "Church Bell", vec![4, 0, 26, 3, 0, 2, 15, 35, 2, 4, 3, 0, 31, 6, 0, 3, 15, 7, 1, 11, 0, 0, 31, 6, 0, 1, 14, 41, 2, 4, 7, 0, 31, 7, 0, 3, 15, 0, 0, 15, 7, 0]);
        self._create_opn_voice("valsound.bell5", "Glocken 1", vec![4, 3, 31, 24, 0, 12, 15, 32, 0, 14, 2, 0, 31, 15, 0, 8, 15, 0, 0, 2, 0, 0, 31, 20, 0, 4, 15, 27, 0, 15, 0, 0, 31, 14, 0, 5, 15, 0, 0, 2, 0, 0]);
        self._create_opn_voice("valsound.bell6", "Harp #1", vec![1, 7, 31, 10, 10, 6, 5, 26, 0, 3, 0, 0, 31, 10, 10, 7, 5, 50, 1, 2, 0, 0, 31, 13, 10, 7, 13, 40, 0, 2, 0, 0, 31, 14, 5, 7, 10, 0, 0, 1, 0, 0]);
        self._create_opn_voice("valsound.bell7", "Harp #2", vec![1, 3, 31, 9, 0, 0, 15, 40, 2, 6, 3, 0, 31, 11, 0, 8, 15, 30, 2, 1, 7, 0, 31, 8, 0, 0, 15, 40, 2, 1, 0, 0, 31, 8, 0, 8, 14, 0, 2, 1, 0, 0]);
        self._create_opn_voice("valsound.bell8", "Kirakira", vec![1, 7, 21, 11, 6, 0, 12, 31, 2, 6, 2, 0, 21, 12, 8, 0, 12, 26, 2, 10, 6, 0, 28, 11, 7, 0, 12, 32, 1, 2, 0, 0, 28, 4, 2, 4, 5, 0, 1, 2, 0, 0]);
        self._create_opn_voice("valsound.bell9", "Marimba", vec![4, 6, 22, 16, 7, 3, 15, 36, 2, 15, 3, 0, 16, 10, 13, 7, 10, 0, 2, 1, 3, 0, 19, 18, 7, 3, 8, 26, 1, 6, 7, 0, 16, 11, 12, 7, 10, 3, 2, 2, 7, 0]);
        self._create_opn_voice("valsound.bell10", "Old Bell", vec![4, 6, 27, 4, 0, 5, 14, 34, 0, 3, 3, 0, 31, 7, 0, 6, 14, 0, 1, 1, 0, 0, 31, 7, 0, 3, 13, 41, 0, 14, 7, 0, 31, 8, 0, 6, 14, 16, 0, 4, 7, 0]);
        self._create_opn_voice("valsound.bell11", "Percus. Bell", vec![5, 3, 31, 12, 0, 9, 5, 38, 0, 12, 0, 0, 31, 15, 4, 5, 11, 9, 0, 3, 0, 0, 31, 12, 4, 8, 12, 9, 0, 2, 3, 0, 31, 6, 4, 8, 11, 9, 0, 1, 7, 0]);
        self._create_opn_voice("valsound.bell12", "Pretty Bell", vec![6, 6, 31, 12, 9, 0, 3, 43, 1, 2, 0, 0, 31, 9, 8, 6, 3, 0, 1, 1, 3, 0, 31, 13, 8, 5, 3, 0, 1, 4, 7, 0, 31, 16, 16, 5, 13, 13, 1, 15, 0, 0]);
        self._create_opn_voice("valsound.bell13", "Synth Bell #0 (from OPM)", vec![6, 2, 31, 5, 5, 5, 2, 30, 0, 7, 7, 0, 31, 8, 5, 7, 15, 0, 0, 3, 7, 0, 31, 6, 7, 7, 5, 0, 0, 0, 3, 0, 31, 8, 5, 5, 2, 10, 0, 1, 3, 0]);
        self._create_opn_voice("valsound.bell14", "Synth Bell #1 (o5)", vec![6, 3, 31, 5, 5, 5, 2, 33, 1, 8, 3, 0, 27, 11, 0, 6, 15, 0, 1, 2, 3, 0, 31, 6, 7, 6, 5, 0, 2, 0, 7, 0, 31, 11, 8, 6, 3, 0, 1, 1, 7, 0]);
        self._create_opn_voice("valsound.bell15", "Synth Bell 2", vec![6, 5, 31, 8, 9, 0, 5, 33, 1, 7, 3, 0, 31, 9, 7, 8, 2, 0, 1, 3, 7, 0, 31, 12, 7, 8, 1, 0, 1, 2, 3, 0, 31, 9, 7, 7, 1, 0, 0, 1, 7, 0]);
        self._create_opn_voice("valsound.bell16", "Vibraphone (AMS-modu.)", vec![4, 5, 24, 14, 0, 7, 15, 50, 1, 12, 3, 0, 24, 10, 0, 7, 15, 0, 1, 4, 7, 0, 26, 14, 0, 6, 15, 57, 1, 4, 7, 0, 26, 8, 0, 6, 15, 0, 2, 4, 3, 0]);
        self._create_opn_voice("valsound.bell17", "Twin Marinba 2 (g&c)", vec![4, 5, 18, 9, 5, 14, 12, 33, 1, 14, 3, 0, 31, 16, 6, 9, 7, 0, 1, 4, 3, 0, 18, 9, 5, 14, 12, 33, 1, 7, 7, 0, 31, 16, 6, 9, 7, 0, 1, 3, 7, 0]);
        self._create_opn_voice("valsound.bell18", "Twin Marinba 1 (g&c)", vec![4, 2, 31, 10, 5, 0, 12, 30, 1, 8, 3, 0, 31, 16, 6, 9, 9, 0, 1, 4, 3, 0, 31, 10, 5, 0, 12, 30, 1, 6, 7, 0, 31, 16, 6, 9, 9, 0, 1, 3, 7, 0]);

        self._begin_category("valsound.brass");
        self._create_opn_voice("valsound.brass1", "Brass std::strings", vec![5, 7, 20, 0, 0, 0, 0, 27, 0, 1, 0, 0, 15, 3, 0, 6, 1, 5, 0, 2, 1, 0, 14, 4, 0, 6, 1, 5, 0, 1, 2, 0, 15, 4, 0, 6, 1, 5, 0, 1, 3, 0]);
        self._create_opn_voice("valsound.brass2", "E.Trumpet (mute)", vec![2, 7, 13, 6, 0, 8, 1, 26, 2, 2, 3, 0, 15, 8, 0, 8, 1, 32, 1, 2, 7, 0, 21, 15, 0, 8, 11, 20, 0, 2, 3, 0, 18, 4, 0, 8, 2, 0, 1, 8, 0, 0]);
        self._create_opn_voice("valsound.brass3", "Horn 2", vec![4, 7, 15, 11, 2, 0, 2, 23, 0, 2, 3, 0, 13, 12, 2, 15, 2, 0, 0, 2, 3, 0, 15, 13, 5, 0, 1, 27, 0, 2, 7, 0, 13, 11, 2, 15, 2, 0, 0, 2, 7, 0]);
        self._create_opn_voice("valsound.brass4", "Alpine Horn #3", vec![5, 7, 15, 10, 0, 6, 5, 35, 0, 1, 0, 0, 15, 5, 0, 8, 2, 6, 0, 2, 2, 0, 15, 5, 0, 8, 2, 6, 0, 1, 5, 0, 15, 5, 0, 8, 2, 6, 0, 1, 0, 0]);
        self._create_opn_voice("valsound.brass5", "Lead Brass", vec![2, 7, 18, 4, 2, 8, 1, 27, 0, 2, 3, 0, 14, 14, 0, 8, 5, 33, 1, 8, 0, 0, 20, 0, 2, 8, 0, 36, 0, 2, 7, 0, 17, 4, 1, 8, 3, 0, 0, 2, 0, 0]);
        self._create_opn_voice("valsound.brass6", "Normal Horn", vec![4, 7, 15, 13, 2, 15, 1, 36, 0, 2, 3, 0, 13, 12, 2, 15, 3, 0, 0, 2, 3, 0, 15, 13, 5, 15, 2, 25, 0, 2, 7, 0, 13, 11, 2, 15, 3, 0, 0, 2, 7, 0]);
        self._create_opn_voice("valsound.brass7", "Synth Oboe", vec![6, 3, 17, 15, 15, 3, 15, 15, 0, 1, 7, 0, 16, 0, 9, 0, 0, 0, 0, 6, 3, 0, 21, 15, 11, 1, 4, 4, 0, 4, 3, 0, 18, 15, 11, 1, 4, 4, 0, 6, 7, 0]);
        self._create_opn_voice("valsound.brass8", "Oboe 2", vec![2, 5, 19, 18, 0, 9, 2, 23, 1, 1, 0, 0, 31, 17, 0, 6, 3, 28, 0, 6, 0, 0, 31, 20, 0, 5, 1, 51, 0, 8, 0, 0, 16, 31, 0, 11, 0, 0, 1, 4, 0, 0]);
        self._create_opn_voice("valsound.brass9", "Attack Brass (q2)", vec![4, 4, 15, 9, 8, 8, 2, 14, 1, 4, 7, 0, 18, 15, 1, 8, 3, 0, 0, 4, 3, 0, 16, 9, 8, 8, 2, 12, 1, 2, 3, 0, 31, 15, 1, 8, 3, 0, 0, 2, 7, 0]);
        self._create_opn_voice("valsound.brass10", "Sax", vec![2, 6, 13, 6, 0, 8, 1, 14, 2, 2, 3, 0, 15, 8, 0, 8, 1, 30, 1, 10, 7, 0, 21, 7, 0, 8, 2, 35, 0, 1, 3, 0, 18, 4, 0, 9, 2, 0, 1, 2, 0, 0]);
        self._create_opn_voice("valsound.brass11", "Soft Brass (lead)", vec![4, 7, 16, 3, 0, 2, 1, 30, 0, 1, 5, 0, 18, 0, 0, 7, 0, 3, 1, 4, 0, 0, 16, 0, 0, 2, 2, 35, 0, 1, 1, 0, 18, 5, 0, 7, 1, 0, 0, 1, 0, 0]);
        self._create_opn_voice("valsound.brass12", "Synth Brass 1 (old)", vec![5, 7, 31, 7, 5, 10, 2, 28, 0, 1, 0, 0, 31, 2, 5, 10, 2, 0, 0, 0, 0, 0, 31, 2, 5, 10, 2, 2, 0, 1, 0, 0, 31, 10, 5, 10, 10, 0, 0, 2, 0, 0]);
        self._create_opn_voice("valsound.brass13", "Synth Brass 2 (old)", vec![5, 7, 31, 10, 1, 10, 2, 28, 1, 2, 0, 0, 31, 12, 1, 10, 2, 2, 0, 1, 0, 0, 31, 12, 1, 10, 2, 4, 0, 2, 0, 0, 31, 12, 1, 10, 10, 3, 0, 4, 0, 0]);
        self._create_opn_voice("valsound.brass14", "Synth Brass 3", vec![4, 7, 15, 9, 0, 9, 2, 22, 0, 2, 7, 0, 23, 4, 2, 9, 5, 0, 0, 2, 7, 0, 14, 10, 0, 9, 2, 20, 0, 2, 3, 0, 20, 4, 0, 9, 2, 0, 0, 2, 3, 0]);
        self._create_opn_voice("valsound.brass15", "Synth Brass #4", vec![5, 7, 20, 0, 0, 0, 0, 22, 0, 4, 0, 0, 18, 12, 0, 8, 1, 0, 0, 8, 0, 0, 20, 12, 0, 8, 1, 0, 0, 4, 6, 0, 22, 12, 0, 8, 1, 0, 0, 4, 2, 0]);
        self._create_opn_voice("valsound.brass16", "Synth Brass 5 (long)", vec![4, 7, 29, 2, 2, 0, 3, 28, 0, 2, 7, 0, 29, 0, 2, 8, 5, 4, 0, 4, 7, 0, 21, 2, 2, 0, 2, 32, 0, 1, 3, 0, 29, 0, 2, 8, 5, 4, 0, 2, 3, 0]);
        self._create_opn_voice("valsound.brass17", "Synth Brass 6", vec![2, 7, 30, 8, 8, 5, 3, 25, 0, 1, 1, 0, 25, 10, 8, 6, 4, 30, 0, 2, 1, 0, 20, 10, 5, 6, 3, 40, 0, 1, 5, 0, 20, 5, 5, 7, 5, 0, 0, 1, 3, 0]);
        self._create_opn_voice("valsound.brass18", "Trumpet", vec![2, 7, 13, 6, 0, 8, 1, 25, 2, 2, 3, 0, 15, 8, 0, 8, 1, 32, 1, 6, 7, 0, 21, 7, 0, 8, 2, 42, 0, 2, 3, 0, 18, 4, 0, 8, 2, 0, 1, 2, 0, 0]);
        self._create_opn_voice("valsound.brass19", "Trumpet 2", vec![2, 6, 13, 6, 0, 8, 1, 14, 2, 2, 3, 0, 15, 8, 0, 8, 1, 30, 1, 12, 7, 0, 21, 7, 0, 8, 2, 38, 0, 2, 3, 0, 18, 4, 0, 8, 2, 0, 2, 2, 0, 0]);
        self._create_opn_voice("valsound.brass20", "Twin Horn (or OL=25)", vec![4, 6, 14, 6, 0, 11, 3, 32, 0, 4, 3, 0, 16, 8, 0, 9, 2, 0, 0, 4, 3, 0, 14, 6, 0, 11, 3, 33, 0, 3, 7, 0, 16, 8, 0, 9, 2, 0, 0, 3, 7, 0]);

        self._begin_category("valsound.guitar");
        self._create_opn_voice("valsound.guitar1", "Guitar VeloLow", vec![1, 3, 31, 11, 6, 0, 2, 45, 1, 7, 0, 0, 31, 7, 5, 0, 5, 35, 1, 2, 0, 0, 31, 7, 6, 0, 5, 40, 1, 1, 0, 0, 31, 13, 5, 5, 1, 0, 1, 1, 0, 0]);
        self._create_opn_voice("valsound.guitar2", "Guitar VeloHigh", vec![1, 4, 31, 11, 6, 0, 2, 43, 1, 9, 0, 0, 31, 7, 5, 0, 5, 35, 1, 2, 0, 0, 31, 7, 6, 0, 5, 35, 1, 1, 0, 0, 31, 13, 6, 5, 1, 0, 1, 1, 0, 0]);
        self._create_opn_voice("valsound.guitar3", "A.Guitar #3", vec![1, 7, 31, 10, 8, 4, 2, 34, 2, 13, 0, 0, 31, 9, 7, 4, 2, 36, 0, 2, 0, 0, 31, 9, 8, 4, 2, 38, 0, 1, 0, 0, 31, 4, 2, 8, 2, 0, 1, 1, 0, 0]);
        self._create_opn_voice("valsound.guitar4", "Cutting E.Guitar", vec![3, 5, 21, 7, 1, 0, 1, 18, 0, 4, 0, 0, 24, 0, 4, 9, 1, 15, 0, 6, 0, 0, 22, 20, 2, 7, 13, 5, 0, 2, 3, 0, 31, 12, 0, 6, 1, 0, 0, 2, 7, 0]);
        self._create_opn_voice("valsound.guitar5", "Dis. Synth (old)", vec![5, 7, 31, 0, 0, 12, 1, 18, 1, 1, 0, 0, 31, 0, 1, 12, 1, 4, 1, 2, 0, 0, 31, 0, 1, 12, 1, 4, 1, 0, 0, 0, 31, 0, 1, 12, 1, 4, 1, 2, 0, 0]);
        self._create_opn_voice("valsound.guitar6", "Dis. Guitar (dra-spi)", vec![0, 7, 16, 15, 1, 3, 3, 26, 0, 3, 3, 0, 19, 31, 1, 3, 0, 27, 0, 1, 0, 0, 26, 31, 1, 3, 0, 26, 0, 1, 5, 0, 27, 31, 1, 8, 0, 0, 0, 1, 0, 0]);
        self._create_opn_voice("valsound.guitar7", "Dis. Guitar (3-)", vec![1, 7, 31, 15, 1, 3, 3, 30, 1, 3, 3, 0, 31, 0, 1, 10, 1, 25, 1, 1, 0, 0, 31, 0, 1, 10, 1, 22, 1, 1, 5, 0, 31, 13, 1, 7, 1, 0, 1, 1, 0, 0]);
        self._create_opn_voice("valsound.guitar8", "Dis. Guitar (3+)", vec![0, 5, 31, 4, 0, 0, 1, 8, 0, 3, 0, 0, 18, 1, 0, 8, 0, 25, 0, 15, 0, 0, 31, 4, 0, 0, 1, 23, 0, 7, 7, 0, 31, 12, 0, 9, 0, 0, 0, 1, 1, 0]);
        self._create_opn_voice("valsound.guitar9", "Feedback Guitar 1", vec![3, 7, 31, 13, 0, 2, 2, 26, 0, 6, 3, 0, 18, 7, 4, 10, 5, 24, 0, 3, 3, 0, 31, 0, 0, 8, 0, 22, 0, 4, 0, 0, 31, 0, 0, 7, 1, 0, 0, 2, 7, 0]);
        self._create_opn_voice("valsound.guitar10", "Hard Dis. Guitar 1", vec![0, 5, 31, 4, 4, 6, 1, 8, 0, 3, 0, 0, 18, 1, 4, 0, 1, 27, 0, 12, 0, 0, 31, 4, 4, 0, 1, 22, 0, 2, 3, 0, 31, 12, 0, 8, 1, 0, 0, 2, 7, 0]);
        self._create_opn_voice("valsound.guitar11", "Hard Dis. Guitar 3", vec![0, 5, 31, 4, 1, 0, 0, 11, 0, 3, 0, 0, 18, 1, 4, 7, 0, 23, 0, 15, 0, 0, 31, 4, 2, 0, 0, 24, 0, 5, 1, 0, 31, 12, 0, 7, 1, 0, 0, 1, 7, 0]);
        self._create_opn_voice("valsound.guitar12", "Dis. Guitar ('94 Hard)", vec![0, 7, 31, 0, 0, 11, 0, 21, 0, 9, 7, 0, 31, 15, 0, 10, 1, 26, 0, 2, 3, 0, 31, 5, 0, 8, 1, 25, 0, 1, 3, 0, 31, 0, 0, 7, 0, 4, 0, 2, 7, 0]);
        self._create_opn_voice("valsound.guitar13", "New Dis. Guitar 1", vec![0, 5, 31, 5, 0, 0, 0, 20, 0, 2, 3, 0, 18, 5, 4, 7, 1, 20, 1, 5, 3, 0, 31, 6, 5, 0, 0, 22, 0, 1, 7, 0, 31, 12, 0, 8, 1, 0, 0, 1, 7, 0]);
        self._create_opn_voice("valsound.guitar14", "New Dis. Guitar 2", vec![0, 5, 31, 5, 0, 0, 0, 20, 0, 3, 3, 0, 18, 5, 4, 7, 1, 20, 1, 7, 3, 0, 31, 6, 5, 0, 0, 22, 0, 1, 7, 0, 31, 12, 0, 8, 1, 0, 0, 1, 7, 0]);
        self._create_opn_voice("valsound.guitar15", "New Dis. Guitar 3", vec![3, 5, 31, 5, 0, 0, 10, 8, 0, 3, 0, 0, 31, 1, 0, 8, 0, 20, 0, 15, 0, 0, 31, 4, 0, 15, 0, 22, 0, 1, 3, 0, 31, 12, 0, 8, 1, 0, 0, 1, 7, 0]);
        self._create_opn_voice("valsound.guitar16", "Overdriven Guitar (AL=013)", vec![1, 7, 31, 5, 0, 0, 1, 30, 0, 3, 0, 0, 18, 5, 0, 8, 1, 21, 0, 2, 2, 0, 31, 5, 4, 0, 1, 29, 0, 1, 6, 0, 31, 11, 0, 8, 1, 0, 0, 1, 2, 0]);
        self._create_opn_voice("valsound.guitar17", "Metal std::strings", vec![3, 7, 26, 16, 7, 4, 8, 24, 2, 8, 7, 0, 22, 15, 6, 4, 9, 22, 2, 12, 2, 0, 26, 9, 2, 7, 8, 43, 1, 3, 0, 0, 30, 8, 2, 8, 8, 0, 2, 4, 0, 0]);
        self._create_opn_voice("valsound.guitar18", "Soft Dis. Guitar", vec![0, 7, 16, 15, 1, 9, 3, 26, 0, 6, 3, 0, 19, 15, 1, 0, 0, 27, 0, 3, 0, 0, 26, 15, 2, 0, 2, 26, 0, 1, 5, 0, 21, 31, 0, 7, 0, 0, 0, 1, 0, 0]);

        self._begin_category("valsound.lead");
        self._create_opn_voice("valsound.lead1", "Acoustic Code", vec![4, 4, 15, 0, 0, 12, 0, 28, 0, 8, 3, 0, 17, 6, 1, 12, 1, 0, 1, 8, 3, 0, 15, 0, 0, 12, 0, 21, 0, 4, 7, 0, 17, 6, 1, 12, 1, 0, 1, 4, 7, 0]);
        self._create_opn_voice("valsound.lead2", "Analog Synth 1", vec![1, 6, 31, 10, 0, 8, 5, 18, 0, 10, 0, 0, 31, 5, 1, 8, 2, 30, 0, 2, 0, 0, 31, 5, 1, 8, 2, 50, 0, 8, 0, 0, 31, 5, 1, 8, 2, 0, 0, 2, 0, 0]);
        self._create_opn_voice("valsound.lead3", "Bosco Lead", vec![6, 5, 28, 2, 2, 6, 0, 20, 0, 5, 7, 0, 10, 4, 4, 6, 0, 10, 0, 2, 3, 0, 15, 2, 2, 6, 0, 0, 0, 3, 7, 0, 15, 4, 4, 6, 0, 0, 0, 1, 3, 0]);
        self._create_opn_voice("valsound.lead4", "Cosmo Lead", vec![3, 7, 31, 0, 0, 0, 0, 25, 0, 0, 3, 0, 15, 0, 0, 1, 0, 25, 0, 1, 7, 0, 22, 0, 0, 1, 0, 23, 0, 1, 7, 0, 18, 0, 0, 6, 0, 0, 0, 1, 0, 0]);
        self._create_opn_voice("valsound.lead5", "Cosmo Lead 2", vec![3, 7, 31, 0, 0, 0, 0, 33, 0, 0, 3, 0, 15, 0, 0, 1, 0, 30, 0, 1, 7, 0, 22, 0, 0, 1, 0, 28, 0, 0, 7, 0, 18, 0, 0, 6, 0, 0, 0, 1, 0, 0]);
        self._create_opn_voice("valsound.lead6", "Digital Lead #1", vec![2, 7, 31, 0, 0, 0, 0, 26, 0, 1, 0, 0, 31, 0, 0, 0, 0, 37, 0, 2, 3, 0, 31, 0, 0, 0, 0, 27, 0, 2, 7, 0, 31, 12, 0, 15, 1, 0, 0, 1, 0, 0]);
        self._create_opn_voice("valsound.lead7", "Double Sin Wave", vec![7, 4, 18, 4, 0, 10, 1, 0, 0, 1, 3, 0, 18, 4, 0, 7, 1, 0, 0, 4, 3, 0, 17, 4, 0, 10, 1, 0, 0, 1, 7, 0, 14, 4, 0, 7, 1, 0, 0, 4, 7, 0]);
        self._create_opn_voice("valsound.lead8", "E.Organ 2 (bright)", vec![6, 7, 31, 0, 0, 9, 0, 33, 0, 5, 7, 0, 31, 13, 0, 9, 1, 0, 0, 3, 3, 0, 31, 0, 0, 9, 0, 3, 0, 2, 3, 0, 31, 0, 0, 9, 0, 0, 0, 1, 7, 0]);
        self._create_opn_voice("valsound.lead9", "E.Organ 2 (voice)", vec![6, 3, 31, 15, 0, 15, 3, 35, 0, 7, 7, 0, 31, 0, 0, 9, 0, 0, 0, 3, 3, 0, 31, 15, 3, 11, 1, 4, 0, 1, 3, 0, 31, 15, 0, 11, 1, 4, 0, 2, 7, 0]);
        self._create_opn_voice("valsound.lead10", "E.Organ 4 (click)", vec![6, 3, 31, 0, 0, 4, 1, 33, 0, 2, 0, 0, 31, 0, 0, 10, 1, 0, 0, 1, 3, 0, 31, 12, 0, 10, 1, 0, 0, 4, 7, 0, 31, 16, 0, 12, 6, 0, 1, 8, 3, 0]);
        self._create_opn_voice("valsound.lead11", "E.Organ 5 (click)", vec![6, 2, 31, 0, 0, 4, 1, 35, 0, 2, 0, 0, 31, 0, 0, 10, 1, 0, 0, 2, 3, 0, 31, 12, 0, 10, 1, 0, 0, 4, 7, 0, 28, 16, 0, 14, 8, 0, 1, 8, 3, 0]);
        self._create_opn_voice("valsound.lead12", "E.Organ 6", vec![6, 7, 31, 15, 0, 0, 1, 33, 0, 7, 7, 0, 31, 10, 0, 9, 1, 0, 0, 4, 3, 0, 31, 0, 0, 9, 0, 3, 0, 1, 3, 0, 31, 0, 0, 9, 0, 0, 0, 2, 7, 0]);
        self._create_opn_voice("valsound.lead13", "E.Organ 7 (church)", vec![6, 7, 31, 0, 0, 9, 0, 33, 0, 4, 7, 0, 31, 0, 0, 9, 0, 0, 0, 4, 3, 0, 31, 0, 0, 9, 0, 0, 0, 2, 3, 0, 31, 0, 0, 9, 0, 0, 0, 1, 7, 0]);
        self._create_opn_voice("valsound.lead14", "Metal Lead", vec![0, 7, 25, 5, 0, 15, 4, 22, 0, 2, 3, 0, 21, 2, 0, 12, 3, 26, 0, 7, 0, 0, 18, 7, 5, 8, 4, 27, 0, 6, 7, 0, 21, 5, 3, 8, 2, 0, 0, 4, 0, 0]);
        self._create_opn_voice("valsound.lead15", "Metal Lead 3", vec![2, 7, 31, 10, 0, 0, 1, 25, 0, 4, 3, 0, 31, 5, 0, 4, 15, 25, 0, 0, 3, 0, 31, 9, 0, 6, 10, 37, 0, 4, 7, 0, 31, 0, 0, 9, 0, 0, 0, 2, 7, 0]);
        self._create_opn_voice("valsound.lead16", "Mono Lead", vec![3, 7, 24, 11, 1, 0, 8, 42, 2, 4, 2, 0, 24, 9, 1, 0, 5, 19, 2, 4, 6, 0, 23, 9, 2, 0, 10, 25, 2, 8, 1, 0, 23, 5, 3, 11, 8, 0, 1, 1, 0, 0]);
        self._create_opn_voice("valsound.lead17", "PSG like PC88 (long)", vec![1, 7, 31, 0, 0, 15, 0, 27, 0, 2, 0, 0, 31, 0, 0, 15, 0, 50, 0, 1, 0, 0, 31, 0, 0, 15, 0, 40, 0, 2, 0, 0, 31, 0, 0, 15, 0, 0, 0, 1, 0, 0]);
        self._create_opn_voice("valsound.lead18", "PSG Cut 1", vec![5, 7, 31, 0, 0, 0, 0, 30, 0, 2, 0, 0, 31, 15, 0, 15, 3, 0, 0, 1, 0, 0, 31, 15, 0, 15, 3, 0, 0, 1, 0, 0, 31, 15, 0, 15, 3, 0, 0, 1, 0, 0]);
        self._create_opn_voice("valsound.lead19", "Attack Synth", vec![0, 7, 31, 15, 1, 0, 1, 40, 0, 8, 0, 0, 31, 15, 1, 0, 1, 20, 1, 4, 0, 0, 31, 15, 1, 0, 1, 37, 0, 1, 0, 0, 31, 15, 1, 8, 3, 0, 0, 1, 0, 0]);
        self._create_opn_voice("valsound.lead20", "Sin Wave", vec![6, 2, 31, 0, 0, 15, 0, 43, 0, 0, 0, 0, 31, 0, 0, 15, 0, 0, 0, 0, 0, 0, 31, 0, 0, 15, 0, 0, 0, 0, 0, 0, 31, 0, 0, 15, 0, 0, 0, 0, 0, 0]);
        self._create_opn_voice("valsound.lead21", "Synth & Bell 2", vec![4, 7, 21, 0, 1, 11, 0, 29, 1, 2, 3, 0, 14, 8, 0, 13, 1, 8, 0, 4, 3, 0, 31, 11, 0, 0, 2, 35, 0, 14, 3, 0, 31, 8, 5, 10, 15, 0, 0, 4, 7, 0]);
        self._create_opn_voice("valsound.lead22", "Chorus #2 (voice) & Bell", vec![4, 7, 21, 0, 1, 11, 0, 35, 1, 2, 3, 0, 14, 8, 0, 13, 1, 0, 0, 2, 3, 0, 31, 12, 0, 0, 2, 44, 0, 14, 3, 0, 31, 9, 5, 10, 15, 0, 0, 8, 7, 0]);
        self._create_opn_voice("valsound.lead23", "Synth 8-4 (cut)", vec![4, 7, 31, 0, 0, 0, 0, 30, 1, 8, 3, 0, 18, 13, 9, 7, 1, 0, 1, 8, 3, 0, 31, 0, 0, 0, 0, 22, 1, 4, 7, 0, 21, 13, 9, 7, 1, 0, 1, 4, 7, 0]);
        self._create_opn_voice("valsound.lead24", "Synth 8-4 (long)", vec![4, 7, 31, 0, 0, 0, 0, 30, 1, 8, 3, 0, 18, 13, 1, 7, 1, 0, 1, 8, 3, 0, 31, 0, 0, 0, 0, 22, 1, 4, 7, 0, 21, 13, 1, 7, 1, 0, 1, 4, 7, 0]);
        self._create_opn_voice("valsound.lead25", "Acoustic Code #2", vec![4, 7, 31, 0, 0, 0, 0, 28, 0, 4, 3, 0, 31, 10, 0, 7, 1, 0, 0, 4, 3, 0, 31, 0, 0, 0, 0, 21, 0, 4, 7, 0, 31, 10, 0, 7, 1, 0, 0, 4, 7, 0]);
        self._create_opn_voice("valsound.lead26", "Acoustic Code #3", vec![4, 7, 31, 0, 0, 0, 0, 28, 0, 4, 3, 0, 31, 10, 0, 7, 1, 0, 0, 8, 3, 0, 31, 0, 0, 0, 0, 21, 0, 4, 7, 0, 31, 10, 0, 7, 1, 0, 0, 4, 7, 0]);
        self._create_opn_voice("valsound.lead27", "Synth FB 4 (long)", vec![3, 7, 25, 7, 0, 0, 5, 23, 0, 2, 7, 0, 17, 0, 0, 9, 0, 32, 0, 4, 3, 0, 25, 7, 0, 0, 6, 27, 0, 2, 3, 0, 16, 8, 0, 9, 1, 0, 0, 2, 0, 0]);
        self._create_opn_voice("valsound.lead28", "Synth FB 5 (long)", vec![4, 7, 22, 4, 0, 0, 3, 22, 0, 2, 3, 0, 16, 8, 0, 9, 2, 0, 0, 8, 3, 0, 22, 0, 0, 0, 0, 15, 0, 2, 7, 0, 16, 8, 0, 9, 2, 0, 0, 8, 7, 0]);
        self._create_opn_voice("valsound.lead29", "Synth Lead 0", vec![4, 6, 24, 7, 1, 0, 0, 23, 0, 1, 3, 0, 23, 8, 0, 6, 1, 0, 0, 1, 3, 0, 24, 7, 1, 0, 0, 12, 0, 1, 7, 0, 15, 8, 0, 8, 1, 8, 0, 3, 7, 0]);
        self._create_opn_voice("valsound.lead30", "Synth Lead 1", vec![3, 7, 14, 10, 0, 15, 1, 25, 0, 2, 0, 0, 31, 0, 7, 15, 0, 15, 0, 1, 3, 0, 31, 0, 0, 15, 0, 30, 0, 2, 7, 0, 31, 0, 0, 15, 0, 0, 0, 1, 0, 0]);
        self._create_opn_voice("valsound.lead31", "Synth Lead 2", vec![2, 7, 31, 4, 2, 8, 1, 25, 0, 4, 3, 0, 14, 14, 0, 8, 5, 32, 1, 4, 0, 0, 21, 0, 2, 8, 0, 35, 0, 2, 7, 0, 21, 4, 1, 8, 3, 0, 0, 2, 0, 0]);
        self._create_opn_voice("valsound.lead32", "Synth Lead 3", vec![3, 7, 20, 0, 0, 0, 0, 29, 0, 2, 3, 0, 18, 12, 0, 8, 1, 25, 0, 2, 7, 0, 20, 12, 0, 8, 1, 30, 0, 1, 3, 0, 22, 12, 0, 8, 1, 0, 0, 2, 0, 0]);
        self._create_opn_voice("valsound.lead33", "Synth Lead 4", vec![4, 5, 25, 31, 1, 3, 1, 10, 0, 2, 3, 0, 31, 10, 1, 10, 2, 0, 0, 4, 7, 0, 25, 31, 1, 3, 1, 5, 1, 2, 7, 0, 31, 10, 1, 10, 2, 0, 0, 4, 3, 0]);
        self._create_opn_voice("valsound.lead34", "Synth Lead 5", vec![4, 6, 31, 10, 0, 8, 2, 16, 0, 11, 7, 0, 31, 3, 0, 8, 2, 18, 0, 1, 7, 0, 31, 3, 0, 8, 2, 50, 0, 8, 3, 0, 31, 3, 0, 8, 2, 0, 0, 2, 3, 0]);
        self._create_opn_voice("valsound.lead35", "Synth Lead 6", vec![4, 5, 31, 0, 0, 0, 0, 22, 0, 2, 7, 0, 18, 10, 0, 6, 1, 0, 0, 8, 7, 0, 31, 0, 0, 0, 0, 23, 0, 4, 3, 0, 18, 10, 0, 6, 1, 0, 0, 4, 3, 0]);
        self._create_opn_voice("valsound.lead36", "Synth Lead 7 (soft FB)", vec![1, 7, 31, 0, 0, 0, 0, 23, 0, 2, 1, 0, 31, 10, 8, 0, 5, 20, 0, 2, 7, 0, 15, 12, 0, 12, 2, 36, 0, 6, 5, 0, 18, 0, 0, 6, 0, 0, 0, 2, 0, 0]);
        self._create_opn_voice("valsound.lead37", "Synth PSG", vec![0, 7, 31, 1, 3, 0, 15, 21, 0, 2, 3, 0, 31, 1, 6, 0, 15, 41, 0, 4, 3, 0, 31, 1, 3, 0, 15, 22, 0, 1, 3, 0, 31, 13, 0, 6, 2, 0, 0, 1, 3, 0]);
        self._create_opn_voice("valsound.lead38", "Synth PSG 2", vec![0, 7, 17, 1, 3, 8, 15, 32, 0, 8, 3, 0, 19, 1, 6, 8, 15, 35, 0, 4, 3, 0, 22, 1, 3, 8, 15, 20, 0, 2, 3, 0, 31, 11, 0, 8, 2, 0, 0, 1, 3, 0]);
        self._create_opn_voice("valsound.lead39", "Synth PSG 3", vec![5, 7, 31, 0, 0, 0, 0, 24, 0, 2, 0, 0, 31, 15, 0, 9, 3, 6, 0, 1, 0, 0, 31, 15, 0, 9, 3, 6, 0, 1, 0, 0, 31, 15, 0, 9, 3, 6, 0, 2, 0, 0]);
        self._create_opn_voice("valsound.lead40", "Synth PSG 4", vec![5, 7, 31, 0, 0, 0, 0, 22, 0, 1, 0, 0, 31, 15, 1, 9, 3, 0, 0, 1, 0, 0, 31, 15, 1, 9, 3, 0, 0, 0, 0, 0, 31, 15, 2, 9, 4, 10, 0, 2, 0, 0]);
        self._create_opn_voice("valsound.lead41", "Synth PSG 5", vec![1, 7, 31, 0, 0, 15, 0, 28, 0, 5, 0, 0, 31, 0, 0, 15, 0, 45, 0, 3, 0, 0, 31, 0, 0, 15, 0, 45, 0, 2, 0, 0, 31, 0, 0, 15, 0, 0, 0, 1, 0, 0]);
        self._create_opn_voice("valsound.lead42", "Sin Water Synth", vec![6, 0, 31, 0, 0, 15, 0, 44, 0, 1, 0, 0, 24, 0, 0, 15, 0, 2, 0, 1, 0, 0, 25, 21, 0, 15, 15, 14, 0, 6, 0, 0, 24, 0, 0, 15, 0, 4, 0, 2, 0, 0]);

        self._begin_category("valsound.percus");
        self._create_opn_voice("valsound.percus1", "Bass Drum 2", vec![0, 0, 30, 26, 0, 13, 15, 26, 0, 1, 0, 0, 30, 28, 0, 14, 15, 37, 0, 15, 3, 0, 30, 16, 0, 8, 15, 5, 0, 0, 0, 0, 29, 16, 0, 8, 15, 0, 0, 0, 0, 0]);
        self._create_opn_voice("valsound.percus2", "Bass Drum 3 (o1f)", vec![2, 5, 24, 19, 0, 0, 15, 30, 2, 1, 3, 0, 31, 18, 13, 14, 15, 30, 1, 0, 0, 0, 31, 19, 13, 8, 15, 5, 1, 1, 7, 0, 31, 16, 15, 12, 15, 0, 1, 1, 0, 0]);
        self._create_opn_voice("valsound.percus3", "Bass Drum RUFINA (o2c)", vec![5, 5, 29, 20, 18, 15, 5, 11, 1, 0, 2, 0, 31, 16, 18, 15, 5, 2, 1, 0, 0, 0, 31, 16, 17, 15, 3, 0, 1, 0, 0, 0, 31, 15, 18, 15, 4, 0, 1, 0, 0, 0]);
        self._create_opn_voice("valsound.percus4", "Bass Drum (-vBend)", vec![3, 7, 31, 8, 0, 1, 5, 8, 2, 15, 1, 0, 31, 21, 4, 1, 12, 18, 2, 1, 0, 0, 31, 26, 0, 15, 15, 14, 2, 0, 0, 0, 31, 13, 10, 15, 15, 0, 2, 0, 0, 0]);
        self._create_opn_voice("valsound.percus5", "Bass Drum 808 2 (-vBend)", vec![6, 3, 31, 16, 10, 15, 15, 15, 0, 0, 0, 0, 31, 15, 10, 15, 15, 0, 0, 0, 0, 0, 28, 15, 20, 15, 15, 0, 0, 2, 3, 0, 26, 15, 20, 15, 15, 0, 0, 2, 7, 0]);
        self._create_opn_voice("valsound.percus6", "Cho-Cho 3 (o2e)", vec![4, 2, 18, 18, 0, 14, 15, 0, 3, 4, 7, 0, 17, 17, 0, 14, 15, 0, 2, 2, 3, 0, 18, 18, 0, 14, 15, 0, 3, 4, 3, 0, 17, 17, 0, 14, 15, 0, 2, 2, 7, 0]);
        self._create_opn_voice("valsound.percus7", "Cowbell 1", vec![3, 7, 31, 18, 19, 6, 2, 8, 1, 12, 1, 0, 31, 18, 12, 6, 2, 35, 1, 7, 2, 0, 31, 17, 13, 6, 3, 32, 1, 7, 3, 0, 31, 19, 15, 9, 1, 0, 0, 2, 7, 0]);
        self._create_opn_voice("valsound.percus8", "Crash Cymbal (noise)", vec![4, 7, 31, 0, 0, 0, 0, 0, 0, 15, 7, 0, 21, 10, 11, 13, 5, 0, 1, 0, 7, 0, 31, 0, 0, 14, 0, 0, 0, 8, 3, 0, 31, 9, 9, 9, 15, 5, 2, 15, 3, 0]);
        self._create_opn_voice("valsound.percus9", "Crash Noise", vec![0, 7, 23, 2, 8, 2, 15, 0, 0, 15, 3, 0, 25, 2, 8, 2, 15, 14, 1, 12, 7, 0, 22, 2, 8, 5, 15, 4, 0, 3, 3, 0, 23, 7, 8, 5, 15, 0, 0, 6, 7, 0]);
        self._create_opn_voice("valsound.percus10", "Crash Noise (short)", vec![0, 7, 23, 2, 8, 2, 15, 0, 2, 15, 3, 0, 25, 2, 8, 2, 15, 14, 3, 12, 7, 0, 22, 2, 8, 5, 15, 4, 2, 3, 3, 0, 23, 7, 8, 5, 15, 0, 2, 6, 7, 0]);
        self._create_opn_voice("valsound.percus11", "Ethnic Percus. 0", vec![3, 7, 31, 19, 6, 3, 13, 40, 1, 10, 3, 0, 31, 12, 4, 0, 5, 34, 1, 4, 3, 0, 31, 16, 6, 10, 14, 36, 1, 2, 7, 0, 31, 14, 6, 6, 15, 0, 1, 0, 0, 0]);
        self._create_opn_voice("valsound.percus12", "Ethnic Percus. 1", vec![4, 6, 31, 16, 0, 5, 15, 35, 0, 0, 3, 0, 31, 5, 15, 15, 15, 0, 0, 0, 3, 0, 31, 21, 0, 15, 11, 15, 0, 4, 1, 0, 31, 20, 21, 9, 2, 0, 0, 0, 7, 0]);
        self._create_opn_voice("valsound.percus13", "Heavy Bass Drum 1", vec![5, 0, 31, 15, 0, 8, 15, 10, 0, 0, 0, 0, 31, 13, 0, 8, 15, 0, 0, 0, 0, 0, 31, 13, 0, 8, 15, 0, 0, 0, 0, 0, 31, 24, 0, 9, 15, 20, 0, 1, 0, 0]);
        self._create_opn_voice("valsound.percus14", "Heavy Bass Drum 2", vec![5, 3, 31, 16, 10, 8, 15, 10, 0, 0, 0, 0, 31, 15, 10, 8, 15, 0, 0, 0, 0, 0, 31, 10, 10, 8, 14, 0, 0, 0, 0, 0, 31, 20, 10, 8, 15, 10, 0, 1, 0, 0]);
        self._create_opn_voice("valsound.percus15", "Heavy Snare Drum 1", vec![4, 7, 31, 0, 0, 0, 0, 0, 0, 15, 0, 0, 28, 0, 15, 13, 0, 0, 0, 4, 0, 0, 31, 18, 15, 7, 4, 0, 1, 0, 0, 0, 31, 5, 12, 7, 0, 0, 0, 0, 0, 0]);
        self._create_opn_voice("valsound.percus16", "Closed Hi-Hat 3", vec![4, 7, 31, 0, 0, 0, 0, 0, 0, 15, 7, 0, 18, 17, 18, 13, 5, 0, 0, 0, 7, 0, 31, 0, 0, 14, 0, 0, 0, 8, 3, 0, 18, 16, 13, 9, 15, 5, 1, 15, 3, 0]);
        self._create_opn_voice("valsound.percus17", "Closed Hi-Hat 4", vec![4, 7, 31, 11, 0, 0, 5, 0, 1, 15, 0, 0, 31, 17, 12, 9, 9, 0, 1, 0, 0, 0, 31, 0, 0, 15, 0, 0, 1, 0, 0, 0, 25, 19, 20, 15, 15, 7, 0, 15, 0, 0]);
        self._create_opn_voice("valsound.percus18", "Closed Hi-Hat 5", vec![4, 7, 31, 0, 0, 0, 0, 0, 0, 15, 7, 0, 21, 18, 18, 13, 5, 0, 0, 0, 7, 0, 31, 0, 0, 14, 0, 0, 0, 8, 3, 0, 31, 17, 13, 9, 15, 5, 1, 15, 3, 0]);
        self._create_opn_voice("valsound.percus19", "Closed Hi-Hat 6 -808-", vec![0, 7, 27, 0, 10, 0, 15, 39, 0, 15, 0, 0, 31, 4, 10, 14, 15, 30, 0, 11, 0, 0, 31, 10, 10, 14, 15, 5, 1, 9, 7, 0, 31, 19, 10, 15, 15, 0, 1, 9, 3, 0]);
        self._create_opn_voice("valsound.percus20", "Metal Hi-Hat #7 (o3-6)", vec![4, 7, 31, 16, 10, 8, 10, 0, 0, 15, 3, 0, 31, 18, 7, 12, 15, 0, 1, 1, 3, 0, 31, 10, 10, 6, 10, 2, 0, 15, 7, 0, 21, 18, 7, 12, 15, 0, 1, 0, 7, 0]);
        self._create_opn_voice("valsound.percus21", "Closed Hi-Hat #8 (o4)", vec![4, 7, 25, 19, 0, 9, 3, 0, 1, 15, 0, 0, 22, 19, 16, 14, 5, 0, 1, 1, 0, 0, 31, 10, 15, 15, 5, 0, 0, 15, 0, 0, 31, 19, 19, 15, 2, 0, 1, 15, 0, 0]);
        self._create_opn_voice("valsound.percus22", "Open Hi-Hat (o4e-g+)", vec![4, 5, 31, 5, 6, 0, 5, 0, 2, 15, 7, 0, 31, 10, 4, 7, 8, 19, 2, 3, 1, 0, 31, 20, 6, 3, 3, 0, 2, 1, 7, 0, 31, 25, 6, 7, 10, 6, 3, 7, 0, 0]);
        self._create_opn_voice("valsound.percus23", "Open Metal Hi-Hat 2 (o4c-)", vec![4, 7, 31, 14, 0, 8, 3, 0, 1, 15, 3, 0, 31, 15, 8, 12, 13, 0, 1, 7, 3, 0, 31, 13, 0, 6, 3, 1, 0, 10, 7, 0, 31, 15, 11, 12, 12, 0, 1, 7, 7, 0]);
        self._create_opn_voice("valsound.percus24", "Open Metal Hi-Hat 3", vec![4, 7, 31, 14, 0, 8, 3, 0, 1, 15, 3, 0, 31, 15, 8, 12, 13, 0, 0, 7, 3, 0, 31, 13, 0, 6, 1, 1, 0, 10, 7, 0, 31, 15, 11, 12, 7, 0, 0, 7, 7, 0]);
        self._create_opn_voice("valsound.percus25", "Open Hi-Hat #4 (o4f)", vec![4, 6, 31, 15, 0, 9, 1, 0, 0, 15, 0, 0, 31, 20, 5, 14, 5, 3, 0, 4, 0, 0, 31, 10, 9, 9, 1, 0, 0, 10, 0, 0, 31, 22, 5, 14, 5, 0, 1, 7, 0, 0]);
        self._create_opn_voice("valsound.percus26", "Metal Ride (o4c,o5c)", vec![4, 5, 20, 5, 0, 0, 5, 11, 2, 15, 3, 0, 18, 11, 9, 7, 11, 0, 2, 8, 3, 0, 31, 19, 0, 3, 3, 0, 1, 15, 7, 0, 16, 12, 9, 7, 11, 0, 2, 7, 7, 0]);
        self._create_opn_voice("valsound.percus27", "Rim Shot #1 (o3c)", vec![0, 7, 31, 11, 0, 15, 15, 37, 1, 15, 1, 0, 31, 12, 0, 15, 15, 40, 1, 10, 2, 0, 31, 17, 0, 15, 15, 13, 2, 0, 3, 0, 31, 16, 0, 15, 15, 0, 2, 0, 7, 0]);
        self._create_opn_voice("valsound.percus28", "Snare Drum (light)", vec![4, 7, 31, 0, 0, 7, 0, 0, 0, 15, 0, 0, 31, 15, 15, 9, 2, 0, 0, 15, 0, 0, 31, 21, 0, 15, 11, 10, 0, 4, 1, 0, 31, 19, 17, 9, 2, 0, 0, 0, 7, 0]);
        self._create_opn_voice("valsound.percus29", "Snare Drum (lighter)", vec![4, 6, 31, 0, 0, 14, 0, 0, 0, 10, 3, 0, 31, 15, 15, 14, 1, 0, 0, 12, 7, 0, 31, 15, 0, 14, 15, 0, 3, 2, 3, 0, 31, 15, 0, 14, 15, 0, 2, 0, 7, 0]);
        self._create_opn_voice("valsound.percus30", "Snare Drum 808 (o2-o3)", vec![4, 7, 31, 0, 0, 0, 0, 5, 0, 15, 7, 0, 31, 18, 17, 15, 1, 0, 0, 9, 3, 0, 31, 19, 0, 15, 15, 0, 0, 0, 7, 0, 26, 21, 16, 15, 15, 0, 0, 0, 3, 0]);
        self._create_opn_voice("valsound.percus31", "Snare 4 -808- (o2)", vec![4, 7, 31, 12, 0, 12, 4, 0, 1, 7, 3, 0, 27, 15, 18, 15, 1, 0, 1, 15, 3, 0, 31, 20, 15, 12, 15, 11, 0, 1, 7, 0, 31, 19, 15, 15, 15, 0, 1, 1, 7, 0]);
        self._create_opn_voice("valsound.percus32", "Snare 5 Franger (o1-2)", vec![4, 7, 31, 16, 0, 0, 2, 6, 0, 15, 7, 0, 31, 18, 15, 15, 0, 0, 0, 9, 3, 0, 28, 20, 0, 15, 15, 0, 0, 0, 7, 0, 25, 16, 15, 15, 15, 0, 0, 0, 3, 0]);
        self._create_opn_voice("valsound.percus33", "Old Tom", vec![4, 7, 31, 11, 0, 1, 15, 0, 0, 15, 3, 0, 31, 20, 14, 15, 5, 0, 0, 1, 3, 0, 31, 16, 15, 5, 15, 48, 0, 0, 7, 0, 31, 11, 15, 15, 15, 0, 0, 0, 7, 0]);
        self._create_opn_voice("valsound.percus34", "Synth Tom 2 (AL=3)", vec![3, 7, 31, 4, 0, 1, 0, 0, 1, 15, 1, 0, 31, 21, 4, 1, 10, 15, 1, 1, 0, 0, 31, 26, 0, 15, 15, 0, 1, 0, 0, 0, 31, 11, 0, 7, 15, 0, 1, 0, 0, 0]);
        self._create_opn_voice("valsound.percus35", "Synth Tom #3 (noisy)", vec![3, 7, 31, 20, 0, 0, 2, 0, 0, 15, 3, 0, 31, 18, 13, 6, 8, 28, 0, 0, 3, 0, 31, 16, 9, 12, 5, 44, 0, 1, 7, 0, 31, 14, 4, 12, 1, 0, 0, 0, 7, 0]);
        self._create_opn_voice("valsound.percus36", "Synth Tom #3", vec![1, 7, 31, 18, 10, 0, 1, 0, 0, 15, 3, 0, 31, 15, 10, 6, 5, 0, 0, 5, 3, 0, 31, 17, 12, 12, 6, 37, 0, 1, 7, 0, 31, 14, 4, 12, 1, 0, 0, 0, 7, 0]);
        self._create_opn_voice("valsound.percus37", "Synth Tom #4 (-DX7-)", vec![3, 7, 31, 4, 0, 1, 0, 0, 1, 11, 0, 0, 31, 21, 4, 1, 6, 25, 1, 1, 3, 0, 31, 26, 0, 15, 15, 0, 1, 0, 0, 0, 31, 11, 0, 7, 15, 0, 1, 0, 0, 0]);
        self._create_opn_voice("valsound.percus38", "Triangle 1 (o5c)", vec![4, 5, 31, 18, 0, 11, 2, 9, 0, 14, 3, 0, 31, 21, 7, 12, 4, 0, 0, 8, 3, 0, 31, 22, 0, 12, 15, 0, 0, 15, 7, 0, 31, 20, 6, 15, 15, 0, 0, 7, 7, 0]);

        self._begin_category("valsound.piano");
        self._create_opn_voice("valsound.piano1", "Acoustic Piano 2 (attack)", vec![4, 5, 31, 5, 0, 0, 0, 23, 1, 1, 3, 0, 20, 10, 3, 7, 8, 0, 1, 1, 3, 0, 31, 3, 0, 0, 0, 25, 1, 1, 7, 0, 31, 12, 3, 7, 10, 2, 1, 1, 7, 0]);
        self._create_opn_voice("valsound.piano2", "Clavichord 1 (backing)", vec![2, 7, 31, 8, 3, 6, 2, 40, 2, 1, 3, 0, 31, 7, 4, 6, 2, 37, 1, 5, 2, 0, 31, 7, 2, 6, 1, 30, 1, 3, 7, 0, 28, 30, 9, 7, 0, 0, 2, 1, 4, 0]);
        self._create_opn_voice("valsound.piano3", "Clavichord 2", vec![2, 6, 31, 15, 8, 6, 2, 35, 0, 12, 3, 0, 31, 6, 2, 6, 2, 32, 0, 3, 0, 0, 31, 6, 2, 6, 1, 32, 0, 1, 7, 0, 31, 8, 6, 7, 4, 0, 0, 2, 0, 0]);
        self._create_opn_voice("valsound.piano4", "Deep Piano 1", vec![2, 5, 31, 9, 4, 0, 2, 38, 0, 0, 3, 0, 22, 7, 3, 9, 3, 31, 1, 3, 0, 0, 31, 7, 3, 2, 3, 27, 1, 0, 7, 0, 28, 7, 1, 7, 1, 0, 1, 0, 0, 0]);
        self._create_opn_voice("valsound.piano5", "Deep Piano 3", vec![2, 0, 31, 20, 9, 0, 2, 8, 1, 0, 7, 0, 31, 11, 3, 1, 1, 23, 1, 4, 3, 0, 31, 13, 5, 2, 2, 30, 0, 0, 3, 0, 31, 0, 4, 6, 0, 0, 1, 1, 7, 0]);
        self._create_opn_voice("valsound.piano6", "E.Piano #2", vec![4, 6, 22, 5, 0, 3, 5, 30, 0, 2, 3, 0, 16, 8, 8, 7, 2, 0, 1, 2, 3, 0, 20, 5, 0, 3, 5, 34, 0, 4, 7, 0, 17, 8, 7, 7, 2, 0, 1, 2, 7, 0]);
        self._create_opn_voice("valsound.piano7", "E.Piano #3", vec![4, 7, 22, 5, 0, 3, 5, 41, 0, 1, 3, 0, 16, 8, 8, 7, 2, 0, 1, 2, 3, 0, 31, 18, 0, 3, 10, 44, 0, 8, 7, 0, 31, 9, 7, 7, 2, 3, 1, 1, 7, 0]);
        self._create_opn_voice("valsound.piano8", "E.Piano #4 (2+)", vec![4, 6, 31, 5, 0, 15, 5, 46, 2, 2, 3, 0, 31, 9, 8, 15, 3, 0, 2, 2, 3, 0, 31, 5, 0, 15, 5, 44, 2, 4, 7, 0, 31, 9, 7, 15, 3, 0, 2, 2, 7, 0]);
        self._create_opn_voice("valsound.piano9", "E.Piano #5 (bell)", vec![4, 7, 31, 7, 0, 9, 5, 35, 0, 6, 3, 0, 31, 11, 7, 14, 4, 5, 1, 2, 3, 0, 31, 10, 9, 9, 5, 35, 0, 12, 7, 0, 31, 11, 7, 14, 4, 5, 0, 2, 7, 0]);
        self._create_opn_voice("valsound.piano10", "E.Piano #6", vec![4, 7, 29, 20, 0, 0, 3, 34, 0, 8, 3, 0, 17, 8, 0, 7, 6, 2, 0, 4, 3, 0, 30, 0, 0, 0, 0, 25, 0, 4, 7, 0, 18, 8, 0, 7, 6, 2, 0, 4, 7, 0]);
        self._create_opn_voice("valsound.piano11", "E.Piano #7", vec![4, 7, 31, 15, 0, 10, 15, 40, 0, 15, 0, 0, 31, 10, 0, 7, 15, 15, 0, 1, 0, 0, 31, 10, 0, 5, 15, 20, 0, 1, 0, 0, 31, 10, 0, 7, 15, 0, 0, 1, 0, 0]);
        self._create_opn_voice("valsound.piano12", "Harpsichord 1", vec![2, 5, 31, 13, 0, 15, 10, 30, 1, 0, 3, 0, 31, 11, 2, 0, 3, 32, 1, 7, 3, 0, 31, 2, 0, 0, 1, 30, 0, 0, 7, 0, 31, 6, 6, 7, 1, 0, 1, 4, 7, 0]);
        self._create_opn_voice("valsound.piano13", "Harpsichord 2", vec![2, 7, 31, 4, 0, 5, 1, 30, 2, 0, 3, 0, 31, 9, 1, 2, 1, 40, 2, 12, 0, 0, 31, 4, 3, 6, 1, 30, 1, 3, 7, 0, 31, 11, 5, 8, 4, 0, 2, 1, 0, 0]);
        self._create_opn_voice("valsound.piano14", "Piano 1 (ML1,10,5,1)", vec![2, 7, 28, 4, 0, 5, 1, 37, 2, 1, 3, 0, 22, 9, 1, 2, 1, 47, 2, 12, 0, 0, 29, 4, 3, 6, 1, 37, 1, 3, 7, 0, 18, 8, 0, 6, 6, 0, 2, 1, 0, 0]);
        self._create_opn_voice("valsound.piano15", "Piano 3", vec![2, 7, 31, 4, 2, 0, 1, 35, 2, 1, 3, 0, 24, 0, 1, 5, 0, 38, 3, 1, 0, 0, 28, 0, 0, 5, 0, 42, 2, 4, 5, 0, 28, 7, 4, 6, 4, 0, 2, 1, 0, 0]);
        self._create_opn_voice("valsound.piano16", "Piano 4", vec![2, 7, 31, 4, 0, 5, 1, 37, 2, 1, 3, 0, 31, 9, 1, 2, 1, 47, 2, 10, 0, 0, 31, 4, 3, 6, 1, 37, 1, 2, 7, 0, 31, 8, 0, 6, 6, 0, 1, 1, 0, 0]);
        self._create_opn_voice("valsound.piano17", "Digital Piano #5", vec![3, 7, 28, 4, 0, 7, 1, 27, 1, 1, 4, 0, 28, 14, 7, 4, 3, 42, 2, 14, 3, 0, 26, 4, 3, 8, 2, 38, 0, 3, 7, 0, 25, 7, 8, 7, 0, 0, 2, 1, 6, 0]);
        self._create_opn_voice("valsound.piano18", "Piano 6 (high-tone)", vec![2, 7, 28, 4, 0, 5, 1, 39, 2, 1, 3, 0, 31, 13, 1, 2, 2, 50, 2, 14, 0, 0, 29, 4, 3, 6, 1, 41, 1, 3, 7, 0, 21, 8, 6, 6, 6, 0, 2, 1, 0, 0]);
        self._create_opn_voice("valsound.piano19", "Panning Harpsichord", vec![2, 7, 31, 2, 10, 6, 14, 40, 3, 12, 3, 0, 25, 2, 15, 6, 14, 32, 0, 9, 0, 0, 30, 2, 4, 6, 14, 34, 3, 5, 7, 0, 20, 2, 8, 6, 14, 0, 3, 1, 0, 0]);
        self._create_opn_voice("valsound.piano20", "Yam Harpsichord", vec![1, 4, 31, 6, 5, 6, 7, 40, 0, 10, 0, 0, 31, 6, 4, 5, 5, 35, 0, 2, 0, 0, 31, 6, 5, 5, 5, 24, 0, 1, 0, 0, 31, 7, 6, 7, 5, 0, 0, 1, 0, 0]);

        self._begin_category("valsound.se");
        self._create_opn_voice("valsound.se1", "S.Effect 1 (detune, o2c)", vec![0, 4, 31, 7, 3, 0, 1, 12, 0, 0, 1, 0, 31, 10, 0, 8, 3, 25, 0, 0, 2, 0, 31, 6, 4, 8, 8, 0, 0, 4, 3, 0, 31, 12, 0, 8, 0, 0, 0, 12, 7, 0]);
        self._create_opn_voice("valsound.se2", "S.Effect 2 (o0-1-2)", vec![1, 2, 31, 6, 7, 15, 3, 8, 0, 0, 3, 0, 31, 6, 9, 15, 6, 8, 0, 0, 7, 0, 6, 6, 6, 15, 1, 10, 0, 12, 3, 0, 31, 5, 4, 15, 2, 0, 0, 0, 0, 0]);
        self._create_opn_voice("valsound.se3", "S.Effect 3 (FB + noise)", vec![4, 6, 4, 3, 0, 15, 4, 0, 0, 0, 7, 0, 8, 7, 7, 15, 1, 0, 0, 7, 7, 0, 5, 0, 0, 15, 0, 0, 0, 0, 3, 0, 8, 6, 3, 15, 1, 0, 0, 4, 3, 0]);

        self._begin_category("valsound.special");
        self._create_opn_voice("valsound.special1", "Digital 1", vec![3, 6, 31, 12, 3, 5, 5, 26, 0, 14, 7, 0, 31, 16, 6, 0, 3, 28, 0, 8, 3, 0, 31, 0, 12, 0, 0, 30, 0, 0, 0, 0, 31, 15, 12, 12, 2, 0, 0, 1, 7, 0]);
        self._create_opn_voice("valsound.special2", "Digital 2", vec![0, 7, 31, 15, 0, 12, 3, 27, 0, 10, 7, 0, 31, 16, 0, 0, 4, 30, 0, 15, 3, 0, 31, 15, 0, 0, 2, 30, 0, 2, 0, 0, 31, 15, 0, 12, 2, 0, 0, 1, 7, 0]);
        self._create_opn_voice("valsound.special3", "Digital Bass 3 (o2-o3)", vec![0, 7, 31, 0, 0, 9, 0, 27, 0, 12, 1, 0, 31, 10, 0, 9, 1, 25, 0, 0, 2, 0, 31, 10, 0, 9, 1, 25, 0, 12, 3, 0, 31, 12, 0, 14, 2, 3, 0, 1, 7, 0]);
        self._create_opn_voice("valsound.special4", "Digital Guitar 3 (o2-o3)", vec![0, 7, 31, 0, 0, 9, 0, 27, 0, 12, 1, 0, 31, 10, 0, 9, 1, 25, 0, 0, 2, 0, 31, 10, 0, 9, 1, 25, 0, 3, 3, 0, 31, 12, 0, 14, 2, 3, 0, 1, 7, 0]);
        self._create_opn_voice("valsound.special5", "Digital 4 (o4a)", vec![5, 0, 31, 31, 0, 0, 0, 61, 0, 9, 0, 0, 31, 31, 0, 13, 0, 3, 0, 3, 0, 0, 31, 31, 0, 13, 0, 15, 0, 9, 0, 0, 31, 31, 0, 13, 0, 6, 0, 6, 0, 0]);

        self._begin_category("valsound.strpad");
        self._create_opn_voice("valsound.strpad1", "Accordion 1", vec![4, 6, 17, 0, 0, 0, 0, 20, 0, 4, 3, 0, 16, 9, 0, 12, 2, 0, 0, 8, 3, 0, 15, 0, 0, 7, 0, 36, 0, 4, 7, 0, 15, 9, 0, 12, 2, 0, 0, 8, 7, 0]);
        self._create_opn_voice("valsound.strpad2", "Accordion 2", vec![4, 6, 21, 0, 1, 11, 0, 22, 1, 4, 3, 0, 14, 8, 0, 13, 1, 0, 0, 4, 3, 0, 21, 0, 1, 10, 0, 30, 1, 4, 7, 0, 14, 8, 1, 13, 1, 0, 0, 4, 7, 0]);
        self._create_opn_voice("valsound.strpad3", "Accordion 3", vec![4, 7, 31, 5, 0, 0, 0, 25, 0, 4, 7, 0, 14, 8, 0, 13, 1, 0, 0, 4, 7, 0, 31, 8, 0, 0, 10, 25, 0, 2, 3, 0, 14, 6, 0, 13, 1, 0, 0, 4, 3, 0]);
        self._create_opn_voice("valsound.strpad4", "Chorus #2 (voice)", vec![4, 6, 21, 0, 1, 11, 0, 40, 1, 4, 3, 0, 14, 8, 0, 13, 1, 0, 0, 4, 3, 0, 21, 0, 1, 10, 0, 37, 1, 4, 7, 0, 14, 8, 1, 13, 1, 0, 0, 4, 7, 0]);
        self._create_opn_voice("valsound.strpad5", "Chorus #3", vec![4, 4, 21, 0, 0, 2, 0, 42, 0, 4, 3, 0, 18, 4, 0, 9, 1, 0, 0, 8, 3, 0, 21, 0, 0, 2, 0, 45, 0, 4, 7, 0, 18, 4, 0, 9, 1, 0, 0, 4, 7, 0]);
        self._create_opn_voice("valsound.strpad6", "Chorus #4", vec![6, 3, 21, 0, 0, 2, 0, 39, 0, 4, 0, 0, 18, 4, 0, 9, 1, 0, 0, 4, 1, 0, 18, 10, 0, 9, 1, 0, 0, 4, 3, 0, 18, 8, 0, 9, 2, 0, 0, 2, 7, 0]);
        self._create_opn_voice("valsound.strpad7", "Fretless std::strings 1", vec![2, 7, 25, 10, 0, 5, 1, 29, 1, 1, 1, 0, 25, 11, 0, 8, 5, 15, 1, 5, 1, 0, 28, 13, 0, 6, 2, 45, 1, 1, 0, 0, 14, 4, 0, 6, 0, 0, 1, 1, 0, 0]);
        self._create_opn_voice("valsound.strpad8", "Fretless std::strings 2", vec![2, 0, 21, 7, 0, 7, 3, 37, 1, 1, 3, 0, 20, 11, 0, 12, 3, 15, 1, 5, 7, 0, 16, 8, 0, 12, 3, 45, 1, 1, 0, 0, 14, 5, 0, 12, 1, 0, 1, 1, 0, 0]);
        self._create_opn_voice("valsound.strpad9", "Fretless std::strings 3", vec![2, 7, 25, 10, 0, 5, 1, 35, 1, 1, 3, 0, 25, 11, 0, 8, 5, 13, 1, 5, 0, 0, 28, 13, 0, 6, 2, 45, 1, 1, 7, 0, 14, 4, 0, 6, 1, 0, 1, 1, 0, 0]);
        self._create_opn_voice("valsound.strpad10", "Fretless std::strings 4 (low)", vec![2, 7, 25, 10, 0, 5, 1, 29, 1, 0, 3, 0, 25, 11, 0, 8, 5, 20, 1, 4, 0, 0, 28, 13, 0, 6, 2, 38, 1, 1, 7, 0, 14, 4, 0, 6, 1, 0, 1, 1, 0, 0]);
        self._create_opn_voice("valsound.strpad11", "Pizzicato #1 (Koto 2)", vec![0, 6, 31, 7, 8, 1, 2, 30, 3, 3, 7, 0, 31, 5, 9, 1, 1, 30, 3, 2, 0, 0, 31, 5, 8, 3, 2, 35, 3, 1, 0, 0, 31, 11, 7, 5, 5, 0, 2, 1, 3, 0]);
        self._create_opn_voice("valsound.strpad12", "Soundtrack (Modoki)", vec![4, 7, 31, 0, 0, 0, 0, 30, 1, 2, 3, 0, 18, 13, 4, 7, 1, 0, 1, 2, 3, 0, 31, 0, 0, 0, 0, 22, 1, 3, 7, 0, 21, 13, 4, 7, 1, 0, 1, 3, 7, 0]);
        self._create_opn_voice("valsound.strpad13", "std::strings", vec![2, 7, 15, 9, 0, 5, 1, 27, 2, 2, 3, 0, 15, 0, 0, 5, 15, 31, 2, 2, 0, 0, 15, 0, 0, 5, 0, 27, 1, 2, 0, 0, 13, 3, 0, 8, 0, 0, 1, 2, 7, 0]);
        self._create_opn_voice("valsound.strpad14", "Synth Accordion", vec![4, 7, 18, 0, 0, 11, 0, 21, 1, 2, 3, 0, 15, 9, 0, 13, 2, 0, 1, 8, 3, 0, 18, 0, 0, 11, 0, 20, 1, 2, 7, 0, 14, 9, 0, 13, 2, 0, 1, 4, 7, 0]);
        self._create_opn_voice("valsound.strpad15", "Phaser Synth", vec![0, 7, 27, 31, 5, 5, 5, 8, 0, 1, 4, 0, 26, 31, 4, 5, 4, 18, 0, 1, 5, 0, 19, 31, 0, 5, 2, 18, 0, 1, 6, 0, 16, 15, 0, 6, 2, 0, 0, 3, 4, 0]);
        self._create_opn_voice("valsound.strpad16", "FB Synth", vec![3, 7, 31, 6, 0, 0, 4, 22, 0, 2, 0, 0, 18, 0, 0, 8, 0, 28, 0, 4, 6, 0, 20, 5, 0, 8, 2, 28, 0, 2, 1, 0, 20, 4, 0, 8, 1, 0, 0, 2, 2, 0]);
        self._create_opn_voice("valsound.strpad17", "Synth std::strings (MB)", vec![3, 7, 21, 0, 0, 0, 0, 30, 0, 1, 1, 0, 14, 7, 7, 6, 3, 25, 0, 2, 6, 0, 15, 0, 0, 5, 0, 38, 0, 5, 6, 0, 18, 0, 0, 6, 0, 0, 0, 2, 0, 0]);
        self._create_opn_voice("valsound.strpad18", "Synth std::strings #2", vec![2, 7, 20, 1, 0, 8, 1, 32, 0, 2, 7, 0, 15, 4, 0, 8, 1, 28, 0, 2, 0, 0, 22, 1, 0, 8, 1, 34, 0, 2, 3, 0, 14, 2, 0, 9, 1, 0, 0, 2, 0, 0]);
        self._create_opn_voice("valsound.strpad19", "Synth Sweep Pad #1", vec![5, 6, 31, 7, 0, 9, 5, 18, 0, 4, 0, 0, 31, 15, 0, 14, 1, 5, 0, 8, 0, 0, 31, 16, 0, 15, 2, 5, 0, 4, 0, 0, 31, 26, 0, 14, 2, 5, 0, 8, 1, 0]);
        self._create_opn_voice("valsound.strpad20", "Twin Synth #1 (calm)", vec![4, 2, 16, 3, 0, 0, 2, 25, 0, 4, 3, 0, 19, 12, 0, 6, 1, 0, 0, 4, 3, 0, 16, 3, 0, 0, 2, 20, 0, 3, 7, 0, 19, 12, 0, 6, 1, 0, 0, 3, 7, 0]);
        self._create_opn_voice("valsound.strpad21", "Twin Synth #2 (FB)", vec![4, 6, 16, 3, 0, 0, 2, 20, 0, 4, 7, 0, 19, 12, 0, 6, 1, 0, 0, 8, 3, 0, 16, 3, 0, 0, 2, 6, 0, 3, 3, 0, 19, 12, 0, 6, 1, 8, 0, 6, 7, 0]);
        self._create_opn_voice("valsound.strpad22", "Twin Synth #3 (FB)", vec![4, 6, 16, 6, 5, 0, 2, 20, 0, 4, 3, 0, 19, 12, 0, 6, 1, 0, 0, 8, 3, 0, 16, 6, 5, 0, 2, 6, 0, 3, 7, 0, 19, 12, 0, 6, 1, 8, 0, 6, 7, 0]);
        self._create_opn_voice("valsound.strpad23", "Vocoder Voice 1", vec![4, 7, 31, 8, 5, 12, 15, 20, 0, 4, 7, 0, 31, 0, 0, 15, 0, 0, 0, 8, 7, 0, 31, 8, 5, 12, 15, 24, 0, 3, 3, 0, 31, 0, 0, 15, 0, 0, 0, 6, 3, 0]);
        self._create_opn_voice("valsound.strpad24", "Voice (o3-o5)", vec![6, 0, 10, 0, 1, 3, 0, 70, 0, 1, 0, 0, 12, 0, 0, 5, 0, 7, 2, 3, 3, 0, 12, 0, 1, 6, 2, 0, 1, 2, 7, 0, 18, 0, 0, 6, 0, 17, 1, 0, 3, 0]);
        self._create_opn_voice("valsound.strpad25", "Voice 2 (o3-o5)", vec![6, 0, 10, 0, 1, 3, 0, 70, 0, 0, 0, 0, 12, 0, 0, 5, 0, 6, 2, 3, 3, 0, 12, 0, 1, 6, 2, 0, 1, 2, 7, 0, 18, 0, 0, 6, 0, 10, 1, 1, 3, 0]);

        self._begin_category("valsound.wind");
        self._create_opn_voice("valsound.wind1", "Clarinet #1", vec![3, 7, 31, 0, 0, 7, 0, 35, 0, 4, 0, 0, 25, 14, 0, 4, 2, 42, 0, 4, 0, 0, 31, 0, 0, 8, 0, 38, 0, 2, 0, 0, 18, 7, 0, 8, 1, 0, 0, 1, 0, 0]);
        self._create_opn_voice("valsound.wind2", "Clarinet #2 (brighter)", vec![3, 7, 31, 0, 0, 7, 0, 40, 0, 8, 0, 0, 27, 17, 0, 4, 4, 45, 0, 8, 0, 0, 31, 0, 0, 8, 0, 37, 0, 4, 0, 0, 18, 7, 0, 8, 1, 0, 0, 1, 0, 0]);
        self._create_opn_voice("valsound.wind3", "E.Flute", vec![3, 0, 20, 0, 0, 10, 0, 38, 0, 6, 0, 0, 14, 16, 0, 10, 5, 36, 0, 2, 3, 0, 18, 18, 0, 10, 3, 40, 0, 4, 0, 0, 14, 12, 0, 10, 1, 0, 1, 2, 0, 0]);
        self._create_opn_voice("valsound.wind4", "E.Flute 2", vec![3, 5, 20, 0, 0, 10, 0, 28, 0, 2, 3, 0, 14, 16, 0, 10, 5, 40, 0, 2, 3, 0, 18, 18, 0, 10, 3, 34, 0, 4, 0, 0, 14, 12, 0, 10, 1, 0, 1, 2, 7, 0]);
        self._create_opn_voice("valsound.wind5", "Flute + Bell", vec![4, 5, 16, 0, 0, 8, 1, 47, 0, 8, 3, 0, 14, 0, 0, 8, 0, 0, 0, 4, 3, 0, 31, 12, 0, 8, 2, 45, 0, 14, 7, 0, 31, 8, 0, 8, 15, 0, 0, 8, 7, 0]);
        self._create_opn_voice("valsound.wind6", "Old Flute", vec![2, 7, 20, 5, 0, 14, 1, 50, 0, 4, 0, 0, 15, 15, 0, 14, 2, 45, 0, 8, 0, 0, 18, 15, 0, 14, 2, 50, 0, 8, 0, 0, 14, 2, 0, 14, 0, 0, 0, 4, 0, 0]);
        self._create_opn_voice("valsound.wind7", "Whistle 1", vec![2, 7, 20, 5, 0, 14, 1, 60, 0, 4, 0, 0, 15, 15, 0, 14, 2, 55, 0, 12, 0, 0, 18, 15, 0, 14, 2, 60, 0, 8, 0, 0, 14, 2, 0, 14, 0, 0, 0, 4, 0, 0]);
        self._create_opn_voice("valsound.wind8", "Whistle 2", vec![2, 7, 20, 5, 0, 14, 1, 55, 0, 2, 0, 0, 15, 15, 0, 14, 2, 55, 0, 8, 0, 0, 18, 15, 0, 14, 2, 60, 0, 8, 0, 0, 14, 2, 0, 14, 0, 0, 0, 4, 0, 0]);

        self._begin_category("valsound.world");
        self._create_opn_voice("valsound.world1", "Banjo (Harpsichord)", vec![1, 7, 31, 7, 0, 10, 15, 38, 0, 12, 7, 0, 31, 8, 6, 7, 3, 52, 2, 10, 1, 0, 31, 12, 6, 7, 3, 25, 0, 1, 0, 0, 31, 11, 7, 7, 3, 0, 2, 3, 5, 0]);
        self._create_opn_voice("valsound.world2", "Koto 1", vec![3, 0, 31, 0, 0, 10, 0, 38, 0, 6, 0, 0, 24, 13, 0, 10, 5, 40, 0, 2, 3, 0, 28, 15, 0, 10, 3, 40, 0, 4, 0, 0, 24, 12, 8, 10, 2, 0, 1, 2, 0, 0]);
        self._create_opn_voice("valsound.world3", "Koto 2", vec![0, 7, 31, 7, 3, 3, 2, 30, 3, 3, 7, 0, 31, 5, 3, 3, 1, 30, 3, 2, 0, 0, 31, 5, 3, 5, 2, 30, 3, 1, 0, 0, 31, 10, 3, 7, 2, 0, 3, 1, 3, 0]);
        self._create_opn_voice("valsound.world4", "Sitar 1", vec![0, 6, 18, 5, 3, 1, 2, 30, 1, 3, 7, 0, 31, 5, 4, 1, 1, 28, 1, 2, 0, 0, 31, 5, 3, 3, 2, 35, 1, 1, 0, 0, 31, 10, 2, 5, 4, 0, 0, 1, 3, 0]);
        self._create_opn_voice("valsound.world5", "Shamisen 2", vec![3, 7, 31, 16, 6, 7, 2, 33, 0, 1, 3, 0, 31, 16, 6, 7, 4, 18, 2, 6, 0, 0, 31, 6, 6, 7, 1, 40, 0, 1, 7, 0, 31, 15, 6, 7, 5, 0, 0, 2, 0, 0]);
        self._create_opn_voice("valsound.world6", "Shamisen 1", vec![2, 7, 31, 16, 6, 7, 2, 33, 0, 1, 3, 0, 31, 16, 6, 7, 4, 18, 2, 8, 0, 0, 31, 6, 6, 7, 1, 40, 0, 1, 7, 0, 31, 15, 6, 7, 5, 0, 0, 2, 0, 0]);
        self._create_opn_voice("valsound.world7", "Synth Shamisen", vec![2, 7, 31, 16, 6, 7, 1, 33, 0, 1, 3, 0, 31, 16, 6, 7, 4, 18, 2, 7, 0, 0, 31, 6, 6, 7, 0, 40, 0, 1, 7, 0, 31, 15, 6, 7, 2, 0, 0, 2, 0, 0]);
    }

    /// C++ `_generate_midi_voices` — cpp lines 351-499.
    fn _generate_midi_voices(&mut self) {
        self._begin_category("midi");
        self._create_ma3_voice("midi.piano1", "GrandPno", vec![3, 0, 8, 15, 7, 0, 6, 15, 39, 0, 1, 1, 0, 0, 0, 14, 3, 2, 3, 2, 28, 1, 3, 5, 0, 0, 0, 13, 1, 1, 4, 3, 22, 0, 0, 1, 0, 0, 0, 13, 3, 2, 6, 4, 0, 1, 2, 1, 0, 0]);
        self._create_ma3_voice("midi.piano2", "BritePno", vec![3, 0, 0, 15, 2, 2, 2, 5, 39, 1, 2, 1, 0, 0, 0, 15, 2, 2, 3, 15, 28, 0, 2, 5, 0, 0, 0, 15, 2, 2, 2, 13, 25, 1, 2, 1, 0, 0, 0, 15, 2, 1, 5, 4, 10, 1, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.piano3", "E.GrandP", vec![5, 6, 0, 13, 2, 2, 4, 6, 20, 1, 1, 4, 0, 0, 0, 13, 1, 1, 6, 7, 8, 1, 2, 1, 0, 0, 0, 13, 3, 3, 4, 14, 11, 1, 1, 2, 0, 0, 0, 13, 1, 1, 5, 15, 8, 1, 2, 2, 0, 0]);
        self._create_ma3_voice("midi.piano4", "HnkyTonk", vec![5, 6, 0, 15, 1, 2, 5, 14, 26, 1, 0, 1, 3, 2, 0, 13, 3, 2, 10, 3, 2, 1, 2, 2, 7, 2, 0, 12, 1, 2, 5, 3, 23, 0, 0, 1, 7, 0, 0, 13, 3, 3, 10, 3, 2, 1, 2, 2, 3, 2]);
        self._create_ma3_voice("midi.piano5", "E.Piano1", vec![3, 1, 0, 11, 3, 2, 10, 3, 27, 1, 1, 3, 0, 1, 0, 11, 2, 2, 9, 4, 27, 0, 3, 3, 0, 0, 0, 10, 4, 1, 4, 1, 19, 1, 1, 2, 0, 0, 0, 10, 1, 1, 7, 8, 5, 1, 0, 1, 0, 1]);
        self._create_ma3_voice("midi.piano6", "E.Piano2", vec![5, 5, 18, 15, 4, 5, 12, 11, 35, 1, 0, 7, 0, 2, 0, 15, 2, 1, 8, 15, 4, 0, 2, 1, 0, 2, 0, 15, 0, 1, 11, 1, 18, 1, 1, 1, 0, 2, 0, 15, 2, 1, 7, 15, 4, 1, 0, 1, 0, 2]);
        self._create_ma3_voice("midi.piano7", "Harpsi.", vec![6, 4, 4, 14, 2, 2, 5, 0, 0, 1, 1, 1, 0, 0, 3, 15, 2, 0, 5, 3, 20, 1, 3, 6, 0, 0, 4, 15, 3, 0, 1, 6, 28, 1, 2, 7, 0, 0, 5, 14, 2, 2, 7, 15, 4, 1, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.piano8", "Clavi.", vec![3, 5, 5, 15, 1, 1, 6, 15, 24, 0, 0, 1, 0, 2, 0, 15, 1, 1, 5, 0, 29, 0, 0, 1, 0, 2, 4, 15, 3, 3, 7, 2, 27, 1, 1, 7, 0, 2, 0, 15, 2, 2, 9, 2, 8, 1, 0, 3, 0, 2]);
        self._create_ma3_voice("midi.chrom1", "Celesta", vec![5, 2, 2, 14, 6, 6, 5, 15, 21, 1, 2, 9, 0, 0, 0, 13, 4, 4, 4, 14, 6, 0, 0, 1, 0, 0, 5, 14, 6, 6, 6, 12, 22, 1, 3, 11, 0, 0, 0, 14, 4, 4, 4, 14, 6, 0, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.chrom2", "Glocken", vec![7, 0, 0, 15, 9, 3, 4, 4, 9, 1, 0, 7, 0, 2, 0, 15, 11, 2, 3, 11, 15, 1, 0, 4, 0, 2, 0, 15, 3, 2, 4, 4, 18, 1, 1, 2, 0, 2, 0, 15, 4, 3, 4, 14, 4, 0, 0, 1, 0, 2]);
        self._create_ma3_voice("midi.chrom3", "MusicBox", vec![5, 0, 1, 5, 5, 2, 2, 0, 32, 1, 2, 2, 0, 0, 0, 15, 4, 3, 2, 0, 1, 1, 1, 1, 3, 0, 1, 10, 5, 2, 2, 0, 28, 1, 0, 9, 0, 0, 0, 15, 2, 1, 1, 0, 6, 1, 0, 1, 7, 0]);
        self._create_ma3_voice("midi.chrom4", "Vibes", vec![5, 0, 0, 12, 4, 2, 4, 2, 23, 0, 0, 7, 0, 2, 0, 13, 9, 2, 5, 6, 7, 1, 2, 4, 0, 2, 0, 12, 4, 2, 3, 2, 30, 1, 0, 8, 0, 2, 0, 13, 2, 3, 4, 15, 7, 0, 0, 1, 0, 1]);
        self._create_ma3_voice("midi.chrom5", "Marimba", vec![5, 7, 0, 10, 7, 4, 4, 15, 40, 1, 1, 12, 0, 0, 0, 11, 4, 4, 5, 15, 5, 0, 0, 1, 0, 0, 0, 11, 7, 6, 4, 15, 33, 1, 0, 6, 0, 0, 0, 13, 4, 5, 5, 15, 5, 1, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.chrom6", "Xylophon", vec![5, 2, 0, 15, 9, 6, 6, 13, 24, 0, 2, 5, 0, 0, 0, 15, 7, 5, 7, 13, 3, 0, 0, 1, 0, 0, 0, 15, 6, 6, 6, 10, 29, 1, 2, 5, 0, 0, 0, 15, 6, 6, 7, 14, 0, 0, 2, 1, 0, 0]);
        self._create_ma3_voice("midi.chrom7", "TubulBel", vec![5, 0, 16, 15, 4, 3, 3, 5, 16, 0, 1, 10, 0, 1, 0, 15, 3, 2, 3, 2, 5, 0, 0, 1, 0, 0, 8, 15, 4, 3, 3, 5, 16, 0, 1, 7, 3, 1, 0, 15, 3, 2, 3, 2, 5, 0, 2, 2, 7, 0]);
        self._create_ma3_voice("midi.chrom8", "Dulcimer", vec![6, 3, 1, 14, 10, 4, 4, 12, 6, 0, 2, 2, 0, 0, 1, 11, 3, 3, 3, 5, 20, 0, 1, 3, 0, 0, 0, 13, 3, 3, 3, 0, 10, 0, 0, 1, 0, 0, 0, 12, 4, 4, 4, 6, 6, 0, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.organ1", "DrawOrgn", vec![2, 0, 0, 15, 4, 0, 12, 0, 0, 0, 2, 0, 0, 1, 4, 15, 5, 0, 12, 0, 0, 0, 1, 1, 0, 0, 9, 13, 5, 0, 12, 1, 7, 0, 2, 3, 1, 1, 4, 15, 1, 0, 12, 0, 7, 0, 2, 2, 0, 1]);
        self._create_ma3_voice("midi.organ2", "PercOrgn", vec![7, 4, 0, 14, 5, 0, 10, 1, 3, 0, 2, 0, 2, 2, 0, 13, 8, 0, 0, 5, 29, 0, 0, 2, 0, 0, 0, 14, 5, 0, 10, 1, 1, 0, 2, 1, 3, 0, 0, 14, 6, 0, 10, 0, 1, 0, 2, 2, 7, 0]);
        self._create_ma3_voice("midi.organ3", "RockOrgn", vec![7, 4, 0, 15, 15, 0, 13, 0, 9, 0, 1, 1, 3, 3, 21, 11, 15, 0, 10, 1, 5, 0, 1, 1, 0, 2, 0, 15, 15, 0, 14, 0, 9, 0, 0, 2, 6, 0, 17, 15, 15, 0, 14, 0, 9, 0, 0, 0, 7, 1]);
        self._create_ma3_voice("midi.organ4", "ChrchOrg", vec![7, 0, 0, 9, 15, 0, 5, 0, 19, 0, 2, 3, 0, 0, 0, 11, 7, 0, 2, 2, 29, 0, 0, 7, 0, 0, 0, 8, 15, 0, 5, 0, 4, 0, 2, 1, 0, 0, 5, 8, 7, 0, 5, 0, 4, 0, 2, 0, 0, 0]);
        self._create_ma3_voice("midi.organ5", "ReedOrgn", vec![5, 3, 16, 7, 8, 0, 5, 1, 24, 1, 2, 2, 0, 0, 0, 5, 15, 0, 6, 0, 0, 1, 2, 1, 0, 0, 5, 6, 12, 0, 5, 3, 10, 1, 1, 1, 0, 0, 0, 5, 15, 0, 7, 0, 0, 1, 1, 2, 0, 0]);
        self._create_ma3_voice("midi.organ6", "Acordion", vec![5, 2, 17, 8, 2, 0, 0, 1, 21, 0, 0, 3, 6, 0, 0, 7, 2, 0, 10, 2, 2, 0, 1, 1, 7, 0, 17, 6, 15, 0, 0, 1, 18, 0, 0, 1, 2, 0, 0, 7, 15, 0, 10, 0, 7, 0, 2, 2, 3, 0]);
        self._create_ma3_voice("midi.organ7", "Harmnica", vec![4, 0, 0, 15, 15, 0, 9, 0, 44, 0, 3, 14, 0, 0, 0, 15, 15, 0, 8, 0, 41, 0, 0, 10, 0, 0, 0, 15, 15, 0, 8, 0, 36, 0, 0, 1, 0, 0, 0, 6, 15, 0, 8, 0, 3, 0, 2, 2, 0, 0]);
        self._create_ma3_voice("midi.organ8", "TangoAcd", vec![5, 4, 12, 7, 12, 0, 0, 0, 15, 0, 0, 2, 1, 1, 0, 7, 2, 0, 10, 0, 10, 0, 2, 2, 1, 0, 5, 7, 15, 0, 0, 0, 20, 0, 0, 1, 0, 0, 16, 7, 15, 0, 10, 0, 10, 0, 0, 1, 0, 1]);
        self._create_ma3_voice("midi.guitar1", "NylonGtr", vec![5, 6, 0, 14, 1, 1, 4, 8, 21, 1, 1, 1, 0, 0, 0, 15, 3, 3, 7, 15, 0, 0, 0, 1, 0, 0, 1, 11, 5, 5, 5, 4, 14, 1, 0, 3, 0, 0, 0, 13, 4, 4, 9, 15, 13, 0, 2, 1, 0, 0]);
        self._create_ma3_voice("midi.guitar2", "SteelGtr", vec![4, 4, 5, 15, 7, 1, 4, 2, 26, 0, 2, 9, 0, 2, 0, 15, 3, 1, 8, 5, 45, 1, 2, 13, 0, 2, 0, 15, 2, 1, 4, 1, 23, 1, 2, 1, 0, 2, 0, 13, 3, 2, 8, 15, 4, 0, 0, 1, 0, 1]);
        self._create_ma3_voice("midi.guitar3", "Jazz Gtr", vec![3, 0, 0, 15, 7, 1, 7, 3, 17, 1, 1, 1, 0, 2, 0, 15, 5, 1, 4, 2, 18, 0, 3, 5, 0, 2, 0, 15, 2, 0, 7, 15, 31, 0, 1, 3, 0, 2, 0, 12, 2, 0, 8, 15, 4, 0, 0, 1, 0, 2]);
        self._create_ma3_voice("midi.guitar4", "CleanGtr", vec![5, 0, 1, 15, 10, 2, 2, 1, 15, 0, 1, 1, 0, 0, 0, 15, 2, 2, 9, 15, 3, 0, 2, 1, 0, 0, 4, 15, 2, 2, 3, 6, 16, 1, 1, 3, 0, 0, 0, 14, 4, 4, 8, 6, 3, 0, 2, 1, 0, 0]);
        self._create_ma3_voice("midi.guitar5", "Mute.Gtr", vec![5, 7, 0, 13, 8, 6, 7, 7, 17, 1, 0, 0, 0, 0, 0, 14, 9, 3, 9, 7, 0, 0, 0, 0, 0, 0, 0, 14, 3, 3, 8, 9, 4, 0, 3, 1, 0, 0, 0, 11, 4, 3, 10, 3, 0, 1, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.guitar6", "Ovrdrive", vec![4, 2, 12, 15, 8, 0, 2, 15, 19, 1, 0, 0, 7, 2, 0, 12, 1, 0, 1, 1, 15, 0, 1, 2, 3, 2, 0, 11, 2, 0, 10, 1, 15, 0, 0, 1, 0, 2, 0, 11, 1, 1, 10, 1, 10, 0, 0, 2, 0, 2]);
        self._create_ma3_voice("midi.guitar7", "Dist.Gtr", vec![4, 4, 3, 11, 12, 0, 2, 0, 8, 0, 1, 2, 0, 0, 0, 12, 5, 0, 10, 1, 29, 0, 0, 1, 0, 0, 8, 12, 5, 0, 10, 1, 23, 0, 0, 2, 0, 0, 6, 12, 1, 0, 10, 5, 15, 0, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.guitar8", "GtrHarmo", vec![5, 5, 8, 15, 2, 8, 7, 0, 22, 0, 1, 0, 0, 0, 8, 13, 3, 3, 9, 15, 6, 0, 0, 2, 0, 0, 0, 11, 2, 8, 7, 0, 17, 0, 0, 0, 0, 0, 6, 10, 7, 7, 7, 15, 13, 0, 0, 2, 0, 0]);
        self._create_ma3_voice("midi.bass1", "Aco.Bass", vec![5, 3, 0, 11, 3, 3, 8, 10, 14, 1, 1, 1, 0, 0, 0, 12, 3, 3, 8, 11, 0, 0, 0, 1, 0, 0, 0, 9, 3, 3, 1, 1, 7, 0, 3, 1, 0, 0, 0, 12, 3, 3, 8, 10, 5, 0, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.bass2", "FngrBass", vec![3, 6, 0, 10, 2, 1, 3, 1, 28, 1, 2, 1, 0, 2, 0, 9, 4, 3, 6, 4, 58, 1, 0, 12, 0, 2, 0, 11, 3, 2, 3, 2, 22, 1, 2, 1, 0, 2, 0, 11, 1, 1, 8, 2, 0, 1, 0, 2, 0, 2]);
        self._create_ma3_voice("midi.bass3", "PickBass", vec![3, 5, 0, 15, 7, 2, 3, 1, 19, 1, 2, 1, 0, 2, 0, 12, 11, 4, 6, 7, 21, 1, 0, 7, 0, 2, 0, 15, 9, 2, 6, 2, 23, 1, 2, 2, 0, 2, 0, 11, 2, 6, 8, 6, 0, 1, 0, 1, 0, 2]);
        self._create_ma3_voice("midi.bass4", "Fretless", vec![3, 4, 0, 12, 3, 2, 3, 1, 29, 1, 2, 1, 2, 2, 0, 10, 3, 3, 6, 3, 25, 1, 2, 1, 1, 2, 0, 9, 3, 2, 6, 1, 25, 1, 2, 1, 0, 2, 0, 11, 1, 2, 8, 2, 0, 1, 0, 2, 0, 2]);
        self._create_ma3_voice("midi.bass5", "SlapBas1", vec![3, 3, 0, 15, 7, 2, 3, 2, 14, 1, 2, 1, 0, 2, 0, 15, 6, 6, 6, 4, 21, 1, 0, 9, 0, 2, 0, 12, 9, 2, 6, 2, 24, 1, 2, 1, 0, 2, 0, 15, 2, 15, 8, 15, 3, 1, 0, 1, 0, 2]);
        self._create_ma3_voice("midi.bass6", "SlapBas2", vec![3, 2, 0, 15, 7, 2, 3, 1, 14, 1, 2, 1, 0, 2, 0, 11, 5, 6, 7, 2, 18, 0, 0, 13, 0, 2, 0, 9, 9, 2, 6, 2, 30, 1, 2, 1, 0, 2, 0, 15, 2, 6, 8, 6, 6, 1, 0, 1, 0, 2]);
        self._create_ma3_voice("midi.bass7", "SynBass1", vec![3, 5, 0, 14, 6, 2, 8, 5, 14, 0, 0, 1, 0, 0, 8, 14, 4, 1, 8, 6, 39, 0, 1, 2, 0, 0, 0, 14, 2, 1, 8, 6, 35, 0, 0, 1, 0, 0, 0, 14, 2, 2, 8, 9, 0, 0, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.bass8", "SynBass2", vec![5, 6, 0, 15, 5, 7, 8, 6, 20, 0, 0, 2, 0, 2, 0, 15, 1, 7, 8, 12, 0, 0, 1, 2, 0, 2, 0, 15, 3, 7, 7, 6, 20, 0, 0, 1, 0, 2, 0, 15, 2, 7, 8, 12, 0, 0, 1, 1, 0, 2]);
        self._create_ma3_voice("midi.strings1", "Violin", vec![5, 2, 12, 6, 0, 0, 3, 0, 18, 1, 2, 1, 0, 0, 0, 6, 4, 0, 7, 2, 3, 0, 0, 1, 5, 0, 6, 14, 5, 7, 10, 0, 6, 1, 1, 4, 0, 0, 0, 6, 7, 7, 7, 15, 3, 0, 2, 1, 1, 0]);
        self._create_ma3_voice("midi.strings2", "Viola", vec![5, 2, 1, 6, 0, 0, 3, 0, 9, 1, 2, 1, 0, 0, 0, 6, 6, 0, 7, 1, 3, 0, 0, 1, 0, 0, 6, 14, 6, 7, 7, 0, 8, 1, 1, 1, 0, 0, 0, 6, 7, 7, 7, 15, 3, 0, 2, 1, 0, 0]);
        self._create_ma3_voice("midi.strings3", "Cello", vec![3, 4, 1, 15, 6, 0, 6, 0, 16, 0, 0, 1, 0, 0, 0, 15, 5, 15, 14, 15, 20, 0, 0, 5, 0, 2, 0, 15, 5, 0, 7, 2, 45, 0, 0, 1, 0, 0, 0, 6, 3, 0, 7, 1, 1, 0, 2, 3, 0, 0]);
        self._create_ma3_voice("midi.strings4", "ContraBs", vec![3, 6, 0, 15, 6, 0, 2, 0, 25, 0, 0, 1, 0, 0, 17, 15, 6, 15, 14, 15, 21, 0, 0, 5, 0, 2, 0, 15, 6, 0, 4, 3, 27, 0, 0, 3, 0, 0, 0, 6, 3, 0, 7, 1, 0, 0, 2, 2, 0, 0]);
        self._create_ma3_voice("midi.strings5", "Trem.Str", vec![5, 3, 20, 7, 2, 0, 3, 1, 22, 1, 2, 1, 2, 0, 0, 6, 3, 0, 6, 1, 2, 0, 0, 2, 0, 1, 12, 7, 3, 0, 4, 0, 22, 1, 1, 1, 4, 0, 0, 6, 3, 0, 6, 1, 2, 0, 0, 1, 0, 2]);
        self._create_ma3_voice("midi.strings6", "Pizz.Str", vec![5, 7, 0, 15, 11, 5, 11, 9, 20, 0, 0, 1, 0, 0, 0, 14, 7, 8, 7, 2, 0, 0, 0, 1, 0, 0, 8, 15, 7, 6, 5, 15, 17, 0, 1, 1, 0, 0, 0, 12, 6, 5, 6, 15, 0, 0, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.strings7", "Harp", vec![3, 6, 0, 15, 8, 5, 8, 4, 41, 0, 0, 2, 0, 0, 0, 11, 8, 7, 9, 4, 33, 0, 0, 5, 0, 0, 0, 11, 7, 3, 2, 4, 33, 0, 2, 1, 0, 0, 0, 15, 4, 2, 2, 1, 4, 1, 0, 1, 0, 2]);
        self._create_ma3_voice("midi.strings8", "Timpani", vec![3, 3, 0, 15, 8, 4, 3, 3, 4, 1, 1, 1, 0, 2, 0, 15, 2, 2, 2, 15, 33, 1, 0, 0, 7, 2, 0, 15, 7, 3, 3, 0, 28, 1, 2, 1, 0, 2, 8, 15, 4, 3, 3, 15, 0, 1, 0, 0, 0, 2]);
        self._create_ma3_voice("midi.ensemble1", "std::strings1", vec![7, 2, 7, 5, 10, 0, 6, 0, 11, 0, 2, 1, 1, 1, 4, 12, 6, 0, 6, 1, 24, 0, 0, 2, 0, 0, 0, 6, 6, 0, 6, 1, 7, 0, 2, 1, 5, 0, 12, 6, 5, 0, 6, 1, 5, 0, 0, 2, 3, 1]);
        self._create_ma3_voice("midi.ensemble2", "std::strings2", vec![7, 3, 9, 6, 10, 0, 5, 0, 0, 0, 0, 1, 3, 0, 2, 12, 6, 0, 5, 1, 23, 0, 0, 1, 0, 1, 1, 5, 6, 0, 6, 0, 0, 0, 2, 1, 5, 0, 27, 5, 5, 0, 6, 1, 7, 0, 0, 1, 7, 0]);
        self._create_ma3_voice("midi.ensemble3", "Syn.Str1", vec![5, 0, 0, 9, 8, 0, 2, 1, 27, 0, 0, 1, 6, 0, 0, 7, 15, 0, 5, 0, 7, 0, 0, 1, 3, 0, 4, 9, 11, 0, 2, 0, 20, 1, 0, 1, 3, 0, 0, 6, 15, 0, 4, 0, 0, 0, 1, 1, 7, 0]);
        self._create_ma3_voice("midi.ensemble4", "Syn.Str2", vec![5, 5, 9, 9, 8, 0, 2, 1, 19, 0, 0, 1, 0, 0, 0, 6, 6, 0, 5, 0, 6, 0, 0, 1, 0, 0, 27, 8, 8, 0, 2, 0, 10, 0, 0, 1, 0, 0, 0, 5, 7, 0, 4, 3, 0, 0, 1, 1, 0, 0]);
        self._create_ma3_voice("midi.ensemble5", "ChoirAah", vec![5, 5, 7, 12, 0, 0, 0, 15, 19, 0, 0, 6, 0, 0, 0, 6, 3, 0, 5, 6, 23, 0, 2, 4, 0, 0, 8, 7, 15, 0, 3, 0, 30, 0, 1, 1, 0, 0, 0, 5, 15, 0, 5, 0, 0, 0, 0, 2, 0, 0]);
        self._create_ma3_voice("midi.ensemble6", "VoiceOoh", vec![5, 7, 7, 12, 0, 0, 0, 15, 20, 1, 0, 5, 0, 1, 0, 7, 3, 0, 5, 6, 20, 0, 2, 4, 0, 0, 10, 7, 7, 0, 4, 3, 26, 0, 2, 1, 0, 0, 0, 9, 1, 0, 5, 0, 0, 0, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.ensemble7", "SynVoice", vec![5, 0, 0, 10, 0, 0, 4, 15, 22, 0, 0, 1, 0, 0, 0, 7, 15, 0, 5, 1, 8, 0, 2, 1, 0, 0, 9, 9, 15, 0, 4, 0, 26, 0, 1, 1, 0, 0, 0, 7, 15, 0, 5, 0, 0, 0, 2, 3, 0, 0]);
        self._create_ma3_voice("midi.ensemble8", "Orch.Hit", vec![7, 5, 8, 15, 4, 4, 6, 6, 0, 1, 0, 4, 0, 1, 6, 12, 5, 3, 3, 1, 0, 0, 0, 1, 0, 0, 6, 12, 7, 6, 6, 0, 0, 0, 0, 1, 0, 0, 6, 11, 7, 7, 6, 0, 0, 0, 0, 0, 0, 0]);
        self._create_ma3_voice("midi.brass1", "Trumpet", vec![3, 6, 1, 8, 8, 0, 5, 1, 20, 0, 0, 1, 0, 0, 0, 10, 8, 0, 5, 4, 23, 0, 2, 3, 0, 0, 0, 7, 7, 0, 6, 1, 26, 0, 2, 1, 0, 0, 0, 9, 15, 0, 8, 0, 6, 0, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.brass2", "Trombone", vec![3, 7, 8, 7, 6, 0, 7, 1, 28, 0, 0, 1, 0, 0, 0, 9, 8, 0, 5, 4, 15, 0, 2, 1, 0, 0, 1, 6, 7, 0, 7, 1, 26, 0, 2, 1, 0, 0, 0, 8, 15, 0, 8, 0, 5, 0, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.brass3", "Tuba", vec![3, 7, 8, 6, 5, 0, 6, 2, 34, 0, 0, 1, 0, 0, 1, 12, 8, 0, 11, 4, 24, 0, 2, 1, 0, 0, 1, 9, 7, 0, 9, 3, 17, 0, 2, 1, 0, 0, 0, 7, 15, 0, 8, 0, 0, 0, 0, 2, 0, 0]);
        self._create_ma3_voice("midi.brass4", "Mute.Trp", vec![5, 0, 10, 7, 0, 0, 7, 5, 26, 0, 0, 3, 0, 0, 2, 9, 13, 0, 9, 4, 0, 0, 0, 0, 0, 0, 17, 7, 9, 0, 6, 1, 19, 0, 0, 5, 0, 0, 2, 8, 7, 0, 9, 2, 0, 0, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.brass5", "Fr.Horn", vec![5, 0, 8, 7, 9, 0, 0, 0, 16, 0, 3, 1, 2, 0, 0, 9, 14, 0, 7, 0, 1, 0, 2, 1, 7, 0, 0, 6, 9, 0, 2, 1, 22, 0, 2, 1, 6, 1, 8, 10, 14, 0, 7, 0, 1, 0, 2, 1, 3, 0]);
        self._create_ma3_voice("midi.brass6", "BrasSect", vec![5, 6, 0, 8, 6, 0, 2, 1, 22, 0, 0, 1, 7, 2, 0, 9, 15, 0, 8, 0, 8, 0, 0, 1, 7, 2, 12, 7, 7, 0, 5, 1, 22, 0, 0, 1, 0, 2, 0, 9, 8, 0, 8, 0, 7, 0, 0, 1, 0, 2]);
        self._create_ma3_voice("midi.brass7", "SynBras1", vec![5, 6, 0, 7, 6, 0, 8, 2, 16, 0, 0, 1, 7, 2, 0, 9, 15, 0, 10, 0, 10, 0, 0, 1, 7, 2, 0, 7, 6, 0, 8, 2, 16, 0, 0, 1, 0, 2, 0, 9, 8, 0, 10, 0, 10, 0, 0, 1, 0, 2]);
        self._create_ma3_voice("midi.brass8", "SynBras2", vec![3, 6, 0, 6, 3, 0, 4, 1, 28, 0, 0, 1, 0, 0, 1, 9, 7, 0, 5, 7, 39, 0, 0, 6, 0, 0, 8, 7, 5, 3, 3, 11, 35, 0, 0, 1, 0, 0, 0, 15, 15, 0, 7, 0, 4, 0, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.reed1", "SprnoSax", vec![3, 0, 0, 15, 9, 6, 6, 3, 29, 1, 0, 3, 0, 0, 0, 8, 2, 0, 6, 0, 26, 0, 0, 1, 0, 0, 1, 8, 5, 0, 0, 0, 12, 0, 1, 1, 0, 0, 0, 8, 6, 0, 8, 1, 3, 0, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.reed2", "Alto Sax", vec![5, 4, 1, 9, 3, 0, 0, 0, 10, 1, 2, 1, 0, 2, 0, 8, 2, 0, 9, 0, 9, 0, 0, 1, 0, 2, 9, 9, 3, 0, 0, 0, 13, 1, 2, 1, 0, 2, 1, 9, 2, 0, 9, 0, 21, 0, 0, 1, 0, 2]);
        self._create_ma3_voice("midi.reed3", "TenorSax", vec![5, 3, 1, 7, 3, 0, 0, 0, 5, 1, 2, 1, 0, 2, 8, 7, 2, 0, 9, 0, 15, 0, 0, 1, 0, 2, 9, 7, 3, 0, 0, 0, 8, 1, 2, 1, 0, 2, 0, 7, 2, 0, 9, 0, 13, 0, 2, 1, 0, 2]);
        self._create_ma3_voice("midi.reed4", "Bari.Sax", vec![5, 6, 0, 7, 3, 0, 5, 0, 18, 1, 2, 1, 0, 0, 0, 7, 2, 0, 8, 2, 6, 1, 0, 2, 0, 0, 2, 7, 5, 0, 1, 0, 14, 1, 1, 2, 0, 0, 0, 7, 4, 0, 8, 1, 5, 1, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.reed5", "Oboe", vec![5, 0, 5, 10, 0, 0, 4, 0, 30, 0, 2, 1, 0, 0, 0, 9, 1, 1, 9, 0, 9, 0, 0, 3, 0, 0, 0, 11, 0, 0, 4, 2, 24, 0, 0, 1, 0, 1, 0, 10, 0, 0, 10, 0, 14, 0, 0, 2, 0, 0]);
        self._create_ma3_voice("midi.reed6", "Eng.Horn", vec![5, 0, 5, 10, 0, 0, 4, 0, 34, 0, 2, 1, 0, 0, 0, 9, 1, 1, 9, 1, 11, 0, 0, 3, 0, 0, 0, 11, 0, 0, 4, 2, 24, 0, 0, 1, 0, 1, 0, 10, 0, 1, 10, 1, 11, 0, 0, 2, 0, 0]);
        self._create_ma3_voice("midi.reed7", "Bassoon", vec![5, 0, 1, 12, 7, 0, 0, 1, 24, 1, 2, 1, 0, 0, 9, 7, 1, 0, 8, 1, 0, 1, 2, 3, 0, 0, 1, 12, 7, 0, 0, 1, 24, 1, 2, 1, 0, 0, 9, 7, 1, 0, 8, 1, 3, 1, 0, 3, 1, 0]);
        self._create_ma3_voice("midi.reed8", "Clarinet", vec![5, 7, 0, 7, 2, 0, 1, 1, 37, 1, 2, 2, 0, 0, 0, 8, 2, 0, 8, 1, 3, 0, 0, 1, 0, 0, 0, 5, 2, 0, 1, 1, 26, 1, 1, 4, 0, 0, 0, 7, 2, 0, 8, 1, 3, 0, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.pipe1", "Piccolo", vec![5, 7, 2, 10, 12, 0, 7, 1, 12, 0, 0, 5, 0, 0, 0, 9, 7, 0, 8, 15, 39, 0, 0, 1, 0, 0, 1, 8, 5, 0, 7, 1, 30, 0, 0, 1, 0, 0, 0, 8, 5, 0, 10, 0, 6, 0, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.pipe2", "Flute", vec![5, 7, 0, 13, 10, 0, 1, 1, 7, 0, 0, 3, 0, 0, 0, 7, 8, 0, 11, 3, 37, 0, 0, 3, 0, 0, 0, 14, 8, 0, 9, 0, 39, 0, 0, 1, 0, 1, 16, 6, 5, 0, 10, 0, 1, 0, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.pipe3", "Recorder", vec![5, 7, 3, 9, 6, 7, 10, 0, 58, 0, 0, 2, 0, 1, 8, 8, 5, 0, 10, 0, 4, 0, 0, 1, 0, 0, 24, 10, 9, 6, 6, 9, 15, 0, 0, 7, 0, 0, 0, 8, 5, 0, 10, 0, 36, 0, 0, 1, 0, 1]);
        self._create_ma3_voice("midi.pipe4", "PanFlute", vec![5, 7, 0, 10, 0, 0, 6, 0, 0, 0, 1, 13, 0, 1, 3, 11, 10, 1, 10, 0, 35, 0, 0, 10, 0, 1, 0, 8, 15, 0, 4, 0, 44, 0, 0, 2, 0, 0, 0, 8, 0, 0, 9, 0, 5, 0, 0, 1, 0, 1]);
        self._create_ma3_voice("midi.pipe5", "Bottle", vec![5, 7, 10, 12, 12, 0, 7, 1, 12, 1, 0, 5, 0, 0, 0, 7, 7, 0, 9, 6, 27, 0, 0, 1, 0, 0, 8, 7, 8, 0, 8, 3, 11, 0, 3, 2, 0, 1, 0, 7, 5, 0, 8, 0, 1, 0, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.pipe6", "Shakhchi", vec![5, 7, 2, 10, 12, 0, 5, 1, 6, 0, 0, 5, 0, 0, 8, 6, 7, 0, 9, 5, 23, 0, 0, 1, 0, 1, 18, 10, 8, 0, 3, 3, 2, 0, 1, 0, 0, 2, 0, 6, 5, 0, 9, 0, 1, 0, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.pipe7", "Whistle", vec![2, 0, 0, 6, 10, 0, 6, 0, 5, 0, 0, 1, 0, 0, 0, 8, 8, 0, 7, 0, 5, 0, 0, 1, 0, 2, 17, 6, 10, 9, 7, 0, 44, 0, 0, 1, 0, 2, 8, 6, 8, 0, 7, 0, 5, 0, 0, 1, 7, 2]);
        self._create_ma3_voice("midi.pipe8", "Ocarina", vec![5, 7, 3, 8, 6, 7, 8, 0, 60, 0, 1, 2, 0, 0, 0, 8, 5, 0, 9, 0, 0, 0, 0, 1, 0, 0, 24, 10, 9, 6, 6, 9, 15, 0, 0, 1, 0, 0, 8, 7, 5, 0, 10, 0, 26, 0, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.lead1", "SquareLd", vec![5, 0, 8, 15, 15, 0, 7, 4, 46, 0, 3, 1, 0, 0, 8, 10, 15, 0, 10, 0, 3, 0, 2, 1, 0, 0, 8, 15, 15, 0, 2, 3, 38, 0, 3, 2, 0, 0, 8, 10, 15, 0, 10, 0, 3, 0, 2, 1, 0, 0]);
        self._create_ma3_voice("midi.lead2", "Saw.Lead", vec![5, 7, 0, 15, 0, 0, 7, 0, 26, 0, 0, 1, 7, 2, 8, 13, 15, 0, 7, 0, 10, 0, 0, 1, 7, 2, 20, 15, 15, 0, 3, 0, 20, 0, 1, 1, 0, 2, 8, 14, 15, 0, 8, 0, 10, 0, 0, 1, 0, 2]);
        self._create_ma3_voice("midi.lead3", "CaliopLd", vec![5, 7, 8, 12, 4, 0, 7, 8, 0, 0, 0, 4, 0, 1, 8, 12, 6, 0, 6, 7, 20, 0, 1, 4, 0, 2, 0, 8, 6, 0, 5, 5, 3, 0, 1, 2, 0, 0, 16, 6, 4, 0, 8, 1, 2, 0, 0, 1, 0, 1]);
        self._create_ma3_voice("midi.lead4", "ChiffLd", vec![5, 0, 8, 7, 7, 0, 2, 6, 4, 0, 1, 1, 4, 0, 8, 15, 6, 0, 8, 0, 11, 0, 0, 1, 0, 0, 8, 7, 7, 0, 2, 6, 3, 0, 1, 1, 1, 0, 8, 15, 6, 0, 8, 0, 11, 0, 0, 1, 2, 0]);
        self._create_ma3_voice("midi.lead5", "CharanLd", vec![5, 0, 1, 9, 2, 0, 6, 2, 8, 0, 2, 1, 4, 2, 12, 9, 1, 0, 8, 2, 14, 0, 0, 2, 0, 0, 1, 9, 2, 0, 6, 2, 10, 0, 2, 1, 1, 2, 12, 9, 1, 0, 8, 2, 14, 0, 0, 2, 1, 0]);
        self._create_ma3_voice("midi.lead6", "Voice Ld", vec![5, 0, 6, 4, 0, 0, 0, 15, 13, 0, 0, 7, 0, 0, 0, 7, 3, 0, 8, 6, 19, 0, 0, 2, 0, 0, 8, 7, 15, 0, 9, 0, 28, 0, 1, 1, 0, 1, 0, 7, 15, 0, 8, 0, 2, 0, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.lead7", "Fifth Ld", vec![7, 0, 24, 12, 1, 1, 8, 1, 21, 0, 0, 1, 0, 2, 0, 12, 1, 1, 6, 2, 15, 0, 0, 1, 1, 2, 0, 12, 0, 0, 8, 0, 26, 0, 0, 2, 4, 2, 16, 12, 1, 1, 8, 1, 6, 0, 0, 3, 1, 2]);
        self._create_ma3_voice("midi.lead8", "Bass &Ld", vec![5, 0, 1, 11, 2, 0, 3, 0, 22, 0, 0, 1, 0, 0, 16, 10, 2, 0, 9, 0, 17, 0, 0, 1, 0, 0, 0, 12, 3, 4, 4, 5, 11, 0, 2, 1, 0, 0, 0, 13, 3, 0, 9, 6, 12, 0, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.pad1", "NewAgePd", vec![5, 5, 1, 15, 15, 3, 3, 0, 38, 1, 0, 7, 0, 2, 0, 15, 7, 4, 4, 0, 11, 0, 2, 5, 0, 2, 1, 6, 1, 0, 1, 0, 24, 0, 2, 1, 7, 0, 0, 8, 1, 0, 5, 1, 0, 0, 2, 1, 0, 0]);
        self._create_ma3_voice("midi.pad2", "Warm Pad", vec![5, 7, 0, 10, 0, 0, 5, 0, 40, 0, 0, 1, 1, 0, 0, 3, 0, 0, 4, 0, 2, 0, 0, 1, 0, 0, 0, 10, 0, 0, 3, 0, 47, 0, 0, 1, 3, 0, 0, 3, 0, 0, 4, 0, 2, 0, 0, 1, 2, 0]);
        self._create_ma3_voice("midi.pad3", "PolySyPd", vec![5, 0, 3, 6, 5, 0, 4, 1, 34, 0, 0, 1, 7, 0, 0, 10, 5, 0, 6, 1, 2, 0, 2, 2, 6, 0, 27, 6, 3, 0, 3, 1, 34, 0, 0, 1, 0, 0, 8, 9, 3, 0, 5, 0, 0, 0, 2, 1, 2, 0]);
        self._create_ma3_voice("midi.pad4", "ChoirPad", vec![5, 2, 8, 10, 0, 0, 0, 15, 33, 0, 0, 1, 7, 2, 0, 4, 3, 0, 7, 3, 22, 0, 2, 8, 0, 0, 0, 7, 3, 0, 3, 0, 33, 0, 1, 1, 0, 0, 0, 6, 15, 0, 5, 0, 0, 0, 0, 2, 3, 0]);
        self._create_ma3_voice("midi.pad5", "BowedPad", vec![5, 4, 0, 2, 1, 0, 3, 4, 42, 0, 2, 7, 0, 0, 0, 6, 2, 0, 5, 3, 0, 0, 0, 1, 0, 0, 0, 2, 1, 0, 3, 4, 42, 0, 2, 7, 1, 0, 0, 6, 2, 0, 5, 3, 0, 0, 0, 1, 2, 0]);
        self._create_ma3_voice("midi.pad6", "MetalPad", vec![5, 6, 10, 15, 2, 0, 3, 0, 23, 1, 2, 1, 0, 0, 0, 5, 6, 0, 4, 0, 3, 0, 2, 1, 5, 0, 0, 15, 6, 0, 3, 3, 7, 1, 1, 1, 0, 0, 0, 5, 7, 0, 4, 0, 4, 0, 1, 1, 1, 0]);
        self._create_ma3_voice("midi.pad7", "Halo Pad", vec![5, 6, 0, 4, 1, 0, 3, 1, 37, 0, 0, 1, 5, 2, 0, 6, 2, 0, 5, 0, 0, 0, 2, 1, 7, 0, 0, 12, 5, 0, 4, 1, 30, 0, 0, 1, 3, 2, 0, 8, 2, 0, 5, 0, 0, 0, 2, 1, 0, 0]);
        self._create_ma3_voice("midi.pad8", "SweepPad", vec![5, 0, 0, 4, 8, 0, 3, 0, 30, 0, 0, 1, 5, 0, 0, 3, 8, 0, 4, 0, 0, 0, 2, 1, 3, 0, 0, 3, 1, 0, 4, 0, 34, 0, 2, 2, 2, 0, 0, 7, 2, 0, 5, 0, 0, 0, 2, 1, 7, 0]);
        self._create_ma3_voice("midi.fx1", "Rain", vec![5, 1, 0, 15, 8, 0, 6, 8, 2, 1, 3, 10, 0, 2, 16, 8, 5, 1, 2, 0, 0, 1, 0, 1, 0, 3, 0, 15, 8, 0, 6, 8, 2, 1, 3, 10, 2, 2, 16, 8, 5, 1, 2, 0, 0, 1, 0, 1, 3, 2]);
        self._create_ma3_voice("midi.fx2", "SoundTrk", vec![5, 3, 0, 6, 3, 0, 3, 3, 18, 0, 2, 3, 0, 0, 0, 4, 2, 0, 4, 0, 9, 0, 1, 3, 0, 0, 16, 6, 3, 0, 3, 3, 16, 0, 0, 1, 0, 0, 0, 4, 1, 0, 4, 1, 3, 0, 2, 1, 0, 0]);
        self._create_ma3_voice("midi.fx3", "Crystal", vec![5, 5, 0, 15, 8, 1, 5, 4, 20, 0, 1, 6, 0, 2, 0, 12, 2, 2, 4, 7, 9, 0, 0, 1, 0, 2, 0, 15, 8, 1, 5, 4, 20, 0, 1, 14, 2, 2, 0, 12, 2, 2, 4, 7, 9, 0, 0, 1, 2, 2]);
        self._create_ma3_voice("midi.fx4", "Atmosphr", vec![5, 4, 20, 6, 3, 2, 4, 15, 21, 0, 1, 1, 0, 0, 0, 15, 3, 0, 4, 3, 0, 0, 0, 1, 0, 0, 20, 12, 3, 0, 4, 15, 16, 0, 1, 2, 3, 0, 0, 9, 6, 5, 4, 3, 12, 0, 0, 2, 7, 0]);
        self._create_ma3_voice("midi.fx5", "Bright", vec![2, 0, 11, 15, 1, 1, 4, 5, 7, 0, 2, 1, 3, 0, 9, 15, 2, 5, 4, 15, 7, 0, 0, 1, 3, 0, 11, 15, 1, 1, 4, 5, 7, 0, 2, 1, 1, 0, 9, 15, 2, 5, 4, 15, 7, 0, 0, 1, 1, 0]);
        self._create_ma3_voice("midi.fx6", "Goblins", vec![5, 4, 13, 1, 1, 0, 1, 2, 18, 0, 2, 3, 7, 2, 0, 2, 1, 0, 4, 1, 9, 0, 2, 3, 0, 0, 0, 2, 1, 0, 2, 0, 20, 0, 0, 1, 0, 2, 0, 3, 1, 0, 3, 0, 3, 0, 2, 1, 4, 2]);
        self._create_ma3_voice("midi.fx7", "Echoes", vec![5, 0, 0, 4, 3, 0, 0, 5, 34, 0, 0, 2, 0, 0, 0, 10, 2, 0, 12, 0, 14, 0, 2, 1, 0, 0, 16, 3, 3, 0, 2, 2, 35, 0, 0, 1, 0, 1, 0, 10, 15, 0, 3, 0, 0, 0, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.fx8", "Sci-Fi", vec![5, 2, 1, 5, 3, 0, 3, 8, 25, 0, 1, 2, 0, 0, 4, 6, 1, 1, 4, 2, 6, 0, 0, 1, 7, 0, 1, 5, 3, 0, 3, 8, 25, 0, 1, 2, 3, 0, 4, 6, 1, 1, 4, 2, 6, 0, 0, 1, 3, 0]);
        self._create_ma3_voice("midi.world1", "Sitar", vec![5, 3, 0, 13, 2, 3, 2, 5, 10, 0, 2, 2, 0, 2, 9, 15, 2, 6, 4, 15, 8, 0, 0, 7, 0, 2, 8, 13, 2, 3, 2, 5, 3, 0, 2, 1, 1, 2, 17, 15, 2, 6, 4, 15, 8, 0, 0, 4, 1, 2]);
        self._create_ma3_voice("midi.world2", "Banjo", vec![5, 0, 4, 13, 3, 2, 1, 1, 10, 1, 0, 1, 0, 0, 0, 13, 3, 3, 5, 14, 0, 1, 2, 3, 0, 0, 1, 15, 7, 4, 2, 1, 15, 0, 0, 6, 0, 0, 1, 15, 7, 8, 8, 14, 0, 0, 2, 1, 0, 0]);
        self._create_ma3_voice("midi.world3", "Shamisen", vec![3, 4, 8, 15, 1, 1, 3, 2, 26, 0, 0, 1, 0, 2, 0, 15, 10, 5, 6, 7, 20, 0, 0, 3, 0, 2, 16, 15, 8, 3, 3, 3, 24, 0, 0, 5, 0, 2, 8, 15, 4, 4, 4, 15, 1, 1, 0, 3, 0, 2]);
        self._create_ma3_voice("midi.world4", "Koto", vec![3, 6, 8, 15, 7, 5, 5, 2, 20, 0, 2, 3, 0, 0, 16, 15, 9, 8, 8, 4, 21, 0, 2, 5, 0, 0, 0, 15, 2, 2, 3, 15, 42, 0, 0, 1, 0, 0, 0, 15, 2, 2, 2, 15, 3, 1, 0, 1, 0, 2]);
        self._create_ma3_voice("midi.world5", "Kalimba", vec![5, 6, 4, 15, 10, 5, 6, 10, 8, 0, 1, 4, 3, 2, 0, 12, 2, 3, 5, 0, 8, 0, 0, 1, 0, 2, 0, 15, 8, 5, 6, 10, 14, 0, 1, 5, 2, 2, 0, 12, 2, 4, 4, 0, 8, 1, 0, 1, 4, 2]);
        self._create_ma3_voice("midi.world6", "Bagpipe", vec![5, 1, 9, 11, 9, 0, 12, 1, 16, 0, 2, 1, 0, 2, 8, 7, 15, 0, 13, 0, 11, 0, 0, 3, 1, 2, 0, 10, 9, 0, 9, 3, 0, 0, 0, 1, 0, 2, 1, 8, 6, 0, 13, 0, 10, 0, 0, 4, 0, 2]);
        self._create_ma3_voice("midi.world7", "Fiddle", vec![5, 2, 1, 8, 9, 0, 3, 1, 7, 1, 2, 1, 0, 0, 0, 6, 6, 0, 7, 2, 0, 0, 0, 1, 0, 2, 4, 12, 6, 7, 7, 0, 9, 1, 1, 2, 0, 0, 0, 9, 3, 7, 8, 14, 25, 0, 2, 1, 0, 0]);
        self._create_ma3_voice("midi.world8", "Shanai", vec![5, 0, 5, 10, 0, 0, 4, 0, 16, 0, 2, 1, 0, 0, 0, 9, 1, 1, 9, 0, 13, 0, 0, 6, 0, 0, 0, 11, 0, 0, 4, 2, 24, 0, 0, 1, 0, 1, 0, 10, 0, 0, 10, 0, 15, 0, 0, 2, 0, 0]);
        self._create_ma3_voice("midi.percus1", "TnklBell", vec![5, 3, 0, 15, 6, 3, 4, 5, 16, 0, 1, 14, 0, 2, 0, 12, 6, 7, 6, 14, 11, 0, 2, 2, 0, 2, 1, 12, 6, 2, 2, 5, 30, 0, 0, 7, 7, 2, 0, 15, 5, 4, 5, 13, 1, 0, 0, 6, 0, 2]);
        self._create_ma3_voice("midi.percus2", "Agogo", vec![5, 1, 0, 14, 10, 4, 4, 2, 23, 0, 0, 7, 0, 2, 0, 15, 7, 6, 6, 1, 8, 0, 0, 5, 0, 2, 0, 14, 9, 6, 4, 2, 33, 0, 0, 10, 7, 2, 0, 15, 7, 6, 6, 7, 4, 0, 0, 2, 0, 2]);
        self._create_ma3_voice("midi.percus3", "SteelDrm", vec![7, 0, 16, 4, 4, 4, 5, 2, 0, 1, 0, 2, 0, 2, 0, 6, 6, 3, 4, 0, 22, 0, 0, 2, 0, 2, 0, 14, 4, 4, 4, 2, 0, 1, 0, 2, 0, 2, 2, 15, 4, 4, 4, 2, 0, 1, 0, 0, 3, 0]);
        self._create_ma3_voice("midi.percus4", "WoodBlok", vec![5, 5, 0, 15, 10, 9, 9, 2, 33, 1, 2, 5, 1, 2, 0, 15, 10, 7, 7, 2, 0, 1, 0, 2, 3, 2, 0, 15, 10, 10, 8, 2, 28, 0, 0, 10, 3, 2, 0, 15, 10, 7, 7, 2, 0, 0, 0, 2, 0, 2]);
        self._create_ma3_voice("midi.percus5", "TaikoDrm", vec![4, 7, 4, 15, 11, 4, 3, 5, 24, 0, 0, 0, 0, 0, 0, 15, 12, 10, 3, 1, 19, 0, 0, 4, 0, 0, 0, 15, 12, 4, 3, 4, 11, 0, 0, 3, 0, 0, 0, 15, 14, 5, 5, 0, 0, 0, 0, 0, 0, 0]);
        self._create_ma3_voice("midi.percus6", "MelodTom", vec![5, 7, 4, 15, 9, 8, 8, 5, 14, 0, 2, 1, 0, 2, 0, 15, 3, 5, 5, 14, 0, 0, 0, 0, 0, 2, 1, 15, 3, 4, 4, 5, 12, 1, 2, 0, 0, 2, 0, 15, 4, 4, 4, 14, 0, 1, 0, 0, 0, 2]);
        self._create_ma3_voice("midi.percus7", "Syn.Drum", vec![5, 2, 12, 14, 11, 8, 10, 2, 0, 1, 0, 0, 7, 0, 0, 12, 3, 0, 4, 15, 0, 1, 0, 0, 0, 0, 24, 15, 4, 0, 4, 7, 0, 0, 0, 0, 0, 2, 22, 15, 7, 0, 7, 15, 0, 0, 0, 0, 0, 2]);
        self._create_ma3_voice("midi.percus8", "RevCymbl", vec![5, 7, 0, 4, 15, 0, 0, 0, 0, 0, 0, 14, 0, 2, 8, 2, 15, 15, 15, 15, 6, 0, 0, 9, 0, 1, 0, 4, 15, 0, 0, 0, 0, 0, 0, 14, 0, 2, 3, 2, 15, 15, 15, 15, 6, 0, 0, 14, 0, 2]);
        self._create_ma3_voice("midi.se1", "FretNoiz", vec![5, 7, 8, 15, 8, 4, 6, 2, 0, 0, 2, 6, 0, 2, 2, 8, 6, 4, 10, 8, 15, 0, 2, 3, 0, 2, 10, 15, 6, 2, 6, 2, 0, 0, 0, 6, 0, 2, 2, 8, 8, 8, 10, 8, 0, 0, 2, 6, 0, 2]);
        self._create_ma3_voice("midi.se2", "BrthNoiz", vec![5, 7, 2, 10, 12, 0, 5, 1, 14, 0, 0, 5, 0, 0, 0, 7, 8, 8, 9, 15, 5, 0, 0, 1, 0, 0, 0, 10, 12, 0, 5, 1, 9, 0, 0, 8, 0, 0, 0, 8, 8, 7, 9, 15, 12, 0, 0, 1, 0, 0]);
        self._create_ma3_voice("midi.se3", "Seashore", vec![3, 7, 0, 15, 15, 0, 0, 0, 4, 0, 0, 3, 0, 0, 4, 15, 2, 2, 2, 15, 21, 0, 0, 0, 0, 0, 1, 15, 0, 4, 4, 15, 12, 0, 0, 0, 0, 0, 0, 1, 2, 4, 4, 15, 0, 0, 0, 0, 0, 0]);
        self._create_ma3_voice("midi.se4", "Tweet", vec![5, 0, 0, 3, 7, 3, 3, 10, 21, 1, 2, 5, 0, 2, 0, 5, 6, 7, 7, 3, 0, 1, 1, 10, 0, 2, 0, 3, 7, 3, 3, 10, 21, 1, 2, 5, 0, 2, 0, 5, 6, 4, 4, 3, 0, 1, 1, 10, 7, 3]);
        self._create_ma3_voice("midi.se5", "Telphone", vec![5, 5, 2, 11, 2, 0, 0, 6, 28, 1, 1, 5, 0, 2, 0, 15, 4, 3, 3, 1, 3, 1, 0, 4, 0, 2, 2, 11, 2, 0, 0, 6, 28, 1, 1, 5, 0, 2, 0, 15, 4, 3, 3, 1, 15, 1, 0, 4, 7, 2]);
        self._create_ma3_voice("midi.se6", "Helicptr", vec![5, 5, 14, 15, 6, 0, 0, 0, 1, 0, 0, 15, 0, 2, 0, 2, 0, 0, 5, 0, 4, 0, 0, 0, 0, 2, 14, 15, 0, 0, 0, 0, 24, 0, 0, 0, 0, 2, 0, 2, 0, 0, 5, 0, 11, 0, 0, 0, 0, 2]);
        self._create_ma3_voice("midi.se7", "Applause", vec![5, 7, 24, 15, 12, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 4, 2, 0, 5, 0, 8, 0, 0, 0, 0, 0, 24, 6, 0, 0, 1, 15, 0, 0, 0, 9, 0, 1, 0, 3, 0, 7, 7, 15, 7, 0, 0, 3, 0, 0]);
        self._create_ma3_voice("midi.se8", "Gunshot", vec![5, 7, 0, 15, 3, 0, 0, 15, 11, 0, 0, 5, 0, 0, 6, 15, 6, 8, 8, 11, 4, 0, 0, 15, 0, 0, 1, 15, 2, 0, 0, 15, 2, 0, 0, 5, 0, 0, 6, 15, 6, 8, 8, 11, 26, 0, 0, 5, 0, 0]);
    }

    /// C++ `_generate_mididrum_voices` — cpp lines 500-567.
    fn _generate_mididrum_voices(&mut self) {
        self._begin_category("midi.drum");
        self._create_ma3_voice("midi.drum24", "Seq Click H", vec![5, 6, 4, 15, 0, 15, 15, 0, 56, 0, 0, 6, 0, 2, 0, 15, 8, 15, 15, 12, 0, 1, 0, 5, 0, 2, 6, 15, 11, 15, 15, 11, 39, 0, 0, 10, 0, 2, 0, 14, 11, 15, 15, 15, 1, 1, 0, 5, 0, 2]);
        self._create_ma3_voice("midi.drum25", "Brush Tap", vec![5, 7, 0, 15, 8, 0, 0, 3, 0, 0, 0, 5, 0, 2, 0, 9, 8, 8, 11, 11, 0, 0, 2, 0, 0, 2, 6, 15, 8, 6, 14, 3, 36, 0, 0, 12, 0, 2, 6, 12, 13, 8, 8, 0, 44, 0, 2, 1, 0, 2]);
        self._create_ma3_voice("midi.drum26", "Brush Swirl L", vec![5, 7, 18, 15, 5, 0, 6, 0, 0, 0, 0, 0, 0, 2, 3, 9, 8, 0, 10, 6, 0, 0, 0, 0, 0, 3, 16, 15, 0, 0, 15, 15, 0, 0, 0, 0, 0, 0, 0, 3, 6, 3, 15, 6, 21, 0, 0, 1, 0, 3]);
        self._create_ma3_voice("midi.drum27", "Brush Slap", vec![5, 7, 0, 15, 0, 0, 0, 0, 0, 0, 0, 4, 0, 2, 13, 12, 8, 6, 12, 9, 0, 0, 0, 14, 0, 2, 7, 11, 10, 4, 7, 13, 9, 0, 0, 0, 0, 2, 0, 8, 8, 11, 11, 13, 0, 0, 2, 0, 0, 2]);
        self._create_ma3_voice("midi.drum28", "Brush Swirl H", vec![5, 7, 18, 15, 5, 0, 3, 0, 0, 0, 0, 5, 5, 2, 3, 9, 6, 0, 10, 6, 0, 0, 0, 3, 0, 3, 16, 15, 0, 0, 15, 15, 0, 0, 0, 9, 0, 0, 20, 3, 6, 3, 15, 6, 16, 0, 0, 3, 0, 3]);
        self._create_ma3_voice("midi.drum29", "Snare Roll", vec![5, 7, 2, 7, 0, 0, 2, 0, 13, 0, 0, 8, 3, 1, 26, 15, 5, 0, 9, 3, 0, 0, 0, 0, 6, 0, 0, 15, 10, 8, 4, 3, 8, 0, 0, 3, 0, 2, 0, 14, 6, 7, 7, 5, 0, 0, 0, 0, 0, 2]);
        self._create_ma3_voice("midi.drum30", "Castanet", vec![5, 6, 1, 15, 7, 5, 9, 15, 2, 0, 0, 7, 0, 2, 6, 10, 8, 5, 15, 15, 0, 1, 0, 5, 0, 2, 5, 15, 5, 6, 5, 0, 39, 0, 0, 2, 0, 2, 0, 12, 10, 9, 9, 10, 20, 0, 0, 5, 0, 2]);
        self._create_ma3_voice("midi.drum31", "Snare L", vec![5, 7, 24, 15, 12, 0, 0, 1, 9, 0, 0, 0, 0, 2, 0, 15, 7, 7, 7, 3, 0, 0, 0, 0, 0, 2, 0, 14, 11, 7, 5, 15, 48, 0, 0, 7, 0, 2, 0, 15, 10, 6, 0, 0, 28, 0, 0, 0, 0, 2]);
        self._create_ma3_voice("midi.drum32", "Sticks", vec![6, 7, 20, 15, 9, 12, 8, 9, 0, 0, 1, 15, 0, 2, 3, 13, 10, 2, 8, 11, 0, 0, 0, 10, 0, 2, 3, 13, 8, 2, 5, 5, 21, 0, 1, 11, 0, 2, 8, 12, 11, 9, 7, 11, 0, 0, 0, 13, 0, 2]);
        self._create_ma3_voice("midi.drum33", "Bass Drum L", vec![5, 1, 4, 15, 10, 11, 6, 15, 0, 0, 0, 2, 0, 2, 0, 15, 6, 6, 5, 7, 16, 0, 0, 2, 0, 2, 1, 11, 9, 7, 4, 7, 13, 0, 0, 5, 0, 2, 12, 15, 7, 8, 5, 7, 0, 0, 0, 1, 0, 2]);
        self._create_ma3_voice("midi.drum34", "Open Rim Shot", vec![5, 7, 2, 15, 0, 5, 5, 0, 5, 1, 0, 12, 0, 2, 0, 15, 7, 7, 7, 7, 0, 1, 0, 11, 0, 2, 0, 15, 10, 6, 6, 8, 0, 0, 0, 12, 0, 2, 2, 15, 7, 7, 7, 7, 0, 0, 0, 7, 0, 2]);
        self._create_ma3_voice("midi.drum35", "Bass Drum M", vec![5, 6, 6, 15, 13, 7, 7, 15, 0, 0, 0, 13, 0, 2, 0, 15, 7, 7, 4, 7, 0, 0, 0, 2, 0, 2, 10, 15, 9, 7, 6, 7, 19, 0, 0, 3, 0, 2, 12, 15, 8, 10, 6, 11, 0, 0, 0, 2, 0, 2]);
        self._create_ma3_voice("midi.drum36", "Bass Drum H", vec![5, 6, 6, 15, 13, 7, 6, 15, 14, 0, 0, 13, 0, 2, 0, 15, 7, 7, 7, 7, 12, 0, 0, 2, 0, 2, 2, 15, 9, 7, 4, 7, 9, 0, 0, 2, 0, 2, 12, 15, 7, 8, 6, 7, 0, 0, 0, 1, 0, 2]);
        self._create_ma3_voice("midi.drum37", "Side Stick", vec![5, 6, 1, 15, 2, 0, 8, 5, 8, 0, 0, 10, 0, 2, 6, 13, 9, 3, 3, 15, 0, 1, 0, 7, 0, 2, 18, 11, 0, 0, 9, 13, 0, 0, 0, 9, 0, 2, 18, 13, 9, 11, 9, 11, 0, 0, 0, 0, 0, 2]);
        self._create_ma3_voice("midi.drum38", "Snare M", vec![5, 7, 13, 15, 0, 5, 7, 0, 7, 1, 0, 12, 0, 2, 0, 15, 7, 10, 9, 7, 0, 1, 0, 8, 0, 2, 0, 15, 5, 7, 8, 6, 0, 0, 0, 12, 0, 2, 2, 15, 7, 7, 6, 7, 1, 0, 0, 7, 0, 2]);
        self._create_ma3_voice("midi.drum39", "Hand Clap", vec![5, 7, 0, 15, 4, 4, 6, 0, 2, 0, 0, 0, 0, 2, 5, 15, 6, 9, 4, 1, 0, 0, 0, 0, 0, 2, 27, 15, 8, 0, 15, 1, 0, 0, 2, 13, 1, 2, 2, 15, 10, 9, 8, 5, 0, 0, 2, 15, 0, 2]);
        self._create_ma3_voice("midi.drum40", "Snare H", vec![5, 7, 2, 15, 1, 0, 7, 0, 10, 0, 0, 12, 0, 2, 0, 15, 7, 11, 7, 13, 0, 0, 0, 11, 0, 0, 28, 15, 10, 6, 5, 8, 9, 0, 0, 9, 0, 2, 18, 15, 8, 10, 6, 1, 0, 0, 0, 7, 0, 2]);
        self._create_ma3_voice("midi.drum41", "Floor Tom L", vec![5, 4, 6, 13, 10, 6, 6, 15, 9, 0, 0, 12, 0, 2, 0, 15, 10, 7, 11, 7, 0, 0, 0, 5, 0, 2, 1, 15, 3, 5, 5, 15, 21, 0, 0, 0, 0, 2, 0, 15, 6, 5, 6, 7, 0, 0, 0, 5, 0, 2]);
        self._create_ma3_voice("midi.drum42", "Hi-Hat Closed", vec![5, 7, 0, 15, 0, 0, 0, 3, 2, 0, 0, 15, 0, 2, 0, 11, 8, 12, 8, 11, 0, 0, 2, 0, 0, 2, 14, 15, 3, 3, 13, 3, 12, 0, 0, 0, 0, 2, 11, 11, 10, 8, 11, 0, 0, 0, 2, 13, 0, 2]);
        self._create_ma3_voice("midi.drum43", "Floor Tom H", vec![5, 0, 6, 12, 5, 6, 6, 15, 0, 0, 0, 12, 0, 2, 19, 15, 9, 7, 11, 7, 0, 0, 0, 11, 0, 2, 1, 15, 4, 4, 5, 15, 26, 0, 0, 0, 0, 2, 0, 15, 6, 4, 6, 5, 0, 0, 0, 5, 0, 2]);
        self._create_ma3_voice("midi.drum44", "Hi-Hat Pedal", vec![5, 7, 0, 15, 0, 0, 0, 3, 0, 0, 0, 12, 0, 2, 0, 7, 8, 8, 8, 11, 15, 0, 2, 0, 0, 2, 20, 15, 3, 5, 14, 3, 8, 0, 0, 6, 0, 2, 11, 8, 3, 8, 8, 0, 0, 0, 2, 5, 0, 2]);
        self._create_ma3_voice("midi.drum45", "Low Tom", vec![5, 0, 6, 12, 5, 6, 6, 15, 1, 0, 0, 10, 0, 2, 18, 15, 8, 7, 9, 7, 0, 0, 0, 1, 0, 2, 1, 15, 3, 5, 5, 15, 17, 0, 0, 0, 0, 2, 0, 15, 6, 5, 5, 7, 0, 0, 0, 5, 0, 2]);
        self._create_ma3_voice("midi.drum46", "Hi-Hat Open", vec![5, 7, 0, 15, 0, 0, 0, 3, 2, 0, 0, 15, 0, 2, 0, 11, 7, 6, 8, 11, 1, 0, 2, 0, 0, 2, 14, 15, 3, 3, 13, 3, 12, 0, 0, 0, 0, 2, 11, 11, 9, 5, 11, 0, 9, 0, 2, 13, 0, 2]);
        self._create_ma3_voice("midi.drum47", "Mid Tom L", vec![5, 0, 6, 12, 5, 6, 6, 15, 0, 0, 0, 10, 0, 2, 18, 15, 9, 7, 10, 6, 0, 0, 0, 8, 0, 2, 0, 15, 4, 2, 5, 15, 28, 0, 0, 0, 0, 2, 0, 15, 6, 5, 7, 7, 2, 0, 0, 4, 0, 2]);
        self._create_ma3_voice("midi.drum48", "Mid Tom H", vec![5, 0, 6, 12, 4, 6, 5, 15, 9, 0, 0, 10, 0, 2, 19, 15, 9, 7, 7, 7, 0, 0, 0, 1, 0, 2, 1, 15, 4, 5, 5, 15, 39, 0, 0, 1, 0, 2, 0, 15, 6, 6, 7, 7, 3, 0, 0, 4, 0, 2]);
        self._create_ma3_voice("midi.drum49", "Crash Cymbal 1", vec![5, 3, 16, 15, 9, 0, 6, 0, 14, 0, 0, 13, 0, 2, 18, 9, 3, 4, 5, 2, 3, 0, 0, 11, 0, 0, 0, 11, 3, 0, 4, 0, 0, 0, 0, 15, 0, 2, 6, 12, 4, 5, 5, 7, 13, 0, 0, 15, 0, 2]);
        self._create_ma3_voice("midi.drum50", "High Tom", vec![5, 0, 6, 12, 4, 6, 7, 15, 4, 0, 0, 8, 0, 2, 19, 15, 10, 7, 8, 7, 0, 0, 0, 6, 0, 2, 5, 15, 5, 5, 6, 15, 50, 0, 0, 1, 0, 2, 0, 12, 6, 5, 6, 7, 0, 0, 0, 5, 0, 2]);
        self._create_ma3_voice("midi.drum51", "Ride Cymbal 1", vec![5, 7, 0, 15, 7, 0, 0, 4, 9, 0, 2, 15, 0, 2, 14, 14, 4, 4, 4, 14, 3, 0, 0, 9, 0, 2, 0, 15, 0, 0, 0, 4, 8, 0, 2, 14, 0, 2, 11, 15, 4, 2, 5, 13, 16, 0, 0, 14, 0, 2]);
        self._create_ma3_voice("midi.drum52", "Chinese Cymbal", vec![5, 7, 0, 15, 3, 7, 2, 1, 31, 0, 0, 0, 0, 2, 30, 14, 3, 5, 2, 0, 0, 1, 0, 5, 0, 0, 0, 15, 1, 8, 3, 6, 15, 0, 0, 2, 0, 2, 6, 9, 2, 5, 3, 0, 9, 0, 0, 0, 0, 2]);
        self._create_ma3_voice("midi.drum53", "Ride Cymbal Cup", vec![5, 7, 19, 15, 7, 0, 0, 4, 22, 0, 2, 15, 0, 2, 12, 15, 5, 4, 4, 14, 0, 0, 0, 15, 0, 2, 19, 15, 0, 0, 0, 4, 12, 0, 2, 12, 0, 2, 11, 15, 5, 2, 5, 13, 27, 0, 0, 15, 0, 2]);
        self._create_ma3_voice("midi.drum54", "Tambourine", vec![5, 6, 0, 8, 7, 4, 1, 2, 10, 0, 0, 11, 0, 2, 13, 14, 7, 12, 11, 5, 0, 0, 0, 5, 0, 2, 8, 8, 7, 5, 2, 2, 0, 0, 0, 11, 0, 2, 8, 13, 7, 12, 7, 6, 0, 0, 0, 15, 0, 2]);
        self._create_ma3_voice("midi.drum55", "Splash Cymbal", vec![5, 7, 25, 12, 6, 3, 3, 0, 0, 0, 0, 8, 0, 0, 3, 9, 3, 4, 3, 12, 0, 1, 0, 13, 0, 0, 13, 5, 0, 3, 3, 0, 1, 0, 0, 5, 0, 0, 19, 12, 3, 5, 8, 6, 0, 1, 0, 15, 0, 0]);
        self._create_ma3_voice("midi.drum56", "Cowbell", vec![5, 4, 0, 15, 9, 8, 5, 3, 0, 1, 2, 3, 0, 2, 0, 10, 7, 5, 3, 3, 0, 1, 0, 0, 0, 2, 2, 15, 15, 11, 9, 10, 3, 1, 0, 4, 0, 2, 0, 15, 12, 5, 5, 3, 32, 1, 0, 1, 0, 2]);
        self._create_ma3_voice("midi.drum57", "Crash Cymbal 2", vec![5, 7, 0, 15, 6, 0, 0, 0, 0, 0, 2, 9, 0, 2, 0, 9, 4, 2, 2, 0, 0, 1, 0, 0, 0, 0, 0, 15, 3, 0, 0, 0, 0, 0, 2, 15, 0, 2, 2, 9, 6, 3, 4, 3, 8, 0, 0, 7, 4, 2]);
        self._create_ma3_voice("midi.drum58", "Vibraslap", vec![5, 7, 29, 13, 1, 2, 6, 0, 0, 0, 0, 1, 2, 1, 8, 15, 6, 5, 9, 2, 0, 0, 0, 0, 6, 0, 29, 15, 5, 4, 4, 0, 29, 0, 0, 6, 0, 2, 30, 15, 6, 5, 4, 0, 0, 0, 0, 0, 0, 2]);
        self._create_ma3_voice("midi.drum59", "Ride Cymbal 2", vec![5, 7, 7, 15, 0, 0, 6, 0, 5, 0, 0, 14, 0, 3, 14, 14, 5, 5, 4, 11, 16, 0, 0, 15, 0, 2, 6, 15, 0, 0, 0, 4, 7, 0, 2, 9, 0, 2, 11, 15, 5, 3, 5, 14, 0, 0, 0, 9, 0, 2]);
        self._create_ma3_voice("midi.drum60", "Bongo H", vec![5, 3, 0, 15, 5, 12, 12, 0, 0, 0, 0, 6, 0, 2, 0, 15, 5, 8, 7, 0, 0, 0, 0, 11, 0, 2, 0, 15, 5, 12, 12, 0, 2, 0, 0, 15, 0, 2, 0, 15, 4, 7, 3, 0, 0, 0, 0, 15, 0, 2]);
        self._create_ma3_voice("midi.drum61", "Bongo L", vec![5, 3, 0, 15, 5, 12, 12, 0, 0, 0, 0, 6, 0, 2, 0, 15, 5, 8, 7, 0, 7, 0, 0, 11, 0, 2, 0, 15, 5, 12, 12, 0, 6, 0, 0, 15, 0, 2, 0, 15, 5, 8, 3, 0, 0, 0, 0, 15, 0, 2]);
        self._create_ma3_voice("midi.drum62", "Conga H Mute", vec![5, 4, 0, 14, 14, 10, 10, 1, 6, 0, 0, 0, 2, 2, 8, 15, 8, 9, 10, 1, 0, 0, 0, 1, 0, 2, 0, 15, 0, 0, 0, 0, 44, 0, 0, 0, 0, 2, 0, 15, 13, 9, 10, 1, 7, 0, 0, 0, 0, 2]);
        self._create_ma3_voice("midi.drum63", "Conga H Open", vec![5, 7, 3, 10, 10, 5, 8, 12, 38, 0, 0, 10, 0, 2, 0, 12, 6, 7, 9, 4, 0, 0, 0, 12, 0, 2, 0, 15, 10, 6, 8, 8, 0, 0, 0, 12, 0, 2, 2, 15, 7, 7, 9, 7, 0, 0, 0, 2, 0, 2]);
        self._create_ma3_voice("midi.drum64", "Conga L", vec![5, 7, 3, 10, 10, 5, 8, 12, 31, 0, 0, 10, 0, 2, 0, 12, 5, 6, 9, 4, 0, 0, 0, 12, 0, 2, 0, 15, 10, 6, 8, 8, 0, 0, 0, 12, 0, 2, 10, 15, 7, 7, 9, 7, 0, 0, 0, 2, 0, 2]);
        self._create_ma3_voice("midi.drum65", "Timbale H", vec![3, 6, 0, 12, 7, 0, 8, 0, 22, 0, 0, 2, 0, 2, 0, 15, 9, 6, 8, 0, 49, 0, 0, 7, 0, 2, 4, 13, 6, 10, 6, 1, 26, 0, 0, 9, 0, 2, 6, 15, 9, 6, 8, 0, 6, 0, 0, 9, 0, 2]);
        self._create_ma3_voice("midi.drum66", "Timbale L", vec![3, 6, 0, 12, 7, 0, 8, 0, 22, 0, 0, 2, 0, 2, 0, 15, 9, 6, 8, 0, 49, 0, 0, 7, 0, 2, 4, 13, 6, 10, 6, 1, 23, 0, 0, 12, 0, 2, 6, 15, 9, 6, 8, 0, 4, 0, 0, 9, 0, 2]);
        self._create_ma3_voice("midi.drum67", "Agogo H", vec![5, 5, 0, 14, 12, 6, 6, 2, 21, 0, 0, 7, 0, 2, 0, 15, 8, 6, 6, 1, 10, 0, 0, 2, 0, 2, 0, 14, 12, 6, 5, 2, 26, 0, 0, 7, 0, 2, 0, 15, 8, 6, 6, 1, 10, 0, 0, 2, 0, 2]);
        self._create_ma3_voice("midi.drum68", "Agogo L", vec![5, 5, 0, 14, 12, 6, 6, 2, 21, 0, 0, 7, 0, 2, 0, 15, 8, 6, 6, 1, 10, 0, 0, 2, 0, 2, 0, 14, 12, 5, 5, 2, 13, 0, 0, 7, 0, 2, 0, 15, 8, 6, 6, 1, 29, 0, 0, 2, 0, 2]);
        self._create_ma3_voice("midi.drum69", "Cabasa", vec![3, 7, 0, 15, 0, 3, 5, 6, 0, 0, 0, 7, 0, 2, 0, 14, 8, 4, 4, 10, 24, 1, 0, 15, 0, 2, 0, 14, 4, 2, 6, 4, 0, 0, 0, 15, 0, 2, 6, 8, 11, 9, 4, 0, 2, 0, 0, 15, 0, 2]);
        self._create_ma3_voice("midi.drum70", "Maracas", vec![4, 7, 0, 15, 0, 3, 5, 6, 4, 0, 0, 15, 0, 2, 0, 8, 8, 4, 4, 6, 3, 1, 0, 15, 0, 2, 6, 12, 9, 3, 3, 15, 0, 0, 0, 15, 0, 2, 12, 7, 10, 10, 3, 8, 0, 0, 0, 15, 0, 2]);
        self._create_ma3_voice("midi.drum71", "Samba Whistle H", vec![5, 5, 1, 15, 0, 0, 7, 0, 10, 0, 0, 0, 0, 1, 0, 12, 1, 0, 12, 1, 43, 0, 0, 14, 0, 1, 0, 15, 0, 0, 7, 0, 23, 0, 0, 0, 0, 2, 0, 8, 1, 0, 12, 0, 6, 0, 0, 15, 0, 1]);
        self._create_ma3_voice("midi.drum72", "Samba Whistle L", vec![5, 5, 1, 15, 0, 0, 7, 0, 10, 0, 0, 0, 0, 1, 0, 12, 1, 0, 12, 1, 43, 0, 0, 14, 0, 1, 0, 15, 0, 0, 7, 0, 23, 0, 0, 0, 0, 2, 0, 8, 1, 0, 12, 0, 6, 0, 0, 15, 0, 1]);
        self._create_ma3_voice("midi.drum73", "Guiro Short", vec![3, 7, 1, 12, 2, 6, 0, 0, 12, 0, 0, 1, 0, 0, 4, 12, 3, 6, 10, 0, 0, 0, 0, 3, 0, 0, 11, 13, 1, 0, 6, 0, 32, 0, 0, 14, 5, 0, 7, 15, 7, 12, 15, 6, 0, 1, 0, 0, 1, 0]);
        self._create_ma3_voice("midi.drum74", "Guiro Long", vec![3, 7, 1, 12, 2, 6, 0, 0, 14, 0, 0, 1, 0, 0, 4, 12, 3, 6, 10, 0, 0, 0, 0, 3, 0, 0, 11, 15, 0, 0, 6, 0, 32, 0, 0, 13, 0, 0, 7, 9, 6, 14, 12, 6, 5, 0, 0, 0, 2, 0]);
        self._create_ma3_voice("midi.drum75", "Claves", vec![4, 0, 0, 13, 7, 6, 6, 10, 11, 0, 0, 7, 0, 2, 0, 15, 14, 11, 3, 10, 13, 0, 0, 7, 0, 2, 0, 15, 11, 8, 0, 12, 0, 1, 0, 0, 0, 2, 8, 15, 3, 8, 8, 0, 0, 1, 0, 15, 0, 2]);
        self._create_ma3_voice("midi.drum76", "Wood Block H", vec![4, 0, 0, 14, 6, 6, 6, 10, 63, 0, 0, 15, 0, 2, 0, 15, 6, 6, 5, 11, 0, 0, 0, 6, 0, 2, 2, 13, 12, 0, 10, 12, 0, 1, 0, 7, 0, 2, 8, 15, 7, 12, 7, 4, 0, 1, 0, 3, 0, 2]);
        self._create_ma3_voice("midi.drum77", "Wood Block L", vec![4, 0, 0, 14, 6, 6, 6, 10, 63, 0, 0, 15, 0, 2, 0, 15, 6, 6, 5, 11, 0, 0, 0, 6, 0, 2, 2, 15, 10, 0, 10, 12, 16, 1, 0, 10, 0, 2, 16, 12, 7, 12, 12, 4, 0, 1, 0, 3, 0, 2]);
        self._create_ma3_voice("midi.drum78", "Cuica Mute", vec![5, 0, 0, 13, 10, 12, 13, 11, 15, 0, 2, 0, 0, 2, 0, 6, 0, 6, 5, 0, 6, 0, 0, 6, 0, 2, 0, 15, 8, 8, 12, 0, 14, 0, 0, 0, 0, 2, 0, 6, 0, 7, 7, 0, 2, 0, 0, 6, 0, 2]);
        self._create_ma3_voice("midi.drum79", "Cuica Open", vec![5, 1, 16, 9, 8, 15, 8, 15, 12, 0, 0, 0, 0, 0, 8, 7, 6, 8, 8, 4, 0, 0, 0, 14, 0, 0, 0, 15, 7, 0, 0, 0, 36, 0, 0, 2, 0, 2, 0, 13, 7, 11, 11, 12, 18, 0, 0, 11, 0, 2]);
        self._create_ma3_voice("midi.drum80", "Triangle Mute", vec![7, 0, 22, 15, 12, 8, 4, 0, 5, 0, 0, 9, 0, 2, 4, 11, 5, 9, 5, 6, 0, 0, 0, 15, 0, 2, 6, 15, 11, 12, 7, 3, 7, 0, 0, 14, 0, 2, 0, 14, 13, 8, 11, 15, 6, 0, 0, 6, 0, 2]);
        self._create_ma3_voice("midi.drum81", "Triangle Open", vec![7, 0, 22, 15, 5, 5, 4, 0, 3, 0, 0, 9, 0, 2, 4, 11, 5, 9, 5, 6, 0, 0, 0, 15, 0, 2, 6, 15, 9, 9, 7, 3, 8, 0, 0, 14, 0, 2, 0, 14, 5, 5, 11, 15, 3, 0, 0, 6, 0, 2]);
        self._create_ma3_voice("midi.drum82", "Shaker", vec![4, 7, 0, 9, 0, 3, 5, 6, 0, 0, 0, 7, 0, 2, 0, 9, 6, 4, 4, 6, 4, 1, 0, 3, 0, 2, 0, 14, 4, 2, 6, 4, 1, 0, 0, 12, 0, 2, 6, 9, 10, 9, 4, 8, 2, 0, 0, 15, 0, 2]);
        self._create_ma3_voice("midi.drum83", "Jingle Bell", vec![5, 0, 2, 7, 6, 0, 0, 0, 20, 0, 0, 6, 0, 2, 3, 6, 11, 5, 5, 4, 0, 0, 0, 2, 0, 2, 2, 3, 0, 0, 2, 0, 7, 1, 0, 6, 0, 2, 2, 7, 5, 2, 6, 15, 0, 0, 0, 9, 0, 2]);
        self._create_ma3_voice("midi.drum84", "Bell Tree", vec![5, 7, 19, 12, 4, 2, 0, 0, 24, 0, 0, 3, 0, 2, 2, 4, 3, 5, 5, 4, 0, 0, 0, 2, 0, 3, 2, 3, 0, 0, 2, 0, 7, 1, 0, 7, 0, 3, 13, 5, 4, 2, 6, 6, 2, 0, 0, 3, 0, 3]);
    }

    /// C++ `_generate_wave_table_voices` — cpp lines 569-757 (wave-9 pass D).
    fn _generate_wave_table_voices(&mut self) {
        self._register_wave_table(vec![1716,914,556,336,190,92,32,4,4,32,92,190,336,556,914,1716,1717,915,557,337,191,93,33,5,5,33,93,191,337,557,915,1717]);
        self._register_wave_table(vec![828,440,266,160,90,42,14,0,0,14,42,90,160,266,440,828,829,441,267,161,91,43,15,1,1,15,43,91,161,267,441,829]);
        self._register_wave_table(vec![1716,914,556,336,190,92,32,4,4,32,92,190,336,556,914,1716,1,107,231,379,563,813,1189,2001,2000,1188,812,562,378,230,106,0]);
        self._register_wave_table(vec![1716,914,556,336,190,92,32,4,4,32,92,190,336,556,914,1716,1209,435,137,15,15,137,435,1209,1208,434,136,14,14,136,434,1208]);
        self._register_wave_table(vec![4,32,92,190,336,556,914,1716,1,33,93,191,337,557,915,1717,1717,915,557,337,191,93,33,1,1716,914,556,336,190,92,32,4]);
        self._register_wave_table(vec![2001,1189,813,563,379,231,107,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,0,106,230,378,562,812,1188,2000]);
        self._register_wave_table(vec![1,51,103,159,221,289,363,445,537,643,767,915,1101,1349,1727,2537,2536,1726,1348,1100,914,766,642,536,444,362,288,220,158,102,50,0]);
        self._register_wave_table(vec![2001,535,129,1,1,69,143,225,317,423,545,695,879,1129,1505,2317,2316,1504,1128,878,694,544,422,316,224,142,68,0,0,128,534,2000]);
        self._register_wave_table(vec![2000,1188,812,562,378,230,106,0,0,106,230,378,562,812,1188,2000,2001,1189,813,563,379,231,107,1,1,107,231,379,563,813,1189,2001]);
        self._register_wave_table(vec![6654,1024,512,212,0,248,626,1438,6654,1025,513,213,1,249,627,1439,6654,1024,512,212,0,248,626,1438,6654,1025,513,213,1,249,627,1439]);
        self._register_wave_table(vec![1208,434,136,14,14,136,434,1208,1209,435,137,15,15,137,435,1209,1084,352,88,14,92,362,1118,1119,363,93,15,89,353,1085,0,1]);
        self._register_wave_table(vec![1540,740,436,310,310,436,740,1540,462,132,0,0,132,462,1422,1671,1670,1423,463,133,1,1,133,463,1541,741,437,311,311,437,741,1541]);
        self._register_wave_table(vec![2000,1188,812,562,378,230,106,0,0,106,230,378,562,812,1188,2000,1439,627,249,1,1,249,627,1439,1438,626,248,0,0,248,626,1438]);
        self._register_wave_table(vec![1668,868,512,296,156,66,16,0,0,16,66,156,296,512,868,1668,1439,627,249,1,1,249,627,1439,1438,626,248,0,0,248,626,1438]);
        self._register_wave_table(vec![0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1]);
        self._register_wave_table(vec![0,0,0,0,0,0,0,0,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1]);
        self._register_wave_table(vec![0,0,0,0,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1]);
        self._register_wave_table(vec![0,0,0,0,0,0,0,0,1,1,1,1,1,1,1,1,0,0,0,1,1,1,0,0,0,0,0,1,1,1,1,1]);
        self._register_wave_table(vec![6654,6654,6654,6654,0,0,0,1,1,1,6654,6654,6654,6654,6654,0,0,0,1,1,1,6654,6654,6654,6654,6654,0,0,0,1,1,1]);
        self._register_wave_table(vec![1,6654,6654,0,0,0,6654,1,1,1,1,6654,6654,6654,6654,0,0,0,0,0,1,0,41,633,5396,944,724,724,944,5396,633,41]);
        self._register_wave_table(vec![6654,6654,6654,1,6,6,6,6,6654,6654,6654,6654,1,6654,6654,1,1,1,1,6654,1,1,1,6654,6654,6654,1,1,1,6654,1,1]);
        self._register_wave_table(vec![1,379,1189,1188,378,0,0,378,1188,1189,379,1,295,553,1043,4105,1066,562,302,144,50,0,0,50,144,302,562,1066,4105,1043,553,295]);
        self._register_wave_table(vec![1188,378,0,0,378,1188,757,6654,757,6654,1917,1111,745,513,349,231,141,79,35,9,1,1,9,35,79,141,231,349,513,745,1111,1917]);
        self._register_wave_table(vec![1188,378,0,0,378,1188,1853,1171,831,607,445,321,225,151,95,53,25,7,1,1,7,25,53,95,151,225,321,445,607,831,1171,1853]);
        self._register_wave_table(vec![1324,512,134,0,300,812,6654,813,301,1,135,513,1325,1324,512,134,0,300,812,6654,1269,599,773,1537,1096,1884,6654,6654,0,0,0,0]);
        self._register_wave_table(vec![1324,512,134,0,300,812,6654,813,301,1,135,513,1325,1324,512,134,0,300,812,6654,1269,599,773,1537,1096,1884,6654,6654,6654,824,824,824]);
        self._register_wave_table(vec![1668,834,482,274,142,60,14,0,0,14,60,142,274,482,834,1668,1068,38,806,494,30,293,1049,20,56,18,1845,39,1190,272,1260,11]);
        self._register_wave_table(vec![1290,134,56,272,956,834,104,60,330,174,2,0,56,994,603,529,2845,700,1186,411,1,25,1,141,185,1,315,1685,1235,307,105,319]);
        self._register_wave_table(vec![1028,32,820,197,112,546,80,1775,26,933,46,330,502,97,620,4,1029,33,821,196,113,547,81,1774,27,932,47,331,503,96,621,5]);

        self._begin_category("svmidi");
        self._create_wave_table_voice("svmidi.piano1", "SV.GrandPno", 13, 63, 24, 14, 32, 4, 2, 1);
        self._create_wave_table_voice("svmidi.piano2", "SV.BritePno", 4, 63, 24, 14, 32, 4, 2, 1);
        self._create_wave_table_voice("svmidi.piano3", "SV.E.GrandP", 20, 63, 24, 14, 32, 4, 2, 1);
        self._create_wave_table_voice("svmidi.piano4", "SV.HnkyTonk", 11, 63, 24, 14, 32, 4, 2, 1);
        self._create_wave_table_voice("svmidi.piano5", "SV.E.Piano1", 27, 63, 24, 14, 32, 4, 2, 1);
        self._create_wave_table_voice("svmidi.piano6", "SV.E.Piano2", 23, 63, 24, 12, 32, 4, 0, 1);
        self._create_wave_table_voice("svmidi.piano7", "SV.Harpsi.", 15, 48, 48, 18, 32, 3, 4, 1);
        self._create_wave_table_voice("svmidi.piano8", "SV.Clavi.", 26, 63, 32, 12, 32, 2, 4, 1);

        self._create_wave_table_voice("svmidi.chrom1", "SV.Celesta", 27, 48, 36, 16, 24, 4, 4, 1);
        self._create_wave_table_voice("svmidi.chrom2", "SV.Glocken", 28, 48, 36, 12, 24, 5, 4, 1);
        self._create_wave_table_voice("svmidi.chrom3", "SV.MusicBox", 27, 63, 36, 24, 24, 2, 4, 1);
        self._create_wave_table_voice("svmidi.chrom4", "SV.Vibes", 27, 63, 40, 12, 24, 2, 4, 1);
        self._create_wave_table_voice("svmidi.chrom5", "SV.Marimba", 8, 63, 40, 24, 24, 0, 4, 1);
        self._create_wave_table_voice("svmidi.chrom6", "SV.Xylophon", 11, 63, 40, 24, 24, 0, 4, 1);
        self._create_wave_table_voice("svmidi.chrom7", "SV.TubulBel", 28, 63, 36, 12, 16, 1, 4, 0);
        self._create_wave_table_voice("svmidi.chrom8", "SV.Dulcimer", 16, 63, 44, 12, 24, 4, 4, 1);

        self._create_wave_table_voice("svmidi.organ1", "SV.DrawOrgn", 11, 63, 32, 0, 40, 2, 4, 1);
        self._create_wave_table_voice("svmidi.organ2", "SV.PercOrgn", 12, 63, 32, 0, 40, 2, 0, 1);
        self._create_wave_table_voice("svmidi.organ3", "SV.RockOrgn", 4, 63, 32, 0, 40, 2, 4, 1);
        self._create_wave_table_voice("svmidi.organ4", "SV.ChrchOrg", 11, 40, 32, 0, 24, 2, 4, 1);
        self._create_wave_table_voice("svmidi.organ5", "SV.ReedOrgn", 25, 63, 0, 0, 40, 15, 4, 1);
        self._create_wave_table_voice("svmidi.organ6", "SV.Acordion", 17, 30, 32, 0, 40, 4, 4, 1);
        self._create_wave_table_voice("svmidi.organ7", "SV.Harmnica", 20, 30, 32, 0, 32, 4, 4, 1);
        self._create_wave_table_voice("svmidi.organ8", "SV.TangoAcd", 17, 36, 32, 0, 40, 2, 4, 1);

        self._create_wave_table_voice("svmidi.guitar1", "SV.NylonGtr", 11, 63, 44, 16, 32, 3, -3, 1);
        self._create_wave_table_voice("svmidi.guitar2", "SV.SteelGtr", 16, 63, 44, 16, 32, 3, -2, 1);
        self._create_wave_table_voice("svmidi.guitar3", "SV.Jazz Gtr", 21, 40, 32, 16, 32, 3, -3, 1);
        self._create_wave_table_voice("svmidi.guitar4", "SV.CleanGtr", 17, 44, 44, 16, 32, 3, 0, 1);
        self._create_wave_table_voice("svmidi.guitar5", "SV.Mute.Gtr", 2, 40, 40, 16, 32, 3, 0, 0);
        self._create_wave_table_voice("svmidi.guitar6", "SV.Ovrdrive", 24, 40, 28, 8, 32, 1, 0, 0);
        self._create_wave_table_voice("svmidi.guitar7", "SV.Dist.Gtr", 20, 40, 28, 8, 32, 1, 0, 0);
        self._create_wave_table_voice("svmidi.guitar8", "SV.GtrHarmo", 10, 48, 32, 22, 32, 6, 2, 1);

        self._create_wave_table_voice("svmidi.bass1", "SV.Aco.Bass", 8, 48, 32, 8, 32, 2, -4, 1);
        self._create_wave_table_voice("svmidi.bass2", "SV.FngrBass", 13, 48, 32, 8, 32, 2, -4, 1);
        self._create_wave_table_voice("svmidi.bass3", "SV.PickBass", 7, 48, 40, 8, 32, 2, -4, 1);
        self._create_wave_table_voice("svmidi.bass4", "SV.Fretless", 7, 36, 32, 8, 32, 2, -4, 1);
        self._create_wave_table_voice("svmidi.bass5", "SV.SlapBas1", 11, 63, 40, 8, 32, 3, -4, 1);
        self._create_wave_table_voice("svmidi.bass6", "SV.SlapBas2", 13, 63, 40, 8, 32, 3, -4, 1);
        self._create_wave_table_voice("svmidi.bass7", "SV.SynBass1", 2, 40, 40, 8, 32, 2, -1, 1);
        self._create_wave_table_voice("svmidi.bass8", "SV.SynBass2", 10, 48, 40, 8, 32, 2, 0, 1);

        self._create_wave_table_voice("svmidi.strings1", "SV.Violin", 19, 24, 4, 4, 32, 15, 6, 1);
        self._create_wave_table_voice("svmidi.strings2", "SV.Viola", 3, 24, 4, 8, 32, 15, 6, 1);
        self._create_wave_table_voice("svmidi.strings3", "SV.Cello", 17, 28, 4, 8, 32, 15, 6, 1);
        self._create_wave_table_voice("svmidi.strings4", "SV.ContraBs", 17, 24, 4, 8, 32, 15, 6, 1);
        self._create_wave_table_voice("svmidi.strings5", "SV.Trem.Str", 10, 24, 4, 8, 32, 15, 6, 1);
        self._create_wave_table_voice("svmidi.strings6", "SV.Pizz.Str", 2, 40, 36, 24, 32, 6, 4, 1);
        self._create_wave_table_voice("svmidi.strings7", "SV.Harp", 8, 40, 36, 24, 24, 6, 4, 1);
        self._create_wave_table_voice("svmidi.strings8", "SV.Timpani", 8, 40, 36, 24, 24, 6, 4, 0);

        self._create_wave_table_voice("svmidi.ensemble1", "SV.std::strings1", 2, 36, 1, 1, 32, 15, 7, 1);
        self._create_wave_table_voice("svmidi.ensemble2", "SV.std::strings2", 23, 24, 1, 1, 32, 15, 6, 1);
        self._create_wave_table_voice("svmidi.ensemble3", "SV.Syn.Str1", 2, 36, 1, 1, 32, 15, 7, 1);
        self._create_wave_table_voice("svmidi.ensemble4", "SV.Syn.Str2", 23, 24, 1, 1, 32, 15, 6, 1);
        self._create_wave_table_voice("svmidi.ensemble5", "SV.ChoirAah", 21, 36, 3, 3, 32, 15, 6, 1);
        self._create_wave_table_voice("svmidi.ensemble6", "SV.VoiceOoh", 11, 24, 3, 3, 32, 15, 6, 1);
        self._create_wave_table_voice("svmidi.ensemble7", "SV.SynVoice", 12, 24, 3, 3, 32, 15, 6, 1);
        self._create_wave_table_voice("svmidi.ensemble8", "SV.Orch.Hit", 26, 40, 32, 24, 32, 8, 2, -3);

        self._create_wave_table_voice("svmidi.brass1", "SV.Trumpet", 10, 38, 32, 8, 32, 3, 0, 1);
        self._create_wave_table_voice("svmidi.brass2", "SV.Trombone", 10, 30, 44, 8, 32, 1, 3, 1);
        self._create_wave_table_voice("svmidi.brass3", "SV.Tuba", 15, 30, 32, 8, 32, 1, 4, 1);
        self._create_wave_table_voice("svmidi.brass4", "SV.Mute.Trp", 18, 32, 44, 8, 32, 1, 4, 1);
        self._create_wave_table_voice("svmidi.brass5", "SV.Fr.Horn", 11, 32, 44, 8, 32, 1, 4, 1);
        self._create_wave_table_voice("svmidi.brass6", "SV.BrasSect", 19, 32, 44, 8, 32, 1, 4, 1);
        self._create_wave_table_voice("svmidi.brass7", "SV.SynBras1", 2, 36, 28, 8, 32, 2, 2, 1);
        self._create_wave_table_voice("svmidi.brass8", "SV.SynBras2", 13, 28, 32, 8, 32, 2, 2, 1);

        self._create_wave_table_voice("svmidi.reed1", "SV.SprnoSax", 6, 32, 44, 8, 32, 1, 2, 1);
        self._create_wave_table_voice("svmidi.reed2", "SV.Alto Sax", 5, 32, 44, 8, 32, 1, -2, 1);
        self._create_wave_table_voice("svmidi.reed3", "SV.TenorSax", 10, 32, 44, 8, 32, 1, 2, 1);
        self._create_wave_table_voice("svmidi.reed4", "SV.Bari.Sax", 10, 32, 44, 8, 32, 1, 2, 1);
        self._create_wave_table_voice("svmidi.reed5", "SV.Oboe", 21, 32, 44, 8, 32, 1, 2, 1);
        self._create_wave_table_voice("svmidi.reed6", "SV.Eng.Horn", 3, 32, 44, 8, 32, 1, 2, 1);
        self._create_wave_table_voice("svmidi.reed7", "SV.Bassoon", 3, 32, 44, 8, 32, 1, 2, 1);
        self._create_wave_table_voice("svmidi.reed8", "SV.Clarinet", 12, 32, 44, 8, 32, 1, 2, 1);

        self._create_wave_table_voice("svmidi.pipe1", "SV.Piccolo", 9, 32, 44, 8, 32, 2, 0, 1);
        self._create_wave_table_voice("svmidi.pipe2", "SV.Flute", 3, 28, 36, 8, 32, 3, -2, 1);
        self._create_wave_table_voice("svmidi.pipe3", "SV.Recorder", 8, 32, 44, 8, 32, 2, 0, 1);
        self._create_wave_table_voice("svmidi.pipe4", "SV.PanFlute", 12, 32, 44, 8, 32, 2, 0, 2);
        self._create_wave_table_voice("svmidi.pipe5", "SV.Bottle", 21, 28, 36, 8, 32, 3, 0, 0);
        self._create_wave_table_voice("svmidi.pipe6", "SV.Shakhchi", 18, 28, 36, 8, 32, 3, 0, 0);
        self._create_wave_table_voice("svmidi.pipe7", "SV.Whistle", 3, 28, 36, 8, 32, 4, 0, 2);
        self._create_wave_table_voice("svmidi.pipe8", "SV.Ocarina", 1, 32, 36, 8, 32, 4, 0, 2);

        self._create_wave_table_voice("svmidi.lead1", "SV.SquareLd", 14, 63, 0, 0, 32, 15, 7, 1);
        self._create_wave_table_voice("svmidi.lead2", "SV.Saw.Lead", 6, 63, 0, 0, 32, 15, 6, 1);
        self._create_wave_table_voice("svmidi.lead3", "SV.CaliopLd", 11, 63, 32, 0, 32, 3, 0, 1);
        self._create_wave_table_voice("svmidi.lead4", "SV.ChiffLd", 18, 63, 32, 0, 32, 3, 0, 1);
        self._create_wave_table_voice("svmidi.lead5", "SV.CharanLd", 17, 63, 32, 0, 32, 3, 1, 1);
        self._create_wave_table_voice("svmidi.lead6", "SV.Voice Ld", 19, 63, 32, 0, 32, 3, 4, 1);
        self._create_wave_table_voice("svmidi.lead7", "SV.Fifth Ld", 18, 63, 0, 0, 32, 15, 8, 0);
        self._create_wave_table_voice("svmidi.lead8", "SV.Bass &Ld", 17, 63, 0, 0, 32, 15, 7, 0);

        self._create_wave_table_voice("svmidi.pad1", "SV.NewAgePd", 27, 40, 12, 0, 28, 6, 4, 1);
        self._create_wave_table_voice("svmidi.pad2", "SV.Warm Pad", 23, 24, 12, 0, 24, 6, 4, 1);
        self._create_wave_table_voice("svmidi.pad3", "SV.PolySyPd", 2, 28, 12, 0, 28, 6, 4, 1);
        self._create_wave_table_voice("svmidi.pad4", "SV.ChoirPad", 11, 24, 12, 0, 28, 6, 4, 1);
        self._create_wave_table_voice("svmidi.pad5", "SV.BowedPad", 1, 16, 12, 0, 24, 6, 4, 1);
        self._create_wave_table_voice("svmidi.pad6", "SV.MetalPad", 20, 24, 12, 0, 24, 6, 4, 1);
        self._create_wave_table_voice("svmidi.pad7", "SV.Halo Pad", 21, 24, 12, 0, 28, 6, 4, 1);
        self._create_wave_table_voice("svmidi.pad8", "SV.SweepPad", 17, 16, 12, 0, 24, 6, 4, 1);

        self._create_wave_table_voice("svmidi.fx1", "SV.Rain", 3, 28, 12, 0, 24, 6, 4, 1);
        self._create_wave_table_voice("svmidi.fx2", "SV.SoundTrk", 18, 24, 12, 0, 24, 6, 4, 1);
        self._create_wave_table_voice("svmidi.fx3", "SV.Crystal", 28, 40, 20, 0, 24, 4, 2, 1);
        self._create_wave_table_voice("svmidi.fx4", "SV.Atmosphr", 27, 40, 20, 0, 24, 4, 4, 1);
        self._create_wave_table_voice("svmidi.fx5", "SV.Bright", 21, 36, 20, 0, 24, 4, 4, 1);
        self._create_wave_table_voice("svmidi.fx6", "SV.Goblins", 27, 32, 20, 0, 24, 4, 4, 1);
        self._create_wave_table_voice("svmidi.fx7", "SV.Echoes", 13, 36, 20, 12, 24, 3, 4, 1);
        self._create_wave_table_voice("svmidi.fx8", "SV.Sci-Fi", 28, 32, 20, 0, 24, 4, 4, 1);

        self._create_wave_table_voice("svmidi.world1", "SV.Sitar", 18, 63, 16, 16, 28, 3, 4, 0);
        self._create_wave_table_voice("svmidi.world2", "SV.Banjo", 3, 63, 36, 20, 28, 2, 4, 1);
        self._create_wave_table_voice("svmidi.world3", "SV.Shamisen", 25, 63, 24, 20, 28, 3, 4, 1);
        self._create_wave_table_voice("svmidi.world4", "SV.Koto", 24, 63, 24, 20, 28, 3, 4, 1);
        self._create_wave_table_voice("svmidi.world5", "SV.Kalimba", 27, 63, 36, 20, 28, 2, 4, 1);
        self._create_wave_table_voice("svmidi.world6", "SV.Bagpipe", 17, 63, 0, 0, 36, 15, 4, 1);
        self._create_wave_table_voice("svmidi.world7", "SV.Fiddle", 10, 63, 0, 0, 36, 15, 4, 1);
        self._create_wave_table_voice("svmidi.world8", "SV.Shanai", 16, 63, 0, 0, 36, 15, 4, 1);

        self._create_wave_table_voice("svmidi.percus1", "SV.TnklBell", 28, 63, 24, 24, 28, 15, 4, 1);
        self._create_wave_table_voice("svmidi.percus2", "SV.Agogo", 27, 63, 24, 24, 28, 15, 4, 1);
        self._create_wave_table_voice("svmidi.percus3", "SV.SteelDrm", 11, 44, 26, 26, 26, 15, 4, 1);
        self._create_wave_table_voice("svmidi.percus4", "SV.WoodBlok", 14, 44, 32, 28, 28, 8, 4, 2);
        self._create_wave_table_voice("svmidi.percus5", "SV.TaikoDrm", 3, 40, 36, 28, 28, 8, 4, 0);
        self._create_single_drum_voice("svmidi.percus6", "SV.MelodTom", 40, 63, 32, 32, 32, 15, 4, -120, 1.0);
        self._create_single_drum_voice("svmidi.percus7", "SV.Syn.Drum", 41, 63, 32, 32, 32, 15, 4, -120, 1.0);
        self._create_single_drum_voice("svmidi.percus8", "SV.RevCymbl", 19, 10, 40, 40, 40, 15, 4, 0, 1.0);

        self._create_wave_table_voice("svmidi.se1", "SV.FretNoiz", 10, 40, 36, 28, 28, 8, 4, 4);
        self._create_wave_table_voice("svmidi.se2", "SV.BrthNoiz", 17, 40, 36, 28, 28, 8, 4, 4);
        self._create_percussive_voice("svmidi.se3", "SV.Seashore", 16, 10, 12, 0, 116, 0);
        self._create_wave_table_voice("svmidi.se4", "SV.Tweet", 18, 40, 36, 28, 28, 8, 4, 4);
        self._create_wave_table_voice("svmidi.se5", "SV.Telphone", 0, 63, 0, 0, 32, 15, 4, 2);
        self._create_percussive_voice("svmidi.se6", "SV.Helicptr", 17, 24, 12, 0, 72, 0);
        self._create_percussive_voice("svmidi.se7", "SV.Applause", 17, 12, 10, 0, 100, 0);
        self._create_percussive_voice("svmidi.se8", "SV.Gunshot", 17, 63, 28, 0, 72, 1);
    }

    /// C++ `_generate_single_drum_voices` — cpp lines 759-824 (wave-9 pass D).
    fn _generate_single_drum_voices(&mut self) {
        self._begin_category("svmidi.drum");
        self._create_single_drum_voice("svmidi.drum24", "Seq Click H", 5, 63, 40, 40, 40, 15, 4, 64, 1.0);
        self._create_single_drum_voice("svmidi.drum25", "Brush Tap", 17, 40, 36, 36, 36, 15, 4, 0, 1.0);
        self._create_single_drum_voice("svmidi.drum26", "Brush Swirl L", 17, 32, 28, 28, 28, 15, 4, 0, 1.0);
        self._create_single_drum_voice("svmidi.drum27", "Brush Slap", 17, 48, 36, 36, 36, 15, 4, 0, 1.0);
        self._create_single_drum_voice("svmidi.drum28", "Brush Swirl H", 17, 48, 28, 28, 28, 15, 4, 0, 1.0);
        self._create_single_drum_voice("svmidi.drum29", "Snare Roll", 16, 28, 24, 24, 24, 15, 4, 0, 1.0);
        self._create_single_drum_voice("svmidi.drum30", "Castanet", 5, 48, 40, 40, 40, 15, 4, 0, 4.0);
        self._create_single_drum_voice("svmidi.drum31", "Snare L", 16, 63, 34, 24, 24, 8, 0, 0, 0.25);
        self._create_single_drum_voice("svmidi.drum32", "Sticks", 1, 63, 40, 28, 28, 8, 6, 0, 4.0);
        self._create_single_drum_voice("svmidi.drum33", "Bass Drum L", 5, 63, 40, 28, 38, 8, 0, -128, 0.5);
        self._create_single_drum_voice("svmidi.drum34", "Open Rim Shot", 16, 63, 36, 28, 28, 6, 6, 0, 1.0);
        self._create_single_drum_voice("svmidi.drum35", "Bass Drum M", 5, 63, 40, 28, 38, 8, 0, -128, 0.5);
        self._create_single_drum_voice("svmidi.drum36", "Bass Drum H", 5, 63, 40, 28, 38, 8, 0, -128, 0.5);
        self._create_single_drum_voice("svmidi.drum37", "Side Stick", 16, 63, 40, 28, 28, 8, 4, 0, 1.0);
        self._create_single_drum_voice("svmidi.drum38", "Snare M", 17, 63, 34, 24, 24, 4, 0, 0, 0.35);
        self._create_single_drum_voice("svmidi.drum39", "Hand Clap", 17, 63, 38, 38, 38, 15, 6, 128, 1.0);
        self._create_single_drum_voice("svmidi.drum40", "Snare H", 17, 63, 34, 28, 28, 4, 0, 0, 0.5);
        self._create_single_drum_voice("svmidi.drum41", "Floor Tom L", 44, 63, 32, 32, 32, 15, 0, -200, 0.2);
        self._create_single_drum_voice("svmidi.drum42", "Hi-Hat Closed", 19, 63, 40, 40, 40, 15, 4, 0, 1.0);
        self._create_single_drum_voice("svmidi.drum43", "Floor Tom H", 44, 63, 32, 32, 32, 15, 0, -200, 0.25);
        self._create_single_drum_voice("svmidi.drum44", "Hi-Hat Pedal", 19, 40, 40, 40, 40, 15, 5, 0, 1.0);
        self._create_single_drum_voice("svmidi.drum45", "Low Tom", 44, 63, 32, 32, 32, 15, 0, -200, 0.375);
        self._create_single_drum_voice("svmidi.drum46", "Hi-Hat Open", 19, 63, 36, 16, 16, 6, 4, 0, 1.0);
        self._create_single_drum_voice("svmidi.drum47", "Mid Tom L", 44, 63, 32, 32, 32, 15, 0, -200, 0.5);
        self._create_single_drum_voice("svmidi.drum48", "Mid Tom H", 44, 63, 32, 32, 32, 15, 0, -200, 0.75);
        self._create_single_drum_voice("svmidi.drum49", "Crash Cymbal 1", 19, 63, 30, 24, 24, 2, 6, 0, 1.0);
        self._create_single_drum_voice("svmidi.drum50", "High Tom", 44, 63, 32, 32, 32, 15, 0, -200, 1.0);
        self._create_single_drum_voice("svmidi.drum51", "Ride Cymbal 1", 18, 63, 40, 28, 28, 8, 6, 0, 2.5);
        self._create_single_drum_voice("svmidi.drum52", "Chinese Cymbal", 16, 63, 32, 24, 24, 3, 6, 0, 1.0);
        self._create_single_drum_voice("svmidi.drum53", "Ride Cymbal Cup", 18, 63, 36, 24, 24, 6, 4, 0, 1.0);
        self._create_single_drum_voice("svmidi.drum54", "Tambourine", 18, 63, 32, 32, 32, 15, 6, 0, 4.0);
        self._create_single_drum_voice("svmidi.drum55", "Splash Cymbal", 17, 63, 32, 24, 24, 3, 6, 0, 1.0);
        self._create_single_drum_voice("svmidi.drum56", "Cowbell", 18, 63, 36, 36, 36, 15, 4, 0, 1.0);
        self._create_single_drum_voice("svmidi.drum57", "Crash Cymbal 2", 19, 63, 30, 24, 24, 2, 6, 0, 1.0);
        self._create_single_drum_voice("svmidi.drum58", "Vibraslap", 18, 63, 36, 36, 36, 15, 4, 0, 1.0);
        self._create_single_drum_voice("svmidi.drum59", "Ride Cymbal 2", 18, 63, 40, 28, 28, 8, 6, 0, 1.5);
        self._create_single_drum_voice("svmidi.drum60", "Bongo H", 40, 63, 36, 36, 36, 15, 4, -32, 1.5);
        self._create_single_drum_voice("svmidi.drum61", "Bongo L", 40, 63, 36, 36, 36, 15, 4, -32, 1.0);
        self._create_single_drum_voice("svmidi.drum62", "Conga H Mute", 1, 63, 40, 40, 40, 15, 4, -200, 4.0);
        self._create_single_drum_voice("svmidi.drum63", "Conga H Open", 45, 63, 36, 36, 36, 15, 4, -32, 1.5);
        self._create_single_drum_voice("svmidi.drum64", "Conga L", 45, 63, 36, 36, 36, 15, 4, -32, 1.0);
        self._create_single_drum_voice("svmidi.drum65", "Timbale H", 25, 63, 36, 36, 36, 15, 4, -64, 2.0);
        self._create_single_drum_voice("svmidi.drum66", "Timbale L", 25, 63, 36, 36, 36, 15, 4, -64, 1.5);
        self._create_single_drum_voice("svmidi.drum67", "Agogo H", 7, 63, 38, 38, 38, 15, 4, -64, 1.5);
        self._create_single_drum_voice("svmidi.drum68", "Agogo L", 7, 63, 38, 38, 38, 15, 4, -64, 1.0);
        self._create_single_drum_voice("svmidi.drum69", "Cabasa", 17, 36, 38, 38, 38, 15, 5, 0, 1.0);
        self._create_single_drum_voice("svmidi.drum70", "Maracas", 19, 32, 36, 36, 36, 15, 6, 0, 1.0);
        self._create_single_drum_voice("svmidi.drum71", "Samba Whistle H", 5, 32, 36, 8, 36, 2, 7, 0, 6.1);
        self._create_single_drum_voice("svmidi.drum72", "Samba Whistle L", 5, 32, 36, 8, 36, 2, 7, 0, 5.1);
        self._create_single_drum_voice("svmidi.drum73", "Guiro Short", 17, 63, 40, 32, 40, 4, 4, 0, 1.0);
        self._create_single_drum_voice("svmidi.drum74", "Guiro Long", 17, 63, 40, 24, 40, 4, 4, 0, 1.0);
        self._create_single_drum_voice("svmidi.drum75", "Claves", 5, 63, 48, 40, 40, 15, 4, 8, 1.0);
        self._create_single_drum_voice("svmidi.drum76", "Wood Block H", 5, 63, 40, 40, 40, 15, 4, -128, 4.1);
        self._create_single_drum_voice("svmidi.drum77", "Wood Block L", 5, 63, 40, 40, 40, 15, 4, -128, 3.1);
        self._create_single_drum_voice("svmidi.drum78", "Cuica Mute", 42, 63, 32, 32, 36, 15, 4, 32, 0.8);
        self._create_single_drum_voice("svmidi.drum79", "Cuica Open", 42, 63, 32, 32, 36, 15, 4, 32, 1.1);
        self._create_single_drum_voice("svmidi.drum80", "Triangle Mute", 7, 63, 44, 44, 44, 6, 2, 0, 7.5);
        self._create_single_drum_voice("svmidi.drum81", "Triangle Open", 7, 63, 38, 16, 16, 6, 2, 0, 7.5);
        self._create_single_drum_voice("svmidi.drum82", "Shaker", 19, 40, 36, 36, 36, 15, 4, 0, 1.0);
        self._create_single_drum_voice("svmidi.drum83", "Jingle Bell", 7, 63, 38, 16, 16, 6, 4, 0, 7.5);
        self._create_single_drum_voice("svmidi.drum84", "Bell Tree", 18, 63, 38, 16, 16, 6, 4, 0, 4.5);
    }

    /// C++ `_create_basic_voice`.
    pub fn _create_basic_voice(&mut self, p_key: &str, p_name: &str, p_channel_num: i32) {
        let voice = Rc::new(RefCell::new(SiONVoice::new(
            enums::MODULE_GENERIC_PG,
            p_channel_num,
            63,
            63,
            0,
            -1,
            0,
            0,
        )));

        voice.borrow_mut().set_name(p_name.to_string());
        self._register_voice(p_key, voice);
    }

    /// C++ `_create_percussive_voice` (header defaults: cutoff=128,
    /// resonance=0 — passed explicitly at Rust call sites).
    #[allow(clippy::too_many_arguments)]
    pub fn _create_percussive_voice(
        &mut self,
        p_key: &str,
        p_name: &str,
        p_wave_shape: i32,
        p_attack_rate: i32,
        p_release_rate: i32,
        p_release_sweep: i32,
        p_cutoff: i32,
        p_resonance: i32,
    ) {
        let voice = Rc::new(RefCell::new(SiONVoice::new(
            enums::MODULE_GENERIC_PG,
            p_wave_shape,
            p_attack_rate,
            p_release_rate,
            0,
            -1,
            0,
            0,
        )));

        if p_attack_rate == 63 {
            voice.borrow_mut().voice.borrow_mut().default_gate_time = 0.0;
        } else {
            let op = voice
                .borrow()
                .voice
                .borrow()
                .channel_params
                .borrow()
                .get_operator_params(0)
                .expect("operator params")
                .clone();
            {
                let mut op = op.borrow_mut();
                op.set_decay_rate(p_release_rate);
                op.set_sustain_rate(p_release_rate);
                op.set_release_rate(p_release_rate);
                op.set_sustain_level(15);
            }
        }

        voice.borrow_mut().voice.borrow_mut().release_sweep = p_release_sweep;
        voice.borrow_mut().set_filter_envelope(
            0,
            p_cutoff,
            p_resonance,
            0,
            0,
            0,
            0,
            128,
            64,
            32,
            128,
        );

        voice.borrow_mut().set_name(p_name.to_string());
        self._register_voice(p_key, voice);
    }

    /// C++ `_create_analog_voice` (header defaults: wave shapes / balance /
    /// pitch diff = 0 — passed explicitly at Rust call sites).
    #[allow(clippy::too_many_arguments)]
    pub fn _create_analog_voice(
        &mut self,
        p_key: &str,
        p_name: &str,
        p_connection_type: i32,
        p_wave_shape1: i32,
        p_wave_shape2: i32,
        p_balance: i32,
        p_pitch_diff: i32,
    ) {
        let voice = Rc::new(RefCell::new(SiONVoice::default_voice()));
        voice
            .borrow_mut()
            .set_analog_like(p_connection_type, p_wave_shape1, p_wave_shape2, p_balance, p_pitch_diff);

        voice.borrow_mut().set_name(p_name.to_string());
        self._register_voice(p_key, voice);
    }

    /// C++ `_create_opn_voice`.
    pub fn _create_opn_voice(&mut self, p_key: &str, p_name: &str, p_params: Vec<i32>) {
        let voice = Rc::new(RefCell::new(SiONVoice::default_voice()));
        voice.borrow_mut().set_params_opn(p_params);

        voice.borrow_mut().set_name(p_name.to_string());
        self._register_voice(p_key, voice);
    }

    /// C++ `_create_ma3_voice`.
    pub fn _create_ma3_voice(&mut self, p_key: &str, p_name: &str, p_params: Vec<i32>) {
        let voice = Rc::new(RefCell::new(SiONVoice::default_voice()));
        voice.borrow_mut().set_params_ma3(p_params);

        voice.borrow_mut().set_name(p_name.to_string());
        self._register_voice(p_key, voice);
    }

    /// C++ `_create_wave_table_voice` (header default: multiple=1 — passed
    /// explicitly at Rust call sites).
    #[allow(clippy::too_many_arguments)]
    pub fn _create_wave_table_voice(
        &mut self,
        p_key: &str,
        p_name: &str,
        p_wave_shape: i32,
        p_attack_rate: i32,
        p_decay_rate: i32,
        p_sustain_rate: i32,
        p_release_rate: i32,
        p_sustain_level: i32,
        p_total_level: i32,
        p_multiple: i32,
    ) {
        let wave_table = self.wave_tables[p_wave_shape as usize].clone();
        let voice = Rc::new(RefCell::new(SiONVoice::new(
            enums::MODULE_SCC,
            p_wave_shape,
            63,
            63,
            0,
            -1,
            0,
            0,
        )));
        voice.borrow_mut().voice.borrow_mut().wave_data =
            Some(Rc::new(wave_table) as Rc<dyn Any>);
        voice.borrow_mut().set_envelope(
            p_attack_rate,
            p_decay_rate,
            p_sustain_rate,
            p_release_rate,
            p_sustain_level,
            p_total_level + 4,
        );
        voice
            .borrow()
            .voice
            .borrow()
            .channel_params
            .borrow()
            .get_operator_params(0)
            .expect("operator params")
            .borrow_mut()
            .set_multiple(p_multiple);

        voice.borrow_mut().set_name(p_name.to_string());
        self._register_voice(p_key, voice);
    }

    /// C++ `_create_single_drum_voice` (header defaults: release_sweep=0,
    /// fine_multiple=1 — passed explicitly at Rust call sites).
    #[allow(clippy::too_many_arguments)]
    pub fn _create_single_drum_voice(
        &mut self,
        p_key: &str,
        p_name: &str,
        p_wave_shape: i32,
        p_attack_rate: i32,
        p_decay_rate: i32,
        p_sustain_rate: i32,
        p_release_rate: i32,
        p_sustain_level: i32,
        p_total_level: i32,
        p_release_sweep: i32,
        p_fine_multiple: f64,
    ) {
        let voice = Rc::new(RefCell::new(SiONVoice::new(
            enums::MODULE_GENERIC_PG,
            p_wave_shape,
            63,
            63,
            0,
            -1,
            0,
            0,
        )));
        voice.borrow_mut().set_envelope(
            p_attack_rate,
            p_decay_rate,
            p_sustain_rate,
            p_release_rate,
            p_sustain_level,
            p_total_level,
        );
        voice
            .borrow()
            .voice
            .borrow()
            .channel_params
            .borrow()
            .get_operator_params(0)
            .expect("operator params")
            .borrow_mut()
            .set_fine_multiple((p_fine_multiple * 128.0) as i32);
        voice.borrow_mut().voice.borrow_mut().release_sweep = p_release_sweep;

        if p_release_sweep != 0 {
            voice.borrow_mut().voice.borrow_mut().default_gate_time = 0.0;
        }

        voice.borrow_mut().set_name(p_name.to_string());
        self._register_voice(p_key, voice);
    }

    /// C++ `_begin_category`. Quirk kept: the `Vec::clone` stored in
    /// `category_map` is a snapshot that never sees later `_register_voice`
    /// pushes (C++ `List` value-semantics copy).
    pub fn _begin_category(&mut self, p_key: &str) {
        err_fail_cond!(
            self.category_map.contains_key(p_key),
            "_category_map.has(p_key)"
        );

        let category: Vec<Rc<RefCell<SiONVoice>>> = Vec::new();
        self.current_category = category.clone();
        self.category_map.insert(p_key.to_string(), category);
    }

    /// C++ `_register_voice`.
    pub fn _register_voice(&mut self, p_key: &str, p_voice: Rc<RefCell<SiONVoice>>) {
        err_fail_cond!(self.voice_map.contains_key(p_key), "_voice_map.has(p_key)");

        self.current_category.push(p_voice.clone());
        self.voice_map.insert(p_key.to_string(), p_voice);
    }

    /// C++ `_register_wave_table`.
    pub fn _register_wave_table(&mut self, p_wavelet: Vec<i32>) {
        self.wave_tables.push(Rc::new(RefCell::new(SiopmWaveTable::new(
            p_wavelet,
            enums::PITCH_TABLE_OPM,
        ))));
    }

    /// C++ static `generate_voices(p_flags)`.
    pub fn generate_voices(p_flags: u32) -> SiONVoicePresetUtil {
        let mut instance = SiONVoicePresetUtil::default();
        instance._generate_voices(p_flags);
        instance
    }

    /// C++ `get_voice_preset_keys()` — `HashMap` iteration order (C++ order
    /// is likewise unspecified).
    pub fn get_voice_preset_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = Vec::new();

        for key in self.voice_map.keys() {
            keys.push(key.clone());
        }

        keys
    }

    /// C++ `get_voice_preset(p_key)`.
    pub fn get_voice_preset(&self, p_key: &str) -> Option<Rc<RefCell<SiONVoice>>> {
        cpp_err_fail_cond_v_msg!(
            !self.voice_map.contains_key(p_key),
            "!_voice_map.has(p_key)",
            None,
            "nullptr",
            format!("SiONVoicePresetUtil: Nonexistent voice preset '{}'.", p_key)
        );

        self.voice_map.get(p_key).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `INCLUDE_DEFAULT` registers only the 16 default voices: bare keys
    /// (cpp `sion_voice_preset_util.cpp:43-62`), the `"default"` category,
    /// and no other family.
    #[test]
    fn generate_default_only() {
        let util = SiONVoicePresetUtil::generate_voices(INCLUDE_DEFAULT);
        assert!(!util.voice_map.is_empty());
        assert!(util.category_map.contains_key("default"));

        let keys = util.get_voice_preset_keys();
        assert!(keys.iter().any(|k| k == "sine"));
        assert!(keys.iter().any(|k| k == "bassdrumm"));
        assert!(keys.iter().any(|k| k == "dualsaw"));
        assert!(util.get_voice_preset("sine").is_some());

        assert!(!util.voice_map.contains_key("valsound.bass1"));
        assert!(!util.voice_map.contains_key("svmidi.piano1"));
    }

    /// `INCLUDE_ALL` yields the documented ~650 presets (>600) and a
    /// representative key plus its `_begin_category` entry for every family.
    #[test]
    fn generate_all_counts() {
        let util = SiONVoicePresetUtil::generate_voices(INCLUDE_ALL);
        let keys = util.get_voice_preset_keys();
        assert!(keys.len() > 600, "keys.len() = {}", keys.len());

        for representative in [
            "sine",
            "valsound.bass1",
            "midi.piano1",
            "midi.drum24",
            "svmidi.piano1",
            "svmidi.drum24",
        ] {
            assert!(
                keys.iter().any(|k| k == representative),
                "missing key {}",
                representative
            );
        }

        for category in [
            "default",
            "valsound.bass",
            "valsound.bell",
            "valsound.brass",
            "valsound.guitar",
            "valsound.lead",
            "valsound.percus",
            "valsound.piano",
            "valsound.se",
            "valsound.special",
            "valsound.strpad",
            "valsound.wind",
            "valsound.world",
            "midi",
            "midi.drum",
            "svmidi",
            "svmidi.drum",
        ] {
            assert!(
                util.category_map.contains_key(category),
                "missing category {}",
                category
            );
        }
    }

    /// One `_create_opn_voice` key (`valsound.bass1`) and one
    /// `_create_wave_table_voice` key (`svmidi.piano1`) round-trip through
    /// `get_mml` to a non-empty `#@` string; the wave-table voice also
    /// carries a downcastable `SiopmWaveTable` payload.
    #[test]
    fn preset_roundtrip_mml() {
        let util = SiONVoicePresetUtil::generate_voices(INCLUDE_ALL);

        let opn = util.get_voice_preset("valsound.bass1").expect("valsound.bass1");
        let mml = opn.borrow().get_mml(0, enums::CHIP_SIOPM, false);
        assert!(mml.starts_with("#@"), "mml = {}", mml);
        assert!(mml.len() > 3);

        let wave_voice = util.get_voice_preset("svmidi.piano1").expect("svmidi.piano1");
        let mml = wave_voice.borrow().get_mml(0, enums::CHIP_SIOPM, false);
        assert!(mml.starts_with("#@"), "mml = {}", mml);
        assert!(mml.len() > 3);

        let wave_data = {
            let outer = wave_voice.borrow();
            let inner = outer.voice.borrow();
            inner.wave_data.clone()
        };
        let wave_data = wave_data.expect("wave data");
        assert!(wave_data.downcast_ref::<Rc<RefCell<SiopmWaveTable>>>().is_some());
    }

    /// Unknown keys return `None` (the compat error line prints to the test
    /// output sink — acceptable).
    #[test]
    fn missing_key_returns_none_and_errors() {
        let util = SiONVoicePresetUtil::generate_voices(INCLUDE_DEFAULT);
        assert!(util.get_voice_preset("nope.nope").is_none());
    }

    /// `INCLUDE_WAVETABLE` alone registers exactly the 29 cpp wave tables
    /// (cpp 583-611), emits `svmidi.*` but no single-drum keys, and
    /// `svmidi.pipe4` (`p_multiple=2`) lands `fine_multiple=256` on op 0.
    #[test]
    fn wave_table_voice_fine_multiple_spot() {
        let util = SiONVoicePresetUtil::generate_voices(INCLUDE_WAVETABLE);
        assert_eq!(util.wave_tables.len(), 29);

        assert!(util.get_voice_preset("svmidi.piano1").is_some());
        assert!(!util.voice_map.contains_key("svmidi.drum24"));

        let pipe = util.get_voice_preset("svmidi.pipe4").expect("svmidi.pipe4");
        let (fine, mult) = {
            let outer = pipe.borrow();
            let inner = outer.voice.borrow();
            let params = inner.channel_params.clone();
            let op = params
                .borrow()
                .get_operator_params(0)
                .expect("operator params");
            let op = op.borrow();
            (op.get_fine_multiple(), op.get_multiple())
        };
        assert_eq!(fine, 2 * 128);
        assert_eq!(mult, 2);
    }
}
