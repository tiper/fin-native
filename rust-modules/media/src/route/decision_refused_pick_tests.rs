//! **A pick the server refuses is not kept** (#1522). A track pick that needs a server conversion is
//! PUT to the Part's selection before `/decision` answers, and its memo used to be written at commit.
//! When the server refused the conversion only the local projection was restored: the refused pick
//! stayed the server's selection and the show's memory, and the next start of the title planned from
//! it and failed the same way. These tests drive a pick through a claim's flight against the loopback
//! server of [`FlightRig`] and read what the server was told and what the profile remembered.

use super::flight_rig::FlightRig;
use super::test_support::*;
use super::*;
use plx_plex::plex::session::prefs::{AudioMemo, SubtitleMemo, SubtitleTrackMemo, TrackMemoryEntry};
use plx_plex::plex::session::{self as session, Session};
use std::time::Duration;

/// The machine the rig registers, the show the playing episode belongs to, and the Part.
const MACHINE: &str = "flight-rig";
const SHOW: &str = "4242";
const PART: i64 = 960_001;
/// The audio and subtitle the server selected before any pick, and the subtitle the viewer burns.
const HELD_AUDIO: i64 = 36_014;
const HELD_SUB: i64 = 55;
const BURN_SUB: i64 = 88;
const OFFSET_SECS: i64 = 60;

fn english_track() -> SubtitleMemo {
    SubtitleMemo::Track(SubtitleTrackMemo { lang: "eng".into(), kind: "image".into(), ..Default::default() })
}

fn japanese_eac3() -> AudioMemo {
    AudioMemo { lang: "jpn".into(), codec: "eac3".into(), channels: 6, ..Default::default() }
}

fn eac3(sid: i64, ordinal: i32) -> CarriedAudio {
    CarriedAudio { sid, ordinal, codec: "eac3".into(), channels: 6, can_normalize_loudness: false, immersive: false, sample_rate: 0, bit_depth: 0 }
}

/// One playing episode of [`SHOW`] on the rig's loopback server, under a seated profile.
struct Lane {
    ps: PlaybackSession,
    /// Declared before the rig: a failing test drops the fields in order, and the fixture's drop
    /// needs the crate-wide lock the rig still holds.
    session: session::TempSession,
    rig: FlightRig,
}

impl Lane {
    fn start(shape: fn(&mut PlaybackSession, Duration) -> FlightRig) -> Lane {
        let mut ps = PlaybackSession::IDLE;
        let rig = shape(&mut ps, Duration::from_millis(10));
        let session = session::TempSession::new("refused-pick");
        session::save(&Session { client_id: "cid-1".into(), account_token: "acct".into(), ..Default::default() });
        session.seated("", "acct");
        seed_memory_for_test::<TrackMemoryEntry>(&session::peek());
        ps.cur_kind = "episode".into();
        ps.cur_show_rk = SHOW.into();
        ps.server_selection = Some(ServerSelection { part_id: PART, audio: Some(HELD_AUDIO), subtitle: Some(HELD_SUB) });
        ps.cur_sub_sid = HELD_SUB;
        ps.cur_audio = Some(eac3(HELD_AUDIO, 1));
        // What is shown is what the reducer last applied: a refused claim restores it.
        commit_in_place_route_projection(&ps, false);
        Lane { ps, rig, session }
    }

    fn remembered(&self) -> Option<TrackMemoryEntry> {
        recall::<TrackMemoryEntry>("", &(MACHINE.to_string(), SHOW.to_string()))
    }

    fn selection(&self) -> Option<(Option<i64>, Option<i64>)> {
        self.ps.server_selection.map(|s| (s.audio, s.subtitle))
    }

    /// The selection PUTs the server has seen, in order.
    fn puts(&self) -> Vec<String> {
        self.rig.requests().into_iter().filter(|r| r.starts_with("PUT ")).collect()
    }

    fn wait_for_puts(&self, n: usize) {
        self.rig.wait_for("the selection PUTs", |r| r.requests().iter().filter(|l| l.starts_with("PUT ")).count() >= n);
    }

