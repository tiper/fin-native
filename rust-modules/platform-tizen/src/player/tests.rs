//! The Tizen URI driver's tests: the driver over a scripted [`HostPlayer`], the shared conformance suite, the
//! port over the contract's fake host table (`hosted::testing`), the composition, and what the platform
//! certifies for the planner.
use super::compose::compose_over;
use super::driver::{plain_requirements, TizenUriDriver, Timing, CAPS, DISPLAY_TAG};
use super::port::{HostPlayer, HostedPlayer, Refusal, Verb, FULL_WINDOW};
use super::qualifier::TizenQualifier;
use crate::host_abi::{self, PlayerEvent, PlayerKind};
use crate::hosted::Drained;
use plx_base::task::MainThread;
use plx_media::player::adapter::PlayerAdapter;
use plx_media::route::uri_preflight::{OriginalQualifier, OriginalRequest, PmsAudioOutput, PmsRecipe, PmsRequest, PmsVerdict,
    ProductEvidence, ResolvePort, SourceSubtitle, SourceSubtitleStream, StandardDirectPlay, UriDecline, Verdict};
use plx_playback_contract::uri::{DisplayReadiness, NativeTrackCheck, NativeTrackIndex, UriDriver, UriSource};
use plx_playback_contract::uri_conformance::{conforms, UriHarness};
use plx_playback_contract::{AttemptId, BackendEvent, BackendKind, BackendSet, CommandId, ContentTime, DispatchResult,
    DriverEvent, DriverSet, ExitDisposition, Failure, Observation, Operation, Playback, ReleaseStatus, SourceKey,
    StopReason, TrackChoice, TrackKind, TrackSelection};
use std::sync::{Arc, Mutex};

const URL: &str = "http://10.0.2.2:32499/library/parts/3134/1/file.mkv?X-Plex-Token=PLACEHOLDER";

#[derive(Default)]
struct Script {
    unavailable: Option<&'static str>,
    opens: Vec<(u32, String, [i32; 4])>,
    verbs: Vec<(u32, Verb)>,
    inbox: Vec<PlayerEvent>,
    refuse_open: Option<Refusal>,
    refuse_verb: Option<Refusal>,
    overflow: bool,
    cues: Vec<host_abi::HostCue>,
    cues_dropped: usize,
    now: u64,
}

/// A scripted host player: records what the driver asks, hands back the events a test queued, and keeps the clock.
#[derive(Clone, Default)]
struct Fake(Arc<Mutex<Script>>);

impl Fake {
    fn with<R>(&self, f: impl FnOnce(&mut Script) -> R) -> R {
        f(&mut self.0.lock().unwrap())
    }
    fn post(&self, epoch: u32, kind: PlayerKind, a: i64, b: i64) {
        self.with(|s| s.inbox.push(PlayerEvent { epoch, kind, a, b }));
    }
    fn advance(&self, ms: u64) {
        self.with(|s| s.now += ms);
    }
    fn epoch(&self) -> u32 {
        self.with(|s| s.opens.last().expect("an open").0)
    }
    fn verbs(&self) -> Vec<Verb> {
        self.with(|s| s.verbs.iter().map(|v| v.1).collect())
    }
}

impl HostPlayer for Fake {
    fn offered(&self) -> Result<(), &'static str> {
        self.with(|s| s.unavailable.map_or(Ok(()), Err))
    }
    fn open(&mut self, epoch: u32, uri: &str, area: [i32; 4]) -> Result<(), Refusal> {
        self.with(|s| match s.refuse_open {
            Some(refusal) => Err(refusal),
            None => {
                s.opens.push((epoch, uri.to_owned(), area));
                Ok(())
            }
        })
    }
    fn verb(&mut self, epoch: u32, verb: Verb) -> Result<(), Refusal> {
        self.with(|s| match s.refuse_verb {
            Some(refusal) => Err(refusal),
            None => {
                s.verbs.push((epoch, verb));
                Ok(())
            }
        })
    }
    fn drain(&mut self, out: &mut Vec<PlayerEvent>) -> Drained {
        self.with(|s| {
            out.append(&mut s.inbox);
            Drained { overflowed: std::mem::take(&mut s.overflow), stale: 0 }
        })
    }
    fn drain_cues(&mut self, out: &mut Vec<host_abi::HostCue>) -> usize {
        self.with(|s| {
            out.append(&mut s.cues);
            std::mem::take(&mut s.cues_dropped)
        })
    }
    fn now_ms(&self) -> u64 {
        self.with(|s| s.now)
    }
}

const TIMING: Timing = Timing { startup_timeout_ms: 1_000, release_allowance_ms: 500, control_timeout_ms: 300,
    stall_grace_ms: 200, frozen_grace_ms: 100, source_ttl_ms: 5_000 };

fn mt() -> MainThread {
    // SAFETY: a host test thread stands in for the loop thread, as the contract's own fakes' tests do.
    unsafe { MainThread::assume() }
}

fn attempt(n: u64) -> AttemptId {
    AttemptId::new(1, n)
}

fn cmd(n: u64) -> CommandId {
    CommandId::for_test(n)
}

fn rig() -> (TizenUriDriver<Fake>, Fake) {
    let fake = Fake::default();
    (TizenUriDriver::new(fake.clone(), TIMING), fake)
}

fn dispatch(driver: &mut TizenUriDriver<Fake>, attempt: AttemptId, command: u64, op: Operation) -> DispatchResult {
    driver.dispatch(BackendKind::Uri, &mt(), attempt, cmd(command), op)
}

fn stage_and_start(driver: &mut TizenUriDriver<Fake>, attempt: AttemptId, tracks: Vec<NativeTrackIndex>) {
    let key = SourceKey::new(attempt.generation());
    assert_eq!(driver.stage_source(attempt, key, UriSource::new(URL.into(), tracks)), Ok(()));
    let start = Operation::Start { source: key, requirements: plain_requirements() };
    assert_eq!(dispatch(driver, attempt, 1, start), DispatchResult::Queued);
}

fn poll(driver: &mut TizenUriDriver<Fake>) -> Vec<DriverEvent> {
    driver.poll(&mt()).into_iter().map(|e| {
        assert_eq!(e.backend, BackendKind::Uri);
        e.event
    }).collect()
}

fn observation(attempt: AttemptId, observation: Observation) -> DriverEvent {
    DriverEvent::Observation { attempt, observation }
}

fn seen<T>(value: T) -> plx_playback_contract::Observed<T> {
    plx_playback_contract::Observed { value, from: plx_playback_contract::EvidenceSource::BackendReport }
}

/// An attempt that is open and prepared: the open, the host's PREPARED, drained.
fn prepared(driver: &mut TizenUriDriver<Fake>, fake: &Fake, n: u64) -> (AttemptId, u32) {
    let a = attempt(n);
    stage_and_start(driver, a, vec![]);
    let epoch = fake.epoch();
    fake.post(epoch, PlayerKind::Prepared, 600_000, 0);
    assert_eq!(poll(driver), [observation(a, Observation::Playback(seen(Playback::ReadyAwaitPlay)))]);
    (a, epoch)
}

#[test]
fn a_start_opens_the_player_over_the_whole_window_and_prepared_is_ready_await_play() {
    let (mut driver, fake) = rig();
    let a = attempt(1);
    stage_and_start(&mut driver, a, vec![]);
    assert_eq!(fake.with(|s| s.opens.clone()), [(1, URL.to_owned(), FULL_WINDOW)]);
    assert!(driver.plane_owned());
    assert_eq!(driver.exit_disposition(), ExitDisposition::PreserveNativeOwnership);
    assert_eq!(poll(&mut driver), [], "nothing is reported until the host says something");
    // A control before the player is prepared is the driver's `Busy`, not a host call.
    assert_eq!(dispatch(&mut driver, a, 2, Operation::Play), DispatchResult::Refused(Failure::Busy));
    assert!(fake.verbs().is_empty());
    fake.post(1, PlayerKind::Prepared, 600_000, 0);
    assert_eq!(poll(&mut driver), [observation(a, Observation::Playback(seen(Playback::ReadyAwaitPlay)))]);
}

#[test]
fn play_is_accepted_by_started_and_pause_by_paused() {
    let (mut driver, fake) = rig();
    let (a, epoch) = prepared(&mut driver, &fake, 1);
    assert_eq!(dispatch(&mut driver, a, 2, Operation::Play), DispatchResult::Queued);
    assert_eq!(fake.verbs(), [Verb::Play]);
    assert_eq!(poll(&mut driver), [], "a queued play is not a playing player");
    fake.post(epoch, PlayerKind::Started, 0, 0);
    assert_eq!(poll(&mut driver), [
        DriverEvent::ControlAccepted { attempt: a, command: cmd(2), operation: Operation::Play, native_acknowledged: true },
        observation(a, Observation::Playback(seen(Playback::Playing))),
    ]);
    fake.post(epoch, PlayerKind::Position, 1_250, 0);
    assert_eq!(poll(&mut driver), [observation(a, Observation::Position(seen(ContentTime::from_millis(1_250))))]);
    assert_eq!(dispatch(&mut driver, a, 3, Operation::Pause), DispatchResult::Queued);
    assert_eq!(fake.verbs(), [Verb::Play, Verb::Pause]);
    fake.post(epoch, PlayerKind::Paused, 0, 0);
    assert_eq!(poll(&mut driver), [
        DriverEvent::ControlAccepted { attempt: a, command: cmd(3), operation: Operation::Pause, native_acknowledged: true },
        observation(a, Observation::Playback(seen(Playback::Paused))),
    ]);
}

#[test]
fn a_seek_lands_on_seek_done_a_refused_one_is_a_refusal_and_an_expired_one_waits_for_the_position() {
    let (mut driver, fake) = rig();
    let (a, epoch) = prepared(&mut driver, &fake, 1);
    for (command, b, want) in [
        (10, 0, DriverEvent::SeekLanded { attempt: a, command: cmd(10), position: Some(seen(ContentTime::from_millis(0))) }),
        (11, 91_000, DriverEvent::SeekLanded { attempt: a, command: cmd(11), position: Some(seen(ContentTime::from_millis(91_000))) }),
        (12, -1, DriverEvent::ControlRefused { attempt: a, command: cmd(12), operation: Operation::Seek(ContentTime::from_millis(5_000)),
            failure: Failure::Refused }),
        // Expired (b = -2): the seek MAY STILL LAND and the position is the authority, so it is no refusal.
        (13, -2, DriverEvent::SeekLanded { attempt: a, command: cmd(13), position: None }),
    ] {
        let target = if b >= 0 { b as u64 } else { 5_000 };
        assert_eq!(dispatch(&mut driver, a, command, Operation::Seek(ContentTime::from_millis(target))), DispatchResult::Queued);
        assert_eq!(fake.verbs().last(), Some(&Verb::Seek { ms: target as i64, command: command as i64 }));
        fake.post(epoch, PlayerKind::SeekDone, command as i64, b);
        assert_eq!(poll(&mut driver), [want], "command {command}");
    }
    // A completion for a command nobody is waiting on maps to nothing.
    fake.post(epoch, PlayerKind::SeekDone, 99, 1);
    assert_eq!(poll(&mut driver), []);
}

#[test]
fn an_unconfirmed_control_is_refused_when_its_deadline_passes() {
    let (mut driver, fake) = rig();
    let (a, _) = prepared(&mut driver, &fake, 1);
    assert_eq!(dispatch(&mut driver, a, 2, Operation::Play), DispatchResult::Queued);
    fake.advance(299);
    assert_eq!(poll(&mut driver), []);
    fake.advance(1);
    assert_eq!(poll(&mut driver), [DriverEvent::ControlRefused { attempt: a, command: cmd(2), operation: Operation::Play,
        failure: Failure::TimedOut }]);
}

