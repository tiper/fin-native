//! **The failure value.** One [`PlaybackFailure`], captured ONCE when an attempt enters Error, and read
//! by everything that shows or reports a failure: the read-out (the words, the glyph, the action row), the
//! report's `playback.failed` and the request metrics. Before it, each of those re-derived the failure from
//! about six session fields on every call: a frame of a standing failure ran the whole derivation twice
//! (`error_captured`, `failure_context_captured`), and the report ran it a third time on the one frame it
//! reported, so the read-out and the report could name different causes for the same attempt.
//!
//! * **Frozen at capture** ([`PlaybackFailure`]): the cause (its kind and its words), the settings in force
//!   ([`FailureSettings`]: the Direct Play mode and the dev trigger's arm) and the offers the attempt's
//!   request and composition made ([`FrozenOffers`]: whether it can be resolved again, whether a Quality rung
//!   can be carried). None of them changes while a failure stands: a new request, a landing and a stop each
//!   clear the value, and a different failtest arm is a different failure.
//! * **Live beside it** ([`LiveOffers`]): the two inputs that DO change while a failure stands. The sandbox
//!   repair becomes busy once started (so *Repair* leaves the row without a new failure), and the item's
//!   tracks can land after the failure (so *Audio & Subtitles* appears without one). They are read per frame
//!   by [`live_offers`] and passed beside the value to [`failure_actions`](super::failure_actions).
//! * **One refinement**: a server's stop note for the same playback that arrives after the failure was
//!   captured as something weaker ([`observe_failure`] applies it through [`crate::route::refine_failure`]).
//!   The derivation it replaces let that note win over every cause but the jail, the tuners and a refused
//!   preview, so the value does too.
//!
//! **One field, one writer** (`docs/sans-io-guide.md`, checklist item 9): the value lives in
//! [`crate::route::PlaybackSession`], and [`observe_failure`] is the only writer that records or refines it
//! (through the route's doors), and it clears it on leaving Error; the three route resets (`clear_play_verdict`,
//! `request_play_with_queue`, `apply_plan`) clear it with the verdict they reset. It runs at the app's IO boundary, three times a frame at most
//! (`app::player_presentation::capture_failure`), because the derivation reads what no machine may read: the
//! server-stop note, the server's subscription, and the dev trigger the app samples and passes in. A reader
//! finds the value or finds none; there is no fallback derivation behind it.

use super::{FailureKind, ErrorShape, PlaybackState};
use plx_plex::plex::session::DirectPlayMode;

/// The settings that were in force for the failed attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailureSettings {
    /// The session's Direct Play mode, frozen for one logical playback.
    pub mode: DirectPlayMode,
    /// The dev trigger's arm (`/tmp/plxnative-failtest=<arm>`) the failure was captured under, `None` when
    /// absent. A different arm is a different failure ([`PlaybackFailure::stands_for`]), which keeps the
    /// simulator's arm-switching workflow.
    pub diagnostic: Option<String>,
}

impl FailureSettings {
    /// Force Direct Play was in effect for the failed attempt. It overrides every quality rung (Force always
    /// requests the original), so no rung can change the outcome. The dev trigger's `policy` arm stands for it.
    pub fn forced(&self) -> bool {
        self.mode == DirectPlayMode::Forced || super::failtest_forced_by(self.diagnostic.as_deref())
    }

    /// Direct Play was set to **Disabled** for the failed attempt: every rung asks the server to convert, so a
    /// refusal of that conversion is answered by the one change that lifts the setting. Never together with
    /// [`forced`](Self::forced) (the session holds one mode; the dev trigger's `disabled` arm only adds this one).
    pub fn direct_play_disabled(&self) -> bool {
        self.mode == DirectPlayMode::Disabled || super::failtest_disabled_by(self.diagnostic.as_deref())
    }
}

