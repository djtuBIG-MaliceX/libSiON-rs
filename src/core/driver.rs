//! `sion_driver.{h,cpp}` — `SiONDriver`, the top-level orchestrator.
//!
//! Port of the standalone libSiON-cpp driver: owns the chip, the effector
//! (via `chip.effector`) and the sequencer; provides compile/render/stream
//! APIs, the job queue, and the event pump. The Godot-era signals became
//! the `on_*` callback fields (C++ parity per the libification notes).
//!
//! Wave-8 deferrals now wired here:
//! - the `SiONTrackEvent` queue lives on the driver (`track_event_queue`),
//!   frame events are dispatched by [`SiONDriver::update`] exactly like
//!   C++ `_process_frame_immediate`;
//! - `SiONTrackEvent` ctor args that C++ read off the driver
//!   (`sample_rate`, `streaming_latency`) are supplied from the driver
//!   state captured by the sequencer callbacks;
//! - `SiMMLSequencer::streaming_latency` stays 0 (C++ never computes a
//!   non-zero value — `sion_driver.cpp` only ever resets it to 0).
//!
//! Borrow model: the chip/sequencer are `Rc<RefCell<_>>` shared with the
//! callbacks; callbacks therefore never touch driver state directly —
//! immediate dispatches ride the `pending_dispatch` queue which the pump
//! drains right after `sequencer.process()` (same position in the pump as
//! the C++ in-process `_dispatch_event` calls). Callbacks must not call
//! back into the driver through a shared handle while it is borrowed
//! (mirrors the C++ "event callbacks must stay cheap, never reenter" rule).

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use crate::chip::channels::ChipContext;
use crate::utils::string::hex_to_int;
use crate::chip::ref_table as chip_ref_table;
use crate::chip::ref_table::SiopmRefTable;
use crate::chip::sound_chip::SiopmSoundChip;
use crate::chip::wave::pcm_data::SiopmWavePcmData;
use crate::chip::wave::pcm_table::SiopmWavePcmTable;
use crate::chip::wave::sampler_data::SiopmWaveSamplerData;
use crate::chip::wave::sampler_table::SiopmWaveSamplerTable;
use crate::chip::wave::table::SiopmWaveTable;
use crate::effector::effector::SiEffector;

use crate::events::sion_event::{
    SionEvent, Vector2, FADE_IN_COMPLETED, FADE_OUT_COMPLETED, FADING, QUEUE_CANCELLED,
    QUEUE_COMPLETED, QUEUE_EXECUTING, SEQUENCE_FINISHED, STREAMING, STREAM_STARTED,
    STREAM_STOPPED,
};
use crate::events::sion_track_event::{
    BPM_CHANGED, NOTE_OFF_FRAME, NOTE_OFF_STREAM, NOTE_ON_FRAME, NOTE_ON_STREAM, SiONTrackEvent,
    STREAMING_BEAT, TrackEventRc, USER_DEFINED,
};
use crate::sample_data::SampleData;
use crate::sequencer::base::mml_event::{self, GLOBAL_WAIT, REPEAT_ALL, TIMER};
use crate::sequencer::base::mml_parser::{self, ticks_msec};
use crate::sequencer::base::mml_sequencer::{MMLSequencerTrait, MmlDataHandle};
use crate::sequencer::base::mml_sequence::{self, MMLSequence, SeqRc};
use crate::sequencer::data::SiMMLData;
use crate::sequencer::ref_table as mml_ref_table;
use crate::sequencer::sequencer::SiMMLSequencer;
use crate::sequencer::track::{
    DRIVER_BACKGROUND, DRIVER_NOTE, DRIVER_SEQUENCE, TRACK_ID_FILTER, TrackRc, USER_CONTROLLED,
};
use crate::sequencer::track::SiMMLTrack;
use crate::sion_enums as enums;
use crate::utils::fader_util::FaderUtil;
use crate::utils::transformer_util;

use crate::core::data::SiONData;
use crate::core::voice::SiONVoice;

/// C++ `SiONDriver::VERSION`.
pub const VERSION: &str = "0.7.0.0";
/// C++ `SiONDriver::VERSION_FLAVOR`.
pub const VERSION_FLAVOR: &str = "beta8";

const TIME_AVERAGING_COUNT: usize = 8;

/// C++ `ExceptionMode` (note-on track-ID conflict behavior).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExceptionMode {
    Ignore = 0,
    Reject = 1,
    Overwrite = 2,
    Shift = 3,
}

/// Event union delivered to `on_event` (replaces the C++ `Ref<SiONEvent>`
/// + `dynamic_cast<SiONTrackEvent*>` pattern).
#[derive(Clone)]
pub enum DriverEvent {
    Event(SionEvent),
    Track(TrackEventRc),
}

impl DriverEvent {
    pub fn get_event_type(&self) -> String {
        match self {
            DriverEvent::Event(ev) => ev.get_event_type(),
            DriverEvent::Track(ev) => ev.borrow().get_event_type(),
        }
    }
}

enum PendingDispatch {
    Event(DriverEvent),
    Timer,
}

#[derive(Clone, Copy, PartialEq)]
enum FrameProcessingType {
    None,
    ProcessingQueue,
    ProcessingImmediate,
}

#[derive(Clone, Copy, PartialEq)]
enum JobType {
    NoJob,
    Compile,
    Render,
}

struct SiONDriverJob {
    job_type: JobType,
    data: Option<Rc<RefCell<SiONData>>>,
    mml_string: String,
    buffer_size: i32,
    channel_num: i32,
    reset_effector: bool,
}

thread_local! {
    static DRIVER_MUTEX: Cell<bool> = const { Cell::new(false) };
    static ALLOW_MULTIPLE_DRIVERS: Cell<bool> = const { Cell::new(false) };
}

/// C++ `SiONDriver`.
pub struct SiONDriver {
    // Signal replacements.
    pub on_event: Option<Box<dyn Fn(&DriverEvent)>>,
    pub on_compilation_finished: Option<Box<dyn Fn(&Rc<RefCell<SiONData>>)>>,
    pub on_render_finished: Option<Box<dyn Fn(&[f64])>>,
    pub on_timer_interval: Option<Box<dyn Fn()>>,

    pub chip: Rc<RefCell<SiopmSoundChip>>,
    pub sequencer: Rc<RefCell<SiMMLSequencer>>,

    data: Option<Rc<RefCell<SiONData>>>,
    mml_string: String,

    fader: FaderUtil,
    fader_volume: f64,
    master_volume: f64,

    background_sample: Option<Rc<RefCell<SampleData>>>,
    background_sample_data: Option<Rc<RefCell<SiopmWaveSamplerData>>>,
    background_loop_point: f64,
    background_voice: SiONVoice,
    background_track: Option<TrackRc>,
    background_fade_out_track: Option<TrackRc>,
    background_fade_out_frames: i32,
    background_fade_in_frames: i32,
    background_fade_gap_frames: i32,
    background_total_fade_frames: i32,
    background_fader: FaderUtil,

    buffer_length: i32,
    channel_num: i32,
    sample_rate: f64,
    bitrate: i32,

    beat_event_enabled: bool,
    stream_event_enabled: bool,
    fading_event_enabled: bool,

    is_streaming: bool,
    in_streaming_process: bool,
    preserve_stop: bool,
    suspend_streaming: bool,
    suspend_while_loading: bool,
    is_finish_sequence_dispatched: bool,
    is_paused: bool,

    render_buffer: Vec<f64>,
    render_buffer_channel_num: i32,
    render_buffer_index: i32,
    render_buffer_size_max: i32,

    chunk_buffer: Vec<f64>,
    chunk_position: usize,

    auto_stop: bool,
    start_position: f64,
    note_on_exception_mode: ExceptionMode,
    notify_change_bpm_on_position_changed: bool,

    current_frame_processing: FrameProcessingType,

    queue_interval: i32,
    queue_length: i32,
    job_progress: f64,
    current_job_type: JobType,
    job_queue: VecDeque<SiONDriverJob>,
    track_event_queue: Rc<RefCell<VecDeque<TrackEventRc>>>,
    pending_dispatch: Rc<RefCell<Vec<PendingDispatch>>>,
    beat_enabled_shared: Rc<Cell<bool>>,
    notify_bpm_shared: Rc<Cell<bool>>,

    timer_sequence: SeqRc,
    timer_interval_event: mml_event::MmlEventRef,

    // C++ `_performance_stats` (SinglyLinkedList ring -> fixed array +
    // cursor, behavior-identical).
    compiling_time: i64,
    rendering_time: i64,
    average_processing_time: i64,
    total_processing_time: i64,
    processing_time_data: [i32; TIME_AVERAGING_COUNT],
    processing_time_index: usize,
    total_processing_time_ratio: f64,
    streaming_time: i64,
    streaming_latency: f64,
    frame_timestamp: i64,
    frame_rate: i32,
}

