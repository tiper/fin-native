//! **A Dolby Vision Profile 5 never has its video copied, whatever route the play took to get
//! there.** A Profile 5 base layer is not a picture the set can show on its own, so every rebuild
//! that sends the file to the server must withdraw the copy permission (`directStream=0`). The
//! route's contract cannot carry that alone: a return to Original resets it to the direct play's,
//! and the audio conversion that follows rebuilds from it. Each test drives a play against the
//! loopback server of [`FlightRig`] and reads the rebuild's own request.

use super::flight_rig::FlightRig;
use super::test_support::settle_pending_native_start;
use super::*;
use plx_os::devcaps::dv::DvCapability;
use std::time::Duration;

const P5: plx_playback_contract::feed::RpuDovi = plx_playback_contract::feed::RpuDovi { profile: 5, bl_compat: 0 };
const P81: plx_playback_contract::feed::RpuDovi = plx_playback_contract::feed::RpuDovi { profile: 8, bl_compat: 1 };

/// A direct play of a file with no Dolby Vision record on a set with confirmed support, open to
/// the RPUs' declaration, which then declares `rpu` as the demuxer's report does.
fn direct_play_declaring(rpu: plx_playback_contract::feed::RpuDovi) -> (PlaybackSession, FlightRig) {
    let mut ps = PlaybackSession::IDLE;
    let rig = FlightRig::start_direct(&mut ps, Duration::ZERO);
    assert!(set_stream_declaration_for_test(
        &mut ps, "hevc", "ac3", 23.976, plx_data::metadata::Dovi::NONE, false, DvCapability::Supported,
    ));
    let none = plx_data::metadata::Dovi::NONE;
    let mode = plx_plex::plex::session::DolbyVisionMode::Auto;
    set_source_declaration_for_test(&mut ps, none, frozen_dv_decision(mode, DvCapability::Supported, none, "hevc"));
    open_rpu_dovi_for_test(&mut ps, mode);
    assert!(declare_rpu_dovi(&mut ps, rpu), "the premise: the RPUs are declared on direct play");
    (ps, rig)
}

/// The viewer lowers the quality (a capped re-encode of the file) and then picks Original again:
/// the recovery restores the direct play from the stored candidate, which carries what the
/// declaration adopted, and resets the route's contract to the direct play's.
fn lower_then_back_to_original(ps: &mut PlaybackSession) {
    install_transcode_for_test(ps, false, false);
    mark_fixed_quality_ceiling_for_test(ps);
    FlightRig::offer_original(ps, true);
    let (dovi, decision) = source_dv_for_test(ps);
    set_auto_original_dv_for_test(ps, dovi, decision);
    assert_eq!(recover_auto_to_original(ps, 90), Some(AutoOriginalReload::Direct));
    settle_pending_native_start(ps, RouteStartResult::Started);
    assert!(!is_transcoding(ps), "the premise: back on the direct play");
    assert!(!ps.cur_contract.no_video_copy, "the premise: the recovery reset the contract");
}

/// The rebuild an audio pick the set cannot decode asks for (a plain retranscode of the route at
/// the pick's position), as the request the server received.
fn audio_conversion_request(ps: &mut PlaybackSession, rig: &FlightRig) -> String {
    let before = rig.decision_requests().len();
    let url = retranscode_for(ps, &worker_ticket(), 120).expect("the rebuild is applied");
    let decisions = rig.decision_requests();
    assert_eq!(decisions.len(), before + 1, "one /decision for the rebuild");
    assert!(url.contains("/transcode/universal/start"), "{url}");
    decisions.last().cloned().expect("the rebuild's /decision")
}

/// Whether `request` gave the server permission to copy the video (`directStream=1`).
fn copies_video(request: &str) -> bool {
    match super::test_support::query_param(request, "directStream") {
        Some("1") => true,
        Some("0") => false,
        other => panic!("directStream={other:?}: {request}"),
    }
}

fn cleanup(ps: &mut PlaybackSession, rig: FlightRig) {
    let _serial = rig.close();
    drop(take_pending_original());
    drop(take_claim_landing());
    reset_player_control_for_test(idle_session_for_test());
    reset_session(ps);
}

#[test]
fn an_inferred_profile_5_back_on_original_never_lets_an_audio_conversion_copy_its_video() {
    let (mut ps, rig) = direct_play_declaring(P5);
    lower_then_back_to_original(&mut ps);
    assert_eq!(stream_dv_presentation(&ps).declared().map(|n| n.profile_id), Some(5), "Original keeps the node");
    let request = audio_conversion_request(&mut ps, &rig);
    assert!(!copies_video(&request), "a Profile 5 base layer was offered for a copy: {request}");
    cleanup(&mut ps, rig);
}

#[test]
fn a_profile_5_with_a_record_never_lets_an_audio_conversion_copy_its_video() {
    let mut ps = PlaybackSession::IDLE;
    let rig = FlightRig::start_direct(&mut ps, Duration::ZERO);
    let p5 = plx_data::metadata::Dovi { present: true, profile: 5, bl_present: true, rpu_present: true, ..plx_data::metadata::Dovi::NONE };
    assert!(set_stream_declaration_for_test(&mut ps, "hevc", "ac3", 23.976, p5, false, DvCapability::Supported));
    let decision = crate::route::stream_dv_decision(&ps);
    set_source_declaration_for_test(&mut ps, p5, decision);
    assert!(!ps.cur_contract.no_video_copy, "the premise: a direct play's contract forbids nothing");
    let request = audio_conversion_request(&mut ps, &rig);
    assert!(!copies_video(&request), "a Profile 5 base layer was offered for a copy: {request}");
    // The same after a trip to a lower quality and back to Original.
    lower_then_back_to_original(&mut ps);
    let request = audio_conversion_request(&mut ps, &rig);
    assert!(!copies_video(&request), "a Profile 5 base layer was offered for a copy: {request}");
    cleanup(&mut ps, rig);
}

/// The control: a Profile 8.1 found in the RPUs has a base layer the set shows, so the conversion
/// still copies the video (and a copy carries the declaration, `copied_video_dv`).
#[test]
fn an_inferred_profile_81_back_on_original_still_copies_its_video_for_an_audio_conversion() {
    let (mut ps, rig) = direct_play_declaring(P81);
    lower_then_back_to_original(&mut ps);
    let request = audio_conversion_request(&mut ps, &rig);
    assert!(copies_video(&request), "a usable base layer lost its copy: {request}");
    cleanup(&mut ps, rig);
}