#[test]
fn completion_ends_once_and_a_completion_before_prepare_is_nothing() {
    let (mut driver, fake) = rig();
    let a = attempt(1);
    stage_and_start(&mut driver, a, vec![]);
    fake.post(1, PlayerKind::Completed, 0, 0);
    assert_eq!(poll(&mut driver), [], "completion alone before the player is prepared proves nothing");
    fake.post(1, PlayerKind::Prepared, 1_000, 0);
    fake.post(1, PlayerKind::Completed, 0, 0);
    fake.post(1, PlayerKind::Completed, 0, 0);
    assert_eq!(poll(&mut driver), [
        observation(a, Observation::Playback(seen(Playback::ReadyAwaitPlay))),
        observation(a, Observation::Playback(seen(Playback::Ended))),
    ]);
}

#[test]
fn an_error_fails_the_attempt_closes_the_player_and_the_release_follows_the_hosts_close() {
    let (mut driver, fake) = rig();
    let (a, epoch) = prepared(&mut driver, &fake, 1);
    fake.post(epoch, PlayerKind::Error, host_abi::PLAYER_FAIL_BASE + 1, 0);
    assert_eq!(poll(&mut driver), [observation(a, Observation::Failed(Failure::Backend))]);
    assert_eq!(fake.verbs(), [Verb::Close], "a failed player is closed");
    assert_eq!(driver.exit_disposition(), ExitDisposition::PreserveNativeOwnership, "not released until the host says so");
    fake.post(epoch, PlayerKind::Closed, 0, 0);
    assert_eq!(poll(&mut driver), [DriverEvent::Release { attempt: a, status: ReleaseStatus::Released }]);
    assert_eq!(driver.exit_disposition(), ExitDisposition::Clean);
    assert!(!driver.plane_owned());
}

#[test]
fn a_player_that_never_prepares_fails_at_its_startup_deadline() {
    let (mut driver, fake) = rig();
    let a = attempt(1);
    stage_and_start(&mut driver, a, vec![]);
    fake.advance(999);
    assert_eq!(poll(&mut driver), []);
    fake.advance(1);
    assert_eq!(poll(&mut driver), [observation(a, Observation::Failed(Failure::TimedOut))]);
    assert_eq!(fake.verbs(), [Verb::Close]);
}

#[test]
fn a_stalled_position_read_fails_the_player_only_if_it_stays_silent() {
    let (mut driver, fake) = rig();
    let (a, epoch) = prepared(&mut driver, &fake, 1);
    fake.post(epoch, PlayerKind::Stall, 0, 0);
    assert_eq!(poll(&mut driver), []);
    fake.advance(150);
    fake.post(epoch, PlayerKind::Position, 4_000, 0);
    assert_eq!(poll(&mut driver), [observation(a, Observation::Position(seen(ContentTime::from_millis(4_000))))]);
    fake.advance(500);
    assert_eq!(poll(&mut driver), [], "the position cleared the stall");
    fake.post(epoch, PlayerKind::Stall, 0, 0);
    assert_eq!(poll(&mut driver), []);
    fake.advance(200);
    assert_eq!(poll(&mut driver), [observation(a, Observation::Failed(Failure::TimedOut))]);
    assert_eq!(fake.verbs(), [Verb::Close]);
}

#[test]
fn a_pause_suspends_the_stall_grace_and_playing_again_starts_it_afresh() {
    let (mut driver, fake) = rig();
    let (a, epoch) = prepared(&mut driver, &fake, 1);
    fake.post(epoch, PlayerKind::Started, 0, 0);
    fake.post(epoch, PlayerKind::Stall, 0, 0);
    fake.advance(100);
    // The viewer's (or Home's) pause arrives while the position read was hung: a paused player is silent on purpose.
    fake.post(epoch, PlayerKind::Paused, 0, 0);
    let events = poll(&mut driver);
    assert_eq!(events.last(), Some(&observation(a, Observation::Playback(seen(Playback::Paused)))));
    fake.advance(10_000);
    assert_eq!(poll(&mut driver), [], "a working paused attempt is not failed for its silence");
    // A stall reported while paused is not armed either.
    fake.post(epoch, PlayerKind::Stall, 0, 0);
    fake.advance(10_000);
    assert_eq!(poll(&mut driver), []);
    // Playing again re-arms nothing by itself; a new STALL starts a new grace.
    fake.post(epoch, PlayerKind::Started, 0, 0);
    fake.advance(10_000);
    let events = poll(&mut driver);
    assert!(!events.iter().any(|e| matches!(e, DriverEvent::Observation { observation: Observation::Failed(_), .. })), "{events:?}");
    fake.post(epoch, PlayerKind::Stall, 0, 0);
    assert_eq!(poll(&mut driver), []);
    fake.advance(200);
    assert_eq!(poll(&mut driver), [observation(a, Observation::Failed(Failure::TimedOut))]);
}

#[test]
fn a_position_that_stays_the_same_while_playing_is_a_stall_the_host_cannot_see() {
    // The emulator's A/V clip: the picture freezes, the position reads keep answering the same value, so no
    // PLAYER_STALL is ever raised and the library kept reporting "playing t=15s" every 10 s.
    let (mut driver, fake) = rig();
    let (a, epoch) = prepared(&mut driver, &fake, 1);
    fake.post(epoch, PlayerKind::Started, 0, 0);
    fake.post(epoch, PlayerKind::Position, 14_900, 0);
    fake.post(epoch, PlayerKind::Position, 15_000, 0);
    poll(&mut driver);
    for _ in 0..3 {
        fake.advance(30);
        fake.post(epoch, PlayerKind::Position, 15_000, 0);
        let events = poll(&mut driver);
        assert!(!events.iter().any(|e| matches!(e, DriverEvent::Observation { observation: Observation::Rebuffering(_), .. })), "not yet");
    }
    fake.advance(30);
    fake.post(epoch, PlayerKind::Position, 15_000, 0);
    assert_eq!(poll(&mut driver), [
        observation(a, Observation::Position(seen(ContentTime::from_millis(15_000)))),
        observation(a, Observation::Rebuffering(seen(true))),
    ], "the frozen position is surfaced as a rebuffer after the grace");
    // Moving again clears it and the stall.
    fake.advance(100);
    fake.post(epoch, PlayerKind::Position, 15_250, 0);
    assert_eq!(poll(&mut driver), [
        observation(a, Observation::Rebuffering(seen(false))),
        observation(a, Observation::Position(seen(ContentTime::from_millis(15_250)))),
    ]);
    fake.advance(1_000);
    fake.post(epoch, PlayerKind::Position, 15_500, 0);
    assert!(!poll(&mut driver).iter().any(|e| matches!(e, DriverEvent::Observation { observation: Observation::Failed(_), .. })));
    // Frozen for good: the same stall grace as a hung read fails the attempt.
    fake.advance(100);
    fake.post(epoch, PlayerKind::Position, 15_500, 0);
    poll(&mut driver);
    fake.advance(200);
    fake.post(epoch, PlayerKind::Position, 15_500, 0);
    let events = poll(&mut driver);
    assert!(events.contains(&observation(a, Observation::Failed(Failure::TimedOut))), "{events:?}");
    assert_eq!(fake.verbs(), [Verb::Close]);
}

#[test]
fn a_paused_player_a_seek_in_flight_and_host_buffering_are_not_a_frozen_picture() {
    let (mut driver, fake) = rig();
    let (a, epoch) = prepared(&mut driver, &fake, 1);
    fake.post(epoch, PlayerKind::Started, 0, 0);
    fake.post(epoch, PlayerKind::Position, 15_000, 0);
    poll(&mut driver);
    // Paused: the position holds on purpose.
    fake.post(epoch, PlayerKind::Paused, 0, 0);
    poll(&mut driver);
    for _ in 0..5 {
        fake.advance(200);
        fake.post(epoch, PlayerKind::Position, 15_000, 0);
        let events = poll(&mut driver);
        assert!(!events.iter().any(|e| matches!(e, DriverEvent::Observation { observation: Observation::Rebuffering(_) | Observation::Failed(_), .. })), "{events:?}");
    }
    // A seek in flight: the position is about to jump.
    fake.post(epoch, PlayerKind::Started, 0, 0);
    fake.post(epoch, PlayerKind::Position, 15_000, 0);
    poll(&mut driver);
    assert_eq!(dispatch(&mut driver, a, 9, Operation::Seek(ContentTime::from_millis(60_000))), DispatchResult::Queued);
    fake.advance(250);
    fake.post(epoch, PlayerKind::Position, 15_000, 0);
    let events = poll(&mut driver);
    assert!(!events.iter().any(|e| matches!(e, DriverEvent::Observation { observation: Observation::Rebuffering(_), .. })), "{events:?}");
}

/// A host buffering period is not part of the frozen-picture clock: the position the host reports holds while it buffers,
/// so leaving the period starts the baseline afresh (as a pause, a play and a seek do). Without that, Buffering(100) after
/// the grace re-armed a stall at once and the readout flapped between Rebuffering(false) and Rebuffering(true).
#[test]
fn leaving_host_buffering_starts_the_frozen_picture_clock_afresh() {
    let (mut driver, fake) = rig();
    let (a, epoch) = prepared(&mut driver, &fake, 1);
    fake.post(epoch, PlayerKind::Started, 0, 0);
    fake.post(epoch, PlayerKind::Position, 15_000, 0);
    poll(&mut driver);
    fake.post(epoch, PlayerKind::Buffering, 40, 0);
    let events = poll(&mut driver);
    assert!(events.contains(&observation(a, Observation::Rebuffering(seen(true)))), "{events:?}");
    // Buffered for longer than the frozen grace with the position holding; then the host reports the buffer full.
    fake.advance(150);
    fake.post(epoch, PlayerKind::Buffering, 100, 0);
    fake.post(epoch, PlayerKind::Position, 15_000, 0);
    let events = poll(&mut driver);
    assert_eq!(events.iter().filter(|e| matches!(e, DriverEvent::Observation { observation: Observation::Rebuffering(_), .. })).count(), 1,
        "only the exit of the buffering period is reported, no stall is re-armed from the old baseline: {events:?}");
    assert!(events.contains(&observation(a, Observation::Rebuffering(seen(false)))), "{events:?}");
    // A picture that then really holds is a stall after a full grace of its own.
    fake.advance(90);
    fake.post(epoch, PlayerKind::Position, 15_000, 0);
    let events = poll(&mut driver);
    assert!(!events.contains(&observation(a, Observation::Rebuffering(seen(true)))), "{events:?}");
    fake.advance(20);
    fake.post(epoch, PlayerKind::Position, 15_000, 0);
    let events = poll(&mut driver);
    assert!(events.contains(&observation(a, Observation::Rebuffering(seen(true)))), "{events:?}");
}

#[test]
fn a_close_refused_as_busy_is_asked_again_and_other_refusals_wait_for_the_deadline() {
    let (mut driver, fake) = rig();
    let (a, epoch) = prepared(&mut driver, &fake, 1);
    // The host's queue is full: nothing was queued, so no PLAYER_CLOSED is coming until the close is asked again.
    fake.with(|s| s.refuse_verb = Some(Refusal::Busy));
    assert_eq!(dispatch(&mut driver, a, 5, Operation::Stop(StopReason::Stop)), DispatchResult::Queued);
    assert!(fake.verbs().is_empty());
    fake.advance(100);
    assert_eq!(poll(&mut driver), [], "still busy: still held, not yet quarantined");
    fake.with(|s| s.refuse_verb = None);
    fake.advance(100);
    assert_eq!(poll(&mut driver), []);
    assert_eq!(fake.verbs(), [Verb::Close], "the next poll asked again and the host took it");
    fake.advance(100);
    assert_eq!(poll(&mut driver), [], "a close the host took is asked once");
    assert_eq!(fake.verbs(), [Verb::Close]);
    fake.post(epoch, PlayerKind::Closed, 0, 0);
    assert_eq!(poll(&mut driver), [DriverEvent::Release { attempt: a, status: ReleaseStatus::Released }]);
    assert_eq!(driver.exit_disposition(), ExitDisposition::Clean);
}

