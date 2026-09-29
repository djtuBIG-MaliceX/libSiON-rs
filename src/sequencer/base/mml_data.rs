//! Port of `libSiON-cpp/src/sequencer/base/mml_data.{h,cpp}` — compiled MML
//! data: sequence group, global sequence, BPM settings and system commands.

use std::cell::RefCell;
use std::rc::Rc;

use crate::sequencer::base::beats_per_minute::BeatsPerMinute;
use crate::sequencer::base::mml_event::MmlEventRef;
use crate::sequencer::base::mml_sequence::{MMLSequence, SeqRc};
use crate::sequencer::base::mml_sequence_group::MMLSequenceGroup;
use crate::sequencer::base::mml_system_command::MMLSystemCommand;

// Controls what tcommand argument is.
// Variant names mirror the C++ `SiONMMLTCommandMode` enumerators verbatim.
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TCommandMode {
    /// BPM.
    TCOMMAND_BPM = 0,
    /// OPNA's TIMERB with 48 ticks/beat.
    TCOMMAND_TIMERB = 1,
    /// Frame count.
    TCOMMAND_FRAME = 2,
}

pub struct MMLData {
    sequence_group: MMLSequenceGroup,
    // C++ `MMLSequence *_global_sequence`; always live (constructor-made).
    global_sequence: SeqRc,

    title: String,
    #[allow(dead_code)] // C++ `_author` has no getter/setter either.
    author: String,

    default_fps: i32,
    tcommand_mode: TCommandMode,
    tcommand_resolution: f64,

    default_velocity_shift: i32,
    default_velocity_mode: i32,
    default_expression_mode: i32,

    initial_bpm: Option<Rc<RefCell<BeatsPerMinute>>>,
    // System commands that cannot be parsed by the system.
    system_commands: Vec<Rc<RefCell<MMLSystemCommand>>>,
}

impl MMLData {
    pub fn get_title(&self) -> String {
        self.title.clone()
    }

    pub fn set_title(&mut self, p_title: String) {
        self.title = p_title;
    }

    pub fn get_default_fps(&self) -> i32 {
        self.default_fps
    }

    pub fn set_default_fps(&mut self, p_value: i32) {
        self.default_fps = p_value;
    }

    pub fn set_tcommand_mode(&mut self, p_mode: TCommandMode) {
        self.tcommand_mode = p_mode;
    }

    pub fn set_tcommand_resolution(&mut self, p_value: f64) {
        self.tcommand_resolution = p_value;
    }

    pub fn get_default_velocity_shift(&self) -> i32 {
        self.default_velocity_shift
    }

    pub fn set_default_velocity_shift(&mut self, p_value: i32) {
        self.default_velocity_shift = p_value;
    }

    pub fn get_default_velocity_mode(&self) -> i32 {
        self.default_velocity_mode
    }

    pub fn set_default_velocity_mode(&mut self, p_value: i32) {
        self.default_velocity_mode = p_value;
    }

    pub fn get_default_expression_mode(&self) -> i32 {
        self.default_expression_mode
    }

    pub fn set_default_expression_mode(&mut self, p_value: i32) {
        self.default_expression_mode = p_value;
    }

    pub fn get_bpm(&self) -> f64 {
        match &self.initial_bpm {
            Some(bpm) => bpm.borrow().get_bpm(),
            None => 0.0,
        }
    }

    /// Setting this to 0 makes data dependent on the driver's BPM.
    pub fn set_bpm(&mut self, p_value: f64) {
        if p_value > 0.0 {
            self.initial_bpm = Some(Rc::new(RefCell::new(BeatsPerMinute::new(
                p_value,
                44100,
                1920,
            ))));
        } else {
            self.initial_bpm = Some(Rc::new(RefCell::new(BeatsPerMinute::default())));
        }
    }

    pub fn get_bpm_settings(&self) -> Option<Rc<RefCell<BeatsPerMinute>>> {
        self.initial_bpm.clone()
    }

    pub fn set_bpm_settings(&mut self, p_settings: Option<Rc<RefCell<BeatsPerMinute>>>) {
        self.initial_bpm = p_settings;
    }

    pub fn get_bpm_from_tcommand(&self, p_param: i32) -> f64 {
        match self.tcommand_mode {
            TCommandMode::TCOMMAND_BPM => p_param as f64 * self.tcommand_resolution,
            TCommandMode::TCOMMAND_FRAME => {
                if p_param != 0 {
                    self.tcommand_resolution / p_param as f64
                } else {
                    120.0
                }
            }
            TCommandMode::TCOMMAND_TIMERB => {
                if p_param >= 0 && p_param < 256 {
                    self.tcommand_resolution / (256 - p_param) as f64
                } else {
                    120.0
                }
            }
        }
    }

    pub fn get_system_commands(&self) -> Vec<Rc<RefCell<MMLSystemCommand>>> {
        self.system_commands.clone()
    }

    pub fn add_system_command(&mut self, p_command: Rc<RefCell<MMLSystemCommand>>) {
        self.system_commands.push(p_command);
    }

    // Sequences.

    pub fn get_global_sequence(&self) -> SeqRc {
        self.global_sequence.clone()
    }

    pub fn get_sequence_group(&mut self) -> &mut MMLSequenceGroup {
        &mut self.sequence_group
    }

    /// `append_new_sequence(List<MMLEvent *> p_events = List<MMLEvent *>())`.
    /// The C++ null-check on the freshly created sequence can't fail here.
    pub fn append_new_sequence(&mut self, p_events: Vec<MmlEventRef>) -> SeqRc {
        let sequence = self.sequence_group.append_new_sequence();
        MMLSequence::from_vector(&sequence, p_events);

        sequence
    }

    //

    pub fn clear(&mut self) {
        self.sequence_group.clear();
        MMLSequence::clear(&self.global_sequence);

        self.title = String::new();
        self.author = String::new();

        self.default_fps = 60;
        self.tcommand_mode = TCommandMode::TCOMMAND_BPM;
        self.tcommand_resolution = 1.0;

        self.default_velocity_mode = 0;
        self.default_expression_mode = 0;

        self.initial_bpm = Some(Rc::new(RefCell::new(BeatsPerMinute::default())));
        self.system_commands.clear();

        // Reset.
        MMLSequence::initialize(&self.global_sequence);
    }

    /// `MMLData()`.
    pub fn new() -> Self {
        Self {
            sequence_group: MMLSequenceGroup::new(),
            global_sequence: MMLSequence::new(false),
            title: String::new(),
            author: String::new(),
            default_fps: 60,
            tcommand_mode: TCommandMode::TCOMMAND_BPM,
            tcommand_resolution: 1.0,
            default_velocity_shift: 4,
            default_velocity_mode: 0,
            default_expression_mode: 0,
            // C++ default-constructs `_initial_bpm` as a null Ref.
            initial_bpm: None,
            system_commands: Vec::new(),
        }
    }
}

impl Default for MMLData {
    fn default() -> Self {
        Self::new()
    }
}
