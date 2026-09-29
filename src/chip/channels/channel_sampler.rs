//! Port of `libSiON-cpp/src/chip/channels/siopm_channel_sampler.{h,cpp}`
//! (`SiOPMChannelSampler`).
//!
//! C++ `SiOPMChannelSampler : SiOPMChannelBase` → [`ChannelSampler`]
//! embedding [`ChannelBase`]; every C++ `virtual` is a
//! [`ChannelBaseTrait`] override. The sampler streams decoded sample vectors
//! straight into [`OutputStream::write_from_vector`] (no operator, no pipe
//! walk).

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use super::channel_base::{ChannelBase, ChannelBaseTrait};
use super::ChipContext;
use crate::chip::params::channel_params::{ChannelParams, STREAM_SEND_SIZE};
use crate::chip::wave::sampler_data::SiopmWaveSamplerData;
use crate::chip::wave::sampler_table::SiopmWaveSamplerTable;

/// C++ `SiOPMChannelSampler`.
pub struct ChannelSampler {
    pub base: ChannelBase,

    bank_number: i32,
    wave_number: i32,
    expression: f64,

    sampler_table: Option<Rc<RefCell<SiopmWaveSamplerTable>>>,
    sample_data: Option<Rc<RefCell<SiopmWaveSamplerData>>>,

    sample_start_phase: i32,
    sample_index: i32,
    sample_pan: i32,
}

impl ChannelSampler {
    /// C++ `SiOPMChannelSampler(SiOPMSoundChip*)` — the C++ ctor body is
    /// empty (only base member init); the manager calls `initialize`
    /// immediately after construction (`siopm_channel_manager.cpp:79-90`).
    pub fn new() -> Self {
        ChannelSampler {
            base: ChannelBase::new(),

            bank_number: 0,
            wave_number: -1,
            expression: 1.0,

            sampler_table: None,
            sample_data: None,

            sample_start_phase: 0,
            sample_index: 0,
            sample_pan: 0,
        }
    }

    fn sample_data_rc(&self) -> Option<Rc<RefCell<SiopmWaveSamplerData>>> {
        self.sample_data.clone()
    }
}

impl ChannelBaseTrait for ChannelSampler {
    fn base(&self) -> &ChannelBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut ChannelBase {
        &mut self.base
    }

    fn get_channel_params(&self, r_params: &mut ChannelParams) {
        for i in 0..STREAM_SEND_SIZE {
            r_params.set_master_volume(i as i32, self.base.volumes[i]);
        }
        r_params.set_pan(self.base.pan);
    }