    /// A burn pick: an embedded picture subtitle the app cannot draw, so the server rebuilds the stream.
    fn burn_pick(&mut self) {
        commit_subtitle_pick(&mut self.ps, 2, BURN_SUB, false, Some(english_track()));
    }

    /// Drain the claim's landing the way the pump does, waiting (bounded) for the worker.
    fn await_landing(&mut self) -> (ClaimedRouteAction, ClaimTail) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            {
                let _frame = plx_base::task::FrameScope::enter();
                if let Some((action, tail, ..)) = take_ready_retranscode_claim(&mut self.ps) {
                    return (action, tail);
                }
            }
            assert!(std::time::Instant::now() < deadline, "timed out waiting for the claim's landing");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// The pump's settlement of a landed claim's tail: `true` when the server's answer was taken.
    fn settle(&mut self, action: &ClaimedRouteAction, tail: ClaimTail) -> bool {
        match tail {
            ClaimTail::Retranscode | ClaimTail::NativeAudio | ClaimTail::Adaptive => {
                finish_route_action(&mut self.ps, action, RouteApplyResult::Prepared)
            }
            // The trial's own `PendingOriginal` owns commit and rollback from here.
            ClaimTail::Original(_) => true,
            ClaimTail::Rejected(_) => {
                finish_route_action(&mut self.ps, action, RouteApplyResult::Rejected);
                false
            }
        }
    }

    /// Claim the queued pick, fly it, land it and settle its tail: `true` when it was accepted.
    fn fly(&mut self) -> bool {
        let action = claim_route_action().expect("the pick queued a claim");
        assert!(
            matches!(execute_retranscode_claim(&mut self.ps, &action, OFFSET_SECS, -1, 0), RetranscodeClaimDispatch::Pending),
            "the claim flew on a worker"
        );
        let (action, tail) = self.await_landing();
        self.settle(&action, tail)
    }

    /// Leave the process as the next test expects it: the rig closed under its lock, the memory
    /// unseeded and the reducer idle.
    fn finish(self) {
        let Lane { rig, session, .. } = self;
        let _serial = rig.close();
        drop(take_pending_original());
        drop(take_claim_landing());
        reset_player_control_for_test(idle_session_for_test());
        unseed_memory_for_test::<TrackMemoryEntry>();
        drop(session);
    }
}

fn is_rejected(tail: &ClaimTail) -> bool {
    matches!(tail, ClaimTail::Rejected(_))
}

/// A selection PUT line's `key` parameter.
fn param<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    query_param(line, key)
}

/// **The server half.** A burn pick whose rebuild the server refuses leaves the server's selection
/// where it was: the pick's PUT, then one write of the earlier subtitle, and no audio (unchanged).
#[test]
fn a_refused_rebuild_pick_puts_the_previous_selection_back() {
    let mut lane = Lane::start(FlightRig::start);
    lane.rig.refuse_decisions(1);
    lane.burn_pick();
    assert!(!lane.fly(), "the server refused the rebuild");
    lane.wait_for_puts(2);
    plx_base::task::drain_workers_for_test();

    let puts = lane.puts();
    assert_eq!(puts.len(), 2, "the pick's PUT, then its undoing: {puts:?}");
    assert_eq!(param(&puts[0], "subtitleStreamID"), Some("88"), "{}", puts[0]);
    assert_eq!(param(&puts[1], "subtitleStreamID"), Some("55"), "the earlier subtitle goes back: {}", puts[1]);
    assert_eq!(param(&puts[1], "audioStreamID"), None, "the audio never changed: {}", puts[1]);
    assert_eq!(lane.selection(), Some((Some(HELD_AUDIO), Some(HELD_SUB))), "the value the server holds again");
    lane.finish();
}

/// A restore names only what it knows: with the earlier selection unknown it sends nothing, rather
/// than a guess (a guessed pair would be a new bad write).
#[test]
fn a_refused_pick_with_no_known_earlier_selection_sends_nothing() {
    let mut lane = Lane::start(FlightRig::start);
    lane.ps.server_selection = None;
    lane.rig.refuse_decisions(1);
    lane.burn_pick();
    assert!(!lane.fly());
    plx_base::task::drain_workers_for_test();
    assert_eq!(lane.puts().len(), 1, "only the pick's own PUT: {:?}", lane.puts());
    lane.finish();
}

