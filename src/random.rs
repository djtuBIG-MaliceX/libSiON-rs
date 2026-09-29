//! PCG32 random generator + Godot-compatible `RandomNumberGenerator` facade.
//! Faithful port of `src/compat/sion_random.h` (pcg-c XSH-RR 64/32 parity).

use std::time::{SystemTime, UNIX_EPOCH};

pub const MULTIPLIER: u64 = 6364136223846793005;
pub const DEFAULT_INCREMENT: u64 = 1442695040888963407;

#[derive(Clone, Debug)]
pub struct Pcg32 {
    state: u64,
    inc: u64,
}

impl Default for Pcg32 {
    fn default() -> Self {
        Self { state: 0, inc: 0 }
    }
}

impl Pcg32 {
    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old
            .wrapping_mul(MULTIPLIER)
            .wrapping_add(self.inc);

        let xs = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        (xs >> rot) | (xs.wrapping_shl((!rot.wrapping_add(1)) & 31))
    }

    pub fn seed(&mut self, p_state: u64, p_seq: u64) {
        self.state = 0;
        self.inc = (p_seq << 1) | 1;
        self.next_u32();
        self.state = self.state.wrapping_add(p_state);
        self.next_u32();
    }

    /// Rejection-sampled bounded draw (pcg32_boundedrand_r parity).
    pub fn bounded(&mut self, p_bound: u32) -> u32 {
        if p_bound == 0 {
            return 0;
        }
        let threshold = u32::MAX % p_bound;
        loop {
            let r = self.next_u32();
            if r >= threshold {
                return r % p_bound;
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct RandomNumberGenerator {
    rng: Pcg32,
    seed: u64,
}

impl Default for RandomNumberGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl RandomNumberGenerator {
    pub fn new() -> Self {
        let mut s = Self {
            rng: Pcg32::default(),
            seed: 0,
        };
        s.randomize();
        s
    }

    pub fn randomize(&mut self) {
        let ticks = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        self.set_seed(ticks);
    }

    pub fn set_seed(&mut self, p_seed: u64) {
        self.seed = p_seed;
        self.rng.seed(p_seed, DEFAULT_INCREMENT);
    }

    pub fn get_seed(&self) -> u64 {
        self.seed
    }

    pub fn randi(&mut self) -> u32 {
        self.rng.next_u32()
    }

    /// Godot parity: inclusive on both ends.
    pub fn randi_range(&mut self, p_from: i32, p_to: i32) -> i32 {
        if p_from < p_to {
            (self.rng.bounded((p_to - p_from) as u32 + 1) as i32).wrapping_add(p_from)
        } else if p_to < p_from {
            (self.rng.bounded((p_from - p_to) as u32 + 1) as i32).wrapping_add(p_to)
        } else {
            p_to
        }
    }

    pub fn randf(&mut self) -> f32 {
        self.rng.next_u32() as f32 / u32::MAX as f32
    }

    /// Godot parity: two draws stitched into a 64-bit mantissa.
    pub fn randd(&mut self) -> f64 {
        let mut a = self.rng.next_u32() as u64;
        a %= 1u64 << 32;
        let mut b = self.rng.next_u32() as u64;
        b %= 1u64 << 32;
        (a as f64 * (1u64 << 32) as f64 + b as f64) * 5.421010862497356e-20
    }
}
