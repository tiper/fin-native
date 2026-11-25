//! One conformance suite for every [`UriDriver`]: the part of the contract a driver must keep whatever
//! its executor is, run over the contract's own [`FakeUriDriver`](crate::test_support::FakeUriDriver) and
//! over the platform drivers that name this crate with `test-support`.
//!
//! A driver's native side is not part of the contract, so the suite never speaks it: a [`UriHarness`]
//! carries the two native answers a case needs (the player is prepared, the release was confirmed) in
//! whatever form its driver takes them, and the suite asserts only what the driver then says through
//! [`DriverSet`]. The cases are the ones every driver here satisfies. A harness for the fake scripts its
//! answers, so what that run pins is the sequence a real driver's events must follow; the Tizen driver's
//! harness feeds its own host port and pins the driver itself.

use crate::uri::{DisplayEvidence, UriDriver, UriSource};
use crate::{AttemptId, BackendEvent, BackendKind, CommandId, DispatchResult, DriverEvent, DriverSet, ExitDisposition,
    Failure, Fidelity, Observation, Operation, Playback, ReleaseStatus, Requirements, SourceKey, SourceKind,
    StopReason, TrackChoice, TrackKind, TrackSelection};
use crate::uri::NativeTrackIndex;
use plx_base::task::MainThread;

/// A driver under test and the native answers a case drives it with.
pub trait UriHarness {
    type Driver: UriDriver<Context = MainThread>;
    fn driver(&mut self) -> &mut Self::Driver;
    /// Called right after a Start was queued: a scripted driver (the fake) takes up the ownership a real one
    /// takes by itself.
    fn after_start(&mut self) {}
    /// The native side reports that the player of the active attempt is prepared.
    fn native_prepared(&mut self);
    /// The native side confirms the release the driver asked for (and, for a driver whose release it
    /// does not wait for, does nothing).
    fn native_closed(&mut self);
}

const URL: &str = "https://example.invalid/a.mkv";

fn mt() -> MainThread {
    // SAFETY: a host test thread stands in for the loop thread, as the fake's own tests do.
    unsafe { MainThread::assume() }
}

fn attempt(n: u64) -> AttemptId {
    AttemptId::new(1, n)
}

fn requirements() -> Requirements {
    Requirements { source: SourceKind::ProgressiveUri, video: Fidelity::Exact, audio: Fidelity::Exact,
        audio_track: TrackChoice::Unspecified, subtitle_track: TrackChoice::Unspecified }
}

fn start(driver: &mut impl UriDriver<Context = MainThread>, attempt: AttemptId, key: SourceKey,
    command: CommandId) -> DispatchResult {
    driver.dispatch(BackendKind::Uri, &mt(), attempt, command,
        Operation::Start { source: key, requirements: requirements() })
}

/// Run every case over a fresh harness each.
pub fn conforms<H: UriHarness>(make: impl Fn() -> H) {
    states_what_it_can_call(&mut make());
    refuses_a_source_it_could_not_play(&mut make());
    answers_for_its_own_backend_only(&mut make());
    runs_one_attempt_from_stage_to_release(&mut make());
}

/// A URI driver answers `capabilities(Uri)` with a progressive, pausable, seekable set that asks for no
/// packet feed and no audio disable, and answers nothing for the Feed; its display evidence is internally
/// consistent. `subtitle_off` is the driver's to state: a pipeline that never selects a text track carries
/// Off by construction (the Tizen driver), one that shows a default subtitle by itself does not (the webOS
/// `UriBackend`, whose `new` asserts it false).
pub fn states_what_it_can_call<H: UriHarness>(harness: &mut H) {
    let driver = harness.driver();
    let caps = driver.capabilities(BackendKind::Uri).expect("a URI driver answers for the URI backend");
    assert!(caps.progressive_uri && caps.pause && caps.seek, "{caps:?}");
    assert!(!caps.packet_feed && !caps.audio_off, "{caps:?}");
    assert_eq!(driver.capabilities(BackendKind::Feed), None);
    let display: DisplayEvidence = driver.display();
    assert_eq!(display.is_ready(), matches!(display.readiness, crate::uri::DisplayReadiness::Ready));
    assert_eq!(driver.exit_disposition(), ExitDisposition::Clean, "a fresh driver owns nothing");
    assert!(driver.poll(&mt()).is_empty(), "nothing happened, nothing is reported");
}