/// An earlier audio that is unknown is not restored, and the known subtitle half still is: pinned
/// because the wire drops `audioStreamID` for 0.
#[test]
fn a_restore_with_the_audio_unknown_puts_back_only_the_subtitle() {
    let mut lane = Lane::start(FlightRig::start);
    lane.ps.server_selection = Some(ServerSelection { part_id: PART, audio: None, subtitle: Some(HELD_SUB) });
    lane.rig.refuse_decisions(1);
    lane.burn_pick();
    assert!(!lane.fly());
    lane.wait_for_puts(2);
    let puts = lane.puts();
    assert_eq!(param(&puts[1], "subtitleStreamID"), Some("55"), "{}", puts[1]);
    assert_eq!(param(&puts[1], "audioStreamID"), None, "{}", puts[1]);
    lane.finish();
}

/// **The memory half.** The pick that rode the refused rebuild is not the show's memory.
#[test]
fn a_refused_rebuild_pick_is_not_remembered_for_the_show() {
    let mut lane = Lane::start(FlightRig::start);
    lane.rig.refuse_decisions(1);
    lane.burn_pick();
    assert_eq!(lane.remembered(), None, "nothing is remembered before the server has agreed");
    assert!(!lane.fly());
    assert_eq!(lane.remembered(), None, "the refused pick was never the show's choice");
    lane.finish();
}

/// The same pick, accepted: remembered when its claim is prepared, and the server's selection is the
/// pick's.
#[test]
fn an_accepted_rebuild_pick_is_remembered_when_its_claim_is_prepared() {
    let mut lane = Lane::start(FlightRig::start);
    lane.burn_pick();
    assert_eq!(lane.remembered(), None, "still only a pick");
    assert!(lane.fly(), "the server accepted the rebuild");
    assert_eq!(lane.remembered(), Some(TrackMemoryEntry::new(MACHINE, SHOW, None, Some(english_track()))));
    assert_eq!(lane.selection(), Some((Some(HELD_AUDIO), Some(BURN_SUB))), "the PUT the server answered");
    lane.finish();
}

/// A native pick asks the server for no conversion, so nothing can refuse it: remembered at commit.
#[test]
fn a_native_pick_is_remembered_at_commit() {
    let mut lane = Lane::start(FlightRig::start_direct);
    assert!(commit_audio_pick(&mut lane.ps, eac3(36_016, 1), Some(japanese_eac3())));
    assert_eq!(
        lane.remembered(),
        Some(TrackMemoryEntry::new(MACHINE, SHOW, Some(japanese_eac3()), None)),
        "written at commit, as before",
    );
    lane.finish();
}

/// A pick made while its claim's answer is outstanding is superseded (the Engine is torn down for an
/// app switch): nothing is written back and nothing is remembered, the pick being owed again to the
/// next claim.
#[test]
fn a_superseded_pick_restores_nothing() {
    let mut lane = Lane::start(FlightRig::start);
    lane.rig.refuse_decisions(1);
    lane.rig.hold_answers();
    lane.burn_pick();
    let action = claim_route_action().expect("claim");
    assert!(matches!(
        execute_retranscode_claim(&mut lane.ps, &action, OFFSET_SECS, -1, 0),
        RetranscodeClaimDispatch::Pending
    ));
    lane.wait_for_puts(1);
    begin_engine_teardown(true);
    lane.rig.release_answers();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while take_ready_flight_count(&mut lane.ps) == 0 && std::time::Instant::now() < deadline {
        discard_stale_flight_landing();
        std::thread::sleep(Duration::from_millis(2));
    }
    plx_base::task::drain_workers_for_test();

    assert_eq!(lane.puts().len(), 1, "nothing was written back for a claim nobody owns: {:?}", lane.puts());
    assert_eq!(lane.remembered(), None);
    assert!(pending_user_route_intent(UserRouteIntent::Retranscode), "the pick is owed again");
    lane.finish();
}

