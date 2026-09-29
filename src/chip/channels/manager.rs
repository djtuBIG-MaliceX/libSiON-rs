//! Port of `libSiON-cpp/src/chip/channels/siopm_channel_manager.{h,cpp}`.
//!
//! `SiOPMChannelManager` → module-level functions over a thread_local
//! [`ChannelManager`] per [`ChannelType`] (the C++ statics
//! `_sound_chip` / `_channel_managers`). Each manager keeps the C++ circular
//! list (`terminator` + `_next`/`_prev` links) as a `VecDeque` of
//! `Rc<RefCell<dyn ChannelBaseTrait>>`:
//! - head = `terminator->_next` (free front zone), tail = `terminator->_prev`
//!   (used zone; newly created channels are appended there),
//! - `_create_channel`: reuse free head (FIFO over the surviving order) or
//!   create via the registered factory when the head is in use,
//! - `_delete_channel`: freed node moves to the very front,
//! so allocation ORDER and free-counts match C++ exactly.
//!
//! C++ instantiates the concrete channel classes inside `_create_channel`;
//! those landed in wave-6b1/6b2, so creation goes through a per-kind factory
//! registry ([`register_factory`]). `SiopmSoundChip::new`
//! (`chip/sound_chip.rs`, C++ `siopm_sound_chip.cpp:92`) registers the
//! FM/PCM/Sampler/KS factories and calls `initialize()` / `finalize()`.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use super::channel_base::{BaseChannel, ChannelBaseTrait};
use super::ChipContext;

/// C++ `SiOPMChannelManager::ChannelType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelType {
    Fm = 0,
    Pcm = 1,
    Sampler = 2,
    Ks = 3,
    Max = 4,
}

/// Number of managed channel kinds (`CHANNEL_FM .. CHANNEL_KS`).
const CHANNEL_KINDS: usize = 4;

/// Shared channel handle (C++ `SiOPMChannelBase*` kept alive by the pool and
/// by referencing tracks/voices).
pub type ChannelRc = Rc<RefCell<dyn ChannelBaseTrait>>;

/// C++ the `_create_channel` `switch` over `_channel_type` — a factory
/// returning a fresh concrete channel, or `None` (→ `nullptr`, the
/// `ERR_FAIL_NULL_V` path).
pub type ChannelFactory = Box<dyn Fn(&mut dyn ChipContext) -> Option<ChannelRc>>;

/// C++ `SiOPMChannelManager` (one per kind).
pub struct ChannelManager {
    channel_type: ChannelType,
    /// The C++ `_terminator` (a real `SiOPMChannelBase` in the circular
    /// list; kept separate here, never handed out).
    terminator: ChannelRc,
    /// Circular list contents after the terminator: front = free zone,
    /// back = used zone.
    list: VecDeque<ChannelRc>,
}

struct ManagerState {
    managers: [ChannelManager; CHANNEL_KINDS],
    factories: [Option<ChannelFactory>; CHANNEL_KINDS],
}

thread_local! {
    static STATE: RefCell<Option<ManagerState>> = const { RefCell::new(None) };
}

fn type_index(p_type: ChannelType) -> usize {
    p_type as usize
}

fn with_state<R>(f: impl FnOnce(&ManagerState) -> R) -> R {
    STATE.with(|cell| {
        let slot = cell.borrow();
        let state = slot
            .as_ref()
            .expect("SiOPMChannelManager::initialize not called");
        f(state)
    })
}

fn with_state_mut<R>(f: impl FnOnce(&mut ManagerState) -> R) -> R {
    STATE.with(|cell| {
        let mut slot = cell.borrow_mut();
        let state = slot
            .as_mut()
            .expect("SiOPMChannelManager::initialize not called");
        f(state)
    })
}

/// C++ `SiOPMChannelManager::SiOPMChannelManager(ChannelType)` + the static
/// map insertion in `initialize` (the chip pointer is passed to every call
/// as a `&mut dyn ChipContext` instead; `_sound_chip` is not stored).
pub fn initialize() {
    STATE.with(|cell| {
        let mut slot = cell.borrow_mut();
        let managers: [ChannelManager; CHANNEL_KINDS] = [
            ChannelManager::new(ChannelType::Fm),
            ChannelManager::new(ChannelType::Pcm),
            ChannelManager::new(ChannelType::Sampler),
            ChannelManager::new(ChannelType::Ks),
        ];
        *slot = Some(ManagerState {
            managers,
            factories: [None, None, None, None],
        });
    });
}

/// C++ `SiOPMChannelManager::finalize()` (drops every pooled channel and the
/// factories).
pub fn finalize() {
    STATE.with(|cell| *cell.borrow_mut() = None);
}

/// Registers the concrete channel ctor for a kind (wave-6b seam replacing
/// the C++ `switch (_channel_type) { new SiOPMChannel*(_sound_chip) }`).
pub fn register_factory(p_type: ChannelType, p_factory: ChannelFactory) {
    with_state_mut(|state| {
        state.factories[type_index(p_type)] = Some(p_factory);
    });
}

