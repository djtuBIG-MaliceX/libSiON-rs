//! Port of `libSiON-cpp/src/utils/translator_util.{h,cpp}`.
//!
//! The C++ class is a stateless all-statics namespace, so it becomes unit
//! [`TranslatorUtil`] with associated functions (no `instance()` singleton
//! exists in C++). `Ref<SiOPMChannelParams>` becomes `&mut ChannelParams` /
//! `&ChannelParams` (the C++ friend-write pattern), `Ref<SiOPMOperatorParams>`
//! elements are the ported `Rc<RefCell<OperatorParams>>` slots.
//!
//! `parse_voice_setting`, `get_voice_setting_as_mml` and
//! `parse_pcm_voice` landed with wave-7c (system-command path).
//! `extract_system_command` has no C++ definition (dead declaration — not
//! ported). `SiMMLRefTable` is a wave-7 module, so the four
//! `algorithm_*` [4][16] tables are provisionally local copies here.
//!
//! `parse_wav` / `parse_wavb` are MML *text* table/hex parsers producing
//! f64 wave data (there is no RIFF/`.wav` binary parsing in C++).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::LazyLock;

use regex::{Captures, Regex};

use crate::chip::params::channel_params::{ChannelParams, MAX_OPERATORS};
use crate::chip::wave::pcm_data::SiopmWavePcmData;
use crate::chip::wave::pcm_table::SiopmWavePcmTable;
use crate::chip::wave::sampler_data::SiopmWaveSamplerData;
use crate::chip::wave::sampler_table::SiopmWaveSamplerTable;
use crate::err_fail_cond_msg;
use crate::err_fail_cond_v;
use crate::sequencer::ref_table::{ALGORITHM_MA3, ALGORITHM_OPL, ALGORITHM_OPX, ALGORITHM_OPM};
use crate::math::clampf;
use crate::sample_data::SampleData;
use crate::sion_enums::{PULSE_CUSTOM, PULSE_MA3_SINE};
use crate::utils::string::{
    hex_to_int, itos, literal_replace_all, pad_zeros, split_string_by_regex, substr, to_float,
    to_int,
};

/// C++ `ERR_FAIL_V_MSG(retval, msg)` — message first (golden line), the
/// `#m_retval` stringification is passed as `$retval_text`.
macro_rules! err_fail_v_msg {
    ($retval:expr, $retval_text:expr, $msg:expr) => {{
        $crate::error::err_print_body(
            &format!(
                "{}\nMethod/function failed. Returning: {}",
                $msg, $retval_text
            ),
            false,
        );
        return $retval;
    }};
}

/// C++ `ERR_FAIL_INDEX_V_MSG(index, size, retval, msg)` — message first; the
/// compat macro never mentions the return value.
macro_rules! err_fail_index_v_msg {
    ($idx:expr, $i_text:expr, $size:expr, $size_text:expr, $retval:expr, $msg:expr) => {
        if $idx < 0 || $idx >= $size {
            $crate::error::err_print_body(
                &format!(
                    "{}\nIndex {} = {} is out of bounds ({} = {}).",
                    $msg, $i_text, $idx, $size_text, $size
                ),
                false,
            );
            return $retval;
        }
    };
}

/// C++ `ERR_FAIL_COND_V_MSG(cond, retval, msg)` for the
/// `MMLTableNumbers`-returning branches — custom message first, the
/// `#m_retval` stringification is always `parsed_table` there.
macro_rules! err_fail_table_cond_v_msg {
    ($cond:expr, $cond_text:expr, $retval:expr, $msg:expr) => {
        if $cond {
            $crate::error::err_print_body(
                &format!(
                    "{}\nCondition \"{}\" is true. Returning: parsed_table",
                    $msg, $cond_text
                ),
                false,
            );
            return $retval;
        }
    };
}

// C++ `SiMMLRefTable::algorithm_*` tables (`sequencer/simml_ref_table.h`,
// wave-7 module). Provisional local copies until the sequencer-ref-table wave
// moves them to `sequencer::ref_table` (see docs/PENDING.md).





/// C++ `RegExMatch::get_string(p_group)` — empty string when the group did
/// not participate in the match (PCRE2 `PCRE2_UNSET`).
fn group_str<'c>(caps: &'c Captures<'_>, p_group: usize) -> &'c str {
    caps.get(p_group).map_or("", |m| m.as_str())
}

/// `SinglyLinkedList<int>` (`src/templates/singly_linked_list.h`) specialized
/// for `parse_table_numbers` / `parse_wav`: the only operations that file
/// exercises are `append`, the `front`/`get`/`next` cursor and `loop()`
/// (rewire the last element's `next` onto a repeat point). Elements are the
/// `Vec` slots, so element pointers become indices.
#[derive(Debug)]
pub struct SinglyLinkedList {
    values: Vec<i32>,
    cursor: Option<usize>,
    wrap: Option<usize>,
}

impl SinglyLinkedList {
    /// Stand-in for `new SinglyLinkedList<int>` (empty, no cursor).
    pub fn new_empty() -> Self {
        SinglyLinkedList {
            values: Vec::new(),
            cursor: None,
            wrap: None,
        }
    }

    /// `SinglyLinkedList(int p_size, T p_value, bool p_ring)` — builds
    /// `p_size` copies of `p_value`, rewires the tail onto the head when
    /// `p_ring` (same `wrap` slot `loop_at` uses), and leaves the cursor at
    /// the head via `front()`. Non-positive sizes fall through to the empty
    /// list, like the C++ early return.
    pub fn new_size_loop(p_size: i32, p_value: i32, p_loop: bool) -> Self {
        let mut list = SinglyLinkedList::new();
        if p_size <= 0 {
            return list;
        }
        for _ in 0..p_size {
            list.append(p_value);
        }
        if p_loop {
            list.wrap = Some(0);
        }
        list.front();
        list
    }

    /// `SinglyLinkedList::size()`.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// `SinglyLinkedList::empty()`.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// `Element::value` at an absolute index, cursor not moved.
    pub fn value_at(&self, p_index: usize) -> i32 {
        self.values[p_index]
    }

    fn new() -> Self {
        SinglyLinkedList {
            values: Vec::new(),
            cursor: None,
            wrap: None,
        }
    }

    /// `SinglyLinkedList::append` — also moves the cursor to the new last
    /// element, like C++ (`_current = _last`).
    pub fn append(&mut self, p_value: i32) {
        self.values.push(p_value);
        self.cursor = Some(self.values.len() - 1);
    }

    /// `SinglyLinkedList::front` — cursor reset is skipped on an empty list.
    pub fn front(&mut self) {
        if self.values.is_empty() {
            return;
        }
        self.cursor = Some(0);
    }

    /// `SinglyLinkedList::get` — the cursor element, `None` for a null
    /// cursor element.
    pub fn get(&self) -> Option<i32> {
        if self.values.is_empty() {
            return None;
        }
        self.cursor.map(|i| self.values[i])
    }

    /// `SinglyLinkedList::next` — wraps when the list loops.
    pub fn next(&mut self) -> Option<i32> {
        if self.values.is_empty() {
            return None;
        }
        let current = self.cursor?;
        self.cursor = self.next_of(current);
        self.cursor.map(|i| self.values[i])
    }

    /// `Element::next()` without moving the cursor.
    pub fn next_of(&self, p_index: usize) -> Option<usize> {
        if p_index + 1 < self.values.len() {
            Some(p_index + 1)
        } else {
            self.wrap
        }
    }

    /// `SinglyLinkedList::loop(p_element)` — `None` loops to the first
    /// element, like the C++ default argument.
    pub fn loop_at(&mut self, p_element: Option<usize>) {
        if self.values.is_empty() {
            return;
        }
        self.wrap = Some(p_element.unwrap_or(0));
    }
}

/// C++ `TranslatorUtil::MMLTableNumbers`.
pub struct MMLTableNumbers {
    pub data: SinglyLinkedList,
    pub length: i32,
    pub repeated: bool,
}

/// C++ `TranslatorUtil::OperatorParamsSizes`. Defaults mirror the C++
/// in-struct initializers.
#[derive(Clone, Copy)]
pub struct OperatorParamsSizes {
    pub pg_type: i32,
    pub total_level: i32,
    pub detune2: i32,
    pub phase: i32,
    pub fixed_pitch: i32,
}

impl Default for OperatorParamsSizes {
    fn default() -> Self {
        OperatorParamsSizes {
            pg_type: 1,
            total_level: 2,
            detune2: 1,
            phase: 1,
            fixed_pitch: 1,
        }
    }
}

/// C++ `TranslatorUtil`.
pub struct TranslatorUtil;