/// How many landings `take_ready_flight` yields on this frame (0 for none or a discarded stale one).
fn take_ready_flight_count(ps: &mut PlaybackSession) -> usize {
    let _frame = plx_base::task::FrameScope::enter();
    usize::from(take_ready_flight(ps, false).is_some())
}

/// A pick made while an earlier claim flies keeps its memo until its own claim lands: only the claim
/// that landed wrote.
#[test]
fn a_pick_queued_behind_a_flight_keeps_its_memo_until_its_own_claim_lands() {
    let mut lane = Lane::start(FlightRig::start);
    lane.burn_pick();
    let first = claim_route_action().expect("claim");
    assert!(matches!(
        execute_retranscode_claim(&mut lane.ps, &first, OFFSET_SECS, -1, 0),
        RetranscodeClaimDispatch::Pending
    ));
    // The viewer then picks audio while the first claim's answer is outstanding.
    let spanish = AudioMemo { lang: "spa".into(), ..japanese_eac3() };
    assert!(commit_audio_pick(&mut lane.ps, eac3(36_016, 2), Some(spanish.clone())));
    let (action, tail) = lane.await_landing();
    assert!(lane.settle(&action, tail), "the burn was accepted");
    // The prepared route starts, which is what lets the next pick's claim begin.
    {
        let _frame = plx_base::task::FrameScope::enter();
        settle_pending_native_start(&mut lane.ps, RouteStartResult::Started);
    }
    assert_eq!(
        lane.remembered(),
        Some(TrackMemoryEntry::new(MACHINE, SHOW, None, Some(english_track()))),
        "only the claim that landed wrote: the audio pick is still waiting for its own",
    );
    assert!(lane.fly(), "the audio rebuild is accepted");
    assert_eq!(
        lane.remembered(),
        Some(TrackMemoryEntry::new(MACHINE, SHOW, Some(spanish), Some(english_track()))),
    );
    lane.finish();
}

/// **The race.** A pick made in place while the refused claim flew is the newer word: the restore does
/// not undo it.
#[test]
fn a_native_pick_after_a_refused_claim_wins() {
    let mut lane = Lane::start(FlightRig::start);
    lane.rig.refuse_decisions(1);
    lane.rig.hold_answers();
    lane.burn_pick();
    let action = claim_route_action().expect("claim");
    assert!(matches!(
        execute_retranscode_claim(&mut lane.ps, &action, OFFSET_SECS, -1, 0),
        RetranscodeClaimDispatch::Pending
    ));
    lane.wait_for_puts(1);
    // While the answer is outstanding the viewer turns subtitles Off: applied in place on the remux,
    // PUT through the serial queue.
    commit_subtitle_pick(&mut lane.ps, -1, 0, false, Some(SubtitleMemo::Off));
    lane.rig.release_answers();
    let (action, tail) = lane.await_landing();
    assert!(is_rejected(&tail), "{tail:?}");
    lane.settle(&action, tail);
    lane.wait_for_puts(2);
    plx_base::task::drain_workers_for_test();

    let puts = lane.puts();
    assert_eq!(puts.len(), 2, "no restore after the newer pick: {puts:?}");
    assert_eq!(param(&puts[1], "subtitleStreamID"), Some("0"), "Off is the last word: {puts:?}");
    lane.finish();
}

