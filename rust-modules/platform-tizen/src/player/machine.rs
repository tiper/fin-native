//! The Tizen URI driver's machine: the pure step behind [`super::driver::TizenUriDriver`].
//!
//! **What it is.** The state machine that turns the contract's addressed commands into the host's player
//! requests and the host's player events into the contract's observations. It is Sans-IO in the sense of
//! `docs/sans-io-guide.md`: it starts no worker, reads no global, reads no clock and calls no host. Every
//! input arrives as an argument (the events the host posted, whether the host offers a player, and `now_ms`,
//! the ONLY clock it sees, as the webOS lifecycle's `Event::Tick { now_ms }` is), and every request leaves as a
//! typed [`Fx`] that the thin executor ([`super::driver`]) performs through its [`super::port::HostPlayer`].
//!
//! **A request the host refuses comes back as an answer.** A command's [`DispatchResult`] is returned at
//! once, so the machine commits the state a request implies and the executor reports a refusal of that request
//! afterwards ([`Machine::open_refused`], [`Machine::verb_refused`]): the machine rolls the commitment back and
//! hands back the refusal the command now answers with. A refused CLOSE has no dispatch to answer, so the
//! machine decides what it means: busy is retried on the next poll, `Stopping` is released at once (the host closes its own player at terminate), anything else waits for the deadline.
//!
//! **Epochs.** The machine mints one `epoch` per open and keeps at most one attempt active. Every request carries
//! it and every event is checked against it: an event whose epoch is not the active one (a completion that lands
//! after a stop, a position of the player a restart replaced) is dropped and counted, never mapped. The hosted
//! library drops the same events one layer earlier (`hosted::player`, an epoch is live from the open until its
//! `PLAYER_CLOSED`); this is the second layer.
//!
//! **Release.** A `Stop` asks the host to close and the attempt is released by the host's `PLAYER_CLOSED` for its
//! epoch. A close the host never confirms is `ReleaseStatus::Quarantined` at the deadline, never a clean release, and the
//! exit disposition stays `PreserveNativeOwnership` until a close is confirmed. A close the host's gate refuses because the
//! library is stopping can never be asked ([`Machine::verb_refused`]): the host closes its own player at terminate, so that
//! release is the host's to make and the machine reports it `Released` at once (the exit disposition `Clean`) rather than
//! quarantining a stop that cannot be confirmed.
//!
//! **What it does not claim.** It never emits `Observation::Presented`: that needs the window's alpha and the video
//! hole (`ecore.rs`, `Shell::opaque_route`), which are not part of this driver, so startup metrics stay Unknown (the
//! contract's stated behaviour for a path that cannot say). Its capabilities are what the host's player is asked to
//! do, not a codec verdict ([`CAPS`]); which streams the platform player decodes is the qualifier's concern
//! ([`super::qualifier`]).
use super::port::{Refusal, Verb, FULL_WINDOW};
use crate::host_abi::{language_of_word, PlayerEvent, PlayerKind, AUDIO_LANGUAGE_REPORTS, PLAYER_FAIL_BASE};
use plx_media::route::uri_preflight::lang_matches;
use plx_playback_contract::uri::{native_index, NativeTrackCheck, NativeTrackIndex, UriSource};
use plx_playback_contract::{AttemptId, Capabilities, CommandId, ContentTime, DispatchResult, DriverEvent,
    EvidenceSource, ExitDisposition, Failure, Observation, Observed, Operation, Playback, ReleaseStatus, Requirements,
    SourceKey, SourceKind, StopReason, TrackChoice, TrackKind, TrackSelection};
use std::collections::VecDeque;

/// What the driver can call: a progressive source, pause and seek. No HLS, no audio disable, no packet feed, no
/// subtitle selection, and no variable speed (that has no flag: the planner refuses any rate but normal for every
/// URI driver). The flags say what is CALLABLE, never a codec veto.
///
/// `audio_track` is `true`: the contract makes a play whose PMS-selected audio stream has an id a `Selected` track
/// requirement, and the start plan settles it with one `SelectTrack` once the player is prepared. The machine asks the
/// host for it ([`Verb::AudioTrack`], `player_select_track` on the host's video thread) at a native index it chooses by
/// evidence, never by position alone ([`choose_native_audio`]), when the Part has more than one audio stream (a sole stream
/// is settled by construction with no host call, as before the verb existed): the source's track map gives the stream's position among
/// the Part's audio streams in PMS's order, and its [`NativeTrackCheck`] the stream's language, which the native track
/// must report too (the host posts each native track's language before PREPARED). It settles the request only when the
/// host's `PLAYER_TRACK` echoes the command with that index: the player's own answer, not a settle by construction. A
/// refusal (`b = -1`), an unanswered request and a refused verb are the command's typed refusal; a stream the map does not
/// name, or whose native track cannot be told by its language, is refused with `Unsupported`, logged, and asks nothing.
///
/// UNMEASURED on a retail set: the native order, and whether the call succeeds in READY for an HTTP progressive source.
/// Only a retail tester's log says either (`docs/tizen.md`, "The audio track is selected").
///
/// `subtitle_off` is `true` by construction too: the host player is given no subtitle (no `player_set_subtitle_path`,
/// no text-track selection, no subtitle callback), and `docs/tizen.md` ("Subtitles off") records the emulator run in
/// which a file with an embedded default text track played with no subtitle on the picture. A play with subtitle
/// streams and none selected is therefore one this driver carries, and its `SelectTrack(Subtitle, Off)` is settled
/// without a host call ([`Machine::select_subtitle`]).
pub const CAPS: Capabilities = Capabilities { progressive_uri: true, hls_uri: false, packet_feed: false, pause: true,
    seek: true, audio_track: true, subtitle_track: false, audio_off: false, subtitle_off: true };

