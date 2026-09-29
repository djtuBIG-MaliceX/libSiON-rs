//! Port of `libSiON-cpp/src/chip/channels/siopm_channel_pcm.{h,cpp}`
//! (`SiOPMChannelPCM`).
//!
//! C++ `SiOPMChannelPCM : SiOPMChannelBase` → [`ChannelPcm`] embedding
//! [`ChannelBase`] plus one [`Operator`]; the C++ `_process_function` ctor
//! lambda (`_no_process`) becomes the [`ChannelBaseTrait::process`]
//! override. Mono/stereo PCM rendering walks the shared chip pipes by
//! absolute cursor index (`Element*` → `Pipe::*_at`, per the wave-6a seam).

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use super::channel_base::{ChannelBase, ChannelBaseTrait};
use super::operator::{EgState, Operator};
use super::{ChipContext, PipeRc};
use crate::chip::params::channel_params::{ChannelParams, STREAM_SEND_SIZE};
use crate::chip::ref_table::SiopmRefTable;
use crate::chip::wave::pcm_data::SiopmWavePcmData;
use crate::chip::wave::pcm_table::SiopmWavePcmTable;

/// C++ `SiOPMChannelPCM::IDLING_THRESHOLD`.
const IDLING_THRESHOLD: i32 = 5120;

/// C++ `SiOPMChannelPCM`.
pub struct ChannelPcm {
    pub base: ChannelBase,

    operator: Rc<RefCell<Operator>>,
    pcm_table: Option<Rc<RefCell<SiopmWavePcmTable>>>,

    // Second set of variables for stereo.
    filter_variables2: [f64; 3],

    amplitude_modulation_depth: i32,
    amplitude_modulation_output_level: i32,
    pitch_modulation_depth: i32,
    pitch_modulation_output_level: i32,

    eg_timer_initial: i32,
    lfo_timer_initial: i32,

    sample_pitch_shift: i32,
    sample_volume: f64,
    sample_pan: i32,

    // Second output pipe for stereo.
    out_pipe2: Option<PipeRc>,
}

impl ChannelPcm {
    /// C++ `SiOPMChannelPCM(SiOPMSoundChip*)` — `_operator = new
    /// SiOPMOperator(p_chip)` + the ctor-tail `initialize(nullptr, 0)`
    /// (already the PCM vtable, like a most-derived C++ ctor).
    pub fn new(ctx: &mut dyn ChipContext) -> Self {
        let mut channel = ChannelPcm {
            base: ChannelBase::new(),

            operator: Rc::new(RefCell::new(Operator::new())),
            pcm_table: None,

            filter_variables2: [0.0; 3],

            amplitude_modulation_depth: 0,
            amplitude_modulation_output_level: 0,
            pitch_modulation_depth: 0,
            pitch_modulation_output_level: 0,

            eg_timer_initial: 0,
            lfo_timer_initial: 0,

            sample_pitch_shift: 0,
            sample_volume: 1.0,
            sample_pan: 0,

            out_pipe2: None,
        };

        ChannelBaseTrait::initialize(&mut channel, None, 0, ctx);
        channel
    }

    // LFO control.