/// **The race, per field.** An in-place subtitle pick queued while a refused AUDIO rebuild flies carries
/// the pending audio in its own PUT, so the selection the main thread holds has moved on both fields. The
/// audio is still the refused pick's and is put back; the subtitle is the newer word and is kept.
#[test]
fn a_refused_audio_rebuild_is_undone_under_an_in_place_subtitle_pick() {
    let mut lane = Lane::start(FlightRig::start);
    lane.rig.refuse_decisions(1);
    lane.rig.hold_answers();
    let spanish = AudioMemo { lang: "spa".into(), ..japanese_eac3() };
    assert!(commit_audio_pick(&mut lane.ps, eac3(36_016, 2), Some(spanish)));
    let action = claim_route_action().expect("claim");
    assert!(matches!(
        execute_retranscode_claim(&mut lane.ps, &action, OFFSET_SECS, -1, 0),
        RetranscodeClaimDispatch::Pending
    ));
    lane.wait_for_puts(1);
    commit_subtitle_pick(&mut lane.ps, -1, 0, false, Some(SubtitleMemo::Off));
    lane.rig.release_answers();
    let (action, tail) = lane.await_landing();
    assert!(is_rejected(&tail), "{tail:?}");
    lane.settle(&action, tail);
    lane.wait_for_puts(3);
    plx_base::task::drain_workers_for_test();

    let puts = lane.puts();
    assert_eq!(puts.len(), 3, "the pick's PUT, the in-place PUT, the restore: {puts:?}");
    assert_eq!(param(&puts[0], "audioStreamID"), Some("36016"), "{}", puts[0]);
    assert_eq!(param(&puts[1], "subtitleStreamID"), Some("0"), "{}", puts[1]);
    assert_eq!(param(&puts[2], "audioStreamID"), Some("36014"), "the refused audio goes back: {}", puts[2]);
    assert_eq!(param(&puts[2], "subtitleStreamID"), None, "the newer subtitle is left alone: {}", puts[2]);
    assert_eq!(lane.selection(), Some((Some(HELD_AUDIO), Some(0))), "the audio is the earlier one, the subtitle the viewer's Off");
    assert_eq!(
        lane.remembered(),
        Some(TrackMemoryEntry::new(MACHINE, SHOW, None, Some(SubtitleMemo::Off))),
        "the refused audio pick is not remembered, the applied Off is"
    );
    lane.finish();
}

/// **Newest pick wins in the memory too.** A subtitle applied in place while an older burn's claim flies
/// is the viewer's latest choice: the burn's claim, accepted afterwards, must not overwrite it.
#[test]
fn an_accepted_rebuild_does_not_overwrite_a_newer_in_place_pick() {
    let mut lane = Lane::start(FlightRig::start);
    lane.rig.hold_answers();
    lane.burn_pick();
    let action = claim_route_action().expect("claim");
    assert!(matches!(
        execute_retranscode_claim(&mut lane.ps, &action, OFFSET_SECS, -1, 0),
        RetranscodeClaimDispatch::Pending
    ));
    lane.wait_for_puts(1);
    commit_subtitle_pick(&mut lane.ps, -1, 0, false, Some(SubtitleMemo::Off));
    assert_eq!(lane.remembered(), Some(TrackMemoryEntry::new(MACHINE, SHOW, None, Some(SubtitleMemo::Off))), "Off applied at once");
    lane.rig.release_answers();
    let (action, tail) = lane.await_landing();
    assert!(!is_rejected(&tail), "{tail:?}");
    assert!(lane.settle(&action, tail), "the older burn was accepted");
    assert_eq!(
        lane.remembered(),
        Some(TrackMemoryEntry::new(MACHINE, SHOW, None, Some(SubtitleMemo::Off))),
        "the viewer's newer Off stays the show's memory"
    );
    lane.finish();
}

/// The same for audio, at the reducer: a rebuild pick still waiting for its claim is superseded by a
/// native pick of the same half made before the claim, which writes at once.
#[test]
fn an_accepted_rebuild_does_not_overwrite_a_newer_native_audio_pick() {
    let mut lane = Lane::start(FlightRig::start_direct);
    let spanish = AudioMemo { lang: "spa".into(), codec: "dts".into(), ..japanese_eac3() };
    let dts = CarriedAudio { codec: "dts".into(), channels: 6, ..eac3(36_016, 2) };
    assert!(commit_audio_pick(&mut lane.ps, dts, Some(spanish)));
    assert_eq!(lane.remembered(), None, "the conversion's pick waits for its claim");
    assert!(commit_audio_pick(&mut lane.ps, eac3(36_017, 3), Some(japanese_eac3())));
    assert_eq!(lane.remembered(), Some(TrackMemoryEntry::new(MACHINE, SHOW, Some(japanese_eac3()), None)), "the native pick writes at once");
    let action = claim_route_action().expect("claim");
    assert!(finish_route_action(&mut lane.ps, &action, RouteApplyResult::Prepared));
    assert_eq!(
        lane.remembered(),
        Some(TrackMemoryEntry::new(MACHINE, SHOW, Some(japanese_eac3()), None)),
        "the older conversion's memo was superseded"
    );
    lane.finish();
}

