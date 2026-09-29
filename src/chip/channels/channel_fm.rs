//! Port of `libSiON-cpp/src/chip/channels/siopm_channel_fm.{h,cpp}`.
//!
//! `SiOPMChannelFM` → [`ChannelFm`]: embeds [`ChannelBase`] plus the four
//! operators. C++ ownership (`std::vector<SiOPMOperator*>` fed by the
//! static `_operator_pool`) becomes `Rc<RefCell<Operator>>` slots plus the
//! thread_local [`finalize_operator_pool`] pool (C++ `finalize_pool`).
//!
//! The C++ `_process_function` member-pointer dispatch
//! (`_process_function_list[_lfo_on][_process_function_type]`, ctor
//! `siopm_channel_fm.cpp:1495-1518`) becomes the [`ChannelBaseTrait::process`]
//! match on `(lfo_on, process_function_type)` — rows differ only for
//! `PROCESS_OP1` and `PROCESS_PCM`; `PROCESS_AFM` aliases
//! `_process_operator2` exactly like the C++ table.
//!
//! `_set_lfo_state` (a virtual absent from the C++ base class,
//! `siopm_channel_fm.h:88`) is the inherent [`ChannelFm::set_lfo_state`];
//! its KS override (`siopm_channel_ks.h:51`) dispatches through
//! [`FmKind::Ks`], because `SiOPMChannelKS` is modeled as an [`FmKind`]
//! variant per CONVENTIONS (KS state / ctor / virtual bodies:
//! [`channel_ks`](super::channel_ks)).
//!
//! The C++ `_sound_chip` pointer reached through `set_algorithm`,
//! `set_feedback`, `set_register` and `set_wave_data` (`set_pipes`,
//! `get_zero_buffer`, operator re-init) collapses into `ctx` parameters
//! added to those [`ChannelBaseTrait`] seams. Inside the process functions
//! the C++ `Element *` locals advance independently of the lists' cursors,
//! which the ring [`Pipe`](super::Pipe) models with absolute indices plus
//! the trailing `set()` cursor writes.

use std::any::Any;
use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

use super::channel_base::{ChannelBase, ChannelBaseTrait, InputMode};
use super::channel_ks::FmKind;
use super::operator::{EgState, Operator};
use super::{ChipContext, Pipe, PipeRc};
use crate::chip::params::channel_params::{ChannelParams, MAX_OPERATORS, STREAM_SEND_SIZE};
use crate::chip::ref_table::SiopmRefTable;
use crate::chip::wave::pcm_data::SiopmWavePcmData;
use crate::chip::wave::pcm_table::SiopmWavePcmTable;
use crate::chip::wave::table::SiopmWaveTable;
use crate::err_fail;
use crate::sion_enums::{PITCH_TABLE_OPM_NOISE, PULSE_NOISE_PULSE, PULSE_PCM};

/// C++ `SiOPMChannelFM::IDLING_THRESHOLD`.
pub const IDLING_THRESHOLD: i32 = 5120;

/// C++ `SiOPMChannelFM::REGISTER_OPM` (the only implemented register map).
pub const REGISTER_OPM: i32 = 0;

/// C++ `SiOPMChannelFM::ProcessType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessType {
    Op1 = 0,
    Op2 = 1,
    Op3 = 2,
    Op4 = 3,
    AnalogLike = 4,
    Ring = 5,
    Sync = 6,
    Afm = 7,
    Pcm = 8,
}

impl ProcessType {
    fn from_index(p_index: i32) -> Self {
        match p_index {
            0 => ProcessType::Op1,
            1 => ProcessType::Op2,
            2 => ProcessType::Op3,
            3 => ProcessType::Op4,
            4 => ProcessType::AnalogLike,
            5 => ProcessType::Ring,
            6 => ProcessType::Sync,
            7 => ProcessType::Afm,
            _ => ProcessType::Pcm,
        }
    }
}

type OperatorRc = Rc<RefCell<Operator>>;

thread_local! {
    /// C++ `static List<SiOPMOperator *> _operator_pool` (`_alloc_operator`
    /// pops the back, `_release_operator` pushes the back).
    static OPERATOR_POOL: RefCell<Vec<OperatorRc>> = const { RefCell::new(Vec::new()) };
    /// C++ function-local `static int _pmd` / `_amd` of
    /// `_set_by_opm_register` (shared across every FM channel instance).
    static OPM_PMD: Cell<i32> = const { Cell::new(0) };
    static OPM_AMD: Cell<i32> = const { Cell::new(0) };
}

/// C++ `SiOPMChannelFM::finalize_pool()` (`sion::finalize()` entry).
pub fn finalize_operator_pool() {
    OPERATOR_POOL.with(|pool| pool.borrow_mut().clear());
}

/// FM sound channel — C++ `SiOPMChannelFM` (and, through
/// [`FmKind::Ks`], `SiOPMChannelKS`).
pub struct ChannelFm {
    pub base: ChannelBase,
    pub kind: FmKind,

    algorithm: i32,
    process_function_type: ProcessType,

    pipe0: PipeRc,
    pipe1: PipeRc,

    register_map_type: i32,
    register_map_channel: i32,

    pub operators: Vec<Option<OperatorRc>>,
    pub active_operator: usize,
    pub operator_count: i32,

    pub amplitude_modulation_depth: i32,
    pub amplitude_modulation_output_level: i32,
    pub pitch_modulation_depth: i32,
    pub pitch_modulation_output_level: i32,

    pub eg_timer_initial: i32,
    pub lfo_timer_initial: i32,
}

impl ChannelFm {
    /// C++ `SiOPMChannelFM(SiOPMSoundChip*)` — the ctor ends with
    /// `initialize(nullptr, 0)`, whose virtuals still resolve to the FM
    /// bodies (`_set_lfo_state` included), exactly like a C++ ctor.
    pub fn new(ctx: &mut dyn ChipContext) -> Self {
        let mut channel = ChannelFm {
            base: ChannelBase::new(),
            kind: FmKind::Fm,

            algorithm: 0,
            process_function_type: ProcessType::Op1,

            pipe0: Rc::new(RefCell::new(Pipe::new(1, 0))),
            pipe1: Rc::new(RefCell::new(Pipe::new(1, 0))),

            register_map_type: REGISTER_OPM,
            register_map_channel: 0,

            operators: vec![None; MAX_OPERATORS as usize],
            active_operator: 0,
            operator_count: 1,

            amplitude_modulation_depth: 0,
            amplitude_modulation_output_level: 0,
            pitch_modulation_depth: 0,
            pitch_modulation_output_level: 0,

            eg_timer_initial: 0,
            lfo_timer_initial: 0,
        };

        channel.operators[0] = Some(channel.alloc_operator());
        channel.update_process_function();

        ChannelBaseTrait::initialize(&mut channel, None, 0, ctx);
        channel
    }

    /// C++ `_alloc_operator()`.
    fn alloc_operator(&self) -> OperatorRc {
        OPERATOR_POOL.with(|pool| {
            let mut pool = pool.borrow_mut();
            match pool.pop() {
                Some(op) => op,
                None => Rc::new(RefCell::new(Operator::new())),
            }
        })
    }

    /// C++ `_release_operator(SiOPMOperator*)`.
    fn release_operator(&self, p_operator: OperatorRc) {
        OPERATOR_POOL.with(|pool| pool.borrow_mut().push(p_operator));
    }

    /// C++ `_update_process_function()` — the PMF rebinding is a Rust
    /// no-op ([`ChannelBaseTrait::process`] reads the live
    /// `(lfo_on, process_function_type)` pair); kept for call-structure
    /// parity with the C++ sites.
    fn update_process_function(&mut self) {}

    /// C++ `_update_operator_count(int)`.
    fn update_operator_count(&mut self, p_count: i32, ctx: &mut dyn ChipContext) {
        if self.operator_count < p_count {
            for i in self.operator_count..p_count {
                let op = self.alloc_operator();
                op.borrow_mut().initialize(ctx);
                self.operators[i as usize] = Some(op);
            }
        } else if self.operator_count > p_count {
            for i in p_count..self.operator_count {
                if let Some(op) = self.operators[i as usize].take() {
                    self.release_operator(op);
                }
            }
        }

        self.operator_count = p_count;
        self.process_function_type = ProcessType::from_index(p_count - 1);
        self.update_process_function();

        self.active_operator = (p_count - 1) as usize;

        if self.base.input_mode == InputMode::Feedback {
            self.set_feedback(0, 0, ctx);
        }
    }

    pub(crate) fn op(&self, p_index: usize) -> OperatorRc {
        self.operators[p_index].clone().unwrap()
    }

    pub(crate) fn active_op(&self) -> OperatorRc {
        self.operators[self.active_operator].clone().unwrap()
    }

    fn op_set_pipes(
        &mut self,
        p_index: usize,
        p_out: PipeRc,
        p_in: Option<PipeRc>,
        p_final: bool,
        ctx: &mut dyn ChipContext,
    ) {
        let op = self.op(p_index);
        op.borrow_mut().set_pipes(ctx, p_out, p_in, p_final);
    }

    fn op_set_base_pipe(&mut self, p_index: usize, p_pipe: PipeRc) {
        let op = self.op(p_index);
        op.borrow_mut().set_base_pipe(p_pipe);
    }

    /// C++ `_set_algorithm_operator1(int)`.
    fn set_algorithm_operator1(&mut self, p_algorithm: i32, ctx: &mut dyn ChipContext) {
        self.update_operator_count(1, ctx);
        self.algorithm = p_algorithm;

        let pipe0 = self.pipe0.clone();
        self.op_set_pipes(0, pipe0, None, true, ctx);
    }