    /// C++ `_set_lfo_state(bool)`.
    fn set_lfo_state(&mut self, p_enabled: bool) {
        self.base.lfo_on = p_enabled as i32;
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
        let op = self.operator.clone();
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

    // Processing.

    /// C++ `_no_process(int p_length)` — the ctor's `_process_function`
    /// target; also the `buffer_no_process` body.
    fn pcm_no_process(&mut self, p_length: i32, ctx: &mut dyn ChipContext) {
        // Rotate the output buffers.
        let pipe_index = (self.base.buffer_index + p_length) & (ctx.get_buffer_length() - 1);
        self.base.out_pipe = ctx.get_pipe(4, pipe_index);
        self.out_pipe2 = ctx.get_pipe(3, pipe_index);
    }

    /// C++ `_update_lfo()`.
    fn pcm_update_lfo(&mut self) {
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

        let op = self.operator.clone();
        op.borrow_mut()
            .set_pm_detune(self.pitch_modulation_output_level);

        self.base.lfo_timer += self.lfo_timer_initial;
    }

    /// C++ `_process_operator_mono(int p_length, bool p_mix)`.
    fn pcm_process_mono(&mut self, p_length: i32, p_mix: bool, ctx: &mut dyn ChipContext) {
        let out_pipe = self.base.out_pipe.clone().expect("out_pipe");
        let base_pipe = if p_mix {
            out_pipe.clone()
        } else {
            ctx.get_zero_buffer()
        };

        let mut out_idx = out_pipe.borrow().cursor();
        let mut base_idx = base_pipe.borrow().cursor();

        let ope0 = self.operator.clone();
        let table = self.base.table().clone();
        let eg_timer_initial = self.eg_timer_initial;

        // Noop.
        if ope0.borrow().get_pcm_end_point() <= 0 {
            for _ in 0..p_length {
                let b = base_pipe.borrow().value_at(base_idx);
                out_pipe.borrow_mut().set_value_at(out_idx, b);
                out_idx = out_pipe.borrow().next_index(out_idx);
                base_idx = base_pipe.borrow().next_index(base_idx);
            }

            out_pipe.borrow_mut().set_cursor(out_idx);
            return;
        }

        for i in 0..p_length {
            let output;

            // Update LFO.
            self.pcm_update_lfo();

            // Update EG.
            ope0.borrow_mut().tick_eg(eg_timer_initial);

            let mut finished = false;

            // Update PG.
            {
                let mut o = ope0.borrow_mut();
                o.tick_pulse_generator(0);

                let mut t = o.get_phase() >> Operator::PCM_WAVE_FIXED_BITS;

                if t >= o.get_pcm_end_point() {
                    if o.get_pcm_loop_point() == -1 {
                        o.set_eg_state(EgState::Off);
                        o.update_eg_output();
                        finished = true;
                    } else {
                        t -= o.get_pcm_end_point() - o.get_pcm_loop_point();
                        let phase_diff = (o.get_pcm_end_point() - o.get_pcm_loop_point())
                            << Operator::PCM_WAVE_FIXED_BITS;
                        o.adjust_phase(-phase_diff);
                    }
                }

                if finished {
                    // Fast forward (current and remaining samples are 0).
                    drop(o);
                    for _ in i..p_length {
                        out_pipe.borrow_mut().set_value_at(out_idx, 0);
                        out_idx = out_pipe.borrow().next_index(out_idx);
                    }
                    break;
                }

                let mut log_idx = o.get_wave_value(t);
                log_idx += o.get_eg_output()
                    + (self.amplitude_modulation_output_level
                        >> o.get_amplitude_modulation_shift());
                output = table.borrow().log_table[log_idx as usize];
            }

            // Output and increment pointers.
            {
                let b = base_pipe.borrow().value_at(base_idx);
                out_pipe.borrow_mut().set_value_at(out_idx, output + b);
                out_idx = out_pipe.borrow().next_index(out_idx);
                base_idx = base_pipe.borrow().next_index(base_idx);
            }
        }

        out_pipe.borrow_mut().set_cursor(out_idx);
    }

    /// C++ `_process_operator_stereo(int p_length, bool p_mix)`.
    fn pcm_process_stereo(&mut self, p_length: i32, p_mix: bool, ctx: &mut dyn ChipContext) {
        let out_pipe = self.base.out_pipe.clone().expect("out_pipe");
        let out_pipe2 = self.out_pipe2.clone().expect("out_pipe2");
        let base_pipe = if p_mix {
            out_pipe.clone()
        } else {
            ctx.get_zero_buffer()
        };
        let base_pipe2 = if p_mix {
            out_pipe2.clone()
        } else {
            ctx.get_zero_buffer()
        };

        let mut out_idx = out_pipe.borrow().cursor();
        let mut base_idx = base_pipe.borrow().cursor();
        let mut out_idx2 = out_pipe2.borrow().cursor();
        let mut base_idx2 = base_pipe2.borrow().cursor();

        let ope0 = self.operator.clone();
        let table = self.base.table().clone();
        let eg_timer_initial = self.eg_timer_initial;

        // Noop.
        if ope0.borrow().get_pcm_end_point() <= 0 {
            for _ in 0..p_length {
                let b = base_pipe.borrow().value_at(base_idx);
                out_pipe.borrow_mut().set_value_at(out_idx, b);
                out_idx = out_pipe.borrow().next_index(out_idx);
                base_idx = base_pipe.borrow().next_index(base_idx);

                let b2 = base_pipe2.borrow().value_at(base_idx2);
                out_pipe2.borrow_mut().set_value_at(out_idx2, b2);
                out_idx2 = out_pipe2.borrow().next_index(out_idx2);
                base_idx2 = base_pipe2.borrow().next_index(base_idx2);
            }

            out_pipe.borrow_mut().set_cursor(out_idx);
            out_pipe2.borrow_mut().set_cursor(out_idx2);
            return;
        }

        for i in 0..p_length {
            let output_left;
            let output_right;

            // Update LFO.
            self.pcm_update_lfo();

            // Update EG.
            ope0.borrow_mut().tick_eg(eg_timer_initial);

            let mut finished = false;

            // Update PG.
            {
                let mut o = ope0.borrow_mut();
                o.tick_pulse_generator(0);

                let mut t = o.get_phase() >> Operator::PCM_WAVE_FIXED_BITS;

                if t >= o.get_pcm_end_point() {
                    if o.get_pcm_loop_point() == -1 {
                        o.set_eg_state(EgState::Off);
                        o.update_eg_output();
                        finished = true;
                    } else {
                        t -= o.get_pcm_end_point() - o.get_pcm_loop_point();
                        let phase_diff = (o.get_pcm_end_point() - o.get_pcm_loop_point())
                            << Operator::PCM_WAVE_FIXED_BITS;
                        o.adjust_phase(-phase_diff);
                    }
                }

                if finished {
                    // Fast forward.
                    drop(o);
                    for _ in i..p_length {
                        out_pipe.borrow_mut().set_value_at(out_idx, 0);
                        out_idx = out_pipe.borrow().next_index(out_idx);
                        out_pipe2.borrow_mut().set_value_at(out_idx2, 0);
                        out_idx2 = out_pipe2.borrow().next_index(out_idx2);
                    }
                    break;
                }

                // Left output.
                {
                    t <<= 1;
                    let mut log_idx = o.get_wave_value(t);
                    log_idx += o.get_eg_output()
                        + (self.amplitude_modulation_output_level
                            >> o.get_amplitude_modulation_shift());
                    output_left = table.borrow().log_table[log_idx as usize];
                }

                // Right output.
                {
                    t += 1;
                    let mut log_idx = o.get_wave_value(t);
                    log_idx += o.get_eg_output()
                        + (self.amplitude_modulation_output_level
                            >> o.get_amplitude_modulation_shift());
                    output_right = table.borrow().log_table[log_idx as usize];
                }
            }

            // Output and increment pointers.
            {
                let b = base_pipe.borrow().value_at(base_idx);
                out_pipe.borrow_mut().set_value_at(out_idx, output_left + b);
                out_idx = out_pipe.borrow().next_index(out_idx);
                base_idx = base_pipe.borrow().next_index(base_idx);

                let b2 = base_pipe2.borrow().value_at(base_idx2);
                out_pipe2.borrow_mut().set_value_at(out_idx2, output_right + b2);
                out_idx2 = out_pipe2.borrow().next_index(out_idx2);
                base_idx2 = base_pipe2.borrow().next_index(base_idx2);
            }
        }

        out_pipe.borrow_mut().set_cursor(out_idx);
        out_pipe2.borrow_mut().set_cursor(out_idx2);
    }

    /// C++ `_write_stream_mono(Element *p_output, int p_length)`.
    fn pcm_write_stream_mono(
        &self,
        p_out_pipe: &PipeRc,
        p_start: usize,
        p_length: i32,
        ctx: &mut dyn ChipContext,
    ) {
        let volume_coef = self.sample_volume * ctx.get_pcm_volume();
        let pan = (self.base.pan + self.sample_pan).clamp(0, 128);
        let buffer_index = self.base.buffer_index;
        let volumes = self.base.volumes.clone();
        let streams = self.base.streams.clone();

        if self.base.has_effect_send {
            for i in 0..STREAM_SEND_SIZE {
                if volumes[i] > 0.0 {
                    let stream = match &streams[i] {
                        Some(stream) => Some(stream.clone()),
                        None => ctx.get_stream_slot(i),
                    };
                    if let Some(stream) = stream {
                        stream.borrow_mut().write(
                            &p_out_pipe.borrow(),
                            p_start,
                            buffer_index,
                            p_length,
                            volumes[i] * volume_coef,
                            pan,
                        );
                    }
                }
            }
        } else {
            let stream = match &streams[0] {
                Some(stream) => Some(stream.clone()),
                None => ctx.get_output_stream(),
            };
            match stream {
                Some(stream) => stream.borrow_mut().write(
                    &p_out_pipe.borrow(),
                    p_start,
                    buffer_index,
                    p_length,
                    volumes[0] * volume_coef,
                    pan,
                ),
                None => crate::error::err_print_body("Parameter \"stream\" is null.", false),
            }
        }
    }

    /// C++ `_write_stream_stereo(Element *left, Element *right, int
    /// p_length)`.
    fn pcm_write_stream_stereo(
        &self,
        p_left: &PipeRc,
        p_left_start: usize,
        p_right: &PipeRc,
        p_right_start: usize,
        p_length: i32,
        ctx: &mut dyn ChipContext,
    ) {
        let volume_coef = self.sample_volume * ctx.get_pcm_volume();
        let pan = (self.base.pan + self.sample_pan).clamp(0, 128);
        let buffer_index = self.base.buffer_index;
        let volumes = self.base.volumes.clone();
        let streams = self.base.streams.clone();

        if self.base.has_effect_send {
            for i in 0..STREAM_SEND_SIZE {
                if volumes[i] > 0.0 {
                    let stream = match &streams[i] {
                        Some(stream) => Some(stream.clone()),
                        None => ctx.get_stream_slot(i),
                    };
                    if let Some(stream) = stream {
                        stream.borrow_mut().write_stereo(
                            &p_left.borrow(),
                            p_left_start,
                            &p_right.borrow(),
                            p_right_start,
                            buffer_index,
                            p_length,
                            volumes[i] * volume_coef,
                            pan,
                        );
                    }
                }
            }
        } else {
            let stream = match &streams[0] {
                Some(stream) => Some(stream.clone()),
                None => ctx.get_output_stream(),
            };
            match stream {
                Some(stream) => stream.borrow_mut().write_stereo(
                    &p_left.borrow(),
                    p_left_start,
                    &p_right.borrow(),
                    p_right_start,
                    buffer_index,
                    p_length,
                    volumes[0] * volume_coef,
                    pan,
                ),
                None => crate::error::err_print_body("Parameter \"stream\" is null.", false),
            }
        }
    }
}

impl ChannelBaseTrait for ChannelPcm {
    fn base(&self) -> &ChannelBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut ChannelBase {
        &mut self.base
    }

