//! Port of `effector/si_effect_stream.{h,cpp}`.
//!
//! `SiEffectStream` runs a chain of effects over its own `SiOPMStream`
//! buffer and fans the result out to chip stream slots at per-destination
//! send volumes. Fully ported in wave-6b2: the stream owns its
//! [`SiopmStream`] (the master stream aliases the chip output stream, like
//! the C++ pointer alias), `process()` writes through `_output_streams[i]`
//! with the chip-slot fallback, and `reset()` resizes the buffer from
//! `ChipContext::get_buffer_length()`.

use std::cell::RefCell;
use std::rc::Rc;

use regex::Regex;

use crate::chip::channels::{ChipContext, OutputStream};
use crate::chip::params::channel_params::STREAM_SEND_SIZE;
use crate::chip::stream::SiopmStream;
use crate::effector::effect_base::EffectBase;
use crate::effector::effector::get_effect_instance;
use crate::utils::string::{is_valid_float, to_float};

pub struct SiEffectStream {
    pub chain: Vec<Rc<RefCell<dyn EffectBase>>>,
    pub depth: i32,
    pub pan: i32,
    pub has_effect_send: bool,
    pub volumes: Vec<f64>,
    /// C++ `SiOPMStream *_stream` — the master aliases the chip output
    /// stream (C++ ctor arg), every other stream owns a fresh one
    /// (`si_effect_stream.cpp:265-274`).
    pub stream: Rc<RefCell<dyn OutputStream>>,
    /// C++ `std::vector<SiOPMStream*> _output_streams` (explicit overrides
    /// for slot 0 via `connect()`; the process tail falls back to the chip).
    pub output_streams: Vec<Option<Rc<RefCell<dyn OutputStream>>>>,
}

impl SiEffectStream {
    /// C++ `SiEffectStream(SiOPMSoundChip*, SiOPMStream *p_stream = nullptr)`:
    /// the optional stream is the master's alias of the chip output stream;
    /// `None` builds a private `SiOPMStream` like the C++ else-branch.
    pub fn new(p_stream: Option<Rc<RefCell<dyn OutputStream>>>) -> Self {
        let stream = match p_stream {
            Some(stream) => stream,
            None => Rc::new(RefCell::new(SiopmStream::new())),
        };
        SiEffectStream {
            chain: Vec::new(),
            depth: 0,
            pan: 64,
            has_effect_send: false,
            volumes: vec![0.0; STREAM_SEND_SIZE],
            stream,
            output_streams: vec![None; STREAM_SEND_SIZE],
        }
    }

    /// C++ `get_stream()`.
    pub fn get_stream(&self) -> Rc<RefCell<dyn OutputStream>> {
        self.stream.clone()
    }

    pub fn get_chain(&self) -> Vec<Rc<RefCell<dyn EffectBase>>> {
        self.chain.clone()
    }

    pub fn set_chain(&mut self, p_effects: Vec<Rc<RefCell<dyn EffectBase>>>) {
        self.chain = p_effects;
    }

    pub fn add_to_chain(&mut self, p_effect: &Rc<RefCell<dyn EffectBase>>) {
        self.chain.push(p_effect.clone());
    }

    pub fn get_depth(&self) -> i32 {
        self.depth
    }

    pub fn get_pan(&self) -> i32 {
        self.pan - 64
    }

    pub fn set_pan(&mut self, p_value: i32) {
        self.pan = (p_value + 64).clamp(0, 128);
    }

    pub fn is_outputting_directly(&self) -> bool {
        !self.has_effect_send && self.volumes[0] == 1.0 && self.pan == 64
    }

    pub fn set_all_stream_send_levels(&mut self, p_param: &[i32]) {
        for i in 0..STREAM_SEND_SIZE {
            let value = p_param[i];
            self.volumes[i] = if value == i32::MIN {
                0.0
            } else {
                (value as f64) * 0.0078125
            };
        }

        self.has_effect_send = false;
        for i in 1..STREAM_SEND_SIZE {
            if self.volumes[i] > 0.0 {
                self.has_effect_send = true;
            }
        }
    }

    pub fn set_stream_send(&mut self, p_stream_num: usize, p_volume: f64) {
        self.volumes[p_stream_num] = p_volume;
        if p_stream_num == 0 {
            return;
        }

        if p_volume > 0.0 {
            self.has_effect_send = true;
            return;
        }

        self.has_effect_send = false;
        for i in 1..STREAM_SEND_SIZE {
            if self.volumes[i] > 0.0 {
                self.has_effect_send = true;
            }
        }
    }