/// The deadlines the machine enforces, in milliseconds on the executor's clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    /// From the open to the player's `PREPARED`.
    pub startup_timeout_ms: u64,
    /// From a close request to the host's `PLAYER_CLOSED`.
    pub release_allowance_ms: u64,
    /// From a play, pause or seek to its confirmation.
    pub control_timeout_ms: u64,
    /// How long after a `PLAYER_STALL` (a position read hung) a PLAYING player may stay silent before it is failed.
    /// The grace does not run while the player is reported paused.
    pub stall_grace_ms: u64,
    /// How long the position the host reports may stay the SAME while the player is reported playing before it is
    /// treated as a stall: the picture is frozen although the position reads keep answering (an A/V clip that stalls
    /// on the emulator, an underrun on a set), so no `PLAYER_STALL` is ever raised. Surfaced as a rebuffer, and then
    /// failed through the same `stall_grace_ms` as a hung read.
    pub frozen_grace_ms: u64,
    /// How long a staged source waits for its Start.
    pub source_ttl_ms: u64,
}

impl Timing {
    /// The shipped values. The host's own seek expiry is 10 s and its stall report 3 s (`plx_host.h`), so these sit above.
    pub const PRODUCT: Timing = Timing { startup_timeout_ms: 20_000, release_allowance_ms: 10_000,
        control_timeout_ms: 15_000, stall_grace_ms: 7_000, frozen_grace_ms: 3_000, source_ttl_ms: 30_000 };
}

/// A request the executor performs for the machine, in order.
pub enum Fx {
    /// Ask the host to open a player for `epoch` on `url` over `area`. The URL carries the Plex token: it is lent to
    /// the host for the call and dropped with the effect, and [`Fx`]'s `Debug` never prints it.
    Open { epoch: u32, url: String, area: [i32; 4] },
    /// Ask the host for `verb` on the player of `epoch`.
    Verb { epoch: u32, verb: Verb },
    /// A line for the event log.
    Log(String),
}

impl std::fmt::Debug for Fx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Fx::Open { epoch, area, .. } => write!(f, "Open {{ epoch: {epoch}, area: {area:?} }}"),
            Fx::Verb { epoch, verb } => write!(f, "Verb {{ epoch: {epoch}, verb: {verb:?} }}"),
            Fx::Log(line) => write!(f, "Log({line:?})"),
        }
    }
}

struct Staged {
    attempt: AttemptId,
    key: SourceKey,
    source: UriSource,
    deadline_ms: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Opened; the host has not reported `PREPARED`.
    Opening,
    Open,
    /// A close was requested at (and by) `deadline_ms`; `PLAYER_CLOSED` is awaited. `retry` is set when the host
    /// refused the request as busy (it queued nothing, so no `PLAYER_CLOSED` is coming): the next step asks again.
    Closing { deadline_ms: u64, retry: bool },
    /// The close was not confirmed in time. Only a `PLAYER_CLOSED` ends it now.
    Quarantined,
}

/// A play, pause or seek the host was asked for and has not confirmed.
struct Pending {
    command: CommandId,
    operation: Operation,
    deadline_ms: u64,
    /// The native index an audio request asked for, which its `PLAYER_TRACK` must echo; `None` for every other control.
    native: Option<i64>,
}

/// ISO 639-2's codes that name no language: a track tagged with one cannot be told apart by its language.
const NO_LANGUAGE: [&str; 4] = ["und", "mul", "mis", "zxx"];

fn names_language(code: &str) -> bool {
    let code = code.trim();
    !code.is_empty() && !NO_LANGUAGE.iter().any(|none| code.eq_ignore_ascii_case(none))
}