#[test]
fn buffering_is_a_rebuffer_only_after_the_first_start() {
    let (mut driver, fake) = rig();
    let (a, epoch) = prepared(&mut driver, &fake, 1);
    fake.post(epoch, PlayerKind::Buffering, 10, 0);
    assert_eq!(poll(&mut driver), [], "startup buffering is not a rebuffer");
    fake.post(epoch, PlayerKind::Started, 0, 0);
    fake.post(epoch, PlayerKind::Buffering, 40, 0);
    fake.post(epoch, PlayerKind::Buffering, 100, 0);
    let events = poll(&mut driver);
    assert_eq!(&events[1..], [
        observation(a, Observation::Rebuffering(seen(true))),
        observation(a, Observation::Rebuffering(seen(false))),
    ]);
}

#[test]
fn a_completion_after_stop_is_dropped_and_the_release_comes_once() {
    let (mut driver, fake) = rig();
    let (a, epoch) = prepared(&mut driver, &fake, 1);
    assert_eq!(dispatch(&mut driver, a, 5, Operation::Stop(StopReason::Stop)), DispatchResult::Queued);
    assert_eq!(fake.verbs(), [Verb::Close]);
    // While the close is in flight the player's own late news is not mapped...
    fake.post(epoch, PlayerKind::Completed, 0, 0);
    fake.post(epoch, PlayerKind::Position, 9, 0);
    assert_eq!(poll(&mut driver), []);
    fake.post(epoch, PlayerKind::Closed, 0, 0);
    assert_eq!(poll(&mut driver), [DriverEvent::Release { attempt: a, status: ReleaseStatus::Released }]);
    // ...and after the release, the same events of that epoch are dropped and counted, never mapped.
    fake.post(epoch, PlayerKind::Completed, 0, 0);
    fake.post(epoch, PlayerKind::Prepared, 1, 0);
    fake.post(epoch, PlayerKind::Closed, 0, 0);
    assert_eq!(poll(&mut driver), [], "exactly one release");
    assert_eq!(driver.stale_dropped(), 3);
}

#[test]
fn a_restart_ignores_the_old_epochs_events() {
    let (mut driver, fake) = rig();
    let (a, first) = prepared(&mut driver, &fake, 1);
    assert_eq!(dispatch(&mut driver, a, 5, Operation::Stop(StopReason::Replace)), DispatchResult::Queued);
    fake.post(first, PlayerKind::Closed, 0, 0);
    assert_eq!(poll(&mut driver), [DriverEvent::Release { attempt: a, status: ReleaseStatus::Released }]);
    let b = attempt(2);
    stage_and_start(&mut driver, b, vec![]);
    let second = fake.epoch();
    assert_ne!(first, second, "an epoch per open");
    fake.post(first, PlayerKind::Completed, 0, 0);
    fake.post(first, PlayerKind::Prepared, 5, 0);
    fake.post(second, PlayerKind::Prepared, 1_000, 0);
    assert_eq!(poll(&mut driver), [observation(b, Observation::Playback(seen(Playback::ReadyAwaitPlay)))]);
    assert_eq!(driver.stale_dropped(), 2);
}

#[test]
fn a_close_the_host_never_confirms_is_quarantined_and_exit_stays_preserved_until_it_is() {
    let (mut driver, fake) = rig();
    let (a, epoch) = prepared(&mut driver, &fake, 1);
    assert_eq!(dispatch(&mut driver, a, 5, Operation::Stop(StopReason::Exit)), DispatchResult::Queued);
    fake.advance(499);
    assert_eq!(poll(&mut driver), []);
    fake.advance(1);
    assert_eq!(poll(&mut driver), [DriverEvent::Release { attempt: a, status: ReleaseStatus::Quarantined }]);
    assert_eq!(driver.exit_disposition(), ExitDisposition::PreserveNativeOwnership, "a deadline is never a clean exit");
    assert_eq!(dispatch(&mut driver, a, 6, Operation::Stop(StopReason::Exit)), DispatchResult::Refused(Failure::ReleaseUncertain));
    // The host's late close is the explicit release.
    fake.post(epoch, PlayerKind::Closed, 0, 0);
    assert_eq!(poll(&mut driver), [DriverEvent::Release { attempt: a, status: ReleaseStatus::Released }]);
    assert_eq!(driver.exit_disposition(), ExitDisposition::Clean);
}

#[test]
fn a_close_the_host_refuses_leaves_the_player_held_and_quarantines_at_the_deadline() {
    let (mut driver, fake) = rig();
    let (a, _) = prepared(&mut driver, &fake, 1);
    fake.with(|s| s.refuse_verb = Some(Refusal::Unavailable));
    assert_eq!(dispatch(&mut driver, a, 5, Operation::Stop(StopReason::Exit)), DispatchResult::Queued);
    assert_eq!(driver.exit_disposition(), ExitDisposition::PreserveNativeOwnership);
    fake.with(|s| s.refuse_verb = None);
    fake.advance(100);
    assert_eq!(poll(&mut driver), [], "a refusal that is not busy is not asked again");
    assert!(fake.verbs().is_empty());
    fake.advance(400);
    assert_eq!(poll(&mut driver), [DriverEvent::Release { attempt: a, status: ReleaseStatus::Quarantined }]);
    assert_eq!(driver.exit_disposition(), ExitDisposition::PreserveNativeOwnership);
}

#[test]
fn a_close_refused_because_the_library_is_stopping_is_the_hosts_to_make_and_does_not_hold_the_exit() {
    let (mut driver, fake) = rig();
    let (a, _) = prepared(&mut driver, &fake, 1);
    fake.with(|s| s.refuse_verb = Some(Refusal::Stopping));
    assert_eq!(dispatch(&mut driver, a, 5, Operation::Stop(StopReason::Exit)), DispatchResult::Queued);
    assert_eq!(poll(&mut driver), [DriverEvent::Release { attempt: a, status: ReleaseStatus::Released }],
        "no deadline wait: the close can never be asked");
    assert_eq!(driver.exit_disposition(), ExitDisposition::Clean);
    assert!(!driver.plane_owned());
}

#[test]
fn an_open_the_host_refuses_is_a_refusal_with_a_release_verdict_and_holds_nothing() {
    let (mut driver, fake) = rig();
    fake.with(|s| s.refuse_open = Some(Refusal::Busy));
    let a = attempt(1);
    let key = SourceKey::new(1);
    assert_eq!(driver.stage_source(a, key, UriSource::new(URL.into(), vec![])), Ok(()));
    let start = Operation::Start { source: key, requirements: plain_requirements() };
    assert_eq!(dispatch(&mut driver, a, 1, start), DispatchResult::Refused(Failure::Busy));
    assert_eq!(poll(&mut driver), [DriverEvent::Release { attempt: a, status: ReleaseStatus::Released }]);
    assert_eq!(driver.exit_disposition(), ExitDisposition::Clean);
    assert!(!driver.plane_owned());
}

#[test]
fn a_start_needs_the_staged_source_of_its_own_attempt_and_one_attempt_at_a_time() {
    let (mut driver, fake) = rig();
    let key = SourceKey::new(1);
    let start = Operation::Start { source: key, requirements: plain_requirements() };
    assert_eq!(dispatch(&mut driver, attempt(1), 1, start), DispatchResult::Refused(Failure::Refused), "nothing staged");
    assert_eq!(driver.stage_source(attempt(2), key, UriSource::new(URL.into(), vec![])), Ok(()));
    assert_eq!(dispatch(&mut driver, attempt(3), 1, start), DispatchResult::Refused(Failure::Refused), "another attempt's source");
    assert_eq!(dispatch(&mut driver, attempt(2), 1, start), DispatchResult::Queued);
    assert_eq!(driver.stage_source(attempt(3), key, UriSource::new(URL.into(), vec![])), Ok(()));
    assert_eq!(dispatch(&mut driver, attempt(3), 2, start), DispatchResult::Refused(Failure::Busy), "attempt 2 holds the player");
    assert_eq!(driver.stage_source(attempt(1), key, UriSource::new(URL.into(), vec![])), Err(Failure::Stale), "an older attempt");
    assert_eq!(dispatch(&mut driver, attempt(9), 3, Operation::Play), DispatchResult::Refused(Failure::Stale));
    assert_eq!(fake.with(|s| s.opens.len()), 1);
    // A staged source expires.
    driver.cancel_staged(attempt(3));
    assert_eq!(driver.stage_source(attempt(4), key, UriSource::new(URL.into(), vec![])), Ok(()));
    fake.advance(5_000);
    assert_eq!(dispatch(&mut driver, attempt(4), 4, start), DispatchResult::Refused(Failure::Busy));
}

/// The host's PREPARED-time report of one native audio track's language: `b` is the code's ASCII bytes, first
/// character in the lowest byte (`plx_host.h`, PLAYER_AUDIO_LANGUAGE).
fn language_word(code: &str) -> i64 {
    let mut bytes = [0u8; 8];
    bytes[..code.len()].copy_from_slice(code.as_bytes());
    i64::from_le_bytes(bytes)
}

/// Start an attempt whose track map names `selection` at `ordinal`, with `check`, on a host whose player reports
/// `native` as its audio tracks' languages ("" = no code) before PREPARED.
fn prepared_with(driver: &mut TizenUriDriver<Fake>, fake: &Fake, selection: TrackSelection, ordinal: i64,
    check: NativeTrackCheck, native: &[&str]) -> (AttemptId, u32) {
    let a = attempt(1);
    let key = SourceKey::new(a.generation());
    let source = UriSource::new(URL.into(), vec![NativeTrackIndex { selection, index: ordinal }]).with_check(selection, check);
    assert_eq!(driver.stage_source(a, key, source), Ok(()));
    let start = Operation::Start { source: key, requirements: plain_requirements() };
    assert_eq!(dispatch(driver, a, 1, start), DispatchResult::Queued);
    let epoch = fake.epoch();
    for (index, code) in native.iter().enumerate() {
        fake.post(epoch, PlayerKind::AudioLanguage, index as i64, if code.is_empty() { 0 } else { language_word(code) });
    }
    fake.post(epoch, PlayerKind::Prepared, 1_000, 0);
    poll(driver);
    (a, epoch)
}

const SECOND: TrackSelection = TrackSelection { kind: TrackKind::Audio, id: 31343 };

fn pick(id: u64) -> Operation {
    Operation::SelectTrack { kind: TrackKind::Audio, choice: TrackChoice::Selected(id) }
}

fn german() -> NativeTrackCheck {
    NativeTrackCheck::Language("ger".into())
}

