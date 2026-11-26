//! The Tizen URI driver: the thin executor around the pure [`Machine`](super::machine::Machine), a [`UriDriver`] over
//! the host's native media player.
//!
//! **What it is.** The machine decides everything ([`super::machine`]); this type holds a [`HostPlayer`] and does
//! the two things a machine may not: it reads the clock (`HostPlayer::now_ms`) and it calls the host. Each
//! [`DriverSet`] entry reads its inputs (the time, whether the host offers a player, the events the host posted),
//! steps the machine with them, and performs the [`Fx`]s the step returned in order. A host that refuses a
//! request is reported back to the machine as the answer to that request. The driver starts no worker, reads no
//! global and has no `cfg(test)` fork: its clock and its host are the [`HostPlayer`] it was built over.
use super::machine::{Fx, Machine};
use super::port::HostPlayer;
use plx_base::eventlog::log;
use plx_base::task::MainThread;
use plx_playback_contract::uri::{DisplayEvidence, UriDriver, UriNotReady, UriSource};
use plx_playback_contract::{AttemptId, BackendEvent, BackendKind, Capabilities, CommandId, DispatchResult, DriverSet,
    ExitDisposition, Failure, Operation, SourceKey, TrackKind, TrackSelection};

pub use super::machine::{Timing, CAPS};

/// The driver's own name for its display path (the boot log's `uri: display path=...` line).
pub const DISPLAY_TAG: &str = "ecore-overlay";

pub struct TizenUriDriver<P: HostPlayer> {
    port: P,
    machine: Machine,
    /// The host's events of one poll, kept to reuse its allocation.
    inbox: Vec<crate::host_abi::PlayerEvent>,
}

impl<P: HostPlayer> TizenUriDriver<P> {
    pub fn new(port: P, timing: Timing) -> Self {
        Self { port, machine: Machine::new(timing), inbox: Vec::new() }
    }

    /// How many host events were dropped because their epoch was not the active one.
    #[cfg(test)]
    pub fn stale_dropped(&self) -> usize {
        self.machine.stale_dropped()
    }

    /// Perform the requests of the step that answered `result`; a request the host refuses is the machine's to
    /// interpret, and may change what the command answers or ask for more (a log line).
    fn perform(&mut self, mut result: DispatchResult, mut fx: Vec<Fx>) -> DispatchResult {
        while !fx.is_empty() {
            for effect in std::mem::take(&mut fx) {
                match effect {
                    Fx::Log(line) => log(&line),
                    Fx::Open { epoch, url, area } => {
                        // The URL is lent to the host for this call and dropped with `url`: it carries the Plex token.
                        if let Err(refusal) = self.port.open(epoch, &url, area) {
                            result = self.machine.open_refused(epoch, refusal);
                        }
                    }
                    Fx::Verb { epoch, verb } => {
                        if let Err(refusal) = self.port.verb(epoch, verb) {
                            if let Some(answer) = self.machine.verb_refused(epoch, verb, refusal, &mut fx) {
                                result = answer;
                            }
                        }
                    }
                }
            }
        }
        result
    }
}

impl<P: HostPlayer> DriverSet for TizenUriDriver<P> {
    type Context = MainThread;

    fn capabilities(&self, backend: BackendKind) -> Option<Capabilities> {
        (backend == BackendKind::Uri).then_some(CAPS)
    }

    fn dispatch(&mut self, backend: BackendKind, _: &MainThread, attempt: AttemptId, command: CommandId,
        operation: Operation) -> DispatchResult {
        if backend != BackendKind::Uri {
            return DispatchResult::Refused(Failure::Unavailable);
        }
        let now = self.port.now_ms();
        let mut fx = Vec::new();
        let result = match operation {
            Operation::Start { source, requirements } =>
                self.machine.start(now, self.port.offered().is_ok(), attempt, source, requirements, &mut fx),
            Operation::Play | Operation::Pause | Operation::Seek(_) =>
                self.machine.control(now, attempt, command, operation, &mut fx),
            Operation::SelectTrack { kind: TrackKind::Audio, choice } =>
                self.machine.select_audio(now, attempt, command, choice, &mut fx),
            Operation::SelectTrack { kind: TrackKind::Subtitle, choice } =>
                self.machine.select_subtitle(attempt, command, choice),
            Operation::Stop(reason) => self.machine.stop(now, attempt, reason, &mut fx),
        };
        self.perform(result, fx)
    }

    fn poll(&mut self, _: &MainThread) -> Vec<BackendEvent> {
        let now = self.port.now_ms();
        let mut inbox = std::mem::take(&mut self.inbox);
        inbox.clear();
        let drained = self.port.drain(&mut inbox);
        let mut fx = Vec::new();
        let events = self.machine.poll(now, &inbox, drained.overflowed, &mut fx);
        self.inbox = inbox;
        self.perform(DispatchResult::Queued, fx);
        events.into_iter().map(|event| BackendEvent { backend: BackendKind::Uri, event }).collect()
    }

    fn discard_staged(&mut self, backend: BackendKind, attempt: AttemptId, key: SourceKey) {
        if backend == BackendKind::Uri {
            self.machine.discard_staged(attempt, key);
        }
    }

    /// The player is held from the open until the host confirmed its close, and a quarantine holds it still.
    fn exit_disposition(&self) -> ExitDisposition {
        self.machine.exit_disposition()
    }
}

impl<P: HostPlayer> UriDriver for TizenUriDriver<P> {
    /// Ready when the host offers a player. That is ALL it rests on: the window's alpha and the video hole are not this
    /// driver's, so it says nothing of whether a picture shows (see [`super::machine`], and `docs/tizen.md`).
    fn display(&self) -> DisplayEvidence {
        if self.port.offered().is_ok() { DisplayEvidence::ready(DISPLAY_TAG) } else { DisplayEvidence::unavailable(DISPLAY_TAG) }
    }

    fn stage_source(&mut self, attempt: AttemptId, key: SourceKey, source: UriSource) -> Result<(), Failure> {
        self.machine.stage(self.port.now_ms(), attempt, key, source)
    }

    fn cancel_staged(&mut self, attempt: AttemptId) {
        self.machine.cancel_staged(attempt);
    }

    /// Owned from the Start until the close is confirmed: an ownership fact, not proof that a picture shows.
    fn plane_owned(&self) -> bool {
        self.machine.plane_owned()
    }

    /// An audio stream the active source's track map names (see [`CAPS`]).
    fn maps_track(&self, selection: TrackSelection) -> bool {
        self.machine.maps_track(selection)
    }

    fn ready_to_start(&mut self, _: &MainThread) -> Result<(), UriNotReady> {
        self.port.offered().map_err(|_| UriNotReady::Driver("the host has no media player"))
    }
}

/// The requirements a plain progressive play states (a helper for the tests).
#[cfg(test)]
pub(super) fn plain_requirements() -> plx_playback_contract::Requirements {
    use plx_playback_contract::{Fidelity, SourceKind, TrackChoice};
    plx_playback_contract::Requirements { source: SourceKind::ProgressiveUri, video: Fidelity::Exact, audio: Fidelity::Exact,
        audio_track: TrackChoice::Unspecified, subtitle_track: TrackChoice::Unspecified }
}
