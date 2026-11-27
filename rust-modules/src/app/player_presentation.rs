//! Live player presentation IO boundary. Machines receive the complete immutable answer.
use plx_appkit::player_hud::{self, Busy, FailureReadout, Presentation};
use plx_media::{player, route};

/// **The failure value's capture, at the app's IO boundary.** Samples the dev trigger `failtest` (which no machine may
/// read) and hands it, with the attempt id, to [`player::observe_failure`]: the one writer of the session's failure,
/// which records it when the attempt enters Error, keeps it while it stands and clears it when the state leaves Error.
/// Idempotent, so it is called ahead of every reader of the value, each frame: at the publication
/// (`publish_product_frame`, ahead of [`capture`]) and before each of the frame's two report observations (a refused
/// plan can land between them). A controlled host (replay, a tape) has no dev trigger and samples none.
pub(super) fn capture_failure(app: &mut super::App) {
    let sampled = app.runtime.bridge(&mut app.bridge).samples_dev_triggers();
    capture_failure_of(&mut app.player.session, sampled);
}

fn capture_failure_of(ps: &mut route::PlaybackSession, sample_trigger: bool) {
    let arm = if sample_trigger { plx_base::devtrig::read("failtest") } else { None };
    // Preserve the old exists semantics for an unreadable trigger as well as an empty file.
    let armed = sample_trigger && (arm.is_some() || plx_base::devtrig::flag("failtest"));
    player::observe_failure(ps, player::FailureTrigger { arm: arm.as_deref(), armed }, player::report::attempt_id());
}

/// The player's presentation for this frame. A failure is READ from the session (captured ahead of this by
/// [`capture_failure`]); only the three inputs that move while it stands are sampled here ([`player::live_offers`]).
pub(super) fn capture(ps: &route::PlaybackSession, meta: plx_data::metadata::MetadataView<'_>) -> Presentation {
    plx_base::step::assert_io_allowed("player presentation capture");
    let state = player::state(ps);
    if let Some(failure) = route::failure(ps) {
        let shape = failure.shape();
        let actions = player::failure_actions(failure, player::live_offers(ps, meta));
        let support = player::support_line(failure.kind());
        let readout = FailureReadout::captured(failure.attempt(), shape.clone(), actions, support, ps.repair_status);
        return Presentation { busy: Busy::Readout(plx_ui::widgets::StatusKind::Failed, shape.caption),
            failure: Some(std::sync::Arc::new(readout)) };
    }
    if state == player::PlaybackState::Error {
        // Unreachable in the loop (`capture_failure` runs ahead of every publication): refused, not guessed at.
        player::log("player presentation: an Error with no captured failure");
        debug_assert!(false, "the failure is captured (capture_failure) ahead of the presentation");
    }
    Presentation { busy: player_hud::busy_surface_with(c"", state,
        player::reported_frame(ps).unwrap_or(false), player::audio_switching()), failure: None }
}

#[cfg(all(test, feature = "devtriggers"))]
mod tests {
    use super::*;
    #[test]
    fn adjacent_live_diagnostic_arms_clears_and_pure_readout_is_coherent() {
        let _serial = plx_base::testlock::serial();
        plx_base::devtrig::with_private_triggers(|| {
            let mut ps = plx_media::route::PlaybackSession::IDLE;
            let store = plx_data::stores::metadata::MetadataStore::default();
            // a frame: the failure is captured first (the loop's `capture_failure`), then the presentation reads it
            let capture = |ps: &mut plx_media::route::PlaybackSession| {
                capture_failure_of(ps, true);
                capture(ps, store.view())
            };
            let neutral = capture(&mut ps);
            assert!(route::failure(&ps).is_none());
            let path = plx_base::devtrig::path("failtest");
            std::fs::write(&path, "policy").unwrap();
            let failed = capture(&mut ps);
            assert!(player_hud::transport_hidden(&failed));
            assert!(failed.failure.is_some());
            assert_eq!(route::failure(&ps).map(player::PlaybackFailure::kind), Some(player::FailureKind::PlaybackPolicy));
            // a different arm is a different failure: the read-out follows the switch
            std::fs::write(&path, "no_backend").unwrap();
            let switched = capture(&mut ps);
            assert_eq!(route::failure(&ps).map(player::PlaybackFailure::kind), Some(player::FailureKind::NoQualifiedBackend));
            assert_ne!(switched.failure.as_ref().unwrap().reason(), failed.failure.as_ref().unwrap().reason());
            std::fs::write(&path, "policy").unwrap();
            let failed = capture(&mut ps);
            std::fs::remove_file(&path).unwrap();
            assert_eq!(capture(&mut ps), neutral, "clearing the LIVE flag must retire the failure");
            assert!(route::failure(&ps).is_none());
            let _step = plx_base::step::StepScope::enter();
            assert!(!player_hud::transport_hidden(&neutral));
            assert!(player_hud::transport_hidden(&failed), "a captured owner does not reread the cleared flag");
            let readout = failed.failure.as_ref().unwrap();
            assert!(!readout.actions().is_empty());
            assert!(readout.frames(&plx_ui::fixture::FixtureMeasure).iter().any(Option::is_some));
            let drawn = plx_ui::record_walk(|| plx_ui::draw_census::capture(||
                player_hud::classic::draw_readout(failed.failure.as_deref(), failed.busy, 0, 0, &plx_ui::fixture::FixtureMeasure)));
            assert!(!drawn.is_empty());
            assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| super::capture(&ps, store.view()))).is_err(), "the producer is explicitly outside StepScope");
        });
    }
}
