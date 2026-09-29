//! Wave-6a behavior-fidelity tests: EG state machine, PG phase math and
//! channel-manager pool order against hand-derived C++ values.

use std::cell::RefCell;
use std::rc::Rc;

use super::channel_base::BaseChannel;
use super::channel_fm::ChannelFm;
use super::channel_ks::{FmKind, KsState, KS_BUFFER_SIZE};
use super::channel_pcm::ChannelPcm;
use super::channel_sampler::ChannelSampler;
use super::manager::{self, ChannelRc, ChannelType};
use super::operator::{EgState, Operator};
use super::{ChipContext, OutputStream, Pipe, PipeRc};
use crate::chip::params::operator_params::OperatorParams;
use crate::chip::ref_table::{self, SiopmRefTable};
use crate::math::fmod;
use crate::sion_enums::{PITCH_TABLE_PCM, PULSE_NOISE_PINK};

struct TestChip {
    zero: PipeRc,
    pipes: Vec<PipeRc>,
    init_params: Rc<RefCell<OperatorParams>>,
    buffer_length: i32,
    output: Option<Rc<RefCell<dyn OutputStream>>>,
}

impl TestChip {
    fn new() -> Self {
        let mut pipes = Vec::new();
        for _ in 0..5 {
            pipes.push(Rc::new(RefCell::new(Pipe::new(2048, 0))));
        }
        TestChip {
            zero: Rc::new(RefCell::new(Pipe::new(1, 0))),
            pipes,
            init_params: Rc::new(RefCell::new(OperatorParams::new())),
            buffer_length: 2048,
            output: None,
        }
    }
}

impl ChipContext for TestChip {
    fn get_zero_buffer(&self) -> PipeRc {
        self.zero.clone()
    }

    fn get_pipe(&mut self, p_pipe_num: i32, p_index: i32) -> Option<PipeRc> {
        if p_pipe_num < 0 || p_pipe_num as usize >= self.pipes.len() {
            return None;
        }
        let pipe = self.pipes[p_pipe_num as usize].clone();
        {
            let mut borrow = pipe.borrow_mut();
            borrow.front();
            borrow.advance(p_index);
        }
        Some(pipe)
    }

    fn get_buffer_length(&self) -> i32 {
        self.buffer_length
    }

    fn get_init_operator_params(&self) -> Rc<RefCell<OperatorParams>> {
        self.init_params.clone()
    }

    fn get_stream_slot(&self, _p_slot: usize) -> Option<Rc<RefCell<dyn OutputStream>>> {
        None
    }

    fn set_stream_slot(&mut self, _p_slot: usize, _p_stream: Option<Rc<RefCell<dyn OutputStream>>>) {
    }

    fn get_output_stream(&self) -> Option<Rc<RefCell<dyn OutputStream>>> {
        self.output.clone()
    }

    fn get_pcm_volume(&self) -> f64 {
        1.0
    }

    fn get_sampler_volume(&self) -> f64 {
        2.0
    }
}

struct NullStream;

impl OutputStream for NullStream {
    fn write(
        &mut self,
        _p_data: &Pipe,
        _p_start: usize,
        _p_offset: i32,
        _p_length: i32,
        _p_volume: f64,
        _p_pan: i32,
    ) {
    }

    fn write_stereo(
        &mut self,
        _p_left: &Pipe,
        _p_left_start: usize,
        _p_right: &Pipe,
        _p_right_start: usize,
        _p_offset: i32,
        _p_length: i32,
        _p_volume: f64,
        _p_pan: i32,
    ) {
    }

    fn write_from_vector(
        &mut self,
        _p_data: &[f64],
        _p_start_data: i32,
        _p_start_buffer: i32,
        _p_length: i32,
        _p_volume: f64,
        _p_pan: i32,
        _p_sample_channel_count: i32,
    ) {
    }

    fn get_channel_count(&self) -> i32 {
        2
    }

    fn set_channel_count(&mut self, _p_value: i32) {}

    fn get_buffer(&self) -> &[f64] {
        &[]
    }

    fn get_buffer_mut(&mut self) -> &mut [f64] {
        &mut []
    }

    fn resize(&mut self, _p_length: usize) {}

    fn clear(&mut self) {}

    fn limit(&mut self) {}

    fn quantize(&mut self, _p_bitrate: i32) {}
}

fn install_base_factories() {
    for kind in [
        ChannelType::Fm,
        ChannelType::Pcm,
        ChannelType::Sampler,
        ChannelType::Ks,
    ] {
        manager::register_factory(kind, Box::new(|_ctx| {
            let channel: ChannelRc = Rc::new(RefCell::new(BaseChannel::new()));
            Some(channel)
        }));
    }
}

// ---- EG: envelope-generator state machine ----

