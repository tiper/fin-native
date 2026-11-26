//! PMS plan/decision tests: plan round-trips, MDE transcode decisions and subtitle
//! selection.

use super::*;
#[allow(unused_imports)]
use super::test_support::*;
use super::test_support::apply_plan;

/// One bare GET to a mock PMS, answered and read to the end.
fn ping(port: i32) {
    use std::io::{Read, Write};
    let mut socket = std::net::TcpStream::connect(("127.0.0.1", port as u16)).expect("connect");
    socket.write_all(b"GET /library/metadata/1 HTTP/1.1\r\nHost: x\r\n\r\n").expect("send");
    let mut answer = Vec::new();
    let _ = socket.read_to_end(&mut answer);
}

/// **The mock PMS refuses to idle quietly.** A server told to expect more requests than the code
/// under test makes used to sit out its whole deadline while the test held the serial lock (six
/// tests spent 8 s each doing it). It now fails the test, naming the count it was promised and
/// the requests it saw; a scenario whose count varies opts out with `exact = false`.
#[test]
fn a_mock_pms_that_never_sees_its_count_fails_the_test() {
    let (port, rx, server) =
        plan_pms_within(3, EMPTY_MC, std::time::Duration::from_millis(300), true);
    ping(port);
    let seen = rx.recv_timeout(std::time::Duration::from_secs(5)).expect("the server publishes what it saw");
    assert_eq!(seen.len(), 1, "{seen:?}");
    let failure = server.join().expect_err("a short count must fail the test that joins the server");
    let message = failure.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(message.contains("expect 3 requests and saw 1"), "{message}");
}

#[test]
fn a_mock_pms_given_an_explicit_upper_bound_may_see_fewer() {
    let (port, rx, server) =
        plan_pms_within(3, EMPTY_MC, std::time::Duration::from_millis(300), false);
    ping(port);
    assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(5)).expect("published").len(), 1);
    server.join().expect("an opted-out count is not a failure");
}

#[test]
fn a_mock_pms_that_sees_exactly_its_count_stops_at_once_and_passes() {
    let (port, rx, server) = plan_pms_within(2, EMPTY_MC, std::time::Duration::from_secs(30), true);
    let started = std::time::Instant::now();
    ping(port);
    ping(port);
    assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(5)).expect("published").len(), 2);
    server.join().expect("the exact count passes");
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "it must not wait for its deadline");
}

/// The identity round trip: `request_play`'s captured id reaches `cur_sid` unchanged, through
/// `ResolveEnv` (the main-thread snapshot) and `Plan` (the worker's output).
///
/// Graded on the resolve that FAILS, deliberately — it is the exit `build_stream` takes first,
/// before any network, and a plan that carries no server is one `apply_plan` cannot install an
/// honest `cur_sid` from. Every richer exit builds on the same field.
#[test]
#[cfg(feature = "devtriggers")]
fn a_plan_round_trips_the_server_the_request_captured() {
    let mut ps = crate::route::PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    let sid = unregistered_sid();

    let env = ResolveEnv::snapshot(&ps, plx_data::stores::metadata::MetadataStore::default().view(), sid, "rk-7");
    assert_eq!(
        env.sid, sid,
        "the snapshot carries the id the request was made with"
    );

    let plan = build_stream(&plx_base::task::OffFrame::for_test(), "rk-7", "/library/parts/5/1/f.mkv", "h264", "ac3", &env);
    assert_eq!(
        plan.sid, sid,
        "a plan that could not resolve still names its server"
    );
    assert!(
        plan.url.is_empty(),
        "no client for that slot, so nothing resolved"
    );
    assert_eq!(
        plan.part_id, 5,
        "…and the rest of the plan is built as usual"
    );

    apply_plan(&mut ps, plan, "rk-7");
    assert_eq!(cur_sid(&ps), sid, "the installed identity is the captured one");
    assert_eq!(cur_rk(&ps), "rk-7", "and its other half");
}

/// **A plan that never reached the codec gate must not claim the source is undecodable.**
///
/// `Plan::source_decodable` is a `bool`, `bool::default()` is `false`, and `false` is a CLAIM —
/// the quality menu renders it as "Converts on server" on the Original row. `build_stream` has
/// an exit (no client for the server slot) that returns before the gate runs at all, so the
/// value has to be `true` on the way in and be overwritten by evidence, not the other way
/// round.
///
/// Differential: with the initializer's explicit `true` removed this fails, and the failure is
/// a line of copy asserting something about a file nobody opened.
#[test]
fn a_plan_that_never_resolved_makes_no_claim_about_the_source() {
    let mut ps = crate::route::PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    let sid = unregistered_sid();
    let env = ResolveEnv::snapshot(&ps, plx_data::stores::metadata::MetadataStore::default().view(), sid, "rk-7");

    let plan = build_stream(&plx_base::task::OffFrame::for_test(), "rk-7", "/library/parts/5/1/f.mkv", "h264", "ac3", &env);
    assert!(
        plan.url.is_empty(),
        "the test needs the exit that precedes the codec gate"
    );
    assert!(
        plan.source_decodable,
        "nobody looked at this file, so nothing may be said about it",
    );
    apply_plan(&mut ps, plan, "rk-7");
    assert!(
        source_decodable(&ps),
        "and the session carries the same silence"
    );
}

/// **The failure read-out's actions after the Engine is released.** Try again, Play automatically
/// and a quality pick all begin with a new playback request of the same item; Back ends the session.
/// The released Engine's standing error (its cause, with the playhead it left in the unpresented
/// resume slot) is what they find: a retry resumes from that position, and the new request ends the
/// standing error, so the read-out does not outlive the attempt it described.
#[test]
fn a_new_request_ends_the_standing_error_of_a_released_engine() {
    let mut ps = crate::route::PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    let mut meta = plx_data::stores::metadata::MetadataStore::default();
    note_engine_failure(&mut ps, crate::player::RuntimeFailure::PlaybackInterrupted, 60_000_000_000);
    assert_eq!(crate::player::state(&ps), crate::player::PlaybackState::Error, "the read-out stands with no Engine");
    assert_eq!(unpresented_resume_ns(&ps), 60_000_000_000, "and a retry resumes from the playhead");

    assert!(request_play(
        &mut ps, &mut meta, plx_os::devcaps::caps(), ServerId::from_raw(0), "film", "/library/parts/2/file.mkv", "h264", "aac", "Film", "", None, "",
    ));
    assert!(engine_failure(&ps).is_none(), "the new attempt ends the standing error");
    assert_ne!(crate::player::state(&ps), crate::player::PlaybackState::Error);

    cancel_play(&mut ps);
    let _ = wait_resolve_workers_for_test(std::time::Duration::from_secs(10));
    reset_player_control_for_test(&ps);
    reset_session(&mut ps);
}

/// Issue #266: `apply_plan` is the ONE place a resolve's audio facts enter the session. The
/// contract's `audio` (what the URL asked for), the outcome (what the decision did with it) and
/// the carried track (what the pipeline feeds) are installed exactly as the plan built them —
/// no fabricated `CarriedAudio` from the flat codec fields, and a plan without a track installs
/// `None` rather than a confident default.
#[test]
fn apply_plan_installs_contract_outcome_audio() {
    let mut ps = crate::route::PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    let carried = CarriedAudio {
        sid: 31,
        ordinal: 2,
        codec: "truehd".into(),
        channels: 8,
        can_normalize_loudness: true,
        immersive: false,
        sample_rate: 0,
        bit_depth: 0,
    };
    let audio = plx_plex::plex::AudioEnhancements { boost_dialog: true, normalize_loudness: false };
    apply_plan(
        &mut ps,
        Plan {
            url: "https://example.invalid/start.mkv".into(),
            acodec: "ac3".into(),
            src_acodec: "truehd".into(),
            contract: plx_plex::plex::EncodeContract { audio, ..Default::default() },
            enhancement: EnhancementOutcome::Applied,
            audio: Some(carried.clone()),
            ..Default::default()
        },
        "rk-enh",
    );
    assert_eq!(ps.cur_contract.audio, audio, "the contract carries what the URL asked for");
    assert_eq!(ps.cur_enhancement, EnhancementOutcome::Applied);
    assert_eq!(ps.cur_audio, Some(carried), "the plan's own track, not one rebuilt from acodec");

    // A plan that carried no track (server default, list not fetched) installs no track.
    apply_plan(
        &mut ps,
        Plan {
            url: "https://example.invalid/f.mkv".into(),
            acodec: "eac3".into(),
            src_acodec: "eac3".into(),
            ..Default::default()
        },
        "rk-plain",
    );
    assert_eq!(ps.cur_contract.audio, plx_plex::plex::AudioEnhancements::NONE);
    assert_eq!(ps.cur_enhancement, EnhancementOutcome::Off);
    assert_eq!(ps.cur_audio, None, "no fabricated track from the flat codec fields");
}

#[test]
#[cfg(feature = "devtriggers")]
fn remux_review_probe_installs_effective_selection_before_decision_and_start() {
    let mut ps = PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    assert!(plx_net::net::global_init() && crate::curlio::available());
    restore_quality(Quality::Auto);
    // Cold client-rendered subtitles stay off server-side; an explicit burn remains a burn.
    for burn in [0, 9] {
        let (port, done, server) = selection_probe_pms(false, burn);
        let sid = plx_plex::plex::register_for_test("selection-probe", "127.0.0.1", port, "token", "selection-probe-client");
        plx_plex::plex::client_for(sid).unwrap().set_link(plx_plex::plex::probe::Location::Remote);
        let mut env = ResolveEnv::snapshot(&ps, plx_data::stores::metadata::MetadataStore::default().view(), sid, "rk-4k");
        env.audio_sid = 1;
        env.sub_sid = burn;
        let mut item = fourk_item_with_subs(sid, vec![
            plx_data::metadata::Stream { id: 1, index: 0, lang_code: "eng".into(), codec: "truehd".into(), channels: 8, default: true, selected: true, ..Default::default() },
            plx_data::metadata::Stream { id: 2, index: 1, lang_code: "eng".into(), codec: "ac3".into(), channels: 6, ..Default::default() },
        ], vec![selected_sub(9, "srt")]);
        item.bitrate = 320;
        env.cached_item = Some(item);
        let plan = build_stream(&plx_base::task::OffFrame::for_test(), "rk-4k", "/library/parts/36013/1/file.mkv", "hevc", "truehd", &env);
        done.send(()).unwrap();
        let requests = server.join().unwrap();
        let decision = requests.iter().position(|(r, _)| r.contains("/decision?") && !r.contains("hasMDE=1")).expect("probe decision");
        let put = requests.iter().position(|(r, _)| r.starts_with("PUT /library/parts/")).expect("selection PUT");
        assert!(put < decision, "PUT must precede probe decision: {requests:?}");
        let start = requests.iter().position(|(r, _)| r.contains("start.mkv")).expect("sample GET");
        assert!(decision < start);
        for index in [decision, start] {
            let (request, state) = &requests[index];
            assert_eq!(*state, (2, burn), "effective PMS selection");
            assert_eq!(query_param(request, "audioStreamID"), Some("2"));
            // Progressive requests omit zero; the prior PUT is what suppresses defaults.
            assert_eq!(query_param(request, "subtitleStreamID"), if burn == 0 { None } else { Some("9") });
            assert_eq!(query_param(request, "subtitles"), if burn == 0 { None } else { Some("burn") });
        }
        assert!(plan.contract.remux && plan.url.contains("start.mkv"), "state-dependent sample must admit remux: {}", plan.url);
    }
    restore_quality(Quality::Original);
    plx_plex::plex::reset_servers_for_test();
}

