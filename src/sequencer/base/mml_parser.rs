//! Port of `libSiON-cpp/src/sequencer/base/mml_parser.{h,cpp}` — the MML
//! compiler singleton.
//!
//! The C++ class is a process-wide singleton owning a pool of `MMLEvent *`.
//! In Rust the pool becomes an arena: [`MMLParser::events`] is a `Vec<MMLEvent>`
//! and every C++ `MMLEvent *` is a [`MmlEventRef`] index into it. The free
//! chain is likewise an index chain threaded through [`MMLEvent::set_next`].
//!
//! `Time::get_singleton()->get_ticks_msec()` (compat `sion_time.h`, a
//! steady-clock millisecond counter) is replaced by [`ticks_msec`] below;
//! only differences are consumed by the C++ code, so an arbitrary Rust
//! `Instant` epoch is compatible.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::Instant;

use regex::{Captures, Regex};

use crate::err_print;
use crate::sequencer::base::mml_event::{self, MMLEvent, MmlEventRef};
use crate::sequencer::base::mml_parser_settings::MMLParserSettings;
use crate::sequencer::base::mml_sequence::MMLSequence;
use crate::utils::string;

// Golden-parity error macros. `compat/sion_errors.h::err_print()` assembles
// the body as `message + "\n" + error`, i.e. for `*_MSG` variants the custom
// message is the FIRST line (the mml-compilation goldens confirm this, e.g.
// `ERROR: MMLParser: Unknown standard event: '/'.`). The shared macros in
// `crate::error` print the condition first, so this module uses the
// `err_print_body` fallback documented in `docs/CONVENTIONS.md` instead.
// See `docs/PENDING-SEQ.md`.

/// C++ `ERR_FAIL_COND_MSG(cond, msg)`.
macro_rules! cpp_err_fail_cond_msg {
    ($cond:expr, $cond_text:expr, $msg:expr) => {
        if $cond {
            $crate::error::err_print_body(
                &format!("{}\nCondition \"{}\" is true.", $msg, $cond_text),
                false,
            );
            return;
        }
    };
}

/// C++ `ERR_FAIL_COND_V_MSG(cond, retval, msg)` — `$retval_text` is the
/// literal stringification (`#m_retval`) of the C++ return expression.
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

/// C++ `ERR_CONTINUE_MSG(cond, msg)`.
macro_rules! cpp_err_continue_msg {
    ($cond:expr, $cond_text:expr, $msg:expr) => {
        if $cond {
            $crate::error::err_print_body(
                &format!("{}\nCondition \"{}\" is true. Continued.", $msg, $cond_text),
                false,
            );
            continue;
        }
    };
}

/// C++ `ERR_FAIL_INDEX_V(index, size, retval)` — the C++ macro never
/// mentions the return value in the message.
macro_rules! cpp_err_fail_index_v {
    ($idx:expr, $i_text:expr, $size:expr, $size_text:expr, $retval:expr) => {
        if $idx < 0 || $idx >= $size {
            $crate::error::err_print_body(
                &format!(
                    "Index {} = {} is out of bounds ({} = {}).",
                    $i_text, $idx, $size_text, $size
                ),
                false,
            );
            return $retval;
        }
    };
}

/// C++ `ERR_FAIL_MSG(msg)`.
macro_rules! cpp_err_fail_msg {
    ($msg:expr) => {{
        $crate::error::err_print_body(&format!("{}\nMethod/function failed.", $msg), false);
        return;
    }};
}

/// C++ `ERR_FAIL_V_MSG(retval, msg)` — `$retval_text` is `#m_retval`.
macro_rules! cpp_err_fail_v_msg {
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

// `MMLParser::MMLRegexIndex` — substring indices into a regex match.
const REX_WHITESPACE: usize = 1;
const REX_SYSTEM: usize = 2;
const REX_NOTE: usize = 4;
const REX_NOTE_SHIFT: usize = 5;
const REX_USER_EVENT: usize = 6;
const REX_EVENT: usize = 7;
const REX_TABLE: usize = 8;
const REX_PARAM: usize = 9;
const REX_PERIOD: usize = 10;

/// `_key_signature_table` — the value of `p_sign` selects one of 15 presets.
const KEY_SIGNATURE_TABLE: [[i32; 7]; 15] = [
    [0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 1, 0, 0, 0],
    [1, 0, 0, 1, 0, 0, 0],
    [1, 0, 0, 1, 1, 0, 0],
    [1, 1, 0, 1, 1, 0, 0],
    [1, 1, 0, 1, 1, 1, 0],
    [1, 1, 1, 1, 1, 1, 0],
    [1, 1, 1, 1, 1, 1, 1],
    [0, 0, 0, 0, 0, 0, -1],
    [0, 0, -1, 0, 0, 0, -1],
    [0, 0, -1, 0, 0, -1, -1],
    [0, -1, -1, 0, 0, -1, -1],
    [0, -1, -1, 0, -1, -1, -1],
    [-1, -1, -1, 0, -1, -1, -1],
    [-1, -1, -1, -1, -1, -1, -1],
];

/// `Time::get_singleton()->get_ticks_msec()` compat shape (see file header).
pub fn ticks_msec() -> i64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_micros() as i64 / 1000
}

/// `RegEx::search_all(subject, offset)` from `compat/sion_regex.cpp`:
/// leftmost matches from `p_offset`, stepping one byte forward past empty
/// matches to avoid looping. Returns full-match byte ranges.
fn search_all(re: &Regex, subject: &str, p_offset: usize) -> Vec<(usize, usize)> {
    let mut matches = Vec::new();
    let mut offset = p_offset;
    while offset <= subject.len() {
        let Some(m) = re.find_at(subject, offset) else {
            break; // No more matches.
        };
        let (start, end) = (m.start(), m.end());
        matches.push((start, end));
        offset = if start == end { end + 1 } else { end };
    }
    matches
}