/// **One ordered channel.** A claim worker's PUT is queued behind what the serial selection worker holds
/// (a refused claim's restore, an in-place pick) and the worker waits for it: sent directly it could
/// overtake the restore queued before it, and the server would end on the pick the restore was undoing.
#[test]
fn a_claim_workers_put_waits_behind_a_queued_restore() {
    let lane = Lane::start(FlightRig::start);
    let sid = cur_sid(&lane.ps);
    lane.rig.hold_answers();
    // The serial worker is busy with an earlier PUT (held at the server), the restore waits behind it.
    assert!(queue_selection(sid, PART, 36_015, Some(HELD_SUB), false));
    lane.wait_for_puts(1);
    assert!(queue_selection(sid, PART, HELD_AUDIO, None, false), "the restore");
    let claim = std::thread::spawn(move || {
        put_selection(&plx_base::task::OffFrame::for_test(), sid, PART, 36_017, HELD_SUB)
    });
    let deadline = std::time::Instant::now() + Duration::from_millis(300);
    while selection_queue().pending.len() < 2 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    lane.rig.release_answers();
    assert!(claim.join().expect("the claim worker").is_some(), "the PUT was answered");
    lane.wait_for_puts(3);
    let audio: Vec<_> = lane.puts().iter().map(|p| param(p, "audioStreamID").unwrap_or("-").to_owned()).collect();
    assert_eq!(audio, ["36015", "36014", "36017"], "the restore is sent before the claim's PUT: {:?}", lane.puts());
    lane.finish();
}

/// A refused rebuild with a second rebuild pick queued behind it: the restore is sent first and the
/// server ends on the second pick, which is what `server_selection` says it holds.
#[test]
fn a_second_rebuild_pick_queued_behind_a_refused_one_ends_the_server_on_it() {
    let mut lane = Lane::start(FlightRig::start);
    lane.rig.refuse_decisions(1);
    assert!(commit_audio_pick(&mut lane.ps, eac3(36_016, 2), Some(japanese_eac3())));
    let action = claim_route_action().expect("claim");
    assert!(matches!(
        execute_retranscode_claim(&mut lane.ps, &action, OFFSET_SECS, -1, 0),
        RetranscodeClaimDispatch::Pending
    ));
    let (action, tail) = lane.await_landing();
    assert!(is_rejected(&tail), "{tail:?}");
    lane.settle(&action, tail);
    // The refusal queued its restore; the viewer's next pick is claimed at once, while it may still be
    // on its way to the server.
    let spanish = AudioMemo { lang: "spa".into(), ..japanese_eac3() };
    assert!(commit_audio_pick(&mut lane.ps, eac3(36_017, 3), Some(spanish.clone())));
    assert!(lane.fly(), "the second pick's rebuild is accepted");
    plx_base::task::drain_workers_for_test();

    let audio: Vec<_> = lane.puts().iter().map(|p| param(p, "audioStreamID").unwrap_or("-").to_owned()).collect();
    assert_eq!(audio, ["36016", "36014", "36017"], "pick, its restore, then the second pick: {:?}", lane.puts());
    assert_eq!(lane.selection(), Some((Some(36_017), Some(HELD_SUB))), "what the server holds");
    assert_eq!(
        lane.remembered(),
        Some(TrackMemoryEntry::new(MACHINE, SHOW, Some(spanish), None)),
        "only the pick the server accepted is remembered"
    );
    lane.finish();
}