impl TranslatorUtil {
    // `_split_data_string` sanitizers (compiled once; C++ rebuilt the fixed
    // patterns per call, which is not observable).
    fn re_comments() -> &'static Regex {
        static RE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"(?s)/\*.*?\*/|//.*?[\r\n]+").expect("valid regex"));
        &RE
    }

    fn re_cleanup() -> &'static Regex {
        static RE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"^[^\d\-.]+|[^\d\-.]+$").expect("valid regex"));
        &RE
    }

    fn re_spaces() -> &'static Regex {
        static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").expect("valid regex"));
        &RE
    }

    fn re_postfix() -> &'static Regex {
        static RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(\d+)?(\*(-?[\d.]+))?([+-][\d.]+)?").expect("valid regex")
        });
        &RE
    }

    fn re_table() -> &'static Regex {
        static RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(\(\s*([,\-\d\s]+)\)[,\s]*(\d+))|(-?\d+)|(\||\[|\](\d*))")
                .expect("valid regex")
        });
        &RE
    }

    fn split_data_string(
        p_params: &mut ChannelParams,
        p_data_string: &str,
        p_channel_param_count: i32,
        p_operator_param_count: i32,
        p_command: &str,
    ) -> Vec<i32> {
        if p_data_string.is_empty() {
            p_params.set_operator_count(0);
            return Vec::new();
        }

        // Godot's RegEx implementation doesn't support passing global flags, but PCRE2 allows local flags, which we can abuse.
        // (?s) enables single line mode (dot matches newline) for the entire expression.
        let mut sanitized_string = literal_replace_all(Self::re_comments(), p_data_string, "");
        sanitized_string = literal_replace_all(Self::re_cleanup(), &sanitized_string, "");
        let string_data = split_string_by_regex(&sanitized_string, "[^\\d\\-.]+");

        for i in 1..=MAX_OPERATORS {
            if string_data.len() as i32 == (p_channel_param_count + p_operator_param_count * i) {
                p_params.set_operator_count(i);

                let mut data = Vec::with_capacity(string_data.len());
                for piece in &string_data {
                    data.push(to_int(piece) as i32);
                }
                return data;
            }
        }

        err_fail_v_msg!(
            Vec::new(),
            "std::vector<int>()",
            format!(
                "Translator: Invalid parameter count in '{}' (channel: {}, each operator: {}).",
                p_command, p_channel_param_count, p_operator_param_count
            )
        );
    }

    fn check_operator_count(
        p_params: &mut ChannelParams,
        p_data_length: i32,
        p_channel_param_count: i32,
        p_operator_param_count: i32,
        p_command: &str,
    ) {
        let op_count = (p_data_length - p_channel_param_count) / p_operator_param_count;
        err_fail_cond_msg!(
            op_count > MAX_OPERATORS,
            "op_count > SiOPMChannelParams::MAX_OPERATORS",
            format!(
                "Translator: Invalid operator count in '{}' (parameters for: {}, max: {}).",
                p_command, op_count, MAX_OPERATORS
            )
        );
        err_fail_cond_msg!(
            (op_count * p_operator_param_count + p_channel_param_count) != p_data_length,
            "(op_count * p_operator_param_count + p_channel_param_count) != p_data_length",
            format!(
                "Translator: Invalid parameter count in '{}' (total: {}, channel: {}, each operator: {}).",
                p_command, p_data_length, p_channel_param_count, p_operator_param_count
            )
        );

        p_params.set_operator_count(op_count);
    }

    // WARN: Max value must be a bitmask, e.g. 0xFF. In other words, it's power-of-2 minus 1 (1, 3, 7, 15, 31, 63, 127, 255, 511).
    fn sanitize_param_loop(p_value: i32, p_min: i32, p_max: i32, p_label: &str) -> i32 {
        if p_value < p_min || p_value > p_max {
            crate::err_print!(
                "Translator: Parameter '{}' value ({}) is outside of valid range ({} : {}). Value will be looped.",
                p_label,
                p_value,
                p_min,
                p_max
            );
        }

        // Special case when -1 is allowed. Other negative values still loop, which is ehhh...
        // But that's how the original is, so why not. We still report all invalid values here.
        if p_min == -1 && p_value == -1 {
            return p_value;
        }

        p_value & p_max
    }

    fn sanitize_param_clamp(p_value: i32, p_min: i32, p_max: i32, p_label: &str) -> i32 {
        if p_value < p_min || p_value > p_max {
            crate::err_print!(
                "Translator: Parameter '{}' value ({}) is outside of valid range ({} : {}). Value will be clamped.",
                p_label,
                p_value,
                p_min,
                p_max
            );
        }

        p_value.clamp(p_min, p_max)
    }

    fn get_params_algorithm(
        p_algorithms: &[[i32; 16]; 4],
        p_operator_count: i32,
        p_data_value: i32,
        p_max_value: i32,
        p_command: &str,
    ) -> i32 {
        let alg_index = p_operator_count - 1;
        err_fail_index_v_msg!(
            alg_index,
            "alg_index",
            4,
            "4",
            -1,
            format!(
                "Translator: Invalid operator count ({}) for the algorithm in '{}'.",
                p_operator_count, p_command
            )
        );

        // WARN: Max value must be a bitmask, e.g. 0xFF. In other words, it's power-of-2 minus 1 (1, 3, 7, 15, 31, 63, 127, 255, 511).
        let alg_data = Self::sanitize_param_loop(p_data_value, 0, p_max_value, "AL");
        err_fail_index_v_msg!(
            alg_data,
            "alg_data",
            16,
            "16",
            -1,
            format!(
                "Translator: Invalid algorithm parameter {} in '{}'.",
                p_data_value, p_command
            )
        );

        let algorithm = p_algorithms[alg_index as usize][alg_data as usize];
        if algorithm == -1 {
            // ERR_FAIL_COND_V_MSG: custom message is the first ERROR: line.
            crate::error::err_print_body(
                &format!(
                    "Translator: Unsupported algorithm parameter {} in '{}'.\nCondition \"algorithm == -1\" is true. Returning: -1",
                    p_data_value, p_command
                ),
                false,
            );
            return -1;
        }

        algorithm
    }

    fn set_siopm_params_by_array(p_params: &mut ChannelParams, p_data: Vec<i32>) {
        if p_params.operator_count == 0 {
            return;
        }

        // #@ (SiOPM) signature:
        // AL[0-15], FB[0-7], FC[0-3],
        // (WS[0-511], AR[0-63], DR[0-63], SR[0-63], RR[0-63], SL[0-15], TL[0-127], KR[0-3], KL[0-3], ML[0-15], D1[0-7], D2[], AM[0-3], PH[-1-255], FN[0-127]) x operator_count

        p_params.algorithm = Self::sanitize_param_loop(p_data[0], 0, 15, "AL");
        p_params.feedback = Self::sanitize_param_clamp(p_data[1], 0, 7, "FB");
        p_params.feedback_connection = Self::sanitize_param_loop(p_data[2], 0, 3, "FC");

        let mut data_index = 3usize;
        for op_index in 0..p_params.operator_count {
            let op_params = p_params.operator_params[op_index as usize].clone();

            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().set_pulse_generator_type(Self::sanitize_param_loop(v, 0, 511, "WS")); // 1
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().attack_rate = Self::sanitize_param_loop(v, 0, 63, "AR"); // 2
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().decay_rate = Self::sanitize_param_loop(v, 0, 63, "DR"); // 3
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().sustain_rate = Self::sanitize_param_loop(v, 0, 63, "SR"); // 4
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().release_rate = Self::sanitize_param_loop(v, 0, 63, "RR"); // 5
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().sustain_level = Self::sanitize_param_loop(v, 0, 15, "SL"); // 6
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().total_level = Self::sanitize_param_loop(v, 0, 127, "TL"); // 7
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().key_scaling_rate = Self::sanitize_param_loop(v, 0, 3, "KR"); // 8
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().key_scaling_level = Self::sanitize_param_loop(v, 0, 3, "KL"); // 9
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().set_multiple(Self::sanitize_param_loop(v, 0, 15, "ML")); // 10
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().detune1 = Self::sanitize_param_loop(v, 0, 7, "D1"); // 11
            op_params.borrow_mut().detune2 = p_data[data_index]; // 12
            data_index += 1;
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().amplitude_modulation_shift =
                Self::sanitize_param_loop(v, 0, 3, "D2"); // 13
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().initial_phase = Self::sanitize_param_loop(v, -1, 255, "PH"); // 14
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().fixed_pitch =
                Self::sanitize_param_loop(v, 0, 127, "FN") << 6; // 15
        }
    }

    fn set_opl_params_by_array(p_params: &mut ChannelParams, p_data: Vec<i32>) {
        if p_params.operator_count == 0 {
            return;
        }

        // #OPL@ signature:
        // AL[0-3], FB[0-7],
        // (WS[0-31], AR[0-15], DR[0-15], RR[0-15], ET[0,1], SL[0-15], TL[0-63], KR[0,1], KL[0-3], ML[0-15], AM[0-3]) x operator_count

        let algorithm = Self::get_params_algorithm(
            &ALGORITHM_OPL,
            p_params.operator_count,
            p_data[0],
            3,
            "#OPL@",
        );
        if algorithm == -1 {
            return;
        }

        p_params.envelope_frequency_ratio = 133;
        p_params.algorithm = algorithm;
        p_params.feedback = Self::sanitize_param_clamp(p_data[1], 0, 7, "FB");

        let mut data_index = 2usize;
        for op_index in 0..p_params.operator_count {
            let op_params = p_params.operator_params[op_index as usize].clone();

            let v = p_data[data_index];
            data_index += 1;
            let pg_type = PULSE_MA3_SINE + Self::sanitize_param_loop(v, 0, 31, "WS");
            op_params.borrow_mut().set_pulse_generator_type(pg_type); // 1

            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().attack_rate =
                (Self::sanitize_param_loop(v, 0, 15, "AR")) << 2; // 2
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().decay_rate =
                (Self::sanitize_param_loop(v, 0, 15, "DR")) << 2; // 3
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().release_rate =
                (Self::sanitize_param_loop(v, 0, 15, "RR")) << 2; // 4

            // If envelope waveform type is 0 — decaying sound, if it is 1 — sustained sound.
            let v = p_data[data_index];
            data_index += 1;
            let n = Self::sanitize_param_loop(v, 0, 1, "ET");
            let rr = op_params.borrow().release_rate;
            op_params.borrow_mut().sustain_rate = if n != 0 { 0 } else { rr }; // 5
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().sustain_level =
                Self::sanitize_param_loop(v, 0, 15, "SL"); // 6
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().total_level =
                Self::sanitize_param_loop(v, 0, 63, "TL"); // 7
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().key_scaling_rate =
                (Self::sanitize_param_loop(v, 0, 1, "KR")) << 1; // 8
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().key_scaling_level =
                Self::sanitize_param_loop(v, 0, 3, "KL"); // 9

            let v = p_data[data_index];
            data_index += 1;
            let i = Self::sanitize_param_loop(v, 0, 15, "ML");
            op_params.borrow_mut().set_multiple(if i == 11 || i == 13 {
                i - 1
            } else if i == 14 {
                i + 1
            } else {
                i
            }); // 10
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().amplitude_modulation_shift =
                Self::sanitize_param_loop(v, 0, 3, "AM"); // 11
        }
    }

    fn set_opm_params_by_array(p_params: &mut ChannelParams, p_data: Vec<i32>) {
        if p_params.operator_count == 0 {
            return;
        }

        // #OPM@ signature:
        // AL[0-7], FB[0-7],
        // (AR[0-31], DR[0-31], SR[0-31], RR[0-15], SL[0-15], TL[0-127], KR[0-3], ML[0-15], D1[0-7], D2[0-3], AM[0-3]) x operator_count

        let algorithm = Self::get_params_algorithm(
            &ALGORITHM_OPM,
            p_params.operator_count,
            p_data[0],
            7,
            "#OPM@",
        );
        if algorithm == -1 {
            return;
        }

        p_params.algorithm = algorithm;
        p_params.feedback = Self::sanitize_param_clamp(p_data[1], 0, 7, "FB");

        let table = crate::chip::ref_table::instance();
        let dt2_table = table.borrow().dt2_table;

        let mut data_index = 2usize;
        for op_index in 0..p_params.operator_count {
            let op_params = p_params.operator_params[op_index as usize].clone();

            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().attack_rate =
                (Self::sanitize_param_loop(v, 0, 31, "AR")) << 1; // 1
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().decay_rate =
                (Self::sanitize_param_loop(v, 0, 31, "DR")) << 1; // 2
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().sustain_rate =
                (Self::sanitize_param_loop(v, 0, 31, "SR")) << 1; // 3
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().release_rate =
                ((Self::sanitize_param_loop(v, 0, 15, "RR")) << 2) + 2; // 4
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().sustain_level =
                Self::sanitize_param_loop(v, 0, 15, "SL"); // 5
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().total_level =
                Self::sanitize_param_loop(v, 0, 127, "TL"); // 6
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().key_scaling_rate =
                Self::sanitize_param_loop(v, 0, 3, "KR"); // 7
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().set_multiple(Self::sanitize_param_loop(v, 0, 15, "ML")); // 8
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().detune1 = Self::sanitize_param_loop(v, 0, 7, "D1"); // 9

            let v = p_data[data_index];
            data_index += 1;
            let n = Self::sanitize_param_loop(v, 0, 3, "D2");
            op_params.borrow_mut().detune2 = dt2_table[n as usize]; // 10
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().amplitude_modulation_shift =
                Self::sanitize_param_loop(v, 0, 3, "AM"); // 11
        }
    }

    fn set_opn_params_by_array(p_params: &mut ChannelParams, p_data: Vec<i32>) {
        if p_params.operator_count == 0 {
            return;
        }

        // #OPN@ signature:
        // AL[0-7], FB[0-7],
        // (AR[0-31], DR[0-31], SR[0-31], RR[0-15], SL[0-15], TL[0-127], KR[0-3], ML[0-15], D1[0-7], AM[0-3]) x operator_count

        // Note: OPM and OPN share the algo list.
        let algorithm = Self::get_params_algorithm(
            &ALGORITHM_OPM,
            p_params.operator_count,
            p_data[0],
            7,
            "#OPN@",
        );
        if algorithm == -1 {
            return;
        }

        p_params.algorithm = algorithm;
        p_params.feedback = Self::sanitize_param_clamp(p_data[1], 0, 7, "FB");

        let mut data_index = 2usize;
        for op_index in 0..p_params.operator_count {
            let op_params = p_params.operator_params[op_index as usize].clone();

            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().attack_rate =
                (Self::sanitize_param_loop(v, 0, 31, "AR")) << 1; // 1
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().decay_rate =
                (Self::sanitize_param_loop(v, 0, 31, "DR")) << 1; // 2
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().sustain_rate =
                (Self::sanitize_param_loop(v, 0, 31, "SR")) << 1; // 3
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().release_rate =
                ((Self::sanitize_param_loop(v, 0, 15, "RR")) << 2) + 2; // 4
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().sustain_level =
                Self::sanitize_param_loop(v, 0, 15, "SL"); // 5
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().total_level =
                Self::sanitize_param_loop(v, 0, 127, "TL"); // 6
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().key_scaling_rate =
                Self::sanitize_param_loop(v, 0, 3, "KR"); // 7
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().set_multiple(Self::sanitize_param_loop(v, 0, 15, "ML")); // 8
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().detune1 = Self::sanitize_param_loop(v, 0, 7, "D1"); // 9
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().amplitude_modulation_shift =
                Self::sanitize_param_loop(v, 0, 3, "AM"); // 10
        }
    }

    fn set_opx_params_by_array(p_params: &mut ChannelParams, p_data: Vec<i32>) {
        if p_params.operator_count == 0 {
            return;
        }

        // #OPX@ signature:
        // AL[0-15], FB[0-7],
        // (WS[0-7?], AR[0-31], DR[0-31], SR[0-31], RR[0-15], SL[0-15], TL[0-127], KR[0-3], ML[0-15], D1[0-7], D2[], AM[0-3]) x operator_count

        let algorithm = Self::get_params_algorithm(
            &ALGORITHM_OPX,
            p_params.operator_count,
            p_data[0],
            15,
            "#OPX@",
        );
        if algorithm == -1 {
            return;
        }

        // LSB is the flag of feedback connection.
        p_params.algorithm = algorithm & 15;
        p_params.feedback = Self::sanitize_param_clamp(p_data[1], 0, 7, "FB");
        p_params.feedback_connection = if algorithm & 16 != 0 { 1 } else { 0 };

        let mut data_index = 2usize;
        for op_index in 0..p_params.operator_count {
            let op_params = p_params.operator_params[op_index as usize].clone();

            // Standard supported values are in the [0-7] range. Values beyond that are supported for custom waves.
            let wave_shape = p_data[data_index];
            data_index += 1;
            if wave_shape < 8 {
                let pg_type =
                    PULSE_MA3_SINE + Self::sanitize_param_loop(wave_shape, 0, 7, "WS");
                op_params.borrow_mut().set_pulse_generator_type(pg_type); // 1
            } else {
                let pg_type = PULSE_CUSTOM
                    + Self::sanitize_param_clamp(wave_shape - 8, 0, 127, "WS");
                op_params.borrow_mut().set_pulse_generator_type(pg_type); // 1
            }

            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().attack_rate =
                (Self::sanitize_param_loop(v, 0, 31, "AR")) << 1; // 2
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().decay_rate =
                (Self::sanitize_param_loop(v, 0, 31, "DR")) << 1; // 3
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().sustain_rate =
                (Self::sanitize_param_loop(v, 0, 31, "SR")) << 1; // 4
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().release_rate =
                ((Self::sanitize_param_loop(v, 0, 15, "RR")) << 2) + 2; // 5
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().sustain_level =
                Self::sanitize_param_loop(v, 0, 15, "SL"); // 6
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().total_level =
                Self::sanitize_param_loop(v, 0, 127, "TL"); // 7
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().key_scaling_rate =
                Self::sanitize_param_loop(v, 0, 3, "KR"); // 8
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().set_multiple(Self::sanitize_param_loop(v, 0, 15, "ML")); // 9
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().detune1 = Self::sanitize_param_loop(v, 0, 7, "D1"); // 10
            op_params.borrow_mut().detune2 = p_data[data_index]; // 11
            data_index += 1;
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().amplitude_modulation_shift =
                Self::sanitize_param_loop(v, 0, 3, "AM"); // 12
        }
    }

    fn set_ma3_params_by_array(p_params: &mut ChannelParams, p_data: Vec<i32>) {
        if p_params.operator_count == 0 {
            return;
        }

        // #MA@ signature:
        // AL[0-7], FB[0-7],
        // (WS[0-31], AR[0-15], DR[0-15], SR[0-15], RR[0-15], SL[0-15], TL[0-63], KR[0,1], KL[0-3], ML[0-15], D1[0-7], AM[0-3]) x operator_count

        let algorithm = Self::get_params_algorithm(
            &ALGORITHM_MA3,
            p_params.operator_count,
            p_data[0],
            7,
            "#MA@",
        );
        if algorithm == -1 {
            return;
        }

        p_params.envelope_frequency_ratio = 133;
        p_params.algorithm = algorithm;
        p_params.feedback = Self::sanitize_param_clamp(p_data[1], 0, 7, "FB");

        let mut data_index = 2usize;
        for op_index in 0..p_params.operator_count {
            let op_params = p_params.operator_params[op_index as usize].clone();

            let v = p_data[data_index];
            data_index += 1;
            let pg_type = PULSE_MA3_SINE + Self::sanitize_param_loop(v, 0, 31, "WS");
            op_params.borrow_mut().set_pulse_generator_type(pg_type); // 1

            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().attack_rate =
                (Self::sanitize_param_loop(v, 0, 15, "AR")) << 2; // 2
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().decay_rate =
                (Self::sanitize_param_loop(v, 0, 15, "DR")) << 2; // 3
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().sustain_rate =
                (Self::sanitize_param_loop(v, 0, 15, "SR")) << 2; // 4
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().release_rate =
                (Self::sanitize_param_loop(v, 0, 15, "RR")) << 2; // 5
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().sustain_level =
                Self::sanitize_param_loop(v, 0, 15, "SL"); // 6
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().total_level =
                Self::sanitize_param_loop(v, 0, 63, "TL"); // 7
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().key_scaling_rate =
                (Self::sanitize_param_loop(v, 0, 1, "KR")) << 1; // 8
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().key_scaling_level =
                Self::sanitize_param_loop(v, 0, 3, "KL"); // 9

            let v = p_data[data_index];
            data_index += 1;
            let i = Self::sanitize_param_loop(v, 0, 15, "ML");
            op_params.borrow_mut().set_multiple(if i == 11 || i == 13 {
                i - 1
            } else if i == 14 {
                i + 1
            } else {
                i
            }); // 10
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().detune1 = Self::sanitize_param_loop(v, 0, 7, "D1"); // 11
            let v = p_data[data_index];
            data_index += 1;
            op_params.borrow_mut().amplitude_modulation_shift =
                Self::sanitize_param_loop(v, 0, 3, "AM"); // 12
        }
    }

    fn set_al_params_by_array(p_params: &mut ChannelParams, p_data: Vec<i32>) {
        p_params.set_operator_count(2);
        p_params.set_analog_like(true);

        // #AL@ signature:
        // CN[0-2], W1[0-511], W2[0-511], BL[-64-+64], DT[]
        // AR[0-63], DR[0-63], SL[0-15], RR[0-63]

        // Can't use _sanitize_param_loop here because 2 is not a valid number for the max value. So hack something ad-hoc together instead.
        if p_data[0] < 0 || p_data[0] > 2 {
            crate::err_print!(
                "Translator: Parameter 'CN' value ({}) is outside of valid range ({} : {}).",
                p_data[0],
                0,
                2
            );
            p_params.algorithm = 0;
        } else {
            p_params.algorithm = p_data[0];
        }

        let op_params0 = p_params.operator_params[0].clone();
        let op_params1 = p_params.operator_params[1].clone();

        op_params0
            .borrow_mut()
            .set_pulse_generator_type(Self::sanitize_param_loop(p_data[1], 0, 511, "W1"));
        op_params1
            .borrow_mut()
            .set_pulse_generator_type(Self::sanitize_param_loop(p_data[2], 0, 511, "W2"));

        let balance = Self::sanitize_param_clamp(p_data[3], -64, 64, "BL");
        let table = crate::chip::ref_table::instance();
        let tl_table = table.borrow().eg_linear_to_total_level_table;
        op_params0.borrow_mut().total_level = tl_table[(64 - balance) as usize];
        op_params1.borrow_mut().total_level = tl_table[(balance + 64) as usize];

        op_params0.borrow_mut().detune2 = 0;
        op_params1.borrow_mut().detune2 = p_data[4];

        op_params0.borrow_mut().attack_rate =
            Self::sanitize_param_loop(p_data[5], 0, 63, "AR");
        op_params0.borrow_mut().decay_rate =
            Self::sanitize_param_loop(p_data[6], 0, 63, "DR");
        op_params0.borrow_mut().sustain_rate = 0;
        op_params0.borrow_mut().release_rate =
            Self::sanitize_param_loop(p_data[8], 0, 15, "RR");
        op_params0.borrow_mut().sustain_level =
            Self::sanitize_param_loop(p_data[7], 0, 63, "SL");
    }

    pub fn parse_siopm_params(p_params: &mut ChannelParams, p_data_string: &str) {
        let data = Self::split_data_string(p_params, p_data_string, 3, 15, "#@");
        Self::set_siopm_params_by_array(p_params, data);
    }

    pub fn parse_opl_params(p_params: &mut ChannelParams, p_data_string: &str) {
        let data = Self::split_data_string(p_params, p_data_string, 2, 11, "#OPL@");
        Self::set_opl_params_by_array(p_params, data);
    }

    pub fn parse_opm_params(p_params: &mut ChannelParams, p_data_string: &str) {
        let data = Self::split_data_string(p_params, p_data_string, 2, 11, "#OPM@");
        Self::set_opm_params_by_array(p_params, data);
    }

    pub fn parse_opn_params(p_params: &mut ChannelParams, p_data_string: &str) {
        let data = Self::split_data_string(p_params, p_data_string, 2, 10, "#OPN@");
        Self::set_opn_params_by_array(p_params, data);
    }

    pub fn parse_opx_params(p_params: &mut ChannelParams, p_data_string: &str) {
        let data = Self::split_data_string(p_params, p_data_string, 2, 12, "#OPX@");
        Self::set_opx_params_by_array(p_params, data);
    }

    pub fn parse_ma3_params(p_params: &mut ChannelParams, p_data_string: &str) {
        let data = Self::split_data_string(p_params, p_data_string, 2, 12, "#MA@");
        Self::set_ma3_params_by_array(p_params, data);
    }

    pub fn parse_al_params(p_params: &mut ChannelParams, p_data_string: &str) {
        let data = Self::split_data_string(p_params, p_data_string, 9, 0, "#AL@");
        Self::set_al_params_by_array(p_params, data);
    }

    pub fn set_siopm_params(p_params: &mut ChannelParams, p_data: Vec<i32>) {
        Self::check_operator_count(p_params, p_data.len() as i32, 3, 15, "#@");
        Self::set_siopm_params_by_array(p_params, p_data);
    }

    pub fn set_opl_params(p_params: &mut ChannelParams, p_data: Vec<i32>) {
        Self::check_operator_count(p_params, p_data.len() as i32, 2, 11, "#OPL@");
        Self::set_opl_params_by_array(p_params, p_data);
    }

    pub fn set_opm_params(p_params: &mut ChannelParams, p_data: Vec<i32>) {
        Self::check_operator_count(p_params, p_data.len() as i32, 2, 11, "#OPM@");
        Self::set_opm_params_by_array(p_params, p_data);
    }

    pub fn set_opn_params(p_params: &mut ChannelParams, p_data: Vec<i32>) {
        Self::check_operator_count(p_params, p_data.len() as i32, 2, 10, "#OPN@");
        Self::set_opn_params_by_array(p_params, p_data);
    }

    pub fn set_opx_params(p_params: &mut ChannelParams, p_data: Vec<i32>) {
        Self::check_operator_count(p_params, p_data.len() as i32, 2, 12, "#OPX@");
        Self::set_opx_params_by_array(p_params, p_data);
    }

    pub fn set_ma3_params(p_params: &mut ChannelParams, p_data: Vec<i32>) {
        Self::check_operator_count(p_params, p_data.len() as i32, 2, 12, "#MA@");
        Self::set_ma3_params_by_array(p_params, p_data);
    }

    pub fn set_al_params(p_params: &mut ChannelParams, p_data: Vec<i32>) {
        err_fail_cond_msg!(
            p_data.len() as i32 != 9,
            "p_data.size() != 9",
            format!(
                "Translator: Invalid parameter count in '{}' (channel: {}, each operator: {}).",
                "#AL@", 9, 0
            )
        );

        Self::set_al_params_by_array(p_params, p_data);
    }

    fn get_algorithm_index(
        p_operator_count: i32,
        p_algorithm: i32,
        p_table: &[[i32; 16]; 4],
        p_command: &str,
    ) -> i32 {
        let alg_index = p_operator_count - 1;
        err_fail_index_v_msg!(
            alg_index,
            "alg_index",
            4,
            "4",
            -1,
            format!(
                "Translator: Invalid operator count in the algorithm parameter 'opc{}/alg{}' in '{}'.",
                p_operator_count, p_algorithm, p_command
            )
        );

        for i in 0..16 {
            if p_algorithm == p_table[alg_index as usize][i] {
                return i as i32;
            }
        }

        err_fail_v_msg!(
            -1,
            "-1",
            format!(
                "Translator: Invalid algorithm parameter 'opc{}/alg{}' in '{}'.",
                p_operator_count, p_algorithm, p_command
            )
        );
    }

    fn get_ma3_from_pg_type(p_pulse_generator_type: i32, p_command: &str) -> i32 {
        // Standard wave types.
        let wave_shape = p_pulse_generator_type - PULSE_MA3_SINE;
        if (0..=31).contains(&wave_shape) {
            return wave_shape;
        }

        // Custom wave types.
        let custom_type = p_pulse_generator_type - PULSE_CUSTOM;
        if (0..=127).contains(&custom_type) {
            return custom_type;
        }

        // Known PG types compatible with wave types.
        match p_pulse_generator_type {
            0 => return 0,     // Sine
            1 | 2 | 128 | 255 => return 24,  // Saw
            4 | 191 | 192 => return 16,      // Triangle
            5 | 72 => return 6,              // Square
            _ => {}
        }

        err_fail_v_msg!(
            -1,
            "-1",
            format!(
                "Translator: Cannot convert pulse generator type ({}) into a wave shape in '{}'.",
                p_pulse_generator_type, p_command
            )
        );
    }

    fn get_nearest_dt2(p_detune: i32) -> i32 {
        if p_detune <= 100 {
            0 // 0
        } else if p_detune <= 420 {
            1 // 384
        } else if p_detune <= 550 {
            2 // 500
        } else {
            3 // 608
        }
    }

    fn balance_total_levels(p_level0: i32, p_level1: i32) -> i32 {
        if p_level0 == p_level1 {
            return 0;
        }
        if p_level0 == 0 {
            return -64;
        }
        if p_level1 == 0 {
            return 64;
        }

        let table = crate::chip::ref_table::instance();
        let tl_table = table.borrow().eg_linear_to_total_level_table;
        for i in 1..128 {
            if p_level0 >= tl_table[i] {
                return (i - 64) as i32;
            }
        }

        64
    }

    pub fn get_siopm_params(p_params: &ChannelParams) -> Vec<i32> {
        if p_params.operator_count == 0 {
            return Vec::new();
        }

        let mut res = vec![
            p_params.algorithm,
            p_params.feedback,
            p_params.feedback_connection,
        ];

        for i in 0..p_params.operator_count {
            let op = p_params.operator_params[i as usize].borrow();
            res.extend_from_slice(&[
                op.pulse_generator_type,
                op.attack_rate,
                op.decay_rate,
                op.sustain_rate,
                op.release_rate,
                op.sustain_level,
                op.total_level,
                op.key_scaling_rate,
                op.key_scaling_level,
                op.get_multiple(),
                op.detune1,
                op.detune2,
                op.amplitude_modulation_shift,
                op.initial_phase,
                op.fixed_pitch >> 6,
            ]);
        }

        res
    }

    pub fn get_opl_params(p_params: &ChannelParams) -> Vec<i32> {
        if p_params.operator_count == 0 {
            return Vec::new();
        }

        let alg_index =
            Self::get_algorithm_index(p_params.operator_count, p_params.algorithm, &ALGORITHM_OPL, "#OPL@");
        if alg_index == -1 {
            return Vec::new();
        }

        let mut res = vec![alg_index, p_params.feedback];

        for i in 0..p_params.operator_count {
            let op = p_params.operator_params[i as usize].borrow();

            let wave_shape = Self::get_ma3_from_pg_type(op.pulse_generator_type, "#OPL@");
            if wave_shape == -1 {
                return Vec::new();
            }

            let egt = if op.sustain_rate == 0 { 1 } else { 0 }; // Envelope generator t?..
            let total_level = if op.total_level < 63 { op.total_level } else { 63 };

            res.extend_from_slice(&[
                wave_shape,
                op.attack_rate >> 2,
                op.decay_rate >> 2,
                op.release_rate >> 2,
                egt,
                op.sustain_level,
                total_level,
                op.key_scaling_rate >> 1,
                op.key_scaling_level,
                op.get_multiple(),
                op.amplitude_modulation_shift,
            ]);
        }

        res
    }

    pub fn get_opm_params(p_params: &ChannelParams) -> Vec<i32> {
        if p_params.operator_count == 0 {
            return Vec::new();
        }

        let alg_index =
            Self::get_algorithm_index(p_params.operator_count, p_params.algorithm, &ALGORITHM_OPM, "#OPM@");
        if alg_index == -1 {
            return Vec::new();
        }

        let mut res = vec![alg_index, p_params.feedback];

        for i in 0..p_params.operator_count {
            let op = p_params.operator_params[i as usize].borrow();

            let detune2 = Self::get_nearest_dt2(op.detune2);

            res.extend_from_slice(&[
                op.attack_rate >> 1,
                op.decay_rate >> 1,
                op.sustain_rate >> 1,
                op.release_rate >> 2,
                op.sustain_level,
                op.total_level,
                op.key_scaling_rate,
                op.get_multiple(),
                op.detune1,
                detune2,
                op.amplitude_modulation_shift,
            ]);
        }

        res
    }

    pub fn get_opn_params(p_params: &ChannelParams) -> Vec<i32> {
        if p_params.operator_count == 0 {
            return Vec::new();
        }

        // Note: OPM and OPN share the algo list.
        let alg_index =
            Self::get_algorithm_index(p_params.operator_count, p_params.algorithm, &ALGORITHM_OPM, "#OPN@");
        if alg_index == -1 {
            return Vec::new();
        }

        let mut res = vec![alg_index, p_params.feedback];

        for i in 0..p_params.operator_count {
            let op = p_params.operator_params[i as usize].borrow();

            res.extend_from_slice(&[
                op.attack_rate >> 1,
                op.decay_rate >> 1,
                op.sustain_rate >> 1,
                op.release_rate >> 2,
                op.sustain_level,
                op.total_level,
                op.key_scaling_rate,
                op.get_multiple(),
                op.detune1,
                op.amplitude_modulation_shift,
            ]);
        }

        res
    }

    pub fn get_opx_params(p_params: &ChannelParams) -> Vec<i32> {
        if p_params.operator_count == 0 {
            return Vec::new();
        }

        let alg_index =
            Self::get_algorithm_index(p_params.operator_count, p_params.algorithm, &ALGORITHM_OPX, "#OPX@");
        if alg_index == -1 {
            return Vec::new();
        }

        let mut res = vec![alg_index, p_params.feedback];

        for i in 0..p_params.operator_count {
            let op = p_params.operator_params[i as usize].borrow();

            let wave_shape = Self::get_ma3_from_pg_type(op.pulse_generator_type, "#OPX@");
            if wave_shape == -1 {
                return Vec::new();
            }

            res.extend_from_slice(&[
                wave_shape,
                op.attack_rate >> 1,
                op.decay_rate >> 1,
                op.sustain_rate >> 1,
                op.release_rate >> 2,
                op.sustain_level,
                op.total_level,
                op.key_scaling_rate,
                op.get_multiple(),
                op.detune1,
                op.detune2,
                op.amplitude_modulation_shift,
            ]);
        }

        res
    }

    pub fn get_ma3_params(p_params: &ChannelParams) -> Vec<i32> {
        if p_params.operator_count == 0 {
            return Vec::new();
        }

        let alg_index =
            Self::get_algorithm_index(p_params.operator_count, p_params.algorithm, &ALGORITHM_MA3, "#MA@");
        if alg_index == -1 {
            return Vec::new();
        }

        let mut res = vec![alg_index, p_params.feedback];

        for i in 0..p_params.operator_count {
            let op = p_params.operator_params[i as usize].borrow();

            let wave_shape = Self::get_ma3_from_pg_type(op.pulse_generator_type, "#MA@");
            if wave_shape == -1 {
                return Vec::new();
            }

            let total_level = if op.total_level < 63 { op.total_level } else { 63 };

            res.extend_from_slice(&[
                wave_shape,
                op.attack_rate >> 2,
                op.decay_rate >> 2,
                op.sustain_rate >> 2,
                op.release_rate >> 2,
                op.sustain_level,
                total_level,
                op.key_scaling_rate >> 1,
                op.key_scaling_level,
                op.get_multiple(),
                op.detune1,
                op.amplitude_modulation_shift,
            ]);
        }

        res
    }

    pub fn get_al_params(p_params: &ChannelParams) -> Vec<i32> {
        if p_params.operator_count != 5 {
            return Vec::new();
        }

        let op0 = p_params.operator_params[0].borrow();
        let op1 = p_params.operator_params[1].borrow();

        let level_balance = Self::balance_total_levels(op0.total_level, op1.total_level);

        vec![
            p_params.algorithm,
            op0.pulse_generator_type,
            op1.pulse_generator_type,
            level_balance,
            op1.detune2,
            op0.attack_rate,
            op0.decay_rate,
            op0.sustain_level,
            op0.release_rate,
        ]
    }

    fn format_mml_comment(p_comment: &str, p_line_end: &str) -> String {
        if p_comment.is_empty() {
            return String::new();
        }

        if p_line_end == "\n" {
            format!(" // {}", p_comment)
        } else {
            format!("/* {} */", p_comment)
        }
    }

    fn format_mml_digit(p_value: i32, p_padded: i32) -> String {
        if p_padded <= 0 {
            return itos(p_value as i64);
        }

        let mut padded_length = p_padded;
        if p_value < 0 {
            padded_length -= 1; // Accounts for the minus sign.
        }

        pad_zeros(&itos(p_value as i64), padded_length)
    }

    fn get_operator_params_sizes(p_params: &ChannelParams) -> OperatorParamsSizes {
        let mut sizes = OperatorParamsSizes::default();

        for i in 0..p_params.operator_count {
            let op = p_params.operator_params[i as usize].borrow();

            macro_rules! max_param_size {
                ($key:expr, $value:expr) => {{
                    let value_string = itos($value as i64);
                    if value_string.len() as i32 > $key {
                        $key = value_string.len() as i32;
                    }
                }};
            }

            max_param_size!(sizes.pg_type, op.pulse_generator_type);
            max_param_size!(sizes.total_level, op.total_level);
            max_param_size!(sizes.detune2, op.detune2);
            max_param_size!(sizes.phase, op.initial_phase);
            max_param_size!(sizes.fixed_pitch, op.fixed_pitch >> 6);
        }

        sizes
    }

    pub fn get_siopm_params_as_mml(
        p_params: &ChannelParams,
        p_separator: &str,
        p_line_end: &str,
        p_comment: &str,
    ) -> String {
        if p_params.get_operator_count() == 0 {
            return String::new();
        }

        // Open MML string.
        let mut mml = String::from("{");

        // Channel parameters.

        mml += &Self::format_mml_digit(p_params.algorithm, 0);
        mml += p_separator;
        mml += &Self::format_mml_digit(p_params.feedback, 0);
        mml += p_separator;
        mml += &Self::format_mml_digit(p_params.feedback_connection, 0);

        // Custom comment message.

        mml += &Self::format_mml_comment(p_comment, p_line_end);

        // Operator parameters.

        let sizes = Self::get_operator_params_sizes(p_params);

        for i in 0..p_params.get_operator_count() {
            let op_rc = p_params.get_operator_params(i).expect("operator params");
            let op = op_rc.borrow();

            mml += p_line_end;

            mml += &Self::format_mml_digit(op.pulse_generator_type, sizes.pg_type);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.attack_rate, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.decay_rate, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.sustain_rate, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.release_rate, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.sustain_level, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.total_level, sizes.total_level);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.key_scaling_rate, 0);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.key_scaling_level, 0);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.get_multiple(), 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.detune1, 0);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.detune2, sizes.detune2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.amplitude_modulation_shift, 0);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.initial_phase, sizes.phase);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.fixed_pitch >> 6, sizes.fixed_pitch);
        }

        // Close MML string.
        mml += "}";

        mml
    }

    pub fn get_opl_params_as_mml(
        p_params: &ChannelParams,
        p_separator: &str,
        p_line_end: &str,
        p_comment: &str,
    ) -> String {
        if p_params.get_operator_count() == 0 {
            return String::new();
        }

        let alg_index = Self::get_algorithm_index(
            p_params.operator_count,
            p_params.algorithm,
            &ALGORITHM_OPL,
            "#OPL@",
        );
        if alg_index == -1 {
            return String::new();
        }

        // Open MML string.
        let mut mml = String::from("{");

        // Channel parameters.

        mml += &Self::format_mml_digit(alg_index, 0);
        mml += p_separator;
        mml += &Self::format_mml_digit(p_params.feedback, 0);

        // Custom comment message.

        mml += &Self::format_mml_comment(p_comment, p_line_end);

        // Operator parameters.

        for i in 0..p_params.get_operator_count() {
            let op_rc = p_params.get_operator_params(i).expect("operator params");
            let op = op_rc.borrow();

            let wave_shape = Self::get_ma3_from_pg_type(op.pulse_generator_type, "#OPL@");
            if wave_shape == -1 {
                return String::new();
            }

            mml += p_line_end;

            mml += &Self::format_mml_digit(wave_shape, 0);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.attack_rate >> 2, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.decay_rate >> 2, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.release_rate >> 2, 2);
            mml += p_separator;
            mml += if op.sustain_rate == 0 { "1" } else { "0" };
            mml += p_separator;
            mml += &Self::format_mml_digit(op.sustain_level, 2);
            mml += p_separator;

            let total_level = if op.total_level < 63 { op.total_level } else { 63 };
            mml += &Self::format_mml_digit(total_level, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.key_scaling_rate >> 1, 0);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.key_scaling_level, 0);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.get_multiple(), 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.amplitude_modulation_shift, 0);
        }

        // Close MML string.
        mml += "}";

        mml
    }

    pub fn get_opm_params_as_mml(
        p_params: &ChannelParams,
        p_separator: &str,
        p_line_end: &str,
        p_comment: &str,
    ) -> String {
        if p_params.get_operator_count() == 0 {
            return String::new();
        }

        let alg_index = Self::get_algorithm_index(
            p_params.operator_count,
            p_params.algorithm,
            &ALGORITHM_OPM,
            "#OPM@",
        );
        if alg_index == -1 {
            return String::new();
        }

        // Open MML string.
        let mut mml = String::from("{");

        // Channel parameters.

        mml += &Self::format_mml_digit(alg_index, 0);
        mml += p_separator;
        mml += &Self::format_mml_digit(p_params.feedback, 0);

        // Custom comment message.

        mml += &Self::format_mml_comment(p_comment, p_line_end);

        // Operator parameters.

        let sizes = Self::get_operator_params_sizes(p_params);

        for i in 0..p_params.get_operator_count() {
            let op_rc = p_params.get_operator_params(i).expect("operator params");
            let op = op_rc.borrow();

            mml += p_line_end;

            mml += &Self::format_mml_digit(op.attack_rate >> 1, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.decay_rate >> 1, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.sustain_rate >> 1, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.release_rate >> 2, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.sustain_level, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.total_level, sizes.total_level);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.key_scaling_level, 0);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.get_multiple(), 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.detune1, 0);
            mml += p_separator;

            let detune2 = Self::get_nearest_dt2(op.detune2);
            mml += &Self::format_mml_digit(detune2, 0);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.amplitude_modulation_shift, 0);
        }

        // Close MML string.
        mml += "}";

        mml
    }

    pub fn get_opn_params_as_mml(
        p_params: &ChannelParams,
        p_separator: &str,
        p_line_end: &str,
        p_comment: &str,
    ) -> String {
        if p_params.get_operator_count() == 0 {
            return String::new();
        }

        // Note: OPM and OPN share the algo list.
        let alg_index = Self::get_algorithm_index(
            p_params.operator_count,
            p_params.algorithm,
            &ALGORITHM_OPM,
            "#OPN@",
        );
        if alg_index == -1 {
            return String::new();
        }

        // Open MML string.
        let mut mml = String::from("{");

        // Channel parameters.

        mml += &Self::format_mml_digit(alg_index, 0);
        mml += p_separator;
        mml += &Self::format_mml_digit(p_params.feedback, 0);

        // Custom comment message.

        mml += &Self::format_mml_comment(p_comment, p_line_end);

        // Operator parameters.

        let sizes = Self::get_operator_params_sizes(p_params);

        for i in 0..p_params.get_operator_count() {
            let op_rc = p_params.get_operator_params(i).expect("operator params");
            let op = op_rc.borrow();

            mml += p_line_end;

            mml += &Self::format_mml_digit(op.attack_rate >> 1, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.decay_rate >> 1, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.sustain_rate >> 1, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.release_rate >> 2, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.sustain_level, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.total_level, sizes.total_level);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.key_scaling_level, 0);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.get_multiple(), 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.detune1, 0);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.amplitude_modulation_shift, 0);
        }

        // Close MML string.
        mml += "}";

        mml
    }

    pub fn get_opx_params_as_mml(
        p_params: &ChannelParams,
        p_separator: &str,
        p_line_end: &str,
        p_comment: &str,
    ) -> String {
        if p_params.get_operator_count() == 0 {
            return String::new();
        }

        let alg_index = Self::get_algorithm_index(
            p_params.operator_count,
            p_params.algorithm,
            &ALGORITHM_OPX,
            "#OPX@",
        );
        if alg_index == -1 {
            return String::new();
        }

        // Open MML string.
        let mut mml = String::from("{");

        // Channel parameters.

        mml += &Self::format_mml_digit(alg_index, 0);
        mml += p_separator;
        mml += &Self::format_mml_digit(p_params.feedback, 0);

        // Custom comment message.

        mml += &Self::format_mml_comment(p_comment, p_line_end);

        // Operator parameters.

        let sizes = Self::get_operator_params_sizes(p_params);

        for i in 0..p_params.get_operator_count() {
            let op_rc = p_params.get_operator_params(i).expect("operator params");
            let op = op_rc.borrow();

            let wave_shape = Self::get_ma3_from_pg_type(op.pulse_generator_type, "#OPX@");
            if wave_shape == -1 {
                return String::new();
            }

            mml += p_line_end;

            mml += &Self::format_mml_digit(wave_shape, 0);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.attack_rate >> 1, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.decay_rate >> 1, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.sustain_rate >> 1, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.release_rate >> 2, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.sustain_level, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.total_level, sizes.total_level);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.key_scaling_level, 0);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.get_multiple(), 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.detune1, 0);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.detune2, sizes.detune2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.amplitude_modulation_shift, 0);
        }

        // Close MML string.
        mml += "}";

        mml
    }

    pub fn get_ma3_params_as_mml(
        p_params: &ChannelParams,
        p_separator: &str,
        p_line_end: &str,
        p_comment: &str,
    ) -> String {
        if p_params.get_operator_count() == 0 {
            return String::new();
        }

        let alg_index = Self::get_algorithm_index(
            p_params.operator_count,
            p_params.algorithm,
            &ALGORITHM_MA3,
            "#MA@",
        );
        if alg_index == -1 {
            return String::new();
        }

        // Open MML string.
        let mut mml = String::from("{");

        // Channel parameters.

        mml += &Self::format_mml_digit(alg_index, 0);
        mml += p_separator;
        mml += &Self::format_mml_digit(p_params.feedback, 0);

        // Custom comment message.

        mml += &Self::format_mml_comment(p_comment, p_line_end);

        // Operator parameters.

        for i in 0..p_params.get_operator_count() {
            let op_rc = p_params.get_operator_params(i).expect("operator params");
            let op = op_rc.borrow();

            let wave_shape = Self::get_ma3_from_pg_type(op.pulse_generator_type, "#MA@");
            if wave_shape == -1 {
                return String::new();
            }

            mml += p_line_end;

            mml += &Self::format_mml_digit(wave_shape, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.attack_rate >> 2, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.decay_rate >> 2, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.sustain_rate >> 2, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.release_rate >> 2, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.sustain_level, 2);
            mml += p_separator;

            let total_level = if op.total_level < 63 { op.total_level } else { 63 };
            mml += &Self::format_mml_digit(total_level, 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.key_scaling_rate >> 1, 0);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.key_scaling_level, 0);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.get_multiple(), 2);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.detune1, 0);
            mml += p_separator;
            mml += &Self::format_mml_digit(op.amplitude_modulation_shift, 0);
        }

        // Close MML string.
        mml += "}";

        mml
    }

    pub fn get_al_params_as_mml(
        p_params: &ChannelParams,
        p_separator: &str,
        p_line_end: &str,
        p_comment: &str,
    ) -> String {
        if p_params.get_operator_count() != 5 {
            return String::new();
        }

        let op_params0 = p_params.operator_params[0].clone();
        let op_params1 = p_params.operator_params[1].clone();
        let op0 = op_params0.borrow();
        let op1 = op_params1.borrow();

        // Open MML string.
        let mut mml = String::from("{");

        // Leading parameters.

        mml += &Self::format_mml_digit(p_params.algorithm, 0);
        mml += p_separator;
        mml += &Self::format_mml_digit(op0.pulse_generator_type, 0);
        mml += p_separator;
        mml += &Self::format_mml_digit(op1.pulse_generator_type, 0);
        mml += p_separator;

        let balanced_levels = Self::balance_total_levels(op0.total_level, op1.total_level);
        mml += &Self::format_mml_digit(balanced_levels, 0);
        mml += p_separator;
        mml += &Self::format_mml_digit(op1.detune2, 0);
        mml += p_separator;

        // Custom comment message.

        mml += &Self::format_mml_comment(p_comment, p_line_end);

        // Trailing parameters.

        mml += p_line_end;

        mml += &Self::format_mml_digit(op0.attack_rate, 0);
        mml += p_separator;
        mml += &Self::format_mml_digit(op0.decay_rate, 0);
        mml += p_separator;
        mml += &Self::format_mml_digit(op0.sustain_level, 0);
        mml += p_separator;
        mml += &Self::format_mml_digit(op0.release_rate, 0);

        // Close MML string.
        mml += "}";

        mml
    }

    // TODO(port): `extract_system_command` is declared in translator_util.h
    // but has no C++ definition anywhere — dead declaration, not ported.

    /// C++ `parse_voice_setting(const Ref<SiMMLVoice>&, sion::String,
    /// std::vector<Ref<SiMMLEnvelopeTable>>)` — applies the
    /// `SiONMML_voice`-style setting string (module header commands) to
    /// the voice/channel params. 22 capture groups: 1 command + 11
    /// arguments (arg `m` lives in group `2 + m * 2`).
    /// NOTE quirks reproduced: the envelope branches index `p_envelopes`
    /// with the MATCH index `i`, not the parsed table number (original
    /// C++ bug), and `get(i)` stands in for the C++ out-of-bounds vector
    /// read; `@er` compares the raw argument string against `"1"`.
    #[allow(clippy::too_many_arguments)]
    pub fn parse_voice_setting(
        p_voice: &Rc<RefCell<crate::sequencer::voice::SiMMLVoice>>,
        p_mml: &str,
        p_envelopes: Vec<Option<Rc<RefCell<crate::sequencer::envelope_table::SiMMLEnvelopeTable>>>>,
    ) {
        static RE_SETTING: LazyLock<Regex> = LazyLock::new(|| {
            let base =
                r"(%[fvx]|@[fpqv]|@er|@lfo|kt?|m[ap]|_?@@|_?n[aptf]|po|p|q|s|x|v)".to_string();
            let args = format!("(-?\\d*){}", r"(\s*,\s*(-?\d*))?".repeat(10));
            Regex::new(&(base + &args)).expect("valid regex")
        });

        let params = p_voice.borrow().channel_params.clone();

        for (i, caps) in RE_SETTING.captures_iter(p_mml).enumerate() {
            let arg = |index: usize, default: i32| -> i32 {
                let value = group_str(&caps, 2 + index * 2);
                if value.is_empty() {
                    default
                } else {
                    to_int(value) as i32
                }
            };
            let arg_mod = |index: usize, modulo: f64, default: f64| -> f64 {
                let value = group_str(&caps, 2 + index * 2);
                if value.is_empty() {
                    default
                } else {
                    to_int(value) as f64 * modulo
                }
            };
            let arg_pos = |index: usize, default: i32| -> i32 {
                let value = to_int(group_str(&caps, 2 + index * 2)) as i32;
                if value > 0 {
                    value
                } else {
                    default
                }
            };

            let command = group_str(&caps, 1);
            let mut voice = p_voice.borrow_mut();
            let mut params = params.borrow_mut();

            macro_rules! env_command {
                ($env_field:ident, $step_field:ident) => {
                    let value = arg(0, 0);
                    if !p_envelopes.is_empty() && (0..255).contains(&value) {
                        voice.$env_field = p_envelopes.get(i).cloned().flatten();
                        voice.$step_field = arg_pos(1, 1);
                    }
                };
            }

            if command == "@f" {
                params.filter_cutoff = arg(0, 128);
                params.filter_resonance = arg(1, 0);
                params.filter_attack_rate = arg(2, 0);
                params.filter_decay_rate1 = arg(3, 0);
                params.filter_decay_rate2 = arg(4, 0);
                params.filter_release_rate = arg(5, 0);
                params.filter_decay_offset1 = arg(6, 128);
                params.filter_decay_offset2 = arg(7, 64);
                params.filter_sustain_offset = arg(8, 32);
                params.filter_release_offset = arg(9, 128);
            } else if command == "@lfo" {
                params.set_lfo_frame(arg(0, 30));
                params.lfo_wave_shape =
                    arg(1, crate::chip::ref_table::LFO_WAVE_TRIANGLE as i32);
            } else if command == "ma" {
                voice.amplitude_modulation_depth = arg(0, 0);
                voice.amplitude_modulation_depth_end = arg(1, 0);
                voice.amplitude_modulation_delay = arg(2, 0);
                voice.amplitude_modulation_term = arg(3, 0);
                params.amplitude_modulation_depth = voice.amplitude_modulation_depth;
            } else if command == "mp" {
                voice.pitch_modulation_depth = arg(0, 0);
                voice.pitch_modulation_depth_end = arg(1, 0);
                voice.pitch_modulation_delay = arg(2, 0);
                voice.pitch_modulation_term = arg(3, 0);
                params.pitch_modulation_depth = voice.pitch_modulation_depth;
            } else if command == "po" {
                voice.portament = arg(0, 30);
            } else if command == "q" {
                voice.default_gate_time = arg_mod(0, 0.125, f64::NAN);
            } else if command == "s" {
                voice.release_sweep = arg(2, 0);
            } else if command == "%f" {
                params.filter_type = arg(0, 0);
            } else if command == "@er" {
                let reset = group_str(&caps, 2) != "1";
                for j in 0..4 {
                    params.operator_params[j].borrow_mut().envelope_reset_on_attack = reset;
                }
            } else if command == "k" {
                voice.pitch_shift = arg(0, 0);
            } else if command == "kt" {
                voice.note_shift = arg(0, 0);
            } else if command == "@v" {
                for j in 0..8 {
                    params.master_volumes[j] =
                        arg_mod(j, 0.0078125, if j == 0 { 0.5 } else { 0.0 });
                }
            } else if command == "p" {
                params.pan = if group_str(&caps, 2).is_empty() {
                    64
                } else {
                    to_int(group_str(&caps, 2)) as i32 * 16
                };
            } else if command == "@p" {
                params.pan = arg(0, 64);
            } else if command == "v" {
                let value = group_str(&caps, 2);
                if !value.is_empty() {
                    voice.velocity = (to_int(value) as i32) << voice.velocity_shift;
                } else {
                    voice.velocity = 256;
                }
            } else if command == "x" {
                voice.expression = arg(0, 128);
            } else if command == "%v" {
                voice.velocity_mode = arg(0, 0);
                voice.velocity_shift = arg(1, 4);
            } else if command == "%x" {
                voice.expression_mode = arg(0, 0);
            } else if command == "@q" {
                voice.default_gate_ticks = arg(0, 0);
                voice.default_key_on_delay_ticks = arg(1, 0);
            } else if command == "@@" {
                env_command!(note_on_tone_envelope, note_on_tone_envelope_step);
            } else if command == "na" {
                env_command!(note_on_amplitude_envelope, note_on_amplitude_envelope_step);
            } else if command == "np" {
                env_command!(note_on_pitch_envelope, note_on_pitch_envelope_step);
            } else if command == "nt" {
                env_command!(note_on_note_envelope, note_on_note_envelope_step);
            } else if command == "nf" {
                env_command!(note_on_filter_envelope, note_on_filter_envelope_step);
            } else if command == "_@@" {
                env_command!(note_off_tone_envelope, note_off_tone_envelope_step);
            } else if command == "_na" {
                env_command!(note_off_amplitude_envelope, note_off_amplitude_envelope_step);
            } else if command == "_np" {
                env_command!(note_off_pitch_envelope, note_off_pitch_envelope_step);
            } else if command == "_nt" {
                env_command!(note_off_note_envelope, note_off_note_envelope_step);
            } else if command == "_nf" {
                env_command!(note_off_filter_envelope, note_off_filter_envelope_step);
            }
        }
    }


    /// `RegEx::search_all` for the table regex: none of the alternatives can
    /// match empty, so `find_iter` is exactly PCRE2's global search.
    pub fn parse_table_numbers(
        p_table_numbers: &str,
        p_postfix: &str,
        p_max_index: i32,
    ) -> MMLTableNumbers {
        let mut parsed_table = MMLTableNumbers {
            data: SinglyLinkedList::new(),
            length: 0,
            repeated: false,
        };

        // Magnification.
        let Some(caps) = Self::re_postfix().captures(p_postfix) else {
            // ERR_FAIL_COND_V(res.is_null(), parsed_table).
            crate::error::err_print_body(
                "Condition \"res.is_null()\" is true. Returning: parsed_table",
                false,
            );
            return parsed_table;
        };

        let mut postfix_size = 1i32;
        let mut postfix_coef = 1.0f64;
        let mut postfix_offset = 0.0f64;

        if !group_str(&caps, 1).is_empty() {
            postfix_size = to_int(group_str(&caps, 1)) as i32;
        }
        if !group_str(&caps, 2).is_empty() {
            postfix_coef = to_float(group_str(&caps, 3));
        }
        if !group_str(&caps, 4).is_empty() {
            postfix_offset = to_float(group_str(&caps, 4));
        }

        // match[1];(n..),m {match[2];n.., match[3];m} / match[4];n / match[5];|[] / match[6]; ]n
        let re_table = Self::re_table();
        let numbers: Vec<Captures<'_>> = re_table.captures_iter(p_table_numbers).collect();

        let mut repeat: Option<usize> = None;
        let mut loop_stack: Vec<Option<usize>> = Vec::new();

        let mut index = 0i32;
        let mut n = 0usize;
        while n < numbers.len() && index < p_max_index {
            let parsed_number = &numbers[n];
            n += 1;

            // Interpolation: "(match[2]..),match[3]"
            if !group_str(parsed_number, 1).is_empty() {
                let arr = split_string_by_regex(group_str(parsed_number, 2), "[,\\s]+");
                let inter_size = to_int(group_str(parsed_number, 3)) as i32;
                err_fail_table_cond_v_msg!(
                    inter_size < 2 || (arr.len() as i32) < 1,
                    "(inter_size < 2 || arr.size() < 1)",
                    parsed_table,
                    "Translator: Failed to parse provided MML table, interpolation data is invalid."
                );

                let mut inter_data: Vec<i32> = Vec::with_capacity(arr.len());
                for piece in &arr {
                    inter_data.push(to_int(piece) as i32);
                }

                if inter_data.len() > 1 {
                    let mut t = 0.0f64;
                    let s = (inter_data.len() as f64 - 1.0) / inter_size as f64;

                    let mut i = 0i32;
                    while i < inter_size && index < p_max_index {
                        let ti0 = t as usize;
                        let ti1 = ti0 + 1;
                        let tr = t - ti0 as f64;

                        let mut value =
                            (inter_data[ti0] as f64 * (1.0 - tr) + inter_data[ti1] as f64 * tr + 0.5)
                                as i32;
                        value =
                            (value as f64 * postfix_coef + postfix_offset + 0.5) as i32;

                        for _j in 0..postfix_size {
                            parsed_table.data.append(value);
                        }
                        index += postfix_size;

                        t += s;
                        i += 1;
                    }
                } else {
                    let value =
                        (inter_data[0] as f64 * postfix_coef + postfix_offset + 0.5) as i32;

                    let mut i = 0i32;
                    while i < inter_size && index < p_max_index {
                        for _j in 0..postfix_size {
                            parsed_table.data.append(value);
                        }
                        index += postfix_size;
                        i += 1;
                    }
                }

            // Single number.
            } else if !group_str(parsed_number, 4).is_empty() {
                let mut value = to_int(group_str(parsed_number, 4)) as i32;
                value = (value as f64 * postfix_coef + postfix_offset + 0.5) as i32;

                for _j in 0..postfix_size {
                    parsed_table.data.append(value);
                }
                index += 1;

            // Loop control characters.
            } else if !group_str(parsed_number, 5).is_empty() {
                let token = group_str(parsed_number, 5);

                // Loop repeat point.
                if token == "|" {
                    repeat = parsed_table.data.cursor;

                // Loop start.
                } else if token == "[" {
                    loop_stack.push(parsed_table.data.cursor);

                // Loop end.
                } else {
                    err_fail_table_cond_v_msg!(
                        loop_stack.is_empty(),
                        "loop_stack.empty()",
                        parsed_table,
                        "Translator: Failed to parse provided MML table, loop data is invalid."
                    );

                    let loop_tail = parsed_table.data.cursor;
                    // `(loop_stack.back()->get())->next()` — C++ dereferences
                    // the stored element pointer; a null element crashes,
                    // mirrored by the expect below.
                    let loop_head = parsed_table
                        .data
                        .next_of(
                            loop_stack
                                .pop()
                                .and_then(|marker| marker)
                                .expect("SinglyLinkedList element"),
                        );
                    err_fail_table_cond_v_msg!(
                        loop_head.is_none(),
                        "!loop_head",
                        parsed_table,
                        "Translator: Failed to parse provided MML table, loop data is invalid."
                    );

                    let mut loop_count = 2i32;
                    if !group_str(parsed_number, 6).is_empty() {
                        loop_count = to_int(group_str(parsed_number, 6)) as i32;
                    }

                    if loop_count > 0 {
                        let tail = loop_tail.expect("SinglyLinkedList element");
                        // C++ walks pointers from loop_head to loop_tail
                        // inclusive (`l != loop_tail->next()`); appends only
                        // ever extend past the tail, so snapshotting the
                        // segment is equivalent.
                        let segment =
                            parsed_table.data.values[loop_head.unwrap()..=tail].to_vec();
                        for _j in (0..loop_count).rev() {
                            for value in &segment {
                                parsed_table.data.append(*value);
                            }
                        }
                    }
                }
            } else {
                err_fail_v_msg!(
                    parsed_table,
                    "parsed_table",
                    "Translator: Failed to parse provided MML table, structure is invalid."
                );
            }
        }

        if let Some(repeat_index) = repeat {
            let next = parsed_table.data.next_of(repeat_index);
            parsed_table.data.loop_at(next);
        }

        parsed_table.length = index;
        parsed_table.repeated = repeat.is_some();
        parsed_table
    }

    pub fn parse_wav(p_table_numbers: &str, p_postfix: &str, r_data: &mut Vec<f64>) {
        let mut table = Self::parse_table_numbers(p_table_numbers, p_postfix, 1024);

        let mut data_length = 2i32;
        while data_length < 1024 && data_length < table.length {
            data_length <<= 1;
        }
        r_data.resize(data_length as usize, 0.0);

        let mut i = 0i32;
        table.data.front();
        while i < data_length && table.data.get().is_some() {
            let value = (table.data.get().unwrap() as f64 + 0.5) * 0.0078125;
            r_data[i as usize] = clampf(value, -1.0, 1.0);

            table.data.next();
            i += 1;
        }

        while i < data_length {
            r_data[i as usize] = 0.0;
            i += 1;
        }
    }

    pub fn parse_wavb(p_hex: &str, r_data: &mut Vec<f64>) {
        let hex = literal_replace_all(Self::re_spaces(), p_hex, "");

        let data_length = (hex.len() >> 1) as i32;
        r_data.resize(data_length as usize, 0.0);

        for i in 0..data_length {
            let value = hex_to_int(&substr(&hex, (i << 1) as usize, 2)) as i32;
            if value < 128 {
                r_data[i as usize] = value as f64 * 0.0078125;
            } else {
                r_data[i as usize] = (value - 256) as f64 * 0.0078125;
            }
        }
    }

    /// C++ `get_voice_setting_as_mml(const Ref<SiMMLVoice>&)` — inverse of
    /// [`parse_voice_setting`](Self::parse_voice_setting). NOTE: C++
    /// `itos(double)` truncates through an implicit `int` conversion —
    /// reproduced with `as i64` casts on the scaled volume/gate values.
    pub fn get_voice_setting_as_mml(
        p_voice: &Rc<RefCell<crate::sequencer::voice::SiMMLVoice>>,
    ) -> String {
        let params = p_voice.borrow().channel_params.clone();
        let voice = p_voice.borrow();
        let params = params.borrow();
        let mut mml = String::new();

        if params.filter_type > 0 {
            mml += "%f";
            mml += &itos(params.filter_type as i64);
        }
        if params.has_filter() || params.has_filter_advanced() {
            mml += &format!(
                "@f{},{}",
                params.filter_cutoff, params.filter_resonance
            );

            if params.has_filter_advanced() {
                mml += &format!(
                    ",{},{},{},{},{},{},{},{}",
                    params.filter_attack_rate,
                    params.filter_decay_rate1,
                    params.filter_decay_rate2,
                    params.filter_release_rate,
                    params.filter_decay_offset1,
                    params.filter_decay_offset2,
                    params.filter_sustain_offset,
                    params.filter_release_offset
                );
            }
        }

        if voice.has_amplitude_modulation()
            || params.has_amplitude_modulation()
            || voice.has_pitch_modulation()
            || params.has_pitch_modulation()
        {
            if params.get_lfo_frame() != 30
                || params.lfo_wave_shape != crate::chip::ref_table::LFO_WAVE_TRIANGLE as i32
            {
                mml += "@lfo";
                mml += &itos(params.get_lfo_frame() as i64);

                if params.lfo_wave_shape != crate::chip::ref_table::LFO_WAVE_TRIANGLE as i32 {
                    mml += ",";
                    mml += &itos(params.lfo_wave_shape as i64);
                }
            }

            if voice.has_amplitude_modulation() {
                mml += &format!(
                    "ma{},{},{},{}",
                    voice.amplitude_modulation_depth,
                    voice.amplitude_modulation_depth_end,
                    voice.amplitude_modulation_delay,
                    voice.amplitude_modulation_term
                );
            } else if params.has_amplitude_modulation() {
                mml += "ma";
                mml += &itos(params.amplitude_modulation_depth as i64);
            }

            if voice.has_pitch_modulation() {
                mml += &format!(
                    "mp{},{},{},{}",
                    voice.pitch_modulation_depth,
                    voice.pitch_modulation_depth_end,
                    voice.pitch_modulation_delay,
                    voice.pitch_modulation_term
                );
            } else if params.has_pitch_modulation() {
                mml += "mp";
                mml += &itos(params.pitch_modulation_depth as i64);
            }
        }

        if voice.velocity_mode != 0 || voice.velocity_shift != 4 {
            mml += &format!("%v{},{}", voice.velocity_mode, voice.velocity_shift);
        }

        if voice.expression_mode != 0 {
            mml += "%x";
            mml += &itos(voice.expression_mode as i64);
        }

        if voice.portament > 0 {
            mml += "po";
            mml += &itos(voice.portament as i64);
        }

        if !voice.default_gate_time.is_nan() {
            mml += "q";
            mml += &itos((voice.default_gate_time * 8.0) as i64);
        }

        if voice.default_gate_ticks > 0 || voice.default_key_on_delay_ticks > 0 {
            mml += &format!(
                "@q{},{}",
                voice.default_gate_ticks, voice.default_key_on_delay_ticks
            );
        }

        if voice.release_sweep > 0 {
            // First argument, release rate, is not implemented.
            mml += "s,";
            mml += &itos(voice.release_sweep as i64);
        }

        if params.operator_params[0].borrow().envelope_reset_on_attack {
            mml += "@er1";
        }

        if voice.pitch_shift != 0 {
            mml += "k";
            mml += &itos(voice.pitch_shift as i64);
        }
        if voice.note_shift != 0 {
            mml += "kt";
            mml += &itos(voice.note_shift as i64);
        }

        if voice.update_volumes {
            let mut volumes_count = if params.master_volumes[0] == 0.5 { 0 } else { 1 };
            for i in 1..8 {
                if params.master_volumes[i] != 0.0 {
                    volumes_count = i + 1;
                }
            }

            if volumes_count > 0 {
                mml += "@v";
                mml += &itos((params.master_volumes[0] * 128.0) as i64);
                for i in 1..volumes_count {
                    mml += ",";
                    mml += &itos((params.master_volumes[i] * 128.0) as i64);
                }
            }

            if params.pan != 64 {
                if params.pan & 15 != 0 {
                    mml += "@p";
                    mml += &itos((params.pan - 64) as i64);
                } else {
                    mml += "p";
                    mml += &itos((params.pan >> 4) as i64);
                }
            }

            if voice.velocity != 256 {
                mml += "v";
                mml += &itos((voice.velocity >> voice.velocity_shift) as i64);
            }
            if voice.expression != 128 {
                mml += "x";
                mml += &itos(voice.expression as i64);
            }
        }

        mml
    }

    /// C++ `PARSE_ARGUMENT(m_index, m_default)` from translator_util.cpp.
    fn parse_arg(args: &[String], p_index: usize, p_default: i32) -> i32 {
        if p_index < args.len() && !args[p_index].is_empty() {
            to_int(&args[p_index]) as i32
        } else {
            p_default
        }
    }

    pub fn parse_sampler_wave(
        p_table: &Rc<RefCell<SiopmWaveSamplerTable>>,
        p_note_number: i32,
        p_mml: &str,
        p_sound_ref_table: &HashMap<String, Rc<RefCell<SampleData>>>,
    ) -> bool {
        let args = split_string_by_regex(p_mml, "\\s*,\\s*");
        err_fail_cond_v!(args.len() as i32 == 0, "args.size() == 0", false);

        let wave_id = &args[0];
        if !p_sound_ref_table.contains_key(wave_id) {
            return false;
        }

        let ignore_note_off = Self::parse_arg(&args, 1, 0) != 0;
        let pan = Self::parse_arg(&args, 2, 0);
        let channel_count = Self::parse_arg(&args, 3, 2);
        let start_point = Self::parse_arg(&args, 4, -1);
        let end_point = Self::parse_arg(&args, 5, -1);
        let loop_point = Self::parse_arg(&args, 6, -1);

        let source = p_sound_ref_table[wave_id].borrow().clone();
        let sampler_data = Rc::new(RefCell::new(SiopmWaveSamplerData::new(
            &source,
            ignore_note_off,
            pan,
            2,
            channel_count,
        )));
        sampler_data
            .borrow_mut()
            .slice(start_point, end_point, loop_point);
        p_table
            .borrow_mut()
            .set_sample(&Some(sampler_data), p_note_number, -1);

        true
    }

    pub fn parse_pcm_wave(
        p_table: &Rc<RefCell<SiopmWavePcmTable>>,
        p_mml: &str,
        p_sound_ref_table: &HashMap<String, Rc<RefCell<SampleData>>>,
    ) -> bool {
        let args = split_string_by_regex(p_mml, "\\s*,\\s*");
        err_fail_cond_v!(args.len() as i32 == 0, "args.size() == 0", false);

        let wave_id = &args[0];
        if !p_sound_ref_table.contains_key(wave_id) {
            return false;
        }

        let sampling_pitch = Self::parse_arg(&args, 1, 69) * 64;
        let key_range_from = Self::parse_arg(&args, 2, 0);
        let key_range_to = Self::parse_arg(&args, 3, 127);
        let channel_count = Self::parse_arg(&args, 4, 2);
        let start_point = Self::parse_arg(&args, 5, -1);
        let end_point = Self::parse_arg(&args, 6, -1);
        let loop_point = Self::parse_arg(&args, 7, -1);

        let source = p_sound_ref_table[wave_id].borrow().clone();
        let pcm_data = Rc::new(RefCell::new(SiopmWavePcmData::new(
            &source,
            sampling_pitch,
            2,
            channel_count,
        )));
        pcm_data.borrow_mut().slice(start_point, end_point, loop_point);
        p_table
            .borrow_mut()
            .set_key_range_data(&Some(pcm_data), key_range_from, key_range_to);

        true
    }

    /// C++ `parse_pcm_voice(const Ref<SiMMLVoice>&, sion::String,
    /// sion::String, std::vector<Ref<SiMMLEnvelopeTable>>)` — the
    /// `#PCMVOICE` body: key-scale volume/pan tables, the operator-0 EG
    /// envelope and the trailing voice-setting postfix. A voice whose
    /// `wave_data` is not a PCM table behaves like the C++ null
    /// downcast (`false`).
    pub fn parse_pcm_voice(
        p_voice: &Rc<RefCell<crate::sequencer::voice::SiMMLVoice>>,
        p_mml: &str,
        p_postfix: &str,
        p_envelopes: Vec<Option<Rc<RefCell<crate::sequencer::envelope_table::SiMMLEnvelopeTable>>>>,
    ) -> bool {
        let table = match p_voice.borrow().wave_data.as_ref() {
            Some(wave) => {
                match wave.downcast_ref::<Rc<RefCell<SiopmWavePcmTable>>>() {
                    Some(table) => table.clone(),
                    None => return false,
                }
            }
            None => return false,
        };

        let args = split_string_by_regex(p_mml, "\\s*,\\s*");

        let volume_note_number = Self::parse_arg(&args, 0, 64);
        let volume_key_range = Self::parse_arg(&args, 1, 0);
        let volume_range = Self::parse_arg(&args, 2, 0);
        let pan_note_number = Self::parse_arg(&args, 3, 64);
        let pan_key_range = Self::parse_arg(&args, 4, 0);
        let pan_width = Self::parse_arg(&args, 5, 0);
        let attack_rate = Self::parse_arg(&args, 6, 63);
        let decay_rate = Self::parse_arg(&args, 7, 0);
        let sustain_rate = Self::parse_arg(&args, 8, 0);
        let release_rate = Self::parse_arg(&args, 9, 63);
        let sustain_level = Self::parse_arg(&args, 10, 0);

        let op_params = p_voice.borrow().channel_params.borrow().operator_params[0].clone();
        {
            let mut op_params = op_params.borrow_mut();
            op_params.attack_rate = attack_rate;
            op_params.decay_rate = decay_rate;
            op_params.sustain_rate = sustain_rate;
            op_params.release_rate = release_rate;
            op_params.sustain_level = sustain_level;
        }

        table.borrow_mut().set_key_scale_volume(
            volume_note_number,
            volume_key_range as f64,
            volume_range as f64,
        );
        table.borrow_mut().set_key_scale_pan(
            pan_note_number,
            pan_key_range as f64,
            pan_width as f64,
        );
        Self::parse_voice_setting(p_voice, p_postfix, p_envelopes);

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(table: &str, postfix: &str, max: i32, take: usize) -> (Vec<i32>, i32, bool) {
        let parsed = TranslatorUtil::parse_table_numbers(table, postfix, max);
        let mut values = Vec::new();
        let mut data = parsed.data;
        data.front();
        while values.len() < take {
            let Some(v) = data.get() else {
                break;
            };
            values.push(v);
            data.next();
        }
        (values, parsed.length, parsed.repeated)
    }

    #[test]
    fn parse_table_numbers_interpolates_with_postfix() {
        // Linear ramp 0..63 to 4 steps, coef x2 from the postfix. Two-stage
        // C++ rounding: (interp + 0.5) then (value * 2 + 0.5):
        // t=0/0.25/0.5/0.75 -> 0, 16->32, 32->64, 47->94.
        let (values, length, repeated) = collect("(0,63),4", "*2", 65536, 1024);
        assert_eq!(values, vec![0, 32, 64, 94]);
        assert_eq!(length, 4);
        assert!(!repeated);
    }

    #[test]
    fn parse_table_numbers_single_and_loop_tokens() {
        // The `|` token sets the repeat point; after the tail the cursor
        // wraps back to the repeat element (100 forever).
        let (values, length, repeated) = collect("0 | 100", "", 65536, 8);
        assert_eq!(values, vec![0, 100, 100, 100, 100, 100, 100, 100]);
        assert_eq!(length, 2);
        assert!(repeated);

        // `[...]2` duplicates the bracketed segment twice more (loop
        // expansion does not advance `index`, so length stays 3).
        let (values, length, _repeated) = collect("1 [2,3]2", "", 65536, 1024);
        assert_eq!(values, vec![1, 2, 3, 2, 3, 2, 3]);
        assert_eq!(length, 3);
    }

    #[test]
    fn parse_wav_clamps_and_zero_fills() {
        // Interpolated ramp 0,16,32,47 -> (v + 0.5) / 128 with clamping.
        let mut wav = Vec::new();
        TranslatorUtil::parse_wav("(0,63),4", "", &mut wav);
        assert_eq!(
            wav,
            vec![0.00390625, 0.12890625, 0.25390625, 0.37109375]
        );

        // Values outside [-128,127] clamp to the full scale.
        let mut clamped = Vec::new();
        TranslatorUtil::parse_wav("200", "", &mut clamped);
        assert_eq!(clamped, vec![1.0, 0.0]);
    }

    #[test]
    fn parse_wavb_decodes_signed_hex_pairs() {
        let mut wav = Vec::new();
        TranslatorUtil::parse_wavb("00 01 40 7F 80 FF", &mut wav);
        assert_eq!(
            wav,
            vec![
                0.0,
                0.0078125,
                0.5,
                127.0 * 0.0078125,
                -1.0,
                -0.0078125
            ]
        );
    }

    #[test]
    fn siopm_params_round_trip() {
        let mut params = ChannelParams::new();
        let data: Vec<i32> = vec![
            0, 3, 1, // AL, FB, FC
            10, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 40, // op 0
            32, 1, 2, 3, 4, 5, 6, 7, 8, 1, 0, 1, 2, 0, 20, // op 1
        ];

        TranslatorUtil::set_siopm_params(&mut params, data.clone());
        assert_eq!(params.operator_count, 2);
        assert_eq!(params.feedback, 3);
        assert_eq!(
            params.operator_params[0].borrow().fixed_pitch,
            40 << 6
        );

        // Not an identity round trip: the setters loop-sanitize KR/KL/D1/AM
        // (7%4=3, 8%4=0, 10%8=2, 12%4=0), everything else passes through.
        let out = TranslatorUtil::get_siopm_params(&params);
        assert_eq!(
            out,
            vec![
                0, 3, 1, // AL, FB, FC
                10, 1, 2, 3, 4, 5, 6, 3, 0, 9, 2, 11, 0, 13, 40, // op 0
                32, 1, 2, 3, 4, 5, 6, 3, 0, 1, 0, 1, 2, 0, 20, // op 1
            ]
        );
    }

    #[test]
    fn parse_opn_params_string_path() {
        let mut params = ChannelParams::new();
        // 2 channel + 2 x 10 operator parameters.
        TranslatorUtil::parse_opn_params(
            &mut params,
            "/* c */ 3, 7, 31,0,0,15,15,100,3,1,0,1, 31,0,0,15,15,100,3,1,0,1 // t",
        );

        assert_eq!(params.operator_count, 2);
        // ALGORITHM_OPM[1][3] == 1.
        assert_eq!(params.algorithm, 1);
        assert_eq!(params.feedback, 7);

        let op = params.operator_params[0].borrow();
        assert_eq!(op.attack_rate, 62);
        assert_eq!(op.decay_rate, 0);
        assert_eq!(op.sustain_rate, 0);
        assert_eq!(op.release_rate, (15 << 2) + 2);
        assert_eq!(op.sustain_level, 15);
        assert_eq!(op.total_level, 100);
        assert_eq!(op.key_scaling_rate, 3);
        assert_eq!(op.get_multiple(), 1);
        assert_eq!(op.detune1, 0);
        assert_eq!(op.amplitude_modulation_shift, 1);
        drop(op);

        // Round-trip through the MML generator keeps the data identical.
        let mml = TranslatorUtil::get_opn_params_as_mml(&params, " ", "\n", "test");
        // `_get_algorithm_index` returns the FIRST row-1 entry equal to the
        // stored algorithm, so alg 3 re-serializes as 1 (upstream collapse).
        assert!(mml.starts_with("{1 7 // test\n"));
        assert!(mml.contains("// test"));
        assert!(mml.ends_with('}'));
    }

    #[test]
    fn ma3_wave_shape_mapping() {
        assert_eq!(TranslatorUtil::get_ma3_from_pg_type(0, "#MA@"), 0);
        assert_eq!(
            TranslatorUtil::get_ma3_from_pg_type(PULSE_MA3_SINE + 5, "#MA@"),
            5
        );
        assert_eq!(
            TranslatorUtil::get_ma3_from_pg_type(PULSE_CUSTOM + 100, "#MA@"),
            100
        );
        assert_eq!(TranslatorUtil::get_ma3_from_pg_type(72, "#MA@"), 6);
        assert_eq!(TranslatorUtil::get_ma3_from_pg_type(100, "#MA@"), -1);
    }

    #[test]
    fn parse_arg_defaults_empty_entries() {
        let args: Vec<String> = vec![String::new(), "12".to_string()];
        assert_eq!(TranslatorUtil::parse_arg(&args, 0, 7), 7);
        assert_eq!(TranslatorUtil::parse_arg(&args, 1, 7), 12);
        assert_eq!(TranslatorUtil::parse_arg(&args, 2, 7), 7);
    }

    #[test]
    fn split_data_string_tokenizes_comments_and_negatives() {
        let mut params = ChannelParams::new();
        let mml = "  3, 7 /* alg fb */ , 31,0,0,15,15,100,3,1,0,1\r\n 31,0,0,15,15,100,3,1,0,-1 // tail\n";

        // Block comment stripped mid-string, line comment (needs the
        // trailing newline) stripped, edges cleanup-trimmed, '-' kept:
        // exactly 2 + 2 x 10 tokens.
        let data = TranslatorUtil::split_data_string(&mut params, mml, 2, 10, "#OPN@");
        assert_eq!(params.operator_count, 2);
        assert_eq!(
            data,
            vec![
                3, 7, //
                31, 0, 0, 15, 15, 100, 3, 1, 0, 1, //
                31, 0, 0, 15, 15, 100, 3, 1, 0, -1,
            ]
        );

        // Full parse path on the same string: rates shift, AM -1 loops to 3.
        TranslatorUtil::parse_opn_params(&mut params, mml);
        assert_eq!(params.algorithm, 1);
        assert_eq!(params.feedback, 7);
        let op = params.operator_params[1].borrow();
        assert_eq!(op.attack_rate, 62);
        assert_eq!(op.release_rate, (15 << 2) + 2);
        assert_eq!(op.total_level, 100);
        assert_eq!(op.amplitude_modulation_shift, -1 & 3);
        drop(op);
    }

    #[test]
    fn split_data_string_preserves_negative_passthrough_and_specials() {
        let mut params = ChannelParams::new();
        let mml = " 0 7 3 10,63,63,0,63,15,100,3,3,15,7,-1,3,-1,20 ";
        let data = TranslatorUtil::split_data_string(&mut params, mml, 3, 15, "#@");

        assert_eq!(params.operator_count, 1);
        assert_eq!(data.len(), 18);
        assert_eq!(data[14], -1); // D2 slot: raw pass-through token
        assert_eq!(data[16], -1); // PH slot: -1 survives tokenizing

        TranslatorUtil::set_siopm_params(&mut params, data);
        let op = params.operator_params[0].borrow();
        assert_eq!(op.detune2, -1); // raw, no sanitizer
        assert_eq!(op.initial_phase, -1); // -1 special case of the loop clamp
        assert_eq!(op.fixed_pitch, 20 << 6);
        assert_eq!(op.amplitude_modulation_shift, 3);
        assert_eq!(op.attack_rate, 63);
        assert_eq!(op.total_level, 100);
        drop(op);
    }

    #[test]
    fn split_data_string_empty_and_invalid_counts() {
        let mut params = ChannelParams::new();
        params.set_operator_count(2);
        let data = TranslatorUtil::split_data_string(&mut params, "", 2, 10, "#OPN@");
        assert!(data.is_empty());
        assert_eq!(params.operator_count, 0);

        // 4 tokens never matches 3 + 15*i: error path returns empty and
        // leaves operator_count untouched (C++ ERR_FAIL_V_MSG before set;
        // ChannelParams::new() initializes the count to 1).
        let mut params = ChannelParams::new();
        let data = TranslatorUtil::split_data_string(&mut params, "1 2 3 4", 3, 15, "#@");
        assert!(data.is_empty());
        assert_eq!(params.operator_count, 1);
    }

    #[test]
    fn parse_voice_setting_module_header_golden() {
        let voice = Rc::new(RefCell::new(crate::sequencer::voice::SiMMLVoice::new()));
        TranslatorUtil::parse_voice_setting(
            &voice,
            "%v3,2@f100,5,1,2,3,4,5,6,7,8k-3kt1po20q8v64x100p2@p70@er1",
            Vec::new(),
        );

        let voice = voice.borrow();
        let params = voice.channel_params.borrow();

        assert_eq!(voice.velocity_mode, 3);
        assert_eq!(voice.velocity_shift, 2);
        assert_eq!(params.filter_cutoff, 100);
        assert_eq!(params.filter_resonance, 5);
        assert_eq!(params.filter_attack_rate, 1);
        assert_eq!(params.filter_decay_rate1, 2);
        assert_eq!(params.filter_decay_rate2, 3);
        assert_eq!(params.filter_release_rate, 4);
        // Missing arguments keep the per-slot defaults of the macro.
        assert_eq!(voice.portament, 20);
        assert!((voice.default_gate_time - 1.0).abs() < 1e-12);
        // "%v" ran BEFORE "v": the 2-bit shift was already in effect.
        assert_eq!(voice.velocity, 64 << 2);
        assert_eq!(voice.expression, 100);
        assert_eq!(voice.pitch_shift, -3);
        assert_eq!(voice.note_shift, 1);
        // "@p" (fine) came last and overwrote "p2" (coarse 32).
        assert_eq!(params.pan, 70);
        assert!(!params.operator_params[0].borrow().envelope_reset_on_attack);
    }

    #[test]
    fn parse_voice_setting_envelope_indexes_by_match() {
        // C++ quirk: the envelope branches read `p_envelopes[i]` with the
        // MATCH index, not the parsed table number.
        let env0 = Rc::new(RefCell::new(
            crate::sequencer::envelope_table::SiMMLEnvelopeTable::default(),
        ));
        let env1 = Rc::new(RefCell::new(
            crate::sequencer::envelope_table::SiMMLEnvelopeTable::default(),
        ));

        let voice = Rc::new(RefCell::new(crate::sequencer::voice::SiMMLVoice::new()));
        TranslatorUtil::parse_voice_setting(
            &voice,
            "x5nf9",
            vec![Some(env0), Some(env1.clone())],
        );

        let voice = voice.borrow();
        let stored = voice.note_on_filter_envelope.clone().expect("stored");
        assert!(Rc::ptr_eq(&stored, &env1));
        assert_eq!(voice.note_on_filter_envelope_step, 1);
    }

    #[test]
    fn voice_setting_mml_roundtrip() {
        let voice = Rc::new(RefCell::new(crate::sequencer::voice::SiMMLVoice::new()));
        TranslatorUtil::parse_voice_setting(
            &voice,
            "%v3,2@f100,5po20q8@q4,2k-3kt1@v96,32v100x64",
            Vec::new(),
        );
        voice.borrow_mut().update_volumes = true;

        let mml = TranslatorUtil::get_voice_setting_as_mml(&voice);
        assert_eq!(mml, "@f100,5%v3,2po20q8@q4,2k-3kt1@v96,32v100x64");

        let voice2 = Rc::new(RefCell::new(crate::sequencer::voice::SiMMLVoice::new()));
        TranslatorUtil::parse_voice_setting(&voice2, &mml, Vec::new());

        let a = voice.borrow();
        let b = voice2.borrow();
        assert_eq!(a.velocity_mode, b.velocity_mode);
        assert_eq!(a.velocity_shift, b.velocity_shift);
        assert_eq!(a.velocity, b.velocity);
        assert_eq!(a.expression, b.expression);
        assert_eq!(a.portament, b.portament);
        assert!((a.default_gate_time - b.default_gate_time).abs() < 1e-12);
        assert_eq!(a.default_gate_ticks, b.default_gate_ticks);
        assert_eq!(a.default_key_on_delay_ticks, b.default_key_on_delay_ticks);
        assert_eq!(a.pitch_shift, b.pitch_shift);
        assert_eq!(a.note_shift, b.note_shift);
        let pa = a.channel_params.borrow();
        let pb = b.channel_params.borrow();
        assert_eq!(pa.filter_cutoff, pb.filter_cutoff);
        assert_eq!(pa.filter_resonance, pb.filter_resonance);
        assert_eq!(pa.master_volumes[0], pb.master_volumes[0]);
        assert_eq!(pa.master_volumes[1], pb.master_volumes[1]);
        assert_eq!(pa.pan, pb.pan);
    }

    #[test]
    fn parse_pcm_voice_golden_and_null_wave() {
        let voice = crate::sequencer::voice::SiMMLVoice::create_blank_pcm_voice(0);
        let ok = TranslatorUtil::parse_pcm_voice(
            &voice,
            "64,12,30,64,0,16,50,10,5,40,9",
            "%v2",
            Vec::new(),
        );
        assert!(ok);

        let voice = voice.borrow();
        let op0 = voice.channel_params.borrow().operator_params[0].clone();
        let op0 = op0.borrow();
        assert_eq!(op0.attack_rate, 50);
        assert_eq!(op0.decay_rate, 10);
        assert_eq!(op0.sustain_rate, 5);
        assert_eq!(op0.release_rate, 40);
        assert_eq!(op0.sustain_level, 9);
        assert_eq!(voice.velocity_mode, 2);
        assert_eq!(voice.velocity_shift, 4);
        drop(op0);
        drop(voice);

        // A voice without PCM wave data fails like the C++ null downcast.
        let plain = Rc::new(RefCell::new(crate::sequencer::voice::SiMMLVoice::new()));
        assert!(!TranslatorUtil::parse_pcm_voice(&plain, "", "", Vec::new()));
    }
}