#[test]
#[cfg(feature = "devtriggers")]
fn remux_review_http_200_refusal_never_gets_media() {
    let mut ps = PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    assert!(plx_net::net::global_init() && crate::curlio::available());
    let (port, done, server) = selection_probe_pms(true, 0);
    let sid = plx_plex::plex::register_for_test("refused-probe", "127.0.0.1", port, "token", "refused-probe-client");
    let sample = measure_remote_remux(&plx_base::task::OffFrame::for_test(), plx_plex::plex::client_for(sid).unwrap(), &plx_os::devcaps::caps(), "rk", 0, "refused-session", 2, 0, false, 320, plx_plex::plex::AudioEnhancements::NONE).sample;
    done.send(()).unwrap();
    let requests = server.join().unwrap();
    assert!(sample.is_none());
    assert!(requests.iter().any(|(r, _)| r.contains("/decision?")));
    assert!(!requests.iter().any(|(r, _)| r.contains("start.mkv")), "refusal must prevent media GET: {requests:?}");
    assert!(!requests.iter().any(|(r, _)| r.contains("closeResourceSession=1")));
    plx_plex::plex::reset_servers_for_test();
}

/// PMS 1.43 503s a Part GET whose session has no MDE decision. A 4K HEVC+EAC3 title used
/// to skip `/decision` because EAC3 is a direct-play audio codec; the plan URL must not
/// be that part until MDE has registered the session.
#[test]
#[cfg(feature = "devtriggers")]
fn original_hevc_eac3_registers_mde_before_returning_the_part() {
    use std::time::Duration;
    let mut ps = crate::route::PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    let (port, rx, server) = plan_pms(2, MDE_DIRECTPLAY);
    let sid = plx_plex::plex::register_for_test(
        "mde-dp-test",
        "127.0.0.1",
        port,
        "token",
        "mde-dp-client",
    );
    let mut env = ResolveEnv::snapshot(&ps, plx_data::stores::metadata::MetadataStore::default().view(), sid, "rk-4k");
    env.cached_item = Some(fourk_item(sid, vec![eac3_track()]));
    let plan = build_stream(
        &plx_base::task::OffFrame::for_test(),
        "rk-4k",
        "/library/parts/36013/1/file.mkv",
        "hevc",
        "eac3",
        &env,
    );
    let requests = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("PMS never saw the resolve");
    server.join().unwrap();

    let decision = requests
        .iter()
        .find(|line| line.contains("/decision?"))
        .unwrap_or_else(|| panic!("MDE was never asked: {requests:?}"));
    assert!(decision.contains("hasMDE=1"), "{decision}");
    assert!(decision.contains("directPlay=1"), "{decision}");
    assert!(decision.contains("audioStreamID=36014"), "{decision}");
    assert_eq!(
        query_param(decision, "subtitleStreamID"),
        Some("0"),
        "no selectable embedded sub → explicit 0: {decision}"
    );
    assert_eq!(
        query_param(decision, "subtitles"),
        Some("none"),
        "MDE must name client-rendered mode; auto 400s a selected sub: {decision}"
    );
    assert!(
        plan.url.contains("/library/parts/36013/"),
        "admitted Original is the raw part: {}",
        plan.url
    );
    assert!(
        !plan.url.contains("start.mkv"),
        "MDE said directplay, so this is not a transcode: {}",
        plan.url
    );
    plx_plex::plex::reset_servers_for_test();
}

/// Smart-DP used to skip MDE so a TrueHD default would not veto the AC3 sibling. The
/// `/decision` query must name that sibling; otherwise PMS evaluates TrueHD and the part
/// GET 503s or the title is sent to a video-downscaling transcode.
#[test]
#[cfg(feature = "devtriggers")]
fn smart_dp_names_the_ac3_sibling_on_mde() {
    use std::time::Duration;
    let mut ps = crate::route::PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    let (port, rx, server) = plan_pms(2, MDE_DIRECTPLAY);
    let sid = plx_plex::plex::register_for_test(
        "mde-smart-dp",
        "127.0.0.1",
        port,
        "token",
        "mde-smart-client",
    );
    let mut env = ResolveEnv::snapshot(&ps, plx_data::stores::metadata::MetadataStore::default().view(), sid, "rk-4k");
    env.cached_item = Some(fourk_item(
        sid,
        vec![
            plx_data::metadata::Stream {
                id: 1,
                index: 0,
                lang_code: "eng".into(),
                codec: "truehd".into(),
                channels: 8,
                default: true,
                selected: true,
                ..Default::default()
            },
            plx_data::metadata::Stream {
                id: 2,
                index: 1,
                lang_code: "eng".into(),
                codec: "ac3".into(),
                channels: 6,
                ..Default::default()
            },
        ],
    ));
    let plan = build_stream(
        &plx_base::task::OffFrame::for_test(),
        "rk-4k",
        "/library/parts/36013/1/file.mkv",
        "hevc",
        "truehd",
        &env,
    );
    let requests = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("PMS never saw the resolve");
    server.join().unwrap();

    let decision = requests
        .iter()
        .find(|line| line.contains("/decision?"))
        .unwrap_or_else(|| panic!("MDE was never asked: {requests:?}"));
    assert!(
        decision.contains("audioStreamID=2"),
        "MDE must see the AC3 sibling, not TrueHD: {decision}"
    );
    assert_eq!(
        query_param(decision, "subtitleStreamID"),
        Some("0"),
        "{decision}"
    );
    assert!(plan.url.contains("/library/parts/36013/"), "{}", plan.url);
    plx_plex::plex::reset_servers_for_test();
}

/// OpenAPI: a Part GET whose decision is a transcode is HTTP 503. Honour MDE rather than
/// returning the part URL the local codec test would have chosen. A `/decision` body that
/// names no video stream cannot claim the video must re-encode, so this unnamed shape remuxes.
#[test]
#[cfg(feature = "devtriggers")]
fn mde_transcode_does_not_return_the_part_url() {
    use std::time::Duration;
    let mut ps = crate::route::PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    let (port, rx, server) = plan_pms(4, MDE_TRANSCODE);
    let sid = plx_plex::plex::register_for_test(
        "mde-tc-test",
        "127.0.0.1",
        port,
        "token",
        "mde-tc-client",
    );
    let mut env = ResolveEnv::snapshot(&ps, plx_data::stores::metadata::MetadataStore::default().view(), sid, "rk-4k");
    env.cached_item = Some(fourk_item(sid, vec![eac3_track()]));
    let plan = build_stream(
        &plx_base::task::OffFrame::for_test(),
        "rk-4k",
        "/library/parts/36013/1/file.mkv",
        "hevc",
        "eac3",
        &env,
    );
    let requests = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("PMS never saw the resolve");
    server.join().unwrap();

    assert!(
        requests
            .iter()
            .any(|line| line.contains("hasMDE=1") && line.contains("directPlay=1")),
        "the first ask is still the MDE handshake: {requests:?}"
    );
    assert!(
        plan.url.contains("start.mkv"),
        "MDE said transcode, so the plan must not GET the part: {}",
        plan.url
    );
    assert!(
        !plan.url.contains("/library/parts/36013/"),
        "a transcode decision must not be served as Original: {}",
        plan.url
    );
    assert!(
        plan.contract.remux,
        "Part.decision=transcode with no Stream[] cannot claim a video re-encode: {}",
        plan.url
    );
    plx_plex::plex::reset_servers_for_test();
}

/// HEVC+TrueHD with no AAC/AC3/EAC3 sibling: MDE transcodes the part (TrueHD is not in the
/// profile) but the video can still be copied. A Part-level veto would re-encode 4K for an
/// audio problem.
#[test]
#[cfg(feature = "devtriggers")]
fn mde_transcode_for_truehd_only_still_remuxes() {
    use std::time::Duration;
    let mut ps = crate::route::PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    let (port, rx, server) = plan_pms(4, MDE_TRANSCODE_COPY);
    let sid = plx_plex::plex::register_for_test(
        "mde-truehd",
        "127.0.0.1",
        port,
        "token",
        "mde-truehd-client",
    );
    let mut env = ResolveEnv::snapshot(&ps, plx_data::stores::metadata::MetadataStore::default().view(), sid, "rk-4k");
    env.cached_item = Some(fourk_item(
        sid,
        vec![plx_data::metadata::Stream {
            id: 36014,
            index: 1,
            lang_code: "eng".into(),
            codec: "truehd".into(),
            channels: 8,
            default: true,
            selected: true,
            ..Default::default()
        }],
    ));
    let plan = build_stream(
        &plx_base::task::OffFrame::for_test(),
        "rk-4k",
        "/library/parts/36013/1/file.mkv",
        "hevc",
        "truehd",
        &env,
    );
    let requests = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("PMS never saw the resolve");
    server.join().unwrap();

    assert!(
        requests
            .iter()
            .any(|line| line.contains("/decision?") && line.contains("hasMDE=1")),
        "MDE was still asked: {requests:?}"
    );
    assert!(
        !plan.url.contains("/library/parts/36013/"),
        "TrueHD-only cannot Original: {}",
        plan.url
    );
    assert!(
        plan.contract.remux,
        "TrueHD-only must codec-copy remux, not re-encode 4K: {}",
        plan.url
    );
    plx_plex::plex::reset_servers_for_test();
}

/// A video-stream `transcode` (bit depth past the profile, …) is the copy veto. Remux here
/// would ship pixels the decoder cannot take.
#[test]
#[cfg(feature = "devtriggers")]
fn mde_video_stream_transcode_forbids_remux() {
    use std::time::Duration;
    let mut ps = crate::route::PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    let (port, rx, server) = plan_pms(4, MDE_TRANSCODE_VIDEO);
    let sid = plx_plex::plex::register_for_test(
        "mde-vid-tc",
        "127.0.0.1",
        port,
        "token",
        "mde-vid-tc-client",
    );
    let mut env = ResolveEnv::snapshot(&ps, plx_data::stores::metadata::MetadataStore::default().view(), sid, "rk-4k");
    env.cached_item = Some(fourk_item(sid, vec![eac3_track()]));
    let plan = build_stream(
        &plx_base::task::OffFrame::for_test(),
        "rk-4k",
        "/library/parts/36013/1/file.mkv",
        "hevc",
        "eac3",
        &env,
    );
    let requests = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("PMS never saw the resolve");
    server.join().unwrap();

    assert!(
        requests
            .iter()
            .any(|line| line.contains("/decision?") && line.contains("hasMDE=1")),
        "MDE was still asked: {requests:?}"
    );
    assert!(
        !plan.url.contains("/library/parts/36013/"),
        "video transcode must not Original: {}",
        plan.url
    );
    assert!(
        !plan.contract.remux,
        "video-stream transcode forbids a codec-copy remux"
    );
    plx_plex::plex::reset_servers_for_test();
}