#[test]
fn eg_attack_levels_match_hand_computed_sequence() {
    ref_table::initialize();

    let mut params = OperatorParams::new();
    params.initialize();
    params.set_attack_rate(31);
    params.set_sustain_level(15);
    params.set_total_level(127);

    let mut op = Operator::new();
    op.set_operator_params(&params);

    // C++ channels run operator initialize() (reset) after params; a fresh
    // operator keeps eg_level 0 until reset/note_on, exactly like C++.
    op.reset();
    let env_bottom = SiopmRefTable::ENV_BOTTOM;
    assert_eq!(op.get_eg_state(), EgState::Off);
    assert_eq!(op.eg_level, env_bottom);
    assert_eq!(op.eg_timer, SiopmRefTable::ENV_TIMER_INITIAL);
    assert_eq!(op.eg_timer_step, 0); // EG_OFF uses eg_timer_steps[96] == 0

    op.note_on();
    assert_eq!(op.get_eg_state(), EgState::Attack);
    // Hand check: ar=31 -> eg_table_selector[31] = 3 (rows i>=16: 16+i),
    // eg_increment_tables_attack[3] = [0,4,4,4,4,4,4,4].
    // clock_ratio = ((3580000/64)<<10)/44100 = 1298
    // eg_timer_steps[31] = (1<<(31>>2))*1298 = 128*1298 = 166144.
    assert_eq!(op.eg_timer_step, 166144);

    // Timer: ENV_TIMER_INITIAL = (2047*3)<<10 = 6288384; first fire on
    // tick 38 (37*166144 <= 6288384 < 38*166144) at counter 0 (increment 0,
    // no change). The timer step (166144) exceeds timer_initial (81920), so
    // every tick fires from tick 38 on -> the decrement `1 + (level >> 4)`
    // happens once per tick EXCEPT when the 8-counter hits 0. Verify the
    // first-change tick, every change against the recurrence, and the first
    // 15 values against the explicit sequence:
    let first_explicit: [i32; 15] = [
        779, 730, 684, 641, 600, 562, 526, 493, 462, 433, 405, 379, 355, 332, 311,
    ];

    let mut prev = op.eg_level; // 832 (attack entry keeps level)
    let mut levels: Vec<i32> = Vec::new();
    let mut ticks = 0;
    let mut first_change_tick = 0;
    while levels.len() < 15 {
        op.tick_eg(81920);
        ticks += 1;
        assert!(ticks < 100000);
        if op.eg_level != prev {
            assert_eq!(op.eg_level, prev - 1 - (prev >> 4));
            if levels.is_empty() {
                first_change_tick = ticks;
            }
            levels.push(op.eg_level);
            prev = op.eg_level;
        }
    }
    assert_eq!(first_change_tick, 39);
    assert_eq!(levels, first_explicit);

    // Continue; `1 + (level >> 4)` from 311 walks down to 0 and the
    // condition `level <= 0` flips to the decay state (sl=15 -> non-zero
    // sustain level -> decay branch entered).
    let mut guard = 0;
    while op.get_eg_state() == EgState::Attack {
        op.tick_eg(81920);
        guard += 1;
        assert!(guard < 100000, "attack never left EG_ATTACK");
    }
    assert_eq!(op.get_eg_state(), EgState::Decay);
    assert_eq!(op.eg_level, 0);
    assert_eq!(op.eg_state_shift_level, 992); // eg_sustain_level_table[15]
}

#[test]
fn eg_zero_attack_rate_falls_through_to_sustain() {
    ref_table::initialize();

    let mut params = OperatorParams::new();
    params.initialize();
    params.set_attack_rate(63); // 63 + ks(0) >= 62 -> Attack [[fallthrough]]
    params.set_decay_rate(0);
    params.set_sustain_level(0); // eg_sustain_level_table[0] == 0 -> Decay fallthrough
    params.set_total_level(0);

    let mut op = Operator::new();
    op.set_operator_params(&params);
    op.reset();

    // `set_sustain_level` indexes `eg_sustain_level_table[p_value]` with the
    // RAW argument; via set_operator_params the argument is pre-masked
    // (`get_sustain_level() & 15`), so sl=0 lands on index 0.
    assert_eq!(op.eg_sustain_level, 0);

    op.note_on();
    // ar=63 skips the attack entry, dr=0 with sustain level 0 falls through
    // to SUSTAIN (index 96 increment, timer 0).
    assert_eq!(op.get_eg_state(), EgState::Sustain);
    assert_eq!(op.eg_level, op.eg_sustain_level);
    assert_eq!(op.eg_increment_table, [0; 8]);
    assert_eq!(op.eg_timer_step, 0);

    op.note_off();
    assert_eq!(op.get_eg_state(), EgState::Release);
    assert_eq!(op.eg_state_shift_level, SiopmRefTable::ENV_BOTTOM);
}

// ---- PG: phase-generator math ----