/// **The native audio index of a stream with siblings, or why there is none.** `ordinal` is the stream's position among
/// the Part's audio streams in PMS's order (the track map), `language` the stream's language as PMS states it, `native`
/// each native track's language in the player's order (`None` = the player gave none). The stream is matched by
/// language ([`lang_matches`], which folds ISO 639-1, 639-2/B and 639-2/T): the ordinal when its native track has the
/// stream's language, else the ONE native track that does. Several such tracks with the ordinal not among them, none, or
/// no language known on the server's side is a refusal.
///
/// What this cannot see: an order swap between siblings of ONE language (an English main track and an English
/// commentary). When several native tracks share the stream's language and the ordinal is among them, the ordinal is
/// trusted, so the player may play the sibling. Language is the only per-track identity the host reads with a verified
/// call (`player_get_track_language_code`); the player API's codec and channel reads are of the CURRENT stream, not of
/// an index, so they cannot tell siblings apart before a selection (`docs/tizen.md`, "The audio track is selected").
pub(super) fn choose_native_audio(ordinal: i64, language: &str, native: &[Option<String>]) -> Result<i64, String> {
    if !names_language(language) {
        return Err(format!("PMS states no language for the stream ({language:?}), so its native track cannot be told"));
    }
    let matching: Vec<i64> = native.iter().enumerate()
        .filter(|(_, code)| code.as_deref().is_some_and(|code| names_language(code) && lang_matches(language, code)))
        .filter_map(|(index, _)| i64::try_from(index).ok())
        .collect();
    match matching.as_slice() {
        _ if matching.contains(&ordinal) => Ok(ordinal),
        [only] => Ok(*only),
        [] => Err(format!("no native audio track reports {language:?} (the player reported {native:?})")),
        _ => Err(format!("native audio tracks {matching:?} all report {language:?} and the ordinal {ordinal} is not one \
            of them (the player reported {native:?})")),
    }
}

struct Active {
    attempt: AttemptId,
    epoch: u32,
    phase: Phase,
    tracks: Vec<NativeTrackIndex>,
    /// The identity evidence the source stated for its tracks ([`UriSource::checks`]).
    checks: Vec<(TrackSelection, NativeTrackCheck)>,
    /// Each native audio track's language as the host reported it before PREPARED, in the player's order.
    native_audio: Vec<Option<String>>,
    start_deadline_ms: u64,
    prepared: bool,
    started_once: bool,
    /// The host reported the player paused (the viewer's pause or Home's) and has not reported it playing since.
    paused: bool,
    ended: bool,
    rebuffering: bool,
    stalled_since: Option<u64>,
    /// The last position the host reported while playing and when it first reported THAT value: a value that stays
    /// the same is a frozen picture ([`Timing::frozen_grace_ms`]). Reset by a pause, a play, a seek and a change of the host's buffering state.
    last_position: Option<(i64, u64)>,
    /// A frozen position was surfaced as a rebuffer and has not moved since.
    frozen: bool,
    controls: Vec<Pending>,
}

/// What reducing one host event asks of the machine beyond the events it pushed.
enum Next {
    Nothing,
    /// The attempt failed (an `Observation::Failed` was pushed): ask the host to close it.
    Close,
    /// The host confirmed the close.
    Released,
}

fn note(out: &mut VecDeque<DriverEvent>, attempt: AttemptId, observation: Observation) {
    out.push_back(DriverEvent::Observation { attempt, observation });
}

fn observed<T>(value: T) -> Observed<T> {
    Observed { value, from: EvidenceSource::BackendReport }
}

/// The pure step of the Tizen URI driver.
pub struct Machine {
    timing: Timing,
    staged: Option<Staged>,
    active: Option<Active>,
    last_started: Option<AttemptId>,
    next_epoch: u32,
    pending: VecDeque<DriverEvent>,
    /// Host events dropped for an epoch that is not the active one, since the machine was built.
    stale_dropped: usize,
    /// When a position or buffering event was last logged ([`EVENT_LOG_EVERY_MS`]).
    last_noisy_log_ms: Option<u64>,
}

/// Position and buffering events arrive several times a second: one of them is logged at most this often.
const EVENT_LOG_EVERY_MS: u64 = 10_000;

impl Machine {
    pub fn new(timing: Timing) -> Self {
        Self { timing, staged: None, active: None, last_started: None, next_epoch: 0, pending: VecDeque::new(),
            stale_dropped: 0, last_noisy_log_ms: None }
    }

    /// How many host events were dropped because their epoch was not the active one.
    #[cfg(test)]
    pub fn stale_dropped(&self) -> usize {
        self.stale_dropped
    }

    fn mint_epoch(&mut self) -> u32 {
        self.next_epoch = self.next_epoch.wrapping_add(1).max(1);
        self.next_epoch
    }

    /// An admission refusal before anything was opened. The common reducer wants a resource-owner verdict even then,
    /// so a machine that holds nothing says `Released` for the attempt; one that holds another attempt says nothing.
    fn refuse_before_open(&mut self, attempt: AttemptId, failure: Failure) -> DispatchResult {
        if self.active.is_none() {
            self.pending.push_back(DriverEvent::Release { attempt, status: ReleaseStatus::Released });
        }
        DispatchResult::Refused(failure)
    }

    // ---- staging ----

    /// Hold `source` for the Start of `attempt` until `now_ms` plus the source's lifetime.
    pub fn stage(&mut self, now_ms: u64, attempt: AttemptId, key: SourceKey, source: UriSource) -> Result<(), Failure> {
        if !source.has_url() || self.last_started.is_some_and(|last| attempt <= last) {
            return Err(Failure::Stale);
        }
        if !source.valid_track_map() {
            return Err(Failure::Unsupported);
        }
        self.staged = Some(Staged { attempt, key, source, deadline_ms: now_ms.saturating_add(self.timing.source_ttl_ms) });
        Ok(())
    }