/// A declared Profile 5 can direct-play, but a remux copy carries no `DolbyHdrInfo`. MDE's
/// video=`copy` (the measured P5 shape) must not override `no_video_copy`.
#[test]
#[cfg(feature = "devtriggers")]
fn mde_transcode_copy_still_refuses_a_profile_5_remux() {
    use std::time::Duration;
    let mut ps = crate::route::PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    let (port, rx, server) = plan_pms(4, MDE_TRANSCODE_COPY);
    let sid = plx_plex::plex::register_for_test(
        "mde-p5-copy",
        "127.0.0.1",
        port,
        "token",
        "mde-p5-copy-client",
    );
    let mut env = ResolveEnv::snapshot(&ps, plx_data::stores::metadata::MetadataStore::default().view(), sid, "rk-4k");
    env.dv_capability = plx_os::devcaps::dv::DvCapability::Supported;
    let mut item = fourk_item(sid, vec![eac3_track()]);
    item.dovi = p5();
    env.cached_item = Some(item);
    let plan = build_stream(
        &plx_base::task::OffFrame::for_test(),
        "rk-4k",
        "/library/parts/36013/1/file.mkv",
        "hevc",
        "eac3",
        &env,
    );
    let requests = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("PMS never saw the resolve");
    server.join().unwrap();

    assert!(
        requests
            .iter()
            .any(|line| line.contains("/decision?") && line.contains("hasMDE=1")),
        "a declared P5 still asks MDE before accepting Original: {requests:?}"
    );
    assert!(
        !plan.contract.remux,
        "a P5 remux is the IPT-PQ bitstream with no declaration: {}",
        plan.url
    );
    assert!(
        plan.contract.no_video_copy,
        "the copy permission has to be withdrawn or PMS copies anyway"
    );
    plx_plex::plex::reset_servers_for_test();
}

#[test]
#[cfg(feature = "devtriggers")]
fn unconfirmed_profile_5_forbids_copy_before_mde() {
    use std::time::Duration;

    use plx_plex::plex::session::DolbyVisionMode;
    // The third row is the Settings > Playback > Dolby Vision Off: a set that says Supported plans
    // as Unsupported, because the mode rides `ResolveEnv` and is applied at the route.
    for (capability, mode) in [
        (plx_os::devcaps::dv::DvCapability::Unknown, DolbyVisionMode::Auto),
        (plx_os::devcaps::dv::DvCapability::Unsupported, DolbyVisionMode::Auto),
        (plx_os::devcaps::dv::DvCapability::Supported, DolbyVisionMode::Off),
    ] {
        let mut ps = crate::route::PlaybackSession::IDLE;
        let _g = fresh_registry(&mut ps);
        // Three requests, not four: the PlayQueue POST, the part-selection PUT and the
        // `/decision` GET. The mock waits for `n` of them or an 8 s deadline, so a count that is
        // one too high made this test sit out the whole deadline once per capability (16 s) while
        // holding the serial lock every other `plx_media` test queues on.
        let (port, rx, server) = plan_pms(3, MDE_TRANSCODE_COPY);
        let sid = plx_plex::plex::register_for_test(
            "mde-p5-no-copy",
            "127.0.0.1",
            port,
            "token",
            "mde-p5-no-copy-client",
        );
        let mut env = ResolveEnv::snapshot(
            &ps,
            plx_data::stores::metadata::MetadataStore::default().view(),
            sid,
            "rk-4k",
        );
        env.dv_capability = capability;
        env.dolby_vision_mode = mode;
        let mut item = fourk_item(sid, vec![eac3_track()]);
        item.dovi = p5();
        env.cached_item = Some(item);
        let plan = build_stream(
            &plx_base::task::OffFrame::for_test(),
            "rk-4k",
            "/library/parts/36013/1/file.mkv",
            "hevc",
            "eac3",
            &env,
        );
        let requests = rx
            .recv_timeout(Duration::from_secs(15))
            .expect("PMS never saw the resolve");
        server.join().unwrap();

        let decision = requests
            .iter()
            .find(|line| line.contains("/decision?"))
            .unwrap_or_else(|| panic!("decision missing: {requests:?}"));
        assert!(decision.contains("directStream=0"), "{capability:?}: {decision}");
        assert!(!plan.contract.remux, "{capability:?}");
        assert_eq!(plan.dovi, plx_data::metadata::Dovi::NONE);
        assert_eq!(
            plan.dv_decision.presentation,
            plx_data::metadata::DvPresentation::NotDv,
        );
        assert!(plan.dv_decision.presentation.declared().is_none());
        // The source decision the resolve froze: the set's own answer, whatever the setting, and
        // the unsupported column's verdict on a Profile 5 (refuse direct play).
        assert_eq!(plan.src_dv_decision.capability, capability, "{mode:?}");
        assert!(plan.src_dv_decision.presentation.refusal().is_some(), "{capability:?} {mode:?}");
        plx_plex::plex::reset_servers_for_test();
    }
}

/// Remote Auto used to probe the Part after MDE registered transcode, which 503s, so bootstrap
/// fell through to HLS and re-encoded 4K for an audio-only veto. The probe has to sample the
/// remux `start.mkv` we would actually play.
#[test]
#[cfg(feature = "devtriggers")]
fn remote_auto_truehd_remux_probes_start_mkv_not_the_part() {
    use std::time::Duration;
    let mut ps = crate::route::PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    if !plx_net::net::global_init() || !crate::curlio::available() {
        return;
    }
    restore_quality(Quality::Auto);
    let probe_bytes = crate::abr::source_probe_plan(320, crate::abr::PROBE_BUDGET_MS)
        .expect("tiny source still has a probe object")
        .target_bytes;
    let (port, rx, server) = plan_pms_with_start_mkv(6, MDE_TRANSCODE_COPY, probe_bytes);
    let sid = plx_plex::plex::register_for_test(
        "mde-remote-truehd",
        "127.0.0.1",
        port,
        "token",
        "mde-remote-truehd-client",
    );
    plx_plex::plex::client_for(sid)
        .expect("registered")
        .set_link(plx_plex::plex::probe::Location::Remote);
    let mut env = ResolveEnv::snapshot(&ps, plx_data::stores::metadata::MetadataStore::default().view(), sid, "rk-4k");
    let mut item = fourk_item(
        sid,
        vec![plx_data::metadata::Stream {
            id: 36014,
            index: 1,
            lang_code: "eng".into(),
            codec: "truehd".into(),
            channels: 8,
            default: true,
            selected: true,
            ..Default::default()
        }],
    );
    item.bitrate = 320;
    env.cached_item = Some(item);
    let plan = build_stream(
        &plx_base::task::OffFrame::for_test(),
        "rk-4k",
        "/library/parts/36013/1/file.mkv",
        "hevc",
        "truehd",
        &env,
    );
    let requests = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("PMS never saw the resolve");
    server.join().unwrap();

    assert!(
        requests
            .iter()
            .any(|line| line.contains("/decision?") && line.contains("hasMDE=1")),
        "MDE was still asked: {requests:?}"
    );
    assert!(
        !requests
            .iter()
            .any(|line| line.contains("GET /library/parts/")),
        "a Part GET after transcode MDE is 503: {requests:?}"
    );
    assert!(
        requests.iter().any(|line| line.contains("start.mkv")),
        "Remote Auto must probe the remux: {requests:?}"
    );
    assert!(
        plan.contract.remux,
        "TrueHD-only Remote Auto must codec-copy remux, not HLS-encode 4K: {}",
        plan.url
    );
    assert!(
        plan.url.contains("start.mkv"),
        "the installed route is the remux: {}",
        plan.url
    );
    assert!(
        !requests.iter().any(|line| line.contains("/transcode/universal/stop")),
        "a successful remux Original leaves the session for the play-path decision: {requests:?}"
    );
    restore_quality(Quality::Original);
    plx_plex::plex::reset_servers_for_test();
}

/// TrueHD default `id=1` is what `env.audio_sid` still carries at resolve start.
/// MDE, the remux probe, the play-path PUT, and the installed start.mkv must all name the
/// AC3 sibling `id=2` that smart-DP will actually feed.
#[test]
#[cfg(feature = "devtriggers")]
fn remote_auto_truehd_remux_probe_names_the_ac3_sibling_not_env_audio_sid() {
    use std::time::Duration;
    let mut ps = crate::route::PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    if !plx_net::net::global_init() || !crate::curlio::available() {
        return;
    }
    restore_quality(Quality::Auto);
    let probe_bytes = crate::abr::source_probe_plan(320, crate::abr::PROBE_BUDGET_MS)
        .expect("tiny source still has a probe object")
        .target_bytes;
    let (port, rx, server) = plan_pms_with_start_mkv(6, MDE_TRANSCODE_COPY, probe_bytes);
    let sid = plx_plex::plex::register_for_test(
        "mde-remote-truehd-ids",
        "127.0.0.1",
        port,
        "token",
        "mde-remote-truehd-ids-client",
    );
    plx_plex::plex::client_for(sid)
        .expect("registered")
        .set_link(plx_plex::plex::probe::Location::Remote);
    let mut env = ResolveEnv::snapshot(&ps, plx_data::stores::metadata::MetadataStore::default().view(), sid, "rk-4k");
    env.audio_sid = 1;
    let mut item = fourk_item(
        sid,
        vec![
            plx_data::metadata::Stream {
                id: 1,
                index: 0,
                lang_code: "eng".into(),
                codec: "truehd".into(),
                channels: 8,
                default: true,
                selected: true,
                ..Default::default()
            },
            plx_data::metadata::Stream {
                id: 2,
                index: 1,
                lang_code: "eng".into(),
                codec: "ac3".into(),
                channels: 6,
                ..Default::default()
            },
        ],
    );
    item.bitrate = 320;
    env.cached_item = Some(item);
    let plan = build_stream(
        &plx_base::task::OffFrame::for_test(),
        "rk-4k",
        "/library/parts/36013/1/file.mkv",
        "hevc",
        "truehd",
        &env,
    );
    let requests = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("PMS never saw the resolve");
    server.join().unwrap();

    let mde = requests
        .iter()
        .find(|line| line.contains("/decision?") && line.contains("hasMDE=1"))
        .unwrap_or_else(|| panic!("MDE was never asked: {requests:?}"));
    assert_eq!(
        query_param(mde, "audioStreamID"),
        Some("2"),
        "MDE must see the AC3 sibling, not env.audio_sid=1: {mde}"
    );
    let remux_probe = requests
        .iter()
        .find(|line| {
            line.contains("start.mkv")
                || (line.contains("/decision?")
                    && line.contains("directPlay=0")
                    && !line.contains("hasMDE=1"))
        })
        .unwrap_or_else(|| panic!("remux probe was never asked: {requests:?}"));
    assert_eq!(
        query_param(remux_probe, "audioStreamID"),
        Some("2"),
        "remux probe must name the AC3 sibling, not env.audio_sid=1: {remux_probe}"
    );
    let put = requests
        .iter()
        .find(|line| line.contains("PUT /library/parts/"))
        .unwrap_or_else(|| panic!("play-path PUT was never asked: {requests:?}"));
    assert_eq!(
        query_param(put, "audioStreamID"),
        Some("2"),
        "PUT must select the AC3 sibling, not env.audio_sid=1: {put}"
    );
    assert!(
        plan.url.contains("start.mkv"),
        "Remote Auto remux installs start.mkv: {}",
        plan.url
    );
    assert_eq!(
        query_param(&plan.url, "audioStreamID"),
        Some("2"),
        "playback URL must name the AC3 sibling, not env.audio_sid=1: {}",
        plan.url
    );
    restore_quality(Quality::Original);
    plx_plex::plex::reset_servers_for_test();
}