#[test]
fn pg_phase_step_matches_opm_formula() {
    ref_table::initialize();

    let table = ref_table::instance();
    let (pitch, dt1, sr_shift, dt2, kc) = {
        let borrow = table.borrow();
        // key code for note 60 comes through note_number_to_key_code (64 —
        // the table skips the reserved 63/79... B slots).
        // pitch_index_shift starts at the raw params detune2 (1) -> index
        // 60*64+1 once set_pitch_index(60*64) runs.
        (
            borrow.pitch_table[0][60 * 64 + 1],
            borrow.dt1_table[2][64],
            borrow.sample_rate_pitch_shift,
            borrow.dt2_table[1],
            borrow.note_number_to_key_code[60],
        )
    };
    assert_eq!(kc, 64);

    let mut params = OperatorParams::new();
    params.initialize();
    params.set_multiple(2);
    params.set_detune1(2);
    params.set_detune2(1);
    params.set_initial_phase(255);

    let mut op = Operator::new();
    op.set_operator_params(&params);
    op.set_pitch_index(60 * 64);

    assert_eq!(op.get_multiple(), 2);
    assert_eq!(op.get_fine_multiple(), 256);
    assert_eq!(op.get_ptss_detune(), 1); // raw params detune2 (C++ verbatim)
    assert_eq!(op.get_key_code(), kc);
    assert_eq!(op.get_key_fraction(), 0);
    assert_eq!(op.get_key_on_phase(), 255);
    assert_eq!(op.get_key_on_phase_raw(), -2);

    // Hand check: wave_fixed_bits 24, but phase_step_shift_filter[OPM] = 0
    // masks the shift to 0 (the OPM pitch table is already sample-scaled);
    // fine_multiple 256 -> net `>> 7` at sample_rate_pitch_shift 0:
    //   phase_step = (pitch + dt1_table[2][kc]) * 256 >> (7 - sr)
    assert_eq!(op.wave_phase_step_shift, 0);
    let expect = (pitch + dt1) * 256 >> (7 - sr_shift);
    assert_eq!(op.phase_step, expect);

    // The operator's own set_detune2 routes the shift through dt2_table.
    let pitch_dt2 = {
        let borrow = table.borrow();
        borrow.pitch_table[0][(60 * 64 + dt2) as usize]
    };
    op.set_detune2(1);
    assert_eq!(op.get_ptss_detune(), dt2);
    let expect_dt2 = (pitch_dt2 + dt1) * 256 >> (7 - sr_shift);
    assert_eq!(op.phase_step, expect_dt2);

    let phase0 = op.get_phase();
    op.tick_pulse_generator(3);
    assert_eq!(op.get_phase() - phase0, op.phase_step + 3);
    op.adjust_phase(-7);
    assert_eq!(op.get_phase(), phase0 + op.phase_step + 3 - 7);
}

#[test]
fn pg_key_on_phase_and_fixed_pitch() {
    ref_table::initialize();

    let mut params = OperatorParams::new();
    params.initialize();
    params.set_multiple(1);
    params.set_initial_phase(64);

    let mut op = Operator::new();
    op.set_operator_params(&params);
    assert_eq!(op.get_key_on_phase_raw(), 64 << 18);

    op.note_on();
    assert_eq!(op.get_phase(), op.get_key_on_phase_raw());

    op.set_key_on_phase(255);
    assert_eq!(op.get_key_on_phase_raw(), -2);
    op.note_on();
    assert_eq!(op.get_phase(), 64 << 18); // unchanged by key-on

    // Fixed pitch locks the key code against pitch-index changes.
    op.set_fixed_pitch_index(72 * 64 + 5);
    assert!(op.is_pitch_fixed());
    assert_eq!(op.get_key_code(), {
        let table = ref_table::instance();
        let borrow = table.borrow();
        borrow.note_number_to_key_code[72]
    });
    let code = op.get_key_code();
    op.set_pitch_index(0);
    assert_eq!(op.get_key_code(), code);
    assert_eq!(op.get_key_fraction(), 5);

    op.set_fixed_pitch_index(0);
    assert!(!op.is_pitch_fixed());
}

// ---- ChannelBase shared stream write path ----

#[test]
fn channel_buffer_writes_through_stream_seam() {
    ref_table::initialize();
    manager::initialize();
    install_base_factories();

    let mut chip = TestChip::new();
    chip.output = Some(Rc::new(RefCell::new(NullStream)));

    let channel = manager::create_channel(ChannelType::Fm, None, 64, &mut chip).unwrap();

    // Idling: buffer() == buffer_no_process(); the OUTPUT_STANDARD branch
    // rewinds the shared pipe-4 cursor to (buffer_index + length).
    channel.borrow_mut().buffer(8, &mut chip);
    assert_eq!(channel.borrow().get_buffer_index(), 72);
    assert_eq!(chip.pipes[4].borrow().cursor(), 72);
    assert!(!Rc::ptr_eq(
        &chip.pipes[4],
        &channel.borrow().base().in_pipe.clone().unwrap()
    ));

    // Non-idling with a zero master volume: the process/rotate default runs
    // and the stream write happens with volume 0.0 (volumes[0] = 0.5 -> 64).
    let channel2 = manager::create_channel(ChannelType::Fm, Some(&channel), 128, &mut chip).unwrap();
    {
        let mut borrow = channel2.borrow_mut();
        let base = borrow.base_mut();
        base.is_idling = false;
    }
    assert_eq!(channel2.borrow().get_master_volume(), 64); // inherited 0.5 * 128
    assert_eq!(channel2.borrow().get_pan(), 0);
    channel2.borrow_mut().set_pan(-16);
    assert_eq!(channel2.borrow().get_pan(), -16);
    channel2.borrow_mut().buffer(4, &mut chip);
    assert_eq!(channel2.borrow().get_buffer_index(), 132);
    // OUTPUT_STANDARD keeps pulling the SHARED pipe 4 (repositioned by the
    // chip to buffer_index each call — cursor == buffer index).
    assert_eq!(chip.pipes[4].borrow().cursor(), 132);

    manager::delete_channel(&channel);
    manager::delete_channel(&channel2);
    assert_eq!(manager::get_free_channel_count(ChannelType::Fm), 2);

    manager::finalize();
}

