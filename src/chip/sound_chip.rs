//! Port of `libSiON-cpp/src/chip/siopm_sound_chip.{h,cpp}`
//! (`SiOPMSoundChip`).
//!
//! The chip owns the interleaved-stereo output [`SiopmStream`] (C++
//! `output_stream` + `get_output_buffer_ptr`), the `STREAM_SEND_SIZE`
//! stream-slot table (slot 0 aliases the output stream), the
//! `PIPE_SIZE`-ring pipe set + 1-slot zero buffer (`SinglyLinkedList<int>`
//! rings → [`Pipe`]), and the [`SiEffector`] the `process` pump drives.
//! C++ `SiOPMChannelManager::initialize(this)` / `finalize()` become the
//! thread_local `manager::initialize()/finalize()` calls (the manager never
//! stores the chip — every channel call passes `&mut dyn ChipContext`), and
//! the concrete-channel `new SiOPMChannel*` switch becomes the factory
//! registrations in [`SiopmSoundChip::new`] (`siopm_channel_manager.cpp:75-90`).
//!
//! The C++ has no `SiOPMSoundChip::process`: the driver pumps
//! `sound_chip->begin_process(); effector->begin_process();
//! sequencer->process(); effector->end_process(); sound_chip->end_process()`
//! (`sion_driver.cpp:379-383` and `:431-435`), and the sequencer drives
//! channels via tracks. [`SiopmSoundChip::process`] replicates that exact
//! statement order with the sequencer stage stood in by
//! "reset every used channel's buffer status, then `buffer(p_length)`"
//! until wave-7/8 lands the sequencer.

use std::cell::RefCell;
use std::rc::Rc;

use crate::chip::params::operator_params::OperatorParams;
use crate::effector::effector::SiEffector;

use super::channels::channel_fm::ChannelFm;
use super::channels::channel_ks::new_channel_ks;
use super::channels::channel_pcm::ChannelPcm;
use super::channels::channel_sampler::ChannelSampler;
use super::channels::manager::{self, ChannelRc, ChannelType};
use super::channels::{ChipContext, OutputStream, Pipe, PipeRc};
use super::stream::SiopmStream;

/// C++ `static const int STREAM_SEND_SIZE = 8` (`siopm_sound_chip.h:43`);
/// `channel_params::STREAM_SEND_SIZE` re-exports this.
pub const STREAM_SEND_SIZE: usize = 8;
/// C++ `static const int PIPE_SIZE = 5` (`siopm_sound_chip.h:44`).
pub const PIPE_SIZE: usize = 5;

/// C++ `SiOPMSoundChip`.
pub struct SiopmSoundChip {
    /// C++ `Ref<SiOPMOperatorParams> init_operator_params` — the C++ ctor
    /// `instantiate()` runs `SiOPMOperatorParams::initialize()` (defaults:
    /// sine / PITCH_TABLE_OPM / attack_rate 63).
    init_operator_params: Rc<RefCell<OperatorParams>>,
    /// C++ `SinglyLinkedList<int> *zero_buffer` (`new ...(1, 0, true)`).
    zero_buffer: PipeRc,
    /// C++ `SiOPMStream *output_stream` — also stream slot 0 and the
    /// effector master stream (one `Rc`, like the C++ pointer aliasing).
    output_stream: Rc<RefCell<dyn OutputStream>>,
    /// C++ `std::vector<SiOPMStream*> stream_slot` (`STREAM_SEND_SIZE`).
    stream_slot: Vec<Option<Rc<RefCell<dyn OutputStream>>>>,
    pcm_volume: f64,
    sampler_volume: f64,
    buffer_length: i32,
    bitrate: i32,
    /// C++ `std::vector<SinglyLinkedList<int>*> _pipe_buffers`
    /// (`PIPE_SIZE`, null until `initialize` allocates them).
    pipe_buffers: Vec<Option<PipeRc>>,
    /// C++ counterpart lives on `SiONDriver`; wave-6b2 embeds it so the
    /// `process` pump can run. `Rc` so slots can be manipulated while the
    /// pump passes `self` as the `ChipContext` (no overlapping borrows).
    pub effector: Rc<RefCell<SiEffector>>,
}

