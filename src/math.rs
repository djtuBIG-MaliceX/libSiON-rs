//! Math helpers replacing `src/compat/sion_math.h` (Godot Math parity: all
//! computations promote to double).

pub fn sin(v: f64) -> f64 {
    v.sin()
}
pub fn cos(v: f64) -> f64 {
    v.cos()
}
pub fn ln(v: f64) -> f64 {
    v.ln()
}
pub fn pow(base: f64, exp: f64) -> f64 {
    base.powf(exp)
}
pub fn sqrt(v: f64) -> f64 {
    v.sqrt()
}
pub fn sinh(v: f64) -> f64 {
    v.sinh()
}
pub fn fmod(x: f64, y: f64) -> f64 {
    x % y
}
pub fn is_nan(v: f64) -> bool {
    v.is_nan()
}

/// Same formula as Godot 4.x.
pub fn linear_to_db(linear: f64) -> f64 {
    linear.ln() * 6.0 / 10f64.ln()
}

/// Godot `clamp` parity.
pub fn clampi(v: i32, min: i32, max: i32) -> i32 {
    v.clamp(min, max)
}
pub fn clampf(v: f64, min: f64, max: f64) -> f64 {
    v.clamp(min, max)
}
pub fn clampf32(v: f32, min: f32, max: f32) -> f32 {
    v.clamp(min, max)
}