/// 720p denies remux, so the encoder can transcode the selected DTS. Putting the smart-DP
/// AC3 sibling here replaced English with the Russian default (reproduced on PMS 1.43.4).
#[test]
#[cfg(feature = "devtriggers")]
fn a_720p_reencode_puts_the_selected_dts_not_the_ac3_sibling() {
    use std::time::Duration;
    let mut ps = crate::route::PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    restore_quality(Quality::P720);
    let (port, rx, server) = plan_pms(3, EMPTY_MC);
    let sid = plx_plex::plex::register_for_test(
        "reencode-selected-dts",
        "127.0.0.1",
        port,
        "token",
        "reencode-selected-dts-client",
    );
    let mut env = ResolveEnv::snapshot(&ps, plx_data::stores::metadata::MetadataStore::default().view(), sid, "rk-4k");
    env.quality = Quality::P720;
    env.src_kbps = 48_000;
    env.audio_sid = 0;
    env.cached_item = Some(fourk_item(
        sid,
        vec![
            plx_data::metadata::Stream {
                id: 2663,
                index: 0,
                lang_code: "rus".into(),
                codec: "ac3".into(),
                channels: 6,
                default: true,
                ..Default::default()
            },
            plx_data::metadata::Stream {
                id: 2669,
                index: 1,
                lang_code: "eng".into(),
                codec: "dca".into(),
                channels: 6,
                selected: true,
                ..Default::default()
            },
        ],
    ));
    let plan = build_stream(
        &plx_base::task::OffFrame::for_test(),
        "rk-4k",
        "/library/parts/36013/1/file.mkv",
        "hevc",
        "dca",
        &env,
    );
    let requests = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("PMS never saw the resolve");
    server.join().unwrap();

    assert!(
        !plan.contract.remux,
        "720p must re-encode, not copy: {}",
        plan.url
    );
    assert!(
        plan.url.contains("start.mkv"),
        "re-encode installs start.mkv: {}",
        plan.url
    );
    let put = requests
        .iter()
        .find(|line| line.contains("PUT /library/parts/"))
        .unwrap_or_else(|| panic!("play-path PUT was never asked: {requests:?}"));
    assert_eq!(
        query_param(put, "audioStreamID"),
        Some("2669"),
        "PUT must keep selected English DTS, not the Russian AC3 sibling: {put}"
    );
    assert_eq!(
        query_param(&plan.url, "audioStreamID"),
        Some("2669"),
        "start.mkv must name that DTS, not the AC3 sibling: {}",
        plan.url
    );
    assert_eq!(plan.audio.as_ref().map_or(0, |a| a.sid), 2669);
    restore_quality(Quality::Original);
    plx_plex::plex::reset_servers_for_test();
}

/// #202 end to end on the re-encode path: a French file whose default (echoed back as
/// `selected`) is French, beside an English track, with no Plex language preference. The 720p
/// re-encode must PUT and name the French default — no built-in English outranks the file.
#[test]
#[cfg(feature = "devtriggers")]
fn a_720p_reencode_keeps_the_files_default_language_over_english() {
    use std::time::Duration;
    let mut ps = crate::route::PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    restore_quality(Quality::P720);
    let (port, rx, server) = plan_pms(3, EMPTY_MC);
    let sid = plx_plex::plex::register_for_test(
        "reencode-default-echo",
        "127.0.0.1",
        port,
        "token",
        "reencode-default-echo-client",
    );
    let mut env = ResolveEnv::snapshot(&ps, plx_data::stores::metadata::MetadataStore::default().view(), sid, "rk-4k");
    env.quality = Quality::P720;
    env.src_kbps = 48_000;
    env.audio_sid = 0;
    env.cached_item = Some(fourk_item(
        sid,
        vec![
            plx_data::metadata::Stream {
                id: 10975,
                index: 0,
                lang_code: "fre".into(),
                codec: "eac3".into(),
                channels: 6,
                default: true,
                selected: true,
                ..Default::default()
            },
            plx_data::metadata::Stream {
                id: 10976,
                index: 1,
                lang_code: "eng".into(),
                codec: "eac3".into(),
                channels: 6,
                ..Default::default()
            },
        ],
    ));
    let plan = build_stream(
        &plx_base::task::OffFrame::for_test(),
        "rk-4k",
        "/library/parts/36013/1/file.mkv",
        "hevc",
        "eac3",
        &env,
    );
    let requests = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("PMS never saw the resolve");
    server.join().unwrap();

    assert!(
        !plan.contract.remux,
        "720p must re-encode, not copy: {}",
        plan.url
    );
    assert!(
        plan.url.contains("start.mkv"),
        "re-encode installs start.mkv: {}",
        plan.url
    );
    let put = requests
        .iter()
        .find(|line| line.contains("PUT /library/parts/"))
        .unwrap_or_else(|| panic!("play-path PUT was never asked: {requests:?}"));
    assert_eq!(
        query_param(put, "audioStreamID"),
        Some("10975"),
        "PUT must keep the French default, not English: {put}"
    );
    assert_eq!(
        query_param(&plan.url, "audioStreamID"),
        Some("10975"),
        "start.mkv must name the French default, not English: {}",
        plan.url
    );
    assert_eq!(plan.audio.as_ref().map_or(0, |a| a.sid), 10975);
    restore_quality(Quality::Original);
    plx_plex::plex::reset_servers_for_test();
}

/// 720p with no language preference keeps the file's direct-playable default (Russian AC3)
/// rather than switching to an unselected English DTS: the transcode speaks the language direct
/// play would have. Only a real pick or a Plex preference moves it (#202).
#[test]
#[cfg(feature = "devtriggers")]
fn a_720p_reencode_keeps_the_default_ac3_over_an_unselected_english_dts() {
    use std::time::Duration;
    let mut ps = crate::route::PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    restore_quality(Quality::P720);
    let (port, rx, server) = plan_pms(3, EMPTY_MC);
    let sid = plx_plex::plex::register_for_test(
        "reencode-pref-lang-dts",
        "127.0.0.1",
        port,
        "token",
        "reencode-pref-lang-dts-client",
    );
    let mut env = ResolveEnv::snapshot(&ps, plx_data::stores::metadata::MetadataStore::default().view(), sid, "rk-4k");
    env.quality = Quality::P720;
    env.src_kbps = 48_000;
    env.audio_sid = 0;
    env.cached_item = Some(fourk_item(
        sid,
        vec![
            plx_data::metadata::Stream {
                id: 2663,
                index: 0,
                lang_code: "rus".into(),
                codec: "ac3".into(),
                channels: 6,
                default: true,
                selected: true,
                ..Default::default()
            },
            plx_data::metadata::Stream {
                id: 2669,
                index: 1,
                lang_code: "eng".into(),
                codec: "dca".into(),
                channels: 6,
                ..Default::default()
            },
        ],
    ));
    let plan = build_stream(
        &plx_base::task::OffFrame::for_test(),
        "rk-4k",
        "/library/parts/36013/1/file.mkv",
        "hevc",
        "dca",
        &env,
    );
    let requests = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("PMS never saw the resolve");
    server.join().unwrap();

    assert!(
        !plan.contract.remux,
        "720p must re-encode, not copy: {}",
        plan.url
    );
    assert!(
        plan.url.contains("start.mkv"),
        "re-encode installs start.mkv: {}",
        plan.url
    );
    let put = requests
        .iter()
        .find(|line| line.contains("PUT /library/parts/"))
        .unwrap_or_else(|| panic!("play-path PUT was never asked: {requests:?}"));
    assert_eq!(
        query_param(put, "audioStreamID"),
        Some("2663"),
        "PUT must name the Russian AC3 default, not the unselected English DTS: {put}"
    );
    assert_eq!(
        query_param(&plan.url, "audioStreamID"),
        Some("2663"),
        "start.mkv must name the default AC3, not the DTS: {}",
        plan.url
    );
    assert_eq!(plan.audio.as_ref().map_or(0, |a| a.sid), 2663);
    restore_quality(Quality::Original);
    plx_plex::plex::reset_servers_for_test();
}

/// Relay Auto cannot Original, so bootstrap installs HLS. The play-path PUT and start.m3u8
/// must still name selected English DTS, not the Russian AC3 sibling a remux would copy.
#[test]
#[cfg(feature = "devtriggers")]
fn a_auto_hls_reencode_puts_the_selected_dts_not_the_ac3_sibling() {
    use std::time::Duration;
    let mut ps = crate::route::PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    restore_quality(Quality::Auto);
    let (port, rx, server) = plan_pms(3, EMPTY_MC);
    let sid = plx_plex::plex::register_for_test(
        "reencode-auto-hls-dts",
        "127.0.0.1",
        port,
        "token",
        "reencode-auto-hls-dts-client",
    );
    plx_plex::plex::client_for(sid)
        .expect("registered")
        .set_link(plx_plex::plex::probe::Location::Relay);
    let mut env = ResolveEnv::snapshot(&ps, plx_data::stores::metadata::MetadataStore::default().view(), sid, "rk-4k");
    env.quality = Quality::Auto;
    env.src_kbps = 48_000;
    env.audio_sid = 0;
    env.cached_item = Some(fourk_item(
        sid,
        vec![
            plx_data::metadata::Stream {
                id: 2663,
                index: 0,
                lang_code: "rus".into(),
                codec: "ac3".into(),
                channels: 6,
                default: true,
                ..Default::default()
            },
            plx_data::metadata::Stream {
                id: 2669,
                index: 1,
                lang_code: "eng".into(),
                codec: "dca".into(),
                channels: 6,
                selected: true,
                ..Default::default()
            },
        ],
    ));
    let plan = build_stream(
        &plx_base::task::OffFrame::for_test(),
        "rk-4k",
        "/library/parts/36013/1/file.mkv",
        "hevc",
        "dca",
        &env,
    );
    let requests = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("PMS never saw the resolve");
    server.join().unwrap();

    assert!(
        !plan.contract.remux,
        "Relay Auto must HLS-encode, not copy: {}",
        plan.url
    );
    assert!(
        plan.url.contains("start.m3u8"),
        "Relay Auto installs start.m3u8: {}",
        plan.url
    );
    let put = requests
        .iter()
        .find(|line| line.contains("PUT /library/parts/"))
        .unwrap_or_else(|| panic!("play-path PUT was never asked: {requests:?}"));
    assert_eq!(
        query_param(put, "audioStreamID"),
        Some("2669"),
        "PUT must keep selected English DTS, not the Russian AC3 sibling: {put}"
    );
    assert_eq!(
        query_param(&plan.url, "audioStreamID"),
        Some("2669"),
        "start.m3u8 must name that DTS, not the AC3 sibling: {}",
        plan.url
    );
    assert_eq!(plan.audio.as_ref().map_or(0, |a| a.sid), 2669);
    restore_quality(Quality::Original);
    plx_plex::plex::reset_servers_for_test();
}