/// **The selected audio stream is asked of the host and settles on its answer.** The ordinal's native track has the
/// stream's language, so the driver sends `VERB_AUDIO_TRACK` with that index and the command id, and the selection
/// settles only when the host's `PLAYER_TRACK` echoes the command with that index. A refusal (`b = -1`), another
/// index, an unanswered request and a refused verb are the command's typed refusal; a stream the map does not name
/// is `Unsupported` and asks nothing.
#[test]
fn the_selected_audio_stream_is_asked_of_the_host_and_settles_on_its_answer() {
    let (mut driver, fake) = rig();
    let unmapped = TrackSelection { kind: TrackKind::Audio, id: 31342 };
    let (a, epoch) = prepared_with(&mut driver, &fake, SECOND, 1, german(), &["eng", "ger"]);
    assert!(driver.maps_track(SECOND) && !driver.maps_track(unmapped));

    assert_eq!(dispatch(&mut driver, a, 3, pick(31343)), DispatchResult::Queued);
    assert_eq!(fake.verbs(), [Verb::AudioTrack { index: 1, command: 3 }]);
    assert!(poll(&mut driver).is_empty(), "nothing settles before the host answers");
    fake.post(epoch, PlayerKind::Track, 3, 1);
    assert_eq!(poll(&mut driver), [
        observation(a, Observation::TrackSelected(seen(SECOND))),
        DriverEvent::TrackSettled { attempt: a, command: cmd(3), kind: TrackKind::Audio, choice: TrackChoice::Selected(31343) },
    ]);

    // The host refused it, or answered with another index: the command's refusal, and the attempt keeps playing.
    for (command, b) in [(4, -1), (5, 0)] {
        assert_eq!(dispatch(&mut driver, a, command, pick(31343)), DispatchResult::Queued);
        fake.post(epoch, PlayerKind::Track, command as i64, b);
        assert_eq!(poll(&mut driver), [DriverEvent::ControlRefused { attempt: a, command: cmd(command), operation: pick(31343),
            failure: Failure::Refused }], "b = {b}");
    }

    // No answer by the control deadline.
    assert_eq!(dispatch(&mut driver, a, 6, pick(31343)), DispatchResult::Queued);
    fake.advance(TIMING.control_timeout_ms);
    assert_eq!(poll(&mut driver), [DriverEvent::ControlRefused { attempt: a, command: cmd(6), operation: pick(31343),
        failure: Failure::TimedOut }]);

    // The host refused the verb at the door.
    fake.with(|s| s.refuse_verb = Some(Refusal::Busy));
    assert_eq!(dispatch(&mut driver, a, 7, pick(31343)), DispatchResult::Refused(Failure::Busy));
    fake.with(|s| s.refuse_verb = None);
    assert!(poll(&mut driver).is_empty(), "a refused verb leaves nothing pending");

    // A stream the map does not name, and a subtitle, ask nothing of the host.
    let asked = fake.verbs().len();
    assert_eq!(dispatch(&mut driver, a, 8, pick(31342)), DispatchResult::Refused(Failure::Unsupported));
    assert_eq!(dispatch(&mut driver, a, 9, Operation::SelectTrack { kind: TrackKind::Subtitle, choice: TrackChoice::Selected(1) }),
        DispatchResult::Refused(Failure::Unsupported));
    assert_eq!(fake.verbs().len(), asked);
}

/// **The track is chosen by its language, not trusted by its position** (review of #1566: LG's service ordered a
/// four-track file differently from PMS). The ordinal is used when its native track has the stream's language;
/// otherwise the ONE native track with that language; otherwise (several, none, or no language known on either
/// side) the request is refused before the host is asked, so a wrong track never settles as the viewer's pick.
#[test]
fn the_native_track_is_the_one_whose_language_is_the_streams() {
    for (native, ordinal, want) in [
        (&["eng", "ger"][..], 1, Some(1)),        // the ordinal's language matches
        (&["ger", "eng"][..], 1, Some(0)),        // it does not; one other track has it
        (&["eng", "ger", "ger"][..], 1, Some(1)), // several have it, the ordinal among them
        (&["ger", "ger", "eng"][..], 2, None),    // several, the ordinal not among them: ambiguous
        (&["eng", "fre"][..], 1, None),           // none has it
        (&["eng", ""][..], 1, None),              // the ordinal's language is unknown and none other has it
        (&["eng", "und"][..], 1, None),           // "und" is no language
        (&[][..], 1, None),                       // the player reported no tracks
    ] {
        let (mut driver, fake) = rig();
        let (a, _) = prepared_with(&mut driver, &fake, SECOND, ordinal, german(), native);
        let answer = dispatch(&mut driver, a, 2, pick(31343));
        match want {
            Some(index) => {
                assert_eq!(answer, DispatchResult::Queued, "{native:?} at {ordinal}");
                assert_eq!(fake.verbs(), [Verb::AudioTrack { index, command: 2 }], "{native:?} at {ordinal}");
            }
            None => {
                assert_eq!(answer, DispatchResult::Refused(Failure::Unsupported), "{native:?} at {ordinal}");
                assert!(fake.verbs().is_empty(), "nothing asked of the host for {native:?} at {ordinal}");
            }
        }
    }
    // A stream whose own language PMS does not state cannot be told apart either.
    for unknown in ["", "und"] {
        let (mut driver, fake) = rig();
        let (a, _) = prepared_with(&mut driver, &fake, SECOND, 1, NativeTrackCheck::Language(unknown.into()), &["eng", "ger"]);
        assert_eq!(dispatch(&mut driver, a, 2, pick(31343)), DispatchResult::Refused(Failure::Unsupported), "{unknown:?}");
    }
}

/// **A sole audio stream is settled by construction, with no host call** (review round 2 of #1566). The player plays
/// the only track it has, so asking it is no evidence and a firmware that reports no track in READY (a count of 0, a
/// failed read, a missing `player_select_track`) would answer -1 and halt the start of every single-audio title, the
/// kind that already plays on the retail set. It settles as main settled it: `TrackSelected` and `TrackSettled` at
/// once, no `VERB_AUDIO_TRACK`. A `Sole` check at any ordinal but 0 is no single stream and is refused.
#[test]
fn a_sole_audio_stream_settles_by_construction_without_asking_the_host() {
    let first = TrackSelection { kind: TrackKind::Audio, id: 31342 };
    let (mut driver, fake) = rig();
    let (a, _) = prepared_with(&mut driver, &fake, first, 0, NativeTrackCheck::Sole, &[]);
    assert_eq!(dispatch(&mut driver, a, 2, pick(31342)), DispatchResult::Queued);
    assert!(fake.verbs().is_empty(), "no verb for a sole stream: {:?}", fake.verbs());
    assert_eq!(poll(&mut driver), [
        observation(a, Observation::TrackSelected(seen(first))),
        DriverEvent::TrackSettled { attempt: a, command: cmd(2), kind: TrackKind::Audio, choice: TrackChoice::Selected(31342) },
    ]);

    let (mut driver, fake) = rig();
    let (a, _) = prepared_with(&mut driver, &fake, first, 1, NativeTrackCheck::Sole, &[]);
    assert_eq!(dispatch(&mut driver, a, 2, pick(31342)), DispatchResult::Refused(Failure::Unsupported));
    assert!(fake.verbs().is_empty());
}

/// PMS states a stream's language as ISO 639-2 (`ger`, `fre`, `cze`: the B codes Plex writes) and a container can
/// tag it as ISO 639-1 (`de`) or 639-2/T (`deu`, `fra`, `ces`): the driver folds both sides through the shared
/// table (`plx_data::metadata::lang_matches`), so every spelling of one language is one language.
#[test]
fn every_spelling_of_one_language_is_one_language() {
    for (pms, native) in [("ger", "de"), ("ger", "deu"), ("deu", "ger"), ("fre", "fr"), ("fre", "fra"), ("eng", "en"),
        ("cze", "ces"), ("dut", "nld"), ("chi", "zho"), ("ENG", "eng")] {
        let (mut driver, fake) = rig();
        let (a, _) = prepared_with(&mut driver, &fake, SECOND, 1, NativeTrackCheck::Language(pms.into()), &["xxx", native]);
        assert_eq!(dispatch(&mut driver, a, 2, pick(31343)), DispatchResult::Queued, "{pms} ~ {native}");
        assert_eq!(fake.verbs(), [Verb::AudioTrack { index: 1, command: 2 }], "{pms} ~ {native}");
    }
    let (mut driver, fake) = rig();
    let (a, _) = prepared_with(&mut driver, &fake, SECOND, 1, german(), &["xxx", "dan"]);
    assert_eq!(dispatch(&mut driver, a, 2, pick(31343)), DispatchResult::Refused(Failure::Unsupported), "ger is not dan");
}

/// A track request the host still holds when the player closes is answered as refused before the release.
#[test]
fn a_track_request_unanswered_at_the_close_is_refused_before_the_release() {
    let (mut driver, fake) = rig();
    let (a, epoch) = prepared_with(&mut driver, &fake, SECOND, 1, german(), &["eng", "ger"]);
    assert_eq!(dispatch(&mut driver, a, 2, pick(31343)), DispatchResult::Queued);
    fake.post(epoch, PlayerKind::Closed, 0, 0);
    assert_eq!(poll(&mut driver), [
        DriverEvent::ControlRefused { attempt: a, command: cmd(2), operation: pick(31343), failure: Failure::Refused },
        DriverEvent::Release { attempt: a, status: ReleaseStatus::Released },
    ]);
}

#[test]
fn a_host_queue_overflow_fails_the_attempt() {
    let (mut driver, fake) = rig();
    let (a, _) = prepared(&mut driver, &fake, 1);
    fake.with(|s| s.overflow = true);
    assert_eq!(poll(&mut driver), [observation(a, Observation::Failed(Failure::Backend))]);
    assert_eq!(fake.verbs(), [Verb::Close]);
}

#[test]
fn the_capabilities_are_progressive_pause_and_seek_and_the_display_rests_on_the_host_offering_a_player() {
    let (driver, fake) = rig();
    assert_eq!(driver.capabilities(BackendKind::Uri), Some(CAPS));
    assert_eq!(driver.capabilities(BackendKind::Feed), None);
    assert!(CAPS.progressive_uri && CAPS.pause && CAPS.seek);
    assert!(!CAPS.hls_uri && !CAPS.packet_feed && !CAPS.audio_off);
    assert!(CAPS.subtitle_track, "a text subtitle is selected on the host and its cues drawn by the app");
    assert!(CAPS.subtitle_off, "the host player draws no subtitle, so Off holds by construction");
    assert!(driver.delivers_captions(), "the app draws the subtitle from the host's cues");
    assert_eq!(driver.display().tag, DISPLAY_TAG);
    assert!(driver.display().is_ready() && !driver.display().preview_rectangle);
    fake.with(|s| s.unavailable = Some("no player"));
    assert_eq!(driver.display().readiness, DisplayReadiness::NotReady(plx_playback_contract::uri::UriNotReady::DisplayUnavailable));
    let mut driver = driver;
    assert_eq!(driver.ready_to_start(&mt()).map_err(|e| e.log_name()), Err("the host has no media player"));
    let a = attempt(1);
    let key = SourceKey::new(1);
    assert_eq!(driver.stage_source(a, key, UriSource::new(URL.into(), vec![])), Ok(()));
    let start = Operation::Start { source: key, requirements: plain_requirements() };
    assert_eq!(dispatch(&mut driver, a, 1, start), DispatchResult::Refused(Failure::Unavailable));
}

/// The shared suite over the Tizen driver: its harness feeds the driver's fake host port.
struct TizenHarness {
    driver: TizenUriDriver<Fake>,
    fake: Fake,
}

impl UriHarness for TizenHarness {
    type Driver = TizenUriDriver<Fake>;
    fn driver(&mut self) -> &mut Self::Driver {
        &mut self.driver
    }
    fn native_prepared(&mut self) {
        self.fake.post(self.fake.epoch(), PlayerKind::Prepared, 1_000, 0);
    }
    fn native_closed(&mut self) {
        self.fake.post(self.fake.epoch(), PlayerKind::Closed, 0, 0);
    }
}

#[test]
fn the_tizen_driver_conforms_to_the_shared_uri_suite() {
    conforms(|| {
        let (driver, fake) = rig();
        TizenHarness { driver, fake }
    });
}

// ---- the port over the contract's fake host table ----