// ---- ChannelManager: pool order ----

#[test]
fn channel_manager_pool_order_matches_cpp_list() {
    ref_table::initialize();
    manager::initialize();
    install_base_factories();

    let mut chip = TestChip::new();
    chip.output = Some(Rc::new(RefCell::new(NullStream)));

    assert_eq!(manager::get_channel_count(ChannelType::Fm), 0);
    assert_eq!(manager::get_free_channel_count(ChannelType::Fm), 0);

    // Fresh head on empty pool -> channel overflow -> factory creates;
    // channels activate at the tail (C++ `_terminator->_prev`).
    let a = manager::create_channel(ChannelType::Fm, None, 0, &mut chip).unwrap();
    let b = manager::create_channel(ChannelType::Fm, None, 128, &mut chip).unwrap();
    let c = manager::create_channel(ChannelType::Fm, None, 256, &mut chip).unwrap();
    assert_eq!(
        manager::pool_order(ChannelType::Fm),
        vec![(false, ChannelType::Fm); 3]
    );
    assert_eq!(b.borrow().get_buffer_index(), 128);
    assert_eq!(a.borrow().base().pan, 64);
    assert_eq!(a.borrow().base().get_channel_type(), ChannelType::Fm);

    // Freed channels move to the front (`terminator->_next`); allocation
    // reuses the FREE HEAD (LIFO among freely-fronted nodes), exactly like
    // the C++ circular list.
    manager::delete_channel(&a);
    manager::delete_channel(&b);
    assert_eq!(manager::get_free_channel_count(ChannelType::Fm), 2);
    assert_eq!(
        manager::pool_order(ChannelType::Fm),
        vec![
            (true, ChannelType::Fm),
            (true, ChannelType::Fm),
            (false, ChannelType::Fm)
        ]
    );

    let d = manager::create_channel(ChannelType::Fm, Some(&c), 512, &mut chip).unwrap();
    assert!(Rc::ptr_eq(&d, &b)); // b was freed last -> front-most -> head
    assert_eq!(manager::get_free_channel_count(ChannelType::Fm), 1);

    let e = manager::create_channel(ChannelType::Fm, None, 640, &mut chip).unwrap();
    assert!(Rc::ptr_eq(&e, &a));
    assert_eq!(manager::get_free_channel_count(ChannelType::Fm), 0);

    // No free nodes: a create with a busy head overflows to a fresh channel
    // appended at the tail (total = created-once channels = 4, the reused
    // nodes never grow the pool, exactly like C++ `_length`).
    let f = manager::create_channel(ChannelType::Fm, None, 768, &mut chip).unwrap();
    assert!(!Rc::ptr_eq(&f, &e));
    assert_eq!(manager::get_channel_count(ChannelType::Fm), 4);

    // Pools are per-kind.
    let p = manager::create_channel(ChannelType::Pcm, None, 0, &mut chip).unwrap();
    assert_eq!(p.borrow().base().get_channel_type(), ChannelType::Pcm);
    assert_eq!(manager::get_channel_count(ChannelType::Fm), 4);
    assert_eq!(manager::get_channel_count(ChannelType::Pcm), 1);

    // Idle channel buffer() rotates the shared cursor: d re-initialized at
    // 512 -> after buffer(8) the shared pipe-4 cursor is at 520 (the cursor
    // is list-global in C++ — f's earlier position 768 is overwritten).
    d.borrow_mut().buffer(8, &mut chip);
    assert_eq!(d.borrow().get_buffer_index(), 520);
    assert_eq!(chip.pipes[4].borrow().cursor(), 520);

    manager::reset_all_channels();
    assert_eq!(manager::get_free_channel_count(ChannelType::Fm), 4);
    assert!(manager::pool_order(ChannelType::Fm).iter().all(|(free, _)| *free));

    manager::finalize();
}

// ---- Wave-6b: concrete channel tests (FM / KS / PCM / Sampler) ----

/// Stream seam that records the raw pipe integers handed to `write`.
#[derive(Default)]
struct CaptureStream {
    written: Vec<i32>,
}

impl OutputStream for CaptureStream {
    fn write(
        &mut self,
        p_data: &Pipe,
        p_start: usize,
        _p_offset: i32,
        p_length: i32,
        _p_volume: f64,
        _p_pan: i32,
    ) {
        let mut idx = p_start;
        for _ in 0..p_length {
            self.written.push(p_data.value_at(idx));
            idx = p_data.next_index(idx);
        }
    }

