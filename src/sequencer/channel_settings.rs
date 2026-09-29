//! `SiMMLChannelSettings` (`libSiON-cpp/src/sequencer/simml_channel_settings.{h,cpp}`).
//!
//! Per-module channel/tone routing table built once by `SiMMLRefTable`.
//! `initialize_tone` / `select_tone` dereference `SiMMLTrack` (wave-7b);
//! see docs/PENDING.md.

use crate::chip::channels::manager;
use crate::chip::channels::manager::ChannelType;
use crate::chip::channels::ChipContext;
use crate::chip::ref_table as chip_ref_table;
use crate::sequencer::base::mml_sequence::SeqRc;
use crate::sequencer::ref_table as mml_ref_table;
use crate::sequencer::track::SiMMLTrack;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SelectToneType {
    SelectToneNone = 0,
    SelectToneNormal = 1,
    SelectToneFm = 2,
}

pub struct SiMMLChannelSettings {
    module_type: i32,
    select_tone_type: SelectToneType,
    channel_type: ChannelType,
    is_suitable_for_fm_voice: bool,
    pub default_operator_count: i32,
    pg_type_list: Vec<i32>,
    pt_type_list: Vec<i32>,
    voice_index_table: Vec<i32>,
    initial_voice_index: i32,
}

impl SiMMLChannelSettings {
    /// C++ `get_module_type` — the ctor's first argument, stored in `_type`.
    pub fn get_module_type(&self) -> i32 {
        self.module_type
    }

    /// C++ `get_select_tone`.
    pub fn get_select_tone(&self) -> SelectToneType {
        self.select_tone_type
    }

    /// C++ `is_select_tone_type`.
    pub fn is_select_tone_type(&self, p_type: SelectToneType) -> bool {
        self.select_tone_type == p_type
    }

    /// C++ `set_select_tone_type`.
    pub fn set_select_tone_type(&mut self, p_type: SelectToneType) {
        self.select_tone_type = p_type;
    }

    /// C++ `get_channel_type`.
    pub fn get_channel_type(&self) -> ChannelType {
        self.channel_type
    }

    /// C++ `set_channel_type`.
    pub fn set_channel_type(&mut self, p_type: ChannelType) {
        self.channel_type = p_type;
    }

    /// C++ `is_suitable_for_fm_voice`.
    pub fn is_suitable_for_fm_voice(&self) -> bool {
        self.is_suitable_for_fm_voice
    }

    /// C++ `set_suitable_for_fm_voice`.
    pub fn set_suitable_for_fm_voice(&mut self, p_suitable: bool) {
        self.is_suitable_for_fm_voice = p_suitable;
    }

    /// C++ `initialize_tone(SiMMLTrack*, int, int)`. The C++ passes
    /// `p_track->get_channel()` to itself as the `initialize` template
    /// (self-copy); the port routes that case through
    /// [`ChannelBaseTrait::initialize_self`]. C++ creates the channel through
    /// the manager (which initializes it) whenever one is missing or of the
    /// wrong type, and only resets the volume offset on the re-init path —
    /// reproduced verbatim.
    pub fn initialize_tone(
        &self,
        track: &mut SiMMLTrack,
        p_channel_num: i32,
        p_buffer_index: usize,
        ctx: &mut dyn ChipContext,
    ) -> i32 {
        let buffer_index = p_buffer_index as i32;

        if track.get_channel().is_none() {
            let channel = manager::create_channel(self.channel_type, None, buffer_index, ctx);
            track.set_channel(channel);
        } else if track.get_channel().map(|c| c.borrow().base().get_channel_type())
            != Some(self.channel_type)
        {
            let old_channel = track.get_channel().cloned().unwrap();
            let channel =
                manager::create_channel(self.channel_type, Some(&old_channel), buffer_index, ctx);
            track.set_channel(channel);
            manager::delete_channel(&old_channel);
        } else {
            let channel = track
                .get_channel()
                .cloned()
                .expect("SiMMLChannelSettings: _channel is null");
            channel.borrow_mut().initialize_self(buffer_index, ctx);
            track.reset_volume_offset();
        }

        // Voice index is the same as channel number, except for PSG, APU,
        // and analog.
        let mut voice_index = self.initial_voice_index;
        let mut channel_num_restricted = p_channel_num;

        if p_channel_num >= 0 && (p_channel_num as usize) < self.voice_index_table.len() {
            voice_index = self.voice_index_table[p_channel_num as usize];
        } else {
            channel_num_restricted = 0;
        }

        track.set_channel_number(if p_channel_num < 0 { -1 } else { p_channel_num });

        let channel = track
            .get_channel()
            .cloned()
            .expect("SiMMLChannelSettings: _channel is null");
        channel.borrow_mut().set_channel_number(channel_num_restricted);
        channel
            .borrow_mut()
            .set_algorithm(self.default_operator_count, false, 0, ctx);

        self.select_tone(track, voice_index, ctx);

        if p_channel_num == -1 {
            -1
        } else {
            voice_index
        }
    }