/// A locator with no URL, or a track map that names one selection twice, is never staged.
pub fn refuses_a_source_it_could_not_play<H: UriHarness>(harness: &mut H) {
    let driver = harness.driver();
    let key = SourceKey::new(1);
    assert_eq!(driver.stage_source(attempt(1), key, UriSource::new(String::new(), vec![])), Err(Failure::Stale));
    let twice = |index| NativeTrackIndex { selection: TrackSelection { kind: TrackKind::Audio, id: 4 }, index };
    assert_eq!(driver.stage_source(attempt(1), key, UriSource::new(URL.into(), vec![twice(1), twice(2)])),
        Err(Failure::Unsupported));
}

/// A command for the Feed backend is refused as unavailable, and discarding a Feed source changes nothing.
pub fn answers_for_its_own_backend_only<H: UriHarness>(harness: &mut H) {
    let driver = harness.driver();
    assert_eq!(driver.dispatch(BackendKind::Feed, &mt(), attempt(1), CommandId::for_test(1), Operation::Play),
        DispatchResult::Refused(Failure::Unavailable));
    driver.discard_staged(BackendKind::Feed, attempt(1), SourceKey::new(1));
    assert!(driver.poll(&mt()).is_empty());
}

/// One attempt, stage to release: the Start is queued, the plane is owned and exit is preserved from
/// then until the release; the prepared player is reported for THIS attempt as `ReadyAwaitPlay`; a Stop
/// is queued; the confirmed release is reported once, for this attempt, and exit is clean again.
pub fn runs_one_attempt_from_stage_to_release<H: UriHarness>(harness: &mut H) {
    let key = SourceKey::new(7);
    let this = attempt(2);
    assert_eq!(harness.driver().stage_source(this, key, UriSource::new(URL.into(), vec![])), Ok(()));
    assert_eq!(start(harness.driver(), this, key, CommandId::for_test(1)), DispatchResult::Queued);
    harness.after_start();
    assert!(harness.driver().plane_owned(), "the plane is owned from the Start");
    assert_eq!(harness.driver().exit_disposition(), ExitDisposition::PreserveNativeOwnership);
    harness.native_prepared();
    let events = harness.driver().poll(&mt());
    assert!(events.iter().any(|e| matches!(e, BackendEvent { backend: BackendKind::Uri,
        event: DriverEvent::Observation { attempt, observation: Observation::Playback(seen) } }
        if *attempt == this && seen.value == Playback::ReadyAwaitPlay)), "{events:?}");
    assert!(events.iter().all(|e| match e.event {
        DriverEvent::Observation { attempt, .. } | DriverEvent::ControlAccepted { attempt, .. }
        | DriverEvent::ControlRefused { attempt, .. } | DriverEvent::SeekLanded { attempt, .. }
        | DriverEvent::TrackSettled { attempt, .. } | DriverEvent::Release { attempt, .. } => attempt == this,
    }), "an event names another attempt: {events:?}");
    assert_eq!(harness.driver().dispatch(BackendKind::Uri, &mt(), this, CommandId::for_test(2),
        Operation::Stop(StopReason::Stop)), DispatchResult::Queued);
    harness.native_closed();
    let released: Vec<_> = harness.driver().poll(&mt()).into_iter()
        .filter(|e| matches!(e.event, DriverEvent::Release { .. })).collect();
    assert_eq!(released, [BackendEvent { backend: BackendKind::Uri,
        event: DriverEvent::Release { attempt: this, status: ReleaseStatus::Released } }]);
    assert_eq!(harness.driver().exit_disposition(), ExitDisposition::Clean);
    assert!(harness.driver().poll(&mt()).is_empty(), "the release is reported once");
}