impl SiONDriver {
    /// C++ `create(p_buffer_length, p_channel_num, p_sample_rate, p_bitrate)`.
    pub fn new(
        p_buffer_length: i32,
        p_channel_num: i32,
        p_sample_rate: i32,
        p_bitrate: i32,
    ) -> Self {
        if !ALLOW_MULTIPLE_DRIVERS.with(|a| a.get()) && DRIVER_MUTEX.with(|m| m.get()) {
            crate::err_print!("SiONDriver: Only one driver instance is allowed.");
            panic!("SiONDriver: Only one driver instance is allowed.");
        }
        DRIVER_MUTEX.with(|m| m.set(true));

        if !(p_buffer_length == 2048 || p_buffer_length == 4096 || p_buffer_length == 8192) {
            crate::err_print!(
                "SiONDriver: Buffer length can only be 2048, 4096, or 8192."
            );
            panic!("SiONDriver: invalid buffer length");
        }
        if p_channel_num != 1 && p_channel_num != 2 {
            crate::err_print!(
                "SiONDriver: Channel number can only be 1 (mono) or 2 (stereo)."
            );
            panic!("SiONDriver: invalid channel number");
        }
        if p_sample_rate != 44100 {
            crate::err_print!("SiONDriver: Sampling rate can only be 44100.");
            panic!("SiONDriver: invalid sample rate");
        }

        let chip = Rc::new(RefCell::new(SiopmSoundChip::new()));
        let sequencer = Rc::new(RefCell::new(SiMMLSequencer::new(chip.clone())));

        let track_event_queue: Rc<RefCell<VecDeque<TrackEventRc>>> =
            Rc::new(RefCell::new(VecDeque::new()));
        let pending_dispatch: Rc<RefCell<Vec<PendingDispatch>>> =
            Rc::new(RefCell::new(Vec::new()));
        let beat_enabled_shared = Rc::new(Cell::new(false));
        let notify_bpm_shared = Rc::new(Cell::new(true));

        let sample_rate = p_sample_rate as f64;
        let streaming_latency = 0.0;

        // C++ ctor callback wiring (`sion_driver.cpp:1270-1273`).
        {
            let mut seq = sequencer.borrow_mut();

            let queue = track_event_queue.clone();
            let pending = pending_dispatch.clone();
            seq.set_note_on_callback(Some(Rc::new(move |t: &mut SiMMLTrack| {
                SiONDriver::publish_note_event_in(
                    t,
                    true,
                    NOTE_ON_FRAME,
                    NOTE_ON_STREAM,
                    &queue,
                    &pending,
                    sample_rate,
                    streaming_latency,
                );
            })));

            let queue = track_event_queue.clone();
            let pending = pending_dispatch.clone();
            seq.set_note_off_callback(Some(Rc::new(move |t: &mut SiMMLTrack| {
                SiONDriver::publish_note_event_in(
                    t,
                    false,
                    NOTE_OFF_FRAME,
                    NOTE_OFF_STREAM,
                    &queue,
                    &pending,
                    sample_rate,
                    streaming_latency,
                );
            })));

            let queue = track_event_queue.clone();
            let pending = pending_dispatch.clone();
            let notify = notify_bpm_shared.clone();
            seq.set_tempo_changed_callback(Some(Rc::new(move |p_buffer_index: i32, p_dummy: bool| {
                let event = Rc::new(RefCell::new(SiONTrackEvent::new(
                    BPM_CHANGED.to_string(),
                    None,
                    p_buffer_index,
                    0,
                    0,
                    sample_rate,
                    streaming_latency,
                )));
                if p_dummy && notify.get() {
                    pending.borrow_mut().push(PendingDispatch::Event(
                        DriverEvent::Track(event),
                    ));
                } else {
                    queue.borrow_mut().push_back(event);
                }
            })));

            let queue = track_event_queue.clone();
            let enabled = beat_enabled_shared.clone();
            seq.set_beat_callback(Some(Rc::new(move |p_buffer_index: i32, p_beat_counter: i32| {
                if !enabled.get() {
                    return;
                }
                queue.borrow_mut().push_back(Rc::new(RefCell::new(SiONTrackEvent::new(
                    STREAMING_BEAT.to_string(),
                    None,
                    p_buffer_index,
                    0,
                    p_beat_counter,
                    sample_rate,
                    streaming_latency,
                ))));
            })));

            let pending = pending_dispatch.clone();
            seq.set_timer_callback(Some(Rc::new(move || {
                pending.borrow_mut().push(PendingDispatch::Timer);
            })));
        }

        // Background voice (`new SiONVoice(MODULE_SAMPLE)` + set_update_volumes).
        let background_voice = SiONVoice::new(
            enums::MODULE_SAMPLE,
            0,
            63,
            63,
            0,
            -1,
            0,
            0,
        );
        background_voice.set_update_volumes(true);

        let timer_sequence = mml_sequence::MMLSequence::new(false);
        MMLSequence::initialize(&timer_sequence);
        MMLSequence::append_new_event(&timer_sequence, REPEAT_ALL, 0, 0);
        MMLSequence::append_new_event(&timer_sequence, TIMER, 0, 0);
        let timer_interval_event =
            MMLSequence::append_new_event(&timer_sequence, GLOBAL_WAIT, 0, 0);

        SiONDriver {
            on_event: None,
            on_compilation_finished: None,
            on_render_finished: None,
            on_timer_interval: None,

            chip,
            sequencer,

            data: None,
            mml_string: String::new(),

            fader: FaderUtil::new(None, 0.0, 1.0, 60),
            fader_volume: 1.0,
            master_volume: 1.0,

            background_sample: None,
            background_sample_data: None,
            background_loop_point: -1.0,
            background_voice,
            background_track: None,
            background_fade_out_track: None,
            background_fade_out_frames: 0,
            background_fade_in_frames: 0,
            background_fade_gap_frames: 0,
            background_total_fade_frames: 0,
            background_fader: FaderUtil::new(None, 0.0, 1.0, 60),

            buffer_length: p_buffer_length,
            channel_num: p_channel_num,
            sample_rate: p_sample_rate as f64,
            bitrate: p_bitrate,

            beat_event_enabled: false,
            stream_event_enabled: false,
            fading_event_enabled: false,

            is_streaming: false,
            in_streaming_process: false,
            preserve_stop: false,
            suspend_streaming: false,
            suspend_while_loading: true,
            is_finish_sequence_dispatched: false,
            is_paused: false,

            render_buffer: Vec::new(),
            render_buffer_channel_num: 0,
            render_buffer_index: 0,
            render_buffer_size_max: 0,

            chunk_buffer: Vec::new(),
            chunk_position: 0,

            auto_stop: false,
            start_position: 0.0,
            note_on_exception_mode: ExceptionMode::Ignore,
            notify_change_bpm_on_position_changed: true,

            current_frame_processing: FrameProcessingType::None,

            queue_interval: 500,
            queue_length: 0,
            job_progress: 0.0,
            current_job_type: JobType::NoJob,
            job_queue: VecDeque::new(),
            track_event_queue,
            pending_dispatch,
            beat_enabled_shared,
            notify_bpm_shared,

            timer_sequence,
            timer_interval_event,

            compiling_time: 0,
            rendering_time: 0,
            average_processing_time: 0,
            total_processing_time: 0,
            processing_time_data: [0; TIME_AVERAGING_COUNT],
            processing_time_index: 0,
            total_processing_time_ratio: (p_sample_rate as f64)
                / ((p_buffer_length * TIME_AVERAGING_COUNT as i32) as f64),
            streaming_time: 0,
            streaming_latency: 0.0,
            frame_timestamp: 0,
            frame_rate: 1,
        }
    }

    /// C++ `are_multiple_drivers_allowed()`.
    pub fn are_multiple_drivers_allowed() -> bool {
        ALLOW_MULTIPLE_DRIVERS.with(|a| a.get())
    }

    /// C++ `set_allow_multiple_drivers(bool)`.
    pub fn set_allow_multiple_drivers(p_allow: bool) {
        ALLOW_MULTIPLE_DRIVERS.with(|a| a.set(p_allow));
    }