/// `RegExMatch::get_string(p_group)` — empty string when the group did not
/// participate in the match (PCRE2 `PCRE2_UNSET`).
fn group_str<'c>(caps: &'c Captures<'_>, p_group: usize) -> &'c str {
    caps.get(p_group).map_or("", |m| m.as_str())
}

pub struct MMLParser {
    /// Event arena replacing the `MMLEvent *` pool.
    pub events: Vec<MMLEvent>,
    free_event_chain: Option<MmlEventRef>,

    // Replaces the raw `MMLParserSettings *`; shared with the owning sequencer.
    settings: Option<Rc<RefCell<MMLParserSettings>>>,
    mml_string: String,

    user_defined_event_map: HashMap<String, i32>,
    event_global_flags: Vec<bool>,

    system_event_strings: Vec<String>,
    sequence_mml_strings: Vec<String>,

    // `MMLParser::_create_mml_regex` state.
    mml_regex: Option<Regex>,
    /// Starting offset for subsequent searches (part of the RegExp object in
    /// the original code).
    mml_regex_last_index: usize,

    key_scale: [i32; 7],
    key_signature: [i32; 7],
    key_signature_custom: [i32; 7],

    // Parsing and events.
    system_event_index: i32,
    sequence_mml_index: i32,
    head_mml_index: usize,
    is_last_event_length: bool,

    terminator: MmlEventRef,
    last_event: MmlEventRef,
    last_sequence_head: MmlEventRef,
    repeat_stack: VecDeque<MmlEventRef>,

    // Timers.
    interrupt_interval: i32,
    start_time: i32,
    parsing_time: i32,

    // Static values.
    static_length: i32,
    static_octave: i32,
    static_note_shift: i32,
}

thread_local! {
    static INSTANCE: RefCell<Option<Rc<RefCell<MMLParser>>>> = const { RefCell::new(None) };
}

/// `MMLParser::get_instance()` — lazily initialized like the ref-table
/// singleton pattern in `docs/CONVENTIONS.md`.
pub fn instance() -> Rc<RefCell<MMLParser>> {
    INSTANCE.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Some(Rc::new(RefCell::new(MMLParser::new())));
        }
        slot.as_ref().unwrap().clone()
    })
}

/// `MMLParser::initialize()` — "Sets the instance internally."
pub fn initialize() {
    let _ = instance();
}

/// `MMLParser::finalize()` — C++ deletes the instance and nulls the pointer;
/// the Rust equivalent resets the singleton's contents in place.
pub fn finalize() {
    INSTANCE.with(|cell| {
        if let Some(inst) = cell.borrow().as_ref() {
            *inst.borrow_mut() = MMLParser::new();
        }
    });
}

impl MMLParser {
    /// `MMLParser()` — the constructor registers itself as `_instance`
    /// ("Do this early so it can be self-referenced"); the OnceLock in
    /// [`instance`] plays that role.
    fn new() -> Self {
        let mut parser = Self {
            events: Vec::new(),
            free_event_chain: None,
            settings: None,
            mml_string: String::new(),
            user_defined_event_map: HashMap::new(),
            event_global_flags: Vec::new(),
            // "This is only the initial size, it will grow automatically as needed."
            system_event_strings: vec![String::new(); 32],
            sequence_mml_strings: vec![String::new(); 32],
            mml_regex: None,
            mml_regex_last_index: 0,
            key_scale: [0, 2, 4, 5, 7, 9, 11],
            key_signature: KEY_SIGNATURE_TABLE[0],
            key_signature_custom: [0; 7], // `_key_signature_custom.resize(7)` zeroed
            system_event_index: 0,
            sequence_mml_index: 0,
            head_mml_index: 0,
            is_last_event_length: false,
            terminator: 0,
            last_event: 0,
            last_sequence_head: 0,
            repeat_stack: VecDeque::new(),
            interrupt_interval: 0,
            start_time: 0,
            parsing_time: 0,
            static_length: 0,
            static_octave: 0,
            static_note_shift: 0,
        };
        // `_terminator = new MMLEvent(0);` — index 0 of the arena; the C++
        // constructor skips `initialize()` for id 0, which leaves exactly the
        // default field values.
        parser.events.push(MMLEvent::new(0, 0, 0));
        parser.terminator = 0;
        parser.last_event = 0;
        parser.last_sequence_head = 0;
        parser
    }

    // Settings.

    /// `_create_mml_regex()` — builds the scanner regex dynamically from the
    /// user-defined event letters in the current settings.
    fn create_mml_regex(&mut self, p_reset: bool) {
        if p_reset {
            self.mml_regex_last_index = 0;
        }

        // It must be cleared first, otherwise we won't recreate it.
        if self.mml_regex.is_some() {
            return;
        }

        // We generate a regular expression string based on some setting
        // information. Specifically, we account for user define event letters.
        let mut user_defs: Vec<String> = self.user_defined_event_map.keys().cloned().collect();

        // Here's the part where user definitions are converted to a
        // regex-compatible string. If there are no definitions, we set the
        // string to "a" as a hacky solution.
        let mut user_defs_str = String::from("a");
        if !user_defs.is_empty() {
            user_defs.sort();
            user_defs.reverse(); // We want descending order.

            user_defs_str = string::join("|", &user_defs);
        }

        // Godot's RegEx implementation doesn't support passing global flags,
        // but PCRE2 allows local flags, which we can abuse. (?s) enables
        // single line mode (dot matches newline) for the entire expression.
        // (The `regex` crate already treats `.` as newline-agnostic and the
        // flag is accepted as a no-op, so the text is kept verbatim.)
        let mut reg_string = String::from("(?s)");
        reg_string += "(\\s+)"; // whitespace [1]
        reg_string += "|(#[^;]*)"; // system [2]
        reg_string += "|("; // --all-- [3]
        reg_string += "([a-g])([-+#]?)"; // note [4][5]
        reg_string += "|(";
        reg_string += &user_defs_str; // module events [6]
        reg_string += ")";
        reg_string += "|(@[qvio]?|&&|!@ns|[rlqovt^<>()\\[\\]/|$%&*,;])"; // default events [7]
        reg_string += "|(\\{.*?\\}[0-9]*\\*?[-0-9.]*[-+0-9.]*)"; // table event [8]
        reg_string += ")\\s*(-?[0-9]*)"; // parameter [9]
        reg_string += "\\s*(\\.*)"; // periods [10]

        // On failure the C++ compat shim prints a PCRE2-flavoured message and
        // leaves the Ref null (later dereferences crash); we keep the regex
        // `None` and bail out of `parse()` instead.
        match Regex::new(&reg_string) {
            Ok(re) => self.mml_regex = Some(re),
            Err(err) => {
                err_print!("RegEx: Compilation failed: {}", err);
            }
        }
    }