/// **The port wired to the hosted contract.** A real `Hosted` over the fake host table: the driver's open reaches
/// the host's `player_open` with the URL it copied, the host's events (posted through `plx_host_post`) come back
/// through the library's player queue as the contract's observations, a Stop is the host's CLOSE verb and the
/// host's `CLOSED` the release; then, after the host stopped the library, a request is refused without calling the
/// host (as `Stopping`), the host's late completion is refused at the door, and the close, which can no longer be asked,
/// is the host's to make at terminate: the release is reported and the exit is clean (a stop with a player open ends
/// `on_stopped` clean).
#[test]
fn the_driver_over_the_hosted_contract_round_trips_and_a_close_after_the_library_stopped_is_the_hosts_to_make() {
    use crate::hosted::testing::{complete, loop_until_quit, seen as host_saw, World};
    let w = World::new();
    assert_eq!(w.start(|_| Ok(()), loop_until_quit(w.hosted, complete())), host_abi::OK);
    let timing = Timing { release_allowance_ms: 40, ..Timing::PRODUCT };
    let mut driver = TizenUriDriver::new(HostedPlayer::new(w.hosted), timing);
    assert!(driver.display().is_ready());
    let a = attempt(1);
    let key = SourceKey::new(1);
    assert_eq!(driver.stage_source(a, key, UriSource::new(URL.into(), vec![])), Ok(()));
    let start = Operation::Start { source: key, requirements: plain_requirements() };
    assert_eq!(dispatch_any(&mut driver, a, 1, start), DispatchResult::Queued);
    let (epoch, uri, area) = host_saw(|s| s.opens.last().cloned()).expect("the host was asked to open");
    assert_eq!((uri.as_str(), area), (URL, FULL_WINDOW));
    let event = |kind, a, b| crate::host_abi::PlxHostEvent { size: host_abi::EVENT_V2_SIZE, kind, epoch, reserved: 0, a, b };
    assert_eq!(w.hosted.post(&event(host_abi::EVENT_PLAYER_PREPARED, 600_000, 0)), host_abi::OK);
    assert_eq!(poll_any(&mut driver), [observation(a, Observation::Playback(seen(Playback::ReadyAwaitPlay)))]);
    assert_eq!(dispatch_any(&mut driver, a, 2, Operation::Seek(ContentTime::from_millis(7_000))), DispatchResult::Queued);
    assert_eq!(host_saw(|s| s.verbs.last().copied()), Some((epoch, host_abi::VERB_SEEK, 7_000, 2)));
    assert_eq!(w.hosted.post(&event(host_abi::EVENT_PLAYER_SEEK_DONE, 2, 7_000)), host_abi::OK);
    assert_eq!(poll_any(&mut driver), [DriverEvent::SeekLanded { attempt: a, command: cmd(2),
        position: Some(seen(ContentTime::from_millis(7_000))) }]);
    // A Stop is the host's CLOSE verb; its CLOSED is the release, and a completion after it is refused at the door.
    assert_eq!(dispatch_any(&mut driver, a, 3, Operation::Stop(StopReason::Stop)), DispatchResult::Queued);
    assert_eq!(host_saw(|s| s.verbs.last().copied()), Some((epoch, host_abi::VERB_CLOSE, 0, 0)));
    assert_eq!(w.hosted.post(&event(host_abi::EVENT_PLAYER_CLOSED, 0, 0)), host_abi::OK);
    assert_eq!(w.hosted.post(&event(host_abi::EVENT_PLAYER_COMPLETED, 0, 0)), host_abi::E_STALE);
    assert_eq!(poll_any(&mut driver), [DriverEvent::Release { attempt: a, status: ReleaseStatus::Released }]);
    assert_eq!(driver.exit_disposition(), ExitDisposition::Clean);
    // A second attempt, then the app stops: the close request is refused by the gate, so no CLOSED can come.
    let b = attempt(2);
    assert_eq!(driver.stage_source(b, key, UriSource::new(URL.into(), vec![])), Ok(()));
    assert_eq!(dispatch_any(&mut driver, b, 4, Operation::Start { source: key, requirements: plain_requirements() }), DispatchResult::Queued);
    let second = host_saw(|s| s.opens.last().map(|o| o.0)).unwrap();
    let verbs_before = host_saw(|s| s.verbs.len());
    w.hosted.stop(std::time::Duration::from_secs(30));
    assert_eq!(dispatch_any(&mut driver, b, 5, Operation::Stop(StopReason::Exit)), DispatchResult::Queued);
    assert_eq!(host_saw(|s| s.verbs.len()), verbs_before, "the host was not called after the stop");
    let late = crate::host_abi::PlxHostEvent { size: host_abi::EVENT_V2_SIZE, kind: host_abi::EVENT_PLAYER_CLOSED, epoch: second, reserved: 0, a: 0, b: 0 };
    assert_eq!(w.hosted.post(&late), host_abi::OK, "the host's CLOSED is still heard while the loop runs (the attempt is released already, so the machine drops it)");
    // The library cannot ask for the close any more (its gate is shut), and the host closes its own player at terminate
    // (`plx_host.h`, STOP): the stop is not held for a confirmation that cannot come, so the exit is clean.
    assert_eq!(poll_any(&mut driver), [DriverEvent::Release { attempt: b, status: ReleaseStatus::Released }]);
    assert_eq!(driver.exit_disposition(), ExitDisposition::Clean);
    w.wait_stopped(1);
}

/// **A CLOSE the host accepted just before the library was stopped is confirmed by the `CLOSED` that arrives just after.** The
/// close was asked and accepted while the gate was open; `plx_host_stop` then closed it, and the host's `PLAYER_CLOSED` (posted by
/// its video thread, or by `PlxPlayer.Shutdown`'s own close) was refused with `E_STATE`, so the attempt stayed `Closing` and the stop
/// held for `STOP_DEADLINE` and reported unclean. That event is the confirmation the stop waits for: it is heard for a live epoch
/// while the loop is still running, and every other kind is still refused.
#[test]
fn a_close_accepted_before_the_stop_is_confirmed_by_the_closed_that_arrives_after_it() {
    use crate::hosted::testing::{complete, loop_until_quit, seen as host_saw, World};
    let w = World::new();
    assert_eq!(w.start(|_| Ok(()), loop_until_quit(w.hosted, complete())), host_abi::OK);
    let timing = Timing { release_allowance_ms: 40, ..Timing::PRODUCT };
    let mut driver = TizenUriDriver::new(HostedPlayer::new(w.hosted), timing);
    let a = attempt(1);
    let key = SourceKey::new(1);
    assert_eq!(driver.stage_source(a, key, UriSource::new(URL.into(), vec![])), Ok(()));
    assert_eq!(dispatch_any(&mut driver, a, 1, Operation::Start { source: key, requirements: plain_requirements() }), DispatchResult::Queued);
    let epoch = host_saw(|s| s.opens.last().map(|o| o.0)).unwrap();
    let event = |kind| crate::host_abi::PlxHostEvent { size: host_abi::EVENT_V2_SIZE, kind, epoch, reserved: 0, a: 0, b: 0 };
    // The close is asked and ACCEPTED (the gate is open); then the host stops the library.
    assert_eq!(dispatch_any(&mut driver, a, 2, Operation::Stop(StopReason::Exit)), DispatchResult::Queued);
    assert_eq!(host_saw(|s| s.verbs.last().copied()), Some((epoch, host_abi::VERB_CLOSE, 0, 0)));
    w.hosted.stop(std::time::Duration::from_secs(30));
    // Its CLOSED arrives after the stop: heard (it is the confirmation), where a completion of the same epoch is not.
    assert_eq!(w.hosted.post(&event(host_abi::EVENT_PLAYER_COMPLETED)), host_abi::E_STATE, "any other kind is refused after the stop");
    assert_eq!(w.hosted.post(&event(host_abi::EVENT_PLAYER_CLOSED)), host_abi::OK, "the confirmation the stop waits for");
    assert_eq!(poll_any(&mut driver), [DriverEvent::Release { attempt: a, status: ReleaseStatus::Released }],
        "released by the host's own confirmation, not at the deadline");
    assert_eq!(driver.exit_disposition(), ExitDisposition::Clean);
    w.wait_stopped(1);
}

fn dispatch_any<P: HostPlayer>(driver: &mut TizenUriDriver<P>, attempt: AttemptId, command: u64, op: Operation) -> DispatchResult {
    driver.dispatch(BackendKind::Uri, &mt(), attempt, cmd(command), op)
}

fn poll_any<P: HostPlayer>(driver: &mut TizenUriDriver<P>) -> Vec<DriverEvent> {
    driver.poll(&mt()).into_iter().map(|e: BackendEvent| e.event).collect()
}

// ---- the composition ----

fn new_adapter() -> PlayerAdapter {
    PlayerAdapter::new(mt())
}

#[test]
fn the_composition_installs_the_driver_and_returns_a_resolver_only_when_it_did() {
    let fake = Fake::default();
    let mut adapter = new_adapter();
    let composition = compose_over(&mut adapter, false, fake.clone());
    assert!(composition.resolver.is_some(), "an installed driver has a resolver");
    assert_eq!(adapter.composed_backends(), BackendSet::URI, "the URI backend and no Feed");
    let evidence = composition.resolver.unwrap().evidence();
    assert_eq!((evidence.callable, evidence.certified), (CAPS, CAPS), "the driver certifies what it can call");
    assert!(evidence.display.is_ready() && evidence.display.tag == DISPLAY_TAG);
    // A second composition onto the same adapter is refused: no resolver for a driver that is not installed.
    assert!(compose_over(&mut adapter, false, fake).resolver.is_none());
}

#[test]
fn a_host_with_no_player_or_a_controlled_run_composes_nothing() {
    let fake = Fake::default();
    fake.with(|s| s.unavailable = Some("the host's table has no player group (version 1)"));
    let mut adapter = new_adapter();
    assert!(compose_over(&mut adapter, false, fake).resolver.is_none());
    assert_eq!(adapter.composed_backends(), BackendSet::NONE);
    let mut adapter = new_adapter();
    assert!(compose_over(&mut adapter, true, Fake::default()).resolver.is_none());
    assert_eq!(adapter.composed_backends(), BackendSet::NONE, "a controlled run touches no native port");
}

// ---- what the platform certifies ----

fn plays() -> StandardDirectPlay {
    StandardDirectPlay { video: true, dolby_vision: true, audio: true, container: true }
}

fn request(standard: StandardDirectPlay, audio_sid: i64, audio_ordinal: Option<i64>) -> OriginalRequest {
    use plx_media::route::SourceSelection;
    OriginalRequest {
        selection: SourceSelection { server: plx_plex::plex::ServerId::UNSET, part_id: 3134, audio_sid, subtitle_sid: 0,
            audio_contract: Default::default() },
        server_origin: plx_plex::plex::Origin::http("10.0.2.2", 32499),
        source_url: URL.to_owned(),
        duration_ms: Some(450_000),
        video_codec: "h264".into(),
        dolby_vision: plx_data::metadata::Dovi::default(),
        dolby_vision_mode: Default::default(),
        dolby_vision_base_layer_only: false,
        uri_opt_in: false,
        standard,
        audio_codec: "aac".into(),
        audio_ordinal,
        has_multiple_audio_tracks: false,
        audio_language: String::new(),
        subtitle_source: None,
        subtitle_streams: Default::default(),
        subtitle_choice: TrackChoice::Unspecified,
        source_kbps: None,
        raster: None,
        requested_rate_permille: 1000,
        resume_ns: 0,
        quality_allows_original: true,
        link_allows_original: true,
        auto_allows_original: true,
        requires_subtitle_burn: false,
        requires_client_subtitle: false,
        preview: false,
    }
}

/// The resolver exactly as the composition builds it, for the viewer's setting OFF (`uri_opt_in` false).
fn resolver() -> ResolvePort {
    let (driver, _) = rig();
    ResolvePort::new(ProductEvidence { display: driver.display(), callable: CAPS, certified: CAPS }, Arc::new(TizenQualifier))
}

fn off() -> plx_base::task::OffFrame {
    plx_base::task::OffFrame::for_test()
}