    fn write_stereo(
        &mut self,
        p_left: &Pipe,
        p_left_start: usize,
        p_right: &Pipe,
        p_right_start: usize,
        _p_offset: i32,
        p_length: i32,
        _p_volume: f64,
        _p_pan: i32,
    ) {
        let mut l = p_left_start;
        let mut r = p_right_start;
        for _ in 0..p_length {
            self.written.push(p_left.value_at(l));
            self.written.push(p_right.value_at(r));
            l = p_left.next_index(l);
            r = p_right.next_index(r);
        }
    }

    fn write_from_vector(
        &mut self,
        _p_data: &[f64],
        _p_start_data: i32,
        _p_start_buffer: i32,
        _p_length: i32,
        _p_volume: f64,
        _p_pan: i32,
        _p_sample_channel_count: i32,
    ) {
    }

    fn get_channel_count(&self) -> i32 {
        2
    }

    fn set_channel_count(&mut self, _p_value: i32) {}

    fn get_buffer(&self) -> &[f64] {
        &[]
    }

    fn get_buffer_mut(&mut self) -> &mut [f64] {
        &mut []
    }

    fn resize(&mut self, _p_length: usize) {}

    fn clear(&mut self) {}

    fn limit(&mut self) {}

    fn quantize(&mut self, _p_bitrate: i32) {}
}

/// KS ctor mirror of `channel_ks::new_channel_ks` that also stashes the
/// concrete handle (the manager hands out `dyn ChannelBaseTrait`).
fn register_ks_factory(
    stash: Rc<RefCell<Vec<Rc<RefCell<ChannelFm>>>>>,
) {
    manager::register_factory(ChannelType::Ks, Box::new(move |ctx| {
        let mut fm = ChannelFm::new(ctx);
        fm.kind = FmKind::Ks(KsState::new());
        let concrete = Rc::new(RefCell::new(fm));
        stash.borrow_mut().push(concrete.clone());
        let erased: ChannelRc = concrete;
        Some(erased)
    }));
}