    fn set_channel_params(
        &mut self,
        p_params: &ChannelParams,
        p_with_volume: bool,
        _p_with_modulation: bool,
        _ctx: &mut dyn ChipContext,
    ) {
        if p_params.get_operator_count() == 0 {
            return;
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
    }

    fn set_wave_data(&mut self, p_wave_data: &dyn Any, _ctx: &mut dyn ChipContext) {
        // C++ assigns the upcast Ref to both typed members; the compat Ref
        // cross-downcasts, so a table sets only `_sampler_table` and a data
        // object only `_sample_data` (the other becomes null).
        self.sampler_table = p_wave_data
            .downcast_ref::<Rc<RefCell<SiopmWaveSamplerTable>>>()
            .cloned();
        self.sample_data = p_wave_data
            .downcast_ref::<Rc<RefCell<SiopmWaveSamplerData>>>()
            .cloned();
    }

    fn set_types(&mut self, p_pg_type: i32, _p_pt_type: i32, _ctx: &mut dyn ChipContext) {
        self.bank_number = p_pg_type & 3;
    }

    fn get_pitch(&self) -> i32 {
        self.wave_number << 6
    }

    fn set_pitch(&mut self, p_value: i32) {
        self.wave_number = p_value >> 6;
    }

    fn set_phase(&mut self, p_value: i32) {
        self.sample_start_phase = p_value;
    }

    // Volume control.

    fn offset_volume(&mut self, p_expression: i32, p_velocity: i32) {
        self.expression = p_expression as f64 * p_velocity as f64 * 0.00006103515625; // 1/16384
    }

    // Processing.

    fn note_on(&mut self) {
        if self.wave_number < 0 {
            return;
        }

        if let Some(table) = self.sampler_table.clone() {
            self.sample_data = table.borrow().get_sample((self.wave_number & 127) as usize);
        }
        if let Some(data) = self.sample_data_rc() {
            if self.sample_start_phase != 255 {
                let initial = data
                    .borrow()
                    .get_initial_sample_index(self.sample_start_phase as f64 * 0.00390625); // 1/256
                let pan = {
                    let borrow = data.borrow();
                    self.base.pan + borrow.get_pan()
                };
                self.sample_index = initial;
                self.sample_pan = pan.clamp(0, 128);
            }
        }

        self.base.is_idling = self.sample_data.is_none();
        self.base.is_note_on = !self.base.is_idling;
    }

    fn note_off(&mut self) {
        let Some(data) = self.sample_data_rc() else {
            return;
        };
        if data.borrow().is_ignoring_note_off() {
            return;
        }

        self.base.is_note_on = false;
        self.base.is_idling = true;

        if self.sampler_table.is_some() {
            self.sample_data = None;
        }
    }

    fn buffer(&mut self, p_length: i32, ctx: &mut dyn ChipContext) {
        let skip = match self.sample_data_rc() {
            None => true,
            Some(data) => data.borrow().get_length() <= 0,
        };
        if self.base.is_idling || skip || self.base.mute {
            ChannelSampler::buffer_no_process(self, p_length, ctx);
            return;
        }

        // Stream extracted data.
        let mut residue = p_length;
        while residue > 0 {
            let data = self.sample_data.clone().expect("sample_data");

            let remaining = data.borrow().get_end_point() - self.sample_index;
            let processed = residue.min(remaining);

            let buffer_index = self.base.buffer_index;
            let volumes = self.base.volumes.clone();
            let streams = self.base.streams.clone();
            let expression = self.expression;
            let sample_index = self.sample_index;
            let sample_pan = self.sample_pan;

            if self.base.has_effect_send {
                for i in 0..STREAM_SEND_SIZE {
                    if volumes[i] > 0.0 {
                        let stream = match &streams[i] {
                            Some(stream) => Some(stream.clone()),
                            None => ctx.get_stream_slot(i),
                        };
                        if let Some(stream) = stream {
                            let volume = volumes[i] * expression * ctx.get_sampler_volume();
                            let wave_data = data.borrow().get_wave_data();
                            stream.borrow_mut().write_from_vector(
                                &wave_data,
                                sample_index,
                                buffer_index,
                                processed,
                                volume,
                                sample_pan,
                                data.borrow().get_channel_count(),
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
                    Some(stream) => {
                        let volume = volumes[0] * expression * ctx.get_sampler_volume();
                        let wave_data = data.borrow().get_wave_data();
                        stream.borrow_mut().write_from_vector(
                            &wave_data,
                            sample_index,
                            buffer_index,
                            processed,
                            volume,
                            sample_pan,
                            data.borrow().get_channel_count(),
                        );
                    }
                    None => crate::error::err_print_body("Parameter \"stream\" is null.", false),
                }
            }

            self.sample_index += processed;
            residue -= processed;

            // If processed length is not enough, try to loop; and if not,
            // stop streaming.
            if residue > 0 {
                let (loop_point, start_point) = {
                    let borrow = data.borrow();
                    (borrow.get_loop_point(), borrow.get_start_point())
                };
                if loop_point >= 0 {
                    self.sample_index = if loop_point > start_point {
                        loop_point
                    } else {
                        start_point
                    };
                } else {
                    self.base.is_idling = true;
                    if self.sampler_table.is_some() {
                        self.sample_data = None;
                    }
                    break;
                }
            }
        }

        self.base.buffer_index += p_length;
    }

    fn buffer_no_process(&mut self, p_length: i32, _ctx: &mut dyn ChipContext) {
        self.base.buffer_index += p_length;
    }

    //

    fn initialize(
        &mut self,
        p_prev: Option<&dyn ChannelBaseTrait>,
        p_buffer_index: i32,
        ctx: &mut dyn ChipContext,
    ) {
        self.initialize_base(p_prev, p_buffer_index, ctx);
        self.reset();
    }

    fn reset(&mut self) {
        self.base.is_note_on = false;
        self.base.is_idling = true;

        self.bank_number = 0;
        self.wave_number = -1;
        self.expression = 1.0;

        let table = self.base.table().clone();
        let bank0 = table.borrow().sampler_tables[0].clone();
        self.sampler_table = Some(bank0);
        self.sample_data = None;

        self.sample_start_phase = 0;
        self.sample_index = 0;
        self.sample_pan = 0;
    }
}
