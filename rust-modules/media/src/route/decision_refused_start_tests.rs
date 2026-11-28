//! **An attempt that fails before its first picture takes its pick back** (#1522). A start of play that
//! needs a server conversion PUTs the Part's selection BEFORE its own `/decision` (the transcoder
//! encodes what is SELECTED), and the plan hands the pair it wrote, with the pair the server held, to the
//! landing (`Plan::selection_sent`, `Plan::selection_fetched`). When that attempt then fails before it
//! showed a picture (a refused plan, a pipeline that never opened), the pick stood as the server's
//! selection and the next start of the title planned from it: a cold-start dead end. These tests drive a
//! landing against the loopback server of [`FlightRig`], observe a state the way the app's frame does
//! ([`settle_start_selection`]) and read what the server was told.

use super::flight_rig::FlightRig;
use super::test_support::apply_plan;
use super::test_support::*;
use super::*;
use crate::route::settle_start_selection;
use std::time::Duration;

/// The Part the rig's route plays, and the pair the server selected before any pick.
const PART: i64 = 960_001;
const HELD_AUDIO: i64 = 36_014;
const HELD_SUB: i64 = 55;
/// The subtitle the viewer picked on the failure screen, which the retry's resolve burns.
const BURN_SUB: i64 = 88;

fn held() -> ServerSelection {
    ServerSelection { part_id: PART, audio: Some(HELD_AUDIO), subtitle: Some(HELD_SUB) }
}

/// One session on the rig's loopback server.
struct Lane {
    ps: PlaybackSession,
    rig: FlightRig,
}

impl Lane {
    fn start() -> Lane {
        let mut ps = PlaybackSession::IDLE;
        let rig = FlightRig::start(&mut ps, Duration::from_millis(10));
        Lane { ps, rig }
    }

    /// What a start's resolve does for a viewer's pick that the server must convert: PUT it to the Part
    /// (on the serial queue, from a worker's side), then land the plan that names it. `refused` is the
    /// server's `/decision` answer: a refusal lands a plan with a verdict and no URL.
    fn land_the_pick(&mut self, refused: bool) {
        let sid = cur_sid(&self.ps);
        let sent = put_selection(&plx_base::task::OffFrame::for_test(), sid, PART, 0, BURN_SUB);
        assert!(sent.is_some(), "the loopback server took the PUT");
        apply_plan(
            &mut self.ps,
            Plan {
                sid,
                part_id: PART,
                url: if refused { String::new() } else { "https://example.invalid/start.mkv".into() },
                verdict: refused.then(|| PlayVerdict::Server("synthetic refusal".into(), DecisionCodes::default())),
                sub_sid: BURN_SUB,
                selection_fetched: Some(held()),
                selection_sent: sent,
                ..Default::default()
            },
            "960001",
        );
    }

    /// The selection PUTs the server has seen, in order.
    fn puts(&self) -> Vec<String> {
        self.rig.requests().into_iter().filter(|r| r.starts_with("PUT ")).collect()
    }

    fn wait_for_puts(&self, n: usize) {
        self.rig.wait_for("the selection PUTs", |r| r.requests().iter().filter(|l| l.starts_with("PUT ")).count() >= n);
    }

    /// The frame's step, as the app runs it (after the capture of the failure).
    fn frame(&mut self) {
        settle_start_selection(&mut self.ps);
    }

    fn selection(&self) -> Option<(Option<i64>, Option<i64>)> {
        self.ps.server_selection.map(|s| (s.audio, s.subtitle))
    }

    fn finish(self) {
        let Lane { rig, .. } = self;
        let _serial = rig.close();
        reset_player_control_for_test(idle_session_for_test());
    }
}

fn param<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    query_param(line, key)
}