    pub fn get_version() -> &'static str {
        VERSION
    }

    pub fn get_version_flavor() -> &'static str {
        VERSION_FLAVOR
    }

    fn effector_handle(&self) -> Rc<RefCell<SiEffector>> {
        self.chip.borrow().effector.clone()
    }

    fn data_handle(&self) -> Option<MmlDataHandle> {
        self.data
            .as_ref()
            .map(|d| MmlDataHandle::Simml(d.borrow().data.clone()))
    }

    // --- Data / user tables -------------------------------------------------

    /// C++ `get_mml_string()`.
    pub fn get_mml_string(&self) -> String {
        self.mml_string.clone()
    }

    /// C++ `get_data()`.
    pub fn get_data(&self) -> Option<Rc<RefCell<SiONData>>> {
        self.data.clone()
    }

    /// C++ `clear_data()`.
    pub fn clear_data(&mut self) {
        self.data = None;
    }

    /// C++ `set_wave_table`.
    pub fn set_wave_table(
        &mut self,
        p_index: i32,
        p_table: &[f64],
    ) -> Option<Rc<RefCell<SiopmWaveTable>>> {
        let mut bits = -1i32;
        let mut n = p_table.len() as i32;
        while n > 0 {
            bits += 1;
            n >>= 1;
        }
        if bits < 2 {
            return None;
        }

        let mut wave_data = transformer_util::transform_pcm_data(p_table, 1, 0, true);
        wave_data.resize((1 << bits) as usize, 0);

        let wave_table = Rc::new(RefCell::new(SiopmWaveTable::new(
            wave_data,
            crate::sion_enums::PITCH_TABLE_OPM,
        )));
        chip_ref_table::instance().borrow_mut().register_wave_table(
            p_index,
            &Some(wave_table.clone()),
        );
        Some(wave_table)
    }

    /// C++ `set_pcm_wave`.
    pub fn set_pcm_wave(
        &mut self,
        p_index: i32,
        p_data: &Rc<RefCell<SampleData>>,
        p_sampling_note: f64,
        p_key_range_from: i32,
        p_key_range_to: i32,
        p_src_channel_num: i32,
        p_channel_num: i32,
    ) -> Rc<RefCell<SiopmWavePcmData>> {
        let pcm_voice = chip_ref_table::instance().borrow_mut().get_global_pcm_voice(
            p_index & (SiopmRefTable::PCM_DATA_MAX as i32 - 1),
        );
        let pcm_table = pcm_voice
            .borrow()
            .wave_data
            .as_ref()
            .and_then(|wave| wave.downcast_ref::<Rc<RefCell<SiopmWavePcmTable>>>())
            .cloned()
            .expect("SiONDriver: global pcm voice without pcm table (C++ null deref)");

        let pcm_data = Rc::new(RefCell::new(SiopmWavePcmData::new(
            &p_data.borrow(),
            (p_sampling_note * 64.0) as i32,
            p_src_channel_num,
            p_channel_num,
        )));
        pcm_table.borrow_mut().set_key_range_data(
            &Some(pcm_data.clone()),
            p_key_range_from,
            p_key_range_to,
        );
        pcm_data
    }

    /// C++ `set_sampler_wave`.
    pub fn set_sampler_wave(
        &self,
        p_index: i32,
        p_data: Option<&Rc<RefCell<SampleData>>>,
        p_ignore_note_off: bool,
        p_pan: i32,
        p_src_channel_num: i32,
        p_channel_num: i32,
    ) -> Rc<RefCell<SiopmWaveSamplerData>> {
        chip_ref_table::instance().borrow_mut().register_sampler_data(
            p_index,
            p_data,
            p_ignore_note_off,
            p_pan,
            p_src_channel_num,
            p_channel_num,
        )
    }

    /// C++ `set_pcm_voice`.
    pub fn set_pcm_voice(&self, p_index: i32, p_voice: &SiONVoice) {
        chip_ref_table::instance().borrow_mut().set_global_pcm_voice(
            p_index & (SiopmRefTable::PCM_DATA_MAX as i32 - 1),
            &p_voice.voice,
        );
    }

    /// C++ `set_sampler_table`.
    pub fn set_sampler_table(&self, p_bank: i32, p_table: Rc<RefCell<SiopmWaveSamplerTable>>) {
        let table = chip_ref_table::instance();
        let mut table = table.borrow_mut();
        table.sampler_tables[(p_bank & (SiopmRefTable::SAMPLER_TABLE_MAX as i32 - 1)) as usize] =
            p_table;
    }

    /// C++ `set_envelope_table`.
    pub fn set_envelope_table(&self, p_index: i32, p_table: Vec<i32>, p_loop_point: i32) {
        let mml = mml_ref_table::instance().expect("SiMMLRefTable not initialized");
        mml.borrow_mut().register_master_envelope_table(
            p_index,
            Some(Rc::new(RefCell::new(
                crate::sequencer::envelope_table::SiMMLEnvelopeTable::new(p_table, p_loop_point),
            ))),
        );
    }

    /// C++ `set_voice`.
    pub fn set_voice(&self, p_index: i32, p_voice: &SiONVoice) {
        if !p_voice.is_suitable_for_fm_voice() {
            crate::err_print!(
                "SiONDriver: Cannot register a voice that is not suitable to be an FM voice."
            );
            return;
        }
        let mml = mml_ref_table::instance().expect("SiMMLRefTable not initialized");
        mml.borrow_mut()
            .register_master_voice(p_index, Some(p_voice.voice.clone()));
    }

    /// C++ `clear_all_user_tables`.
    pub fn clear_all_user_tables(&self) {
        chip_ref_table::instance().borrow_mut().reset_all_user_tables();
        let mml = mml_ref_table::instance().expect("SiMMLRefTable not initialized");
        mml.borrow_mut().reset_all_user_tables();
    }

    /// C++ `create_user_controllable_track`.
    pub fn create_user_controllable_track(&self, p_track_id: i32) -> Option<TrackRc> {
        let internal_track_id = (p_track_id & TRACK_ID_FILTER) | USER_CONTROLLED;
        self.sequencer
            .borrow_mut()
            .create_controllable_track(internal_track_id, false)
    }

    /// C++ `notify_user_defined_track`.
    pub fn notify_user_defined_track(&self, p_event_trigger_id: i32, p_note: i32) {
        let residue = self.sequencer.borrow().get_stream_writing_residue();
        let event = Rc::new(RefCell::new(SiONTrackEvent::new(
            USER_DEFINED.to_string(),
            None,
            residue,
            p_note,
            p_event_trigger_id,
            self.sample_rate,
            self.streaming_latency,
        )));
        self.track_event_queue.borrow_mut().push_back(event);
    }

    // --- Sound parameters ---------------------------------------------------

    pub fn get_fader(&mut self) -> &mut FaderUtil {
        &mut self.fader
    }

    pub fn get_track_count(&self) -> i32 {
        self.sequencer.borrow().get_tracks().len() as i32
    }

    pub fn get_max_track_count(&self) -> i32 {
        self.sequencer.borrow().get_max_track_count()
    }

    pub fn set_max_track_count(&mut self, p_value: i32) {
        if p_value < 1 {
            crate::err_print!("SiONDriver: Max track limit cannot be lower than 1.");
            return;
        }
        self.sequencer.borrow_mut().set_max_track_count(p_value);
    }

    pub fn get_volume(&self) -> f64 {
        self.master_volume
    }

    pub fn set_volume(&mut self, p_value: f64) {
        if p_value < 0.0 || p_value > 1.0 {
            crate::err_print!(
                "SiONDriver: Volume must be between 0.0 and 1.0 (inclusive)."
            );
            return;
        }
        self.master_volume = p_value;
    }

    pub fn get_bpm(&self) -> f64 {
        self.sequencer.borrow().get_effective_bpm()
    }

    pub fn set_bpm(&mut self, p_value: f64) {
        if p_value < 1.0 || p_value > 4000.0 {
            crate::err_print!(
                "SiONDriver: BPM must be between 1 and 4000 (inclusive)."
            );
            return;
        }
        self.sequencer.borrow_mut().set_effective_bpm(p_value);
    }

    pub fn get_buffer_length(&self) -> i32 {
        self.buffer_length
    }

    pub fn get_channel_num(&self) -> i32 {
        self.channel_num
    }

    pub fn get_sample_rate(&self) -> f64 {
        self.sample_rate
    }

    pub fn get_bitrate(&self) -> i32 {
        self.bitrate
    }

    pub fn get_note_on_exception_mode(&self) -> ExceptionMode {
        self.note_on_exception_mode
    }

    pub fn set_note_on_exception_mode(&mut self, p_mode: ExceptionMode) {
        self.note_on_exception_mode = p_mode;
    }

    pub fn get_auto_stop(&self) -> bool {
        self.auto_stop
    }

    pub fn set_auto_stop(&mut self, p_enabled: bool) {
        self.auto_stop = p_enabled;
    }

    pub fn is_notify_change_bpm_on_position_changed(&self) -> bool {
        self.notify_change_bpm_on_position_changed
    }

    pub fn set_notify_change_bpm_on_position_changed(&mut self, p_enabled: bool) {
        self.notify_change_bpm_on_position_changed = p_enabled;
        self.notify_bpm_shared.set(p_enabled);
    }

    pub fn get_streaming_position(&self) -> f64 {
        self.sequencer.borrow().get_processed_sample_count() as f64 * 1000.0 / self.sample_rate
    }

    pub fn set_start_position(&mut self, p_value: f64) {
        self.start_position = p_value;
        let ready = self.sequencer.borrow().is_ready_to_process();
        if ready {
            self.sequencer.borrow_mut().reset_all_tracks();
            let samples = (p_value * self.sample_rate * 0.001) as i32;
            self.sequencer.borrow_mut().process_dummy(samples);
        }
    }

    pub fn get_suspend_while_loading(&self) -> bool {
        self.suspend_while_loading
    }

    pub fn set_suspend_while_loading(&mut self, p_enabled: bool) {
        self.suspend_while_loading = p_enabled;
    }

    pub fn set_beat_event_enabled(&mut self, p_enabled: bool) {
        self.beat_event_enabled = p_enabled;
        self.beat_enabled_shared.set(p_enabled);
    }

    pub fn set_stream_event_enabled(&mut self, p_enabled: bool) {
        self.stream_event_enabled = p_enabled;
    }

    pub fn set_fading_event_enabled(&mut self, p_enabled: bool) {
        self.fading_event_enabled = p_enabled;
    }

    pub fn is_streaming(&self) -> bool {
        self.is_streaming
    }

    pub fn is_paused(&self) -> bool {
        self.is_paused
    }

    pub fn get_compiling_time(&self) -> i64 {
        self.compiling_time
    }

    pub fn get_rendering_time(&self) -> i64 {
        self.rendering_time
    }

    pub fn get_processing_time(&self) -> i64 {
        self.average_processing_time
    }

    pub fn get_streaming_latency(&self) -> f64 {
        self.streaming_latency
    }

    pub fn get_queue_job_progress(&self) -> f64 {
        self.job_progress
    }

    pub fn get_queue_length(&self) -> i32 {
        self.job_queue.len() as i32
    }

    pub fn is_queue_executing(&self) -> bool {
        self.job_progress > 0.0 && self.job_progress < 1.0
    }

    // --- Background sound ---------------------------------------------------

    fn set_background_sample_inner(&mut self, p_sound: Option<Rc<RefCell<SampleData>>>) {
        self.background_sample = p_sound;
        self.background_sample_data = self
            .background_sample
            .as_ref()
            .map(|sample| {
                Rc::new(RefCell::new(SiopmWaveSamplerData::new(
                    &sample.borrow(),
                    true,
                    0,
                    2,
                    0,
                )))
            });

        if self.is_streaming {
            self.start_background_sample();
        }
    }

    pub fn set_background_sample(
        &mut self,
        p_sound: Option<Rc<RefCell<SampleData>>>,
        p_mix_level: f64,
        p_loop_point: f64,
    ) {
        self.set_background_sample_volume(p_mix_level);
        self.background_loop_point = p_loop_point;
        self.set_background_sample_inner(p_sound);
    }

    pub fn clear_background_sample(&mut self) {
        self.background_loop_point = -1.0;
        self.set_background_sample_inner(None);
    }

    fn start_background_sample(&mut self) {
        let start_frame;
        let end_frame;
        let chip = self.chip.clone();

        // Currently fading out -> stop fade out track.
        if let Some(track) = self.background_fade_out_track.take() {
            track.borrow_mut().set_disposable();
            track
                .borrow_mut()
                .key_off(0, true, &mut *chip.borrow_mut());
        }

        // Background sound is playing now -> fade out.
        if self.background_track.is_some() {
            self.background_fade_out_track = self.background_track.take();
            start_frame = 0;
        } else {
            start_frame = self.background_fade_out_frames + self.background_fade_gap_frames;
        }

        // Play sound with fade in.
        if self.background_sample_data.is_some() {
            let data = self.background_sample_data.clone().unwrap();
            {
                let mut voice = self.background_voice.voice.borrow_mut();
                voice.wave_data = Some(Rc::new(data.clone()) as Rc<dyn std::any::Any>);
            }
            if self.background_loop_point != -1.0 {
                data.borrow_mut()
                    .slice(-1, -1, (self.background_loop_point * 44100.0) as i32);
            }

            let track = self
                .sequencer
                .borrow_mut()
                .create_controllable_track(DRIVER_BACKGROUND, false)
                .expect("SiONDriver: failed to allocate background track");
            track.borrow_mut().set_expression(128);
            self.background_voice.voice.borrow().update_track_voice(
                &mut track.borrow_mut(),
                &mut *chip.borrow_mut(),
            );
            track.borrow_mut().key_on(
                60,
                0,
                (self.background_fade_out_frames + self.background_fade_gap_frames)
                    * self.buffer_length,
            );
            self.background_track = Some(track);

            end_frame = self.background_total_fade_frames;
        } else {
            self.background_voice.voice.borrow_mut().wave_data = None;
            self.background_loop_point = -1.0;
            end_frame = self.background_fade_out_frames + self.background_fade_gap_frames;
        }

        // Set up the fader. The C++ set_fade fired `_fade_background_callback`
        // with the START value right away (callback installed in the ctor);
        // nothing ever pumped `execute()`, so that single firing is the whole
        // observable background-fade behavior and is replicated inline.
        if end_frame - start_frame > 0 {
            self.background_fader
                .compute_fade(start_frame as f64, end_frame as f64, end_frame - start_frame);
            let value = self.background_fader.get_value();
            self.fade_background_callback(value);
        } else if let Some(track) = self.background_fade_out_track.take() {
            track.borrow_mut().set_disposable();
            track
                .borrow_mut()
                .key_off(0, true, &mut *chip.borrow_mut());
        }
    }

    fn fade_background_callback(&mut self, p_value: f64) {
        let chip = self.chip.clone();
        let mut fade_out = 0.0;
        let mut fade_in = 0.0;

        if self.background_fade_out_track.is_some() {
            if self.background_fade_out_frames > 0 {
                fade_out = 1.0 - p_value / self.background_fade_out_frames as f64;
                fade_out = fade_out.clamp(0.0, 1.0);
            }
            self.background_fade_out_track
                .as_ref()
                .unwrap()
                .borrow_mut()
                .set_expression((fade_out * 128.0) as i32);
        }

        if self.background_track.is_some() {
            if self.background_fade_in_frames > 0 {
                fade_in =
                    1.0 - (self.background_total_fade_frames as f64 - p_value)
                        / self.background_fade_in_frames as f64;
                fade_in = fade_in.clamp(0.0, 1.0);
            } else {
                fade_in = 1.0;
            }
            self.background_track
                .as_ref()
                .unwrap()
                .borrow_mut()
                .set_expression((fade_in * 128.0) as i32);
        }

        if self.background_fade_out_track.is_some() && (fade_out == 0.0 || fade_in == 1.0) {
            let track = self.background_fade_out_track.take().unwrap();
            track.borrow_mut().set_disposable();
            track
                .borrow_mut()
                .key_off(0, true, &mut *chip.borrow_mut());
        }
    }

    pub fn get_background_sample_fade_out_time(&self) -> f64 {
        self.background_fade_out_frames as f64 * self.buffer_length as f64 / self.sample_rate
    }

    pub fn set_background_sample_fade_out_time(&mut self, p_time: f64) {
        let ratio = self.sample_rate / self.buffer_length as f64;
        self.background_fade_out_frames = (p_time * ratio) as i32;
        self.background_total_fade_frames = self.background_fade_out_frames
            + self.background_fade_in_frames
            + self.background_fade_gap_frames;
    }

    pub fn get_background_sample_fade_in_time(&self) -> f64 {
        self.background_fade_in_frames as f64 * self.buffer_length as f64 / self.sample_rate
    }

    pub fn set_background_sample_fade_in_times(&mut self, p_time: f64) {
        let ratio = self.sample_rate / self.buffer_length as f64;
        self.background_fade_in_frames = (p_time * ratio) as i32;
        self.background_total_fade_frames = self.background_fade_out_frames
            + self.background_fade_in_frames
            + self.background_fade_gap_frames;
    }

    pub fn get_background_sample_fade_gap_time(&self) -> f64 {
        self.background_fade_gap_frames as f64 * self.buffer_length as f64 / self.sample_rate
    }

    pub fn set_background_sample_fade_gap_time(&mut self, p_time: f64) {
        let ratio = self.sample_rate / self.buffer_length as f64;
        self.background_fade_gap_frames = (p_time * ratio) as i32;
        self.background_total_fade_frames = self.background_fade_out_frames
            + self.background_fade_in_frames
            + self.background_fade_gap_frames;
    }

    pub fn get_background_sample_volume(&self) -> f64 {
        let params = self.background_voice.voice.borrow().channel_params.clone();
        params.borrow().get_master_volume(0)
    }

    pub fn set_background_sample_volume(&mut self, p_value: f64) {
        let params = self.background_voice.voice.borrow().channel_params.clone();
        params.borrow_mut().set_master_volume(0, p_value);
        if let Some(track) = &self.background_track {
            track.borrow_mut().set_master_volume((p_value * 128.0) as i32);
        }
        if let Some(track) = &self.background_fade_out_track {
            track.borrow_mut().set_master_volume((p_value * 128.0) as i32);
        }
    }

    // --- System commands ------------------------------------------------------

    fn parse_system_command(&mut self, p_data: &Rc<RefCell<SiMMLData>>) -> bool {
        let mut effect_set = false;
        let commands = p_data.borrow().base.get_system_commands();
        let effector = self.effector_handle();

        for command in commands {
            let c = command.borrow();
            if c.command == "#EFFECT" {
                effect_set = true;
                effector.borrow_mut().parse_global_effect_mml(
                    c.number as usize,
                    &c.content,
                    &c.postfix,
                    &mut *self.chip.borrow_mut(),
                );
            } else if c.command == "#WAVCOLOR" || c.command == "#WAVC" {
                let wave_color = hex_to_int(&c.content) as u32;
                let number = c.number;
                drop(c);
                let vector = transformer_util::wave_color_to_vector(wave_color, 0);
                let _ = self.set_wave_table(number, &vector);
            }
        }

        effect_set
    }

    fn prepare_compile(&mut self, p_mml: String, p_data: &Rc<RefCell<SiONData>>) {
        p_data.borrow_mut().data.borrow_mut().base.clear();
        self.data = Some(p_data.clone());
        self.mml_string = p_mml.clone();
        self.sequencer.borrow_mut().prepare_compile(
            Some(MmlDataHandle::Simml(p_data.borrow().data.clone())),
            p_mml,
        );

        self.job_progress = 0.01;
        self.compiling_time = 0;
        self.current_job_type = JobType::Compile;
    }

    fn prepare_render(
        &mut self,
        p_data: &Rc<RefCell<SiONData>>,
        p_buffer_size: i32,
        p_buffer_channel_num: i32,
        p_reset_effector: bool,
    ) {
        self.prepare_process(Some(p_data.clone()), p_reset_effector);

        self.render_buffer.clear();
        self.render_buffer.resize(p_buffer_size as usize, 0.0);

        self.render_buffer_channel_num = if p_buffer_channel_num == 2 { 2 } else { 1 };
        self.render_buffer_size_max = p_buffer_size;
        self.render_buffer_index = 0;

        self.job_progress = 0.01;
        self.rendering_time = 0;
        self.current_job_type = JobType::Render;
    }

    /// C++ `_rendering()` — one `_buffer_length`-frame offline block.
    fn rendering(&mut self) -> bool {
        // Processing (same sandwich as `_stream_block`).
        self.chip.borrow().begin_process();
        self.effector_handle().borrow_mut().begin_process();
        self.sequencer.borrow_mut().process();
        self.drain_pending();
        {
            let chip = self.chip.borrow();
            chip.effector.borrow_mut().end_process(&*chip as &dyn ChipContext);
        }
        self.chip.borrow().end_process();

        let mut finished = false;

        // Limit the rendering length.
        let rendering_length = self.buffer_length << 1;
        let mut buffer_extension = self.buffer_length << (self.render_buffer_channel_num - 1);

        if self.render_buffer_size_max != 0
            && self.render_buffer_size_max < self.render_buffer_index + buffer_extension
        {
            buffer_extension = self.render_buffer_size_max - self.render_buffer_index;
            finished = true;
        }

        // Extend the buffer.
        if (self.render_buffer.len() as i32) < self.render_buffer_index + buffer_extension {
            self.render_buffer
                .resize((self.render_buffer_index + buffer_extension) as usize, 0.0);
        }

        // Read the output.
        let channel = self.render_buffer_channel_num;
        let index = self.render_buffer_index as usize;
        let buffer_len = self.render_buffer.len();
        self.chip.borrow().with_output_buffer(|output_buffer| {
            if channel == 2 {
                let mut i = 0;
                let mut j = index;
                while i < rendering_length as usize && j < buffer_len {
                    self.render_buffer[j] = output_buffer[i];
                    i += 1;
                    j += 1;
                }
            } else {
                let mut i = 0;
                let mut j = index;
                while i < rendering_length as usize && j < buffer_len {
                    self.render_buffer[j] = output_buffer[i];
                    i += 2;
                    j += 1;
                }
            }
        });

        // Increment the index.
        self.render_buffer_index += buffer_extension;

        finished
            || (self.render_buffer_size_max == 0 && self.sequencer.borrow().is_finished())
    }

    fn prepare_stream(&mut self, p_data: Option<Rc<RefCell<SiONData>>>, p_reset_effector: bool) {
        self.prepare_process(p_data.clone(), p_reset_effector);

        self.total_processing_time = 0;
        self.processing_time_data = [0; TIME_AVERAGING_COUNT];
        self.processing_time_index = 0;

        self.chunk_buffer.clear();
        self.chunk_position = 0;

        self.is_paused = false;
        self.is_finish_sequence_dispatched = p_data.is_none();

        // Start streaming.
        self.is_streaming = true;
        self.suspend_streaming = true;

        self.set_processing_immediate();
    }

    pub fn stream(&mut self, p_reset_effector: bool) {
        self.stop();
        self.prepare_stream(None, p_reset_effector);
    }

    pub fn play(&mut self, p_data: &Rc<RefCell<SiONData>>, p_reset_effector: bool) {
        self.stop();
        self.prepare_stream(Some(p_data.clone()), p_reset_effector);
    }

    /// C++ convenience overload (`play_mml` compiles first).
    pub fn play_mml(&mut self, p_mml: String, p_reset_effector: bool) {
        let data = self.compile(p_mml);
        self.play(&data, p_reset_effector);
    }

    pub fn stop(&mut self) {
        if !self.is_streaming {
            return;
        }
        if self.in_streaming_process {
            self.preserve_stop = true;
            return;
        }

        self.preserve_stop = false;
        self.is_paused = false;
        self.is_streaming = false;

        // Original SiON doesn't clear the data, but that seems like an oversight.
        self.clear_data();
        self.clear_background_sample();
        self.clear_processing();

        self.fader.stop();
        self.fader_volume = 1.0;

        self.chunk_buffer.clear();
        self.chunk_position = 0;

        self.sequencer.borrow_mut().stop_sequence();

        self.dispatch_event(&DriverEvent::Event(SionEvent::new(
            STREAM_STOPPED.to_string(),
            Vec::new(),
        )));

        self.streaming_latency = 0.0;
    }

    pub fn reset(&mut self) {
        self.sequencer.borrow_mut().reset_all_tracks();
    }

    pub fn pause(&mut self) {
        if self.is_streaming {
            self.is_paused = true;
        }
    }

    pub fn resume(&mut self) {
        self.is_paused = false;
    }

    pub fn compile(&mut self, p_mml: String) -> Rc<RefCell<SiONData>> {
        self.stop();

        let start_time = ticks_msec();
        let temp_data = Rc::new(RefCell::new(SiONData::new()));
        self.prepare_compile(p_mml, &temp_data);

        // 0 ensures the process completes within the same "frame".
        self.job_progress = self.sequencer.borrow_mut().compile(0);
        self.compiling_time = ticks_msec() - start_time;
        self.mml_string = String::new();

        let data = self.data.clone().expect("prepare_compile stored data");
        if let Some(callback) = &self.on_compilation_finished {
            callback(&data);
        }
        data
    }

    pub fn queue_compile(&mut self, p_mml: String) -> i32 {
        if p_mml.is_empty() {
            crate::err_print!(
                "SiONDriver: Cannot queue a compile task, the MML string is empty."
            );
            return self.job_queue.len() as i32;
        }

        let sion_data = Rc::new(RefCell::new(SiONData::new()));
        self.job_queue.push_back(SiONDriverJob {
            job_type: JobType::Compile,
            data: Some(sion_data),
            mml_string: p_mml,
            buffer_size: 0,
            channel_num: 2,
            reset_effector: false,
        });
        self.job_queue.len() as i32
    }

    /// C++ `render(p_data, ...)` — offline render to an interleaved buffer.
    pub fn render(
        &mut self,
        p_data: &Rc<RefCell<SiONData>>,
        p_buffer_size: i32,
        p_buffer_channel_num: i32,
        p_reset_effector: bool,
    ) -> Vec<f64> {
        self.stop();

        let start_time = ticks_msec();
        self.prepare_render(p_data, p_buffer_size, p_buffer_channel_num, p_reset_effector);

        // Render everything.
        loop {
            if self.rendering() {
                break;
            }
        }
        self.rendering_time = ticks_msec() - start_time;

        let buffer = self.render_buffer.clone();
        if let Some(callback) = &self.on_render_finished {
            callback(&buffer);
        }
        buffer
    }

    /// C++ `render_mml(p_mml, ...)` convenience overload.
    pub fn render_mml(
        &mut self,
        p_mml: String,
        p_buffer_size: i32,
        p_buffer_channel_num: i32,
        p_reset_effector: bool,
    ) -> Vec<f64> {
        let data = self.compile(p_mml);
        self.render(&data, p_buffer_size, p_buffer_channel_num, p_reset_effector)
    }

    /// C++ `queue_render(Ref<SiONData>, ...)`.
    pub fn queue_render(
        &mut self,
        p_data: &Rc<RefCell<SiONData>>,
        p_buffer_size: i32,
        p_buffer_channel_num: i32,
        p_reset_effector: bool,
    ) -> i32 {
        if p_buffer_size <= 0 {
            crate::err_print!(
                "SiONDriver: Cannot queue a render task, the buffer size must be a positive number."
            );
            return self.job_queue.len() as i32;
        }

        self.job_queue.push_back(SiONDriverJob {
            job_type: JobType::Render,
            data: Some(p_data.clone()),
            mml_string: String::new(),
            buffer_size: p_buffer_size,
            channel_num: p_buffer_channel_num,
            reset_effector: p_reset_effector,
        });
        self.job_queue.len() as i32
    }

    /// C++ `queue_render(String, ...)` convenience overload (data is
    /// shared between the queued compile + render).
    pub fn queue_render_mml(
        &mut self,
        p_mml: String,
        p_buffer_size: i32,
        p_buffer_channel_num: i32,
        p_reset_effector: bool,
    ) -> i32 {
        if p_mml.is_empty() {
            crate::err_print!(
                "SiONDriver: Cannot queue a render task, the MML string is empty."
            );
            return self.job_queue.len() as i32;
        }
        if p_buffer_size <= 0 {
            crate::err_print!(
                "SiONDriver: Cannot queue a render task, the buffer size must be a positive number."
            );
            return self.job_queue.len() as i32;
        }

        let sion_data = Rc::new(RefCell::new(SiONData::new()));
        self.job_queue.push_back(SiONDriverJob {
            job_type: JobType::Compile,
            data: Some(sion_data.clone()),
            mml_string: p_mml,
            buffer_size: 0,
            channel_num: 2,
            reset_effector: false,
        });

        self.queue_render(&sion_data, p_buffer_size, p_buffer_channel_num, p_reset_effector)
    }

    pub fn fade_in(&mut self, p_time: f64) {
        let frames = (p_time * self.sample_rate / self.buffer_length as f64) as i32;
        // C++ set_fade fired `_fade_callback(0.0)` synchronously; replicate.
        self.fader.compute_fade(0.0, 1.0, frames);
        if frames != 0 {
            let value = self.fader.get_value();
            self.on_fade_step(value);
        }
    }

    pub fn fade_out(&mut self, p_time: f64) {
        let frames = (p_time * self.sample_rate / self.buffer_length as f64) as i32;
        self.fader.compute_fade(1.0, 0.0, frames);
        if frames != 0 {
            let value = self.fader.get_value();
            self.on_fade_step(value);
        }
    }

    fn convert_event_length(&self, p_length: f64) -> i32 {
        // Driver methods expect length in 1/16ths of a beat; the event
        // length is in resolution units.
        let resolution = self
            .sequencer
            .borrow()
            .base()
            .get_parser_settings()
            .borrow()
            .resolution;
        let beat_resolution = resolution as f64 / 4.0;
        (p_length * beat_resolution * 0.0625) as i32
    }

    fn on_fade_step(&mut self, p_value: f64) {        self.fader_volume = p_value;
        if !self.fading_event_enabled {
            return;
        }
        self.dispatch_event(&DriverEvent::Event(SionEvent::new(
            FADING.to_string(),
            Vec::new(),
        )));
    }

    pub fn set_beat_callback_interval(&mut self, p_length_16th: f64) {
        if p_length_16th < 0.0 {
            crate::err_print!(
                "SiONDriver: Beat callback interval value cannot be less than zero."
            );
            return;
        }

        let mut filter = 1i32;
        let mut length = p_length_16th;
        while length > 1.5 {
            filter <<= 1;
            length *= 0.5;
        }

        self.sequencer.borrow_mut().set_beat_callback_filter(filter - 1);
    }

    pub fn set_timer_interval(&mut self, p_length: f64) {
        if p_length < 0.0 {
            crate::err_print!("SiONDriver: Timer interval value cannot be less than zero.");
            return;
        }

        let length = self.convert_event_length(p_length);
        let event = self.timer_interval_event;
        mml_parser::instance().borrow_mut().events[event].set_length(length);

        if p_length > 0.0 {
            let pending = self.pending_dispatch.clone();
            self.sequencer
                .borrow_mut()
                .set_timer_callback(Some(Rc::new(move || {
                    pending.borrow_mut().push(PendingDispatch::Timer);
                })));
        } else {
            self.sequencer.borrow_mut().set_timer_callback(None);
        }
    }

    // --- Processing pump ------------------------------------------------------

    fn set_processing_queue(&mut self) {
        if self.current_frame_processing != FrameProcessingType::None {
            crate::err_print!(
                "SiONDriver: Cannot begin processing the queue, driver is busy ({}).",
                self.current_frame_processing as i32
            );
            return;
        }
        self.current_frame_processing = FrameProcessingType::ProcessingQueue;
    }

    fn set_processing_immediate(&mut self) {
        if self.current_frame_processing != FrameProcessingType::None {
            crate::err_print!(
                "SiONDriver: Cannot begin immediate processing, driver is busy ({}).",
                self.current_frame_processing as i32
            );
            return;
        }
        self.current_frame_processing = FrameProcessingType::ProcessingImmediate;
        self.frame_timestamp = ticks_msec();
    }

    fn clear_processing(&mut self) {
        self.current_frame_processing = FrameProcessingType::None;
    }

    /// C++ `_prepare_process` — order of operations is critical
    /// (`sion_driver.cpp:910-950`).
    fn prepare_process(&mut self, p_data: Option<Rc<RefCell<SiONData>>>, p_reset_effector: bool) {
        if p_data.is_some() {
            self.data = p_data;
        }

        let data_handle = self.data_handle();

        // Initialize DSP / reset all channels.
        self.chip
            .borrow_mut()
            .initialize(self.channel_num, self.bitrate, self.buffer_length);
        self.chip.borrow_mut().reset();

        // Initialize or reset effectors.
        let effector = self.effector_handle();
        if p_reset_effector {
            effector
                .borrow_mut()
                .initialize(&mut *self.chip.borrow_mut());
        } else {
            effector.borrow_mut().reset(&*self.chip.borrow() as &dyn ChipContext);
        }

        // Set sequencer tracks (after sound_chip reset).
        self.sequencer.borrow_mut().prepare_process(
            data_handle,
            self.sample_rate as i32,
            self.buffer_length,
        );

        // Parse #EFFECT (after effector reset).
        let data_inner = self
            .data
            .as_ref()
            .map(|d| d.borrow().data.clone());
        if let Some(inner) = data_inner {
            self.parse_system_command(&inner);
        }

        // Set effector connections.
        effector
            .borrow_mut()
            .prepare_process(&mut *self.chip.borrow_mut());

        self.track_event_queue.borrow_mut().clear();
        self.pending_dispatch.borrow_mut().clear();

        // Set position if we don't start from the top.
        let has_data = self.data.is_some();
        if has_data && self.start_position > 0.0 {
            let samples = (self.start_position * self.sample_rate * 0.001) as i32;
            self.sequencer.borrow_mut().process_dummy(samples);
        }

        if self.background_sample_data.is_some() {
            self.start_background_sample();
        }

        let timer_active = mml_parser::instance().borrow().events[self.timer_interval_event]
            .get_length()
            > 0;
        if timer_active {
            self.sequencer
                .borrow_mut()
                .set_global_sequence(Some(self.timer_sequence.clone()));
        }
    }

    pub fn update(&mut self) {
        if self.current_frame_processing != FrameProcessingType::None {
            self.process_frame();
        }
    }

    fn process_frame(&mut self) {
        match self.current_frame_processing {
            FrameProcessingType::ProcessingQueue => self.process_frame_queue(),
            FrameProcessingType::ProcessingImmediate => self.process_frame_immediate(),
            FrameProcessingType::None => {}
        }
    }

    fn process_frame_queue(&mut self) {
        let start_time = ticks_msec();

        match self.current_job_type {
            JobType::Compile => {
                let interval = self.queue_interval;
                self.job_progress = self.sequencer.borrow_mut().compile(interval);
                self.compiling_time += ticks_msec() - start_time;
            }
            JobType::Render => {
                self.job_progress += (1.0 - self.job_progress) * 0.5;

                let mut rendering_time = ticks_msec() - start_time;
                while rendering_time <= self.queue_interval as i64 {
                    if self.rendering() {
                        self.job_progress = 1.0;
                        break;
                    }
                    rendering_time = ticks_msec() - start_time;
                }

                self.rendering_time += ticks_msec() - start_time;
            }
            JobType::NoJob => {}
        }

        // Finish the job and prepare the next one.
        if self.job_progress == 1.0 {
            match self.current_job_type {
                JobType::Compile => {
                    if let (Some(callback), Some(data)) =
                        (&self.on_compilation_finished, &self.data)
                    {
                        callback(data);
                    }
                }
                JobType::Render => {
                    if let Some(callback) = &self.on_render_finished {
                        let buffer = self.render_buffer.clone();
                        callback(&buffer);
                    }
                }
                JobType::NoJob => {}
            }

            if self.prepare_next_job() {
                return; // Queue is finished.
            }
        }

        self.dispatch_event(&DriverEvent::Event(SionEvent::new(
            QUEUE_EXECUTING.to_string(),
            Vec::new(),
        )));
    }

    fn process_frame_immediate(&mut self) {
        // Calculate the framerate.
        let t = ticks_msec();
        self.frame_rate = (t - self.frame_timestamp) as i32;
        self.frame_timestamp = t;

        // This is true at the start of streaming.
        if self.suspend_streaming {
            self.suspend_streaming = false;
            self.dispatch_event(&DriverEvent::Event(SionEvent::new(
                STREAM_STARTED.to_string(),
                Vec::new(),
            )));
            return;
        }

        if self.preserve_stop {
            self.stop();
        }

        // Process events and keep the ones which are still remaining.
        if !self.track_event_queue.borrow().is_empty() {
            let events: Vec<TrackEventRc> =
                self.track_event_queue.borrow_mut().drain(..).collect();
            for event in events {
                if event.borrow_mut().decrement_timer(self.frame_rate) {
                    self.dispatch_event(&DriverEvent::Track(event));
                } else {
                    self.track_event_queue.borrow_mut().push_back(event);
                }
            }
        }
    }

    fn prepare_next_job(&mut self) -> bool {
        self.data = None;
        self.mml_string = String::new();

        self.current_job_type = JobType::NoJob;
        if self.job_queue.is_empty() {
            self.queue_length = 0;
            self.clear_processing();

            self.dispatch_event(&DriverEvent::Event(SionEvent::new(
                QUEUE_COMPLETED.to_string(),
                Vec::new(),
            )));
            return true;
        }

        let job = self.job_queue.pop_front().unwrap();

        match job.job_type {
            JobType::Compile => {
                if job.mml_string.is_empty() {
                    crate::warn_print!(
                        "SiONDriver: Invalid compile job queued up, missing MML string."
                    );
                    return self.prepare_next_job();
                }
                let data = job.data.clone().expect("queued compile job carries data");
                self.prepare_compile(job.mml_string, &data);
            }
            JobType::Render => {
                if job.buffer_size <= 0 {
                    crate::warn_print!(
                        "SiONDriver: Invalid render job queued up, buffer size must be a positive number."
                    );
                    return self.prepare_next_job();
                }
                let data = job.data.clone().expect("queued render job carries data");
                self.prepare_render(
                    &data,
                    job.buffer_size,
                    job.channel_num,
                    job.reset_effector,
                );
            }
            JobType::NoJob => {
                crate::warn_print!("SiONDriver: Unknown job queued up.");
                return self.prepare_next_job();
            }
        }

        false
    }

    pub fn start_queue(&mut self, p_interval: i32) -> i32 {
        self.stop();

        self.queue_length = self.job_queue.len() as i32;
        if self.queue_length > 0 {
            self.queue_interval = p_interval;
            self.prepare_next_job();
            self.set_processing_queue();
        }

        self.queue_length
    }

    /// C++ `_cancel_all_jobs` (defined but uncalled in the C++ tree;
    /// exposed here for parity with the declaration).
    pub fn cancel_all_jobs(&mut self) {
        self.data = None;
        self.mml_string = String::new();

        self.current_job_type = JobType::NoJob;
        self.job_progress = 0.0;
        self.job_queue.clear();
        self.queue_length = 0;
        self.clear_processing();

        self.dispatch_event(&DriverEvent::Event(SionEvent::new(
            QUEUE_CANCELLED.to_string(),
            Vec::new(),
        )));
    }

    pub fn get_queue_total_progress(&self) -> f64 {
        if self.queue_length == 0 {
            return 1.0;
        }
        if self.queue_length == self.job_queue.len() as i32 {
            return 0.0;
        }
        (self.queue_length as f64 - self.job_queue.len() as f64 - 1.0 + self.job_progress)
            / self.queue_length as f64
    }

    /// C++ `_publish_note_event` (static seam for the sequencer callbacks —
    /// frame events join the deferred queue, stream events ride the
    /// immediate-dispatch queue drained right after `process()`).
    #[allow(clippy::too_many_arguments)]
    fn publish_note_event_in(
        p_track: &mut SiMMLTrack,
        p_note_on: bool,
        p_frame_event: &str,
        p_stream_event: &str,
        queue: &Rc<RefCell<VecDeque<TrackEventRc>>>,
        pending: &Rc<RefCell<Vec<PendingDispatch>>>,
        p_sample_rate: f64,
        p_streaming_latency: f64,
    ) {
        let p_type = if p_note_on {
            p_track.get_event_trigger_type_on()
        } else {
            p_track.get_event_trigger_type_off()
        };

        let make = |event_type: &str| {
            Rc::new(RefCell::new(SiONTrackEvent::new(
                event_type.to_string(),
                None,
                p_track.get_buffer_index(),
                p_track.get_note(),
                p_track.get_event_trigger_id(),
                p_sample_rate,
                p_streaming_latency,
            )))
        };

        // Frame event; dispatch later.
        if p_type & 1 != 0 {
            queue.borrow_mut().push_back(make(p_frame_event));
            return;
        }

        // Stream event; dispatch immediately (deferred to the pump seam).
        if p_type & 2 != 0 {
            pending.borrow_mut().push(PendingDispatch::Event(DriverEvent::Track(
                make(p_stream_event),
            )));
        }
    }

    // --- Realtime block pump --------------------------------------------------

    /// C++ `_stream_block(std::vector<double> &r_block)` into the internal
    /// `_chunk_buffer`.
    fn stream_block(&mut self) {
        self.in_streaming_process = true;

        let start_time = ticks_msec();
        self.streaming_time = start_time;

        // Processing (the C++ driver pump sandwich).
        self.chip.borrow().begin_process();
        self.effector_handle().borrow_mut().begin_process();
        self.sequencer.borrow_mut().process();
        // Immediate callbacks fired inside `process()` dispatch here —
        // same pump position as C++ `_dispatch_event` mid-process.
        self.drain_pending();
        {
            let chip = self.chip.borrow();
            chip.effector
                .borrow_mut()
                .end_process(&*chip as &dyn ChipContext);
        }
        self.chip.borrow().end_process();

        // Calculate an average processing time.
        let frame_time = (ticks_msec() - start_time) as i32;
        let idx = self.processing_time_index;
        self.processing_time_index = (idx + 1) % TIME_AVERAGING_COUNT;
        self.total_processing_time -= self.processing_time_data[idx] as i64;
        self.processing_time_data[idx] = frame_time;
        self.total_processing_time += frame_time as i64;
        self.average_processing_time =
            (self.total_processing_time as f64 * self.total_processing_time_ratio) as i64;

        // Write samples. Master and fader volume replace the Godot
        // AudioStreamPlayer gain.
        let gain = self.master_volume * self.fader_volume;
        self.chunk_buffer.clear();
        self.chip.borrow().with_output_buffer(|output_buffer| {
            let mut i = 0;
            while i + 1 < output_buffer.len() {
                self.chunk_buffer.push(output_buffer[i] * gain);
                self.chunk_buffer.push(output_buffer[i + 1] * gain);
                i += 2;
            }
        });

        // Dispatch events.
        if self.stream_event_enabled {
            let stream_buffer: Vec<Vector2> = self
                .chunk_buffer
                .chunks(2)
                .map(|pair| (pair[0], pair.get(1).copied().unwrap_or(0.0)))
                .collect();
            self.dispatch_event(&DriverEvent::Event(SionEvent::new(
                STREAMING.to_string(),
                stream_buffer,
            )));
        }
        if !self.is_finish_sequence_dispatched && self.sequencer.borrow().is_sequence_finished() {
            self.dispatch_event(&DriverEvent::Event(SionEvent::new(
                SEQUENCE_FINISHED.to_string(),
                Vec::new(),
            )));
            self.is_finish_sequence_dispatched = true;
        }

        // Fader step (C++ `_fader->execute()` — the installed callback set
        // `_fader_volume` / fired FADING inside execute; replicated via
        // `execute_compute` + `on_fade_step`).
        let (completed, value) = self.fader.execute_compute();
        if let Some(value) = value {
            self.on_fade_step(value);
        }

        let finished = if completed {
            let stream_buffer: Vec<Vector2> = self
                .chunk_buffer
                .chunks(2)
                .map(|pair| (pair[0], pair.get(1).copied().unwrap_or(0.0)))
                .collect();
            let event_type = if self.fader.is_incrementing() {
                FADE_IN_COMPLETED
            } else {
                FADE_OUT_COMPLETED
            };
            self.dispatch_event(&DriverEvent::Event(SionEvent::new(
                event_type.to_string(),
                stream_buffer,
            )));
            !self.fader.is_incrementing()
        } else {
            self.sequencer.borrow().is_finished()
        };

        if finished && self.auto_stop {
            self.stop();
        }

        self.in_streaming_process = false;
    }

    /// C++ `render_chunk(float *p_buffer, int p_frames)` — realtime sink
    /// filling `p_frames * 2` interleaved stereo f32 samples (master
    /// volume included). Zero-fills when idle/paused; safe from any
    /// audio-callback cadence.
    pub fn render_chunk(&mut self, p_buffer: &mut [f32]) {
        if p_buffer.is_empty() {
            return;
        }

        let mut remaining = p_buffer.len();
        let mut pos = 0;

        while remaining > 0 {
            if self.chunk_position >= self.chunk_buffer.len() {
                self.chunk_buffer.clear();
                self.chunk_position = 0;

                if !self.is_streaming || self.is_paused || self.suspend_streaming {
                    for sample in p_buffer.iter_mut() {
                        *sample = 0.0;
                    }
                    return;
                }

                self.stream_block();
            }

            let available = self.chunk_buffer.len() - self.chunk_position;
            let to_copy = available.min(remaining);
            for i in 0..to_copy {
                p_buffer[pos + i] = self.chunk_buffer[self.chunk_position + i] as f32;
            }
            self.chunk_position += to_copy;
            pos += to_copy;
            remaining -= to_copy;

            // stop() may have been requested from the event callbacks above;
            // do not render new blocks.
            if !self.is_streaming {
                for sample in p_buffer[pos..].iter_mut() {
                    *sample = 0.0;
                }
                break;
            }
        }
    }

    /// Drains the immediate-dispatch queue (note/timer events fired while
    /// the driver is borrowed inside the sequencer pump).
    fn drain_pending(&mut self) {
        let items: Vec<PendingDispatch> =
            self.pending_dispatch.borrow_mut().drain(..).collect();
        for item in items {
            match item {
                PendingDispatch::Event(event) => self.dispatch_event(&event),
                PendingDispatch::Timer => {
                    if let Some(callback) = &self.on_timer_interval {
                        callback();
                    }
                }
            }
        }
    }

    /// C++ `_dispatch_event` — single routing point for all signals.
    fn dispatch_event(&mut self, p_event: &DriverEvent) {
        let signal_name = p_event.get_event_type();
        if signal_name.is_empty() {
            crate::err_print!("");
            return;
        }
        if let Some(callback) = &self.on_event {
            callback(p_event);
        }
    }

    // --- Note / sequence playback ---------------------------------------------

    /// C++ `_find_or_create_track`. NOTE: C++ call sites pass their
    /// arguments SWAPPED (`(p_delay, p_quant, p_track_id)` into
    /// `(p_track_id, p_delay, p_quant)`) — upstream quirk preserved by the
    /// libification (`sion_driver.cpp:749`), so the callers below cast at
    /// the call site exactly like the C++ implicit conversions did.
    fn find_or_create_track(
        &mut self,
        p_track_id: i32,
        p_delay: f64,
        p_quant: f64,
        p_disposable: bool,
    ) -> (Option<TrackRc>, i32) {
        if p_delay < 0.0 {
            crate::err_print!("SiONDriver: Playback delay cannot be less than zero.");
            return (None, 0);
        }

        let internal_track_id = (p_track_id & TRACK_ID_FILTER) | DRIVER_NOTE;
        let mut delay_samples = self
            .sequencer
            .borrow()
            .base().calculate_sample_delay(0, p_delay, p_quant) as i32;

        let mut track = None;

        // Check track ID conflicts.
        if self.note_on_exception_mode != ExceptionMode::Ignore {
            let mut found = self
                .sequencer
                .borrow_mut()
                .find_active_track(internal_track_id, delay_samples);

            if found.is_some() && self.note_on_exception_mode == ExceptionMode::Reject {
                return (None, delay_samples);
            }
            if self.note_on_exception_mode == ExceptionMode::Shift {
                let step = self.sequencer.borrow().base().calculate_sample_length(p_quant) as i32;
                while found.is_some() {
                    delay_samples += step;
                    found = self
                        .sequencer
                        .borrow_mut()
                        .find_active_track(internal_track_id, delay_samples);
                }
            }
            track = found;
        }

        if track.is_some() {
            return (track, delay_samples);
        }

        let created = self
            .sequencer
            .borrow_mut()
            .create_controllable_track(internal_track_id, p_disposable);
        if created.is_none() {
            crate::err_print!(
                "SiONDriver: Failed to allocate a track for playback. Pushing the limits?"
            );
        }
        (created, delay_samples)
    }

    pub fn sample_on(
        &mut self,
        p_sample_number: i32,
        p_length: f64,
        p_delay: f64,
        p_quant: f64,
        p_track_id: i32,
        p_disposable: bool,
    ) -> Option<TrackRc> {
        if !self.is_streaming {
            crate::err_print!(
                "SiONDriver: Driver is not streaming, you must call SiONDriver.stream() first."
            );
            return None;
        }
        if p_length < 0.0 {
            crate::err_print!("SiONDriver: Sample length cannot be less than zero.");
            return None;
        }

        // C++ argument-swap quirk kept (see find_or_create_track).
        let (track, delay_samples) =
            self.find_or_create_track(p_delay as i32, p_quant, p_track_id as f64, p_disposable);
        let track = track?;

        let chip = self.chip.clone();
        track.borrow_mut().set_channel_module_type(
            enums::MODULE_SAMPLE,
            0,
            i32::MIN,
            &mut *chip.borrow_mut(),
        );
        let length = self.convert_event_length(p_length);
        track
            .borrow_mut()
            .key_on(p_sample_number, length, delay_samples);
        Some(track)
    }

    pub fn note_on(
        &mut self,
        p_note: i32,
        p_voice: Option<&SiONVoice>,
        p_length: f64,
        p_delay: f64,
        p_quant: f64,
        p_track_id: i32,
        p_disposable: bool,
    ) -> Option<TrackRc> {
        if !self.is_streaming {
            crate::err_print!(
                "SiONDriver: Driver is not streaming, you must call SiONDriver.stream() first."
            );
            return None;
        }
        if p_length < 0.0 {
            crate::err_print!("SiONDriver: Note length cannot be less than zero.");
            return None;
        }

        // C++ argument-swap quirk kept (see find_or_create_track).
        let (track, delay_samples) =
            self.find_or_create_track(p_delay as i32, p_quant, p_track_id as f64, p_disposable);
        let track = track?;

        if let Some(voice) = p_voice {
            let chip = self.chip.clone();
            voice.update_track_voice(&mut track.borrow_mut(), &mut *chip.borrow_mut());
        }
        let length = self.convert_event_length(p_length);
        track.borrow_mut().key_on(p_note, length, delay_samples);
        Some(track)
    }

    pub fn note_on_with_bend(
        &mut self,
        p_note: i32,
        p_note_to: i32,
        p_bend_length: f64,
        p_voice: Option<&SiONVoice>,
        p_length: f64,
        p_delay: f64,
        p_quant: f64,
        p_track_id: i32,
        p_disposable: bool,
    ) -> Option<TrackRc> {
        if !self.is_streaming {
            crate::err_print!(
                "SiONDriver: Driver is not streaming, you must call SiONDriver.stream() first."
            );
            return None;
        }
        if p_length < 0.0 {
            crate::err_print!("SiONDriver: Note length cannot be less than zero.");
            return None;
        }
        if p_bend_length < 0.0 {
            crate::err_print!("SiONDriver: Pitch bending length cannot be less than zero.");
            return None;
        }

        // C++ argument-swap quirk kept (see find_or_create_track).
        let (track, delay_samples) =
            self.find_or_create_track(p_delay as i32, p_quant, p_track_id as f64, p_disposable);
        let track = track?;

        if let Some(voice) = p_voice {
            let chip = self.chip.clone();
            voice.update_track_voice(&mut track.borrow_mut(), &mut *chip.borrow_mut());
        }
        let length = self.convert_event_length(p_length);
        track.borrow_mut().key_on(p_note, length, delay_samples);
        let bend_length = self.convert_event_length(p_bend_length);
        track.borrow_mut().bend_note(p_note_to, bend_length);
        Some(track)
    }

    pub fn note_off(
        &mut self,
        p_note: i32,
        p_track_id: i32,
        p_delay: f64,
        p_quant: f64,
        p_stop_immediately: bool,
    ) -> Vec<TrackRc> {
        if !self.is_streaming {
            crate::err_print!(
                "SiONDriver: Driver is not streaming, you must call SiONDriver.stream() first."
            );
            return Vec::new();
        }
        if p_delay < 0.0 {
            crate::err_print!("SiONDriver: Note off delay cannot be less than zero.");
            return Vec::new();
        }

        let internal_track_id = (p_track_id & TRACK_ID_FILTER) | DRIVER_NOTE;
        let delay_samples = self
            .sequencer
            .borrow()
            .base().calculate_sample_delay(0, p_delay, p_quant) as i32;

        let tracks: Vec<TrackRc> = self.sequencer.borrow().get_tracks();
        let chip = self.chip.clone();
        let mut result = Vec::new();

        for track in tracks {
            let mut tb = track.borrow_mut();
            if tb.get_internal_track_id() != internal_track_id {
                continue;
            }

            if p_note == -1
                || (p_note == tb.get_note()
                    && tb.get_channel()
                        .expect("SiONDriver: note_off track without channel (C++ null deref)")
                        .borrow()
                        .is_note_on())
            {
                tb.key_off(delay_samples, p_stop_immediately, &mut *chip.borrow_mut());
                result.push(track.clone());
            } else if tb.get_executor().get_waiting_note() == p_note {
                // This track is waiting for this note to start.
                tb.key_on(p_note, 1, delay_samples);
                result.push(track.clone());
            }
        }

        result
    }

    pub fn sequence_on(
        &mut self,
        p_data: &Rc<RefCell<SiONData>>,
        p_voice: Option<&SiONVoice>,
        p_length: f64,
        p_delay: f64,
        p_quant: f64,
        p_track_id: i32,
        p_disposable: bool,
    ) -> Vec<TrackRc> {
        if p_length < 0.0 {
            crate::err_print!("SiONDriver: Sequence length cannot be less than zero.");
            return Vec::new();
        }
        if p_delay < 0.0 {
            crate::err_print!("SiONDriver: Sequence delay cannot be less than zero.");
            return Vec::new();
        }

        let internal_track_id = (p_track_id & TRACK_ID_FILTER) | DRIVER_SEQUENCE;
        let delay_samples = self
            .sequencer
            .borrow()
            .base().calculate_sample_delay(0, p_delay, p_quant) as i32;
        let length_samples = self.sequencer.borrow().base().calculate_sample_length(p_length) as i32;

        let inner = p_data.borrow().data.clone();
        let mut sequence = {
            let mut data = inner.borrow_mut();
            data.base.get_sequence_group().get_head_sequence()
        };

        let mut result = Vec::new();
        let chip = self.chip.clone();

        while let Some(seq) = sequence {
            if seq.borrow().is_active() {
                let track = self
                    .sequencer
                    .borrow_mut()
                    .create_controllable_track(internal_track_id, p_disposable);
                let track = match track {
                    Some(track) => track,
                    None => {
                        crate::err_print!(
                            "SiONDriver: Failed to allocate a track for playback. Pushing the limits?"
                        );
                        return result;
                    }
                };

                track.borrow_mut().sequence_on(
                    Some(inner.clone()),
                    Some(seq.clone()),
                    length_samples,
                    delay_samples,
                );
                if let Some(voice) = p_voice {
                    voice.update_track_voice(&mut track.borrow_mut(), &mut *chip.borrow_mut());
                }
                result.push(track);
            }
            sequence = MMLSequence::get_next_sequence(&seq);
        }

        result
    }

    pub fn sequence_off(
        &mut self,
        p_track_id: i32,
        p_delay: f64,
        p_quant: f64,
        p_stop_with_reset: bool,
    ) -> Vec<TrackRc> {
        if p_delay < 0.0 {
            crate::err_print!("SiONDriver: Sequence off delay cannot be less than zero.");
            return Vec::new();
        }

        let internal_track_id = (p_track_id & TRACK_ID_FILTER) | DRIVER_SEQUENCE;
        let delay_samples = self
            .sequencer
            .borrow()
            .base().calculate_sample_delay(0, p_delay, p_quant) as i32;

        let tracks: Vec<TrackRc> = self.sequencer.borrow().get_tracks();
        let chip = self.chip.clone();
        let mut result = Vec::new();

        for track in tracks {
            if track.borrow().get_internal_track_id() != internal_track_id {
                continue;
            }
            track
                .borrow_mut()
                .sequence_off(delay_samples, p_stop_with_reset, &mut *chip.borrow_mut());
            result.push(track);
        }

        result
    }
}