/// A remux probe that registers `/decision` and then gets no `start.mkv` body must
/// physical-stop (`closeResourceSession=0`) so the HLS `/decision` on the same identity
/// is not 503'd. Closing the Streaming Resource would.
#[test]
#[cfg(feature = "devtriggers")]
fn remote_auto_failed_remux_sample_physical_stops_before_hls() {
    use std::time::Duration;
    let mut ps = crate::route::PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    if !plx_net::net::global_init() || !crate::curlio::available() {
        return;
    }
    restore_quality(Quality::Auto);
    let (port, rx, server) = plan_pms_with_start_mkv(8, MDE_TRANSCODE_COPY, 0);
    let sid = plx_plex::plex::register_for_test(
        "mde-remote-truehd-stop",
        "127.0.0.1",
        port,
        "token",
        "mde-remote-truehd-stop-client",
    );
    plx_plex::plex::client_for(sid)
        .expect("registered")
        .set_link(plx_plex::plex::probe::Location::Remote);
    let mut env = ResolveEnv::snapshot(&ps, plx_data::stores::metadata::MetadataStore::default().view(), sid, "rk-4k");
    let mut item = fourk_item(
        sid,
        vec![plx_data::metadata::Stream {
            id: 36014,
            index: 1,
            lang_code: "eng".into(),
            codec: "truehd".into(),
            channels: 8,
            default: true,
            selected: true,
            ..Default::default()
        }],
    );
    item.bitrate = 320;
    env.cached_item = Some(item);
    let plan = build_stream(
        &plx_base::task::OffFrame::for_test(),
        "rk-4k",
        "/library/parts/36013/1/file.mkv",
        "hevc",
        "truehd",
        &env,
    );
    let requests = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("PMS never saw the resolve");
    server.join().unwrap();

    assert!(
        requests.iter().any(|line| line.contains("start.mkv")),
        "the remux probe still ran: {requests:?}"
    );
    let stop = requests
        .iter()
        .find(|line| line.contains("/transcode/universal/stop"))
        .unwrap_or_else(|| panic!("failed remux sample must /stop: {requests:?}"));
    assert_eq!(
        query_param(stop, "closeResourceSession"),
        Some("0"),
        "physical-stop keeps the Streaming Resource for the HLS that follows: {stop}"
    );
    assert!(
        !requests.iter().any(|line| {
            line.contains("/transcode/universal/stop")
                && query_param(line, "closeResourceSession") == Some("1")
        }),
        "closeResourceSession=1 would 503 the next start: {requests:?}"
    );
    assert!(
        !plan.contract.remux,
        "no remux sample → Auto falls through to HLS: {}",
        plan.url
    );
    restore_quality(Quality::Original);
    plx_plex::plex::reset_servers_for_test();
}

/// An empty / unusable MDE body must not fall back to Original — that Part GET 503s on 1.43.
/// Remux/re-encode via a separate registering decision is still allowed.
#[test]
#[cfg(feature = "devtriggers")]
fn unreachable_mde_does_not_return_the_part_url() {
    use std::time::Duration;
    let mut ps = crate::route::PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    let (port, rx, server) = plan_pms(4, EMPTY_MC);
    let sid = plx_plex::plex::register_for_test(
        "mde-empty",
        "127.0.0.1",
        port,
        "token",
        "mde-empty-client",
    );
    let mut env = ResolveEnv::snapshot(&ps, plx_data::stores::metadata::MetadataStore::default().view(), sid, "rk-4k");
    env.cached_item = Some(fourk_item(sid, vec![eac3_track()]));
    let plan = build_stream(
        &plx_base::task::OffFrame::for_test(),
        "rk-4k",
        "/library/parts/36013/1/file.mkv",
        "hevc",
        "eac3",
        &env,
    );
    let requests = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("PMS never saw the resolve");
    server.join().unwrap();

    assert!(
        requests
            .iter()
            .any(|line| line.contains("/decision?") && line.contains("hasMDE=1")),
        "MDE was still asked: {requests:?}"
    );
    assert!(
        plan.url.contains("start.mkv"),
        "no usable MDE → remux/re-encode, not Original: {}",
        plan.url
    );
    assert!(
        !plan.url.contains("/library/parts/36013/"),
        "unreachable MDE must not serve the Part: {}",
        plan.url
    );
    assert!(
        plan.contract.remux,
        "HEVC+EAC3 with unreachable MDE still codec-copy remuxes: {}",
        plan.url
    );
    plx_plex::plex::reset_servers_for_test();
}

/// Selected embedded SRT is client-rendered on Original. MDE must name that stream id
/// *and* `subtitles=none`: 1.43.4 400s hasMDE+directPlay with a selected subtitle and
/// the default `auto`. The mock PMS rejects that shape, so omitting the mode fail-closes
/// into remux + PUT sub=0 (the selected subtitle disappears).
#[test]
#[cfg(feature = "devtriggers")]
fn selected_embedded_srt_names_id_and_client_rendered_mode_on_mde() {
    use std::time::Duration;
    let mut ps = crate::route::PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    let (port, rx, server) = plan_pms(2, MDE_DIRECTPLAY);
    let sid = plx_plex::plex::register_for_test(
        "mde-srt",
        "127.0.0.1",
        port,
        "token",
        "mde-srt-client",
    );
    let mut env = ResolveEnv::snapshot(&ps, plx_data::stores::metadata::MetadataStore::default().view(), sid, "rk-4k");
    env.cached_item = Some(fourk_item_with_subs(
        sid,
        vec![eac3_track()],
        vec![selected_sub(55001, "srt")],
    ));
    let plan = build_stream(
        &plx_base::task::OffFrame::for_test(),
        "rk-4k",
        "/library/parts/36013/1/file.mkv",
        "hevc",
        "eac3",
        &env,
    );
    let requests = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("PMS never saw the resolve");
    server.join().unwrap();

    let decision = requests
        .iter()
        .find(|line| line.contains("/decision?"))
        .unwrap_or_else(|| panic!("MDE was never asked: {requests:?}"));
    assert_eq!(
        query_param(decision, "subtitleStreamID"),
        Some("55001"),
        "MDE must evaluate the SRT track Original will render: {decision}"
    );
    assert_eq!(
        query_param(decision, "subtitles"),
        Some("none"),
        "client-rendered mode; auto is the 1.43.4 400: {decision}"
    );
    assert!(
        plan.url.contains("/library/parts/36013/"),
        "selected SRT must stay Original, not remux: {}",
        plan.url
    );
    plx_plex::plex::reset_servers_for_test();
}

/// Selected PGS is client-rendered on Original; MDE must see that stream id (and the profile
/// must list pgs) so the decision stays directplay.
#[test]
#[cfg(feature = "devtriggers")]
fn selected_pgs_names_subtitle_stream_id_on_mde() {
    use std::time::Duration;
    let mut ps = crate::route::PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    let (port, rx, server) = plan_pms(2, MDE_DIRECTPLAY);
    let sid =
        plx_plex::plex::register_for_test("mde-pgs", "127.0.0.1", port, "token", "mde-pgs-client");
    let mut env = ResolveEnv::snapshot(&ps, plx_data::stores::metadata::MetadataStore::default().view(), sid, "rk-4k");
    env.cached_item = Some(fourk_item_with_subs(
        sid,
        vec![eac3_track()],
        vec![selected_sub(99001, "pgs")],
    ));
    let plan = build_stream(
        &plx_base::task::OffFrame::for_test(),
        "rk-4k",
        "/library/parts/36013/1/file.mkv",
        "hevc",
        "eac3",
        &env,
    );
    let requests = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("PMS never saw the resolve");
    server.join().unwrap();

    let decision = requests
        .iter()
        .find(|line| line.contains("/decision?"))
        .unwrap_or_else(|| panic!("MDE was never asked: {requests:?}"));
    assert_eq!(
        query_param(decision, "subtitleStreamID"),
        Some("99001"),
        "MDE must evaluate the PGS track Original will render: {decision}"
    );
    assert_eq!(
        query_param(decision, "subtitles"),
        Some("none"),
        "named bitmap still client-rendered, not auto/burn: {decision}"
    );
    assert!(
        plan.url.contains("/library/parts/36013/"),
        "PGS + EAC3 stays Original when MDE allows: {}",
        plan.url
    );
    plx_plex::plex::reset_servers_for_test();
}

/// A selected external sidecar is not in the container. MDE must see subtitleStreamID=0
/// so it does not force a burn; renderable text sidecars are restored by the client at landing.
#[test]
#[cfg(feature = "devtriggers")]
fn external_selected_sub_sends_subtitle_stream_id_zero_on_mde() {
    use std::time::Duration;
    let mut ps = crate::route::PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    let (port, rx, server) = plan_pms(2, MDE_DIRECTPLAY);
    let sid = plx_plex::plex::register_for_test(
        "mde-ext-sub",
        "127.0.0.1",
        port,
        "token",
        "mde-ext-client",
    );
    let mut env = ResolveEnv::snapshot(&ps, plx_data::stores::metadata::MetadataStore::default().view(), sid, "rk-4k");
    env.cached_item = Some(fourk_item_with_subs(
        sid,
        vec![eac3_track()],
        vec![{
            let mut s = selected_sub(88001, "srt");
            s.external = true;
            s
        }],
    ));
    let plan = build_stream(
        &plx_base::task::OffFrame::for_test(),
        "rk-4k",
        "/library/parts/36013/1/file.mkv",
        "hevc",
        "eac3",
        &env,
    );
    let requests = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("PMS never saw the resolve");
    server.join().unwrap();

    let decision = requests
        .iter()
        .find(|line| line.contains("/decision?"))
        .unwrap_or_else(|| panic!("MDE was never asked: {requests:?}"));
    assert_eq!(
        query_param(decision, "subtitleStreamID"),
        Some("0"),
        "sidecar is not burned on Original: {decision}"
    );
    assert_ne!(
        query_param(decision, "subtitleStreamID"),
        Some("88001"),
        "must not advertise the external id: {decision}"
    );
    assert!(
        plan.url.contains("/library/parts/36013/"),
        "MDE with subs off stays Original: {}",
        plan.url
    );
    plx_plex::plex::reset_servers_for_test();
}

