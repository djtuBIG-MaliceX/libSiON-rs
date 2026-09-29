//! Port of `effector/si_effector.{h,cpp}`.
//!
//! `SiEffector` owns the master effect stream, up to `STREAM_SEND_SIZE`
//! global slot streams (slot 0 aliases the master stream, like the C++
//! pointer aliasing — modeled with `Rc`), a free-stream pool and per-track
//! local streams. The static `_effect_instances` registry (name → reusable
//! effect pool) becomes `thread_local` maps, per the CONVENTIONS singleton
//! pattern; `register_effect<T>` loses its template and takes an explicit
//! factory pointer that `get_effect_instance` uses to spawn new pool
//! instances, keeping the C++ reuse-then-create semantics verbatim.
//!
//! `SiOPMSoundChip`/`SiOPMStream` are ported (`chip/sound_chip.rs`,
//! `chip/stream.rs`): the ctor takes the chip's output stream `Rc` (the
//! master stream aliases it, like the C++ `_sound_chip->get_output_stream()`
//! pointer alias) and the slot/stream plumbing is wired; `SiONDriver`
//! remains wave-8 (`core/driver.rs`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::chip::channels::{ChipContext, OutputStream};
use crate::chip::params::channel_params::STREAM_SEND_SIZE;
use crate::effector::effect_base::EffectBase;
use crate::effector::effects::autopan::EffectAutopan;
use crate::effector::effects::compressor::EffectCompressor;
use crate::effector::effects::distortion::EffectDistortion;
use crate::effector::effects::downsampler::EffectDownsampler;
use crate::effector::effects::equalizer::EffectEqualizer;
use crate::effector::effects::speaker_simulator::EffectSpeakerSimulator;
use crate::effector::effects::stereo_chorus::EffectStereoChorus;
use crate::effector::effects::stereo_delay::EffectStereoDelay;
use crate::effector::effects::stereo_expander::EffectStereoExpander;
use crate::effector::effects::stereo_reverb::EffectStereoReverb;
use crate::effector::effects::wave_shaper::EffectWaveShaper;
use crate::effector::filters::all_pass::AllPassFilter;
use crate::effector::filters::band_pass::BandPassFilter;
use crate::effector::filters::controllable_high_pass::ControllableHighPass;
use crate::effector::filters::controllable_low_pass::ControllableLowPass;
use crate::effector::filters::high_boost::HighBoostFilter;
use crate::effector::filters::high_pass::HighPassFilter;
use crate::effector::filters::low_boost::LowBoostFilter;
use crate::effector::filters::low_pass::LowPassFilter;
use crate::effector::filters::notch::NotchFilter;
use crate::effector::filters::peak::PeakFilter;
use crate::effector::filters::vowel::VowelFilter;
use crate::effector::stream::SiEffectStream;

pub type EffectInstance = Rc<RefCell<dyn EffectBase>>;
pub type EffectFactory = fn() -> EffectInstance;

thread_local! {
    static EFFECT_INSTANCES: RefCell<HashMap<String, Vec<EffectInstance>>> =
        RefCell::new(HashMap::new());
    static EFFECT_FACTORIES: RefCell<HashMap<String, EffectFactory>> =
        RefCell::new(HashMap::new());
}

fn new_instance<T: EffectBase + Default + 'static>() -> EffectInstance {
    let effect: EffectInstance = Rc::new(RefCell::new(T::default()));
    effect.borrow_mut().set_free(false);
    effect.borrow_mut().reset();
    effect
}

/// C++ `register_effect<T>(p_name)`: resets the reusable instance pool for
/// the name. The concrete type arrives as `p_factory` instead of a template.
pub fn register_effect(p_name: &str, p_factory: EffectFactory) {
    EFFECT_INSTANCES.with(|instances| {
        instances
            .borrow_mut()
            .insert(p_name.to_string(), Vec::new());
    });
    EFFECT_FACTORIES.with(|factories| {
        factories
            .borrow_mut()
            .insert(p_name.to_string(), p_factory);
    });
}

