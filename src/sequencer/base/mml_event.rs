//! Port of `libSiON-cpp/src/sequencer/base/mml_event.{h,cpp}` — a single MML
//! event node.
//!
//! In C++ events are pool-allocated by [`crate::sequencer::base::mml_parser`]
//! and chained by raw pointers. In Rust the parser owns an arena
//! (`Vec<MMLEvent>`) and every reference to an event is an arena index
//! ([`MmlEventRef`]); the getters/setters below operate on the record.

use crate::sequencer::base::mml_parser;
use crate::utils::string::itos;

/// Arena index of an [`MMLEvent`] inside the global [`mml_parser`] instance
/// (replaces the C++ `MMLEvent *`).
pub type MmlEventRef = usize;

// Default MML command ids (MMLEvent::EventID).
pub const NO_OP: i32 = 0;
pub const PROCESS: i32 = 1;
pub const REST: i32 = 2;
pub const NOTE: i32 = 3;
// The following 4 are not implemented in the original code:
// LENGTH = 4, TEI = 5, OCTAVE = 6, OCTAVE_SHIFT = 7.
pub const KEY_ON_DELAY: i32 = 8;
pub const QUANT_RATIO: i32 = 9;
pub const QUANT_COUNT: i32 = 10;
pub const VOLUME: i32 = 11;
pub const VOLUME_SHIFT: i32 = 12;
pub const FINE_VOLUME: i32 = 13;
pub const SLUR: i32 = 14;
pub const SLUR_WEAK: i32 = 15;
pub const PITCHBEND: i32 = 16;
pub const REPEAT_BEGIN: i32 = 17;
pub const REPEAT_BREAK: i32 = 18;
pub const REPEAT_END: i32 = 19;
pub const MOD_TYPE: i32 = 20;
pub const MOD_PARAM: i32 = 21;
pub const INPUT_PIPE: i32 = 22;
pub const OUTPUT_PIPE: i32 = 23;
pub const REPEAT_ALL: i32 = 24;
pub const PARAMETER: i32 = 25;
pub const SEQUENCE_HEAD: i32 = 26;
pub const SEQUENCE_TAIL: i32 = 27;
pub const SYSTEM_EVENT: i32 = 28;
pub const TABLE_EVENT: i32 = 29;
pub const GLOBAL_WAIT: i32 = 30;
pub const TEMPO: i32 = 31;
pub const TIMER: i32 = 32;
pub const REGISTER: i32 = 33;
pub const DEBUG_INFO: i32 = 34;
pub const INTERNAL_CALL: i32 = 35;
pub const INTERNAL_WAIT: i32 = 36;
pub const DRIVER_NOTE: i32 = 37;

/// First user defined command.
pub const USER_DEFINED: i32 = 64;

pub const COMMAND_MAX: usize = 128;

#[derive(Clone, Copy, Debug)]
pub struct MMLEvent {
    id: i32,
    data: i32,
    length: i32,

    next: Option<MmlEventRef>,
    /// Repeating event.
    jump: Option<MmlEventRef>,
}

impl MMLEvent {
    pub fn get_id(&self) -> i32 {
        self.id
    }
    pub fn set_id(&mut self, p_value: i32) {
        self.id = p_value;
    }
    pub fn get_data(&self) -> i32 {
        self.data
    }
    pub fn set_data(&mut self, p_value: i32) {
        self.data = p_value;
    }
    pub fn get_length(&self) -> i32 {
        self.length
    }
    pub fn set_length(&mut self, p_value: i32) {
        self.length = p_value;
    }

    pub fn get_next(&self) -> Option<MmlEventRef> {
        self.next
    }
    pub fn set_next(&mut self, p_event: Option<MmlEventRef>) {
        self.next = p_event;
    }
    pub fn get_jump(&self) -> Option<MmlEventRef> {
        self.jump
    }
    pub fn set_jump(&mut self, p_event: Option<MmlEventRef>) {
        self.jump = p_event;
    }

    pub fn initialize(&mut self, p_id: i32, p_data: i32, p_length: i32) {
        self.id = p_id & 0x7f;
        self.data = p_data; // Prefer values below 0xffffff.
        self.length = p_length;

        self.next = None;
        self.jump = None;
    }

    pub fn as_text(&self) -> String {
        "#".to_string()
            + &itos(i64::from(self.id))
            + "{"
            + &itos(i64::from(self.data))
            + ","
            + &itos(i64::from(self.length))
            + "}"
    }