    /// C++ `_set_algorithm_operator2(int)`.
    fn set_algorithm_operator2(&mut self, p_algorithm: i32, ctx: &mut dyn ChipContext) {
        self.update_operator_count(2, ctx);
        self.algorithm = p_algorithm;

        let pipe0 = self.pipe0.clone();
        match self.algorithm {
            0 => {
                self.op_set_pipes(0, pipe0.clone(), None, false, ctx);
                self.op_set_pipes(1, pipe0.clone(), Some(pipe0), true, ctx);
            }
            1 => {
                self.op_set_pipes(0, pipe0.clone(), None, true, ctx);
                self.op_set_pipes(1, pipe0, None, true, ctx);
            }
            2 => {
                self.op_set_pipes(0, pipe0.clone(), None, true, ctx);
                self.op_set_pipes(1, pipe0.clone(), Some(pipe0.clone()), true, ctx);
                self.op_set_base_pipe(1, pipe0);
            }
            _ => {
                self.op_set_pipes(0, pipe0.clone(), None, true, ctx);
                self.op_set_pipes(1, pipe0, None, true, ctx);
            }
        }
    }

    /// C++ `_set_algorithm_operator3(int)`.
    fn set_algorithm_operator3(&mut self, p_algorithm: i32, ctx: &mut dyn ChipContext) {
        self.update_operator_count(3, ctx);
        self.algorithm = p_algorithm;

        let pipe0 = self.pipe0.clone();
        let pipe1 = self.pipe1.clone();
        match self.algorithm {
            0 => {
                self.op_set_pipes(0, pipe0.clone(), None, false, ctx);
                self.op_set_pipes(1, pipe0.clone(), Some(pipe0.clone()), false, ctx);
                self.op_set_pipes(2, pipe0.clone(), Some(pipe0), true, ctx);
            }
            1 => {
                self.op_set_pipes(0, pipe0.clone(), None, false, ctx);
                self.op_set_pipes(1, pipe0.clone(), None, false, ctx);
                self.op_set_pipes(2, pipe0.clone(), Some(pipe0), true, ctx);
            }
            2 => {
                self.op_set_pipes(0, pipe0.clone(), None, true, ctx);
                self.op_set_pipes(1, pipe1.clone(), None, false, ctx);
                self.op_set_pipes(2, pipe0, Some(pipe1), true, ctx);
            }
            3 => {
                self.op_set_pipes(0, pipe0.clone(), None, false, ctx);
                self.op_set_pipes(1, pipe0.clone(), Some(pipe0.clone()), true, ctx);
                self.op_set_pipes(2, pipe0, None, true, ctx);
            }
            4 => {
                self.op_set_pipes(0, pipe1.clone(), None, false, ctx);
                self.op_set_pipes(1, pipe0.clone(), Some(pipe1.clone()), true, ctx);
                self.op_set_pipes(2, pipe0, Some(pipe1), true, ctx);
            }
            5 => {
                self.op_set_pipes(0, pipe0.clone(), None, true, ctx);
                self.op_set_pipes(1, pipe0.clone(), None, true, ctx);
                self.op_set_pipes(2, pipe0, None, true, ctx);
            }
            6 => {
                self.op_set_pipes(0, pipe0.clone(), None, false, ctx);
                self.op_set_pipes(1, pipe0.clone(), Some(pipe0.clone()), true, ctx);
                self.op_set_base_pipe(1, pipe0.clone());
                self.op_set_pipes(2, pipe0, None, true, ctx);
            }
            _ => {
                self.op_set_pipes(0, pipe0.clone(), None, true, ctx);
                self.op_set_pipes(1, pipe0.clone(), None, true, ctx);
                self.op_set_pipes(2, pipe0, None, true, ctx);
            }
        }
    }

    /// C++ `_set_algorithm_operator4(int)`.
    fn set_algorithm_operator4(&mut self, p_algorithm: i32, ctx: &mut dyn ChipContext) {
        self.update_operator_count(4, ctx);
        self.algorithm = p_algorithm;

        let pipe0 = self.pipe0.clone();
        let pipe1 = self.pipe1.clone();
        match self.algorithm {
            0 => {
                self.op_set_pipes(0, pipe0.clone(), None, false, ctx);
                self.op_set_pipes(1, pipe0.clone(), Some(pipe0.clone()), false, ctx);
                self.op_set_pipes(2, pipe0.clone(), Some(pipe0.clone()), false, ctx);
                self.op_set_pipes(3, pipe0.clone(), Some(pipe0), true, ctx);
            }
            1 => {
                self.op_set_pipes(0, pipe0.clone(), None, false, ctx);
                self.op_set_pipes(1, pipe0.clone(), None, false, ctx);
                self.op_set_pipes(2, pipe0.clone(), Some(pipe0.clone()), false, ctx);
                self.op_set_pipes(3, pipe0.clone(), Some(pipe0), true, ctx);
            }
            2 => {
                self.op_set_pipes(0, pipe0.clone(), None, false, ctx);
                self.op_set_pipes(1, pipe1.clone(), None, false, ctx);
                self.op_set_pipes(2, pipe0.clone(), Some(pipe1), false, ctx);
                self.op_set_pipes(3, pipe0.clone(), Some(pipe0), true, ctx);
            }
            3 => {
                self.op_set_pipes(0, pipe0.clone(), None, false, ctx);
                self.op_set_pipes(1, pipe0.clone(), Some(pipe0.clone()), false, ctx);
                self.op_set_pipes(2, pipe0.clone(), None, false, ctx);
                self.op_set_pipes(3, pipe0.clone(), Some(pipe0), true, ctx);
            }
            4 => {
                self.op_set_pipes(0, pipe0.clone(), None, false, ctx);
                self.op_set_pipes(1, pipe0.clone(), Some(pipe0.clone()), true, ctx);
                self.op_set_pipes(2, pipe1.clone(), None, false, ctx);
                self.op_set_pipes(3, pipe0, Some(pipe1), true, ctx);
            }
            5 => {
                self.op_set_pipes(0, pipe1.clone(), None, false, ctx);
                self.op_set_pipes(1, pipe0.clone(), Some(pipe1.clone()), true, ctx);
                self.op_set_pipes(2, pipe0.clone(), Some(pipe1.clone()), true, ctx);
                self.op_set_pipes(3, pipe0, Some(pipe1), true, ctx);
            }
            6 => {
                self.op_set_pipes(0, pipe0.clone(), None, false, ctx);
                self.op_set_pipes(1, pipe0.clone(), Some(pipe0.clone()), true, ctx);
                self.op_set_pipes(2, pipe0.clone(), None, true, ctx);
                self.op_set_pipes(3, pipe0, None, true, ctx);
            }
            7 => {
                self.op_set_pipes(0, pipe0.clone(), None, true, ctx);
                self.op_set_pipes(1, pipe0.clone(), None, true, ctx);
                self.op_set_pipes(2, pipe0.clone(), None, true, ctx);
                self.op_set_pipes(3, pipe0, None, true, ctx);
            }
            8 => {
                self.op_set_pipes(0, pipe0.clone(), None, true, ctx);
                self.op_set_pipes(1, pipe1.clone(), None, false, ctx);
                self.op_set_pipes(2, pipe1.clone(), Some(pipe1.clone()), false, ctx);
                self.op_set_pipes(3, pipe0, Some(pipe1), true, ctx);
            }
            9 => {
                self.op_set_pipes(0, pipe0.clone(), None, true, ctx);
                self.op_set_pipes(1, pipe1.clone(), None, false, ctx);
                self.op_set_pipes(2, pipe0.clone(), Some(pipe1), true, ctx);
                self.op_set_pipes(3, pipe0, None, true, ctx);
            }
            10 => {
                self.op_set_pipes(0, pipe0.clone(), None, false, ctx);
                self.op_set_pipes(1, pipe0.clone(), None, false, ctx);
                self.op_set_pipes(2, pipe0.clone(), None, false, ctx);
                self.op_set_pipes(3, pipe0.clone(), Some(pipe0), true, ctx);
            }
            11 => {
                self.op_set_pipes(0, pipe0.clone(), None, true, ctx);
                self.op_set_pipes(1, pipe1.clone(), None, false, ctx);
                self.op_set_pipes(2, pipe1.clone(), None, false, ctx);
                self.op_set_pipes(3, pipe0, Some(pipe1), true, ctx);
            }
            12 => {
                self.op_set_pipes(0, pipe0.clone(), None, false, ctx);
                self.op_set_pipes(1, pipe0.clone(), Some(pipe0.clone()), true, ctx);
                self.op_set_base_pipe(1, pipe0.clone());
                self.op_set_pipes(2, pipe1.clone(), None, false, ctx);
                self.op_set_pipes(3, pipe0, Some(pipe1), true, ctx);
            }
            _ => {
                self.op_set_pipes(0, pipe0.clone(), None, true, ctx);
                self.op_set_pipes(1, pipe0.clone(), None, true, ctx);
                self.op_set_pipes(2, pipe0.clone(), None, true, ctx);
                self.op_set_pipes(3, pipe0, None, true, ctx);
            }
        }
    }

    /// C++ `_set_algorithm_analog_like(int)`.
    fn set_algorithm_analog_like(&mut self, p_algorithm: i32, ctx: &mut dyn ChipContext) {
        self.update_operator_count(2, ctx);
        let pipe0 = self.pipe0.clone();
        self.op_set_pipes(0, pipe0.clone(), None, true, ctx);
        self.op_set_pipes(1, pipe0, None, true, ctx);

        self.algorithm = if (0..=3).contains(&p_algorithm) { p_algorithm } else { 0 };
        self.process_function_type =
            ProcessType::from_index(ProcessType::AnalogLike as i32 + self.algorithm);
        self.update_process_function();
    }