impl SiopmSoundChip {
    /// C++ `static const int STREAM_SEND_SIZE = 8` (module-level alias
    /// `STREAM_SEND_SIZE` — `channel_params::STREAM_SEND_SIZE` re-exports it).
    pub const STREAM_SEND_SIZE: usize = crate::chip::sound_chip::STREAM_SEND_SIZE;
    /// C++ `static const int PIPE_SIZE = 5`.
    pub const PIPE_SIZE: usize = crate::chip::sound_chip::PIPE_SIZE;

    /// C++ `SiOPMSoundChip()`. Registers the four concrete-channel factories
    /// (the wave-6b1 seam standing in for the `_create_channel` switch;
    /// C++ `siopm_sound_chip.cpp:92` `SiOPMChannelManager::initialize(this)`).
    pub fn new() -> Self {
        let output_stream: Rc<RefCell<dyn OutputStream>> =
            Rc::new(RefCell::new(SiopmStream::new()));

        let chip = SiopmSoundChip {
            init_operator_params: Rc::new(RefCell::new(OperatorParams::new())),
            zero_buffer: Rc::new(RefCell::new(Pipe::new(1, 0))),
            output_stream: output_stream.clone(),
            stream_slot: vec![None; Self::STREAM_SEND_SIZE],
            pcm_volume: 4.0,
            sampler_volume: 2.0,
            buffer_length: 0,
            bitrate: 0,
            pipe_buffers: vec![None; Self::PIPE_SIZE],
            effector: Rc::new(RefCell::new(SiEffector::new(output_stream))),
        };

        // C++ `SiOPMChannelManager::initialize(this)` (siopm_sound_chip.cpp:92):
        // thread_local pools per kind + concrete-channel factories.
        manager::initialize();
        manager::register_factory(ChannelType::Fm, Box::new(|ctx| {
            let channel: ChannelRc = Rc::new(RefCell::new(ChannelFm::new(ctx)));
            Some(channel)
        }));
        manager::register_factory(ChannelType::Pcm, Box::new(|ctx| {
            let channel: ChannelRc = Rc::new(RefCell::new(ChannelPcm::new(ctx)));
            Some(channel)
        }));
        manager::register_factory(ChannelType::Sampler, Box::new(|_ctx| {
            let channel: ChannelRc = Rc::new(RefCell::new(ChannelSampler::new()));
            Some(channel)
        }));
        manager::register_factory(ChannelType::Ks, Box::new(|ctx| {
            Some(new_channel_ks(ctx))
        }));

        chip
    }

    /// C++ `~SiOPMSoundChip()` — `SiOPMChannelManager::finalize()`
    /// (`siopm_sound_chip.cpp:105`); the owned `Rc`/`Box` buffers drop
    /// naturally.
    fn finalize(&mut self) {
        manager::finalize();
    }

    /// C++ `get_channel_count()` (`siopm_sound_chip.cpp:20-22`, forwarded
    /// to the output stream).
    pub fn get_channel_count(&self) -> i32 {
        self.output_stream.borrow().get_channel_count()
    }

    /// C++ `get_buffer_length()`.
    pub fn get_buffer_length(&self) -> i32 {
        self.buffer_length
    }

    /// C++ `get_bitrate()`.
    pub fn get_bitrate(&self) -> i32 {
        self.bitrate
    }

    /// C++ `get_output_buffer_ptr()` (`siopm_sound_chip.cpp:16-18`): hands
    /// the interleaved output buffer to the caller as a borrow, mirroring
    /// the raw-pointer scope of the C++ accessor.
    pub fn with_output_buffer<R>(&self, f: impl FnOnce(&[f64]) -> R) -> R {
        f(self.output_stream.borrow().get_buffer())
    }