/// **An in-place subtitle PUT names the subtitle only.** Burn C flies, audio N waits for its claim, an
/// in-place Off is PUT: were that PUT to repeat the pending audio, the server would hold N with no claim
/// to undo it, and N's claim would capture it as the earlier selection and restore nothing when refused.
#[test]
fn an_in_place_subtitle_put_does_not_write_a_pending_audio_pick() {
    let mut lane = Lane::start(FlightRig::start);
    lane.rig.hold_answers();
    lane.burn_pick();
    let action = claim_route_action().expect("claim");
    assert!(matches!(
        execute_retranscode_claim(&mut lane.ps, &action, OFFSET_SECS, -1, 0),
        RetranscodeClaimDispatch::Pending
    ));
    lane.wait_for_puts(1);
    assert!(commit_audio_pick(&mut lane.ps, eac3(36_016, 2), Some(japanese_eac3())));
    commit_subtitle_pick(&mut lane.ps, -1, 0, false, Some(SubtitleMemo::Off));
    lane.rig.release_answers();
    let (action, tail) = lane.await_landing();
    lane.settle(&action, tail);
    if pending_route_start().is_some() {
        let _frame = plx_base::task::FrameScope::enter();
        settle_pending_native_start(&mut lane.ps, RouteStartResult::Started);
    }
    lane.wait_for_puts(2);
    assert_eq!(param(&lane.puts()[1], "audioStreamID"), None, "the in-place PUT names no audio: {:?}", lane.puts());
    assert_eq!(param(&lane.puts()[1], "subtitleStreamID"), Some("0"));
    assert_eq!(lane.selection().map(|s| s.0), Some(Some(HELD_AUDIO)), "the server still holds the earlier audio");

    // The audio pick's own claim is refused and puts the earlier audio back.
    lane.rig.refuse_decisions(1);
    assert!(!lane.fly(), "refused");
    plx_base::task::drain_workers_for_test();
    let puts = lane.puts();
    let last = puts.last().expect("a PUT");
    assert_eq!(param(last, "audioStreamID"), Some("36014"), "the refused audio goes back: {puts:?}");
    assert_eq!(param(last, "subtitleStreamID"), None, "{puts:?}");
    assert_eq!(lane.selection(), Some((Some(HELD_AUDIO), Some(0))));
    lane.finish();
}

/// A send that panics must not strand the serial queue: its waiter is released (a PUT that did not
/// happen), a waiter behind it is not left waiting for a thread that is gone, and the next
/// `put_selection` is served by a fresh worker.
#[test]
fn a_panicking_send_releases_its_waiters_and_the_next_put_gets_a_fresh_worker() {
    fn panics_on_audio_one(_: &plx_base::task::OffFrame, req: &QueuedSelection) -> bool {
        assert_ne!(req.aud, 1, "the send panicked");
        true
    }
    let lane = Lane::start(FlightRig::start);
    let sid = cur_sid(&lane.ps);
    let (first, first_answer) = std::sync::mpsc::sync_channel(1);
    let (second, second_answer) = std::sync::mpsc::sync_channel(1);
    assert!(enqueue_selection_via(panics_on_audio_one, sid, PART, 1, Some(HELD_SUB), false, Some(first)));
    assert!(enqueue_selection_via(panics_on_audio_one, sid, PART, 2, Some(HELD_SUB), false, Some(second)));
    let wait = Duration::from_secs(5);
    // The panicked request's sender dropped with the unwind: no answer, which a waiter reads as "not sent".
    assert!(matches!(first_answer.recv_timeout(wait), Err(std::sync::mpsc::RecvTimeoutError::Disconnected)));
    // The one behind it was either released or served by a fresh worker; never left waiting.
    assert!(!matches!(second_answer.recv_timeout(wait), Err(std::sync::mpsc::RecvTimeoutError::Timeout)));
    let deadline = std::time::Instant::now() + wait;
    while !selection_queue_idle() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(selection_queue_idle(), "the queue is not stuck behind a dead worker");
    let put = put_selection(&plx_base::task::OffFrame::for_test(), sid, PART, HELD_AUDIO, HELD_SUB);
    assert!(put.is_some(), "the next PUT gets a fresh worker and is answered");
    lane.finish();
}