    fn clear_mml_regex(&mut self) {
        self.mml_regex = None;
        self.mml_regex_last_index = 0;
    }

    pub fn set_user_defined_event_map(&mut self, p_event_map: HashMap<String, i32>) {
        // Original code checks if the map is the same before assigning. This
        // can be expensive and a problem to check for us.
        self.user_defined_event_map = p_event_map;
        self.clear_mml_regex();
    }

    pub fn set_global_event_flags(&mut self, p_event_flags: Vec<bool>) {
        self.event_global_flags = p_event_flags;
    }

    /// `get_command_letters(HashMap<int, sion::String> *r_letter_map)`.
    pub fn get_command_letters(&self, r_letter_map: &mut HashMap<i32, String>) {
        r_letter_map.insert(mml_event::NOTE, "c".into());
        r_letter_map.insert(mml_event::REST, "r".into());
        r_letter_map.insert(mml_event::QUANT_RATIO, "q".into());
        r_letter_map.insert(mml_event::QUANT_COUNT, "@q".into());
        r_letter_map.insert(mml_event::VOLUME, "v".into());
        r_letter_map.insert(mml_event::FINE_VOLUME, "@v".into());
        r_letter_map.insert(mml_event::MOD_TYPE, "%".into());
        r_letter_map.insert(mml_event::MOD_PARAM, "@".into());
        r_letter_map.insert(mml_event::INPUT_PIPE, "@i".into());
        r_letter_map.insert(mml_event::OUTPUT_PIPE, "@o".into());
        r_letter_map.insert(mml_event::VOLUME_SHIFT, "(".into());
        r_letter_map.insert(mml_event::SLUR, "&".into());
        r_letter_map.insert(mml_event::SLUR_WEAK, "&&".into());
        r_letter_map.insert(mml_event::PITCHBEND, "*".into());
        r_letter_map.insert(mml_event::PARAMETER, ",".into());
        r_letter_map.insert(mml_event::REPEAT_ALL, "$".into());
        r_letter_map.insert(mml_event::REPEAT_BEGIN, "[".into());
        r_letter_map.insert(mml_event::REPEAT_END, "]".into());
        r_letter_map.insert(mml_event::REPEAT_BREAK, "|".into());
        r_letter_map.insert(mml_event::TEMPO, "t".into());
    }

    fn register_system_event_string(&mut self, p_event: String) -> i32 {
        if (self.system_event_strings.len() as i32) <= self.system_event_index {
            let len = self.system_event_strings.len();
            self.system_event_strings.resize(len * 2, String::new()); // TODO zeroed
        }

        self.system_event_strings[self.system_event_index as usize] = p_event;
        self.system_event_index += 1;
        self.system_event_index - 1
    }

    pub fn get_system_event_string(&self, p_event: MmlEventRef) -> String {
        let data = self.events[p_event].get_data();
        cpp_err_fail_index_v!(
            data,
            "p_event->get_data()",
            self.system_event_strings.len() as i32,
            "_system_event_strings.size()",
            String::new()
        );

        self.system_event_strings[data as usize].clone()
    }

    fn register_sequence_mml_strings(&mut self, p_mml: String) -> i32 {
        if (self.sequence_mml_strings.len() as i32) <= self.sequence_mml_index {
            let len = self.sequence_mml_strings.len();
            self.sequence_mml_strings.resize(len * 2, String::new()); // TODO zeroed
        }

        self.sequence_mml_strings[self.sequence_mml_index as usize] = p_mml;
        self.sequence_mml_index += 1;
        self.sequence_mml_index - 1
    }

    pub fn get_sequence_mml(&self, p_event: MmlEventRef) -> String {
        let length = self.events[p_event].get_length();
        if length == -1 {
            return String::new();
        }
        cpp_err_fail_index_v!(
            length,
            "p_event->get_length()",
            self.sequence_mml_strings.len() as i32,
            "_sequence_mml_strings.size()",
            String::new()
        );

        self.sequence_mml_strings[length as usize].clone()
    }

    // Key.

