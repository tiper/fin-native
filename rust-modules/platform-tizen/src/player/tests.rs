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
    ProductEvidence, ResolvePort, StandardDirectPlay, UriDecline, Verdict};
use plx_playback_contract::uri::{DisplayReadiness, NativeTrackIndex, UriDriver, UriSource};
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

#[test]
fn only_the_first_audio_stream_is_selectable_and_the_driver_says_so() {
    let (mut driver, fake) = rig();
    let first = TrackSelection { kind: TrackKind::Audio, id: 31342 };
    let second = TrackSelection { kind: TrackKind::Audio, id: 31343 };
    let a = attempt(1);
    stage_and_start(&mut driver, a, vec![NativeTrackIndex { selection: first, index: 0 }, NativeTrackIndex { selection: second, index: 1 }]);
    fake.post(fake.epoch(), PlayerKind::Prepared, 1_000, 0);
    poll(&mut driver);
    assert!(driver.maps_track(first) && !driver.maps_track(second));
    let pick = |id| Operation::SelectTrack { kind: TrackKind::Audio, choice: TrackChoice::Selected(id) };
    assert_eq!(dispatch(&mut driver, a, 2, pick(31342)), DispatchResult::Queued);
    assert_eq!(poll(&mut driver), [
        observation(a, Observation::TrackSelected(seen(first))),
        DriverEvent::TrackSettled { attempt: a, command: cmd(2), kind: TrackKind::Audio, choice: TrackChoice::Selected(31342) },
    ]);
    assert_eq!(dispatch(&mut driver, a, 3, pick(31343)), DispatchResult::Refused(Failure::Unsupported));
    assert_eq!(dispatch(&mut driver, a, 4, Operation::SelectTrack { kind: TrackKind::Subtitle, choice: TrackChoice::Selected(1) }),
        DispatchResult::Refused(Failure::Unsupported));
    assert!(poll(&mut driver).is_empty(), "a refused selection is the planner's typed answer: it fails and tears down nothing");
    assert!(fake.verbs().is_empty(), "no track verb exists on the host: nothing was asked of it");
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
    assert!(!CAPS.hls_uri && !CAPS.packet_feed && !CAPS.subtitle_track && !CAPS.audio_off);
    assert!(CAPS.subtitle_off, "the host player is given no subtitle, so Off holds by construction");
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
        subtitle_source: None,
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

/// The host player has no track verb, so a play that selects any audio stream but the first is declined instead
/// of started on the wrong track, and a play whose position is unknown is too.
#[test]
fn a_play_that_selects_another_audio_stream_is_declined() {
    for ordinal in [Some(1), Some(3), None] {
        let verdict = resolver().qualify(&off(), request(plays(), 31343, ordinal));
        assert!(matches!(verdict, Verdict::Unknown(UriDecline::NotCertified)), "ordinal {ordinal:?}");
    }
    // No selected stream at all (a silent file) needs no track.
    assert!(matches!(resolver().qualify(&off(), request(plays(), 0, None)), Verdict::Known(_)));
}

/// Which stream the retail player starts on when a source has several is unmeasured, so a source with more than one
/// audio stream is not certified, whichever stream is selected (the webOS precedent `ManyAudioTracksExported`).
#[test]
fn a_source_with_several_audio_streams_is_not_certified() {
    for (sid, ordinal) in [(31342, Some(0)), (31343, Some(1)), (0, None)] {
        let mut req = request(plays(), sid, ordinal);
        req.has_multiple_audio_tracks = true;
        let verdict = resolver().qualify(&off(), req);
        assert!(matches!(verdict, Verdict::Unknown(UriDecline::NotCertified)), "audio {sid}");
    }
    assert!(matches!(resolver().qualify(&off(), request(plays(), 31342, Some(0))), Verdict::Known(_)), "one stream is");
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

/// A track selection the contract states for a subtitle is outside the capabilities: no offer carries one.
#[test]
fn a_selected_subtitle_is_not_offered() {
    let mut req = request(plays(), 31342, Some(0));
    req.subtitle_choice = TrackChoice::Selected(9);
    req.selection.subtitle_sid = 9;
    assert!(!matches!(resolver().qualify(&off(), req), Verdict::Known(_)));
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