/// The offers that are facts of the failed attempt's request and of the composition, so they hold for as long as
/// the failure does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrozenOffers {
    /// The failed attempt has a Plex request that can be resolved again. A URL/dev-trigger playback has none, so
    /// nothing that retries can do anything.
    pub can_retry: bool,
    /// Some composed backend could carry a Quality rung the viewer picks
    /// ([`crate::route::PlaybackSession::quality_pick_carriable`]: a Feed, or a URI provider that certifies a
    /// converted output) and the item is not a channel. Without one the ladder is withheld from the More menu and
    /// a rung is refused, so the read-out's *Change quality* would open an empty ladder: it is not offered.
    pub quality_carriable: bool,
}

/// The two inputs of the action row that change while a failure stands (see the module doc), passed beside the
/// value rather than frozen into it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveOffers {
    /// The sandbox repair can still be started (nothing is running or finished).
    pub repair_idle: bool,
    /// The failed item has audio or subtitle tracks the picker can list ([`super::failed_tracks_available`]).
    pub tracks_available: bool,
}

/// **Why one attempt failed, and what its viewer may do about it.** Immutable once recorded (the one
/// refinement, [`refined`](Self::refined), returns a new value), and owned by the
/// [`PlaybackSession`](crate::route::PlaybackSession) through [`crate::route::failure`].
#[derive(Clone, Debug, PartialEq)]
pub struct PlaybackFailure {
    attempt: i64,
    cause: ErrorShape,
    settings: FailureSettings,
    offers: FrozenOffers,
}

impl PlaybackFailure {
    /// The attempt this failure belongs to: `player::report`'s attempt id, which a request mints (retries
    /// included), so a failure that follows a retry is a different value to anything that keys on it (the
    /// read-out's focus, see `FailureReadout::attempt`).
    pub fn attempt(&self) -> i64 {
        self.attempt
    }

    /// The cause as the read-out and the report name it: the stable kind and the words.
    pub fn shape(&self) -> &ErrorShape {
        &self.cause
    }

    pub fn kind(&self) -> FailureKind {
        self.cause.kind
    }

    pub fn settings(&self) -> &FailureSettings {
        &self.settings
    }

    pub fn forced(&self) -> bool {
        self.settings.forced()
    }

    pub fn direct_play_disabled(&self) -> bool {
        self.settings.direct_play_disabled()
    }

    pub fn can_retry(&self) -> bool {
        self.offers.can_retry
    }

    pub fn quality_carriable(&self) -> bool {
        self.offers.quality_carriable
    }

    /// Is this the failure of `attempt` captured under dev-trigger arm `diagnostic`? A request mints a new attempt,
    /// and a different arm is a different failure, so either one makes [`observe_failure`] capture again.
    pub(crate) fn stands_for(&self, diagnostic: Option<&str>, attempt: i64) -> bool {
        self.attempt == attempt && self.settings.diagnostic.as_deref() == diagnostic
    }

    /// The same failure with a better-informed cause: settings and offers stand, the kind and the words are
    /// replaced.
    pub(crate) fn refined(mut self, cause: ErrorShape) -> Self {
        self.cause = cause;
        self
    }

    /// A failure of `kind` under `mode` with `offers`, with no words: for the tests that grade the action
    /// table, which reads only the kind, the settings and the offers.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(kind: FailureKind, mode: DirectPlayMode, offers: FrozenOffers) -> Self {
        Self {
            attempt: 0,
            cause: ErrorShape {
                kind,
                caption: c"",
                panel: "",
                readout: "",
                detail: std::borrow::Cow::Borrowed(""),
                no_pass: false,
            },
            settings: FailureSettings { mode, diagnostic: None },
            offers,
        }
    }

    /// The same failure with another attempt ordinal, for the tests that stand for two attempts.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_attempt_for_test(mut self, attempt: i64) -> Self {
        self.attempt = attempt;
        self
    }
}

/// What the app sampled of the dev trigger `/tmp/plxnative-failtest` for this frame: its arm (`None` when absent
/// or unreadable), and whether the trigger is armed at all (an empty or unreadable file arms it with no arm). The
/// IO is the app's (`app::player_presentation::capture_failure`); a controlled host samples none
/// ([`NONE`](Self::NONE)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FailureTrigger<'a> {
    pub arm: Option<&'a str>,
    pub armed: bool,
}