/// **The server half.** A retry carrying the failure screen's pick PUTs it before its `/decision`; the
/// server refuses the conversion; the pick must not stay the Part's selection.
#[test]
fn a_refused_start_puts_the_servers_selection_back() {
    let mut lane = Lane::start();
    lane.land_the_pick(true);
    assert_eq!(crate::player::state(&lane.ps), crate::player::PlaybackState::Error, "premise: the plan was refused");
    assert_eq!(lane.selection(), Some((Some(HELD_AUDIO), Some(BURN_SUB))), "premise: the pick is what the server holds now");
    lane.frame();
    lane.wait_for_puts(2);
    plx_base::task::drain_workers_for_test();

    let puts = lane.puts();
    assert_eq!(puts.len(), 2, "the pick's PUT, then its undoing: {puts:?}");
    assert_eq!(param(&puts[0], "subtitleStreamID"), Some("88"), "{}", puts[0]);
    assert_eq!(param(&puts[1], "subtitleStreamID"), Some("55"), "the earlier subtitle goes back: {}", puts[1]);
    assert_eq!(param(&puts[1], "audioStreamID"), None, "the audio never changed: {}", puts[1]);
    assert_eq!(lane.selection(), Some((Some(HELD_AUDIO), Some(HELD_SUB))), "the value the server holds again");

    // A second frame of the same failure writes nothing more.
    lane.frame();
    plx_base::task::drain_workers_for_test();
    assert_eq!(lane.puts().len(), 2, "taken once: {:?}", lane.puts());
    lane.finish();
}

/// **An accepted start whose pipeline then fails before a picture** is undone too: the plan was
/// playable, so the failure arrives as an engine's, not a verdict.
#[test]
fn a_pipeline_failure_before_the_first_frame_restores_the_selection() {
    let mut lane = Lane::start();
    lane.land_the_pick(false);
    lane.frame();
    plx_base::task::drain_workers_for_test();
    assert_eq!(lane.puts().len(), 1, "nothing is undone while the attempt is still opening: {:?}", lane.puts());

    note_engine_failure(&mut lane.ps, crate::player::RuntimeFailure::PlaybackInterrupted, 0);
    assert_eq!(crate::player::state(&lane.ps), crate::player::PlaybackState::Error);
    lane.frame();
    lane.wait_for_puts(2);
    plx_base::task::drain_workers_for_test();
    let puts = lane.puts();
    assert_eq!(puts.len(), 2, "{puts:?}");
    assert_eq!(param(&puts[1], "subtitleStreamID"), Some("55"), "{}", puts[1]);
    lane.finish();
}

/// **The first picture proves the pick.** Once the attempt has played, a later failure (a stall minutes in)
/// leaves the viewer's selection where it is: it was on screen.
#[test]
fn a_failure_after_the_first_frame_restores_nothing() {
    let mut lane = Lane::start();
    lane.land_the_pick(false);
    lane.ps.step_readout(crate::player::readout::ReadoutEvent::StateDerived(crate::player::PlaybackState::Playing));
    assert!(crate::player::is_playing(&lane.ps), "premise: a picture is presented");
    lane.frame();

    note_engine_failure(&mut lane.ps, crate::player::RuntimeFailure::PlaybackInterrupted, 60_000_000_000);
    assert_eq!(crate::player::state(&lane.ps), crate::player::PlaybackState::Error);
    lane.frame();
    plx_base::task::drain_workers_for_test();
    assert_eq!(lane.puts().len(), 1, "only the pick's own PUT: {:?}", lane.puts());
    assert_eq!(lane.selection(), Some((Some(HELD_AUDIO), Some(BURN_SUB))));
    lane.finish();
}