/// Selected embedded `mov_text` (iTunes MP4) is client-rendered; the profile lists it so
/// MDE must see the stream id and stay Original rather than re-encoding.
#[test]
#[cfg(feature = "devtriggers")]
fn selected_mov_text_names_subtitle_stream_id_on_mde() {
    use std::time::Duration;
    let mut ps = crate::route::PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    let (port, rx, server) = plan_pms(2, MDE_DIRECTPLAY);
    let sid = plx_plex::plex::register_for_test(
        "mde-mov-text",
        "127.0.0.1",
        port,
        "token",
        "mde-mov-client",
    );
    let mut env = ResolveEnv::snapshot(&ps, plx_data::stores::metadata::MetadataStore::default().view(), sid, "rk-4k");
    env.cached_item = Some(fourk_item_with_subs(
        sid,
        vec![eac3_track()],
        vec![selected_sub(77001, "mov_text")],
    ));
    let plan = build_stream(
        &plx_base::task::OffFrame::for_test(),
        "rk-4k",
        "/library/parts/36013/1/file.mkv",
        "hevc",
        "eac3",
        &env,
    );
    let requests = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("PMS never saw the resolve");
    server.join().unwrap();

    let decision = requests
        .iter()
        .find(|line| line.contains("/decision?"))
        .unwrap_or_else(|| panic!("MDE was never asked: {requests:?}"));
    assert_eq!(
        query_param(decision, "subtitleStreamID"),
        Some("77001"),
        "MDE must evaluate the mov_text track Original will render: {decision}"
    );
    assert!(
        plan.url.contains("/library/parts/36013/"),
        "mov_text + EAC3 stays Original when MDE allows: {}",
        plan.url
    );
    plx_plex::plex::reset_servers_for_test();
}

/// PMS reports DVD bitmaps as `dvd_subtitle`; listing only `dvd` would MDE-transcode and
/// then forbid remux. The stream id must land on `/decision`.
#[test]
#[cfg(feature = "devtriggers")]
fn selected_dvd_subtitle_names_subtitle_stream_id_on_mde() {
    use std::time::Duration;
    let mut ps = crate::route::PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    let (port, rx, server) = plan_pms(2, MDE_DIRECTPLAY);
    let sid = plx_plex::plex::register_for_test(
        "mde-dvd-sub",
        "127.0.0.1",
        port,
        "token",
        "mde-dvd-client",
    );
    let mut env = ResolveEnv::snapshot(&ps, plx_data::stores::metadata::MetadataStore::default().view(), sid, "rk-4k");
    env.cached_item = Some(fourk_item_with_subs(
        sid,
        vec![eac3_track()],
        vec![selected_sub(66001, "dvd_subtitle")],
    ));
    let plan = build_stream(
        &plx_base::task::OffFrame::for_test(),
        "rk-4k",
        "/library/parts/36013/1/file.mkv",
        "hevc",
        "eac3",
        &env,
    );
    let requests = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("PMS never saw the resolve");
    server.join().unwrap();

    let decision = requests
        .iter()
        .find(|line| line.contains("/decision?"))
        .unwrap_or_else(|| panic!("MDE was never asked: {requests:?}"));
    assert_eq!(
        query_param(decision, "subtitleStreamID"),
        Some("66001"),
        "MDE must evaluate the dvd_subtitle track: {decision}"
    );
    assert!(
        plan.url.contains("/library/parts/36013/"),
        "dvd_subtitle + EAC3 stays Original when MDE allows: {}",
        plan.url
    );
    plx_plex::plex::reset_servers_for_test();
}

/// #584: a file whose only audio is DTS-HD MA must never direct-play on a television whose codec
/// table has no DTS decoder, even though the server's MDE (which judged the profile, not the
/// track the TV can feed) answers `directplay`. The server converts the audio on a video-copy
/// remux instead; a direct play would build an audio sink for a stream the pipeline cannot decode
/// and never start. Covers the codec spellings PMS uses, and an item whose tracks are not loaded
/// yet (the Media-level audio codec alone names the audio).
#[test]
#[cfg(feature = "devtriggers")]
fn an_undecodable_dts_only_file_does_not_direct_play_when_mde_says_it_may() {
    use std::time::Duration;
    for (codec, with_tracks) in [("dca", true), ("dts", true), ("dca", false), ("dts", false)] {
        let mut ps = crate::route::PlaybackSession::IDLE;
        let _g = fresh_registry(&mut ps);
        assert!(plx_net::net::global_init());
        let (port, rx, server) = plan_pms_within(3, MDE_DIRECTPLAY, Duration::from_secs(3), false);
        let sid = plx_plex::plex::register_for_test("dts-only", "127.0.0.1", port, "token", "dts-only-client");
        let mut env = ResolveEnv::snapshot(&ps, plx_data::stores::metadata::MetadataStore::default().view(), sid, "rk-4k");
        let tracks = if with_tracks {
            vec![plx_data::metadata::Stream {
                id: 29381,
                index: 1,
                lang_code: "eng".into(),
                codec: codec.into(),
                channels: 6,
                default: true,
                selected: true,
                profile: "dts-hd ma".into(),
                ..Default::default()
            }]
        } else {
            Vec::new()
        };
        env.cached_item = Some(fourk_item(sid, tracks));
        let plan = build_stream(
            &plx_base::task::OffFrame::for_test(), "rk-4k", "/library/parts/36013/1/file.mkv", "hevc", codec, &env,
        );
        let requests = rx.recv_timeout(Duration::from_secs(15)).expect("PMS never saw the resolve");
        server.join().unwrap();
        assert!(
            !plan.url.contains("/library/parts/"),
            "{codec} (tracks loaded: {with_tracks}) must not direct-play: {} / {requests:?}",
            plan.url
        );
        plx_plex::plex::reset_servers_for_test();
    }
}

/// The dev set's table shape for DTS: the decoder is listed with a 6-channel ceiling.
#[cfg(feature = "devtriggers")]
fn dts_row_caps() -> plx_os::devcaps::Caps {
    plx_os::devcaps::Caps {
        audio: "aac,ac3,eac3,dts".into(),
        audio_channels: [("dts".into(), 6)].into(),
        ..plx_os::devcaps::Caps::assumed()
    }
}

/// The other side of #584: on a set whose table DOES list a DTS decoder (here the dev set's 6ch
/// row), PMS's `dca` 5.1 track direct-plays when MDE allows, and the profile the request carries
/// names DTS in PMS's spelling. Before the canonicaliser the `dca` track never matched the `dts`
/// the caps listed, so it always converted.
#[test]
#[cfg(feature = "devtriggers")]
fn a_dca_track_direct_plays_on_a_set_with_a_dts_row() {
    use std::time::Duration;
    let mut ps = crate::route::PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    // The start edge a play request runs (`request_play_with_queue`): the attempt carries the
    // capability value the play start derived, and the resolve below plans and profiles from that
    // value alone. This set's table lists a DTS row, so that value is handed in, not installed.
    ps.begin_attempt(std::sync::Arc::new(dts_row_caps()));
    assert!(plx_net::net::global_init());
    let (port, rx, server) = plan_pms_within(3, MDE_DIRECTPLAY, Duration::from_secs(3), false);
    let sid = plx_plex::plex::register_for_test("dca-row", "127.0.0.1", port, "token", "dca-row-client");
    let mut env = ResolveEnv::snapshot(&ps, plx_data::stores::metadata::MetadataStore::default().view(), sid, "rk-4k");
    env.cached_item = Some(fourk_item(sid, vec![plx_data::metadata::Stream {
        id: 29381,
        index: 1,
        lang_code: "eng".into(),
        codec: "dca".into(),
        channels: 6,
        default: true,
        selected: true,
        ..Default::default()
    }]));
    let plan = build_stream(
        &plx_base::task::OffFrame::for_test(), "rk-4k", "/library/parts/36013/1/file.mkv", "hevc", "dca", &env,
    );
    let requests = rx.recv_timeout(Duration::from_secs(15)).expect("PMS never saw the resolve");
    server.join().unwrap();
    assert!(plan.url.contains("/library/parts/"), "dca on a DTS-row set must direct-play: {} / {requests:?}", plan.url);
    let decision = requests.iter().find(|l| l.contains("/decision?")).expect("MDE was asked");
    assert!(decision.contains("dca") && !decision.contains("dts"), "the profile names DTS as PMS does: {decision}");
    plx_plex::plex::reset_servers_for_test();
}


/// **The profile PMS reads is built from the attempt's own capability value.** Two attempts in one
/// process, one with a DTS row (`a`) and one narrowed by [`plx_os::devcaps::Caps::without`]
/// (`b`): the ProgressiveMkv transcode URL of `a` carries the `dca` copy lane and the one of `b`
/// does not, from the `caps` each [`TranscodeSpec`] holds. No global is installed or read; before
/// the attempt carried its value, both URLs came from the one process-wide answer.
#[test]
#[cfg(feature = "devtriggers")]
fn a_transcode_profile_names_the_codecs_of_the_attempts_own_caps() {
    use plx_os::devcaps::{Caps, Facet};
    let a = dts_row_caps();
    let b = a.without(&Facet::Audio { codec: "dts".into(), channels: 2 });
    let mut ps = crate::route::PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    let sid = plx_plex::plex::register_for_test("attempt-caps", "127.0.0.1", 1, "token", "attempt-caps-client");
    let client = plx_plex::plex::client_for(sid).expect("the registered server");
    let url = |caps: &Caps| {
        let contract = plx_plex::plex::EncodeContract::original(false, plx_plex::plex::AudioEnhancements::NONE);
        assert_eq!(contract.delivery, plx_plex::plex::TranscodeDelivery::ProgressiveMkv);
        let spec = crate::route::plan::transcode_spec(caps, "5", 0, "s", "s", plx_plex::plex::TranscodeOffset::Fresh, 0, 0, contract);
        client.transcode_start_url(&spec).to_url()
    };
    let (with_dts, without_dts) = (url(&a), url(&b));
    assert!(with_dts.contains("dca"), "the DTS row lists the copy lane: {with_dts}");
    assert!(!without_dts.contains("dca"), "the narrowed attempt must not ask PMS to copy DTS: {without_dts}");
    assert!(without_dts.contains("ac3"), "the rest of the profile is unchanged: {without_dts}");
    plx_plex::plex::reset_servers_for_test();
}

/// The fixture of the pick-spend tests: a movie page holding the viewer's subtitles-Off pick for file 9001,
/// and the plans resolved from it (the page's copy of the item, the pick applied, as the resolve does).
struct PickRig {
    ps: crate::route::PlaybackSession,
    store: plx_data::stores::metadata::MetadataStore,
    sid: plx_plex::plex::ServerId,
    off: plx_data::metadata::PreplayPick,
    _g: plx_base::testlock::Serial,
}

impl PickRig {
    const URL: &'static str = "https://example.invalid/m1.mkv";

    fn new() -> Self {
        use plx_data::metadata::{PreplayPick, SubPick};
        use plx_data::stores::metadata::MetadataCmd;
        let mut ps = crate::route::PlaybackSession::IDLE;
        let guard = fresh_registry(&mut ps);
        let sid = plx_plex::plex::ServerId::from_raw(0);
        let mut store = plx_data::stores::metadata::MetadataStore::default();
        let stream = |id| plx_data::metadata::Stream { id, ..Default::default() };
        plx_data::metadata::install_for_test(store.state_mut(), Some(plx_data::metadata::Detail {
            sid,
            rk: "m1".into(),
            kind: "movie".into(),
            part: "/library/parts/9001/1/file.mkv".into(),
            audio: vec![stream(11)],
            subs: vec![stream(21)],
            ..Default::default()
        }));
        let off = PreplayPick { part_id: 9001, audio_sid: 11, sub: SubPick::Off };
        assert!(store.run(MetadataCmd::PatchTracks { sid, rk: "m1".into(), pick: off }));
        Self { ps, store, sid, off, _g: guard }
    }