    /// `MMLEvent::_to_string()` — note that the C++ `vformat` passes
    /// `id, length, data` for the `id/data/len` placeholders (upstream
    /// argument order preserved verbatim).
    pub fn to_string_repr(&self) -> String {
        let parser = mml_parser::instance();
        let parser = parser.borrow();

        let next_id = self
            .next
            .map(|e| itos(i64::from(parser.events[e].get_id())))
            .unwrap_or_else(|| "null".to_string());
        let jump_id = self
            .jump
            .map(|e| itos(i64::from(parser.events[e].get_id())))
            .unwrap_or_else(|| "null".to_string());

        let mut chain_str = String::new();
        chain_str += "next=";
        chain_str += &next_id;
        chain_str += ", ";
        chain_str += "jump=";
        chain_str += &jump_id;

        format!(
            "MMLEvent: id={}, data={}, len={}, {}",
            self.id, self.length, self.data, chain_str
        )
    }

    /// `MMLEvent::get_id_from_mml()` — map an MML command letter to its event
    /// id, or 0 when unknown.
    pub fn get_id_from_mml(p_mml: &str) -> i32 {
        if p_mml == "c"
            || p_mml == "d"
            || p_mml == "e"
            || p_mml == "f"
            || p_mml == "g"
            || p_mml == "a"
            || p_mml == "b"
        {
            return NOTE;
        }
        if p_mml == "r" {
            return REST;
        }
        if p_mml == "q" {
            return QUANT_RATIO;
        }
        if p_mml == "@q" {
            return QUANT_COUNT;
        }
        if p_mml == "v" {
            return VOLUME;
        }
        if p_mml == "@v" {
            return FINE_VOLUME;
        }
        if p_mml == "%" {
            return MOD_TYPE;
        }
        if p_mml == "@" {
            return MOD_PARAM;
        }
        if p_mml == "@i" {
            return INPUT_PIPE;
        }
        if p_mml == "@o" {
            return OUTPUT_PIPE;
        }
        if p_mml == "(" || p_mml == ")" {
            return VOLUME_SHIFT;
        }
        if p_mml == "&" {
            return SLUR;
        }
        if p_mml == "&&" {
            return SLUR_WEAK;
        }
        if p_mml == "*" {
            return PITCHBEND;
        }
        if p_mml == "," {
            return PARAMETER;
        }
        if p_mml == "$" {
            return REPEAT_ALL;
        }
        if p_mml == "[" {
            return REPEAT_BEGIN;
        }
        if p_mml == "]" {
            return REPEAT_END;
        }
        if p_mml == "|" {
            return REPEAT_BREAK;
        }
        if p_mml == "t" {
            return TEMPO;
        }

        0
    }

    /// `MMLEvent::get_parameters()` — walk the PARAMETER chain starting at
    /// `start`, filling `r_params` (missing trailing values become
    /// `INT32_MIN`). Returns the last event visited.
    pub fn get_parameters(
        start: MmlEventRef,
        r_params: &mut [i32],
        p_length: i32,
    ) -> MmlEventRef {
        let parser = mml_parser::instance();
        let mut event = start;

        let mut i = 0;
        while i < p_length {
            let (data, next, next_id) = {
                let p = parser.borrow();
                let e = &p.events[event];
                (
                    e.get_data(),
                    e.get_next(),
                    e.get_next().map(|n| p.events[n].get_id()),
                )
            };
            r_params[i as usize] = data;
            i += 1;

            if next.is_none() || next_id != Some(PARAMETER) {
                break;
            }

            event = next.unwrap();
        }
        while i < p_length {
            r_params[i as usize] = i32::MIN;
            i += 1;
        }

        event
    }

    /// `MMLEvent(p_id = 0, p_data = 0, p_length = 0)` — note the constructor
    /// only calls `initialize()` when `p_id > 1`.
    pub fn new(p_id: i32, p_data: i32, p_length: i32) -> Self {
        let mut e = Self {
            id: NO_OP,
            data: 0,
            length: 0,
            next: None,
            jump: None,
        };
        if p_id > 1 {
            e.initialize(p_id, p_data, p_length);
        }
        e
    }
}

impl Default for MMLEvent {
    fn default() -> Self {
        Self::new(0, 0, 0)
    }
}