/// **A revert costs one extra PUT and nothing else.** A transient failure (a stall) reverts the server's
/// selection like any other, but the viewer's pick stays what the retry carries, and the retry's resolve
/// PUTs it again before its own `/decision`.
#[test]
fn a_retry_after_a_transient_revert_still_plays_the_pick() {
    let mut lane = Lane::start();
    lane.land_the_pick(false);
    note_engine_failure(&mut lane.ps, crate::player::RuntimeFailure::PlaybackInterrupted, 0);
    lane.frame();
    lane.wait_for_puts(2);
    assert_eq!(cur_sub_sid(&lane.ps), BURN_SUB, "the revert is the server's only: the pick on screen and carried stays");

    // The retry: the same pick, resolved again over the item the server now describes as held.
    let sid = cur_sid(&lane.ps);
    let sent = put_selection(&plx_base::task::OffFrame::for_test(), sid, PART, 0, BURN_SUB);
    apply_plan(
        &mut lane.ps,
        Plan {
            sid,
            part_id: PART,
            url: "https://example.invalid/start.mkv".into(),
            sub_sid: BURN_SUB,
            selection_fetched: Some(held()),
            selection_sent: sent,
            ..Default::default()
        },
        "960001",
    );
    plx_base::task::drain_workers_for_test();
    let puts = lane.puts();
    assert_eq!(puts.len(), 3, "pick, revert, pick again: {puts:?}");
    assert_eq!(
        puts.iter().map(|p| param(p, "subtitleStreamID")).collect::<Vec<_>>(),
        [Some("88"), Some("55"), Some("88")],
        "{puts:?}"
    );
    assert_eq!(lane.selection(), Some((Some(HELD_AUDIO), Some(BURN_SUB))), "the retry's own record");

    // …and the retry, on playing, is no longer provisional: its failure later undoes nothing.
    lane.ps.step_readout(crate::player::readout::ReadoutEvent::StateDerived(crate::player::PlaybackState::Playing));
    lane.frame();
    note_engine_failure(&mut lane.ps, crate::player::RuntimeFailure::PlaybackInterrupted, 0);
    lane.frame();
    plx_base::task::drain_workers_for_test();
    assert_eq!(lane.puts().len(), 3, "{:?}", lane.puts());
    lane.finish();
}

impl Lane {
    /// The retry of a failed start that the viewer's pick survived into, planned as a DIRECT play: its resolve
    /// converts nothing, so it PUTs nothing, and the pick (`sub_sid`) is drawn by the app. `fetched` is what the
    /// server describes the Part as holding.
    fn land_a_direct_retry(&mut self, sub_sid: i64) {
        let sid = cur_sid(&self.ps);
        apply_plan(
            &mut self.ps,
            Plan {
                sid,
                part_id: PART,
                url: "https://example.invalid/part.mkv".into(),
                sub_sid,
                selection_fetched: Some(held()),
                selection_sent: None,
                ..Default::default()
            },
            "960001",
        );
    }

    fn play(&mut self) {
        self.ps.step_readout(crate::player::readout::ReadoutEvent::StateDerived(crate::player::PlaybackState::Playing));
        assert!(crate::player::is_playing(&self.ps), "premise: a picture is presented");
        self.frame();
    }
}

/// **A revert must not strand a pick the retry plays without converting.** A start burns subtitle 88 and PUTs it,
/// fails before a picture, and the pick is written back (55). The retry is a direct play: it PUTs nothing and
/// draws 88 itself. The first picture of a landing that plays what was taken back puts it on the server again, so
/// the next cold start of the movie (no show memory to fall back on) plans from 88 and not 55.
#[test]
fn a_direct_play_retry_that_plays_puts_the_pick_back() {
    let mut lane = Lane::start();
    lane.land_the_pick(false);
    note_engine_failure(&mut lane.ps, crate::player::RuntimeFailure::PlaybackInterrupted, 0);
    lane.frame();
    lane.wait_for_puts(2);
    lane.land_a_direct_retry(BURN_SUB);
    lane.frame();
    plx_base::task::drain_workers_for_test();
    assert_eq!(lane.puts().len(), 2, "nothing is written while the retry is still opening: {:?}", lane.puts());

    lane.play();
    lane.wait_for_puts(3);
    plx_base::task::drain_workers_for_test();
    let puts = lane.puts();
    assert_eq!(puts.len(), 3, "pick, revert, pick again: {puts:?}");
    assert_eq!(param(&puts[2], "subtitleStreamID"), Some("88"), "{}", puts[2]);
    assert_eq!(param(&puts[2], "audioStreamID"), None, "the audio never moved: {}", puts[2]);
    assert_eq!(lane.selection(), Some((Some(HELD_AUDIO), Some(BURN_SUB))));

    // Once: a later frame, or a later failure, writes nothing more.
    lane.frame();
    note_engine_failure(&mut lane.ps, crate::player::RuntimeFailure::PlaybackInterrupted, 0);
    lane.frame();
    plx_base::task::drain_workers_for_test();
    assert_eq!(lane.puts().len(), 3, "{:?}", lane.puts());
    lane.finish();
}