impl FailureTrigger<'_> {
    pub const NONE: FailureTrigger<'static> = FailureTrigger { arm: None, armed: false };

    /// Does the trigger force a read-out even where the state is not Error?
    fn forces(&self) -> bool {
        self.arm.is_some() || self.armed
    }
}

/// **Capture, keep, refine or clear the session's failure for this frame.** The one writer that records or
/// refines [`crate::route::failure`] (the route's resets clear it with the verdict they reset), called ahead of every reader of it (`app::player_presentation::capture_failure`
/// at the frame's publication and before each of its two report observations), so a reader finds the value of
/// THIS frame. Idempotent: a frame that changes nothing writes nothing.
///
/// * The state is not Error and no trigger forces a read-out: clear (a retry that left Error, a withdrawn
///   request, a removed trigger file).
/// * Otherwise, with no failure recorded for this `attempt` under this trigger arm: derive and record it.
/// * Otherwise it stands; only [`refine_server_stop`] can change it.
///
/// `attempt` is `player::report`'s id, which the app reads at the boundary.
pub fn observe_failure(ps: &mut crate::route::PlaybackSession, trigger: FailureTrigger<'_>, attempt: i64) {
    if !trigger.forces() && super::state(ps) != PlaybackState::Error {
        crate::route::clear_failure(ps);
        return;
    }
    if crate::route::failure(ps).is_some_and(|standing| standing.stands_for(trigger.arm, attempt)) {
        refine_server_stop(ps);
        return;
    }
    let failure = derive_failure(ps, trigger.arm, attempt);
    crate::route::record_failure(ps, failure);
}

/// The derivation, called by [`observe_failure`] alone (and by the tests that need a value without a frame).
fn derive_failure(ps: &crate::route::PlaybackSession, diagnostic: Option<&str>, attempt: i64) -> PlaybackFailure {
    PlaybackFailure {
        attempt,
        cause: super::cause_of(ps, diagnostic),
        settings: FailureSettings {
            mode: crate::route::direct_play_mode_of(ps),
            diagnostic: diagnostic.map(str::to_owned),
        },
        offers: FrozenOffers {
            can_retry: crate::route::can_retry_current_play(ps),
            quality_carriable: !crate::route::is_live(ps) && ps.quality_pick_carriable(),
        },
    }
}

/// **A server's stop note arriving after a weaker capture.** The server's note for THIS playback outranks every
/// cause but the jail, a busy tuner, a refused preview and a dev-trigger arm (the order the derivation always
/// had), so a failure captured as one of the others (an interrupted stream, say) becomes the stop with its
/// reason once the note lands. Nothing else about a standing failure changes.
fn refine_server_stop(ps: &mut crate::route::PlaybackSession) {
    let Some(standing) = crate::route::failure(ps) else { return };
    if standing.settings.diagnostic.is_some()
        || matches!(standing.kind(), FailureKind::JailMissingRtkmem | FailureKind::TunersBusy | FailureKind::PreviewNotDirect)
    {
        return;
    }
    let Some(reason) = super::with_server_stop(crate::route::playback_session_id(ps), |note| note.reason.clone()) else {
        return;
    };
    if standing.kind() == FailureKind::ServerStopped && standing.cause.detail == reason.as_str() {
        return;
    }
    crate::route::refine_failure(ps, super::server_stop_shape(reason));
}

/// The inputs of the action row that are read live, per frame (see the module doc).
pub fn live_offers(ps: &crate::route::PlaybackSession, meta: plx_data::metadata::MetadataView<'_>) -> LiveOffers {
    LiveOffers {
        repair_idle: ps.repair_status == plx_os::tv::native_video::State::Idle,
        tracks_available: super::failed_tracks_available(ps, meta),
    }
}