    pub fn cancel_staged(&mut self, attempt: AttemptId) {
        if self.staged.as_ref().is_some_and(|s| s.attempt == attempt) {
            self.staged = None;
        }
    }

    pub fn discard_staged(&mut self, attempt: AttemptId, key: SourceKey) {
        if self.staged.as_ref().is_some_and(|s| s.attempt == attempt && s.key == key) {
            self.staged = None;
        }
    }

    // ---- commands ----

    /// Start `attempt` on its staged source. `offered` is the host's answer to "is there a player", captured by the
    /// executor. The open is committed here (the attempt is active and opening) and emitted as an [`Fx::Open`];
    /// a host that refuses it is [`open_refused`](Self::open_refused).
    pub fn start(&mut self, now_ms: u64, offered: bool, attempt: AttemptId, key: SourceKey, requirements: Requirements,
        fx: &mut Vec<Fx>) -> DispatchResult {
        // A Start consumes its own attempt's staged source even when it is then refused; another attempt's is untouched.
        let staged = if self.staged.as_ref().is_some_and(|s| s.attempt == attempt) { self.staged.take() } else { None };
        if self.active.is_some() {
            return self.refuse_before_open(attempt, Failure::Busy);
        }
        if self.last_started.is_some_and(|last| attempt <= last) {
            return self.refuse_before_open(attempt, Failure::Stale);
        }
        if requirements.source != SourceKind::ProgressiveUri || !CAPS.supports(requirements) {
            return self.refuse_before_open(attempt, Failure::Unsupported);
        }
        let Some(staged) = staged else { return self.refuse_before_open(attempt, Failure::Refused) };
        if staged.key != key || now_ms >= staged.deadline_ms {
            return self.refuse_before_open(attempt, Failure::Stale);
        }
        if !staged.source.can_supply(requirements) {
            return self.refuse_before_open(attempt, Failure::Unsupported);
        }
        if !offered {
            return self.refuse_before_open(attempt, Failure::Unavailable);
        }
        let epoch = self.mint_epoch();
        let checks = staged.source.checks().to_vec();
        let (url, tracks) = staged.source.into_parts();
        // The attempt id is spent by a Start the host then refuses: the planner mints a new one for its next try.
        self.last_started = Some(attempt);
        self.active = Some(Active { attempt, epoch, phase: Phase::Opening, tracks, checks, native_audio: Vec::new(),
            start_deadline_ms: now_ms.saturating_add(self.timing.startup_timeout_ms), prepared: false, started_once: false,
            paused: false, ended: false, rebuffering: false, stalled_since: None, last_position: None, frozen: false,
            controls: Vec::new() });
        fx.push(Fx::Open { epoch, url, area: FULL_WINDOW });
        DispatchResult::Queued
    }

    /// The host refused the open of `epoch`: nothing is held, and the Start that asked answers with the refusal.
    pub fn open_refused(&mut self, epoch: u32, refusal: Refusal) -> DispatchResult {
        let attempt = self.active.as_ref().filter(|a| a.epoch == epoch).map(|a| a.attempt);
        match attempt {
            Some(attempt) => {
                self.active = None;
                self.refuse_before_open(attempt, refusal.failure())
            }
            None => DispatchResult::Refused(refusal.failure()),
        }
    }

    /// The host refused `verb` on `epoch`. A refused play, pause or seek takes back the control it queued and the
    /// command answers with the refusal; a refused CLOSE has no command to answer, so it is returned as `None`
    /// after the machine decided what it means (see the module note; a close refused because the library is stopping
    /// is a release the host owns).
    pub fn verb_refused(&mut self, epoch: u32, verb: Verb, refusal: Refusal, fx: &mut Vec<Fx>) -> Option<DispatchResult> {
        if verb == Verb::Close && refusal == Refusal::Stopping && self.active.as_ref().is_some_and(|a| a.epoch == epoch) {
            // The library was told to stop and its gate to the host is shut: the close can never be asked. The host
            // closes its own player when it terminates (`plx_host.h`, STOP), so the release is the host's to make and
            // the library must not hold the stop for a confirmation that cannot come.
            let attempt = self.active.take().map(|a| a.attempt)?;
            fx.push(Fx::Log("tizen-uri: the library is stopping, so the close cannot be asked; the host closes its own player at terminate".to_owned()));
            self.pending.push_back(DriverEvent::Release { attempt, status: ReleaseStatus::Released });
            return None;
        }
        let active = self.active.as_mut().filter(|a| a.epoch == epoch)?;
        match verb {
            Verb::Close => {
                fx.push(Fx::Log(format!("tizen-uri: the host refused the close ({refusal:?}); the release is not confirmed")));
                if let Phase::Closing { deadline_ms, .. } = active.phase {
                    active.phase = Phase::Closing { deadline_ms, retry: refusal == Refusal::Busy };
                }
                None
            }
            Verb::Play | Verb::Pause | Verb::Seek { .. } | Verb::AudioTrack { .. } => {
                active.controls.pop();
                Some(DispatchResult::Refused(refusal.failure()))
            }
        }
    }