    /// C++ `_set_by_opm_register(int p_address, int p_data)`.
    fn set_by_opm_register(&mut self, p_address: i32, p_data: i32, ctx: &mut dyn ChipContext) {
        if p_address < 0x20 {
            match p_address {
                15 => {
                    if self.register_map_channel == 7
                        && self.operator_count == 4
                        && (p_data & 128) != 0
                    {
                        let op = self.op(3);
                        let mut op = op.borrow_mut();
                        op.set_pulse_generator_type(PULSE_NOISE_PULSE);
                        op.set_pitch_table_type(PITCH_TABLE_OPM_NOISE);
                        op.set_pitch_index(((p_data & 31) << 6) + 2048);
                    }
                }
                24 => {
                    let step = self.base.table().borrow().lfo_timer_steps[p_data as usize];
                    self.set_lfo_timer(step);
                }
                25 => {
                    if p_data & 128 != 0 {
                        OPM_PMD.with(|c| c.set(p_data & 127));
                    } else {
                        OPM_AMD.with(|c| c.set(p_data & 127));
                    }
                }
                27 => {
                    self.initialize_lfo(p_data & 3, Vec::new());
                }
                _ => {}
            }
        } else if self.register_map_channel == (p_address & 7) {
            if p_address < 0x40 {
                match (p_address - 0x20) >> 3 {
                    0 => {
                        self.set_algorithm(4, false, p_data & 7, ctx);
                        self.set_feedback((p_data >> 3) & 7, 0, ctx);

                        let value = p_data >> 6;
                        self.base.volumes[0] = if value != 0 { 0.5 } else { 0.0 };
                        self.base.pan = if value == 1 {
                            128
                        } else if value == 2 {
                            0
                        } else {
                            64
                        };
                    }
                    1 => {
                        for i in 0..4 {
                            let op = self.op(i);
                            op.borrow_mut().set_key_code(p_data & 127);
                        }
                    }
                    2 => {
                        for i in 0..4 {
                            let op = self.op(i);
                            op.borrow_mut().set_key_fraction(p_data & 127);
                        }
                    }
                    3 => {
                        let pitch_mod_shift = (p_data >> 4) & 7;
                        let amplitude_mod_shift = p_data & 3;

                        if p_data & 128 != 0 {
                            let pmd = OPM_PMD.with(|c| c.get());
                            if pitch_mod_shift < 6 {
                                self.set_pitch_modulation(pmd >> (6 - pitch_mod_shift));
                            } else {
                                self.set_pitch_modulation(pmd << (pitch_mod_shift - 5));
                            }
                        } else {
                            let amd = OPM_AMD.with(|c| c.get());
                            if amplitude_mod_shift > 0 {
                                self.set_amplitude_modulation(amd << (amplitude_mod_shift - 1));
                            } else {
                                self.set_amplitude_modulation(0);
                            }
                        }
                    }
                    _ => {}
                }
            } else {
                let ops = [0, 2, 1, 3];
                let op_index = ops[((p_address >> 3) & 3) as usize];
                let op = self.op(op_index);

                match (p_address - 0x40) >> 5 {
                    0 => {
                        let mut op = op.borrow_mut();
                        op.set_detune1((p_data >> 4) & 7);
                        op.set_multiple(p_data & 15);
                    }
                    1 => {
                        op.borrow_mut().set_total_level(p_data & 127);
                    }
                    2 => {
                        let mut op = op.borrow_mut();
                        op.set_key_scaling_rate((p_data >> 6) & 3);
                        op.set_attack_rate((p_data & 31) << 1);
                    }
                    3 => {
                        let mut op = op.borrow_mut();
                        op.set_amplitude_modulation_shift(((p_data >> 7) & 1) << 1);
                        op.set_decay_rate((p_data & 31) << 1);
                    }
                    4 => {
                        let options = [0, 384, 500, 608];
                        let mut op = op.borrow_mut();
                        op.set_ptss_detune(options[((p_data >> 6) & 3) as usize]);
                        op.set_sustain_rate((p_data & 31) << 1);
                    }
                    5 => {
                        let mut op = op.borrow_mut();
                        op.set_sustain_level((p_data >> 4) & 15);
                        op.set_release_rate((p_data & 15) << 2);
                    }
                    _ => {}
                }
            }
        }
    }

    /// C++ `set_params_by_value(...)` (`SET_OP_PARAM` macro — `INT32_MIN`
    /// marks "leave unchanged").
    pub(crate) fn set_params_by_value(
        &mut self,
        p_ar: i32,
        p_dr: i32,
        p_sr: i32,
        p_rr: i32,
        p_sl: i32,
        p_tl: i32,
        p_ksr: i32,
        p_ksl: i32,
        p_mul: i32,
        p_dt1: i32,
        p_dt2: i32,
        p_ams: i32,
        p_phase: i32,
        p_fix_note: i32,
    ) {
        let op = self.active_op();
        let mut op = op.borrow_mut();

        if p_ar != i32::MIN {
            op.set_attack_rate(p_ar);
        }
        if p_dr != i32::MIN {
            op.set_decay_rate(p_dr);
        }
        if p_sr != i32::MIN {
            op.set_sustain_rate(p_sr);
        }
        if p_rr != i32::MIN {
            op.set_release_rate(p_rr);
        }
        if p_sl != i32::MIN {
            op.set_sustain_level(p_sl);
        }
        if p_tl != i32::MIN {
            op.set_total_level(p_tl);
        }
        if p_ksr != i32::MIN {
            op.set_key_scaling_rate(p_ksr);
        }
        if p_ksl != i32::MIN {
            op.set_key_scaling_level(p_ksl, false);
        }
        if p_mul != i32::MIN {
            op.set_multiple(p_mul);
        }
        if p_dt1 != i32::MIN {
            op.set_detune1(p_dt1);
        }
        if p_dt2 != i32::MIN {
            op.set_ptss_detune(p_dt2);
        }
        if p_ams != i32::MIN {
            op.set_amplitude_modulation_shift(p_ams);
        }
        if p_phase != i32::MIN {
            op.set_key_on_phase(p_phase);
        }

        if p_fix_note != i32::MIN {
            op.set_fixed_pitch_index(p_fix_note << 6);
        }
    }

    // LFO control.

    /// C++ `virtual _set_lfo_state(bool)`; the KS override
    /// (`siopm_channel_ks.cpp:106-108`) pins `_lfo_on` to 0.
    fn set_lfo_state(&mut self, p_enabled: bool) {
        if matches!(self.kind, FmKind::Ks(_)) {
            self.base.lfo_on = 0;
            return;
        }

        self.base.lfo_on = p_enabled as i32;
        self.update_process_function();

        self.base.lfo_timer_step = if p_enabled {
            self.base.lfo_timer_step_buffer
        } else {
            0
        };
    }

    /// C++ `_set_lfo_timer(int)`.
    fn set_lfo_timer(&mut self, p_value: i32) {
        self.base.lfo_timer = if p_value > 0 { 1 } else { 0 };
        self.base.lfo_timer_step = p_value;
        self.base.lfo_timer_step_buffer = p_value;
    }

    // Processing.

    /// C++ `_update_lfo(int p_op_count)`.
    fn update_lfo(&mut self, p_op_count: i32) {
        self.base.lfo_timer -= self.base.lfo_timer_step;
        if self.base.lfo_timer >= 0 {
            return;
        }

        self.base.lfo_phase = (self.base.lfo_phase + 1) & 255;

        let value_base = self.base.lfo_wave_table[self.base.lfo_phase as usize];
        self.amplitude_modulation_output_level =
            (value_base * self.amplitude_modulation_depth) >> 7 << 3;
        self.pitch_modulation_output_level =
            (((value_base << 1) - 255) * self.pitch_modulation_depth) >> 8;

        if p_op_count > 0 {
            if let Some(op) = &self.operators[0] {
                op.borrow_mut().set_pm_detune(self.pitch_modulation_output_level);
            }
        }
        if p_op_count > 1 {
            if let Some(op) = &self.operators[1] {
                op.borrow_mut().set_pm_detune(self.pitch_modulation_output_level);
            }
        }
        if p_op_count > 2 {
            if let Some(op) = &self.operators[2] {
                op.borrow_mut().set_pm_detune(self.pitch_modulation_output_level);
            }
        }
        if p_op_count > 3 {
            if let Some(op) = &self.operators[3] {
                op.borrow_mut().set_pm_detune(self.pitch_modulation_output_level);
            }
        }

        self.base.lfo_timer += self.lfo_timer_initial;
    }

    /// C++ `_process_operator1_lfo_off(int p_length)`.
    pub(crate) fn process_operator1_lfo_off(&mut self, p_length: i32) {
        let in_pipe = self.base.in_pipe.clone().unwrap();
        let base_pipe = self.base.base_pipe.clone().unwrap();
        let out_pipe = self.base.out_pipe.clone().unwrap();
        let table = self.base.table().clone();
        let ope0 = self.op(0);
        let eg_timer_initial = self.eg_timer_initial;
        let input_level = self.base.input_level;

        let mut in_idx = in_pipe.borrow().cursor();
        let mut base_idx = base_pipe.borrow().cursor();
        let mut out_idx = out_pipe.borrow().cursor();

        for _ in 0..p_length {
            let output;

            ope0.borrow_mut().tick_eg(eg_timer_initial);

            {
                let mut o = ope0.borrow_mut();
                o.tick_pulse_generator(0);
                let in_value = in_pipe.borrow().value_at(in_idx);
                let t = ((o.get_phase().wrapping_add(in_value.wrapping_shl(input_level as u32)))
                    & SiopmRefTable::PHASE_FILTER)
                    >> o.get_wave_fixed_bits();

                let mut log_idx = o.get_wave_value(t);
                log_idx += o.get_eg_output();
                output = table.borrow().log_table[log_idx as usize];

                o.get_feed_pipe().borrow_mut().set_value(output);
            }

            let b = base_pipe.borrow().value_at(base_idx);
            out_pipe.borrow_mut().set_value_at(out_idx, output + b);

            in_idx = in_pipe.borrow().next_index(in_idx);
            base_idx = base_pipe.borrow().next_index(base_idx);
            out_idx = out_pipe.borrow().next_index(out_idx);
        }

        in_pipe.borrow_mut().set_cursor(in_idx);
        base_pipe.borrow_mut().set_cursor(base_idx);
        out_pipe.borrow_mut().set_cursor(out_idx);
    }