/// **Planner level, through the Tizen composition's own resolver**: an ordinary h264/aac direct-play title that
/// selects the first audio stream resolves to a certified URL carrier with the viewer's opt-in off, the carrier
/// names the progressive source the driver can call, and the driver accepts exactly that requirement.
/// (The route's own end-to-end rig, W6c, is private to `plx_media`; this is the resolver-level half, and what the
/// planner does with a `Known` verdict is the route's tested behaviour.)
#[test]
fn an_ordinary_direct_play_title_resolves_to_the_tizen_uri_offer() {
    let Verdict::Known(known) = resolver().qualify(&off(), request(plays(), 31342, Some(0))) else {
        panic!("an ordinary title is certified");
    };
    assert_eq!(known.requirements.source, plx_playback_contract::SourceKind::ProgressiveUri);
    assert_eq!(known.requirements.audio_track, TrackChoice::Selected(31342));
    assert!(CAPS.supports(known.requirements), "the driver can call what the offer requires");
    // The same driver takes the offer's source and start.
    let (mut driver, _) = rig();
    let key = SourceKey::new(1);
    assert_eq!(driver.stage_source(attempt(1), key, known.source), Ok(()));
    let start = Operation::Start { source: key, requirements: known.requirements };
    assert_eq!(dispatch(&mut driver, attempt(1), 1, start), DispatchResult::Queued);
    // The viewer's opt-in changes nothing: the driver certifies itself.
    let mut opted = request(plays(), 31342, Some(0));
    opted.uri_opt_in = true;
    assert!(matches!(resolver().qualify(&off(), opted), Verdict::Known(_)));
}

/// A source Standard would not direct-play (DTS or TrueHD audio the set cannot play natively, an unstreamable
/// container) is declined with the typed reason, and no converted PMS output is certified: the planner then has no
/// qualified backend and says so, instead of falling to a Feed that is not there.
#[test]
fn a_dts_or_non_direct_play_source_is_refused_with_its_typed_reason_and_no_conversion_is_certified() {
    let no_audio = StandardDirectPlay { audio: false, ..plays() };
    let no_container = StandardDirectPlay { container: false, ..plays() };
    let no_video = StandardDirectPlay { video: false, ..plays() };
    for (standard, why) in [(no_audio, UriDecline::StandardAudio), (no_container, UriDecline::StandardContainer),
        (no_video, UriDecline::StandardVideo)] {
        let verdict = resolver().qualify(&off(), request(standard, 31342, Some(0)));
        assert!(matches!(verdict, Verdict::Unknown(got) if got == why), "{why:?}");
    }
    let tizen: &dyn OriginalQualifier = &TizenQualifier;
    let selection = request(plays(), 31342, Some(0)).selection;
    for recipe in [PmsRecipe::Remux, PmsRecipe::Encode, PmsRecipe::Hls] {
        let pms = PmsRequest { selection, source_url: URL.to_owned(), resource_session: "session".into(), duration_ms: Some(450_000),
            resume_ns: 0, recipe, adaptive: false, video_codec: "h264".into(), audio_output: PmsAudioOutput::Codec("aac".into()),
            source_dovi: plx_data::metadata::Dovi::default(), output_dovi: None, dolby_vision_mode: Default::default(),
            applied_audio: Default::default(), subtitle_choice: TrackChoice::Unspecified, requires_client_subtitle: false,
            burned_subtitle: false, accepted_burn_sid: None, requested_rate_permille: 1000, preview: false };
        assert!(matches!(tizen.qualify_pms(&off(), pms), PmsVerdict::Unknown), "{}", match recipe {
            PmsRecipe::Remux => "remux", PmsRecipe::Encode => "encode", PmsRecipe::Hls => "hls" });
    }
}

/// **A source with several audio streams is certified for the one the play selects** (discussion #1460: every multi-audio
/// title was refused on the first retail set). The offer's track map names that stream at its position among the Part's
/// audio streams (`audio_ordinal`), which is the index the driver asks the host player to select. A play that selects a
/// stream whose position is unknown is still declined: there would be no index to ask for.
#[test]
fn a_source_with_several_audio_streams_is_certified_for_the_selected_stream() {
    for (sid, ordinal) in [(31342, 0), (31343, 1), (31345, 3)] {
        let mut req = request(plays(), sid, Some(ordinal));
        req.has_multiple_audio_tracks = true;
        req.audio_language = "ger".into();
        let Verdict::Known(known) = resolver().qualify(&off(), req) else { panic!("audio {sid} at {ordinal} is certified") };
        assert_eq!(known.requirements.audio_track, TrackChoice::Selected(sid as u64));
        let selection = TrackSelection { kind: TrackKind::Audio, id: sid as u64 };
        assert_eq!(known.source.check_of(selection), Some(&NativeTrackCheck::Language("ger".into())),
            "a multi-audio offer carries the stream's language for the driver to match");
        let (_, tracks) = known.source.into_parts();
        assert_eq!(tracks, [NativeTrackIndex { selection: TrackSelection { kind: TrackKind::Audio, id: sid as u64 }, index: ordinal }]);
    }
    for multiple in [false, true] {
        let mut unknown = request(plays(), 31343, None);
        unknown.has_multiple_audio_tracks = multiple;
        assert!(matches!(resolver().qualify(&off(), unknown), Verdict::Unknown(UriDecline::NotCertified)),
            "a selected stream with no position (multiple {multiple})");
    }
    // No selected stream at all (a silent file) needs no track.
    assert!(matches!(resolver().qualify(&off(), request(plays(), 0, None)), Verdict::Known(_)));
    // A single-stream offer says so: its ordinal is the track, with no language to match.
    let Verdict::Known(single) = resolver().qualify(&off(), request(plays(), 31342, Some(0))) else { panic!("one stream") };
    assert_eq!(single.source.check_of(TrackSelection { kind: TrackKind::Audio, id: 31342 }), Some(&NativeTrackCheck::Sole));
}

/// The emulator has no audio device, so its runs serve VIDEO-ONLY media (the mock's `--extra-media-video-only`). The planner
/// states such a source as `audio_sid = 0`, no ordinal, an empty audio codec and `StandardDirectPlay.audio = true` (`plan.rs`:
/// no track list yields `(-1, "", 0)` and "no codec at all is a file with no audio"). That source is certified, its offer
/// selects no track (so the startup plan issues no `SelectTrack`), and the driver accepts it and starts it.
#[test]
fn a_video_only_source_is_certified_and_started_with_no_track_to_settle() {
    let mut silent = request(plays(), 0, None);
    silent.audio_codec = String::new();
    let Verdict::Known(known) = resolver().qualify(&off(), silent) else { panic!("a silent file is certified") };
    assert_eq!(known.requirements.audio_track, TrackChoice::Unspecified, "nothing to select");
    assert!(CAPS.supports(known.requirements));
    let (mut driver, _) = rig();
    let key = SourceKey::new(1);
    assert_eq!(driver.stage_source(attempt(1), key, known.source), Ok(()));
    let start = Operation::Start { source: key, requirements: known.requirements };
    assert_eq!(dispatch(&mut driver, attempt(1), 1, start), DispatchResult::Queued);
}

/// **A title with subtitle streams and none selected plays.** The planner sets `subtitle_choice: Off` whenever the item has
/// ANY subtitle stream and none is selected, which on a real library is most titles: the first retail report (a 2023
/// UA65CU7700) logged that refusal for every title but the one without subtitles. The host player is never given a
/// subtitle, so the driver states `subtitle_off` ([`CAPS`]) and the resolver lets Off through: the offer certifies it,
/// and the driver starts it and settles the startup `SelectTrack(Subtitle, Off)` without a host call. The other viewer
/// refusals still end the play before the qualifier is asked.
#[test]
fn a_title_with_subtitle_streams_and_none_selected_is_certified_and_started() {
    let mut off_choice = request(plays(), 31342, Some(0));
    off_choice.subtitle_choice = TrackChoice::Off;
    let Verdict::Known(known) = resolver().qualify(&off(), off_choice) else { panic!("subtitles off is certified") };
    assert_eq!(known.requirements.subtitle_track, TrackChoice::Off);
    assert!(CAPS.subtitle_off && CAPS.supports(known.requirements));
    let (mut driver, fake) = rig();
    let a = attempt(1);
    let key = SourceKey::new(1);
    assert_eq!(driver.stage_source(a, key, known.source), Ok(()));
    let start = Operation::Start { source: key, requirements: known.requirements };
    assert_eq!(dispatch(&mut driver, a, 1, start), DispatchResult::Queued);
    let off_pick = Operation::SelectTrack { kind: TrackKind::Subtitle, choice: TrackChoice::Off };
    assert_eq!(dispatch(&mut driver, a, 2, off_pick), DispatchResult::Refused(Failure::Busy), "not before PREPARED");
    fake.post(fake.epoch(), PlayerKind::Prepared, 1_000, 0);
    poll(&mut driver);
    assert_eq!(dispatch(&mut driver, a, 3, off_pick), DispatchResult::Queued);
    assert_eq!(poll(&mut driver), [
        DriverEvent::TrackSettled { attempt: a, command: cmd(3), kind: TrackKind::Subtitle, choice: TrackChoice::Off },
    ]);
    assert!(!fake.verbs().iter().any(|v| !matches!(v, Verb::Play)), "no subtitle verb was asked of the host: {:?}", fake.verbs());
}

/// The viewer refusals that still end a play before the qualifier is asked: with no Feed these end as
/// `NoQualifiedBackend`.
#[test]
fn the_viewer_refusals_end_the_play_before_the_qualifier_is_asked() {
    for (name, tweak, why) in [
        ("client subtitle", (|r: &mut OriginalRequest| r.requires_client_subtitle = true) as fn(&mut OriginalRequest),
            UriDecline::ClientSubtitle),
        ("relay", |r| r.link_allows_original = false, UriDecline::Relay),
        ("auto on a non-local link", |r| r.auto_allows_original = false, UriDecline::AutoOnNonLocalLink),
        ("burned subtitle", |r| r.requires_subtitle_burn = true, UriDecline::SubtitleBurn),
    ] {
        let mut req = request(plays(), 31342, Some(0));
        tweak(&mut req);
        let verdict = resolver().qualify(&off(), req);
        assert!(matches!(verdict, Verdict::Refused(got) if got == why), "{name}");
    }
    let mut boost = request(plays(), 31342, Some(0));
    boost.selection.audio_contract = plx_plex::plex::AudioEnhancements { boost_dialog: true, normalize_loudness: false };
    let verdict = resolver().qualify(&off(), boost);
    assert!(matches!(verdict, Verdict::Refused(UriDecline::AudioEnhancement)), "boost dialog");
}

fn text_sub(sid: u64, ordinal: i64, codec: &str, language: &str) -> SourceSubtitleStream {
    SourceSubtitleStream { sid, ordinal, codec: codec.into(), language: language.into() }
}

/// A play that selects the embedded subtitle `sid` among `streams` (the reproduction: a server-selected SubRip track).
fn subtitled(streams: Vec<SourceSubtitleStream>, sid: u64) -> OriginalRequest {
    let codec = streams.iter().find(|s| s.sid == sid).map_or_else(String::new, |s| s.codec.clone());
    let mut req = request(plays(), 31342, Some(0));
    req.subtitle_choice = TrackChoice::Selected(sid);
    req.selection.subtitle_sid = sid as i64;
    req.subtitle_source = Some(SourceSubtitle { codec, embedded: true });
    req.subtitle_streams = streams.into();
    req
}