    /// Ask the host to close the active attempt and wait for its confirmation.
    fn begin_close(&mut self, now_ms: u64, fx: &mut Vec<Fx>) {
        let Some(active) = self.active.as_mut() else { return };
        if matches!(active.phase, Phase::Closing { .. } | Phase::Quarantined) {
            return;
        }
        active.phase = Phase::Closing { deadline_ms: now_ms.saturating_add(self.timing.release_allowance_ms), retry: false };
        fx.push(Fx::Verb { epoch: active.epoch, verb: Verb::Close });
    }

    /// A transport request (play, pause, seek) for the active attempt.
    pub fn control(&mut self, now_ms: u64, attempt: AttemptId, command: CommandId, operation: Operation,
        fx: &mut Vec<Fx>) -> DispatchResult {
        let Some(active) = self.active.as_mut().filter(|a| a.attempt == attempt) else {
            return DispatchResult::Refused(Failure::Stale);
        };
        if active.phase != Phase::Open || !active.prepared {
            return DispatchResult::Refused(Failure::Busy);
        }
        let verb = match operation {
            Operation::Play => Verb::Play,
            Operation::Pause => Verb::Pause,
            Operation::Seek(time) => {
                let Ok(ms) = i64::try_from(time.as_millis()) else { return DispatchResult::Refused(Failure::Unsupported) };
                let Ok(command) = i64::try_from(command.get()) else { return DispatchResult::Refused(Failure::Unsupported) };
                Verb::Seek { ms, command }
            }
            _ => return DispatchResult::Refused(Failure::Unsupported),
        };
        active.controls.push(Pending { command, operation, deadline_ms: now_ms.saturating_add(self.timing.control_timeout_ms),
            native: None });
        fx.push(Fx::Verb { epoch: active.epoch, verb });
        DispatchResult::Queued
    }

    /// An audio request: the stream the source's track map names, asked of the host at the native index its evidence
    /// picks ([`choose_native_audio`]) and settled when the host's `PLAYER_TRACK` answers (see [`CAPS`]). A stream the
    /// map does not name, or whose native track cannot be told, is `Unsupported` and logged.
    pub fn select_audio(&mut self, now_ms: u64, attempt: AttemptId, command: CommandId, choice: TrackChoice,
        fx: &mut Vec<Fx>) -> DispatchResult {
        let Some(active) = self.active.as_mut().filter(|a| a.attempt == attempt) else {
            return DispatchResult::Refused(Failure::Stale);
        };
        let TrackChoice::Selected(id) = choice else { return DispatchResult::Refused(Failure::Unsupported) };
        let track = TrackSelection { kind: TrackKind::Audio, id };
        if active.phase != Phase::Open || !active.prepared {
            return DispatchResult::Refused(Failure::Busy);
        }
        let Some(ordinal) = native_index(&active.tracks, track) else { return DispatchResult::Refused(Failure::Unsupported) };
        let check = active.checks.iter().find(|(stated, _)| *stated == track).map(|(_, check)| check);
        let chosen = match check {
            // The only audio stream is the one the player plays: settled by construction, as before the verb existed,
            // with no host call. Asking would prove nothing, and a firmware that reports no track for an HTTP source in
            // READY would answer -1 and halt the start of every single-audio title.
            Some(NativeTrackCheck::Sole) if ordinal == 0 => {
                fx.push(Fx::Log(format!("tizen-uri: audio stream {id} is the only one: selected by construction, no host call")));
                self.pending.push_back(DriverEvent::Observation { attempt, observation: Observation::TrackSelected(observed(track)) });
                self.pending.push_back(DriverEvent::TrackSettled { attempt, command, kind: TrackKind::Audio, choice });
                return DispatchResult::Queued;
            }
            Some(NativeTrackCheck::Sole) => Err(format!("a sole stream cannot sit at ordinal {ordinal}")),
            Some(NativeTrackCheck::Language(language)) => choose_native_audio(ordinal, language, &active.native_audio),
            None => Err("the source states no identity for the stream".to_owned()),
        };
        let index = match chosen {
            Ok(index) => index,
            Err(why) => {
                fx.push(Fx::Log(format!("tizen-uri: audio stream {id} at ordinal {ordinal} refused: {why}")));
                return DispatchResult::Refused(Failure::Unsupported);
            }
        };
        if index != ordinal {
            fx.push(Fx::Log(format!("tizen-uri: audio stream {id} is native track {index}, not its ordinal {ordinal}, by its language")));
        }
        let Ok(command_word) = i64::try_from(command.get()) else { return DispatchResult::Refused(Failure::Unsupported) };
        let operation = Operation::SelectTrack { kind: TrackKind::Audio, choice };
        active.controls.push(Pending { command, operation, deadline_ms: now_ms.saturating_add(self.timing.control_timeout_ms),
            native: Some(index) });
        fx.push(Fx::Verb { epoch: active.epoch, verb: Verb::AudioTrack { index, command: command_word } });
        DispatchResult::Queued
    }