    /// C++ `select_tone(SiMMLTrack*, int)`. The `SELECT_TONE_NORMAL` clamp
    /// keeps the index in range except when the lists are empty (a C++
    /// out-of-bounds vector read there; the port indexes like the C++). A
    /// `null` voice or empty init sequence both become `None` (C++
    /// `nullptr`). The `Ref` borrow the caller holds on these settings stays
    /// shared, so the re-entry into `set_channel_module_type` (voice update
    /// path) re-borrows the same RefCell immutably — defined.
    pub fn select_tone(
        &self,
        track: &mut SiMMLTrack,
        p_voice_index: i32,
        ctx: &mut dyn ChipContext,
    ) -> Option<SeqRc> {
        if p_voice_index == -1 {
            return None;
        }

        match self.select_tone_type {
            SelectToneType::SelectToneNormal => {
                let mut voice_index = p_voice_index;
                if voice_index < 0 || voice_index as usize >= self.pg_type_list.len() {
                    voice_index = self.initial_voice_index;
                }

                let channel = track
                    .get_channel()
                    .cloned()
                    .expect("SiMMLChannelSettings: _channel is null");
                channel.borrow_mut().set_types(
                    self.pg_type_list[voice_index as usize],
                    self.pt_type_list[voice_index as usize],
                    ctx,
                );
            }
            SelectToneType::SelectToneFm => {
                let mut voice_index = p_voice_index;
                if voice_index < 0 || voice_index as usize >= mml_ref_table::VOICE_MAX {
                    voice_index = 0;
                }

                let voice = mml_ref_table::instance()
                    .expect("SiMMLRefTable not initialized")
                    .borrow()
                    .get_voice(voice_index)?;

                if voice.borrow().should_update_track_parameters() {
                    voice.borrow().update_track_voice(track, ctx);
                    return None;
                }

                let channel = track
                    .get_channel()
                    .cloned()
                    .expect("SiMMLChannelSettings: _channel is null");
                {
                    let params = voice.borrow().channel_params.clone();
                    let params = params.borrow();
                    channel
                        .borrow_mut()
                        .set_channel_params(&params, false, false, ctx);
                }
                track.reset_volume_offset();

                let init_sequence = voice.borrow().channel_params.borrow().get_init_sequence()?;
                if init_sequence.borrow().is_empty() {
                    return None;
                }
                return Some(init_sequence);
            }
            SelectToneType::SelectToneNone => {}
        }

        None
    }

    /// C++ `get_pg_type_list`.
    pub fn get_pg_type_list(&self) -> Vec<i32> {
        self.pg_type_list.clone()
    }

    /// C++ `get_pg_type`.
    pub fn get_pg_type(&self, p_index: usize) -> Option<i32> {
        self.pg_type_list.get(p_index).copied()
    }

    /// C++ `set_pg_type`.
    pub fn set_pg_type(&mut self, p_index: usize, p_type: i32) {
        self.pg_type_list[p_index] = p_type;
    }

    /// C++ `get_pt_type_list`.
    pub fn get_pt_type_list(&self) -> Vec<i32> {
        self.pt_type_list.clone()
    }

    /// C++ `get_pt_type`.
    pub fn get_pt_type(&self, p_index: usize) -> Option<i32> {
        self.pt_type_list.get(p_index).copied()
    }

    /// C++ `set_pt_type`.
    pub fn set_pt_type(&mut self, p_index: usize, p_type: i32) {
        self.pt_type_list[p_index] = p_type;
    }

    /// C++ `get_voice_index_table`.
    pub fn get_voice_index_table(&self) -> Vec<i32> {
        self.voice_index_table.clone()
    }

    /// C++ `get_voice_index`.
    pub fn get_voice_index(&self, p_index: usize) -> Option<i32> {
        self.voice_index_table.get(p_index).copied()
    }

    /// C++ `set_voice_index`.
    pub fn set_voice_index(&mut self, p_index: usize, p_value: i32) {
        self.voice_index_table[p_index] = p_value;
    }

    /// C++ `get_initial_voice_index`.
    pub fn get_initial_voice_index(&self) -> i32 {
        self.initial_voice_index
    }

    /// C++ `set_initial_voice_index`.
    pub fn set_initial_voice_index(&mut self, p_value: i32) {
        self.initial_voice_index = p_value;
    }

    /// C++ `SiMMLChannelSettings(p_module_type, p_pg_type, p_length, p_step,
    /// p_channel_count)`. `p_module_type` is stored as `_type`
    /// ([`Self::get_module_type`]).
    pub fn new(
        p_module_type: i32,
        p_pg_type: i32,
        p_length: usize,
        p_step: i32,
        p_channel_count: usize,
    ) -> Self {
        let table = chip_ref_table::instance();
        let table = table.borrow();

        let mut pg_type_list = vec![0i32; p_length];
        let mut pt_type_list = vec![0i32; p_length];
        let mut idx = p_pg_type;
        for i in 0..p_length {
            pg_type_list[i] = idx;
            pt_type_list[i] = table
                .get_wave_table(idx)
                .map(|w| w.borrow().get_default_pitch_table_type())
                .unwrap_or(crate::sion_enums::PITCH_TABLE_OPM);
            idx += p_step;
        }

        drop(table);

        let voice_index_table = (0..p_channel_count as i32).collect();

        SiMMLChannelSettings {
            module_type: p_module_type,
            select_tone_type: SelectToneType::SelectToneNormal,
            channel_type: ChannelType::Fm,
            is_suitable_for_fm_voice: true,
            default_operator_count: 1,
            pg_type_list,
            pt_type_list,
            voice_index_table,
            initial_voice_index: 0,
        }
    }
}