    /// C++ `_process_operator1_lfo_on(int p_length)`.
    pub(crate) fn process_operator1_lfo_on(&mut self, p_length: i32) {
        let in_pipe = self.base.in_pipe.clone().unwrap();
        let base_pipe = self.base.base_pipe.clone().unwrap();
        let out_pipe = self.base.out_pipe.clone().unwrap();
        let table = self.base.table().clone();
        let ope0 = self.op(0);
        let eg_timer_initial = self.eg_timer_initial;
        let input_level = self.base.input_level;

        let mut in_idx = in_pipe.borrow().cursor();
        let mut base_idx = base_pipe.borrow().cursor();
        let mut out_idx = out_pipe.borrow().cursor();

        for _ in 0..p_length {
            let output;

            self.update_lfo(1);

            ope0.borrow_mut().tick_eg(eg_timer_initial);

            {
                let mut o = ope0.borrow_mut();
                o.tick_pulse_generator(0);
                let in_value = in_pipe.borrow().value_at(in_idx);
                let t = ((o.get_phase().wrapping_add(in_value.wrapping_shl(input_level as u32)))
                    & SiopmRefTable::PHASE_FILTER)
                    >> o.get_wave_fixed_bits();

                let mut log_idx = o.get_wave_value(t);
                log_idx += o.get_eg_output()
                    + (self.amplitude_modulation_output_level >> o.get_amplitude_modulation_shift());
                output = table.borrow().log_table[log_idx as usize];

                o.get_feed_pipe().borrow_mut().set_value(output);
            }

            let b = base_pipe.borrow().value_at(base_idx);
            out_pipe.borrow_mut().set_value_at(out_idx, output + b);

            in_idx = in_pipe.borrow().next_index(in_idx);
            base_idx = base_pipe.borrow().next_index(base_idx);
            out_idx = out_pipe.borrow().next_index(out_idx);
        }

        in_pipe.borrow_mut().set_cursor(in_idx);
        base_pipe.borrow_mut().set_cursor(base_idx);
        out_pipe.borrow_mut().set_cursor(out_idx);
    }

    /// C++ `_process_operator2(int p_length)`.
    pub(crate) fn process_operator2(&mut self, p_length: i32) {
        let in_pipe = self.base.in_pipe.clone().unwrap();
        let base_pipe = self.base.base_pipe.clone().unwrap();
        let out_pipe = self.base.out_pipe.clone().unwrap();
        let table = self.base.table().clone();
        let ope0 = self.op(0);
        let ope1 = self.op(1);
        let op0_out = ope0.borrow().get_out_pipe().unwrap().clone();
        let op0_base = ope0.borrow().get_base_pipe().unwrap().clone();
        let op1_in = ope1.borrow().get_in_pipe().unwrap().clone();
        let op1_out = ope1.borrow().get_out_pipe().unwrap().clone();
        let op1_base = ope1.borrow().get_base_pipe().unwrap().clone();
        let eg_timer_initial = self.eg_timer_initial;
        let input_level = self.base.input_level;

        let mut in_idx = in_pipe.borrow().cursor();
        let mut base_idx = base_pipe.borrow().cursor();
        let mut out_idx = out_pipe.borrow().cursor();

        for _ in 0..p_length {
            self.pipe0.borrow_mut().set_value(0);

            self.update_lfo(2);

            {
                ope0.borrow_mut().tick_eg(eg_timer_initial);

                let mut o = ope0.borrow_mut();
                o.tick_pulse_generator(0);
                let in_value = in_pipe.borrow().value_at(in_idx);
                let t = ((o.get_phase().wrapping_add(in_value.wrapping_shl(input_level as u32)))
                    & SiopmRefTable::PHASE_FILTER)
                    >> o.get_wave_fixed_bits();

                let mut log_idx = o.get_wave_value(t);
                log_idx += o.get_eg_output()
                    + (self.amplitude_modulation_output_level >> o.get_amplitude_modulation_shift());
                let output = table.borrow().log_table[log_idx as usize];

                o.get_feed_pipe().borrow_mut().set_value(output);

                let b = op0_base.borrow().value();
                op0_out.borrow_mut().set_value(output + b);
            }

            {
                ope1.borrow_mut().tick_eg(eg_timer_initial);

                let mut o = ope1.borrow_mut();
                o.tick_pulse_generator(0);
                let in_value = op1_in.borrow().value();
                let t = ((o.get_phase().wrapping_add(in_value.wrapping_shl(o.get_fm_shift() as u32)))
                    & SiopmRefTable::PHASE_FILTER)
                    >> o.get_wave_fixed_bits();

                let mut log_idx = o.get_wave_value(t);
                log_idx += o.get_eg_output()
                    + (self.amplitude_modulation_output_level >> o.get_amplitude_modulation_shift());
                let output = table.borrow().log_table[log_idx as usize];

                o.get_feed_pipe().borrow_mut().set_value(output);

                let b = op1_base.borrow().value();
                op1_out.borrow_mut().set_value(output + b);
            }

            let pipe0_value = self.pipe0.borrow().value();
            let b = base_pipe.borrow().value_at(base_idx);
            out_pipe.borrow_mut().set_value_at(out_idx, pipe0_value + b);

            in_idx = in_pipe.borrow().next_index(in_idx);
            base_idx = base_pipe.borrow().next_index(base_idx);
            out_idx = out_pipe.borrow().next_index(out_idx);
        }

        in_pipe.borrow_mut().set_cursor(in_idx);
        base_pipe.borrow_mut().set_cursor(base_idx);
        out_pipe.borrow_mut().set_cursor(out_idx);
    }

    /// C++ `_process_operator3(int p_length)`.
    pub(crate) fn process_operator3(&mut self, p_length: i32) {
        let in_pipe = self.base.in_pipe.clone().unwrap();
        let base_pipe = self.base.base_pipe.clone().unwrap();
        let out_pipe = self.base.out_pipe.clone().unwrap();
        let table = self.base.table().clone();
        let ope0 = self.op(0);
        let ope1 = self.op(1);
        let ope2 = self.op(2);
        let op0_out = ope0.borrow().get_out_pipe().unwrap().clone();
        let op0_base = ope0.borrow().get_base_pipe().unwrap().clone();
        let op1_in = ope1.borrow().get_in_pipe().unwrap().clone();
        let op1_out = ope1.borrow().get_out_pipe().unwrap().clone();
        let op1_base = ope1.borrow().get_base_pipe().unwrap().clone();
        let op2_in = ope2.borrow().get_in_pipe().unwrap().clone();
        let op2_out = ope2.borrow().get_out_pipe().unwrap().clone();
        let op2_base = ope2.borrow().get_base_pipe().unwrap().clone();
        let eg_timer_initial = self.eg_timer_initial;
        let input_level = self.base.input_level;

        let mut in_idx = in_pipe.borrow().cursor();
        let mut base_idx = base_pipe.borrow().cursor();
        let mut out_idx = out_pipe.borrow().cursor();

        for _ in 0..p_length {
            self.pipe0.borrow_mut().set_value(0);
            self.pipe1.borrow_mut().set_value(0);

            self.update_lfo(3);

            {
                ope0.borrow_mut().tick_eg(eg_timer_initial);

                let mut o = ope0.borrow_mut();
                o.tick_pulse_generator(0);
                let in_value = in_pipe.borrow().value_at(in_idx);
                let t = ((o.get_phase().wrapping_add(in_value.wrapping_shl(input_level as u32)))
                    & SiopmRefTable::PHASE_FILTER)
                    >> o.get_wave_fixed_bits();

                let mut log_idx = o.get_wave_value(t);
                log_idx += o.get_eg_output()
                    + (self.amplitude_modulation_output_level >> o.get_amplitude_modulation_shift());
                let output = table.borrow().log_table[log_idx as usize];

                o.get_feed_pipe().borrow_mut().set_value(output);

                let b = op0_base.borrow().value();
                op0_out.borrow_mut().set_value(output + b);
            }

            {
                ope1.borrow_mut().tick_eg(eg_timer_initial);

                let mut o = ope1.borrow_mut();
                o.tick_pulse_generator(0);
                let in_value = op1_in.borrow().value();
                let t = ((o.get_phase().wrapping_add(in_value.wrapping_shl(o.get_fm_shift() as u32)))
                    & SiopmRefTable::PHASE_FILTER)
                    >> o.get_wave_fixed_bits();

                let mut log_idx = o.get_wave_value(t);
                log_idx += o.get_eg_output()
                    + (self.amplitude_modulation_output_level >> o.get_amplitude_modulation_shift());
                let output = table.borrow().log_table[log_idx as usize];

                o.get_feed_pipe().borrow_mut().set_value(output);

                let b = op1_base.borrow().value();
                op1_out.borrow_mut().set_value(output + b);
            }

            {
                ope2.borrow_mut().tick_eg(eg_timer_initial);

                let mut o = ope2.borrow_mut();
                o.tick_pulse_generator(0);
                let in_value = op2_in.borrow().value();
                let t = ((o.get_phase().wrapping_add(in_value.wrapping_shl(o.get_fm_shift() as u32)))
                    & SiopmRefTable::PHASE_FILTER)
                    >> o.get_wave_fixed_bits();

                let mut log_idx = o.get_wave_value(t);
                log_idx += o.get_eg_output()
                    + (self.amplitude_modulation_output_level >> o.get_amplitude_modulation_shift());
                let output = table.borrow().log_table[log_idx as usize];

                o.get_feed_pipe().borrow_mut().set_value(output);

                let b = op2_base.borrow().value();
                op2_out.borrow_mut().set_value(output + b);
            }

            let pipe0_value = self.pipe0.borrow().value();
            let b = base_pipe.borrow().value_at(base_idx);
            out_pipe.borrow_mut().set_value_at(out_idx, pipe0_value + b);

            in_idx = in_pipe.borrow().next_index(in_idx);
            base_idx = base_pipe.borrow().next_index(base_idx);
            out_idx = out_pipe.borrow().next_index(out_idx);
        }

        in_pipe.borrow_mut().set_cursor(in_idx);
        base_pipe.borrow_mut().set_cursor(base_idx);
        out_pipe.borrow_mut().set_cursor(out_idx);
    }