    pub fn set_key_signature(&mut self, p_sign: &str) {
        if p_sign.is_empty() {
            self.key_signature = KEY_SIGNATURE_TABLE[0];
            return;
        }

        // Please make sure to keep a sensible chord order below.

        if p_sign == "C" || p_sign == "Am" {
            self.key_signature = KEY_SIGNATURE_TABLE[0];
            return;
        }
        if p_sign == "G" || p_sign == "Em" {
            self.key_signature = KEY_SIGNATURE_TABLE[1];
            return;
        }
        if p_sign == "D" || p_sign == "Bm" {
            self.key_signature = KEY_SIGNATURE_TABLE[2];
            return;
        }
        if p_sign == "A" || p_sign == "F+m" || p_sign == "F#m" {
            self.key_signature = KEY_SIGNATURE_TABLE[3];
            return;
        }
        if p_sign == "E" || p_sign == "C+m" || p_sign == "C#m" {
            self.key_signature = KEY_SIGNATURE_TABLE[4];
            return;
        }
        if p_sign == "B" || p_sign == "G+m" || p_sign == "G#m" {
            self.key_signature = KEY_SIGNATURE_TABLE[5];
            return;
        }
        if p_sign == "F+" || p_sign == "F#" || p_sign == "D+m" || p_sign == "D#m" {
            self.key_signature = KEY_SIGNATURE_TABLE[6];
            return;
        }
        if p_sign == "C+" || p_sign == "C#" || p_sign == "A+m" || p_sign == "A#m" {
            self.key_signature = KEY_SIGNATURE_TABLE[7];
            return;
        }
        if p_sign == "F" || p_sign == "Dm" {
            self.key_signature = KEY_SIGNATURE_TABLE[8];
            return;
        }
        if p_sign == "B-" || p_sign == "Bb" || p_sign == "Gm" {
            self.key_signature = KEY_SIGNATURE_TABLE[9];
            return;
        }
        if p_sign == "E-" || p_sign == "Eb" || p_sign == "Cm" {
            self.key_signature = KEY_SIGNATURE_TABLE[10];
            return;
        }
        if p_sign == "A-" || p_sign == "Ab" || p_sign == "Fm" {
            self.key_signature = KEY_SIGNATURE_TABLE[11];
            return;
        }
        if p_sign == "D-" || p_sign == "Db" || p_sign == "B-m" || p_sign == "Bbm" {
            self.key_signature = KEY_SIGNATURE_TABLE[12];
            return;
        }
        if p_sign == "G-" || p_sign == "Gb" || p_sign == "E-m" || p_sign == "Ebm" {
            self.key_signature = KEY_SIGNATURE_TABLE[13];
            return;
        }
        if p_sign == "C-" || p_sign == "Cb" || p_sign == "A-m" || p_sign == "Abm" {
            self.key_signature = KEY_SIGNATURE_TABLE[14];
            return;
        }

        // Generate a custom signature if there is no match.

        const NOTE_LETTERS: [u8; 7] = [b'c', b'd', b'e', b'f', b'g', b'a', b'b'];

        for i in 0..7 {
            self.key_signature_custom[i] = 0;
        }

        let arr = string::split_string_by_regex(p_sign, "[\\s,]");
        // Note that the original code is broken here (it tries to get the
        // first and the second character on the Array object). I assume that
        // the intention is to check each split substring and parse it as a
        // note in the key table. If there are duplicate notes, then the
        // latter overrides the former. If notes are missing then they are set
        // to 0 in the table.
        for item in &arr {
            let note_sign = string::to_lower(item);
            let note_letter = string::unicode_at(&note_sign, 0);
            let note_idx = NOTE_LETTERS
                .iter()
                .position(|c| *c as i32 == note_letter)
                .map_or(-1, |i| i as i32);
            cpp_err_continue_msg!(
                note_idx == -1,
                "note_idx == -1",
                format!("MMLParser: Cannot recognize '{p_sign}' as a key signature.")
            );

            if note_sign.len() > 1 {
                let note_shift = note_sign.as_bytes()[1];
                if note_shift == b'+' || note_shift == b'#' {
                    self.key_signature_custom[note_idx as usize] = 1;
                } else if note_shift == b'-' || note_shift == b'b' {
                    self.key_signature_custom[note_idx as usize] = -1;
                } else {
                    self.key_signature_custom[note_idx as usize] = 0;
                }
            } else {
                self.key_signature_custom[note_idx as usize] = 0;
            }
        }

        self.key_signature = self.key_signature_custom;
    }

    // Parsing and events.

    fn push_mml_event(&mut self, p_event_id: i32, p_data: i32, p_length: i32) -> MmlEventRef {
        let new_event = self.alloc_event(p_event_id, p_data, p_length);
        self.events[self.last_event].set_next(Some(new_event));
        self.last_event = new_event;

        self.last_event
    }

    fn add_mml_event(
        &mut self,
        p_event_id: i32,
        p_data: i32,
        p_length: i32,
        p_note_option: bool,
    ) -> Option<MmlEventRef> {
        if p_note_option {
            // Note option events are inserted after NOTE.
            cpp_err_fail_cond_v_msg!(
                self.events[self.last_event].get_id() != mml_event::NOTE,
                "_last_event->get_id() != MMLEvent::NOTE",
                None,
                "nullptr",
                "MMLParser: Commands '*' and '&' can only come after a note."
            );
            let length = self.events[self.last_event].get_length();
            self.events[self.last_event].set_length(0);
            self.push_mml_event(p_event_id, p_data, length);
        } else {
            // Create channel data chain.
            if p_event_id == mml_event::SEQUENCE_HEAD {
                let last = self.last_event;
                self.events[self.last_sequence_head].set_jump(Some(last));
                let head = self.push_mml_event(p_event_id, p_data, p_length);
                self.last_sequence_head = head;
                self.reset_state_track();

            // Concatenate REST events.
            } else if p_event_id == mml_event::REST
                && self.events[self.last_event].get_id() == mml_event::REST
            {
                let last = self.last_event;
                let new_length =
                    self.events[last].get_length() + p_length;
                self.events[last].set_length(new_length);

            // Handle normally.
            } else {
                self.push_mml_event(p_event_id, p_data, p_length);
                // Data is the count of global events.
                if *self.event_global_flags.get(p_event_id as usize).unwrap_or(&false) {
                    let data = self.events[self.last_sequence_head].get_data();
                    self.events[self.last_sequence_head].set_data(data + 1);
                }
            }
        }

        self.is_last_event_length = false;
        Some(self.last_event)
    }