/// C++ `SiOPMChannelManager::create_channel(ChannelType, SiOPMChannelBase
/// *p_prev, int p_buffer_index)` — returns `None` when no factory is
/// registered or it fails (the C++ `ERR_FAIL_NULL_V(new_channel, nullptr)`
/// path).
pub fn create_channel(
    p_type: ChannelType,
    p_prev: Option<&ChannelRc>,
    p_buffer_index: i32,
    ctx: &mut dyn ChipContext,
) -> Option<ChannelRc> {
    with_state_mut(move |state| {
        let index = type_index(p_type);
        let manager = &mut state.managers[index];
        let new_channel = if manager
            .list
            .front()
            .map(|head| head.borrow().base().is_free)
            .unwrap_or(false)
        {
            // The head channel is free -> The head will be a new channel.
            manager.list.pop_front()
        } else {
            // The head channel is active -> channel overflow.
            let created = match state.factories[index].as_ref() {
                Some(factory) => factory(ctx),
                None => {
                    crate::error::err_print_body(
                        "Parameter \"new_channel\" is null. Returning: null",
                        false,
                    );
                    None
                }
            };
            created.map(|channel| {
                channel.borrow_mut().base_mut().channel_type = manager.channel_type;
                channel
            })
        };

        let new_channel = new_channel?;

        // Set new channel to the tail and activate.
        new_channel.borrow_mut().base_mut().is_free = false;
        manager.list.push_back(new_channel.clone());

        // initialize
        if let Some(prev) = p_prev {
            let prev_ref = prev.borrow();
            new_channel
                .borrow_mut()
                .initialize(Some(&*prev_ref as &dyn ChannelBaseTrait), p_buffer_index, ctx);
        } else {
            new_channel
                .borrow_mut()
                .initialize(None, p_buffer_index, ctx);
        }

        Some(new_channel)
    })
}

/// C++ `SiOPMChannelManager::delete_channel(SiOPMChannelBase*)`.
pub fn delete_channel(p_channel: &ChannelRc) {
    let channel_type = p_channel.borrow().base().channel_type;

    with_state_mut(move |state| {
        state.managers[type_index(channel_type)].delete_channel(p_channel);
    });
}

/// C++ `SiOPMChannelManager::initialize_all_channels()`.
pub fn initialize_all_channels(ctx: &mut dyn ChipContext) {
    with_state_mut(move |state| {
        for manager in state.managers.iter_mut() {
            manager.initialize_all(ctx);
        }
    });
}

/// C++ `SiOPMChannelManager::reset_all_channels()`.
pub fn reset_all_channels() {
    with_state_mut(|state| {
        for manager in state.managers.iter_mut() {
            manager.reset_all();
        }
    });
}

/// In-use (`!_is_free`) channels across every kind, pool order
/// (`Fm` → `Pcm` → `Sampler` → `Ks`, `terminator->_next` →
/// `terminator->_prev` within each kind). The C++ has no such walker — the
/// sequencer drives channels in track order (`simml_sequencer.cpp:440-462`)
/// — the wave-6b2 chip `process` pump uses this as its stand-in until
/// wave-7/8 lands the sequencer.
pub fn used_channels() -> Vec<ChannelRc> {
    with_state(|state| {
        state
            .managers
            .iter()
            .flat_map(|manager| manager.list.iter())
            .filter(|channel| !channel.borrow().base().is_free)
            .cloned()
            .collect()
    })
}

/// C++ `SiOPMChannelManager::get_length()` for one kind: total pooled
/// channels (free + used); the terminator is not counted in `_length`.
pub fn get_channel_count(p_type: ChannelType) -> usize {
    let index = type_index(p_type);
    with_state_mut(move |state| state.managers[index].list.len())
}

/// Free-channel count of one kind (C++: walk the list counting
/// `_is_free`).
pub fn get_free_channel_count(p_type: ChannelType) -> usize {
    let index = type_index(p_type);
    with_state_mut(move |state| {
        state.managers[index]
            .list
            .iter()
            .filter(|channel| channel.borrow().base().is_free)
            .count()
    })
}

/// Pool order for tests / diagnostics: `(is_free, channel_type)` pairs in
/// C++ list order, `terminator->_next` → `terminator->_prev`.
pub fn pool_order(p_type: ChannelType) -> Vec<(bool, ChannelType)> {
    let index = type_index(p_type);
    with_state_mut(move |state| {
        state.managers[index]
            .list
            .iter()
            .map(|channel| {
                let borrow = channel.borrow();
                (borrow.base().is_free, borrow.base().channel_type)
            })
            .collect()
    })
}

impl ChannelManager {
    /// The C++ `_terminator` channel (a real `SiOPMChannelBase` closing the
    /// circular list; never handed out, only created/destroyed with the pool).
    pub fn terminator(&self) -> &ChannelRc {
        &self.terminator
    }

    fn new(p_channel_type: ChannelType) -> Self {
        let terminator = Rc::new(RefCell::new(BaseChannel::new()));
        ChannelManager {
            channel_type: p_channel_type,
            terminator,
            list: VecDeque::new(),
        }
    }

    /// C++ `_delete_channel(SiOPMChannelBase*)`: mark free and move the
    /// node to the front of the list (`_terminator->_next`).
    fn delete_channel(&mut self, p_channel: &ChannelRc) {
        p_channel.borrow_mut().base_mut().is_free = true;

        if let Some(position) = self
            .list
            .iter()
            .position(|channel| Rc::ptr_eq(channel, p_channel))
        {
            self.list.remove(position);
        }
        self.list.push_front(p_channel.clone());
    }

    /// C++ `_initialize_all()`: front → back, mark every channel free and
    /// `initialize(nullptr, 0)`.
    fn initialize_all(&mut self, ctx: &mut dyn ChipContext) {
        let channels: Vec<ChannelRc> = self.list.iter().cloned().collect();
        for channel in channels {
            {
                channel.borrow_mut().base_mut().is_free = true;
            }
            channel.borrow_mut().initialize(None, 0, ctx);
        }
    }

    /// C++ `_reset_all()`: front → back, mark every channel free and
    /// `reset()`.
    fn reset_all(&mut self) {
        let channels: Vec<ChannelRc> = self.list.iter().cloned().collect();
        for channel in channels {
            channel.borrow_mut().base_mut().is_free = true;
            channel.borrow_mut().reset();
        }
    }
}