    /// C++ `_process_operator4(int p_length)`.
    pub(crate) fn process_operator4(&mut self, p_length: i32) {
        let in_pipe = self.base.in_pipe.clone().unwrap();
        let base_pipe = self.base.base_pipe.clone().unwrap();
        let out_pipe = self.base.out_pipe.clone().unwrap();
        let table = self.base.table().clone();
        let ope0 = self.op(0);
        let ope1 = self.op(1);
        let ope2 = self.op(2);
        let ope3 = self.op(3);
        let op0_out = ope0.borrow().get_out_pipe().unwrap().clone();
        let op0_base = ope0.borrow().get_base_pipe().unwrap().clone();
        let op1_in = ope1.borrow().get_in_pipe().unwrap().clone();
        let op1_out = ope1.borrow().get_out_pipe().unwrap().clone();
        let op1_base = ope1.borrow().get_base_pipe().unwrap().clone();
        let op2_in = ope2.borrow().get_in_pipe().unwrap().clone();
        let op2_out = ope2.borrow().get_out_pipe().unwrap().clone();
        let op2_base = ope2.borrow().get_base_pipe().unwrap().clone();
        let op3_in = ope3.borrow().get_in_pipe().unwrap().clone();
        let op3_out = ope3.borrow().get_out_pipe().unwrap().clone();
        let op3_base = ope3.borrow().get_base_pipe().unwrap().clone();
        let eg_timer_initial = self.eg_timer_initial;
        let input_level = self.base.input_level;

        let mut in_idx = in_pipe.borrow().cursor();
        let mut base_idx = base_pipe.borrow().cursor();
        let mut out_idx = out_pipe.borrow().cursor();

        for _ in 0..p_length {
            self.pipe0.borrow_mut().set_value(0);
            self.pipe1.borrow_mut().set_value(0);

            self.update_lfo(4);

            {
                ope0.borrow_mut().tick_eg(eg_timer_initial);

                let mut o = ope0.borrow_mut();
                o.tick_pulse_generator(0);
                let in_value = in_pipe.borrow().value_at(in_idx);
                let t = ((o.get_phase().wrapping_add(in_value.wrapping_shl(input_level as u32)))
                    & SiopmRefTable::PHASE_FILTER)
                    >> o.get_wave_fixed_bits();

                let mut log_idx = o.get_wave_value(t);
                log_idx += o.get_eg_output()
                    + (self.amplitude_modulation_output_level >> o.get_amplitude_modulation_shift());
                let output = table.borrow().log_table[log_idx as usize];

                o.get_feed_pipe().borrow_mut().set_value(output);

                let b = op0_base.borrow().value();
                op0_out.borrow_mut().set_value(output + b);
            }

            {
                ope1.borrow_mut().tick_eg(eg_timer_initial);

                let mut o = ope1.borrow_mut();
                o.tick_pulse_generator(0);
                let in_value = op1_in.borrow().value();
                let t = ((o.get_phase().wrapping_add(in_value.wrapping_shl(o.get_fm_shift() as u32)))
                    & SiopmRefTable::PHASE_FILTER)
                    >> o.get_wave_fixed_bits();

                let mut log_idx = o.get_wave_value(t);
                log_idx += o.get_eg_output()
                    + (self.amplitude_modulation_output_level >> o.get_amplitude_modulation_shift());
                let output = table.borrow().log_table[log_idx as usize];

                o.get_feed_pipe().borrow_mut().set_value(output);

                let b = op1_base.borrow().value();
                op1_out.borrow_mut().set_value(output + b);
            }

            {
                ope2.borrow_mut().tick_eg(eg_timer_initial);

                let mut o = ope2.borrow_mut();
                o.tick_pulse_generator(0);
                let in_value = op2_in.borrow().value();
                let t = ((o.get_phase().wrapping_add(in_value.wrapping_shl(o.get_fm_shift() as u32)))
                    & SiopmRefTable::PHASE_FILTER)
                    >> o.get_wave_fixed_bits();

                let mut log_idx = o.get_wave_value(t);
                log_idx += o.get_eg_output()
                    + (self.amplitude_modulation_output_level >> o.get_amplitude_modulation_shift());
                let output = table.borrow().log_table[log_idx as usize];

                o.get_feed_pipe().borrow_mut().set_value(output);

                let b = op2_base.borrow().value();
                op2_out.borrow_mut().set_value(output + b);
            }

            {
                ope3.borrow_mut().tick_eg(eg_timer_initial);

                let mut o = ope3.borrow_mut();
                o.tick_pulse_generator(0);
                let in_value = op3_in.borrow().value();
                let t = ((o.get_phase().wrapping_add(in_value.wrapping_shl(o.get_fm_shift() as u32)))
                    & SiopmRefTable::PHASE_FILTER)
                    >> o.get_wave_fixed_bits();

                let mut log_idx = o.get_wave_value(t);
                log_idx += o.get_eg_output()
                    + (self.amplitude_modulation_output_level >> o.get_amplitude_modulation_shift());
                let output = table.borrow().log_table[log_idx as usize];

                o.get_feed_pipe().borrow_mut().set_value(output);

                let b = op3_base.borrow().value();
                op3_out.borrow_mut().set_value(output + b);
            }

            let pipe0_value = self.pipe0.borrow().value();
            let b = base_pipe.borrow().value_at(base_idx);
            out_pipe.borrow_mut().set_value_at(out_idx, pipe0_value + b);

            in_idx = in_pipe.borrow().next_index(in_idx);
            base_idx = base_pipe.borrow().next_index(base_idx);
            out_idx = out_pipe.borrow().next_index(out_idx);
        }

        in_pipe.borrow_mut().set_cursor(in_idx);
        base_pipe.borrow_mut().set_cursor(base_idx);
        out_pipe.borrow_mut().set_cursor(out_idx);
    }

    /// Fast-forward tail shared by both PCM process functions: writes the
    /// base pipe (C++ `out_pipe->value = base_pipe->value`) and advances the
    /// channel pipe cursors through the remainder of the block.
    fn run_pcm_stream_tail(
        p_in_pipe: &PipeRc,
        p_base_pipe: &PipeRc,
        p_out_pipe: &PipeRc,
        r_in_idx: &mut usize,
        r_base_idx: &mut usize,
        r_out_idx: &mut usize,
    ) {
        let b = p_base_pipe.borrow().value_at(*r_base_idx);
        p_out_pipe.borrow_mut().set_value_at(*r_out_idx, b);
        *r_in_idx = p_in_pipe.borrow().next_index(*r_in_idx);
        *r_base_idx = p_base_pipe.borrow().next_index(*r_base_idx);
        *r_out_idx = p_out_pipe.borrow().next_index(*r_out_idx);
    }

    /// C++ `_process_pcm_lfo_off(int p_length)`.
    pub(crate) fn process_pcm_lfo_off(&mut self, p_length: i32) {
        let in_pipe = self.base.in_pipe.clone().unwrap();
        let base_pipe = self.base.base_pipe.clone().unwrap();
        let out_pipe = self.base.out_pipe.clone().unwrap();
        let table = self.base.table().clone();
        let ope0 = self.op(0);
        let eg_timer_initial = self.eg_timer_initial;
        let input_level = self.base.input_level;

        let mut in_idx = in_pipe.borrow().cursor();
        let mut base_idx = base_pipe.borrow().cursor();
        let mut out_idx = out_pipe.borrow().cursor();

        for i in 0..p_length {
            let mut output = 0;
            let mut ended = false;

            ope0.borrow_mut().tick_eg(eg_timer_initial);

            {
                let mut o = ope0.borrow_mut();
                o.tick_pulse_generator(0);
                let in_value = in_pipe.borrow().value_at(in_idx);
                let mut t = o
                    .get_phase()
                    .wrapping_add(in_value.wrapping_shl(input_level as u32))
                    >> o.get_wave_fixed_bits();

                if t >= o.get_pcm_end_point() {
                    if o.get_pcm_loop_point() == -1 {
                        o.set_eg_state(EgState::Off);
                        o.update_eg_output();
                        ended = true;
                    } else {
                        t -= o.get_pcm_end_point() - o.get_pcm_loop_point();
                        let phase_diff = (o.get_pcm_end_point() - o.get_pcm_loop_point())
                            << o.get_wave_fixed_bits();
                        o.adjust_phase(-phase_diff);
                    }
                }

                if !ended {
                    let mut log_idx = o.get_wave_value(t);
                    log_idx += o.get_eg_output();
                    output = table.borrow().log_table[log_idx as usize];

                    o.get_feed_pipe().borrow_mut().set_value(output);
                }
            }

            if ended {
                // Fast forward (C++ `for (; i < p_length; i++)`).
                for _ in i..p_length {
                    ChannelFm::run_pcm_stream_tail(
                        &in_pipe,
                        &base_pipe,
                        &out_pipe,
                        &mut in_idx,
                        &mut base_idx,
                        &mut out_idx,
                    );
                }
                break;
            }

            let b = base_pipe.borrow().value_at(base_idx);
            out_pipe.borrow_mut().set_value_at(out_idx, output + b);

            in_idx = in_pipe.borrow().next_index(in_idx);
            base_idx = base_pipe.borrow().next_index(base_idx);
            out_idx = out_pipe.borrow().next_index(out_idx);
        }

        in_pipe.borrow_mut().set_cursor(in_idx);
        base_pipe.borrow_mut().set_cursor(base_idx);
        out_pipe.borrow_mut().set_cursor(out_idx);
    }