    fn reset_state(&mut self) {
        let mut event = self.events[self.terminator].get_next();
        while let Some(e) = event {
            event = self.free_event(e);
        }

        self.system_event_index = 0;
        self.sequence_mml_index = 0;
        self.last_event = self.terminator;
        self.last_sequence_head = self.push_mml_event(mml_event::SEQUENCE_HEAD, 0, 0);

        self.reset_state_track();
    }

    fn reset_state_track(&mut self) {
        let (default_length, default_octave) = {
            let settings = self
                .settings
                .as_ref()
                .expect("MMLParser: settings must be set before parsing");
            let settings = settings.borrow();
            (settings.get_default_length(), settings.get_default_octave())
        };
        self.static_length = default_length;
        self.static_octave = default_octave;
        self.static_note_shift = 0;
        self.is_last_event_length = false;

        self.repeat_stack.clear();
        self.head_mml_index = self.mml_regex_last_index;
    }

    /// `prepare_parse()` — the C++ signature passes a raw settings pointer;
    /// Rust shares the [`MMLParserSettings`] through an `Rc` so live edits
    /// keep the aliasing semantics.
    pub fn prepare_parse(&mut self, p_settings: Rc<RefCell<MMLParserSettings>>, p_mml: String) {
        self.settings = Some(p_settings);
        self.mml_string = p_mml;
        self.parsing_time = ticks_msec() as i32;

        self.create_mml_regex(true);
        self.reset_state();
    }

    fn parse_length(&self, caps: &Captures) -> i32 {
        // This is an abbreviation, return INT32_MIN.
        if group_str(caps, REX_PARAM).is_empty() {
            return i32::MIN;
        }

        let mut length = string::to_int(group_str(caps, REX_PARAM)) as i32;
        if length == 0 {
            return 0;
        }

        let resolution = self
            .settings
            .as_ref()
            .expect("MMLParser: settings must be set before parsing")
            .borrow()
            .resolution;
        length = resolution / length;
        cpp_err_fail_cond_v_msg!(
            length < 1 || length > resolution,
            "length < 1 || length > _settings->resolution",
            0,
            "0",
            format!(
                "MMLParser: Command 'length' has argument ({length}) outside of valid range (1 : {resolution})."
            )
        );

        length
    }

    fn parse_param(&self, caps: &Captures, p_default: i32) -> i32 {
        let param = group_str(caps, REX_PARAM);
        if !param.is_empty() {
            return string::to_int(param) as i32;
        }

        p_default
    }

    fn parse_period(caps: &Captures) -> i32 {
        group_str(caps, REX_PERIOD).len() as i32
    }