    /// C++ `begin_process()`: clear the output stream.
    pub fn begin_process(&self) {
        self.output_stream.borrow_mut().clear();
    }

    /// C++ `end_process()`: limit to -1..1, then quantize when a bitrate
    /// was requested.
    pub fn end_process(&self) {
        let mut stream = self.output_stream.borrow_mut();
        stream.limit();
        if self.bitrate != 0 {
            stream.quantize(self.bitrate);
        }
    }

    /// C++ `initialize(int p_channel_count, int p_bitrate,
    /// int p_buffer_length)` (`siopm_sound_chip.cpp:49-76`). `p_channel_count`
    /// is unused by the C++ body (the output stream stays stereo); kept for
    /// the wave-8 driver call shape.
    pub fn initialize(&mut self, p_channel_count: i32, p_bitrate: i32, p_buffer_length: i32) {
        let _ = p_channel_count;
        self.bitrate = p_bitrate;

        // Reset stream slot.
        for slot in self.stream_slot.iter_mut() {
            *slot = None;
        }
        self.stream_slot[0] = Some(self.output_stream.clone());

        // Reallocate buffer.
        if self.buffer_length != p_buffer_length {
            self.buffer_length = p_buffer_length;
            self.output_stream
                .borrow_mut()
                .resize((self.buffer_length << 1) as usize);

            for i in 0..Self::PIPE_SIZE {
                self.pipe_buffers[i] = Some(Rc::new(RefCell::new(Pipe::new(
                    self.buffer_length as usize,
                    0,
                ))));
            }
        }

        self.pcm_volume = 4.0;
        self.sampler_volume = 2.0;

        manager::initialize_all_channels(self);
    }

    /// C++ `reset()` (`siopm_sound_chip.cpp:78-80`).
    pub fn reset(&mut self) {
        manager::reset_all_channels();
    }

    /// Wave-6b2 render pump — the C++ driver block pump
    /// (`sion_driver.cpp:431-435`):
    /// `sound_chip->begin_process(); effector->begin_process();
    /// sequencer->process(); effector->end_process();
    /// sound_chip->end_process()`. The sequencer stage (wave-7/8) is
    /// stood in by the used-channel loop: `reset_channel_buffer_status()`
    /// (`simml_sequencer.cpp:441`) + `buffer(p_length)` (the track
    /// executor call) per used channel, manager pool order
    /// (`Fm → Pcm → Sampler → Ks`, pool front → back).
    pub fn process(&mut self, p_length: i32) {
        self.begin_process();

        let effector = self.effector.clone();
        effector.borrow_mut().begin_process();

        let channels = manager::used_channels();
        for channel in channels {
            channel.borrow_mut().reset_channel_buffer_status();
            channel.borrow_mut().buffer(p_length, self);
        }

        effector.borrow_mut().end_process(&*self as &dyn ChipContext);

        self.end_process();
    }
}

impl Default for SiopmSoundChip {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for SiopmSoundChip {
    fn drop(&mut self) {
        self.finalize();
    }
}

impl ChipContext for SiopmSoundChip {
    fn get_zero_buffer(&self) -> PipeRc {
        self.zero_buffer.clone()
    }

    /// C++ `get_pipe(int p_pipe_num, int p_index)` (`siopm_sound_chip.cpp:24-33`):
    /// both `ERR_FAIL_INDEX_V` become `None`; the ring cursor is rewound to
    /// `p_index` (`front(); advance(p_index)`) and the SAME shared list is
    /// returned. A not-yet-initialized null pipe is UB in C++ (a crash);
    /// the Rust port panics instead.
    fn get_pipe(&mut self, p_pipe_num: i32, p_index: i32) -> Option<PipeRc> {
        crate::err_fail_index_v!(
            p_pipe_num,
            "p_pipe_num",
            self.pipe_buffers.len() as i32,
            "_pipe_buffers.size()",
            None
        );

        let pipe = self.pipe_buffers[p_pipe_num as usize]
            .clone()
            .expect("SiOPMSoundChip::get_pipe: pipe buffer is null (call initialize first)");
        {
            let mut borrow = pipe.borrow_mut();
            borrow.front();
            borrow.advance(p_index);
        }
        Some(pipe)
    }