    /// C++ `_process_pcm_lfo_on(int p_length)`.
    pub(crate) fn process_pcm_lfo_on(&mut self, p_length: i32) {
        let in_pipe = self.base.in_pipe.clone().unwrap();
        let base_pipe = self.base.base_pipe.clone().unwrap();
        let out_pipe = self.base.out_pipe.clone().unwrap();
        let table = self.base.table().clone();
        let ope0 = self.op(0);
        let eg_timer_initial = self.eg_timer_initial;
        let input_level = self.base.input_level;

        let mut in_idx = in_pipe.borrow().cursor();
        let mut base_idx = base_pipe.borrow().cursor();
        let mut out_idx = out_pipe.borrow().cursor();

        for i in 0..p_length {
            let mut output = 0;
            let mut ended = false;

            self.update_lfo(1);

            ope0.borrow_mut().tick_eg(eg_timer_initial);

            {
                let mut o = ope0.borrow_mut();
                o.tick_pulse_generator(0);
                let in_value = in_pipe.borrow().value_at(in_idx);
                let mut t = o
                    .get_phase()
                    .wrapping_add(in_value.wrapping_shl(input_level as u32))
                    >> o.get_wave_fixed_bits();

                if t >= o.get_pcm_end_point() {
                    if o.get_pcm_loop_point() == -1 {
                        o.set_eg_state(EgState::Off);
                        o.update_eg_output();
                        ended = true;
                    } else {
                        t -= o.get_pcm_end_point() - o.get_pcm_loop_point();
                        let phase_diff = (o.get_pcm_end_point() - o.get_pcm_loop_point())
                            << o.get_wave_fixed_bits();
                        o.adjust_phase(-phase_diff);
                    }
                }

                if !ended {
                    let mut log_idx = o.get_wave_value(t);
                    log_idx += o.get_eg_output()
                        + (self.amplitude_modulation_output_level
                            >> o.get_amplitude_modulation_shift());
                    output = table.borrow().log_table[log_idx as usize];

                    o.get_feed_pipe().borrow_mut().set_value(output);
                }
            }

            if ended {
                // Fast forward (C++ `for (; i < p_length; i++)`).
                for _ in i..p_length {
                    ChannelFm::run_pcm_stream_tail(
                        &in_pipe,
                        &base_pipe,
                        &out_pipe,
                        &mut in_idx,
                        &mut base_idx,
                        &mut out_idx,
                    );
                }
                break;
            }

            let b = base_pipe.borrow().value_at(base_idx);
            out_pipe.borrow_mut().set_value_at(out_idx, output + b);

            in_idx = in_pipe.borrow().next_index(in_idx);
            base_idx = base_pipe.borrow().next_index(base_idx);
            out_idx = out_pipe.borrow().next_index(out_idx);
        }

        in_pipe.borrow_mut().set_cursor(in_idx);
        base_pipe.borrow_mut().set_cursor(base_idx);
        out_pipe.borrow_mut().set_cursor(out_idx);
    }

    /// C++ `_process_analog_like(int p_length)`.
    pub(crate) fn process_analog_like(&mut self, p_length: i32) {
        let in_pipe = self.base.in_pipe.clone().unwrap();
        let base_pipe = self.base.base_pipe.clone().unwrap();
        let out_pipe = self.base.out_pipe.clone().unwrap();
        let table = self.base.table().clone();
        let ope0 = self.op(0);
        let ope1 = self.op(1);
        let eg_timer_initial = self.eg_timer_initial;
        let input_level = self.base.input_level;

        let mut in_idx = in_pipe.borrow().cursor();
        let mut base_idx = base_pipe.borrow().cursor();
        let mut out_idx = out_pipe.borrow().cursor();

        for _ in 0..p_length {
            let output0;
            let output1;

            self.update_lfo(2);

            ope0.borrow_mut().tick_eg(eg_timer_initial);
            {
                let o0 = ope0.borrow();
                ope1.borrow_mut().update_eg_output_from(&o0);
            }

            {
                let mut o = ope0.borrow_mut();
                o.tick_pulse_generator(0);
                let in_value = in_pipe.borrow().value_at(in_idx);
                let t = ((o.get_phase().wrapping_add(in_value.wrapping_shl(input_level as u32)))
                    & SiopmRefTable::PHASE_FILTER)
                    >> o.get_wave_fixed_bits();

                let mut log_idx = o.get_wave_value(t);
                log_idx += o.get_eg_output()
                    + (self.amplitude_modulation_output_level >> o.get_amplitude_modulation_shift());
                output0 = table.borrow().log_table[log_idx as usize];
            }

            {
                let mut o = ope1.borrow_mut();
                o.tick_pulse_generator(0);
                let t = (o.get_phase() & SiopmRefTable::PHASE_FILTER) >> o.get_wave_fixed_bits();

                let mut log_idx = o.get_wave_value(t);
                log_idx += o.get_eg_output()
                    + (self.amplitude_modulation_output_level
                        >> ope0.borrow().get_amplitude_modulation_shift());
                output1 = table.borrow().log_table[log_idx as usize];
            }

            ope0.borrow().get_feed_pipe().borrow_mut().set_value(output0);

            let b = base_pipe.borrow().value_at(base_idx);
            out_pipe
                .borrow_mut()
                .set_value_at(out_idx, output0 + output1 + b);

            in_idx = in_pipe.borrow().next_index(in_idx);
            base_idx = base_pipe.borrow().next_index(base_idx);
            out_idx = out_pipe.borrow().next_index(out_idx);
        }

        in_pipe.borrow_mut().set_cursor(in_idx);
        base_pipe.borrow_mut().set_cursor(base_idx);
        out_pipe.borrow_mut().set_cursor(out_idx);
    }

    /// C++ `_process_ring(int p_length)`.
    pub(crate) fn process_ring(&mut self, p_length: i32) {
        let in_pipe = self.base.in_pipe.clone().unwrap();
        let base_pipe = self.base.base_pipe.clone().unwrap();
        let out_pipe = self.base.out_pipe.clone().unwrap();
        let table = self.base.table().clone();
        let ope0 = self.op(0);
        let ope1 = self.op(1);
        let eg_timer_initial = self.eg_timer_initial;
        let input_level = self.base.input_level;

        let mut in_idx = in_pipe.borrow().cursor();
        let mut base_idx = base_pipe.borrow().cursor();
        let mut out_idx = out_pipe.borrow().cursor();

        for _ in 0..p_length {
            let output;

            self.update_lfo(2);

            ope0.borrow_mut().tick_eg(eg_timer_initial);
            {
                let o0 = ope0.borrow();
                ope1.borrow_mut().update_eg_output_from(&o0);
            }

            {
                let mut log_idx;

                {
                    let mut o = ope0.borrow_mut();
                    o.tick_pulse_generator(0);
                    let in_value = in_pipe.borrow().value_at(in_idx);
                    let t = ((o.get_phase().wrapping_add(in_value.wrapping_shl(input_level as u32)))
                        & SiopmRefTable::PHASE_FILTER)
                        >> o.get_wave_fixed_bits();
                    log_idx = o.get_wave_value(t);
                }

                {
                    let mut o = ope1.borrow_mut();
                    o.tick_pulse_generator(0);
                    let t =
                        (o.get_phase() & SiopmRefTable::PHASE_FILTER) >> o.get_wave_fixed_bits();

                    log_idx += o.get_wave_value(t);
                    log_idx += o.get_eg_output()
                        + (self.amplitude_modulation_output_level
                            >> ope0.borrow().get_amplitude_modulation_shift());
                    output = table.borrow().log_table[log_idx as usize];
                }
            }

            ope0.borrow().get_feed_pipe().borrow_mut().set_value(output);

            let b = base_pipe.borrow().value_at(base_idx);
            out_pipe.borrow_mut().set_value_at(out_idx, output + b);

            in_idx = in_pipe.borrow().next_index(in_idx);
            base_idx = base_pipe.borrow().next_index(base_idx);
            out_idx = out_pipe.borrow().next_index(out_idx);
        }

        in_pipe.borrow_mut().set_cursor(in_idx);
        base_pipe.borrow_mut().set_cursor(base_idx);
        out_pipe.borrow_mut().set_cursor(out_idx);
    }

    /// C++ `_process_sync(int p_length)`.
    pub(crate) fn process_sync(&mut self, p_length: i32) {
        let in_pipe = self.base.in_pipe.clone().unwrap();
        let base_pipe = self.base.base_pipe.clone().unwrap();
        let out_pipe = self.base.out_pipe.clone().unwrap();
        let table = self.base.table().clone();
        let ope0 = self.op(0);
        let ope1 = self.op(1);
        let eg_timer_initial = self.eg_timer_initial;
        let input_level = self.base.input_level;

        let mut in_idx = in_pipe.borrow().cursor();
        let mut base_idx = base_pipe.borrow().cursor();
        let mut out_idx = out_pipe.borrow().cursor();

        for _ in 0..p_length {
            let output;

            self.update_lfo(2);

            ope0.borrow_mut().tick_eg(eg_timer_initial);
            {
                let o0 = ope0.borrow();
                ope1.borrow_mut().update_eg_output_from(&o0);
            }

            {
                let raw = {
                    let mut o = ope0.borrow_mut();
                    let in_value = in_pipe.borrow().value_at(in_idx);
                    o.tick_pulse_generator(in_value.wrapping_shl(input_level as u32));
                    let crossed = o.get_phase() & SiopmRefTable::PHASE_MAX != 0;
                    let raw = o.get_key_on_phase_raw();
                    let phase = o.get_phase();
                    o.set_phase(phase & SiopmRefTable::PHASE_FILTER);
                    (crossed, raw)
                };
                if raw.0 {
                    ope1.borrow_mut().set_phase(raw.1);
                }
            }

            {
                let mut o = ope1.borrow_mut();
                o.tick_pulse_generator(0);
                let t = (o.get_phase() & SiopmRefTable::PHASE_FILTER) >> o.get_wave_fixed_bits();

                let mut log_idx = o.get_wave_value(t);
                log_idx += o.get_eg_output()
                    + (self.amplitude_modulation_output_level
                        >> ope0.borrow().get_amplitude_modulation_shift());
                output = table.borrow().log_table[log_idx as usize];
            }

            ope0.borrow().get_feed_pipe().borrow_mut().set_value(output);

            let b = base_pipe.borrow().value_at(base_idx);
            out_pipe.borrow_mut().set_value_at(out_idx, output + b);

            in_idx = in_pipe.borrow().next_index(in_idx);
            base_idx = base_pipe.borrow().next_index(base_idx);
            out_idx = out_pipe.borrow().next_index(out_idx);
        }

        in_pipe.borrow_mut().set_cursor(in_idx);
        base_pipe.borrow_mut().set_cursor(base_idx);
        out_pipe.borrow_mut().set_cursor(out_idx);
    }