/// C++ `SiEffector::get_effect_instance`: reuse a free pooled instance
/// (`set_free(false)` + `reset()`), else spawn one via the registered
/// factory and append it to the pool.
pub fn get_effect_instance(p_name: &str) -> Option<EffectInstance> {
    let has = EFFECT_INSTANCES.with(|instances| instances.borrow().contains_key(p_name));
    if !has {
        crate::error::err_print_body(
            &format!(
                "SiEffector: Effect called '{}' does not exist.\nCondition \"!_effect_instances.has(p_name)\" is true. Returning: null",
                p_name
            ),
            false,
        );
        return None;
    }

    let reuse = EFFECT_INSTANCES.with(|instances| -> Option<EffectInstance> {
        let mut instances = instances.borrow_mut();
        let list = instances.get_mut(p_name)?;
        for effect in list.iter_mut() {
            if effect.borrow().is_free() {
                effect.borrow_mut().set_free(false);
                effect.borrow_mut().reset();
                return Some(effect.clone());
            }
        }
        None
    });
    if reuse.is_some() {
        return reuse;
    }

    let factory = EFFECT_FACTORIES.with(|factories| -> Option<EffectFactory> {
        factories.borrow().get(p_name).copied()
    });
    let effect = factory?();
    EFFECT_INSTANCES.with(|instances| {
        if let Some(list) = instances.borrow_mut().get_mut(p_name) {
            list.push(effect.clone());
        }
    });
    Some(effect)
}

fn register_default_effects() {
    register_effect("autopan", new_instance::<EffectAutopan>);
    register_effect("comp", new_instance::<EffectCompressor>);
    register_effect("dist", new_instance::<EffectDistortion>);
    register_effect("ds", new_instance::<EffectDownsampler>);
    register_effect("eq", new_instance::<EffectEqualizer>);
    register_effect("speaker", new_instance::<EffectSpeakerSimulator>);
    register_effect("chorus", new_instance::<EffectStereoChorus>);
    register_effect("delay", new_instance::<EffectStereoDelay>);
    register_effect("stereo", new_instance::<EffectStereoExpander>);
    register_effect("reverb", new_instance::<EffectStereoReverb>);
    register_effect("ws", new_instance::<EffectWaveShaper>);

    register_effect("af", new_instance::<AllPassFilter>);
    register_effect("bf", new_instance::<BandPassFilter>);
    register_effect("hb", new_instance::<HighBoostFilter>);
    register_effect("hf", new_instance::<HighPassFilter>);
    register_effect("lb", new_instance::<LowBoostFilter>);
    register_effect("lf", new_instance::<LowPassFilter>);
    register_effect("nf", new_instance::<NotchFilter>);
    register_effect("pf", new_instance::<PeakFilter>);
    register_effect("vowel", new_instance::<VowelFilter>);

    register_effect("nhf", new_instance::<ControllableHighPass>);
    register_effect("nlf", new_instance::<ControllableLowPass>);
}

pub type EffectStreamRef = Rc<RefCell<SiEffectStream>>;

fn err_index_msg(p_slot: i32) {
    crate::error::err_print_body(
        &format!(
            "SiEffector: Invalid effect slot index.\nIndex p_slot = {} is out of bounds (SiOPMSoundChip::STREAM_SEND_SIZE = {}).",
            p_slot, STREAM_SEND_SIZE
        ),
        false,
    );
}

pub struct SiEffector {
    pub master_effect: EffectStreamRef,
    pub free_effect_streams: Vec<EffectStreamRef>,
    pub local_effects: Vec<EffectStreamRef>,
    pub global_effects: Vec<Option<EffectStreamRef>>,
    pub global_effect_count: i32,
}

impl SiEffector {
    /// C++ `SiEffector(SiOPMSoundChip *p_chip)`: the C++ chip pointer is
    /// only used for `chip->get_output_stream()` here (everywhere else the
    /// chip arrives as a `&dyn ChipContext` per CONVENTIONS), so the ctor
    /// takes that stream `Rc` — the master effect stream aliases it.
    pub fn new(p_output_stream: Rc<RefCell<dyn OutputStream>>) -> Self {
        let mut effector = SiEffector {
            master_effect: Rc::new(RefCell::new(SiEffectStream::new(Some(p_output_stream)))),
            free_effect_streams: Vec::new(),
            local_effects: Vec::new(),
            global_effects: vec![None; STREAM_SEND_SIZE],
            global_effect_count: 0,
        };
        effector.global_effects[0] = Some(effector.master_effect.clone());

        register_default_effects();

        effector
    }