    pub fn parse(&mut self, p_interrupt: i32) -> Option<MmlEventRef> {
        self.interrupt_interval = p_interrupt;
        self.start_time = ticks_msec() as i32;

        self.create_mml_regex(false);

        // A null `_mml_regex` dereference would crash C++; bail out instead.
        let re = self.mml_regex.clone()?;
        let subject = self.mml_string.clone();

        // Start parsing.
        let ranges = search_all(&re, &subject, self.mml_regex_last_index);

        for (start, _end) in ranges {
            // Leftmost-first re-find at the recorded start reproduces the
            // original `RegExMatch` capture set.
            let Some(caps) = re.captures_at(&subject, start) else {
                break;
            };
            let whole_end = caps.get(0).map_or(start, |m| m.end());
            let match_string = &subject[start.min(subject.len())..whole_end];
            self.mml_regex_last_index = whole_end + 1;

            if match_string.is_empty() {
                break; // Stop parsing if there is an empty match.
            }

            if !group_str(&caps, REX_WHITESPACE).is_empty() {
                continue; // This is a comment.
            }

            // If this gets set to true, we will exit early with an empty result.
            let mut halt = false;

            // Note events.
            if !group_str(&caps, REX_NOTE).is_empty() {
                // We want to convert the a-g range to the c-b range. We are
                // guaranteed to have letters a through g from the regex, so we
                // subtract the code of C. Then, if we underflow, we correct it
                // by shifting the value by 7.
                let mut note = string::unicode_at(group_str(&caps, REX_NOTE), 0) - i32::from(b'c');
                if note < 0 {
                    note += 7;
                }

                let mut shift = self.key_signature[note as usize];
                let shift_string = group_str(&caps, REX_NOTE_SHIFT);
                if shift_string == "+" || shift_string == "#" {
                    shift += 1;
                } else if shift_string == "-" {
                    shift -= 1;
                }

                let mml_to_note = self
                    .settings
                    .as_ref()
                    .expect("MMLParser: settings must be set before parsing")
                    .borrow()
                    .get_mml_to_note_offset();
                let length = self.parse_length(&caps);
                let period = Self::parse_period(&caps);
                let note_number = self.key_scale[note as usize] + shift + mml_to_note;
                self.op_note(note_number, length, period);

            // User defined events.
            } else if !group_str(&caps, REX_USER_EVENT).is_empty() {
                let event_str = group_str(&caps, REX_USER_EVENT).to_string();
                cpp_err_continue_msg!(
                    !self.user_defined_event_map.contains_key(&event_str),
                    "!_user_defined_event_map.has(event_str)",
                    format!("MMLParser: Unknown user-defined event: '{event_str}'.")
                );
                let param = self.parse_param(&caps, i32::MIN);
                let id = self.user_defined_event_map[&event_str];
                self.add_mml_event(id, param, 0, false);

            // Standard events.
            } else if !group_str(&caps, REX_EVENT).is_empty() {
                let event_str = group_str(&caps, REX_EVENT).to_string();

                // Formatting below is enforced like this for readability.
                let settings_default = {
                    self.settings
                        .as_ref()
                        .expect("MMLParser: settings must be set before parsing")
                        .borrow()
                        .clone()
                };

                // Rest events.
                if event_str == "r" {
                    let length = self.parse_length(&caps);
                    let period = Self::parse_period(&caps);
                    self.op_rest(length, period);
                }
                // Length events.
                else if event_str == "l" {
                    let length = self.parse_length(&caps);
                    let period = Self::parse_period(&caps);
                    self.op_length(length, period);
                } else if event_str == "^" {
                    let length = self.parse_length(&caps);
                    let period = Self::parse_period(&caps);
                    self.op_tie(length, period);
                } else if event_str == "&" {
                    self.op_slur();
                } else if event_str == "&&" {
                    self.op_slur_weak();
                } else if event_str == "*" {
                    self.op_portament();
                } else if event_str == "q" {
                    let value = self.parse_param(&caps, settings_default.default_quant_ratio);
                    self.op_quant(value);
                } else if event_str == "@q" {
                    let value = self.parse_param(&caps, settings_default.default_quant_count);
                    self.op_at_quant(value);
                }
                // Pitch events.
                else if event_str == "o" {
                    let value = self.parse_param(&caps, settings_default.get_default_octave());
                    self.op_octave(value);
                } else if event_str == "<" {
                    let value = self.parse_param(&caps, 1);
                    self.op_octave_shift(value);
                } else if event_str == ">" {
                    let value = self.parse_param(&caps, 1);
                    self.op_octave_shift(-value);
                } else if event_str == "!@ns" {
                    let value = self.parse_param(&caps, 0);
                    self.op_note_shift(value);
                } else if event_str == "v" {
                    let value = self.parse_param(&caps, settings_default.default_volume);
                    self.op_volume(value);
                } else if event_str == "@v" {
                    let value = self.parse_param(&caps, settings_default.default_fine_volume);
                    self.op_at_volume(value);
                } else if event_str == "(" {
                    let value = self.parse_param(&caps, 1);
                    self.op_volume_shift(value);
                } else if event_str == ")" {
                    let value = self.parse_param(&caps, 1);
                    self.op_volume_shift(-value);
                }
                // Repeat events.
                else if event_str == "$" {
                    self.op_repeat_point();
                } else if event_str == "[" {
                    let value = self.parse_param(&caps, 2);
                    self.op_repeat_begin(value);
                } else if event_str == "|" {
                    self.op_repeat_break();
                } else if event_str == "]" {
                    let value = self.parse_param(&caps, i32::MIN);
                    self.op_repeat_end(value);
                }
                // Other events.
                else if event_str == "%" {
                    let value = self.parse_param(&caps, i32::MIN);
                    self.op_mod_type(value);
                } else if event_str == "@" {
                    let value = self.parse_param(&caps, i32::MIN);
                    self.op_mod_param(value);
                } else if event_str == "@i" {
                    let value = self.parse_param(&caps, 0);
                    self.op_input(value);
                } else if event_str == "@o" {
                    let value = self.parse_param(&caps, 0);
                    self.op_output(value);
                } else if event_str == "," {
                    let value = self.parse_param(&caps, i32::MIN);
                    self.op_parameter(value);
                } else if event_str == "t" {
                    // C++ passes the `double` default_bpm to the `int`
                    // p_default parameter — truncating conversion, kept as-is.
                    let value = self.parse_param(&caps, settings_default.default_bpm as i32);
                    self.op_tempo(value);
                } else if event_str == ";" {
                    halt = self.op_end_sequence();
                } else {
                    cpp_err_continue_msg!(
                        true,
                        "true",
                        format!("MMLParser: Unknown standard event: '{event_str}'.")
                    );
                }

            // System events.
            } else if !group_str(&caps, REX_SYSTEM).is_empty() {
                cpp_err_fail_cond_v_msg!(
                    self.events[self.last_event].get_id() != mml_event::SEQUENCE_HEAD,
                    "_last_event->get_id() != MMLEvent::SEQUENCE_HEAD",
                    None,
                    "nullptr",
                    "MMLParser: System commands are only allowed at the top of the channel sequence."
                );

                let system_string = group_str(&caps, REX_SYSTEM).to_string();
                let data = self.register_system_event_string(system_string);
                self.add_mml_event(mml_event::SYSTEM_EVENT, data, 0, false);

            // Table events.
            } else if !group_str(&caps, REX_TABLE).is_empty() {
                let table_string = group_str(&caps, REX_TABLE).to_string();
                let data = self.register_system_event_string(table_string);
                self.add_mml_event(mml_event::TABLE_EVENT, data, 0, false);

            // Invalid syntax.
            } else {
                cpp_err_fail_v_msg!(
                    None,
                    "nullptr",
                    format!("MMLParser: Invalid syntax encountered: '{match_string}'.")
                );
            }

            if halt {
                return None;
            }
        }

        // Done parsing.

        cpp_err_fail_cond_v_msg!(
            !self.repeat_stack.is_empty(),
            "_repeat_stack.size() != 0",
            None,
            "nullptr",
            "MMLParser: Too many items in the repeat stack for command '['."
        );

        if self.events[self.last_event].get_id() != mml_event::SEQUENCE_HEAD {
            let last = self.last_event;
            self.events[self.last_sequence_head].set_jump(Some(last));
        }

        self.parsing_time = ticks_msec() as i32 - self.parsing_time;

        let head_event = self.events[self.terminator].get_next();
        self.events[self.terminator].set_next(None);

        head_event
    }

    pub fn get_parse_progress(&self) -> f64 {
        if self.mml_string.is_empty() {
            return 0.0;
        }

        self.mml_regex_last_index as f64 / (string::length(&self.mml_string) + 1) as f64
    }

