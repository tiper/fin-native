//! **What each button of the playback-failure read-out does**, pinned through the app's real performers.
//!
//! The read-out's four recoveries (*Try again*, *Play automatically*, a rung of *Change quality*, a pick of
//! *Audio & Subtitles*) start a new attempt of the failed request. These tests drive each one the way the
//! viewer's press reaches the app (`player_requests` for the screen's requests, `apply_quality` for the `…`
//! popover's rung) on a session that has CAPTURED the failure whose row offers it, and pin what the press does:
//! whether the attempt started, which request it repeats, why the viewer asked (the attempt's origin), which
//! preference moved and how often it was published, which frame was damaged, and what a refusal leaves alone.
//! They are written against the performers as they were before the recoveries went through one typed door, and
//! they hold through it: a recovery is the same requests in the same order.

use crate::app::run::lifecycle_regression_tests::app;
use crate::app::App;
use plx_media::player::report::AttemptOrigin;
use plx_media::player::{self, FailureTrigger};
use plx_media::route::{self, PlayVerdict, Quality};
use plx_plex::plex::session::{DirectPlayMode, DirectPlayModePref};
use plx_screens::registry::PlayerReq;

/// The failure `arm` of the dev trigger, captured the way the frame captures it: the read-out the viewer sees.
fn capture_arm(app: &mut App, arm: &str) {
    player::observe_failure(&mut app.player.session, FailureTrigger { arm: Some(arm), armed: false }, player::report::attempt_id());
}

/// A session that asked for item `m1` and whose attempt FAILED with a transient cause (an interrupted stream), so
/// its row offers *Try again*, and (this build composes a Feed) *Change quality*.
fn interrupted() -> App {
    let mut app = app();
    route::seed_request_for_test(&mut app.player.session, "m1");
    capture_arm(&mut app, "connection");
    app
}

/// A session whose attempt was REFUSED by the server (`state()` is Error: a real failure, not a read-out forced by the
/// dev trigger), so its row offers *Change quality*.
fn refused_by_the_server() -> App {
    let mut app = app();
    route::seed_request_for_test(&mut app.player.session, "m1");
    route::refuse_for_test(&mut app.player.session, PlayVerdict::Server("Cannot convert.".into(), route::DecisionCodes::default()));
    assert_eq!(player::state(&app.player.session), player::PlaybackState::Error);
    player::observe_failure(&mut app.player.session, FailureTrigger::NONE, player::report::attempt_id());
    app
}

/// The row the read-out offers right now, as the presentation computes it.
fn row(app: &App) -> Vec<player::FailureAction> {
    let failure = route::failure(&app.player.session).expect("the failure is captured");
    player::failure_actions(failure, player::live_offers(&app.player.session, app.runtime.metadata_view()))
}

/// Everything a press can be seen to have done.
#[derive(Debug, PartialEq)]
struct Seen {
    /// a resolve of the request is in flight: the attempt started
    pending: bool,
    /// the key of the request that attempt repeats
    request: Option<String>,
    /// why the viewer asked, as the report records it
    origin: AttemptOrigin,
    /// the failure of the attempt that was replaced is gone
    failure_cleared: bool,
    /// the live Direct Play preference (what *Play automatically* persists; the mode the NEW attempt resolves under is
    /// pinned in `plx_media`: `the_recovery_doors_overrides_set_the_mode_the_retry_resolves_under`)
    mode: DirectPlayMode,
    /// how many times it was published since the test began
    mode_publishes: u32,
    /// the live Quality preference
    quality: Quality,
    /// the page was damaged by the press
    damaged: bool,
}

fn press(app: &mut App, req: PlayerReq) -> Seen {
    let publishes = route::publishes_for_test::<DirectPlayModePref>();
    let damage = app.pages.present.page_damage_epoch();
    crate::app::playback::player_requests(
        &mut app.player, &mut app.adapters.player, &mut app.repause_at, vec![req], 0,
        &mut app.refresh_hubs, &mut app.pages, &mut app.runtime.bridge(&mut app.bridge),
    );
    seen(app, publishes, damage)
}

fn seen(app: &App, publishes: u32, damage: u64) -> Seen {
    Seen {
        pending: route::play_pending(),
        request: route::retry_rk(&app.player.session),
        origin: player::report::error_context_for_test(&app.player.session).attempt_origin,
        failure_cleared: route::failure(&app.player.session).is_none(),
        mode: route::direct_play_mode(),
        mode_publishes: route::publishes_for_test::<DirectPlayModePref>() - publishes,
        quality: route::quality(),
        damaged: app.pages.present.page_damage_epoch() != damage,
    }
}

/// Put the preferences a test moves back, and end the resolve a retry started, so nothing outlives the test.
struct Tidy(Quality, DirectPlayMode);
impl Tidy {
    fn new() -> Self { Tidy(route::quality(), route::direct_play_mode()) }
}
impl Drop for Tidy {
    fn drop(&mut self) {
        route::restore_quality(self.0);
        route::restore_direct_play_mode(self.1);
    }
}