#[test]
fn ks_channel_first_samples_match_hand_derived_recurrence() {
    // C++ derivation (siopm_channel_ks.cpp) of the first three KS samples
    // after `initialize / set_release_rate(32) / set_pitch(60*64) / note_on`
    // on a fresh channel (delay buffer all zero, note_on leaves it zero
    // because `int *= 0.3` truncates 0):
    //
    //   sample i of `_apply_karplus_strong` (lines 143-172):
    //     LFO: _lfo_timer starts at 0 (initialize_base runs
    //          initialize_lfo -> 1, then set_lfo_cycle_time(333) -> 0),
    //          step = (LFO_TIMER_INITIAL / (333 * 44100/255000)) as int
    //                 << sample_rate_pitch_shift.
    //          Sample 0: 0 - step < 0 -> fires (phase 0->1); PM depth is 0
    //          so _pitch_modulation_output_level stays 0; next fire is at
    //          sample ceil(INIT/step)-1 ~ 57 (outside this block).
    //     pitch_idx = _ks_pitch_index(3840) + op0 ptss_detune(0) + pmol(0)
    //     wave_length_max = pitch_wave_length[3840]  (> 4 samples)
    //     _ks_delay_buffer_index: 0 -> 1 -> 2 -> 3 -> 4 (all below wlm,
    //          no fmod wrap in the first 4 samples)
    //     out   = out * _decay;                       (_decay = 0.98,
    //     out  += (buf[idx] - out) * _decay_lpf + v;   copied from
    //                                                  _ks_decay_lpf by
    //                                                  note_on; 1 - 32/64
    //                                                  = 0.5 via
    //                                                  set_release_rate)
    //     buf[idx] = (int)out;  out_pipe = (int)out;
    //   => s0 = v0 (out = 0*0.98 + (0-0)*0.5 + v0)
    //      s1 = trunc((v0*0.98)*(1-0.5) + v1)      (buf[2] = 0)
    //      s2 = trunc((out1*0.98)*(1-0.5) + v2)    (buf[3] = 0)
    //   with v_i = the FM operator1 output of sample i (the untouched
    //   `_process_operator1_lfo_off` value): log_table[wave[t] + eg],
    //   t = (phase & PHASE_FILTER) >> wave_fixed_bits, phase stepping by
    //   the C++ PG math. v_i is re-derived here with a standalone
    //   Operator driven through the public API exactly as the channel
    //   configures op0 (see siopm_channel_ks.cpp:241-243), so the KS
    //   delay-line math is checked against independent primitives.
    ref_table::initialize();
    manager::initialize();

    let stash: Rc<RefCell<Vec<Rc<RefCell<ChannelFm>>>>> = Rc::new(RefCell::new(Vec::new()));
    register_ks_factory(stash.clone());

    let mut chip = TestChip::new();
    let capture = Rc::new(RefCell::new(CaptureStream::default()));
    chip.output = Some(capture.clone());

    // KS factory == channel_ks::new_channel_ks (same construction).
    let channel = manager::create_channel(ChannelType::Ks, None, 0, &mut chip).unwrap();
    assert_eq!(channel.borrow().base().get_channel_type(), ChannelType::Ks);
    assert!(matches!(stash.borrow()[0].borrow().kind, FmKind::Ks(_)));
    assert_eq!(manager::get_channel_count(ChannelType::Ks), 1);

    channel.borrow_mut().set_types(0, 0, &mut chip);
    channel.borrow_mut().set_release_rate(32); // ks_decay_lpf = 1 - 32/64
    channel.borrow_mut().set_pitch(60 * 64);
    // The KS init tail leaves key_on_phase = -1, which makes note_on seed a
    // RANDOM phase (siopm_operator.cpp:548-552 — RandomNumberGenerator per
    // call). Pin it to 0 so the hand-derived values are exact.
    channel.borrow_mut().set_phase(0);
    {
        let handles = stash.borrow();
        let fm = handles[0].borrow();
        let FmKind::Ks(ks) = &fm.kind else {
            panic!("KS kind expected");
        };
        assert_eq!(ks.seed_type, 0);
        assert_eq!(ks.ks_pitch_index, 3840);
        assert!((ks.ks_decay_lpf - 0.5).abs() < f64::EPSILON);
        assert_eq!(ks.delay_buffer.len(), KS_BUFFER_SIZE);
        assert_eq!(fm.eg_timer_initial, SiopmRefTable::ENV_TIMER_INITIAL);
    }

    channel.borrow_mut().note_on();
    channel.borrow_mut().buffer(4, &mut chip);
    assert_eq!(channel.borrow().get_buffer_index(), 4);
    assert_eq!(capture.borrow().written.len(), 4);

    // Standalone operator mirror of the channel op0 chain.
    let mut o2 = Operator::new();
    o2.initialize(&mut chip);
    o2.initialize(&mut chip); // create_channel initialize runs it again
    // ks_initialize_post: set_params_by_value(48,48,0,63,15,0,0,0,1,0,0,0,-1,0)
    o2.set_attack_rate(48);
    o2.set_decay_rate(48);
    o2.set_sustain_rate(0);
    o2.set_release_rate(63);
    o2.set_sustain_level(15);
    o2.set_total_level(0);
    o2.set_key_scaling_rate(0);
    o2.set_key_scaling_level(0, false);
    o2.set_multiple(1);
    o2.set_detune1(0);
    o2.set_ptss_detune(0);
    o2.set_amplitude_modulation_shift(0);
    o2.set_key_on_phase(-1);
    o2.set_fixed_pitch_index(0);
    o2.set_key_on_phase(0);
    o2.set_pulse_generator_type(PULSE_NOISE_PINK);
    o2.set_pitch_table_type(PITCH_TABLE_PCM);
    o2.note_on();

    let table = ref_table::instance();
    let mut v = [0i32; 4];
    let mut dup_probe = (0i32, 0i32, 0i32);
    let mut dup_probe2 = (0i32, 0i32, 0i32, 0i32, 0i32, 0i32, 0i32, 0i32);
    for i in 0..4 {
        o2.tick_eg(SiopmRefTable::ENV_TIMER_INITIAL);
        o2.tick_pulse_generator(0);
        let t = (o2.get_phase() & SiopmRefTable::PHASE_FILTER) >> o2.get_wave_fixed_bits();
        let log_idx = (o2.get_wave_value(t) + o2.get_eg_output()) as usize;
        v[i] = table.borrow().log_table[log_idx];
        if i == 3 {
            dup_probe = (o2.get_pitch_index(), o2.phase_step, o2.get_wave_fixed_bits());
            dup_probe2 = (
                o2.get_phase(),
                o2.eg_level,
                o2.eg_timer,
                o2.eg_counter,
                o2.get_eg_output(),
                o2.wave_table[2040],
                o2.wave_table[2041],
                o2.wave_table[2042],
            );
        }
    }

    // KS recurrence (documented above).
    let pitch_shift = table.borrow().sample_rate_pitch_shift;
    let lfo_step = ((SiopmRefTable::LFO_TIMER_INITIAL as f64 / (333.0 * 0.17294117647058824))
        as i32)
        << pitch_shift;
    let wlm = table.borrow().pitch_wave_length[3840];
    assert!(wlm > 4.0);

    let mut buf = vec![0i32; KS_BUFFER_SIZE];
    let mut out = 0.0f64;
    let mut dbi = 0.0f64;
    let mut lfo_timer = 0i32;
    let mut lfo_phase = 0i32;
    let mut expected = [0i32; 3];
    for i in 0..4 {
        lfo_timer -= lfo_step;
        if lfo_timer < 0 {
            lfo_phase = (lfo_phase + 1) & 255;
            lfo_timer += SiopmRefTable::LFO_TIMER_INITIAL;
        }
        dbi += 1.0;
        if dbi >= wlm {
            dbi = fmod(dbi, wlm);
        }
        let idx = dbi as usize;
        out *= 0.98;
        out += (buf[idx] as f64 - out) * 0.5 + v[i] as f64;
        buf[idx] = out as i32;
        if i < 3 {
            expected[i] = out as i32;
        }
    }

    {
        let handles = stash.borrow();
        let fm = handles[0].borrow();
        let op = fm.op(0);
        let o = op.borrow();
        assert_eq!(
            (
                o.get_pitch_index(),
                o.phase_step,
                o.get_wave_fixed_bits(),
                o.get_phase(),
                o.eg_level,
                o.eg_timer,
                o.eg_counter,
                o.get_eg_output(),
                o.wave_table[2040],
                o.wave_table[2041],
                o.wave_table[2042],
            ),
            (
                dup_probe.0,
                dup_probe.1,
                dup_probe.2,
                dup_probe2.0,
                dup_probe2.1,
                dup_probe2.2,
                dup_probe2.3,
                dup_probe2.4,
                dup_probe2.5,
                dup_probe2.6,
                dup_probe2.7,
            ),
            "channel op diverged from mirror"
        );
    }

    let written = capture.borrow();
    assert_eq!(
        &written.written[..3],
        &expected[..],
        "v={v:?} expected={expected:?} written={:?}",
        &written.written[..4]
    );
    drop(written);

    {
        let handles = stash.borrow();
        let fm = handles[0].borrow();
        let FmKind::Ks(ks) = &fm.kind else {
            panic!("KS kind expected");
        };
        assert_eq!(ks.delay_buffer[1], expected[0]);
        assert_eq!(ks.delay_buffer[2], expected[1]);
        assert_eq!(ks.delay_buffer[3], expected[2]);
        assert!((ks.delay_buffer_index - 4.0).abs() < f64::EPSILON);
    }

    // note_off switches to the mute decay pair (siopm_channel_ks.cpp:125-128).
    channel.borrow_mut().note_off();
    {
        let handles = stash.borrow();
        let fm = handles[0].borrow();
        let FmKind::Ks(ks) = &fm.kind else {
            panic!("KS kind expected");
        };
        assert!((ks.decay_lpf - 0.5).abs() < f64::EPSILON);
        assert!((ks.decay - 0.75).abs() < f64::EPSILON);
    }

    manager::delete_channel(&channel);
    manager::finalize();
}