    fn get_buffer_length(&self) -> i32 {
        self.buffer_length
    }

    fn get_init_operator_params(&self) -> Rc<RefCell<OperatorParams>> {
        self.init_operator_params.clone()
    }

    /// C++ `get_stream_slot(int p_slot)` — no bounds check in C++ (callers
    /// stay in `0..STREAM_SEND_SIZE`); OOB panics here.
    fn get_stream_slot(&self, p_slot: usize) -> Option<Rc<RefCell<dyn OutputStream>>> {
        self.stream_slot[p_slot].clone()
    }

    fn set_stream_slot(&mut self, p_slot: usize, p_stream: Option<Rc<RefCell<dyn OutputStream>>>) {
        self.stream_slot[p_slot] = p_stream;
    }

    fn get_output_stream(&self) -> Option<Rc<RefCell<dyn OutputStream>>> {
        Some(self.output_stream.clone())
    }

    fn get_pcm_volume(&self) -> f64 {
        self.pcm_volume
    }

    fn get_sampler_volume(&self) -> f64 {
        self.sampler_volume
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chip::channels::operator::Operator;
    use crate::chip::ref_table;
    use crate::chip::ref_table::SiopmRefTable;
    use crate::chip::stream::SiopmStream;
    use crate::effector::effector::get_effect_instance;

    fn output_buffer_copy(chip: &SiopmSoundChip) -> Vec<f64> {
        chip.with_output_buffer(|buf| buf.to_vec())
    }

    #[test]
    fn initialize_owns_stereo_output_buffer_and_empty_pump_is_silent() {
        ref_table::initialize();
        let mut chip = SiopmSoundChip::new();
        chip.initialize(2, 0, 64);

        // C++ defaults: output stream stays stereo (p_channel_count is
        // unused), buffer = get_buffer_length() << 1 interleaved doubles.
        assert_eq!(chip.get_channel_count(), 2);
        assert_eq!(chip.get_buffer_length(), 64);
        assert_eq!(chip.get_bitrate(), 0);

        // Slot 0 aliases the output stream (siopm_sound_chip.cpp:56).
        assert!(chip.get_stream_slot(0).is_some());
        assert!(chip.get_stream_slot(3).is_none());
        assert_eq!(chip.get_pcm_volume(), 4.0);
        assert_eq!(chip.get_sampler_volume(), 2.0);

        // No channels in use: the pump keeps the guarantee that C++ gives
        // an empty chip — an all-zero output block.
        for _ in 0..3 {
            chip.process(64);
        }
        let buffer = output_buffer_copy(&chip);
        assert_eq!(buffer.len(), 128);
        assert!(buffer.iter().all(|value| *value == 0.0));
    }

    #[test]
    fn pump_buffers_used_fm_channel_and_first_samples_are_exact() {
        ref_table::initialize();
        let mut chip = SiopmSoundChip::new();
        chip.initialize(2, 0, 64);

        let channel = manager::create_channel(ChannelType::Fm, None, 0, &mut chip).unwrap();
        channel.borrow_mut().note_on();
        chip.process(64);

        // Mirror of the freshly-initialized channel op0 driven through the
        // identical public-API chain (same derivation as the wave-6b1 FM
        // test): init params sine / PITCH_TABLE_OPM / ar=63 / fine 128,
        // `_process_operator1_lfo_off` arithmetic.
        let mut op = Operator::new();
        op.initialize(&mut chip);
        op.initialize(&mut chip);
        op.note_on();

        // channel_base default: volumes[0] = 0.5, pan = 64; stream write
        // math: volume = p_volume * i2n, left = pan_table[128-pan] * volume.
        let (i2n, pan_left, pan_right) = {
            let table = ref_table::instance();
            let table = table.borrow();
            (table.i2n, table.pan_table[128 - 64], table.pan_table[64])
        };
        let volume_left = pan_left * (0.5 * i2n);
        let volume_right = pan_right * (0.5 * i2n);

        let mut expected = [0f64; 2];
        for expected in expected.iter_mut() {
            op.tick_eg(SiopmRefTable::ENV_TIMER_INITIAL);
            op.tick_pulse_generator(0);
            let t = (op.get_phase() & SiopmRefTable::PHASE_FILTER) >> op.get_wave_fixed_bits();
            let log_idx = (op.get_wave_value(t) + op.get_eg_output()) as usize;
            let value = ref_table::instance().borrow().log_table[log_idx] as f64;
            *expected = value * volume_left;
            assert_eq!(value * volume_right, *expected, "pan 64: L == R");
        }

        let buffer = output_buffer_copy(&chip);
        assert_eq!(buffer.len(), 128);
        assert_eq!(buffer[0], expected[0]);
        assert_eq!(buffer[1], expected[0]);
        assert_eq!(buffer[2], expected[1]);
        assert_eq!(buffer[3], expected[1]);
        assert_ne!(buffer[0], 0.0);

        // A silent block after note release / idling must not grow garbage:
        // pump again past the ring start, output stays finite.
        chip.process(64);
        let buffer = output_buffer_copy(&chip);
        assert!(buffer.iter().all(|value| value.is_finite()));
    }

    #[test]
    fn write_from_vector_matches_cpp_pan_and_level_math() {
        ref_table::initialize();
        let (pan_table, _) = {
            let table = ref_table::instance();
            let table = table.borrow();
            (table.pan_table, ())
        };

        // --- stereo buffer, stereo data: siopm_stream.cpp:114-126 ---
        let mut stream = SiopmStream::new();
        stream.resize(12);
        let data = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let pan: i32 = 96; let volume = 0.25;
        let volume_left = pan_table[(128 - pan) as usize] * volume;
        let volume_right = pan_table[pan as usize] * volume;
        stream.write_from_vector(&data, 0, 1, 2, volume, pan, 2);
        let mut expect = [0.0; 12];
        // Frames 1..3 take data frames 0..2 (j = start_data<<1 walk).
        expect[2] = data[0] * volume_left;
        expect[3] = data[1] * volume_right;
        expect[4] = data[2] * volume_left;
        expect[5] = data[3] * volume_right;
        assert_eq!(stream.get_buffer(), &expect[..]);

        // --- stereo buffer, mono data: siopm_stream.cpp:127-138 (0.707) ---
        let mut stream = SiopmStream::new();
        stream.resize(6);
        let data = [1.0, -2.0, 3.0];
        let pan: i32 = 32; let volume = 0.5;
        let volume_left = pan_table[(128 - pan) as usize] * volume * 0.707;
        let volume_right = pan_table[pan as usize] * volume * 0.707;
        stream.write_from_vector(&data, 0, 0, 3, volume, pan, 1);
        let expect = [
            data[0] * volume_left,
            data[0] * volume_right,
            data[1] * volume_left,
            data[1] * volume_right,
            data[2] * volume_left,
            data[2] * volume_right,
        ];
        assert_eq!(stream.get_buffer(), &expect[..]);

        // Accumulation across calls (+= semantics) + mid-buffer start.
        let mut before = stream.get_buffer().to_vec();
        stream.write_from_vector(&data, 1, 1, 1, volume, pan, 1);
        let after = stream.get_buffer().to_vec();
        assert_eq!(after[0], before[0]);
        assert_eq!(after[2], before[2] + data[1] * volume_left);
        assert_eq!(after[3], before[3] + data[1] * volume_right);
        before[2] += data[1] * volume_left;
        assert_eq!(after[2], before[2]);

        // --- mono buffer, stereo data: siopm_stream.cpp:140-150 (volume*0.5) ---
        let mut stream = SiopmStream::new();
        stream.set_channel_count(1);
        stream.resize(4);
        let data = [1.0, 2.0, 3.0, 4.0];
        let volume = 0.25;
        stream.write_from_vector(&data, 0, 0, 2, volume, 7, 2);
        let expect = [
            (data[0] + data[1]) * volume * 0.5,
            (data[0] + data[1]) * volume * 0.5,
            (data[2] + data[3]) * volume * 0.5,
            (data[2] + data[3]) * volume * 0.5,
        ];
        assert_eq!(stream.get_buffer(), &expect[..]);

        // --- mono buffer, mono data: siopm_stream.cpp:151-159 ---
        let mut stream = SiopmStream::new();
        stream.set_channel_count(1);
        stream.resize(4);
        let data = [0.5, -0.25];
        let volume = 0.75;
        stream.write_from_vector(&data, 0, 0, 2, volume, 7, 1);
        let expect = [
            data[0] * volume,
            data[0] * volume,
            data[1] * volume,
            data[1] * volume,
        ];
        assert_eq!(stream.get_buffer(), &expect[..]);

        // limit + quantize (chip end_process math): clamp then int-grid.
        let mut stream = SiopmStream::new();
        stream.resize(3);
        stream.get_buffer_mut()[0] = 2.0;
        stream.get_buffer_mut()[1] = -2.0;
        stream.get_buffer_mut()[2] = 0.0;
        stream.limit();
        assert_eq!(stream.get_buffer(), &[1.0, -1.0, 0.0]);
        stream.quantize(1); // r = 2, ir = 1: (int)(x*2) >> 1
        assert_eq!(stream.get_buffer(), &[1.0, -1.0, 0.0]);
    }

    #[test]
    fn effector_master_chain_processes_chip_output_end_to_end() {
        ref_table::initialize();

        let render = |with_downsampler: bool| -> Vec<f64> {
            let mut chip = SiopmSoundChip::new();
            chip.initialize(2, 0, 64);

            if with_downsampler {
                let ds = get_effect_instance("ds").unwrap();
                // frequency_shift 1 (2-frame hold), bitrate 30, stereo.
                ds.borrow_mut().set_by_mml(&[1.0, 30.0, 2.0]);
                let effector = chip.effector.clone();
                effector.borrow_mut().add_slot_effect(0, &ds, &mut chip);
                effector.borrow_mut().prepare_process(&mut chip);
            }

            let channel = manager::create_channel(ChannelType::Fm, None, 0, &mut chip).unwrap();
            // C6 so the output actually oscillates (pitch 0 would be a DC
            // hold of one repeated sample).
            channel.borrow_mut().set_pitch(60 * 64);
            channel.borrow_mut().note_on();
            chip.process(64);

            let buffer = output_buffer_copy(&chip);
            drop(chip); // Drop -> manager::finalize (C++ dtor :105)
            buffer
        };

        let raw = render(false);
        let eff = render(true);

        assert_ne!(raw, eff, "master chain must act on the output buffer");

        // Downsampler stereo math (downsampler.rs, pinned in wave-5):
        // per 2-frame block, both frames become trunc(sum * 2^30/2) * 2^-30.
        let bc0: f64 = (1i32 << 30) as f64 / 2.0;
        let bc1: f64 = 1.0 / (1i32 << 30) as f64;
        for block in 0..32 {
            let base = block * 4;
            let quant_left = ((raw[base] + raw[base + 2]) * bc0) as i32 as f64 * bc1;
            let quant_right = ((raw[base + 1] + raw[base + 3]) * bc0) as i32 as f64 * bc1;
            assert_eq!(eff[base], quant_left);
            assert_eq!(eff[base + 1], quant_right);
            assert_eq!(eff[base + 2], quant_left);
            assert_eq!(eff[base + 3], quant_right);
        }
    }
}