    pub fn alloc_event(&mut self, p_event_id: i32, p_data: i32, p_length: i32) -> MmlEventRef {
        let event = if let Some(head) = self.free_event_chain.take() {
            self.free_event_chain = self.events[head].get_next();
            head
        } else {
            self.events.push(MMLEvent::new(0, 0, 0));
            self.events.len() - 1
        };

        self.events[event].initialize(p_event_id, p_data, p_length);
        event
    }

    pub fn free_event(&mut self, p_event: MmlEventRef) -> Option<MmlEventRef> {
        let next = self.events[p_event].get_next();
        self.events[p_event].set_next(self.free_event_chain);
        self.free_event_chain = Some(p_event);

        next
    }

    pub fn free_all_events(&mut self, p_sequence: &mut MMLSequence) {
        let (Some(head_event), Some(tail_event)) = (p_sequence.get_head_event(), p_sequence.get_tail_event()) else {
            return;
        };
        p_sequence.set_head_event(None);
        p_sequence.set_tail_event(None);

        let jump = self.events[head_event].get_jump();
        self.events[jump.expect("MMLParser: sequence head must have a jump")].set_next(Some(tail_event));
        self.events[tail_event].set_next(self.free_event_chain);
        self.free_event_chain = Some(head_event);
    }

    // Parsing operations.

    fn calculate_length(&self, p_length: i32, p_period: i32) -> i32 {
        let mut length = p_length;
        if length == i32::MIN {
            length = self.static_length;
        }

        let length_step = length;
        let mut period = p_period;
        while period > 0 {
            length += length_step >> period;
            period -= 1;
        }

        length
    }

    /// Note operations.

    fn op_note(&mut self, p_note: i32, p_length: i32, p_period: i32) {
        let note = p_note + self.static_octave * 12 + self.static_note_shift;
        let note = crate::math::clampi(note, 0, 127);

        let length = self.calculate_length(p_length, p_period);
        self.add_mml_event(mml_event::NOTE, note, length, false);
    }

    fn op_rest(&mut self, p_length: i32, p_period: i32) {
        let length = self.calculate_length(p_length, p_period);
        self.add_mml_event(mml_event::REST, 0, length, false);
    }

    /// Length operations.

    fn op_length(&mut self, p_length: i32, p_period: i32) {
        self.static_length = self.calculate_length(p_length, p_period);
        self.is_last_event_length = true;
    }

    fn op_tie(&mut self, p_length: i32, p_period: i32) {
        if self.is_last_event_length {
            self.static_length += self.calculate_length(p_length, p_period);
        } else {
            let last_id = self.events[self.last_event].get_id();
            if last_id == mml_event::REST || last_id == mml_event::NOTE {
                let add = self.calculate_length(p_length, p_period);
                let last = self.last_event;
                let new_length = self.events[last].get_length() + add;
                self.events[last].set_length(new_length);
            } else {
                cpp_err_fail_msg!("MMLParser: Invalid tie command syntax.");
            }
        }
    }

    fn op_slur(&mut self) {
        self.add_mml_event(mml_event::SLUR, 0, 0, true);
    }

    fn op_slur_weak(&mut self) {
        self.add_mml_event(mml_event::SLUR_WEAK, 0, 0, true);
    }

    fn op_portament(&mut self) {
        self.add_mml_event(mml_event::PITCHBEND, 0, 0, true);
    }

    fn op_quant(&mut self, p_value: i32) {
        let (min, max) = {
            let s = self
                .settings
                .as_ref()
                .expect("MMLParser: settings must be set before parsing")
                .borrow();
            (s.min_quant_ratio, s.max_quant_ratio)
        };
        cpp_err_fail_cond_msg!(
            p_value < min || p_value > max,
            "p_value < _settings->min_quant_ratio || p_value > _settings->max_quant_ratio",
            format!(
                "MMLParser: Command 'q' has argument ({p_value}) outside of valid range ({min} : {max})."
            )
        );
        self.add_mml_event(mml_event::QUANT_RATIO, p_value, 0, false);
    }

    fn op_at_quant(&mut self, p_value: i32) {
        let (min, max) = {
            let s = self
                .settings
                .as_ref()
                .expect("MMLParser: settings must be set before parsing")
                .borrow();
            (s.min_quant_count, s.max_quant_count)
        };
        cpp_err_fail_cond_msg!(
            p_value < min || p_value > max,
            "p_value < _settings->min_quant_count || p_value > _settings->max_quant_count",
            format!(
                "MMLParser: Command '@q' has argument ({p_value}) outside of valid range ({min} : {max})."
            )
        );
        self.add_mml_event(mml_event::QUANT_COUNT, p_value, 0, false);
    }

    /// Pitch operations.

    fn op_octave(&mut self, p_value: i32) {
        let (min, max) = {
            let s = self
                .settings
                .as_ref()
                .expect("MMLParser: settings must be set before parsing")
                .borrow();
            (s.min_octave, s.max_octave)
        };
        cpp_err_fail_cond_msg!(
            p_value < min || p_value > max,
            "p_value < _settings->min_octave || p_value > _settings->max_octave",
            format!(
                "MMLParser: Command 'o' has argument ({p_value}) outside of valid range ({min} : {max})."
            )
        );
        self.static_octave = p_value;
    }

    fn op_octave_shift(&mut self, p_value: i32) {
        let polarization = self
            .settings
            .as_ref()
            .expect("MMLParser: settings must be set before parsing")
            .borrow()
            .octave_polarization;
        self.static_octave += p_value * polarization;
    }

    fn op_note_shift(&mut self, p_value: i32) {
        self.static_note_shift += p_value;
    }