    fn get_channel_params(&self, r_params: &mut ChannelParams) {
        r_params.set_operator_count(1);

        r_params.set_algorithm(0);
        r_params.set_envelope_frequency_ratio(self.base.frequency_ratio);

        r_params.set_feedback(0);
        r_params.set_feedback_connection(0);

        r_params.set_lfo_wave_shape(self.base.lfo_wave_shape);
        r_params.set_lfo_frequency_step(self.base.lfo_timer_step_buffer);

        r_params.set_amplitude_modulation_depth(self.amplitude_modulation_depth);
        r_params.set_pitch_modulation_depth(self.pitch_modulation_depth);

        for i in 0..STREAM_SEND_SIZE {
            r_params.set_master_volume(i as i32, self.base.volumes[i]);
        }
        r_params.set_pan(self.base.pan);

        if let Some(op_params) = r_params.get_operator_params(0) {
            self.operator.borrow().get_operator_params(&mut op_params.borrow_mut());
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
        // set_feedback is commented out in the C++ original.

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

        if let Some(op_params) = p_params.get_operator_params(0) {
            let borrow = op_params.borrow();
            self.operator.borrow_mut().set_operator_params(&borrow);
        }
    }

    fn set_wave_data(&mut self, p_wave_data: &dyn Any, _ctx: &mut dyn ChipContext) {
        let mut pcm_data = p_wave_data
            .downcast_ref::<Rc<RefCell<SiopmWavePcmData>>>()
            .cloned();
        self.pcm_table = p_wave_data
            .downcast_ref::<Rc<RefCell<SiopmWavePcmTable>>>()
            .cloned();
        if let Some(pcm_table) = &self.pcm_table {
            pcm_data = pcm_table.borrow().get_note_data(60);
        }

        if let Some(pcm_data) = &pcm_data {
            self.sample_pitch_shift = pcm_data.borrow().get_sampling_pitch() - 4416; // 69*64
        }

        self.operator.borrow_mut().set_pcm_data(pcm_data.as_ref());
    }

    fn set_parameters(&mut self, p_params: Vec<i32>, _ctx: &mut dyn ChipContext) {
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

    fn set_types(&mut self, p_pg_type: i32, _p_pt_type: i32, ctx: &mut dyn ChipContext) {
        let pcm_table = self.base.table().clone().borrow().get_pcm_data(p_pg_type);
        if let Some(pcm_table) = pcm_table {
            self.set_wave_data(&pcm_table, ctx);
        } else {
            self.sample_pitch_shift = 0;
            self.operator.borrow_mut().set_pcm_data(None);
        }
    }

    fn set_all_attack_rate(&mut self, p_value: i32) {
        self.operator.borrow_mut().set_attack_rate(p_value);
    }

    fn set_all_release_rate(&mut self, p_value: i32) {
        self.operator.borrow_mut().set_release_rate(p_value);
    }

    fn get_pitch(&self) -> i32 {
        self.operator.borrow().get_pitch_index() + self.sample_pitch_shift
    }

    fn set_pitch(&mut self, p_value: i32) {
        if let Some(table) = self.pcm_table.clone() {
            let note = p_value >> 6;
            let pcm_data = table.borrow().get_note_data(note);

            if let Some(pcm_data) = &pcm_data {
                self.sample_pitch_shift = pcm_data.borrow().get_sampling_pitch() - 4416; // 69*64
                self.sample_volume = table.borrow().get_note_volume(note);
                self.sample_pan = table.borrow().get_note_pan(note);
            }

            self.operator.borrow_mut().set_pcm_data(pcm_data.as_ref());
        }

        self.operator
            .borrow_mut()
            .set_pitch_index(p_value - self.sample_pitch_shift);
    }

    fn set_release_rate(&mut self, p_value: i32) {
        self.operator.borrow_mut().set_release_rate(p_value);
    }

    fn set_total_level(&mut self, p_value: i32) {
        self.operator.borrow_mut().set_total_level(p_value);
    }

    fn set_fine_multiple(&mut self, p_value: i32) {
        self.operator.borrow_mut().set_fine_multiple(p_value);
    }

    fn set_phase(&mut self, p_value: i32) {
        self.operator.borrow_mut().set_key_on_phase(p_value);
    }

    fn set_detune(&mut self, p_value: i32) {
        self.operator.borrow_mut().set_ptss_detune(p_value);
    }

    fn set_fixed_pitch(&mut self, p_value: i32) {
        self.operator.borrow_mut().set_fixed_pitch_index(p_value);
    }

    fn set_ssg_envelope_control(&mut self, p_value: i32) {
        self.operator.borrow_mut().set_ssg_type(p_value);
    }

    fn set_envelope_reset(&mut self, p_reset: bool) {
        self.operator
            .borrow_mut()
            .set_envelope_reset_on_attack(p_reset);
    }

    // Volume control.

    fn offset_volume(&mut self, p_expression: i32, p_velocity: i32) {
        let expression_index = (p_expression << 1) as usize;
        let offset =
            self.base.expression_table[expression_index] + self.base.velocity_table[p_velocity as usize];

        self.operator.borrow_mut().offset_total_level(offset);
    }

    // LFO control.

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

        self.pcm_table = None;
        self.operator.borrow_mut().set_pm_detune(0);
    }

    fn set_amplitude_modulation(&mut self, p_depth: i32) {
        self.amplitude_modulation_depth = p_depth << 2;
        self.amplitude_modulation_output_level = (self.base.lfo_wave_table
            [self.base.lfo_phase as usize]
            * self.amplitude_modulation_depth)
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
            self.operator.borrow_mut().set_pm_detune(0);
        }
    }

