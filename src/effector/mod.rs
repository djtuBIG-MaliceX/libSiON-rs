//! Effector subsystem: filters + effects + composite + stream + effector.
//! Port of `libSiON-cpp/src/effector/`.

pub mod composite;
pub mod effect_base;
pub mod effects;
pub mod effector;
pub mod filters;
pub mod stream;

pub use composite::{EffectComposite, SlottedEffect, SLOTS_MAX};
pub use effect_base::{get_mml_arg, EffectBase, EffectSettings};
pub use effector::{
    get_effect_instance, register_effect, EffectFactory as EffectorFactory, EffectInstance,
    EffectStreamRef, SiEffector,
};
pub use filters::all_pass::AllPassFilter;
pub use filters::band_pass::BandPassFilter;
pub use filters::base::{ChannelValues, FilterBase, THRESHOLD as FILTER_THRESHOLD};
pub use filters::controllable_base::{ControllableFilterBase, ControllableLfo, EnvelopeCursor};
pub use filters::controllable_high_pass::ControllableHighPass;
pub use filters::controllable_low_pass::ControllableLowPass;
pub use filters::high_boost::HighBoostFilter;
pub use filters::high_pass::HighPassFilter;
pub use filters::low_boost::LowBoostFilter;
pub use filters::low_pass::LowPassFilter;
pub use filters::notch::NotchFilter;
pub use filters::peak::PeakFilter;
pub use filters::vowel::{Formant, FormantEvent, FormantTap, VowelFilter};
pub use stream::SiEffectStream;

pub use effects::autopan::EffectAutopan;
pub use effects::compressor::EffectCompressor;
pub use effects::distortion::EffectDistortion;
pub use effects::downsampler::EffectDownsampler;
pub use effects::equalizer::{EffectEqualizer, PipeChannel};
pub use effects::speaker_simulator::EffectSpeakerSimulator;
pub use effects::stereo_chorus::EffectStereoChorus;
pub use effects::stereo_delay::EffectStereoDelay;
pub use effects::stereo_expander::EffectStereoExpander;
pub use effects::stereo_reverb::EffectStereoReverb;
pub use effects::wave_shaper::EffectWaveShaper;