    fn alloc_stream(&mut self, p_depth: i32, ctx: &dyn ChipContext) -> EffectStreamRef {
        let stream = match self.free_effect_streams.pop() {
            Some(stream) => stream,
            None => Rc::new(RefCell::new(SiEffectStream::new(None))),
        };
        stream.borrow_mut().initialize(ctx, p_depth);
        stream
    }

    fn get_global_stream(
        &mut self,
        p_slot: usize,
        ctx: &mut dyn ChipContext,
    ) -> EffectStreamRef {
        if (p_slot as i32) < 0 || p_slot >= STREAM_SEND_SIZE {
            err_index_msg(p_slot as i32);
            return self.master_effect.clone();
        }

        if self.global_effects[p_slot].is_none() {
            let stream = self.alloc_stream(0, ctx);
            self.global_effects[p_slot] = Some(stream.clone());
            ctx.set_stream_slot(p_slot, Some(stream.borrow().get_stream()));
            self.global_effect_count += 1;
        }

        self.global_effects[p_slot].clone().unwrap()
    }

    pub fn get_global_effect_count(&self) -> i32 {
        self.global_effect_count
    }

    pub fn get_slot_effects(&self, p_slot: i32) -> Vec<EffectInstance> {
        if p_slot < 0 || (p_slot as usize) >= STREAM_SEND_SIZE {
            err_index_msg(p_slot);
            return Vec::new();
        }

        match &self.global_effects[p_slot as usize] {
            None => Vec::new(),
            Some(stream) => stream.borrow().get_chain(),
        }
    }

    pub fn add_slot_effect(
        &mut self,
        p_slot: i32,
        p_effect: &EffectInstance,
        ctx: &mut dyn ChipContext,
    ) {
        if p_slot < 0 || (p_slot as usize) >= STREAM_SEND_SIZE {
            err_index_msg(p_slot);
            return;
        }

        let stream = self.get_global_stream(p_slot as usize, ctx);
        stream.borrow_mut().add_to_chain(p_effect);
        p_effect.borrow_mut().prepare_process();
    }

    pub fn set_slot_effects(
        &mut self,
        p_slot: i32,
        p_effects: Vec<EffectInstance>,
        ctx: &mut dyn ChipContext,
    ) {
        if p_slot < 0 || (p_slot as usize) >= STREAM_SEND_SIZE {
            err_index_msg(p_slot);
            return;
        }

        let stream = self.get_global_stream(p_slot as usize, ctx);
        stream.borrow_mut().set_chain(p_effects);
        stream.borrow_mut().prepare_process();
    }

    pub fn clear_slot_effects(&mut self, p_slot: i32, ctx: &dyn ChipContext) {
        if p_slot < 0 || (p_slot as usize) >= STREAM_SEND_SIZE {
            err_index_msg(p_slot);
            return;
        }

        if p_slot == 0 {
            self.master_effect.borrow_mut().initialize(ctx, 0);
        } else {
            if self.global_effects[p_slot as usize].is_some() {
                let stream = self.global_effects[p_slot as usize].take().unwrap();
                stream.borrow_mut().free();
                self.free_effect_streams.push(stream);
            }
        }
    }

    pub fn create_local_effect(
        &mut self,
        p_depth: i32,
        p_effects: Vec<EffectInstance>,
        ctx: &dyn ChipContext,
    ) -> EffectStreamRef {
        let effect = self.alloc_stream(p_depth, ctx);
        effect.borrow_mut().set_chain(p_effects);
        effect.borrow_mut().prepare_process();

        if p_depth == 0 {
            self.local_effects.push(effect.clone());
            return effect;
        }

        let mut i = self.local_effects.len();
        while i > 0 {
            i -= 1;
            if self.local_effects[i].borrow().get_depth() >= p_depth {
                self.local_effects.insert(i, effect.clone());
                return effect;
            }
        }

        self.local_effects.insert(0, effect.clone());
        effect
    }