    /// A subtitle request. The host player has no subtitle verb, so only Off can be honoured, and it is settled by
    /// construction: the player is never given a subtitle to show ([`CAPS`]). Selecting a subtitle is `Unsupported`.
    pub fn select_subtitle(&mut self, attempt: AttemptId, command: CommandId, choice: TrackChoice) -> DispatchResult {
        let Some(active) = self.active.as_ref().filter(|a| a.attempt == attempt) else {
            return DispatchResult::Refused(Failure::Stale);
        };
        if choice != TrackChoice::Off {
            return DispatchResult::Refused(Failure::Unsupported);
        }
        if active.phase != Phase::Open || !active.prepared {
            return DispatchResult::Refused(Failure::Busy);
        }
        self.pending.push_back(DriverEvent::TrackSettled { attempt, command, kind: TrackKind::Subtitle, choice });
        DispatchResult::Queued
    }

    /// A Stop of `attempt`: close, and wait for the host's confirmation.
    pub fn stop(&mut self, now_ms: u64, attempt: AttemptId, reason: StopReason, fx: &mut Vec<Fx>) -> DispatchResult {
        let Some(active) = self.active.as_ref().filter(|a| a.attempt == attempt) else {
            return DispatchResult::Refused(Failure::Stale);
        };
        if active.phase == Phase::Quarantined {
            return DispatchResult::Refused(Failure::ReleaseUncertain);
        }
        // Why the library closes the player (Suspend is the app's park at the background), for the next run's log.
        fx.push(Fx::Log(format!("tizen-uri: stop requested ({reason:?}); the host is asked to close the player")));
        self.begin_close(now_ms, fx);
        DispatchResult::Queued
    }

    // ---- the host's events ----

    /// Fold one host event into `active`, pushing what it means; the reducer is pure over (`active`, event, now).
    fn reduce(active: &mut Active, out: &mut VecDeque<DriverEvent>, event: PlayerEvent, now_ms: u64) -> Next {
        let attempt = active.attempt;
        if event.kind == PlayerKind::Closed {
            return Next::Released;
        }
        // After a close request nothing but the release matters, but a seek still in flight is answered.
        let live = matches!(active.phase, Phase::Opening | Phase::Open);
        match event.kind {
            PlayerKind::Prepared if live && !active.prepared => {
                active.prepared = true;
                active.phase = Phase::Open;
                note(out, attempt, Observation::Playback(observed(Playback::ReadyAwaitPlay)));
            }
            PlayerKind::Started if live && active.prepared => {
                active.started_once = true;
                active.paused = false;
                active.last_position = None;
                // A pause's stall belongs to the pause: playing again starts the grace afresh (a new STALL re-arms it).
                active.stalled_since = None;
                if let Some(at) = active.controls.iter().position(|c| c.operation == Operation::Play) {
                    let control = active.controls.remove(at);
                    out.push_back(DriverEvent::ControlAccepted { attempt, command: control.command,
                        operation: control.operation, native_acknowledged: true });
                }
                out.push_back(DriverEvent::Observation { attempt, observation: Observation::Playback(observed(Playback::Playing)) });
            }
            PlayerKind::Paused if live && active.prepared => {
                // A paused player is silent on purpose: the stall grace is suspended while it is.
                active.paused = true;
                active.last_position = None;
                active.stalled_since = None;
                if let Some(at) = active.controls.iter().position(|c| c.operation == Operation::Pause) {
                    let control = active.controls.remove(at);
                    out.push_back(DriverEvent::ControlAccepted { attempt, command: control.command,
                        operation: control.operation, native_acknowledged: true });
                }
                out.push_back(DriverEvent::Observation { attempt, observation: Observation::Playback(observed(Playback::Paused)) });
            }
            PlayerKind::Position if live && active.prepared => {
                let advanced = active.last_position.map_or(true, |(seen, _)| seen != event.a);
                if advanced {
                    // The position moved: the picture is not frozen and the read is not hung.
                    active.last_position = Some((event.a, now_ms));
                    active.stalled_since = None;
                    if std::mem::take(&mut active.frozen) && !active.rebuffering {
                        note(out, attempt, Observation::Rebuffering(observed(false)));
                    }
                }
                if let Ok(ms) = u64::try_from(event.a) {
                    out.push_back(DriverEvent::Observation { attempt,
                        observation: Observation::Position(observed(ContentTime::from_millis(ms))) });
                }
            }
            PlayerKind::SeekDone => {
                let at = active.controls.iter().position(|c| matches!(c.operation, Operation::Seek(_))
                    && i64::try_from(c.command.get()) == Ok(event.a));
                if let Some(at) = at {
                    let control = active.controls.remove(at);
                    active.last_position = None; // the position jumps: a new baseline
                    out.push_back(match event.b {
                        ms if ms >= 0 => DriverEvent::SeekLanded { attempt, command: control.command,
                            position: u64::try_from(ms).ok().map(|ms| observed(ContentTime::from_millis(ms))) },
                        // Expired (no completion within the host's 10 s): the seek MAY STILL LAND and `PLAYER_POSITION`
                        // is the authority on where the player is (`plx_host.h`), so this is no refusal of it. It is
                        // answered as landed with no position of its own: the next position the host reports says where.
                        -2 => DriverEvent::SeekLanded { attempt, command: control.command, position: None },
                        _ => DriverEvent::ControlRefused { attempt, command: control.command,
                            operation: control.operation, failure: Failure::Refused },
                    });
                }
            }
            // The host reports these before PREPARED, one per native audio track; a report after it changes nothing.
            PlayerKind::AudioLanguage if live && !active.prepared => {
                if let Some(index) = usize::try_from(event.a).ok().filter(|index| *index < AUDIO_LANGUAGE_REPORTS) {
                    if active.native_audio.len() <= index {
                        active.native_audio.resize(index + 1, None);
                    }
                    active.native_audio[index] = language_of_word(event.b);
                }
            }
            PlayerKind::Track => {
                let at = active.controls.iter().position(|c| matches!(c.operation, Operation::SelectTrack { .. })
                    && i64::try_from(c.command.get()) == Ok(event.a));
                if let Some(at) = at {
                    let control = active.controls.remove(at);
                    let Operation::SelectTrack { kind, choice } = control.operation else { unreachable!("matched above") };
                    let selected = match choice {
                        TrackChoice::Selected(id) => Some(TrackSelection { kind, id }),
                        _ => None,
                    }.filter(|_| live && event.b >= 0 && control.native == Some(event.b));
                    match selected {
                        Some(track) => {
                            note(out, attempt, Observation::TrackSelected(observed(track)));
                            out.push_back(DriverEvent::TrackSettled { attempt, command: control.command, kind, choice });
                        }
                        // Refused (b = -1), another index than the one asked, or a close already requested.
                        None => out.push_back(DriverEvent::ControlRefused { attempt, command: control.command,
                            operation: control.operation, failure: Failure::Refused }),
                    }
                }
            }
            PlayerKind::Buffering if live && active.started_once => {
                let rebuffering = event.a < 100;
                if rebuffering != active.rebuffering {
                    active.rebuffering = rebuffering;
                    // The position holds while the host buffers: entering or leaving the period starts the frozen-picture
                    // clock afresh (as a pause, a play and a seek do), or the exit would re-arm a stall from the old value.
                    active.last_position = None;
                    note(out, attempt, Observation::Rebuffering(observed(rebuffering)));
                }
            }
            // A completion before the player was prepared is no end of anything.
            PlayerKind::Completed if live && active.prepared && !active.ended => {
                active.ended = true;
                note(out, attempt, Observation::Playback(observed(Playback::Ended)));
            }
            PlayerKind::Error | PlayerKind::Interrupted if live => {
                note(out, attempt, Observation::Failed(Failure::Backend));
                return Next::Close;
            }
            PlayerKind::Stall if live && !active.paused => {
                active.stalled_since.get_or_insert(now_ms);
            }
            _ => {}
        }
        Next::Nothing
    }