    /// C++ `SiOPMChannelFM::offset_volume` body (KS calls it with a fixed
    /// expression of 128 after tracking its own `_expression`).
    pub(crate) fn fm_offset_volume(&mut self, p_expression: i32, p_velocity: i32) {
        let expression_index = (p_expression << 1) as usize;
        let offset = self.base.expression_table[expression_index] + self.base.velocity_table[p_velocity as usize];

        for i in 0..self.operator_count as usize {
            let op = self.op(i);
            if op.borrow().is_final() {
                op.borrow_mut().offset_total_level(offset);
            } else {
                op.borrow_mut().offset_total_level(0);
            }
        }
    }
}

impl ChannelBaseTrait for ChannelFm {
    fn base(&self) -> &ChannelBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut ChannelBase {
        &mut self.base
    }

    fn get_channel_params(&self, r_params: &mut ChannelParams) {
        r_params.set_operator_count(self.operator_count);

        r_params.set_algorithm(self.algorithm);
        r_params.set_envelope_frequency_ratio(self.base.frequency_ratio);

        r_params.set_feedback(0);
        r_params.set_feedback_connection(0);
        for i in 0..self.operator_count as usize {
            let op = self.op(i);
            if let Some(in_pipe) = &self.base.in_pipe {
                if Rc::ptr_eq(in_pipe, op.borrow().get_feed_pipe()) {
                    r_params.set_feedback(self.base.input_level - 6);
                    r_params.set_feedback_connection(i as i32);
                    break;
                }
            }
        }

        r_params.set_lfo_wave_shape(self.base.lfo_wave_shape);
        r_params.set_lfo_frequency_step(self.base.lfo_timer_step_buffer);

        r_params.set_amplitude_modulation_depth(self.amplitude_modulation_depth);
        r_params.set_pitch_modulation_depth(self.pitch_modulation_depth);

        for i in 0..STREAM_SEND_SIZE {
            r_params.set_master_volume(i as i32, self.base.volumes[i]);
        }
        r_params.set_pan(self.base.pan);

        for i in 0..self.operator_count as usize {
            let op = self.op(i);
            if let Some(op_params) = r_params.get_operator_params(i as i32) {
                let borrow = op.borrow();
                borrow.get_operator_params(&mut op_params.borrow_mut());
            }
        }
    }

    fn set_channel_params(
        &mut self,
        p_params: &ChannelParams,
        p_with_volume: bool,
        p_with_modulation: bool,
        ctx: &mut dyn ChipContext,
    ) {
        if p_params.get_operator_count() == 0 {
            return;
        }

        self.set_algorithm(
            p_params.get_operator_count(),
            p_params.is_analog_like(),
            p_params.get_algorithm(),
            ctx,
        );
        self.set_frequency_ratio(p_params.get_envelope_frequency_ratio());
        self.set_feedback(p_params.get_feedback(), p_params.get_feedback_connection(), ctx);

        if p_with_modulation {
            self.initialize_lfo(p_params.get_lfo_wave_shape(), Vec::new());
            self.set_lfo_timer(p_params.get_lfo_frequency_step());

            self.set_amplitude_modulation(p_params.get_amplitude_modulation_depth());
            self.set_pitch_modulation(p_params.get_pitch_modulation_depth());
        }

        if p_with_volume {
            for i in 0..STREAM_SEND_SIZE {
                self.base.volumes[i] = p_params.get_master_volume(i as i32);
            }

            self.base.has_effect_send = false;
            for i in 1..STREAM_SEND_SIZE {
                if self.base.volumes[i] > 0.0 {
                    self.base.has_effect_send = true;
                    break;
                }
            }

            self.base.pan = p_params.get_pan();
        }

        self.base.filter_type = p_params.get_filter_type();
        {
            let filter_cutoff = p_params.get_filter_cutoff();
            let filter_resonance = p_params.get_filter_resonance();
            let filter_ar = p_params.get_filter_attack_rate();
            let filter_dr1 = p_params.get_filter_decay_rate1();
            let filter_dr2 = p_params.get_filter_decay_rate2();
            let filter_rr = p_params.get_filter_release_rate();
            let filter_dc1 = p_params.get_filter_decay_offset1();
            let filter_dc2 = p_params.get_filter_decay_offset2();
            let filter_sc = p_params.get_filter_sustain_offset();
            let filter_rc = p_params.get_filter_release_offset();
            self.set_sv_filter(
                filter_cutoff,
                filter_resonance,
                filter_ar,
                filter_dr1,
                filter_dr2,
                filter_rr,
                filter_dc1,
                filter_dc2,
                filter_sc,
                filter_rc,
            );
        }

        for i in 0..self.operator_count as usize {
            if let Some(op_params) = p_params.get_operator_params(i as i32) {
                let borrow = op_params.borrow();
                self.op(i).borrow_mut().set_operator_params(&borrow);
            }
        }
    }

    fn set_wave_data(&mut self, p_wave_data: &dyn Any, ctx: &mut dyn ChipContext) {
        let mut pcm_data = p_wave_data
            .downcast_ref::<Rc<RefCell<SiopmWavePcmData>>>()
            .cloned();
        let pcm_table = p_wave_data
            .downcast_ref::<Rc<RefCell<SiopmWavePcmTable>>>()
            .cloned();
        if let Some(pcm_table) = &pcm_table {
            pcm_data = pcm_table.borrow().get_note_data(60);
        }

        if let Some(pcm_data) = &pcm_data {
            if !pcm_data.borrow().get_wavelet().is_empty() {
                self.update_operator_count(1, ctx);
                self.process_function_type = ProcessType::Pcm;
                self.update_process_function();
                self.op(0).borrow_mut().set_pcm_data(Some(pcm_data));
                self.set_envelope_reset(true);

                return;
            }
        }

        if let Some(wave_table) = p_wave_data
            .downcast_ref::<Rc<RefCell<SiopmWaveTable>>>()
            .cloned()
        {
            if !wave_table.borrow().get_wavelet().is_empty() {
                self.op(0).borrow_mut().set_wave_table(&wave_table);
                for i in 1..MAX_OPERATORS as usize {
                    if let Some(op) = &self.operators[i] {
                        op.borrow_mut().set_wave_table(&wave_table);
                    }
                }
            }
        }
    }

    fn set_channel_number(&mut self, p_value: i32) {
        self.register_map_channel = p_value;
    }

    fn set_register(&mut self, p_address: i32, p_data: i32, ctx: &mut dyn ChipContext) {
        match self.register_map_type {
            REGISTER_OPM => self.set_by_opm_register(p_address, p_data, ctx),
            _ => {}
        }
    }

    fn set_algorithm(
        &mut self,
        p_operator_count: i32,
        p_analog_like: bool,
        p_algorithm: i32,
        ctx: &mut dyn ChipContext,
    ) {
        if p_analog_like {
            self.set_algorithm_analog_like(p_algorithm, ctx);
            return;
        }

        match p_operator_count {
            1 => self.set_algorithm_operator1(p_algorithm, ctx),
            2 => self.set_algorithm_operator2(p_algorithm, ctx),
            3 => self.set_algorithm_operator3(p_algorithm, ctx),
            4 => self.set_algorithm_operator4(p_algorithm, ctx),
            _ => err_fail!("SiOPMChannelFM: Invalid number of operators."),
        }
    }

    fn set_feedback(&mut self, p_level: i32, p_connection: i32, ctx: &mut dyn ChipContext) {
        if p_level > 0 {
            let mut connection = p_connection;
            if connection < 0 || connection >= self.operator_count {
                connection = 0;
            }

            let pipe = self.op(connection as usize).borrow().get_feed_pipe().clone();
            pipe.borrow_mut().set_value(0);
            self.base.in_pipe = Some(pipe);
            self.base.input_level = p_level + 6;
            self.base.input_mode = InputMode::Feedback;
        } else {
            self.base.in_pipe = Some(ctx.get_zero_buffer());
            self.base.input_level = 0;
            self.base.input_mode = InputMode::Zero;
        }
    }

    fn set_parameters(&mut self, p_params: Vec<i32>, ctx: &mut dyn ChipContext) {
        if matches!(self.kind, FmKind::Ks(_)) {
            self.ks_set_parameters(&p_params, ctx);
            return;
        }

        self.set_params_by_value(
            p_params[1],
            p_params[2],
            p_params[3],
            p_params[4],
            p_params[5],
            p_params[6],
            p_params[7],
            p_params[8],
            p_params[9],
            p_params[10],
            p_params[11],
            p_params[12],
            p_params[13],
            p_params[14],
        );
    }

