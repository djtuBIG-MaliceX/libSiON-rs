//! `SiONTrackEvent` (`libSiON-cpp/src/events/sion_track_event.{h,cpp}`).
//!
//! C++ derives from [`SionEvent`] and reads three values off `SiONDriver`
//! in its ctor (`sequencer->get_sample_rate()`, `get_streaming_latency()`)
//! — the driver is the wave-8 `core/driver.rs` module, so the ctor takes
//! those two as explicit parameters instead (docs/PENDING.md). The C++
//! frame-trigger timer is an `int` initialized from the `double` delay
//! (truncation); reproduced. A track with a null channel hits the C++
//! `_track->get_channel()->get_buffer_index()` null deref; the port
//! panics through `SiMMLTrack::get_buffer_index` (same observation class).

use std::cell::RefCell;
use std::rc::Rc;

use crate::events::sion_event::SionEvent;
use crate::sequencer::track::TrackRc;

/// Event types, doubling as signal names.
pub const NOTE_ON_STREAM: &str = "note_on_stream";
pub const NOTE_OFF_STREAM: &str = "note_off_stream";
pub const NOTE_ON_FRAME: &str = "note_on_frame";
pub const NOTE_OFF_FRAME: &str = "note_off_frame";
pub const STREAMING_BEAT: &str = "streaming_beat";
pub const BPM_CHANGED: &str = "bpm_changed";
pub const USER_DEFINED: &str = "user_defined_event";

pub struct SiONTrackEvent {
    base: SionEvent,
    track: Option<TrackRc>,
    event_trigger_id: i32,
    note: i32,
    buffer_index: i32,
    frame_trigger_delay: f64,
    frame_trigger_timer: i32,
}

impl SiONTrackEvent {
    /// C++ `get_track()`.
    pub fn get_track(&self) -> Option<TrackRc> {
        self.track.clone()
    }

    /// C++ `get_event_trigger_id()`.
    pub fn get_event_trigger_id(&self) -> i32 {
        self.event_trigger_id
    }

    /// C++ `get_note()`.
    pub fn get_note(&self) -> i32 {
        self.note
    }

    /// C++ `get_buffer_index()`.
    pub fn get_buffer_index(&self) -> i32 {
        self.buffer_index
    }

    /// C++ `get_frame_trigger_delay()`.
    pub fn get_frame_trigger_delay(&self) -> f64 {
        self.frame_trigger_delay
    }

    /// C++ `get_event_type()` (base-class passthrough).
    pub fn get_event_type(&self) -> String {
        self.base.get_event_type()
    }

    /// C++ `decrement_timer(int p_frame_rate)`.
    pub fn decrement_timer(&mut self, p_frame_rate: i32) -> bool {
        self.frame_trigger_timer -= p_frame_rate;
        self.frame_trigger_timer <= 0
    }

    /// C++ `SiONTrackEvent(p_type, p_driver, p_track, p_buffer_index,
    /// p_note, p_event_trigger_id)`. `p_sample_rate` / `p_streaming_latency`
    /// replace the driver reads per the module header; `p_track` overrides
    /// `p_note` / `p_event_trigger_id` / `p_buffer_index` when present
    /// (channel deref panic == C++ null deref).
    pub fn new(
        p_type: String,
        p_track: Option<TrackRc>,
        p_buffer_index: i32,
        p_note: i32,
        p_event_trigger_id: i32,
        p_sample_rate: f64,
        p_streaming_latency: f64,
    ) -> Self {
        let (note, event_trigger_id, buffer_index) = match &p_track {
            Some(track) => {
                let t = track.borrow();
                (t.get_note(), t.get_event_trigger_id(), t.get_buffer_index())
            }
            None => (p_note, p_event_trigger_id, p_buffer_index),
        };
        let frame_trigger_delay =
            (buffer_index as f64) / p_sample_rate + p_streaming_latency;
        SiONTrackEvent {
            base: SionEvent::new(p_type, Vec::new()),
            track: p_track,
            event_trigger_id,
            note,
            buffer_index,
            frame_trigger_delay,
            frame_trigger_timer: frame_trigger_delay as i32,
        }
    }
}

/// C++ `Ref<SiONTrackEvent>` handle: the wave-8 driver owns the
/// `Vec<Ref<SiONTrackEvent>>` queue, pumps `decrement_timer` through it
/// and dispatches the events through `on_event(Ref<SiONEvent>)`.
pub type TrackEventRc = Rc<RefCell<SiONTrackEvent>>;