/// **A selected embedded text subtitle is offered** (it was declined with `SubtitleNotCarried` before, so every
/// title with a server-selected subtitle stayed off the URI engine). Every embedded subtitle stream of the Part is mapped
/// at its ordinal with its identity check, the selected one is the requirement, and the certificate carries its render.
/// A Part with one subtitle stream maps it `Sole`; with several, each by its language.
#[test]
fn a_selected_embedded_text_subtitle_is_offered_with_every_stream_mapped() {
    let Verdict::Known(known) = resolver().qualify(&off(), subtitled(vec![text_sub(9, 0, "srt", "eng")], 9)) else {
        panic!("a sole SubRip track is certified");
    };
    assert_eq!(known.requirements.subtitle_track, TrackChoice::Selected(9));
    assert!(CAPS.supports(known.requirements));
    let nine = TrackSelection { kind: TrackKind::Subtitle, id: 9 };
    assert_eq!(known.source.check_of(nine), Some(&NativeTrackCheck::Sole));
    assert_eq!(known.source.check_of(TrackSelection { kind: TrackKind::Audio, id: 31342 }), Some(&NativeTrackCheck::Sole),
        "the audio stream's check survives the subtitle map");
    assert_eq!(known.certified_subtitle_source, Some(SourceSubtitle { codec: "srt".into(), embedded: true }));

    let three = vec![text_sub(11, 0, "srt", "eng"), text_sub(12, 1, "srt", "fre"), text_sub(13, 2, "ass", "ger")];
    let Verdict::Known(known) = resolver().qualify(&off(), subtitled(three, 12)) else { panic!("text siblings") };
    assert_eq!(known.source.check_of(TrackSelection { kind: TrackKind::Subtitle, id: 12 }),
        Some(&NativeTrackCheck::Language("fre".into())));
    let (_, tracks) = known.source.into_parts();
    let subs: Vec<_> = tracks.iter().filter(|t| t.selection.kind == TrackKind::Subtitle).map(|t| (t.selection.id, t.index)).collect();
    assert_eq!(subs, [(11, 0), (12, 1), (13, 2)], "every stream is mapped, so a pick mid-play can be asked for");

    // Subtitles Off on such a Part still maps them: a later pick has its index.
    let mut off_choice = request(plays(), 31342, Some(0));
    off_choice.subtitle_choice = TrackChoice::Off;
    off_choice.subtitle_streams = vec![text_sub(9, 0, "srt", "eng")].into();
    let Verdict::Known(known) = resolver().qualify(&off(), off_choice) else { panic!("off is certified") };
    assert_eq!(known.source.check_of(nine), Some(&NativeTrackCheck::Sole));
    assert_eq!(known.certified_subtitle_source, None, "nothing selected, nothing to render");
}

/// **A picture subtitle, an unmeasured codec, or no stream list keeps a selected subtitle declined.** PGS and VobSub
/// carry no cue text, and whether the player counts one among its TEXT tracks is not measured, so a sibling of that kind
/// leaves a text track's native index unproven; `mov_text` was not measured raising cues. A burned subtitle is still
/// refused before the qualifier.
#[test]
fn a_picture_or_unmeasured_subtitle_keeps_the_selected_subtitle_declined() {
    for (streams, sid, why) in [
        (vec![text_sub(9, 0, "pgs", "eng")], 9, "a selected PGS track"),
        (vec![text_sub(9, 0, "srt", "eng"), text_sub(10, 1, "pgs", "eng")], 9, "a SubRip track with a PGS sibling"),
        (vec![text_sub(9, 0, "vobsub", "eng"), text_sub(10, 1, "srt", "eng")], 10, "a SubRip track with a VobSub sibling"),
        (vec![text_sub(9, 0, "mov_text", "eng")], 9, "mov_text, not measured"),
        (vec![], 9, "no stream list"),
    ] {
        let verdict = resolver().qualify(&off(), subtitled(streams, sid));
        assert!(matches!(verdict, Verdict::Unknown(UriDecline::SubtitleNotCarried)), "{why}");
    }
    let mut burned = subtitled(vec![text_sub(9, 0, "srt", "eng")], 9);
    burned.requires_subtitle_burn = true;
    assert!(matches!(resolver().qualify(&off(), burned), Verdict::Refused(UriDecline::SubtitleBurn)));
}

const SUB_EN: TrackSelection = TrackSelection { kind: TrackKind::Subtitle, id: 11 };
const SUB_FR: TrackSelection = TrackSelection { kind: TrackKind::Subtitle, id: 12 };

fn pick_sub(id: u64) -> Operation {
    Operation::SelectTrack { kind: TrackKind::Subtitle, choice: TrackChoice::Selected(id) }
}

fn cue(epoch: u32, position_ms: i64, duration_ms: i64, text: &str) -> host_abi::HostCue {
    host_abi::HostCue { epoch, position_ms, duration_ms, text: text.into() }
}

/// Start an attempt whose map names two subtitle streams (English at 0, French at 1, each by language) on a host whose
/// player reports `native` as its TEXT tracks' languages before PREPARED.
fn prepared_with_subtitles(driver: &mut TizenUriDriver<Fake>, fake: &Fake, native: &[&str]) -> (AttemptId, u32) {
    let a = attempt(1);
    let key = SourceKey::new(a.generation());
    let source = UriSource::new(URL.into(), vec![NativeTrackIndex { selection: SUB_EN, index: 0 },
        NativeTrackIndex { selection: SUB_FR, index: 1 }])
        .with_check(SUB_EN, NativeTrackCheck::Language("eng".into()))
        .with_check(SUB_FR, NativeTrackCheck::Language("fre".into()));
    assert_eq!(driver.stage_source(a, key, source), Ok(()));
    let start = Operation::Start { source: key, requirements: plain_requirements() };
    assert_eq!(dispatch(driver, a, 1, start), DispatchResult::Queued);
    let epoch = fake.epoch();
    for (index, code) in native.iter().enumerate() {
        fake.post(epoch, PlayerKind::TextLanguage, index as i64, language_word(code));
    }
    fake.post(epoch, PlayerKind::Prepared, 60_000, 0);
    poll(driver);
    (a, epoch)
}

fn captions(driver: &mut TizenUriDriver<Fake>) -> Vec<(u64, u64, u64, String)> {
    let mut out = Vec::new();
    driver.drain_captions(&mut out);
    out.into_iter().map(|c| (c.track.id, c.start.as_millis(), c.duration_ms, c.text)).collect()
}

/// **A selected subtitle is asked of the host and its cues become the app's captions** (the emulator run of
/// 2026-11-26: the player raises a text track's cues, draws nothing, and switches tracks mid-play). The stream is matched
/// to its native TEXT track by language, as audio is, `VERB_TEXT_TRACK` carries that index, and the request settles on the
/// host's `PLAYER_TRACK`. Only from then are cues handed over: each with text and a duration is a caption of that track at
/// the player's position, the empty "cue ended" ones and the cues of another epoch are not. Off settles with no host call
/// and drops the cues from then on; a stream whose native track cannot be told is refused before the host is asked.
#[test]
fn a_selected_subtitle_is_asked_of_the_host_and_its_cues_become_captions() {
    let (mut driver, fake) = rig();
    let (a, epoch) = prepared_with_subtitles(&mut driver, &fake, &["fr", "en"]);
    assert!(driver.maps_track(SUB_EN) && driver.maps_track(SUB_FR));
    fake.with(|s| s.cues.push(cue(epoch, 1_000, 1_500, "before any selection")));
    assert!(poll(&mut driver).is_empty());
    assert!(captions(&mut driver).is_empty(), "no cue is handed over before the host settled a track");

    // English sits at ordinal 0 but the player lists it second: the language picks native track 1.
    assert_eq!(dispatch(&mut driver, a, 2, pick_sub(11)), DispatchResult::Queued);
    assert_eq!(fake.verbs(), [Verb::TextTrack { index: 1, command: 2 }]);
    fake.post(epoch, PlayerKind::Track, 2, 1);
    assert_eq!(poll(&mut driver), [
        observation(a, Observation::TrackSelected(seen(SUB_EN))),
        DriverEvent::TrackSettled { attempt: a, command: cmd(2), kind: TrackKind::Subtitle, choice: TrackChoice::Selected(11) },
    ]);
    fake.with(|s| s.cues.extend([cue(epoch, 2_003, 1_500, "TWO 01 top\nTWO 01 bottom"), cue(epoch, 3_501, 0, ""),
        cue(epoch + 1, 4_000, 1_500, "another epoch"), cue(epoch, 4_001, 1_500, "{brace} kept")]));
    assert!(poll(&mut driver).is_empty());
    assert_eq!(captions(&mut driver), [(11, 2_003, 1_500, "TWO 01 top\nTWO 01 bottom".to_owned()),
        (11, 4_001, 1_500, "{brace} kept".to_owned())]);
    assert!(captions(&mut driver).is_empty(), "drained once");

    // A refused switch keeps the settled track's cues flowing.
    assert_eq!(dispatch(&mut driver, a, 3, pick_sub(12)), DispatchResult::Queued);
    fake.post(epoch, PlayerKind::Track, 3, -1);
    assert_eq!(poll(&mut driver), [DriverEvent::ControlRefused { attempt: a, command: cmd(3), operation: pick_sub(12),
        failure: Failure::Refused }]);
    fake.with(|s| s.cues.push(cue(epoch, 6_001, 1_500, "still English")));
    assert!(poll(&mut driver).is_empty());
    assert_eq!(captions(&mut driver), [(11, 6_001, 1_500, "still English".to_owned())]);

    // Off: settled at once, no host call, and the cues stop.
    let asked = fake.verbs().len();
    let off_pick = Operation::SelectTrack { kind: TrackKind::Subtitle, choice: TrackChoice::Off };
    assert_eq!(dispatch(&mut driver, a, 4, off_pick), DispatchResult::Queued);
    fake.with(|s| s.cues.push(cue(epoch, 8_001, 1_500, "after off")));
    assert_eq!(poll(&mut driver), [DriverEvent::TrackSettled { attempt: a, command: cmd(4), kind: TrackKind::Subtitle,
        choice: TrackChoice::Off }]);
    assert_eq!(fake.verbs().len(), asked, "Off asks nothing of the host");
    assert!(captions(&mut driver).is_empty(), "no caption after Off");

    // A full host queue is logged and nothing else changes.
    fake.with(|s| s.cues_dropped = 3);
    assert!(poll(&mut driver).is_empty());
}

/// **A cue is never drawn as the old track's after a switch** (review of #1575). The host selects the new native track and
/// only then posts `PLAYER_TRACK`, so cues of the NEW track can reach the library while the old one is still the settled
/// subtitle; and cues have a queue of their own, so one drained in the same poll as the settle has no order against it.
/// While a subtitle `TextTrack` is unanswered no cue is handed over, and a poll hands its cues over before the events it
/// settles: a cue is dropped rather than labelled with a track it may not belong to.
#[test]
fn a_cue_raised_while_a_subtitle_switch_is_unanswered_is_never_the_old_tracks() {
    let (mut driver, fake) = rig();
    let (a, epoch) = prepared_with_subtitles(&mut driver, &fake, &["fr", "en"]);
    assert_eq!(dispatch(&mut driver, a, 2, pick_sub(11)), DispatchResult::Queued);
    fake.post(epoch, PlayerKind::Track, 2, 1);
    poll(&mut driver);
    fake.with(|s| s.cues.push(cue(epoch, 2_000, 1_500, "ENGLISH 01")));
    poll(&mut driver);
    assert_eq!(captions(&mut driver), [(11, 2_000, 1_500, "ENGLISH 01".to_owned())]);

    // The switch to French (native text track 0): the player already raises French cues before the host answers.
    assert_eq!(dispatch(&mut driver, a, 3, pick_sub(12)), DispatchResult::Queued);
    fake.with(|s| s.cues.push(cue(epoch, 4_000, 1_500, "FRENCH 02")));
    assert!(poll(&mut driver).is_empty());
    assert!(captions(&mut driver).is_empty(), "a cue raised during the switch is not the English track's");

    // The answer and one more cue in the same poll: the cue has no order against the settle, so it is not handed over.
    fake.post(epoch, PlayerKind::Track, 3, 0);
    fake.with(|s| s.cues.push(cue(epoch, 6_000, 1_500, "FRENCH 03")));
    assert_eq!(poll(&mut driver), [
        observation(a, Observation::TrackSelected(seen(SUB_FR))),
        DriverEvent::TrackSettled { attempt: a, command: cmd(3), kind: TrackKind::Subtitle, choice: TrackChoice::Selected(12) },
    ]);
    assert!(captions(&mut driver).iter().all(|(track, ..)| *track == 12), "never labelled English");

    // From the next poll on, the cues are French.
    fake.with(|s| s.cues.push(cue(epoch, 8_000, 1_500, "FRENCH 04")));
    poll(&mut driver);
    assert_eq!(captions(&mut driver), [(12, 8_000, 1_500, "FRENCH 04".to_owned())]);
}