/// **Try again** repeats the failed request unchanged, as the viewer's own retry, and touches no preference.
#[test]
fn try_again_repeats_the_request_as_a_user_retry_and_writes_no_preference() {
    let _serial = plx_base::testlock::serial();
    let _tidy = Tidy::new();
    let mut app = interrupted();
    assert!(row(&app).contains(&player::FailureAction::TryAgain), "rig: the row offers Try again: {:?}", row(&app));
    let (mode, quality) = (route::direct_play_mode(), route::quality());
    let seen = press(&mut app, PlayerRequestOf::try_again());
    assert_eq!(seen, Seen {
        pending: true, request: Some("m1".into()), origin: AttemptOrigin::UserRetry, failure_cleared: true,
        mode, mode_publishes: 0, quality, damaged: true,
    });
    route::cancel_play(&mut app.player.session);
}

/// **Play automatically** moves the Direct Play preference to Auto (published once, the one write door), then repeats
/// the request as the viewer's retry.
#[test]
fn play_automatically_persists_auto_once_and_repeats_the_request_as_a_user_retry() {
    let _serial = plx_base::testlock::serial();
    let _tidy = Tidy::new();
    route::restore_direct_play_mode(DirectPlayMode::Forced);
    let mut app = app();
    route::seed_request_for_test(&mut app.player.session, "m1");
    capture_arm(&mut app, "policy");
    assert!(row(&app).contains(&player::FailureAction::PlayAutomatically), "rig: {:?}", row(&app));
    let quality = route::quality();
    let seen = press(&mut app, PlayerRequestOf::play_automatically());
    assert_eq!(seen, Seen {
        pending: true, request: Some("m1".into()), origin: AttemptOrigin::UserRetry, failure_cleared: true,
        mode: DirectPlayMode::Auto, mode_publishes: 1, quality, damaged: true,
    });
    route::cancel_play(&mut app.player.session);
}

/// A **rung of Change quality** on a failed playback persists the rung, then repeats the request as a quality change,
/// and the press reports `true` (the popover has nothing to do but close).
#[test]
fn a_rung_on_the_failure_persists_then_repeats_the_request_as_a_quality_change() {
    let _serial = plx_base::testlock::serial();
    let _tidy = Tidy::new();
    let mut app = refused_by_the_server();
    assert!(row(&app).contains(&player::FailureAction::ChangeQuality), "rig: {:?}", row(&app));
    let rung = if route::quality() == Quality::P720 { Quality::P480 } else { Quality::P720 };
    let mode = route::direct_play_mode();
    let publishes = route::publishes_for_test::<DirectPlayModePref>();
    let damage = app.pages.present.page_damage_epoch();
    let mut present = plx_machine::present::Present::new();
    let took = crate::app::playback::apply_quality(
        &mut app.player.session, &mut app.adapters.player, &mut app.runtime.bridge(&mut app.bridge), rung, &mut present);
    assert!(took, "a rung on the failure screen is always taken");
    let seen = seen(&app, publishes, damage);
    assert_eq!((seen.pending, seen.request.as_deref(), seen.origin, seen.failure_cleared),
        (true, Some("m1"), AttemptOrigin::QualityChange, true));
    assert_eq!((seen.quality, seen.mode, seen.mode_publishes), (rung, mode, 0), "the rung is the persisted preference");
    assert!(present.page_damaged(), "the press damages the page");
    route::cancel_play(&mut app.player.session);
}

/// A rung on a failure with NOTHING to resolve again (a URL playback has no Plex request) is refused: its row draws no
/// *Change quality*, so a rung that reaches the app (a `...` popover that was already open when the failure landed, the
/// socket) is neither persisted nor retried. This is the one place the door differs from the performers it replaced,
/// which persisted the rung and reported `true` for any Error; no button of the row can reach it.
#[test]
fn a_rung_with_nothing_to_retry_is_refused_not_persisted() {
    let _serial = plx_base::testlock::serial();
    let _tidy = Tidy::new();
    let mut app = app();
    route::refuse_for_test(&mut app.player.session, PlayVerdict::Server("Cannot convert.".into(), route::DecisionCodes::default()));
    player::observe_failure(&mut app.player.session, FailureTrigger::NONE, player::report::attempt_id());
    assert!(!row(&app).contains(&player::FailureAction::ChangeQuality), "rig: {:?}", row(&app));
    let quality = route::quality();
    let rung = if quality == Quality::P720 { Quality::P480 } else { Quality::P720 };
    let mut present = plx_machine::present::Present::new();
    let took = crate::app::playback::apply_quality(
        &mut app.player.session, &mut app.adapters.player, &mut app.runtime.bridge(&mut app.bridge), rung, &mut present);
    assert!(!took, "a rung the failure does not offer is refused");
    assert_eq!(route::quality(), quality, "and not persisted");
    assert!(!route::play_pending() && !present.page_damaged(), "nothing started, nothing damaged");
}

