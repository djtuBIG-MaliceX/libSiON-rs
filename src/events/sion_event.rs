//! `SiONEvent` (`libSiON-cpp/src/events/sion_event.{h,cpp}`).
//!
//! Plain event payload; the C++ stored a raw `SiONDriver *` back-pointer
//! (`_driver`, surfaced via `get_driver()` / `get_data()`). The driver is
//! the wave-8 `core/driver.rs` module, so the Rust port carries no driver
//! handle yet: `get_data()` (which dereferenced the driver) is wired in
//! wave-8, the port records the deferred fields in `docs/PENDING.md`.

/// C++ `PackedVector2Array` (`compat/sion_audio.h`: `Vector2{double x,y}`).
pub type Vector2 = (f64, f64);

/// Event types, doubling as signal names (C++ `static const char *`).
pub const QUEUE_EXECUTING: &str = "queue_executing";
pub const QUEUE_COMPLETED: &str = "queue_completed";
pub const QUEUE_CANCELLED: &str = "queue_cancelled";

pub const STREAMING: &str = "streaming";
pub const STREAM_STARTED: &str = "stream_started";
pub const STREAM_STOPPED: &str = "stream_stopped";
pub const SEQUENCE_FINISHED: &str = "sequence_finished";

pub const FADING: &str = "fading";
pub const FADE_IN_COMPLETED: &str = "fade_in_completed";
pub const FADE_OUT_COMPLETED: &str = "fade_out_completed";

#[derive(Clone)]
pub struct SionEvent {
    event_type: String,
    stream_buffer: Vec<Vector2>,
}

impl SionEvent {
    /// C++ `get_event_type()`.
    pub fn get_event_type(&self) -> String {
        self.event_type.clone()
    }

    /// C++ `get_stream_buffer()`.
    pub fn get_stream_buffer(&self) -> Vec<Vector2> {
        self.stream_buffer.clone()
    }

    /// `SiONEvent(p_type, p_driver, p_stream_buffer)` — the `p_driver`
    /// argument is dropped with the wave-8 driver deferral above.
    pub fn new(p_type: String, p_stream_buffer: Vec<Vector2>) -> Self {
        SionEvent {
            event_type: p_type,
            stream_buffer: p_stream_buffer,
        }
    }
}