/// **A newer subtitle request supersedes an unanswered one** (final review of #1575). The contract lets a second
/// `SelectTrack` go out while the first is pending, and only the newer one completes. So Off, picked while the host has not
/// yet answered a pick, turns the cues off for good: the stale `PLAYER_TRACK` that follows neither makes that track the
/// captioned one nor reports it selected. A newer pick supersedes the same way, and only ITS answer settles.
#[test]
fn off_or_a_newer_pick_supersedes_an_unanswered_subtitle_pick() {
    let (mut driver, fake) = rig();
    let (a, epoch) = prepared_with_subtitles(&mut driver, &fake, &["fr", "en"]);
    // English (native 1) is asked; before the host answers, the viewer picks Off.
    assert_eq!(dispatch(&mut driver, a, 2, pick_sub(11)), DispatchResult::Queued);
    let off_pick = Operation::SelectTrack { kind: TrackKind::Subtitle, choice: TrackChoice::Off };
    assert_eq!(dispatch(&mut driver, a, 3, off_pick), DispatchResult::Queued);
    assert_eq!(poll(&mut driver), [DriverEvent::TrackSettled { attempt: a, command: cmd(3), kind: TrackKind::Subtitle,
        choice: TrackChoice::Off }]);
    // The host's late answer to the English pick, then English cues.
    fake.post(epoch, PlayerKind::Track, 2, 1);
    assert!(poll(&mut driver).is_empty(), "the superseded pick neither settles nor reports a selection");
    fake.with(|s| s.cues.push(cue(epoch, 2_000, 1_500, "ENGLISH 01")));
    poll(&mut driver);
    assert!(captions(&mut driver).is_empty(), "Off stays off: no caption of the superseded track");

    // English, then French before English is answered: only French's answer settles, and its cues are French.
    assert_eq!(dispatch(&mut driver, a, 4, pick_sub(11)), DispatchResult::Queued);
    assert_eq!(dispatch(&mut driver, a, 5, pick_sub(12)), DispatchResult::Queued);
    fake.post(epoch, PlayerKind::Track, 4, 1);
    assert!(poll(&mut driver).is_empty(), "the superseded English pick reports nothing");
    fake.post(epoch, PlayerKind::Track, 5, 0);
    assert_eq!(poll(&mut driver), [
        observation(a, Observation::TrackSelected(seen(SUB_FR))),
        DriverEvent::TrackSettled { attempt: a, command: cmd(5), kind: TrackKind::Subtitle, choice: TrackChoice::Selected(12) },
    ]);
    fake.with(|s| s.cues.push(cue(epoch, 4_000, 1_500, "FRENCH 02")));
    poll(&mut driver);
    assert_eq!(captions(&mut driver), [(12, 4_000, 1_500, "FRENCH 02".to_owned())]);
}

/// **A pick the HOST refuses supersedes nothing** (review of #1579). The newer pick's `TextTrack` verb can be refused by the
/// host's queue (Busy, Stale, Args); the older pick is then still the one the contract holds pending, so it must still
/// settle (or time out) on the host's answer, and no cue is handed over while it is unanswered.
#[test]
fn a_pick_whose_verb_the_host_refuses_leaves_the_earlier_pick_pending() {
    for answer in [true, false] {
        let (mut driver, fake) = rig();
        let (a, epoch) = prepared_with_subtitles(&mut driver, &fake, &["fr", "en"]);
        // French (native 0) is the settled track.
        assert_eq!(dispatch(&mut driver, a, 2, pick_sub(12)), DispatchResult::Queued);
        fake.post(epoch, PlayerKind::Track, 2, 0);
        poll(&mut driver);
        // English (native 1) is asked; a pick of French again is then refused by the host's queue.
        assert_eq!(dispatch(&mut driver, a, 3, pick_sub(11)), DispatchResult::Queued);
        fake.with(|s| s.refuse_verb = Some(Refusal::Busy));
        assert_eq!(dispatch(&mut driver, a, 4, pick_sub(12)), DispatchResult::Refused(Failure::Busy));
        fake.with(|s| s.refuse_verb = None);
        // The player already raises English cues: with English unanswered none is handed over as French's.
        fake.with(|s| s.cues.push(cue(epoch, 1_000, 1_500, "ENGLISH 00")));
        assert!(poll(&mut driver).is_empty());
        assert!(captions(&mut driver).is_empty(), "no cue while the earlier pick is unanswered (answer: {answer})");
        if answer {
            fake.post(epoch, PlayerKind::Track, 3, 1);
            assert_eq!(poll(&mut driver), [
                observation(a, Observation::TrackSelected(seen(SUB_EN))),
                DriverEvent::TrackSettled { attempt: a, command: cmd(3), kind: TrackKind::Subtitle,
                    choice: TrackChoice::Selected(11) },
            ], "the earlier pick still settles");
            fake.with(|s| s.cues.push(cue(epoch, 2_000, 1_500, "ENGLISH 01")));
            poll(&mut driver);
            assert_eq!(captions(&mut driver), [(11, 2_000, 1_500, "ENGLISH 01".to_owned())]);
        } else {
            fake.advance(TIMING.control_timeout_ms);
            assert_eq!(poll(&mut driver), [DriverEvent::ControlRefused { attempt: a, command: cmd(3),
                operation: pick_sub(11), failure: Failure::TimedOut }], "the earlier pick still times out");
        }
    }
}

/// The subtitle's native track is told by its language exactly as an audio stream's is: a language no native TEXT track
/// reports, or two that do with the ordinal not among them, is refused before the host is asked.
#[test]
fn a_subtitle_whose_native_track_cannot_be_told_is_refused_before_the_host_is_asked() {
    for native in [&["de", "it"][..], &[][..], &["fr", "fr"][..]] {
        let (mut driver, fake) = rig();
        let (a, _) = prepared_with_subtitles(&mut driver, &fake, native);
        assert_eq!(dispatch(&mut driver, a, 2, pick_sub(11)), DispatchResult::Refused(Failure::Unsupported), "{native:?}");
        assert!(fake.verbs().is_empty(), "{native:?}");
    }
    // A sole stream needs no language, but is still asked: the verb is what turns its cues on.
    let (mut driver, fake) = rig();
    let a = attempt(1);
    let key = SourceKey::new(a.generation());
    let source = UriSource::new(URL.into(), vec![NativeTrackIndex { selection: SUB_EN, index: 0 }])
        .with_check(SUB_EN, NativeTrackCheck::Sole);
    assert_eq!(driver.stage_source(a, key, source), Ok(()));
    assert_eq!(dispatch(&mut driver, a, 1, Operation::Start { source: key, requirements: plain_requirements() }),
        DispatchResult::Queued);
    fake.post(fake.epoch(), PlayerKind::Prepared, 60_000, 0);
    poll(&mut driver);
    assert_eq!(dispatch(&mut driver, a, 2, pick_sub(11)), DispatchResult::Queued);
    assert_eq!(fake.verbs(), [Verb::TextTrack { index: 0, command: 2 }]);
}

/// **A track pick the Tizen driver cannot carry does not replace the playing title.** Through the Tizen composition's
/// own resolver and a session whose backend set has no Feed (the Tizen composition), with a source selection bound, the
/// shared reconfigure door refuses a source audio or subtitle pick with a typed reason and spends nothing. The red-first
/// reproduction (a playing URI attempt that holds a request, which the old door re-planned and retired) lives in
/// `plx_media`'s `a_source_track_pick_no_composed_backend_can_carry_is_refused_before_the_title_is_replaced`: a request and an
/// attempt view can only be seated there (`RouteStartAttempt::fixture` is `plx_media`'s `test-support`, which this crate's
/// tests do not enable). This one pins that the Tizen resolver's own answer is what reaches that gate.
#[test]
fn a_track_pick_the_driver_cannot_carry_is_refused_by_the_session_and_the_title_keeps_playing() {
    use plx_media::route::{request_viewer_reconfigure, viewer_reconfigure_refusal, PlaybackSession, SubMode,
        ViewerReconfigure, ViewerReconfigureRefusal};
    assert!(!resolver().carries_source_track_change());
    let mut session = PlaybackSession::IDLE;
    session.install_backends(BackendSet::URI);
    session.install_uri_preflight_port(resolver());
    assert!(!session.source_track_change_carriable());
    let selection = plx_media::route::SourceSelection { server: plx_plex::plex::ServerId::UNSET, part_id: 7,
        audio_sid: 31342, subtitle_sid: 0, audio_contract: plx_plex::plex::AudioEnhancements::NONE };
    session.bind_source_selection(selection);
    let mut meta = plx_data::stores::metadata::MetadataStore::default();
    for pick in [ViewerReconfigure::SourceAudio(31343),
        ViewerReconfigure::SourceSubtitle { sid: 9, mode: SubMode::Track, forced_lang: None },
        ViewerReconfigure::SourceTracks { audio_sid: 31343, subtitle_sid: 9 }] {
        assert_eq!(viewer_reconfigure_refusal(&session, &pick), Some(ViewerReconfigureRefusal::NoBackendCarriesTrack));
        assert!(!request_viewer_reconfigure(&mut session, &mut meta, pick, Some(5_000_000_000), Some(false)));
    }
    assert_eq!(session.metric_request(), None, "no replacement request was begun");
    assert_eq!(session.source_selection(), Some(selection), "the playing source selection is untouched");
    // With a Feed in the set (webOS) the same pick is not refused here.
    session.install_backends(BackendSet::BOTH);
    assert!(session.source_track_change_carriable());
}

/// One log line per player event with its payload (so an emulator or retail run shows SEEK_DONE's b, the Home sequence,
/// a STALL, an ERROR code), the repeating kinds on a 10 s clock. Read from the pure step's own `Fx::Log`.
#[test]
fn every_player_event_is_logged_once_and_positions_and_buffering_are_rate_limited() {
    use super::machine::{Fx, Machine};
    let mut machine = Machine::new(TIMING);
    let mut fx = Vec::new();
    let key = SourceKey::new(1);
    machine.stage(0, attempt(1), key, UriSource::new(URL.into(), vec![])).unwrap();
    machine.start(0, true, attempt(1), key, plain_requirements(), &mut fx);
    fx.clear();
    let ev = |kind, a, b| PlayerEvent { epoch: 1, kind, a, b };
    let lines = |machine: &mut Machine, now: u64, events: &[PlayerEvent]| {
        let mut fx = Vec::new();
        machine.poll(now, events, false, &mut fx);
        fx.into_iter().filter_map(|f| if let Fx::Log(line) = f { Some(line) } else { None }).collect::<Vec<_>>()
    };
    let got = lines(&mut machine, 10, &[ev(PlayerKind::Prepared, 600_000, 0), ev(PlayerKind::Started, 0, 0),
        ev(PlayerKind::Position, 1_000, 0), ev(PlayerKind::Position, 1_250, 0), ev(PlayerKind::SeekDone, 7, -2),
        ev(PlayerKind::Stall, 0, 0)]);
    assert_eq!(got, [
        "tizen-uri: player event Prepared epoch=1 a=600000 b=0",
        "tizen-uri: player event Started epoch=1 a=0 b=0",
        "tizen-uri: player event Position epoch=1 a=1000 b=0",
        "tizen-uri: player event SeekDone epoch=1 a=7 b=-2",
        "tizen-uri: player event Stall epoch=1 a=0 b=0",
    ], "the second position inside 10 s is not logged");
    assert!(lines(&mut machine, 5_000, &[ev(PlayerKind::Position, 2_000, 0)]).is_empty());
    assert_eq!(lines(&mut machine, 10_010, &[ev(PlayerKind::Position, 3_000, 0)]),
        ["tizen-uri: player event Position epoch=1 a=3000 b=0"], "once every 10 s");
}