/// A server's stop ends the playback: no recovery of any kind is taken, and nothing is written.
#[test]
fn a_server_stop_refuses_every_recovery_before_it_writes_anything() {
    let _serial = plx_base::testlock::serial();
    let _tidy = Tidy::new();
    route::restore_direct_play_mode(DirectPlayMode::Forced);
    let mut app = app();
    route::seed_request_for_test(&mut app.player.session, "m1");
    route::note_engine_failure(&mut app.player.session, player::attempt_view::RuntimeFailure::ServerStopped, 0);
    assert!(player::server_stop_applies(&app.player.session));
    player::observe_failure(&mut app.player.session, FailureTrigger::NONE, player::report::attempt_id());
    let (mode, quality) = (route::direct_play_mode(), route::quality());
    let rung = if quality == Quality::P720 { Quality::P480 } else { Quality::P720 };
    let publishes = route::publishes_for_test::<DirectPlayModePref>();
    for req in [PlayerRequestOf::try_again(), PlayerRequestOf::play_automatically()] {
        let seen = press(&mut app, req);
        assert!(!seen.pending && !seen.damaged, "a stopped playback starts nothing");
    }
    let mut present = plx_machine::present::Present::new();
    assert!(!crate::app::playback::apply_quality(
        &mut app.player.session, &mut app.adapters.player, &mut app.runtime.bridge(&mut app.bridge), rung, &mut present),
        "the rung is refused on a server stop");
    assert_eq!((route::direct_play_mode(), route::quality()), (mode, quality), "no preference moved");
    assert_eq!(route::publishes_for_test::<DirectPlayModePref>(), publishes, "and none was published");
}

/// **A rung while the playback is NOT in Error is the live ladder's**, never the failure's: the dev trigger records a
/// failure over a playing session, and its rung must not be read as a recovery.
#[test]
fn a_rung_over_a_forced_read_out_is_not_a_recovery() {
    let _serial = plx_base::testlock::serial();
    let _tidy = Tidy::new();
    let mut app = interrupted();
    assert_ne!(player::state(&app.player.session), player::PlaybackState::Error, "rig: the read-out is forced, the state is not Error");
    let mut present = plx_machine::present::Present::new();
    let rung = if route::quality() == Quality::P720 { Quality::P480 } else { Quality::P720 };
    let _ = crate::app::playback::apply_quality(
        &mut app.player.session, &mut app.adapters.player, &mut app.runtime.bridge(&mut app.bridge), rung, &mut present);
    assert!(!route::play_pending(), "the live ladder asks nothing of the resolver for a session that is not playing");
}

/// **A recovery the failure does not offer is refused, and writes nothing**: *Play automatically* on a failure that is
/// not under Force or Disabled (so its row never drew it) neither persists Auto nor retries, and the failure stands.
#[test]
fn a_recovery_the_row_does_not_offer_changes_nothing() {
    let _serial = plx_base::testlock::serial();
    let _tidy = Tidy::new();
    let mut app = interrupted();
    assert!(!row(&app).contains(&player::FailureAction::PlayAutomatically), "rig: {:?}", row(&app));
    let (mode, quality) = (route::direct_play_mode(), route::quality());
    let seen = press(&mut app, PlayerRequestOf::play_automatically());
    assert_eq!(seen, Seen {
        pending: false, request: Some("m1".into()), origin: seen.origin, failure_cleared: false,
        mode, mode_publishes: 0, quality, damaged: false,
    });
}

/// **A recovery before the failure is captured is refused**, not guessed at: an Error the frame has not captured yet
/// has no row to offer anything.
#[test]
fn a_recovery_before_the_failure_is_captured_is_refused() {
    let _serial = plx_base::testlock::serial();
    let _tidy = Tidy::new();
    let mut app = app();
    route::seed_request_for_test(&mut app.player.session, "m1");
    route::refuse_for_test(&mut app.player.session, PlayVerdict::Server("Cannot convert.".into(), route::DecisionCodes::default()));
    assert!(route::failure(&app.player.session).is_none(), "rig: nothing captured");
    let quality = route::quality();
    let rung = if quality == Quality::P720 { Quality::P480 } else { Quality::P720 };
    let mut present = plx_machine::present::Present::new();
    assert!(!crate::app::playback::apply_quality(
        &mut app.player.session, &mut app.adapters.player, &mut app.runtime.bridge(&mut app.bridge), rung, &mut present));
    assert_eq!(route::quality(), quality);
    assert!(!route::play_pending());
}

/// The requests the screen's failure row emits for its two retrying buttons. One place, so that moving the buttons onto
/// the typed recovery door changes this and nothing the tests assert.
struct PlayerRequestOf;
impl PlayerRequestOf {
    fn try_again() -> PlayerReq { PlayerReq::Recover(player::Recovery::RetrySame) }
    fn play_automatically() -> PlayerReq { PlayerReq::Recover(player::Recovery::PlayAutomatically) }
}