    /// The deadlines: startup, a hung player, an unconfirmed control, an unconfirmed close.
    fn deadlines(&mut self, now_ms: u64, fx: &mut Vec<Fx>) {
        let Some(active) = self.active.as_mut() else { return };
        let attempt = active.attempt;
        let mut fail = false;
        match active.phase {
            Phase::Opening | Phase::Open => {
                if !active.prepared && now_ms >= active.start_deadline_ms {
                    fail = true;
                }
                // A position that stays the same while the player plays is a stall the host cannot see (its position
                // reads keep returning): surfaced as a rebuffer, then failed by the stall grace. Not while the host
                // itself reports buffering, nor with a seek in flight (the position is about to jump).
                if !fail && active.started_once && !active.paused && !active.ended && !active.rebuffering && !active.frozen
                    && !active.controls.iter().any(|c| matches!(c.operation, Operation::Seek(_)))
                    && active.last_position.is_some_and(|(_, since)| now_ms.saturating_sub(since) >= self.timing.frozen_grace_ms) {
                    active.frozen = true;
                    active.stalled_since.get_or_insert(now_ms);
                    fx.push(Fx::Log("tizen-uri: the position has not moved while playing; treated as a stall".to_owned()));
                    self.pending.push_back(DriverEvent::Observation { attempt, observation: Observation::Rebuffering(observed(true)) });
                }
                if !fail && !active.paused
                    && active.stalled_since.is_some_and(|since| now_ms.saturating_sub(since) >= self.timing.stall_grace_ms) {
                    fail = true;
                }
                let (expired, kept): (Vec<_>, Vec<_>) = active.controls.drain(..).partition(|c| now_ms >= c.deadline_ms);
                active.controls = kept;
                for control in expired {
                    fx.push(Fx::Log(format!("tizen-uri: control {:?} unconfirmed; refused (TimedOut)", control.operation)));
                    self.pending.push_back(DriverEvent::ControlRefused { attempt, command: control.command,
                        operation: control.operation, failure: Failure::TimedOut });
                }
                if fail {
                    self.pending.push_back(DriverEvent::Observation { attempt, observation: Observation::Failed(Failure::TimedOut) });
                }
            }
            Phase::Closing { deadline_ms, .. } if now_ms >= deadline_ms => {
                active.phase = Phase::Quarantined;
                self.pending.push_back(DriverEvent::Release { attempt, status: ReleaseStatus::Quarantined });
            }
            Phase::Closing { deadline_ms, retry: true } => {
                // The host queued nothing for the refused close, so no PLAYER_CLOSED is coming: ask again.
                active.phase = Phase::Closing { deadline_ms, retry: false };
                fx.push(Fx::Verb { epoch: active.epoch, verb: Verb::Close });
            }
            Phase::Closing { .. } | Phase::Quarantined => {}
        }
        if fail {
            self.begin_close(now_ms, fx);
        }
    }