#[test]
fn fm_channel_create_note_on_process_first_sample() {
    ref_table::initialize();
    manager::initialize();

    let stash: Rc<RefCell<Vec<Rc<RefCell<ChannelFm>>>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = stash.clone();
    manager::register_factory(ChannelType::Fm, Box::new(move |ctx| {
        let concrete = Rc::new(RefCell::new(ChannelFm::new(ctx)));
        sink.borrow_mut().push(concrete.clone());
        let erased: ChannelRc = concrete;
        Some(erased)
    }));

    let mut chip = TestChip::new();
    let capture = Rc::new(RefCell::new(CaptureStream::default()));
    chip.output = Some(capture.clone());

    let channel = manager::create_channel(ChannelType::Fm, None, 0, &mut chip).unwrap();
    assert_eq!(channel.borrow().base().get_channel_type(), ChannelType::Fm);

    channel.borrow_mut().note_on();
    assert!(channel.borrow().is_note_on());
    channel.borrow_mut().buffer(2, &mut chip);

    // Mirror of the freshly-initialized channel op0 (init params: sine /
    // PITCH_TABLE_OPM / ar=63 / fine_multiple 128 / phase 0, pitch_index 0),
    // stepped through the exact `_process_operator1_lfo_off` arithmetic.
    let mut o2 = Operator::new();
    o2.initialize(&mut chip);
    o2.initialize(&mut chip);
    o2.note_on();
    let mut expect = [0i32; 2];
    for i in 0..2 {
        o2.tick_eg(SiopmRefTable::ENV_TIMER_INITIAL);
        o2.tick_pulse_generator(0);
        let t = (o2.get_phase() & SiopmRefTable::PHASE_FILTER) >> o2.get_wave_fixed_bits();
        let log_idx = (o2.get_wave_value(t) + o2.get_eg_output()) as usize;
        expect[i] = ref_table::instance().borrow().log_table[log_idx];
    }
    let written = capture.borrow();
    assert_eq!(written.written.len(), 2);
    assert_eq!(written.written[0], expect[0]);
    assert_eq!(written.written[1], expect[1]);

    let channel2 = manager::create_channel(ChannelType::Fm, None, 0, &mut chip).unwrap();
    channel2.borrow_mut().set_pitch(60 * 64);
    assert_eq!(channel2.borrow().get_pitch(), 60 * 64);
    channel2.borrow_mut().set_all_attack_rate(31);
    assert_eq!(
        stash.borrow()[1].borrow().op(0).borrow().get_eg_state(),
        EgState::Off
    );

    manager::delete_channel(&channel);
    manager::delete_channel(&channel2);
    manager::finalize();
}