    fn op_volume(&mut self, p_value: i32) {
        let max = self
            .settings
            .as_ref()
            .expect("MMLParser: settings must be set before parsing")
            .borrow()
            .max_volume;
        cpp_err_fail_cond_msg!(
            p_value < 0 || p_value > max,
            "p_value < 0 || p_value > _settings->max_volume",
            format!(
                "MMLParser: Command 'v' has argument ({p_value}) outside of valid range (0 : {max})."
            )
        );
        self.add_mml_event(mml_event::VOLUME, p_value, 0, false);
    }

    fn op_at_volume(&mut self, p_value: i32) {
        let max = self
            .settings
            .as_ref()
            .expect("MMLParser: settings must be set before parsing")
            .borrow()
            .max_fine_volume;
        cpp_err_fail_cond_msg!(
            p_value < 0 || p_value > max,
            "p_value < 0 || p_value > _settings->max_fine_volume",
            format!(
                "MMLParser: Command '@v' has argument ({p_value}) outside of valid range (0 : {max})."
            )
        );
        self.add_mml_event(mml_event::FINE_VOLUME, p_value, 0, false);
    }

    fn op_volume_shift(&mut self, p_value: i32) {
        let polarization = self
            .settings
            .as_ref()
            .expect("MMLParser: settings must be set before parsing")
            .borrow()
            .volume_polarization;
        let value = p_value * polarization;

        let last_id = self.events[self.last_event].get_id();
        if last_id == mml_event::VOLUME_SHIFT || last_id == mml_event::VOLUME {
            let data = self.events[self.last_event].get_data();
            self.events[self.last_event].set_data(data + value);
        } else {
            self.add_mml_event(mml_event::VOLUME_SHIFT, value, 0, false);
        }
    }

    /// Repeat operations.

    fn op_repeat_point(&mut self) {
        self.add_mml_event(mml_event::REPEAT_ALL, 0, 0, false);
    }

    fn op_repeat_begin(&mut self, p_count: i32) {
        cpp_err_fail_cond_msg!(
            p_count < 1 || p_count > 65535,
            "p_count < 1 || p_count > 65535",
            format!(
                "MMLParser: Command '[' has argument ({p_count}) outside of valid range (1 : 65535)."
            )
        );
        self.add_mml_event(mml_event::REPEAT_BEGIN, p_count, 0, false);
        let last = self.last_event;
        self.repeat_stack.push_front(last);
    }

    fn op_repeat_break(&mut self) {
        cpp_err_fail_cond_msg!(
            self.repeat_stack.is_empty(),
            "_repeat_stack.size() == 0",
            "MMLParser: Not enough items in the repeat stack for command '|'."
        );
        self.add_mml_event(mml_event::REPEAT_BREAK, 0, 0, false);
        let begin_event = self.repeat_stack[0];
        let last = self.last_event;
        self.events[last].set_jump(Some(begin_event));
    }

    fn op_repeat_end(&mut self, p_count: i32) {
        cpp_err_fail_cond_msg!(
            self.repeat_stack.is_empty(),
            "_repeat_stack.size() == 0",
            "MMLParser: Not enough items in the repeat stack for command ']'."
        );
        self.add_mml_event(mml_event::REPEAT_END, 0, 0, false);

        let begin_event = self.repeat_stack.pop_front().expect("repeat stack");

        let last = self.last_event;
        self.events[last].set_jump(Some(begin_event));
        self.events[begin_event].set_jump(Some(last));

        if p_count != i32::MIN {
            cpp_err_fail_cond_msg!(
                p_count < 1 || p_count > 65535,
                "p_count < 1 || p_count > 65535",
                format!(
                    "MMLParser: Command ']' has argument ({p_count}) outside of valid range (1 : 65535)."
                )
            );
            self.events[begin_event].set_data(p_count);
        }
    }

    /// Other operations.

    fn op_mod_type(&mut self, p_type: i32) {
        self.add_mml_event(mml_event::MOD_TYPE, p_type, 0, false);
    }

    fn op_mod_param(&mut self, p_param: i32) {
        self.add_mml_event(mml_event::MOD_PARAM, p_param, 0, false);
    }

    fn op_input(&mut self, p_pipe: i32) {
        self.add_mml_event(mml_event::INPUT_PIPE, p_pipe, 0, false);
    }

    fn op_output(&mut self, p_pipe: i32) {
        self.add_mml_event(mml_event::OUTPUT_PIPE, p_pipe, 0, false);
    }

    fn op_parameter(&mut self, p_param: i32) {
        self.add_mml_event(mml_event::PARAMETER, p_param, 0, false);
    }

    fn op_tempo(&mut self, p_tempo: i32) {
        self.add_mml_event(mml_event::TEMPO, p_tempo, 0, false);
    }

    fn op_end_sequence(&mut self) -> bool {
        // This method returns true when it's a good moment to split parsing
        // into different frames. Setting _interrupt_interval to 0 disables
        // this behavior.

        if self.events[self.last_event].get_id() == mml_event::SEQUENCE_HEAD {
            return false; // Last sequence was empty, continue parsing the next one immediately.
        }

        let next_event = self.events[self.last_sequence_head].get_next();
        if let Some(next_event) = next_event {
            if self.events[next_event].get_id() == mml_event::DEBUG_INFO {
                // NOTE: the C++ passes `from + index` as the substr LENGTH —
                // reproduced verbatim, not "fixed".
                let mml = string::substr(
                    &self.mml_string,
                    self.head_mml_index,
                    self.head_mml_index + self.mml_regex_last_index,
                );
                let index = self.register_sequence_mml_strings(mml);
                self.events[next_event].set_data(index);
            }
        }

        self.add_mml_event(mml_event::SEQUENCE_HEAD, 0, 0, false);

        if self.interrupt_interval == 0 {
            return false;
        }
        self.interrupt_interval < (ticks_msec() as i32 - self.start_time)
    }
}