    pub fn delete_local_effect(&mut self, p_effect: &EffectStreamRef) {
        if let Some(pos) = self
            .local_effects
            .iter()
            .position(|stream| Rc::ptr_eq(stream, p_effect))
        {
            self.local_effects.remove(pos);
        }
        p_effect.borrow_mut().free();
        self.free_effect_streams.push(p_effect.clone());
    }

    pub fn parse_global_effect_mml(
        &mut self,
        p_slot: usize,
        p_mml: &str,
        p_postfix: &str,
        ctx: &mut dyn ChipContext,
    ) {
        if (p_slot as i32) < 0 || p_slot >= STREAM_SEND_SIZE {
            err_index_msg(p_slot as i32);
            return;
        }

        let stream = self.get_global_stream(p_slot, ctx);
        stream.borrow_mut().parse_mml(p_slot, p_mml, p_postfix, ctx);
    }

    pub fn prepare_process(&mut self, ctx: &mut dyn ChipContext) {
        self.global_effect_count = 0;
        for i in 1..STREAM_SEND_SIZE {
            // Reset sound chip's stream slot.
            ctx.set_stream_slot(i, None);

            if let Some(stream) = &self.global_effects[i] {
                let channel_count = stream.borrow_mut().prepare_process();
                if channel_count > 0 {
                    ctx.set_stream_slot(i, Some(stream.borrow().get_stream()));
                    self.global_effect_count += 1;
                }
            }
        }

        self.master_effect.borrow_mut().prepare_process();
    }

    pub fn begin_process(&mut self) {
        // Do nothing with the master effect.

        for effect in self.local_effects.iter() {
            effect.borrow().get_stream().borrow_mut().clear();
        }

        for i in 1..STREAM_SEND_SIZE {
            if let Some(effect) = &self.global_effects[i] {
                effect.borrow().get_stream().borrow_mut().clear();
            }
        }
    }

    pub fn end_process(&mut self, ctx: &dyn ChipContext) {
        let buffer_length = ctx.get_buffer_length();

        for effect in self.local_effects.iter() {
            effect.borrow_mut().process(ctx, 0, buffer_length, true);
        }

        for i in 1..STREAM_SEND_SIZE {
            if let Some(effect) = &self.global_effects[i] {
                if effect.borrow().is_outputting_directly() {
                    effect.borrow_mut().process(ctx, 0, buffer_length, false);

                    // Mix the directly-outputting global into the chip
                    // output buffer (si_effector.cpp:281-285).
                    let source = effect.borrow().get_stream();
                    let output = ctx
                        .get_output_stream()
                        .expect("SiEffector::end_process: output stream is null");
                    {
                        let mut output = output.borrow_mut();
                        let source = source.borrow();
                        let output = output.get_buffer_mut();
                        let source = source.get_buffer();
                        for j in 0..output.len() {
                            output[j] += source[j];
                        }
                    }
                } else {
                    effect.borrow_mut().process(ctx, 0, buffer_length, true);
                }
            }
        }

        self.master_effect
            .borrow_mut()
            .process(ctx, 0, buffer_length, false);
    }

    pub fn reset(&mut self, ctx: &dyn ChipContext) {
        for stream in self.local_effects.iter() {
            stream.borrow_mut().reset(ctx);
        }

        for i in 1..STREAM_SEND_SIZE {
            if let Some(stream) = &self.global_effects[i] {
                stream.borrow_mut().reset(ctx);
            }
        }

        self.master_effect.borrow_mut().reset(ctx);
        self.global_effects[0] = Some(self.master_effect.clone());
    }

    /// C++ `SiEffector::initialize()` (called from `SiONDriver::play()`
    /// with `reset = true`): releases all local/global slot chains back to
    /// the free pool and re-initializes the master stream.
    pub fn initialize(&mut self, ctx: &dyn ChipContext) {
        for stream in self.local_effects.drain(..) {
            stream.borrow_mut().free();
            self.free_effect_streams.push(stream);
        }

        for i in 1..STREAM_SEND_SIZE {
            if let Some(stream) = self.global_effects[i].take() {
                stream.borrow_mut().free();
                self.free_effect_streams.push(stream);
            }
        }
        self.global_effect_count = 0;

        self.master_effect.borrow_mut().initialize(ctx, 0);
        self.global_effects[0] = Some(self.master_effect.clone());
    }
}