#[test]
fn pcm_and_sampler_channels_build_through_the_manager() {
    ref_table::initialize();
    manager::initialize();

    manager::register_factory(ChannelType::Pcm, Box::new(|ctx| {
        let concrete = Rc::new(RefCell::new(ChannelPcm::new(ctx)));
        let erased: ChannelRc = concrete;
        Some(erased)
    }));
    manager::register_factory(ChannelType::Sampler, Box::new(|_ctx| {
        let concrete = Rc::new(RefCell::new(ChannelSampler::new()));
        let erased: ChannelRc = concrete;
        Some(erased)
    }));

    let mut chip = TestChip::new();
    let capture = Rc::new(RefCell::new(CaptureStream::default()));
    chip.output = Some(capture.clone());

    let pcm = manager::create_channel(ChannelType::Pcm, None, 64, &mut chip).unwrap();
    assert_eq!(pcm.borrow().base().get_channel_type(), ChannelType::Pcm);
    // Fresh PCM channel idles; buffer_no_process rotates both output pipes.
    pcm.borrow_mut().buffer(16, &mut chip);
    assert_eq!(pcm.borrow().get_buffer_index(), 80);
    assert!(capture.borrow().written.is_empty());

    let sampler = manager::create_channel(ChannelType::Sampler, None, 0, &mut chip).unwrap();
    assert_eq!(sampler.borrow().base().get_channel_type(), ChannelType::Sampler);
    // No wave data loaded: note_on idles, get_pitch == wave_number << 6.
    sampler.borrow_mut().note_on();
    assert!(!sampler.borrow().is_note_on());
    assert_eq!(sampler.borrow().get_pitch(), -1 << 6);
    sampler.borrow_mut().buffer(8, &mut chip);
    assert_eq!(sampler.borrow().get_buffer_index(), 8);
    assert!(capture.borrow().written.is_empty());

    manager::delete_channel(&pcm);
    manager::delete_channel(&sampler);
    manager::finalize();
}


#[test]
fn ks_factory_and_karplus_strong_params() {
    use super::channel_ks::new_channel_ks;

    ref_table::initialize();

    // The real channel_ks::new_channel_ks through the manager factory.
    manager::initialize();
    manager::register_factory(ChannelType::Ks, Box::new(|ctx| Some(new_channel_ks(ctx))));
    let mut chip = TestChip::new();
    let capture = Rc::new(RefCell::new(CaptureStream::default()));
    chip.output = Some(capture.clone());
    let channel = manager::create_channel(ChannelType::Ks, None, 0, &mut chip).unwrap();
    assert_eq!(channel.borrow().base().get_channel_type(), ChannelType::Ks);
    assert_eq!(manager::get_channel_count(ChannelType::Ks), 1);
    manager::delete_channel(&channel);
    manager::finalize();

    // Concrete handle for the inherent KS API.
    manager::initialize();
    let stash: Rc<RefCell<Vec<Rc<RefCell<ChannelFm>>>>> = Rc::new(RefCell::new(Vec::new()));
    register_ks_factory(stash.clone());
    let mut chip2 = TestChip::new();
    chip2.output = Some(capture);
    let channel = manager::create_channel(ChannelType::Ks, None, 0, &mut chip2).unwrap();

    // set_karplus_strong_params(48,48,0,0,-1,8): wave_shape -1 -> pink
    // noise, tension 8 -> ks_decay_lpf = 1 - 8/64.
    stash.borrow()[0].borrow_mut().set_karplus_strong_params(48, 48, 0, 0, -1, 8, &mut chip2);
    {
        let handles = stash.borrow();
        let fm = handles[0].borrow();
        let FmKind::Ks(ks) = &fm.kind else {
            panic!("KS kind expected");
        };
        assert_eq!(ks.seed_type, 0);
        assert!((ks.ks_decay_lpf - (1.0 - 8.0 / 64.0)).abs() < f64::EPSILON);
        assert_eq!(fm.op(0).borrow().get_pulse_generator_type(), PULSE_NOISE_PINK);
        assert_eq!(fm.operator_count, 1);
    }

    // set_parameters defaults branch (KS_SEED_DEFAULT): p_params[5] INT32_MIN
    // -> pink noise, seed stays DEFAULT.
    stash.borrow()[0]
        .borrow_mut()
        .ks_set_parameters(&[i32::MIN, 40, 40, 0, 0, i32::MIN], &mut TestChip::new());
    {
        let handles = stash.borrow();
        let fm = handles[0].borrow();
        let FmKind::Ks(ks) = &fm.kind else {
            panic!("KS kind expected");
        };
        assert_eq!(ks.seed_type, 0);
        assert_eq!(fm.op(0).borrow().get_pulse_generator_type(), PULSE_NOISE_PINK);
    }

    // The KS_SEED_FM branch stores the seed raw (C++ enum cast) and runs the
    // C++ ERR_FAIL_INDEX guard on _ks_seed_index.
    stash.borrow()[0].borrow_mut().ks_set_parameters(&[1, 300, 0, 0, 0, 0], &mut TestChip::new());
    {
        let handles = stash.borrow();
        let fm = handles[0].borrow();
        let FmKind::Ks(ks) = &fm.kind else {
            panic!("KS kind expected");
        };
        assert_eq!(ks.seed_type, 1); // stored raw like the C++ cast
        assert_eq!(ks.seed_index, 300);
    }

    manager::delete_channel(&channel);
    manager::finalize();
}
