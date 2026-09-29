//! Port of `effector/si_effect_base.h`.
//!
//! C++ `SiEffectBase` is the abstract base of every effect and filter. Its
//! only data member is `_is_free`; the rest of the class is a virtual
//! surface (`prepare_process`, `process`, `set_by_mml`, `reset`). The Rust
//! port mirrors that surface as the object-safe trait [`EffectBase`], with
//! trait defaults matching the C++ no-op base bodies. The `_is_free` state
//! lives per concrete effect inside an embedded [`EffectSettings`] struct;
//! the trait reaches it through the required `settings`/`settings_mut`
//! accessors so `is_free`/`set_free` keep single-definition semantics.
//!
//! Buffers are always stereo-interleaved `f64` (`[L0,R0,L1,R1,...]`),
//! mirroring the C++ `std::vector<double>*`; `p_start_index`/`p_length` are
//! in sample-frames and are doubled internally to index the stereo pairs.

use crate::math::is_nan;

/// Mirrors the `SiEffectBase::_is_free` member.
#[derive(Debug, Clone, Copy)]
pub struct EffectSettings {
    pub is_free: bool,
}

impl EffectSettings {
    pub fn new() -> Self {
        EffectSettings { is_free: true }
    }
}

impl Default for EffectSettings {
    fn default() -> Self {
        Self::new()
    }
}

/// Virtual surface of C++ `SiEffectBase`.
pub trait EffectBase {
    fn settings(&self) -> &EffectSettings;
    fn settings_mut(&mut self) -> &mut EffectSettings;

    fn is_free(&self) -> bool {
        self.settings().is_free
    }

    fn set_free(&mut self, p_free: bool) {
        self.settings_mut().is_free = p_free;
    }

    fn prepare_process(&mut self) -> i32 {
        1
    }

    fn process(
        &mut self,
        p_channels: i32,
        _r_buffer: &mut [f64],
        _p_start_index: i32,
        _p_length: i32,
    ) -> i32 {
        p_channels
    }

    fn set_by_mml(&mut self, _p_args: &[f64]) {}

    fn reset(&mut self) {}
}

/// Helper for `set_by_mml` implementations; mirrors
/// `SiEffectBase::_get_mml_arg`.
pub fn get_mml_arg(p_args: &[f64], p_index: i32, p_default: f64) -> f64 {
    if p_index < 0 || (p_index as usize) >= p_args.len() {
        return p_default;
    }
    let value = p_args[p_index as usize];
    if is_nan(value) {
        p_default
    } else {
        value
    }
}