    pub fn get_stream_send(&self, p_stream_num: usize) -> f64 {
        self.volumes[p_stream_num]
    }

    /// C++ `connect(SiOPMStream *p_output)`: explicit override for the
    /// slot-0 destination of the process tail.
    pub fn connect(&mut self, p_output: Option<Rc<RefCell<dyn OutputStream>>>) {
        self.output_streams[0] = p_output;
    }

    pub fn prepare_process(&mut self) -> i32 {
        if self.chain.is_empty() {
            return 0;
        }

        let channel_count = self.chain[0].borrow_mut().prepare_process();
        self.stream.borrow_mut().set_channel_count(channel_count);
        for i in 1..self.chain.len() {
            self.chain[i].borrow_mut().prepare_process();
        }

        self.stream.borrow().get_channel_count()
    }

    /// C++ `process(int p_start_idx, int p_length, bool p_write_in_stream =
    /// true)`; `ctx` replaces `_sound_chip` (slot fallback in the write tail).
    pub fn process(
        &mut self,
        ctx: &dyn ChipContext,
        p_start_idx: i32,
        p_length: i32,
        p_write_in_stream: bool,
    ) -> i32 {
        let stream = self.stream.clone();
        let mut channel_count;
        {
            let mut guard = stream.borrow_mut();
            channel_count = guard.get_channel_count();
            let buffer = guard.get_buffer_mut();
            for i in 0..self.chain.len() {
                channel_count = self.chain[i]
                    .borrow_mut()
                    .process(channel_count, buffer, p_start_idx, p_length);
            }
        }

        if p_write_in_stream {
            let guard = stream.borrow();
            let buffer = guard.get_buffer();
            if self.has_effect_send {
                for i in 0..STREAM_SEND_SIZE {
                    if self.volumes[i] > 0.0 {
                        let stream = match &self.output_streams[i] {
                            Some(stream) => Some(stream.clone()),
                            None => ctx.get_stream_slot(i),
                        };
                        if let Some(stream) = stream {
                            stream.borrow_mut().write_from_vector(
                                buffer,
                                p_start_idx,
                                p_start_idx,
                                p_length,
                                self.volumes[i],
                                self.pan,
                                2,
                            );
                        }
                    }
                }
            } else {
                // C++ dereferences the slot-0 stream without a null check
                // (crash on null); the Rust port panics with a message.
                let stream = match &self.output_streams[0] {
                    Some(stream) => stream.clone(),
                    None => ctx
                        .get_output_stream()
                        .expect("SiEffectStream::process: output stream is null"),
                };
                stream.borrow_mut().write_from_vector(
                    buffer,
                    p_start_idx,
                    p_start_idx,
                    p_length,
                    self.volumes[0],
                    self.pan,
                    2,
                );
            }
        }

        channel_count
    }

    fn add_effect(&mut self, p_cmd: &str, p_args: &[f64]) {
        crate::err_fail_cond_msg!(
            p_cmd.is_empty(),
            "p_cmd.empty()",
            "SiEffectStream: Trying to add an effect with no name."
        );

        if let Some(effect) = get_effect_instance(p_cmd) {
            effect.borrow_mut().set_by_mml(p_args);
            self.chain.push(effect);
        }
    }

    fn set_postfix_param(&mut self, p_slot: usize, p_cmd: &str, p_args: &[f64], p_argc: i32) {
        if p_cmd.is_empty() {
            crate::error::err_print_body(
                &format!(
                    "SiEffectStream: Trying to set an effect param with no name in slot {}.\nCondition \"p_cmd.empty()\" is true.",
                    p_slot
                ),
                false,
            );
            return;
        }

        if p_cmd == "p" {
            self.set_pan(((p_args[0] as i32) << 4) - 64);
        } else if p_cmd == "@p" {
            self.set_pan(p_args[0] as i32);
        } else if p_cmd == "@v" {
            let value = (p_args[0] as i32) as f64 * 0.0078125;
            self.set_stream_send(0, value.clamp(0.0, 1.0));

            let mut max_count = p_argc;
            if (max_count + (p_slot as i32)) >= (STREAM_SEND_SIZE as i32) {
                max_count = STREAM_SEND_SIZE as i32 - (p_slot as i32) - 1;
            }

            let mut i = 1;
            while i < max_count {
                let value = (p_args[i as usize] as i32) as f64 * 0.0078125;
                self.set_stream_send((i + p_slot as i32) as usize, value.clamp(0.0, 1.0));
                i += 1;
            }
        } else {
            crate::err_print!(
                "SiEffectStream: Trying to set an unknown effect param ({}) in slot {}.",
                p_cmd,
                p_slot
            );
        }
    }