    // Processing.

    fn note_on(&mut self) {
        self.operator.borrow_mut().note_on();
        self.base.is_note_on = true;
        self.base.is_idling = false;

        self.base.base_note_on();
    }

    fn note_off(&mut self) {
        self.operator.borrow_mut().note_off();
        self.base.is_note_on = false;

        self.base.base_note_off();
    }

    fn reset_channel_buffer_status(&mut self) {
        self.base.buffer_index = 0;
        let op = self.operator.borrow();
        self.base.is_idling =
            op.get_eg_output() > IDLING_THRESHOLD && op.get_eg_state() != EgState::Attack;
    }

    /// C++ `_process_function` — the PCM ctor pins this to `_no_process`.
    fn process(&mut self, p_length: i32, ctx: &mut dyn ChipContext) {
        self.pcm_no_process(p_length, ctx);
    }

    fn buffer(&mut self, p_length: i32, ctx: &mut dyn ChipContext) {
        if self.base.is_idling {
            ChannelPcm::buffer_no_process(self, p_length, ctx);
            return;
        }

        if self.operator.borrow().get_pcm_channel_num() == 1 {
            // Preserve the start of the output pipe.
            let start = self
                .base
                .out_pipe
                .as_ref()
                .expect("out_pipe")
                .borrow()
                .cursor();

            self.pcm_process_mono(p_length, false, ctx);

            if self.base.filter_on {
                let out_pipe = self.base.out_pipe.clone().expect("out_pipe");
                let base = self.base_mut();
                let mut variables = base.filter_variables;
                base.apply_sv_filter(&out_pipe, start, p_length, &mut variables);
                base.filter_variables = variables;
            }

            if !self.base.mute {
                let out_pipe = self.base.out_pipe.clone().expect("out_pipe");
                self.pcm_write_stream_mono(&out_pipe, start, p_length, ctx);
            }
        } else {
            // Preserve the start of output pipes.
            let left_start = self
                .base
                .out_pipe
                .as_ref()
                .expect("out_pipe")
                .borrow()
                .cursor();
            let right_start = self.out_pipe2.as_ref().expect("out_pipe2").borrow().cursor();

            self.pcm_process_stereo(p_length, false, ctx);

            if self.base.filter_on {
                let out_pipe = self.base.out_pipe.clone().expect("out_pipe");
                let out_pipe2 = self.out_pipe2.clone().expect("out_pipe2");
                {
                    let base = self.base_mut();
                    let mut variables = base.filter_variables;
                    base.apply_sv_filter(&out_pipe, left_start, p_length, &mut variables);
                    base.filter_variables = variables;
                }
                let mut variables2 = self.filter_variables2;
                self.base
                    .apply_sv_filter(&out_pipe2, right_start, p_length, &mut variables2);
                self.filter_variables2 = variables2;
            }

            if !self.base.mute {
                let out_pipe = self.base.out_pipe.clone().expect("out_pipe");
                let out_pipe2 = self.out_pipe2.clone().expect("out_pipe2");
                self.pcm_write_stream_stereo(
                    &out_pipe,
                    left_start,
                    &out_pipe2,
                    right_start,
                    p_length,
                    ctx,
                );
            }
        }
    }

    fn buffer_no_process(&mut self, p_length: i32, ctx: &mut dyn ChipContext) {
        self.pcm_no_process(p_length, ctx);
        self.base.buffer_index += p_length;
    }

    //

    fn initialize(
        &mut self,
        p_prev: Option<&dyn ChannelBaseTrait>,
        p_buffer_index: i32,
        ctx: &mut dyn ChipContext,
    ) {
        self.operator.borrow_mut().initialize(ctx);

        self.base.is_note_on = false;
        self.out_pipe2 = ctx.get_pipe(3, p_buffer_index);

        self.filter_variables2 = [0.0; 3];

        self.sample_pitch_shift = 0;
        self.sample_volume = 1.0;
        self.sample_pan = 0;

        self.initialize_base(p_prev, p_buffer_index, ctx);
    }

    fn reset(&mut self) {
        self.operator.borrow_mut().reset();
        self.base.is_note_on = false;
        self.base.is_idling = true;
    }
}