impl Drop for SiONDriver {


    fn drop(&mut self) {
        if self.is_streaming {
            self.in_streaming_process = false;
            self.stop();
        }
        DRIVER_MUTEX.with(|m| m.set(false));
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::data::SiONData;
    use crate::core::voice::SiONVoice;
    use crate::core::core as sion;
    use crate::sion_enums as enums;
    use crate::sample_data::SampleData;

    const TUNE: &str = "%0,0 v15 o4 c8 d e f g a b >c8 o5 <cdefgab o4 >c2;";

    fn events_capture() -> (Rc<RefCell<Vec<String>>>, Box<dyn Fn(&DriverEvent)>) {
        let seen = Rc::new(RefCell::new(Vec::<String>::new()));
        let sink = seen.clone();
        let cb = Box::new(move |event: &DriverEvent| {
            sink.borrow_mut().push(event.get_event_type());
        });
        (seen, cb)
    }

    #[test]
    fn offline_render_is_non_silent_and_full_length() {
        sion::initialize();
        let mut driver = SiONDriver::new(2048, 2, 44100, 0);
        let buffer = driver.render_mml(TUNE.to_string(), 44100 * 1, 2, true);
        assert_eq!(buffer.len(), 44100);
        let peak = buffer.iter().fold(0.0f64, |acc, v| acc.max(v.abs()));
        assert!(peak > 0.01, "silent render: peak {peak}");
    }

    #[test]
    fn stream_pump_dispatches_start_finish_and_auto_stops() {
        sion::initialize();
        let mut driver = SiONDriver::new(2048, 2, 44100, 0);
        driver.set_auto_stop(true);
        let (seen, cb) = events_capture();
        driver.on_event = Some(cb);

        let data = driver.compile(TUNE.to_string());
        driver.play(&data, true);

        let mut scratch = vec![0.0f32; 2048 * 2];
        let mut peak = 0.0f32;
        for _ in 0..(44100 * 20 / 2048) {
            driver.update();
            driver.render_chunk(&mut scratch);
            peak = peak.max(scratch.iter().fold(0.0f32, |a, v| a.max(v.abs())));
            if !driver.is_streaming() {
                break;
            }
        }

        let seen = seen.borrow();
        assert!(seen.contains(&"stream_started".to_string()), "{:?}", *seen);
        assert!(seen.contains(&"sequence_finished".to_string()));
        assert!(seen.contains(&"stream_stopped".to_string()));
        assert!(!driver.is_streaming());
        assert!(peak > 0.001, "silent stream: peak {peak}");
    }

    #[test]
    fn note_on_voice_and_user_defined_events_round_trip() {
        sion::initialize();
        let mut driver = SiONDriver::new(2048, 2, 44100, 0);
        let (seen, cb) = events_capture();
        driver.on_event = Some(cb);

        driver.stream(true);
        driver.update(); // releases STREAM_STARTED

        let mut voice = SiONVoice::default_voice();
        voice.set_envelope(63, 40, 40, 63, 8, 0);
        driver.note_on(60, Some(&voice), 8.0, 0.0, 0.0, 3, true);

        driver.update();
        driver.notify_user_defined_track(5, 60);

        let mut scratch = vec![0.0f32; 2048 * 2];
        let mut peak = 0.0f32;
        for _ in 0..40 {
            driver.update();
            driver.render_chunk(&mut scratch);
            peak = peak.max(scratch.iter().fold(0.0f32, |a, v| a.max(v.abs())));
        }

        let seen = seen.borrow();
        assert!(seen.contains(&"note_on_stream".to_string()) || seen.contains(&"user_defined_event".to_string()), "{:?}", *seen);
        assert!(peak > 0.001, "note_on produced no audio: peak {peak}");
    }

    #[test]
    fn modulation_envelope_tables_pump_without_double_borrow() {
        sion::initialize();
        let mut driver = SiONDriver::new(2048, 2, 44100, 0);
        driver.stream(true);
        driver.update();

        let mut voice = SiONVoice::default_voice();
        voice.set_envelope(63, 24, 24, 63, 10, 0);
        voice.set_amplitude_modulation(0, 255, 8, 2);
        voice.set_pitch_modulation(0, 127, 8, 2);
        driver.note_on(60, Some(&voice), 12.0, 0.0, 0.0, 3, true);

        let mut scratch = vec![0.0f32; 2048 * 2];
        let mut peak = 0.0f32;
        for _ in 0..40 {
            driver.update();
            driver.render_chunk(&mut scratch);
            peak = peak.max(scratch.iter().fold(0.0f32, |a, v| a.max(v.abs())));
        }
        assert!(peak > 0.001, "mod-envelope note produced no audio: peak {peak}");
    }

    #[test]
    fn voice_mml_round_trip_via_driver_tables() {
        sion::initialize();
        let mut voice = SiONVoice::default_voice();
        voice.set_params(vec![2, 0, 0, 0, 31, 31, 0, 0, 15, 0, 0, 0, 1, 0, 0, 0, 0, 60]);
        let mml = voice.get_mml(4, enums::CHIP_AUTO, false);
        assert!(mml.starts_with("#@4"), "unexpected mml: {mml}");

        let mut loaded = SiONVoice::default_voice();
        let index = loaded.set_by_mml(&mml);
        assert_eq!(index, 4);
        assert_eq!(loaded.voice.borrow().chip_type, enums::CHIP_SIOPM);
        assert!(!loaded.get_params().is_empty());
    }

    #[test]
    fn data_pcm_and_sampler_wave_registration() {
        sion::initialize();
        let driver = SiONDriver::new(2048, 2, 44100, 0);
        let data = Rc::new(RefCell::new(SiONData::new()));
        let sample = Rc::new(RefCell::new(SampleData::from_floats(vec![
            0.0, 0.5, 1.0, 0.5, 0.0, -0.5, -1.0, -0.5,
        ])));

        let pcm = data.borrow().set_pcm_wave(3, &sample, 60.0, 0, 127, 1, 1);
        assert!(pcm.is_some());

        let sampler = data.borrow().set_sampler_wave(100, &sample, false, 0, 1, 1);
        assert_eq!(
            sampler.borrow().get_channel_count(),
            1,
            "sampler data should keep channel count"
        );

        driver.set_sampler_wave(50, Some(&sample), true, 0, 1, 1);
    }
}
