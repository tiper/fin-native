//! The seam between the Tizen URI driver and the host's media player: [`HostPlayer`], what the driver may ask
//! of the host and what it may read back, and [`HostedPlayer`], its one production implementation over the
//! hosted contract ([`crate::hosted::Hosted`], `include/plx_host.h` PLAYER).
//!
//! The driver is a state machine (`logic never performs IO`): it starts no worker and reads no global. Every
//! effect it has leaves through this port, and every result comes back through [`HostPlayer::drain`] as an
//! addressed [`PlayerEvent`]. A host test gives the driver a fake port; the contract's own fake host table
//! (`hosted::testing`) is what [`HostedPlayer`]'s tests run over.
use crate::host_abi::{self, PlayerEvent};
use crate::hosted::{Drained, Hosted};
use plx_media::player::adapter::MonotonicMs;
use plx_playback_contract::Failure;

/// A request the host refused: nothing was queued.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The host has no media player, or its video thread is not running (or has been shut down). The library having been told to stop is [`Refusal::Stopping`], not this.
    Unavailable,
    /// The host's request queue is full.
    Busy,
    /// A bad argument (epoch 0, an empty URI).
    Args,
    /// A verb for an epoch the host no longer holds.
    Stale,
    /// The library was told to stop (`plx_host_stop`): its gate to the host is closed and the host was not asked.
    Stopping,
}

impl Refusal {
    /// The contract's own word for it.
    pub fn failure(self) -> Failure {
        match self {
            Refusal::Unavailable => Failure::Unavailable,
            Refusal::Busy => Failure::Busy,
            Refusal::Args => Failure::Refused,
            Refusal::Stale => Failure::Stale,
            Refusal::Stopping => Failure::Unavailable,
        }
    }

    fn of(code: i32) -> Self {
        match code {
            host_abi::PLAYER_E_BUSY => Refusal::Busy,
            host_abi::PLAYER_E_ARGS => Refusal::Args,
            host_abi::PLAYER_E_STALE => Refusal::Stale,
            host_abi::PLAYER_E_STOPPING => Refusal::Stopping,
            _ => Refusal::Unavailable,
        }
    }
}

/// The host player's verbs after `open`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    Play,
    Pause,
    /// Seek to `ms`; `command` is the library's command id, echoed in `PLAYER_SEEK_DONE`.
    Seek { ms: i64, command: i64 },
    Close,
}

/// What the driver asks of the host's player, and the clock its deadlines read.
pub trait HostPlayer {
    /// Whether the host offers a media player at all, or the reason it does not.
    fn offered(&self) -> Result<(), &'static str>;
    /// Ask the host to open a player for `epoch` on `uri` over the window rectangle `area`. Returns at once; the
    /// host's answer is events. The URI carries the Plex token: the port passes it on and never keeps or logs it.
    fn open(&mut self, epoch: u32, uri: &str, area: [i32; 4]) -> Result<(), Refusal>;
    /// Ask the host for `verb` on the player of `epoch`. Returns at once.
    fn verb(&mut self, epoch: u32, verb: Verb) -> Result<(), Refusal>;
    /// Every player event the host has posted since the last call, in order (the host's queue is lossless but for
    /// position and buffering), and what was lost.
    fn drain(&mut self, out: &mut Vec<PlayerEvent>) -> Drained;
    /// Monotonic milliseconds, for the driver's deadlines.
    fn now_ms(&self) -> u64;
}

/// The window the player is shown over: the library's one window is the whole 1920x1080 canvas
/// (`ecore.rs` logs any other screen as unhandled), so the overlay covers all of it.
pub const FULL_WINDOW: [i32; 4] = [0, 0, 1920, 1080];

/// [`HostPlayer`] over the hosted contract: the requests go through the host's table and the library's gate, the
/// events come from the library's player queue.
pub struct HostedPlayer {
    hosted: &'static Hosted,
    clock: MonotonicMs,
}

impl HostedPlayer {
    pub fn new(hosted: &'static Hosted) -> Self {
        Self { hosted, clock: MonotonicMs::start() }
    }
}

impl HostPlayer for HostedPlayer {
    fn offered(&self) -> Result<(), &'static str> {
        self.hosted.player_support().map_err(host_abi::NoPlayer::reason)
    }

    fn open(&mut self, epoch: u32, uri: &str, area: [i32; 4]) -> Result<(), Refusal> {
        self.hosted.player_open(epoch, uri, area).map_err(Refusal::of)
    }

    fn verb(&mut self, epoch: u32, verb: Verb) -> Result<(), Refusal> {
        let (code, a, b) = match verb {
            Verb::Play => (host_abi::VERB_PLAY, 0, 0),
            Verb::Pause => (host_abi::VERB_PAUSE, 0, 0),
            Verb::Seek { ms, command } => (host_abi::VERB_SEEK, ms, command),
            Verb::Close => (host_abi::VERB_CLOSE, 0, 0),
        };
        self.hosted.player_verb(epoch, code, a, b).map_err(Refusal::of)
    }

    fn drain(&mut self, out: &mut Vec<PlayerEvent>) -> Drained {
        self.hosted.drain_player(out)
    }

    fn now_ms(&self) -> u64 {
        self.clock.now_ms()
    }
}