    /// A plan resolved NOW (the pick, if it is still held, applied to the item).
    fn plan(&self, url: &str) -> Plan {
        let mut playing = self.store.view().cached_playing(self.sid, "m1").expect("the page is the cache");
        playing.apply_picks(&self.store.view().picks_of(self.sid, "m1"), 9001);
        Plan { sid: self.sid, url: url.into(), part_id: 9001, playing: Some(playing), ..Default::default() }
    }

    /// The plan lands and the native Load is claimed, which is as far as `apply_plan` and the loop's
    /// synchronous acceptance go: nothing has said yet whether the engine started.
    fn land(&mut self, plan: Plan) -> Option<RouteStartAttempt> {
        let start = super::apply_plan(&mut self.ps, &mut self.store, plan, "m1", CandidatePurpose::NewCandidate);
        start.and_then(claim_route_start_attempt)
    }

    /// The engine's answer for `attempt`, then the loop's drain.
    fn settle(&mut self, attempt: RouteStartAttempt, result: RouteStartResult) -> bool {
        let settled = settle_route_start(&mut self.ps, attempt, result);
        spend_confirmed_picks(&mut self.ps, &mut self.store);
        settled
    }

    fn picks(&self) -> Vec<plx_data::metadata::PreplayPick> {
        self.store.view().picks_of(self.sid, "m1").to_vec()
    }

    /// The request in force for item `rk` (the one `request_play` publishes before its resolve).
    fn request(&self, rk: &str) -> PlaybackRequest {
        PlaybackRequest {
            sid: self.sid, rk: rk.into(), part: "/library/parts/9001/1/file.mkv".into(), vcodec: "h264".into(),
            acodec: "ac3".into(), title: "Film".into(), ctx: String::new(), preview: false, live: None, version: None,
            show_rk: String::new(),
        }
    }

    /// A landed playback whose start failed, the failure screen's pick `pick` held on it.
    fn failed_with_pick(&mut self, pick: &plx_data::metadata::PreplayPick) {
        let plan = self.plan(Self::URL);
        self.ps.request = Some(self.request("m1"));
        let first = self.land(plan).expect("a playable plan");
        assert!(self.settle(first, RouteStartResult::StartFailed));
        memory::hold_pick_for_retry(&mut self.ps, pick);
        assert_eq!(memory::failed_pick(&self.ps), Some(*pick), "setup: the failure screen shows the viewer's pick");
    }
}

/// **(b) A pick is spent by a playback that STARTS, not by a plan that was refused** (#1532): a refused
/// plan installs its item too (the verdict feeds the read-out), but nothing played, so BACK and Play must
/// still find the pick; the playable plan that follows spends it once the engine confirms it. A hero
/// preview is not the viewer's playback and spends nothing, and a playable retry that carries the failure
/// screen's pair (planned under no pick) still spends the pick held for its file, or the page's next
/// Resume would apply it. Through the real `apply_plan` and the real `settle_route_start`.
#[test]
fn a_refused_plan_keeps_the_pick_and_the_started_one_spends_it() {
    use plx_data::stores::metadata::MetadataCmd;
    let mut rig = PickRig::new();
    let off = rig.off;
    let (refused, playable) = (rig.plan(""), rig.plan(PickRig::URL));
    let (preview, retry) = (rig.plan(PickRig::URL), rig.plan(PickRig::URL));
    assert!(rig.land(refused).is_none(), "a refused plan has no start to wait for");
    spend_confirmed_picks(&mut rig.ps, &mut rig.store);
    assert_eq!(rig.picks(), [off], "a refused first attempt spent nothing");
    rig.ps.request = Some(PlaybackRequest {
        sid: rig.sid, rk: "m1".into(), part: "/library/parts/9001/1/file.mkv".into(), vcodec: "h264".into(),
        acodec: "ac3".into(), title: "Film".into(), ctx: String::new(), preview: true, live: None, version: None,
        show_rk: String::new(),
    });
    let previewed = rig.land(preview).expect("a preview's plan is playable");
    assert!(rig.settle(previewed, RouteStartResult::Started));
    assert_eq!(rig.picks(), [off], "a hero preview is not the viewer's playback");
    rig.ps.request = None;
    let attempt = rig.land(playable).expect("a playable plan");
    spend_confirmed_picks(&mut rig.ps, &mut rig.store);
    assert_eq!(rig.picks(), [off], "the plan's landing is not the start");
    assert!(rig.settle(attempt, RouteStartResult::Started));
    assert!(rig.picks().is_empty(), "the playback that started under the pick spent it");

    // the retry that carries the failure screen's pair is planned under no pick (`picks_in_force`), yet
    // its start is the start the pick was waiting for
    assert!(rig.store.run(MetadataCmd::PatchTracks { sid: rig.sid, rk: "m1".into(), pick: off }));
    let mut under_retry = ResolveEnv::snapshot(&rig.ps, rig.store.view(), rig.sid, "m1");
    under_retry.selection_before_retry = Some(SelectionBeforeRetry {
        retry_part_id: 9001, audio_sid: 0, sub_sid: 0, subtitle_override: None, sub_mode: SubMode::Track, memory: Vec::new(),
    });
    assert!(under_retry.picks_in_force().is_empty(), "setup: the retry is planned under no pick");
    let attempt = rig.land(retry).expect("the retry's plan is playable");
    assert_eq!(rig.picks(), [off], "the retry's landing is not its start either");
    assert!(rig.settle(attempt, RouteStartResult::Started));
    assert!(rig.picks().is_empty(), "a start that was not fed the pick still spends it");
}

/// **An engine that fails to start spends nothing**: the viewer picks subtitles Off and presses Play, the
/// plan lands, the engine then fails (`StartFailed`, and `NoRoute` is the same edge), and the failure screen
/// shows. BACK and Play is a fresh launch: it must find the pick, be planned under it, and spend it only
/// when THAT start is confirmed.
#[test]
fn a_plan_that_lands_and_then_fails_to_start_keeps_the_pick_for_the_next_launch() {
    for failure in [RouteStartResult::StartFailed, RouteStartResult::NoRoute] {
        let mut rig = PickRig::new();
        let off = rig.off;
        // the plans are resolved at their presses; the first install marks the page's copy as played
        let (first, second) = (rig.plan(PickRig::URL), rig.plan(PickRig::URL));
        let attempt = rig.land(first).expect("a playable plan");
        assert!(rig.settle(attempt, failure));
        assert_eq!(rig.picks(), [off], "{failure:?}: the viewer's Off outlives a start that never happened");
        // BACK, Play: the resolve snapshots the picks of a launch with no retry pair
        let env = ResolveEnv::snapshot(&rig.ps, rig.store.view(), rig.sid, "m1");
        assert_eq!(env.picks_in_force(), [off], "{failure:?}: the fresh launch is planned under the pick");
        let attempt = rig.land(second).expect("the second launch's plan");
        assert!(rig.settle(attempt, RouteStartResult::Started));
        assert!(rig.picks().is_empty(), "{failure:?}: the launch that started spent it");
    }
}

/// **A superseded or abandoned attempt never spends**: an attempt's own `Started` after its transaction was
/// replaced is refused by the reducer, and what the replaced plan held went with it.
#[test]
fn a_superseded_attempt_does_not_spend_the_pick() {
    let mut rig = PickRig::new();
    let off = rig.off;
    let (a, b, refused, third) = (rig.plan(PickRig::URL), rig.plan(PickRig::URL), rig.plan(""), rig.plan(PickRig::URL));
    let first = rig.land(a).expect("a playable plan");
    assert!(rig.settle(first, RouteStartResult::StartFailed));
    // a second launch lands, and a refused one replaces it before its engine answers
    let second = rig.land(b).expect("the second launch");
    assert!(rig.land(refused).is_none());
    assert!(!rig.settle(first, RouteStartResult::Started), "the first attempt's late Started is refused");
    assert!(!rig.settle(second, RouteStartResult::Started), "the replaced launch's Started is refused too");
    assert_eq!(rig.picks(), [off], "no superseded or abandoned attempt spent the pick");
    // a transaction aborted before any Load result spends nothing either
    let attempt = rig.land(third).expect("a third launch");
    assert!(abort_route_start(RouteStartTransaction { serial: attempt.serial }, RouteStartResult::StartFailed));
    assert!(!rig.settle(attempt, RouteStartResult::Started));
    assert_eq!(rig.picks(), [off]);
}

/// **The pick is held for the PLAN, not for one route transaction**: a start that failed and is retried
/// under a transaction of its own (`begin_route_start` from `Failed`, as a cold resume's rebuild or a
/// recovery does) is the same landed plan starting, and its confirmation spends the pick. A playback the
/// viewer left in between (a full stop) is abandoned, and the same later start spends nothing.
#[test]
fn a_retry_of_the_landed_plan_under_a_new_transaction_spends_and_a_stopped_one_does_not() {
    let mut rig = PickRig::new();
    let plan = rig.plan(PickRig::URL);
    let first = rig.land(plan).expect("a playable plan");
    assert!(rig.settle(first, RouteStartResult::StartFailed));
    let retry = begin_route_start().expect("a failed route owns no engine: a new transaction");
    assert_ne!(retry.serial, first.serial, "setup: the retry is a transaction of its own");
    let second = claim_route_start_attempt(retry).expect("the retry's attempt");
    assert!(rig.settle(second, RouteStartResult::Started));
    assert!(rig.picks().is_empty(), "the landed plan's start spent the pick under its retry's transaction");

    drop(rig); // the rig holds the process-wide test lock
    let mut rig = PickRig::new();
    let off = rig.off;
    let plan = rig.plan(PickRig::URL);
    let first = rig.land(plan).expect("a playable plan");
    assert!(rig.settle(first, RouteStartResult::StartFailed));
    retire_stopped_playback(&mut rig.ps, StopKind::Visit);
    let retry = begin_route_start().expect("a new transaction");
    let second = claim_route_start_attempt(retry).expect("the retry's attempt");
    assert!(rig.settle(second, RouteStartResult::Started));
    assert_eq!(rig.picks(), [off], "an abandoned playback never spends");
}

/// **A request cancelled before it lands leaves the pick with the plan it restores**: the plan lands and
/// its start fails; a retry's request begins (it clears the live selection, as every request does) and is
/// cancelled, which restores the old plan; that plan's start is then confirmed and spends the pick. A
/// request dropping the pair at its start would lose it here, the pick surviving the page for good.
#[test]
fn a_cancelled_request_keeps_the_restored_plans_pick_to_spend() {
    let mut rig = PickRig::new();
    let off = rig.off;
    let plan = rig.plan(PickRig::URL);
    let first = rig.land(plan).expect("a playable plan");
    assert!(rig.settle(first, RouteStartResult::StartFailed));
    assert_eq!(rig.picks(), [off], "setup: the failed start spent nothing");

    begin_playback_request().expect("request admission");
    begin_track_selection(&mut rig.ps, None);
    cancel_playback_request(&mut rig.ps, true);

    let retry = begin_route_start().expect("the restored plan's failed route owns no engine");
    let second = claim_route_start_attempt(retry).expect("the restored plan's attempt");
    assert!(rig.settle(second, RouteStartResult::Started));
    assert!(rig.picks().is_empty(), "the restored plan's start spent the pick the cancelled request found held");
}