    /// One step: the host's `events` (and whether its queue `overflowed`) at `now_ms`, and everything that is due
    /// by then. Returns what the machine observed, in order; requests it wants made are pushed onto `fx`.
    pub fn poll(&mut self, now_ms: u64, events: &[PlayerEvent], overflowed: bool, fx: &mut Vec<Fx>) -> Vec<DriverEvent> {
        if self.staged.as_ref().is_some_and(|s| now_ms >= s.deadline_ms) {
            self.staged = None;
        }
        let mut close = false;
        if overflowed {
            // The host's lossless queue lost an event: part of the player's story is gone, so the attempt cannot be trusted.
            fx.push(Fx::Log("tizen-uri: the host's player queue overflowed; the attempt is failed".to_owned()));
            if let Some(active) = self.active.as_ref().filter(|a| matches!(a.phase, Phase::Opening | Phase::Open)) {
                self.pending.push_back(DriverEvent::Observation { attempt: active.attempt, observation: Observation::Failed(Failure::Backend) });
                close = true;
            }
        }
        for event in events.iter().copied() {
            // One line per player event, so a run can show what the host said (SEEK_DONE's b, the Home sequence, a
            // STALL, an ERROR code). The two kinds that repeat many times a second are logged on a 10 s clock. No event
            // carries the URL or the token.
            let noisy = matches!(event.kind, PlayerKind::Position | PlayerKind::Buffering);
            if !noisy || self.last_noisy_log_ms.map_or(true, |at| now_ms.saturating_sub(at) >= EVENT_LOG_EVERY_MS) {
                if noisy {
                    self.last_noisy_log_ms = Some(now_ms);
                }
                fx.push(Fx::Log(format!("tizen-uri: player event {:?} epoch={} a={} b={}", event.kind, event.epoch, event.a, event.b)));
            }
            if event.kind == PlayerKind::Error {
                let who = if event.a >= PLAYER_FAIL_BASE { "the host" } else { "the native player" };
                fx.push(Fx::Log(format!("tizen-uri: {who} reported an error (code {})", event.a)));
            }
            let Some(active) = self.active.as_mut().filter(|a| a.epoch == event.epoch) else {
                self.stale_dropped += 1;
                if self.stale_dropped == 1 {
                    fx.push(Fx::Log(format!("tizen-uri: a host event ({:?}) of epoch {} is not the active player's; dropped",
                        event.kind, event.epoch)));
                }
                continue;
            };
            match Self::reduce(active, &mut self.pending, event, now_ms) {
                Next::Nothing => {}
                Next::Close => close = true,
                Next::Released => {
                    let attempt = active.attempt;
                    // Every seek and track request still held is answered before the release: the host sends its
                    // SEEK_DONE and PLAYER_TRACK first, but a machine must not depend on it.
                    for control in std::mem::take(&mut active.controls) {
                        self.pending.push_back(DriverEvent::ControlRefused { attempt, command: control.command,
                            operation: control.operation, failure: Failure::Refused });
                    }
                    self.pending.push_back(DriverEvent::Release { attempt, status: ReleaseStatus::Released });
                    self.active = None;
                    close = false;
                }
            }
        }
        if close {
            self.begin_close(now_ms, fx);
        }
        self.deadlines(now_ms, fx);
        self.pending.drain(..).collect()
    }

    // ---- facts ----

    /// The player is held from the open until the host confirmed its close, and a quarantine holds it still.
    pub fn exit_disposition(&self) -> ExitDisposition {
        if self.active.is_some() { ExitDisposition::PreserveNativeOwnership } else { ExitDisposition::Clean }
    }

    /// Owned from the Start until the close is confirmed.
    pub fn plane_owned(&self) -> bool {
        self.active.is_some()
    }

    /// An audio stream the active source's track map names: the ones [`Machine::select_audio`] can ask the host for.
    pub fn maps_track(&self, selection: TrackSelection) -> bool {
        selection.kind == TrackKind::Audio
            && self.active.as_ref().is_some_and(|a| native_index(&a.tracks, selection).is_some())
    }
}