/// **Nothing extra for a play whose pick the server already holds**, and nothing for a retry that plays something
/// else than the pick that was taken back: the viewer's own newer choice is not ours to overwrite.
#[test]
fn a_play_that_matches_the_server_sends_nothing_extra() {
    // a normal start: its PUT is proven by the picture
    let mut lane = Lane::start();
    lane.land_the_pick(false);
    lane.play();
    plx_base::task::drain_workers_for_test();
    assert_eq!(lane.puts().len(), 1, "only the start's own PUT: {:?}", lane.puts());
    lane.finish();

    // a retry that plays another subtitle than the one taken back
    let mut lane = Lane::start();
    lane.land_the_pick(false);
    note_engine_failure(&mut lane.ps, crate::player::RuntimeFailure::PlaybackInterrupted, 0);
    lane.frame();
    lane.wait_for_puts(2);
    lane.land_a_direct_retry(HELD_SUB);
    lane.play();
    plx_base::task::drain_workers_for_test();
    assert_eq!(lane.puts().len(), 2, "the retry plays what the server holds: {:?}", lane.puts());
    lane.finish();
}

/// **The real resolve, end to end.** A 720p re-encode of an item whose server selection is English audio and
/// subtitle 9, started with subtitles Off: `build_stream` PUTs the selection (audio kept, subtitle Off) before its
/// `/decision`, the server refuses the conversion, and the REFUSED plan still carries what the landing needs (the
/// verdict, the PUT it sent, what the server held before it, the Part and the server). Landed and captured as a
/// frame does, the server's subtitle goes back to 9.
#[test]
fn a_refused_resolve_hands_its_put_to_the_landing_and_is_undone() {
    const AUDIO: i64 = 2669;
    const SELECTED_SUB: i64 = 9;
    const REAL_PART: i64 = 36_013;
    let mut lane = Lane::start();
    lane.rig.refuse_decisions(16);
    restore_quality(Quality::P720);
    let sid = cur_sid(&lane.ps);
    let mut env = ResolveEnv::snapshot(&lane.ps, plx_data::stores::metadata::MetadataStore::default().view(), sid, "rk-4k");
    env.quality = Quality::P720;
    env.src_kbps = 48_000;
    env.audio_sid = 0;
    env.sub_sid = 0;
    let stream = |id, kind: &str| plx_data::metadata::Stream {
        id, index: 0, lang_code: "eng".into(), codec: kind.into(), channels: 6, selected: true, ..Default::default()
    };
    env.cached_item = Some(fourk_item_with_subs(sid, vec![stream(AUDIO, "dca")], vec![stream(SELECTED_SUB, "srt")]));
    let plan = build_stream(&plx_base::task::OffFrame::for_test(), "rk-4k", "/library/parts/36013/1/file.mkv", "hevc", "dca", &env);

    assert!(matches!(plan.verdict, Some(PlayVerdict::Server(..))), "the server refused the conversion: {:?}", plan.verdict);
    assert!(plan.url.is_empty(), "a refused plan has nothing to play");
    assert_eq!((plan.sid, plan.part_id), (sid, REAL_PART));
    assert_eq!(
        plan.selection_fetched,
        Some(ServerSelection { part_id: REAL_PART, audio: Some(AUDIO), subtitle: Some(SELECTED_SUB) }),
        "what the server held before the resolve's PUT"
    );
    assert_eq!(
        plan.selection_sent,
        Some(ServerSelection { part_id: REAL_PART, audio: Some(AUDIO), subtitle: Some(0) }),
        "the PUT the resolve sent before its refused /decision"
    );

    apply_plan(&mut lane.ps, plan, "rk-4k");
    assert_eq!(crate::player::state(&lane.ps), crate::player::PlaybackState::Error);
    lane.frame();
    lane.wait_for_puts(2);
    plx_base::task::drain_workers_for_test();
    let puts = lane.puts();
    assert_eq!(puts.len(), 2, "the resolve's PUT, then its undoing: {puts:?}");
    assert_eq!(param(&puts[0], "subtitleStreamID"), Some("0"), "{}", puts[0]);
    assert_eq!(param(&puts[1], "subtitleStreamID"), Some("9"), "the selected subtitle goes back: {}", puts[1]);
    assert_eq!(param(&puts[1], "audioStreamID"), None, "the audio never changed: {}", puts[1]);
    lane.finish();
}