    fn set_types(&mut self, p_pg_type: i32, p_pt_type: i32, ctx: &mut dyn ChipContext) {
        if matches!(self.kind, FmKind::Ks(_)) {
            self.ks_set_types(p_pg_type);
            return;
        }

        if p_pg_type >= PULSE_PCM {
            let pcm_table = crate::chip::ref_table::instance().borrow().get_pcm_data(p_pg_type - PULSE_PCM);
            if let Some(pcm_table) = pcm_table {
                self.set_wave_data(&pcm_table, ctx);
            }
        } else {
            let op = self.active_op();
            let mut op = op.borrow_mut();
            op.set_pulse_generator_type(p_pg_type);
            op.set_pitch_table_type(p_pt_type);
            drop(op);
            self.update_process_function();
        }
    }

    fn set_all_attack_rate(&mut self, p_value: i32) {
        if matches!(self.kind, FmKind::Ks(_)) {
            let op = self.op(0);
            let mut op = op.borrow_mut();
            op.set_attack_rate(p_value);
            op.set_decay_rate(if p_value > 48 { 48 } else { p_value });
            op.set_total_level(if p_value > 48 { 0 } else { 48 - p_value });
            return;
        }

        for i in 0..self.operator_count as usize {
            let op = self.op(i);
            if op.borrow().is_final() {
                op.borrow_mut().set_attack_rate(p_value);
            }
        }
    }

    fn set_all_release_rate(&mut self, p_value: i32) {
        if matches!(self.kind, FmKind::Ks(_)) {
            self.ks_set_decay_lpf(p_value);
            return;
        }

        for i in 0..self.operator_count as usize {
            let op = self.op(i);
            if op.borrow().is_final() {
                op.borrow_mut().set_release_rate(p_value);
            }
        }
    }

    fn get_pitch(&self) -> i32 {
        if let FmKind::Ks(ks) = &self.kind {
            return ks.ks_pitch_index;
        }

        if self.operator_count == 0 {
            return 0;
        }
        self.op((self.operator_count - 1) as usize)
            .borrow()
            .get_pitch_index()
    }

    fn set_pitch(&mut self, p_value: i32) {
        if let FmKind::Ks(ks) = &mut self.kind {
            ks.ks_pitch_index = p_value;
            return;
        }

        for i in 0..self.operator_count as usize {
            self.op(i).borrow_mut().set_pitch_index(p_value);
        }
    }

    fn set_active_operator_index(&mut self, p_value: i32) {
        let index = crate::math::clampi(p_value, 0, self.operator_count - 1);
        self.active_operator = index as usize;
    }

    fn set_release_rate(&mut self, p_value: i32) {
        if matches!(self.kind, FmKind::Ks(_)) {
            self.ks_set_decay_lpf(p_value);
            return;
        }

        self.active_op().borrow_mut().set_release_rate(p_value);
    }

    fn set_total_level(&mut self, p_value: i32) {
        self.active_op().borrow_mut().set_total_level(p_value);
    }

    fn set_fine_multiple(&mut self, p_value: i32) {
        self.active_op().borrow_mut().set_fine_multiple(p_value);
    }

    fn set_phase(&mut self, p_value: i32) {
        self.active_op().borrow_mut().set_key_on_phase(p_value);
    }

    fn set_detune(&mut self, p_value: i32) {
        self.active_op().borrow_mut().set_ptss_detune(p_value);
    }

    fn set_fixed_pitch(&mut self, p_value: i32) {
        if matches!(self.kind, FmKind::Ks(_)) {
            for i in 0..self.operator_count as usize {
                self.op(i).borrow_mut().set_fixed_pitch_index(i as i32);
            }
            return;
        }

        self.active_op().borrow_mut().set_fixed_pitch_index(p_value);
    }

    fn set_ssg_envelope_control(&mut self, p_value: i32) {
        self.active_op().borrow_mut().set_ssg_type(p_value);
    }

    fn set_envelope_reset(&mut self, p_reset: bool) {
        for i in 0..self.operator_count as usize {
            self.op(i).borrow_mut().set_envelope_reset_on_attack(p_reset);
        }
    }

    fn offset_volume(&mut self, p_expression: i32, p_velocity: i32) {
        if let FmKind::Ks(ks) = &mut self.kind {
            ks.expression = p_expression as f64 * 0.0078125;
            self.fm_offset_volume(128, p_velocity);
            return;
        }

        self.fm_offset_volume(p_expression, p_velocity);
    }

    fn set_frequency_ratio(&mut self, p_ratio: i32) {
        self.base.frequency_ratio = p_ratio;

        let value_coef = if p_ratio != 0 {
            100.0 / p_ratio as f64
        } else {
            1.0
        };
        self.eg_timer_initial = (SiopmRefTable::ENV_TIMER_INITIAL as f64 * value_coef) as i32;
        self.lfo_timer_initial = (SiopmRefTable::LFO_TIMER_INITIAL as f64 * value_coef) as i32;
    }

    fn initialize_lfo(&mut self, p_waveform: i32, p_custom_wave_table: Vec<i32>) {
        self.base.initialize_lfo(p_waveform, p_custom_wave_table);

        self.set_lfo_state(false);

        self.amplitude_modulation_depth = 0;
        self.pitch_modulation_depth = 0;
        self.amplitude_modulation_output_level = 0;
        self.pitch_modulation_output_level = 0;

        for op in self.operators.iter().flatten() {
            op.borrow_mut().set_pm_detune(0);
        }
    }

    fn set_amplitude_modulation(&mut self, p_depth: i32) {
        self.amplitude_modulation_depth = p_depth << 2;
        self.amplitude_modulation_output_level =
            (self.base.lfo_wave_table[self.base.lfo_phase as usize] * self.amplitude_modulation_depth)
                >> 7
                << 3;

        self.set_lfo_state(self.pitch_modulation_depth != 0 || self.amplitude_modulation_depth > 0);
    }

    fn set_pitch_modulation(&mut self, p_depth: i32) {
        self.pitch_modulation_depth = p_depth;
        self.pitch_modulation_output_level = (((self.base.lfo_wave_table
            [self.base.lfo_phase as usize]
            << 1)
            - 255)
            * self.pitch_modulation_depth)
            >> 8;

        self.set_lfo_state(self.pitch_modulation_depth != 0 || self.amplitude_modulation_depth > 0);

        if self.pitch_modulation_depth == 0 {
            for op in self.operators.iter().flatten() {
                op.borrow_mut().set_pm_detune(0);
            }
        }
    }

    fn note_on(&mut self) {
        if let FmKind::Ks(ks) = &mut self.kind {
            ks.note_on_pre(&mut self.operators);
        }

        for i in 0..self.operator_count as usize {
            self.op(i).borrow_mut().note_on();
        }

        self.base.is_note_on = true;
        self.base.is_idling = false;
        self.base.base_note_on();
    }

    fn note_off(&mut self) {
        if let FmKind::Ks(ks) = &mut self.kind {
            ks.note_off();
            return;
        }

        for i in 0..self.operator_count as usize {
            self.op(i).borrow_mut().note_off();
        }

        self.base.is_note_on = false;
        self.base.base_note_off();
    }

    fn reset_channel_buffer_status(&mut self) {
        self.base.buffer_index = 0;

        if matches!(self.kind, FmKind::Ks(_)) {
            self.base.is_idling = false;
            return;
        }

        self.base.is_idling = true;
        for i in 0..self.operator_count as usize {
            let op = self.op(i);
            let op = op.borrow();
            if op.is_final()
                && (op.get_eg_output() < IDLING_THRESHOLD
                    || op.get_eg_state() == EgState::Attack)
            {
                self.base.is_idling = false;
                break;
            }
        }
    }

    fn process(&mut self, p_length: i32, _ctx: &mut dyn ChipContext) {
        let lfo_on = self.base.lfo_on != 0;
        match (lfo_on, self.process_function_type) {
            (false, ProcessType::Op1) => self.process_operator1_lfo_off(p_length),
            (true, ProcessType::Op1) => self.process_operator1_lfo_on(p_length),
            (_, ProcessType::Op2) => self.process_operator2(p_length),
            (_, ProcessType::Op3) => self.process_operator3(p_length),
            (_, ProcessType::Op4) => self.process_operator4(p_length),
            (_, ProcessType::AnalogLike) => self.process_analog_like(p_length),
            (_, ProcessType::Ring) => self.process_ring(p_length),
            (_, ProcessType::Sync) => self.process_sync(p_length),
            (_, ProcessType::Afm) => self.process_operator2(p_length),
            (false, ProcessType::Pcm) => self.process_pcm_lfo_off(p_length),
            (true, ProcessType::Pcm) => self.process_pcm_lfo_on(p_length),
        }
    }

    fn buffer(&mut self, p_length: i32, ctx: &mut dyn ChipContext) {
        if matches!(self.kind, FmKind::Ks(_)) {
            self.ks_buffer(p_length, ctx);
            return;
        }

        ChannelBaseTrait::buffer_base(self, p_length, ctx);
    }

    fn initialize(
        &mut self,
        p_prev: Option<&dyn ChannelBaseTrait>,
        p_buffer_index: i32,
        ctx: &mut dyn ChipContext,
    ) {
        if matches!(self.kind, FmKind::Ks(_)) {
            self.ks_initialize_pre();
        }

        self.update_operator_count(1, ctx);
        self.op(0).borrow_mut().initialize(ctx);

        self.base.is_note_on = false;
        self.initialize_base(p_prev, p_buffer_index, ctx);

        if matches!(self.kind, FmKind::Ks(_)) {
            self.ks_initialize_post(ctx);
        }
    }

    fn reset(&mut self) {
        if let FmKind::Ks(ks) = &mut self.kind {
            for value in ks.delay_buffer.iter_mut() {
                *value = 0;
            }
        }

        for i in 0..self.operator_count as usize {
            self.op(i).borrow_mut().reset();
        }

        self.base.is_note_on = false;
        self.base.is_idling = true;
    }
}