/// Test-only: the failure `ps` stands for right now, derived without a frame (no dev trigger, attempt 0).
#[cfg(any(test, feature = "test-support"))]
pub fn failure_for_test(ps: &crate::route::PlaybackSession) -> PlaybackFailure {
    derive_failure(ps, None, 0)
}

/// Test-only: the words of the failure `ps` stands for right now, derived without a frame.
#[cfg(any(test, feature = "test-support"))]
pub fn error_for_test(ps: &crate::route::PlaybackSession) -> ErrorShape {
    super::cause_of(ps, None)
}

/// Test-only: the same, under the dev trigger's arm `diagnostic`.
#[cfg(any(test, feature = "test-support"))]
pub fn failure_under_for_test(ps: &crate::route::PlaybackSession, diagnostic: Option<&str>) -> PlaybackFailure {
    derive_failure(ps, diagnostic, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route::{PlayVerdict, PlaybackSession};

    fn refused() -> PlaybackSession {
        let mut ps = PlaybackSession::IDLE;
        crate::route::refuse_for_test(&mut ps, PlayVerdict::DirectPlayDisabled);
        ps
    }

    /// **The failure is derived once per attempt, not once per reader.** A standing failure is not derived again by a
    /// frame (or by the second and third capture of one frame): what changed under it is not read, until a request
    /// mints a new attempt.
    #[test]
    fn the_failure_is_captured_once_per_attempt() {
        let _g = plx_base::testlock::serial();
        let mut ps = refused();
        assert!(crate::route::failure(&ps).is_none(), "nothing is captured until the frame captures it");
        observe_failure(&mut ps, FailureTrigger::NONE, 7);
        let first = crate::route::failure(&ps).cloned().expect("an Error state is captured");
        assert_eq!(first.attempt(), 7);
        assert_eq!(first.kind(), FailureKind::PlaybackPolicy);

        // something the derivation reads changes under the standing failure: it does not move
        ps.jail_load_blocked = true;
        observe_failure(&mut ps, FailureTrigger::NONE, 7);
        observe_failure(&mut ps, FailureTrigger::NONE, 7);
        assert_eq!(crate::route::failure(&ps), Some(&first), "the same attempt keeps its value");

        // the next attempt's failure is its own derivation
        observe_failure(&mut ps, FailureTrigger::NONE, 8);
        let second = crate::route::failure(&ps).expect("recaptured");
        assert_eq!((second.attempt(), second.kind()), (8, FailureKind::JailMissingRtkmem));
    }

    /// **A failure that is no longer one is cleared**: a removed trigger file, a retry that left Error, a request.
    #[test]
    fn leaving_error_clears_the_failure() {
        let _g = plx_base::testlock::serial();
        let mut ps = PlaybackSession::IDLE;
        let armed = FailureTrigger { arm: Some("policy"), armed: true };
        observe_failure(&mut ps, armed, 3);
        assert_eq!(crate::route::failure(&ps).map(PlaybackFailure::kind), Some(FailureKind::PlaybackPolicy));
        assert!(crate::route::failure(&ps).unwrap().forced(), "the policy arm stands for Force");
        observe_failure(&mut ps, FailureTrigger::NONE, 3);
        assert!(crate::route::failure(&ps).is_none(), "the trigger went away and nothing is in Error");

        let mut ps = refused();
        observe_failure(&mut ps, FailureTrigger::NONE, 3);
        assert!(crate::route::failure(&ps).is_some());
        crate::route::clear_play_verdict_for_test(&mut ps);
        assert!(crate::route::failure(&ps).is_none(), "a new play clears the failure with the verdict it stood on");
        observe_failure(&mut ps, FailureTrigger::NONE, 3);
        assert!(crate::route::failure(&ps).is_none(), "and a frame of the healthy session captures none");
    }

    /// **A different dev-trigger arm is a different failure** (the simulator's arm-switching workflow).
    #[test]
    fn a_different_failtest_arm_replaces_the_failure() {
        let _g = plx_base::testlock::serial();
        let mut ps = PlaybackSession::IDLE;
        observe_failure(&mut ps, FailureTrigger { arm: Some("policy"), armed: true }, 1);
        observe_failure(&mut ps, FailureTrigger { arm: Some("no_backend"), armed: true }, 1);
        assert_eq!(crate::route::failure(&ps).map(PlaybackFailure::kind), Some(FailureKind::NoQualifiedBackend));
        assert_eq!(crate::route::failure(&ps).unwrap().settings().diagnostic.as_deref(), Some("no_backend"));
    }

    /// **Repair stays live.** Starting the sandbox repair makes *Repair* leave the row while the failure stands, and
    /// the failure value does not move: the offer is a [`LiveOffers`] input, not part of the capture.
    #[test]
    fn repair_leaves_the_row_once_a_repair_starts() {
        use super::super::{failure_actions, FailureAction};
        let _g = plx_base::testlock::serial();
        let mut ps = PlaybackSession::IDLE;
        ps.jail_load_blocked = true;
        observe_failure(&mut ps, FailureTrigger::NONE, 1);
        let failure = crate::route::failure(&ps).cloned().expect("the jail is captured");
        assert_eq!(failure.kind(), FailureKind::JailMissingRtkmem);
        let live = |ps: &PlaybackSession| LiveOffers {
            repair_idle: ps.repair_status == plx_os::tv::native_video::State::Idle,
            tracks_available: false,
        };
        assert!(failure_actions(&failure, live(&ps)).contains(&FailureAction::Repair));
        ps.repair_status = plx_os::tv::native_video::State::Running;
        observe_failure(&mut ps, FailureTrigger::NONE, 1);
        assert_eq!(crate::route::failure(&ps), Some(&failure), "the capture is untouched by the repair");
        assert!(!failure_actions(&failure, live(&ps)).contains(&FailureAction::Repair), "the live offer left the row");
    }

    /// **Tracks stay live.** The item's tracks can land after the failure was captured; *Audio & Subtitles* then joins
    /// the row of the same, unmoved failure.
    #[test]
    fn tracks_landing_after_the_capture_add_audio_and_subtitles() {
        use super::super::{failure_actions, FailureAction};
        let failure = PlaybackFailure::for_test(
            FailureKind::PlaybackInterrupted,
            DirectPlayMode::Auto,
            FrozenOffers { can_retry: true, quality_carriable: true },
        );
        let before = failure_actions(&failure, LiveOffers { repair_idle: true, tracks_available: false });
        let after = failure_actions(&failure, LiveOffers { repair_idle: true, tracks_available: true });
        assert!(!before.contains(&FailureAction::ChangeTracks));
        assert_eq!(after.iter().position(|a| *a == FailureAction::ChangeTracks), Some(before.len() - 1), "before Back");
        assert_eq!(
            after.iter().copied().filter(|a| *a != FailureAction::ChangeTracks).collect::<Vec<_>>(),
            before,
            "nothing else about the row moved"
        );
    }

    /// **Every runtime cause reads the same through the capture as through the derivation it replaced.**
    #[test]
    fn every_runtime_cause_is_captured_as_it_was_derived() {
        use super::super::RuntimeFailure as R;
        let _g = plx_base::testlock::serial();
        for cause in [R::Unknown, R::MediaSource, R::PlaybackInterrupted, R::TvPipeline, R::LoadTimeout, R::ReleaseTimeout,
            R::ResolveFailed, R::ServerStopped] {
            let mut ps = PlaybackSession::IDLE;
            crate::route::note_engine_failure(&mut ps, cause, 0);
            assert_eq!(super::super::state(&ps), PlaybackState::Error, "{cause:?}");
            observe_failure(&mut ps, FailureTrigger::NONE, 1);
            let captured = crate::route::failure(&ps).unwrap_or_else(|| panic!("{cause:?} is captured"));
            assert_eq!(*captured.shape(), error_for_test(&ps), "{cause:?}");
            assert_eq!(*captured, failure_for_test(&ps).with_attempt_for_test(1), "{cause:?}");
        }
    }
}