/// **A landing for another item or another part replaces the pair**: plan A (`m1`, part 9001) lands and
/// fails to start, so the viewer's pick for it is still held; plan B (another item, then another file of
/// the same item) lands and starts. B spends nothing of A's: the spend is keyed by the item and part it
/// started, and A's pick stays held for A's page. Through the real `apply_plan` and `settle_route_start`.
#[test]
fn a_landing_for_another_item_or_part_replaces_the_pair_and_spends_nothing_of_the_first() {
    let mut rig = PickRig::new();
    let off = rig.off;
    // resolved at their presses: the first install marks the page's copy as played
    let (plan_a, mut other, mut other_part) = (rig.plan(PickRig::URL), rig.plan(PickRig::URL), rig.plan(PickRig::URL));
    other.part_id = 9002;
    other.playing.as_mut().expect("the plan carries its item").rk = "m2".into();
    other_part.part_id = 9002;
    let a = rig.land(plan_a).expect("plan A is playable");
    assert!(rig.settle(a, RouteStartResult::StartFailed));
    assert_eq!(rig.picks(), [off], "setup: A's failed start spent nothing");
    let spend_of = |rig: &PickRig| rig.ps.pick_spend_on_start.clone().map(|s| (s.rk, s.part_id));
    assert_eq!(spend_of(&rig), Some(("m1".to_string(), 9001)), "setup: the pair is A's");

    // B, another item: `m2`'s own file, landed under its own rating key
    let attempt = super::apply_plan(&mut rig.ps, &mut rig.store, other, "m2", CandidatePurpose::NewCandidate)
        .and_then(claim_route_start_attempt).expect("plan B is playable");
    assert_eq!(spend_of(&rig), Some(("m2".to_string(), 9002)), "the landing replaced the pair");
    assert!(rig.settle(attempt, RouteStartResult::Started));
    assert!(rig.ps.pick_spend_on_start.is_none(), "B's confirmation released its own pair");
    assert_eq!(rig.picks(), [off], "B spent nothing of A's");

    // B', another file of the same item: the pair is (m1, 9002), not A's (m1, 9001)
    let attempt = rig.land(other_part).expect("plan B' is playable");
    assert_eq!(spend_of(&rig), Some(("m1".to_string(), 9002)), "the landing replaced the pair");
    assert!(rig.settle(attempt, RouteStartResult::Started));
    assert_eq!(rig.picks(), [off], "another file's start spent nothing of the pick held for 9001");
}

/// **The failure screen's pick survives a retry whose request is cancelled** (`held_pick` follows the rule
/// `pick_spend_on_start` does: a REQUEST does not clear it, the LANDING that consumes it does). The plan
/// lands and its start fails; the viewer picks in the panel and presses Try again; Home runs `cancel_play`
/// during the resolve. The panel's seed (`failed_pick`) and the next retry's context must still read the
/// viewer's pick, not the failed attempt's pair; a later retry that lands consumes it once.
#[test]
fn a_cancelled_retry_request_keeps_the_failure_screens_held_pick_until_a_landing_consumes_it() {
    use plx_data::metadata::{PreplayPick, SubPick};
    let mut rig = PickRig::new();
    // resolved at their presses: the first install marks the page's copy as played
    let (first_plan, retry_plan) = (rig.plan(PickRig::URL), rig.plan(PickRig::URL));
    rig.ps.request = Some(rig.request("m1"));
    let first = rig.land(first_plan).expect("a playable plan");
    assert!(rig.settle(first, RouteStartResult::StartFailed));

    let pick = PreplayPick { part_id: 9001, audio_sid: 11, sub: SubPick::Track(21) };
    assert_ne!(memory::failed_pick(&rig.ps), Some(pick), "setup: the failed attempt stood on another pair");
    memory::hold_pick_for_retry(&mut rig.ps, &pick);
    assert_eq!(memory::failed_pick(&rig.ps), Some(pick));

    // Try again: the request begins, carrying the held pick, and is cancelled before it lands
    let retry = current_retry_context(&rig.ps, 0);
    begin_playback_request().expect("request admission");
    begin_track_selection(&mut rig.ps, Some(retry));
    cancel_playback_request(&mut rig.ps, true);
    let held = |sel: Option<HeldSelection>| sel.map(|s| (s.part_id, s.audio_sid, s.sub_sid));
    assert_eq!(memory::failed_pick(&rig.ps), Some(pick), "the panel still opens on the viewer's pick");
    assert_eq!(held(current_retry_context(&rig.ps, 0).selection), Some((9001, 11, 21)), "and the next retry carries it");

    // the retry that lands consumes it, once
    rig.land(retry_plan).expect("the retry's plan is playable");
    assert!(rig.ps.held_pick.is_none(), "the landing consumed it");
}

/// **A failure screen never reads another item's held pick**: A's failure pick is held, then a request for
/// item B fails before it lands. B's failure shows (`failed_pick`) and retries (`current_retry_context`)
/// no pick, not A's. B's fresh request ENDS A's pick (a new player visit), so a fresh request for A finds
/// nothing either; only a RETRY of A, which is the same visit, reads it.
#[test]
fn another_items_failure_never_reads_the_held_pick() {
    use plx_data::metadata::{PreplayPick, SubPick};
    let mut rig = PickRig::new();
    let pick = PreplayPick { part_id: 9001, audio_sid: 11, sub: SubPick::Track(21) };
    rig.failed_with_pick(&pick);

    // the reader keys the item: with B's request in force A's pick is not read, with A's it is
    let request_a = rig.request("m1");
    rig.ps.request = Some(rig.request("m2"));
    assert_eq!(held_pick(&rig.ps), None, "B's request does not read A's pick");
    rig.ps.request = Some(request_a.clone());
    assert!(held_pick(&rig.ps).is_some(), "A's own request does");

    // item B is requested (fresh) and fails before it lands
    rig.ps.request = Some(rig.request("m2"));
    begin_track_selection(&mut rig.ps, None);
    assert_eq!(memory::failed_pick(&rig.ps), None, "B's failure has no pick: A's is not B's");
    assert_eq!(current_retry_context(&rig.ps, 0).selection, None, "and B's retry carries none");
    assert!(rig.ps.held_pick.is_none(), "B's fresh request ended the player visit");
    rig.ps.request = Some(request_a.clone());
    begin_track_selection(&mut rig.ps, None);
    assert_eq!(memory::failed_pick(&rig.ps), None, "a fresh request for A does not find the older pick");

    // a RETRY for A is the same visit: it finds it
    memory::hold_pick_for_retry(&mut rig.ps, &pick);
    let retry = current_retry_context(&rig.ps, 0);
    begin_track_selection(&mut rig.ps, Some(retry));
    assert_eq!(memory::failed_pick(&rig.ps), Some(pick), "a retry for A reads it");
}

/// **The older pick never beats the newer one** (the scenario that put `held_pick` under ONE lifetime rule): A
/// fails, the viewer picks P1 on the failure panel, the retry is cancelled, BACK to Detail, P2 is picked
/// there and Play is pressed (a fresh request), and A fails before it lands. The failure panel and its
/// Try again start from P2's (standing) selection, not the stale P1.
#[test]
fn a_fresh_request_ends_the_failure_screens_held_pick() {
    use plx_data::metadata::{PreplayPick, SubPick};
    let mut rig = PickRig::new();
    let p1 = PreplayPick { part_id: 9001, audio_sid: 11, sub: SubPick::Track(21) };
    rig.failed_with_pick(&p1);

    // Try again, cancelled by BACK
    let retry = current_retry_context(&rig.ps, 0);
    begin_playback_request().expect("request admission");
    begin_track_selection(&mut rig.ps, Some(retry));
    cancel_playback_request(&mut rig.ps, true);
    assert_eq!(memory::failed_pick(&rig.ps), Some(p1), "a cancelled retry keeps it");

    // Detail: P2 picked there, Play pressed: a fresh request for A that fails before it lands
    begin_playback_request().expect("request admission");
    begin_track_selection(&mut rig.ps, None);
    cancel_playback_request(&mut rig.ps, false);
    assert!(rig.ps.held_pick.is_none(), "the fresh request ended the older pick");
    assert_ne!(memory::failed_pick(&rig.ps), Some(p1), "the panel does not seed the older pick");
    assert_ne!(current_retry_context(&rig.ps, 0).selection.map(|s| (s.part_id, s.audio_sid, s.sub_sid)), Some((9001, 11, 21)), "and Try again does not carry it");
}

/// **A full stop ends the held pick**: the failure screen's pick does not outlive the player visit, so a Play
/// after the stop starts from nothing held ([`retire_stopped_playback`], beside `pick_spend_on_start`).
#[test]
fn a_full_stop_then_play_holds_no_pick() {
    use plx_data::metadata::{PreplayPick, SubPick};
    let mut rig = PickRig::new();
    let pick = PreplayPick { part_id: 9001, audio_sid: 11, sub: SubPick::Track(21) };
    rig.failed_with_pick(&pick);
    retire_stopped_playback(&mut rig.ps, StopKind::Visit);
    assert!(rig.ps.held_pick.is_none(), "the stop ended the visit");
    rig.ps.request = Some(rig.request("m1"));
    begin_track_selection(&mut rig.ps, None);
    assert_ne!(memory::failed_pick(&rig.ps), Some(pick), "Play after the stop: no pick");
    assert_ne!(current_retry_context(&rig.ps, 0).selection.map(|s| (s.part_id, s.audio_sid, s.sub_sid)), Some((9001, 11, 21)));
}

/// **A cancelled retry, then a retry: the pick is kept** (a retry and `cancel_play` are the same visit, a background
/// suspend included), and a fresh in-flight resolve's own retry context (the contract re-request, the URI
/// feed fallback) cannot apply a stale one, because the fresh request held none.
#[test]
fn a_cancelled_retry_then_a_retry_keeps_the_pick_and_a_fresh_request_carries_none() {
    use plx_data::metadata::{PreplayPick, SubPick};
    let mut rig = PickRig::new();
    let pick = PreplayPick { part_id: 9001, audio_sid: 11, sub: SubPick::Track(21) };
    rig.failed_with_pick(&pick);
    for _ in 0..2 {
        let retry = current_retry_context(&rig.ps, 0);
        assert_eq!(retry.selection.map(|s| (s.part_id, s.audio_sid, s.sub_sid)), Some((9001, 11, 21)), "the retry carries the pick");
        begin_playback_request().expect("request admission");
        begin_track_selection(&mut rig.ps, Some(retry));
        cancel_playback_request(&mut rig.ps, true);
        assert_eq!(memory::failed_pick(&rig.ps), Some(pick), "kept across the cancelled retry");
    }

    // a fresh request that is still in flight: `current_retry_context` (the contract re-request and the
    // feed fallback read it) has no held pick to apply
    begin_track_selection(&mut rig.ps, None);
    assert_ne!(current_retry_context(&rig.ps, 0).selection.map(|s| (s.part_id, s.audio_sid, s.sub_sid)), Some((9001, 11, 21)));
}