/// **An Original recovery that is refused** writes back the candidate's PUT: with nothing owed the
/// refusal is a plain rejection and one restore (the candidate's audio, which the viewer never chose).
#[test]
fn a_refused_original_recovery_puts_the_previous_selection_back() {
    let mut lane = Lane::start(FlightRig::start);
    FlightRig::offer_original(&mut lane.ps, false);
    lane.rig.refuse_decisions(1);
    FlightRig::queue_manual_original(&lane.ps, false);
    let action = claim_route_action().expect("claim");
    assert!(matches!(
        execute_recover_original_claim(&mut lane.ps, &action, OFFSET_SECS, RecoveryCause::ManualOriginal, -1, 0),
        RetranscodeClaimDispatch::Pending
    ));
    let (action, tail) = lane.await_landing();
    assert!(is_rejected(&tail), "{tail:?}");
    lane.settle(&action, tail);
    plx_base::task::drain_workers_for_test();
    let puts = lane.puts();
    assert!(!puts.is_empty(), "the recovery wrote the candidate's selection: {puts:?}");
    assert_eq!(puts.len(), 2, "the candidate's PUT, then its undoing: {puts:?}");
    assert_eq!(param(&puts[0], "audioStreamID"), Some("42"), "{}", puts[0]);
    assert_eq!(param(&puts[1], "audioStreamID"), Some("36014"), "the earlier audio goes back: {}", puts[1]);
    assert_eq!(param(&puts[1], "subtitleStreamID"), None, "the subtitle was not changed: {}", puts[1]);
    assert_eq!(lane.selection(), Some((Some(HELD_AUDIO), Some(HELD_SUB))), "the value the server holds again");
    lane.finish();
}

/// **A displaced pick under a refused Original**: the Original is refused, and so is the rebuild the
/// pick is owed. Nothing the claim wrote stays, and the pick is not remembered.
#[test]
fn a_refused_original_with_a_displaced_pick_restores_and_remembers_nothing() {
    let mut lane = Lane::start(FlightRig::start);
    FlightRig::offer_original(&mut lane.ps, false);
    lane.burn_pick();
    lane.rig.refuse_decisions(2);
    FlightRig::queue_manual_original(&lane.ps, true);
    let action = claim_route_action().expect("claim");
    assert!(matches!(
        execute_recover_original_claim(&mut lane.ps, &action, OFFSET_SECS, RecoveryCause::ManualOriginal, -1, 0),
        RetranscodeClaimDispatch::Pending
    ));
    let (action, tail) = lane.await_landing();
    assert!(is_rejected(&tail), "{tail:?}");
    lane.settle(&action, tail);
    plx_base::task::drain_workers_for_test();
    assert_eq!(lane.remembered(), None, "the refused pick was never the show's choice");
    assert_eq!(lane.selection(), Some((Some(HELD_AUDIO), Some(HELD_SUB))), "the selection the server held before");
    lane.finish();
}

/// **An Original recovery that is ACCEPTED lands through `original_tail`**, which never reaches
/// `finish_route_action(.., Prepared)`: the pick a displaced rebuild carried is remembered there.
#[test]
fn an_accepted_displaced_pick_landing_as_an_original_trial_is_remembered() {
    let mut lane = Lane::start(FlightRig::start);
    FlightRig::offer_original(&mut lane.ps, false);
    lane.burn_pick();
    FlightRig::queue_manual_original(&lane.ps, true);
    assert_eq!(lane.remembered(), None);
    let action = claim_route_action().expect("claim");
    assert!(matches!(
        execute_recover_original_claim(&mut lane.ps, &action, OFFSET_SECS, RecoveryCause::ManualOriginal, -1, 0),
        RetranscodeClaimDispatch::Pending
    ));
    let (_, tail) = lane.await_landing();
    assert!(matches!(tail, ClaimTail::Original(_)), "{tail:?}");
    assert_eq!(
        lane.remembered(),
        Some(TrackMemoryEntry::new(MACHINE, SHOW, None, Some(english_track()))),
        "accepted by the server, so remembered",
    );
    lane.finish();
}
