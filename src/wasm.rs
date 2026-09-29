//! WASM playground surface (wave-10) — ports nothing from libSiON-cpp.
//!
//! Exposes [`WasmPlayer`] to JavaScript for the `web/` page. The driver is
//! `Rc<RefCell<_>>`-based and therefore `!Send`/`!Sync`: the player must be
//! created, played and pumped on a single browser thread (the main/UI
//! thread driving `pump` from the audio callback glue). Never move it off
//! that thread. Panics are not caught; they surface to JS as exceptions.

use std::sync::Once;

use wasm_bindgen::prelude::*;

use crate::core::core as sion;
use crate::core::driver::SiONDriver;

static INIT: Once = Once::new();

/// Frame-oriented pump wrapper around [`SiONDriver`].
#[wasm_bindgen]
pub struct WasmPlayer {
    driver: SiONDriver,
    block_frames: usize,
    block: Vec<f32>,
    staging: std::collections::VecDeque<f32>,
}

#[wasm_bindgen]
impl WasmPlayer {
    #[wasm_bindgen(constructor)]
    pub fn new(p_sample_rate: u32, p_channels: u32) -> WasmPlayer {
        INIT.call_once(sion::initialize);
        let channels = p_channels as usize;
        let driver = SiONDriver::new(2048, p_channels as i32, p_sample_rate as i32, 0);
        WasmPlayer {
            driver,
            block_frames: 2048,
            block: vec![0.0f32; 2048 * channels],
            staging: std::collections::VecDeque::new(),
        }
    }

    pub fn play(&mut self, p_mml: String) -> bool {
        self.driver.set_auto_stop(true);
        self.driver.play_mml(p_mml, true);
        self.staging.clear();
        self.driver.is_streaming()
    }

    pub fn stop(&mut self) {
        self.driver.stop();
        self.staging.clear();
    }

    pub fn is_streaming(&self) -> bool {
        self.driver.is_streaming()
    }

    pub fn set_volume(&mut self, p_value: f64) {
        self.driver.set_volume(p_value);
    }

    /// Fills ALL of `p_out` (interleaved frames, `frames * channels`
    /// samples) from the driver; zero-fills the remainder once streaming
    /// has finished so the JS audio stream stays alive and silent.
    /// Returns `p_out.len()`.
    pub fn pump(&mut self, p_out: &mut [f32]) -> usize {
        let mut pos = 0;
        while pos < p_out.len() {
            if self.staging.is_empty() {
                if !self.driver.is_streaming() {
                    break;
                }
                self.driver.update();
                self.driver.render_chunk(&mut self.block);
                self.staging.extend(self.block.iter().copied());
            }
            let want = p_out.len() - pos;
            let take = want.min(self.staging.len());
            for i in 0..take {
                p_out[pos + i] = self.staging.pop_front().unwrap();
            }
            pos += take;
            if take == 0 {
                break;
            }
        }
        for sample in p_out[pos..].iter_mut() {
            *sample = 0.0;
        }
        p_out.len()
    }

    pub fn block_frames(&self) -> usize {
        self.block_frames
    }
}
