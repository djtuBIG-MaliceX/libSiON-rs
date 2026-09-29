//! Port of `libSiON-cpp/src/sequencer/base/beats_per_minute.{h,cpp}` —
//! abstraction to calculate BPM-related numbers automatically.

use crate::math::clampf;
use crate::sequencer::base::mml_sequencer::FIXED_BITS;

#[derive(Clone, Debug)]
pub struct BeatsPerMinute {
    bpm: f64,
    sample_rate: i32,
    resolution: i32,

    tick_per_sample: f64,
    /// Sample per tick in fixed unit.
    sample_per_tick: f64,
    /// 16th beat per sample.
    beat_16th_per_sample: f64,
    /// Sample per 16th beat.
    sample_per_beat_16th: f64,
}

impl BeatsPerMinute {
    pub fn get_bpm(&self) -> f64 {
        self.bpm
    }

    pub fn get_sample_rate(&self) -> i32 {
        self.sample_rate
    }

    pub fn get_tick_per_sample(&self) -> f64 {
        self.tick_per_sample
    }

    pub fn get_sample_per_tick(&self) -> f64 {
        self.sample_per_tick
    }

    pub fn get_beat_16th_per_sample(&self) -> f64 {
        self.beat_16th_per_sample
    }

    pub fn get_sample_per_beat_16th(&self) -> f64 {
        self.sample_per_beat_16th
    }

    pub fn update(&mut self, p_bpm: f64, p_sample_rate: i32) -> bool {
        let bpm = clampf(p_bpm, 1.0, 511.0);

        if bpm == self.bpm && p_sample_rate == self.sample_rate {
            return false;
        }

        self.bpm = bpm;
        self.sample_rate = p_sample_rate;

        self.tick_per_sample =
            (self.resolution as f64 * self.bpm) / ((self.sample_rate * 240) as f64);
        self.beat_16th_per_sample = self.bpm / (self.sample_rate * 15) as f64; // 60 / 4
        self.sample_per_beat_16th = 1.0 / self.beat_16th_per_sample;
        self.sample_per_tick =
            (1.0 / self.tick_per_sample) * (1i64 << FIXED_BITS) as f64;

        true
    }

    /// `BeatsPerMinute(p_bpm = 120, p_sample_rate = 44100, p_resolution = 1920)`.
    pub fn new(p_bpm: f64, p_sample_rate: i32, p_resolution: i32) -> Self {
        let mut s = Self {
            bpm: 0.0,
            sample_rate: 0,
            resolution: p_resolution,
            tick_per_sample: 0.0,
            sample_per_tick: 0.0,
            beat_16th_per_sample: 0.0,
            sample_per_beat_16th: 0.0,
        };
        s.update(p_bpm, p_sample_rate);
        s
    }
}

impl Default for BeatsPerMinute {
    /// `Ref<BeatsPerMinute>::instantiate()` — default-constructed object
    /// (`BeatsPerMinute(120, 44100, 1920)`).
    fn default() -> Self {
        Self::new(120.0, 44100, 1920)
    }
}