    pub fn parse_mml(
        &mut self,
        p_slot: usize,
        p_mml: &str,
        p_postfix: &str,
        ctx: &dyn ChipContext,
    ) {
        const MAX_ARGC: usize = 16;

        let mut command = String::new();
        let mut args: Vec<f64> = vec![f64::NAN; MAX_ARGC];
        let mut argc: usize = 0;

        self.initialize(ctx, 0);

        let re_mml = Regex::new("([a-zA-Z_]+|,)\\s*([.\\-\\d]+)?").unwrap();
        let re_postfix = Regex::new("(p|@p|@v|,)\\s*([.\\-\\d]+)?").unwrap();

        for caps in search_all(&re_mml, p_mml) {
            let group1 = caps.get(1).map_or("", |m| m.as_str());
            if group1 == "," {
                consume_arg(&caps, 2, &mut args, &mut argc);
            } else {
                if !command.is_empty() {
                    self.add_effect(&command, &args);
                }
                command.clear();
                for a in args.iter_mut() {
                    *a = f64::NAN;
                }
                argc = 0;

                command = group1.to_string();
                consume_arg(&caps, 2, &mut args, &mut argc);
            }
        }

        if !command.is_empty() {
            self.add_effect(&command, &args);
        }
        command.clear();
        for a in args.iter_mut() {
            *a = f64::NAN;
        }
        argc = 0;

        for caps in search_all(&re_postfix, p_postfix) {
            let group1 = caps.get(1).map_or("", |m| m.as_str());
            if group1 == "," {
                consume_arg(&caps, 2, &mut args, &mut argc);
            } else {
                if !command.is_empty() {
                    self.set_postfix_param(p_slot, &command, &args, argc as i32);
                }
                command.clear();
                for a in args.iter_mut() {
                    *a = f64::NAN;
                }
                argc = 0;

                command = group1.to_string();
                consume_arg(&caps, 2, &mut args, &mut argc);
            }
        }

        if !command.is_empty() {
            self.set_postfix_param(p_slot, &command, &args, argc as i32);
        }
    }

    pub fn initialize(&mut self, ctx: &dyn ChipContext, p_depth: i32) {
        self.free();
        self.reset(ctx);

        for i in 0..STREAM_SEND_SIZE {
            self.volumes[i] = 0.0;
            self.output_streams[i] = None;
        }

        self.volumes[0] = 1.0;
        self.pan = 64;
        self.has_effect_send = false;
        self.depth = p_depth;
    }

    /// C++ `reset()` — resizes the owned stream to the chip buffer length.
    pub fn reset(&mut self, ctx: &dyn ChipContext) {
        let length = (ctx.get_buffer_length() << 1) as usize;
        self.stream.borrow_mut().resize(length);
        self.stream.borrow_mut().clear();
    }

    pub fn free(&mut self) {
        for effect in self.chain.iter() {
            effect.borrow_mut().set_free(true);
        }
        self.chain.clear();
    }
}

impl Default for SiEffectStream {
    fn default() -> Self {
        // C++ ctor `SiEffectStream(chip, nullptr)` — own fresh SiOPMStream.
        Self::new(None)
    }
}

fn consume_arg(
    caps: &regex::Captures,
    p_index: usize,
    args: &mut Vec<f64>,
    argc: &mut usize,
) {
    let text = caps.get(p_index).map_or("", |m| m.as_str());
    if is_valid_float(text) {
        if *argc >= args.len() {
            args.resize(*argc + 1, f64::NAN);
        }
        args[*argc] = to_float(text);
    }
    *argc += 1;
}

/// `RegEx::search_all` from `compat/sion_regex.cpp`: leftmost matches,
/// stepping one byte forward past empty matches to avoid looping.
fn search_all<'t>(re: &Regex, subject: &'t str) -> Vec<regex::Captures<'t>> {
    let mut result = Vec::new();
    let mut offset = 0usize;
    while offset <= subject.len() {
        let Some(m) = re.find_at(subject, offset) else {
            break;
        };
        let (start, end) = (m.start(), m.end());
        if let Some(caps) = re.captures(&subject[start..end]) {
            result.push(caps);
        }
        offset = if start == end { end + 1 } else { end };
    }
    result
}
