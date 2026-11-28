//! **The remembered version** (`version_choices`, a tenant of the memory door): what Resume and
//! Play start when they name no version, what a play that names one leaves behind, and what the
//! Detail page's facts line describes. Per ITEM and per profile, kept across a reload, and a choice
//! whose media is gone from the item plays exactly what no choice would.
use super::test_support::*;
use super::*;
use plx_data::pms::MediaVersion;
use plx_plex::plex::session::prefs::ProfileList;
use plx_plex::plex::session::{self as session, Preferences, Session, VersionChoiceEntry};
use plx_plex::plex::MediaId;

const MACHINE: &str = "machine-vc";

fn record_with(lists: Vec<ProfileList<VersionChoiceEntry>>) -> Session {
    Session {
        client_id: "cid-1".into(),
        account_token: "acct".into(),
        prefs: Preferences { version_choices: lists, ..Default::default() },
        ..Default::default()
    }
}

fn entry(rk: &str, media: i64) -> VersionChoiceEntry {
    VersionChoiceEntry { machine_id: MACHINE.into(), rating_key: rk.into(), media_id: media, ..Default::default() }
}

fn list(user: &str, items: Vec<VersionChoiceEntry>) -> ProfileList<VersionChoiceEntry> {
    let account = plx_plex::plex::grant::account_key("acct");
    ProfileList { user: user.into(), account, items, ..Default::default() }
}

fn tidy() {
    plx_plex::plex::reset_servers_for_test();
    unseed_memory_for_test::<VersionChoiceEntry>();
}

fn versions(ids: &[i64]) -> Vec<MediaVersion> {
    ids.iter().map(|&id| MediaVersion { id: MediaId(id), ..Default::default() }).collect()
}

/// A seated owner, a registered server and an empty (but seeded) mirror: the state a signed-in
/// television is in. The guard serialises the process-global registry and mirror.
fn seated() -> (plx_base::testlock::Serial, session::TempSession, ServerId) {
    let mut ps = PlaybackSession::IDLE;
    let guard = fresh_registry(&mut ps);
    let session = session::TempSession::new("version-choice");
    assert_eq!(session.seated("", "acct"), plx_plex::plex::grant::account_key("acct"));
    let sid = plx_plex::plex::register_for_test(MACHINE, "127.0.0.1", 32411, "tok", "cid-vc");
    seed_memory_for_test::<VersionChoiceEntry>(&record_with(Vec::new()));
    (guard, session, sid)
}

#[test]
fn a_choice_is_per_item_and_per_profile_and_survives_a_reload() {
    let (_g, session, sid) = seated();
    assert_eq!(chosen_version(sid, "10"), None, "nothing is remembered before a choice");
    assert!(choose_version(sid, "10", MediaId(3086)));
    assert_eq!(chosen_version(sid, "10"), Some(MediaId(3086)));
    assert_eq!(chosen_version(sid, "11"), None, "the next episode does not inherit it (one item, one choice)");

    // A reload: the mirror is dropped and seeded again from what the file holds.
    let stamp = plx_plex::plex::grant::account_key("acct");
    let file = record_with(vec![list("", vec![entry("10", 3086)])]);
    unseed_memory_for_test::<VersionChoiceEntry>();
    assert_eq!(chosen_version(sid, "10"), None, "an unseeded mirror answers nothing");
    seed_memory_for_test::<VersionChoiceEntry>(&file);
    assert_eq!(chosen_version(sid, "10"), Some(MediaId(3086)), "the stored choice is read back after a reload");

    // Another profile's list is its own.
    let kid_file = record_with(vec![list("", vec![entry("10", 3086)]), list("kid", vec![entry("10", 3082)])]);
    seed_memory_for_test::<VersionChoiceEntry>(&kid_file);
    assert_eq!(session.seated("kid", "acct"), stamp);
    assert_eq!(chosen_version(sid, "10"), Some(MediaId(3082)), "a profile reads its own choice");
    assert_eq!(session.seated("", "acct"), stamp);
    assert_eq!(chosen_version(sid, "10"), Some(MediaId(3086)));

    // A choice written to the file is soft-parsed like the other tenants and round-trips.
    let json = serde_json::to_value(&kid_file.prefs).unwrap();
    assert_eq!(json["version_choices"][0]["items"][0]["media_id"], 3086);
    let back: Preferences = serde_json::from_value(json).unwrap();
    assert_eq!(back.version_choices, kid_file.prefs.version_choices, "the list round-trips through the file");
    tidy();
}

#[test]
fn a_choice_needs_a_seated_profile_and_a_registered_server() {
    let (_g, session, sid) = seated();
    session.watching("");
    assert!(!choose_version(sid, "10", MediaId(3086)), "a publication with no stamp seats nobody");
    assert_eq!(chosen_version(sid, "10"), None);
    assert!(!choose_version(ServerId::UNSET, "10", MediaId(3086)), "no server, no machine id to key it by");
    tidy();
}

#[test]
fn a_play_that_names_no_version_takes_the_remembered_one_and_a_named_one_is_remembered() {
    let (_g, _session, sid) = seated();
    let two = versions(&[3082, 3086]);
    assert_eq!(settle_version(sid, "10", None, Some(&two)), None, "nothing remembered: the server's first file, as before");
    assert_eq!(settle_version(sid, "10", Some(MediaId(3086)), Some(&two)), Some(MediaId(3086)), "a named version stands");
    assert_eq!(chosen_version(sid, "10"), Some(MediaId(3086)), "and is what the next play takes");
    assert_eq!(settle_version(sid, "10", None, Some(&two)), Some(MediaId(3086)), "Resume plays the chosen file");
    assert_eq!(settle_version(sid, "10", None, None), Some(MediaId(3086)), "an item whose versions are not loaded takes it too");
    assert_eq!(settle_version(sid, "11", None, Some(&two)), None, "it is not carried to the next episode");
    // Playing the first file on purpose is a choice as well.
    assert_eq!(settle_version(sid, "10", Some(MediaId(3082)), Some(&two)), Some(MediaId(3082)));
    assert_eq!(settle_version(sid, "10", None, Some(&two)), Some(MediaId(3082)));
    tidy();
}

#[test]
fn a_stale_choice_plays_what_no_choice_would() {
    let (_g, _session, sid) = seated();
    assert!(choose_version(sid, "10", MediaId(3086)));
    let gone = versions(&[3082, 3090]);
    let stale = settle_version(sid, "10", None, Some(&gone));
    unseed_memory_for_test::<VersionChoiceEntry>();
    seed_memory_for_test::<VersionChoiceEntry>(&record_with(Vec::new()));
    let none = settle_version(sid, "10", None, Some(&gone));
    assert_eq!(stale, None, "the media is gone: no version is named, so the automatic default applies");
    assert_eq!(stale, none, "the same answer as having no stored id");
    tidy();
}

#[test]
fn the_facts_follow_the_choice_and_a_stale_or_first_one_changes_nothing() {
    use plx_base::detail_parts::DynamicRange;
    use plx_data::metadata::Detail;
    let caps = plx_os::devcaps::Caps::assumed();
    let uhd = MediaVersion {
        id: MediaId(1), resolution: "4k".into(), width: 3840, height: 2160, vcodec: "hevc".into(), hdr: true, ..Default::default()
    };
    let hd = MediaVersion {
        id: MediaId(2), resolution: "1080".into(), width: 1920, height: 1080, vcodec: "h264".into(), ..Default::default()
    };
    let d = Detail {
        rk: "10".into(), part: "/primary".into(), video_resolution: "4k".into(), vcodec: "hevc".into(), hdr: true,
        versions: vec![uhd, hd], ..Default::default()
    };
    let facts = |mode, chosen| {
        crate::route::played_video(
            &d, mode, &caps, plx_plex::plex::session::DolbyVisionMode::Auto, plx_os::devcaps::dv::DvCapability::Unknown, chosen,
        )
        .unwrap()
    };
    let default = facts(DirectPlayMode::Auto, None);
    assert!(default.version.is_none() && default.range() == DynamicRange::Hdr && default.resolution == "4k");
    let chosen = facts(DirectPlayMode::Auto, Some(MediaId(2)));
    assert_eq!(chosen.version.map(|v| v.id), Some(MediaId(2)), "the chosen file is the one described");
    assert_eq!((chosen.resolution, chosen.range()), ("1080", DynamicRange::Sdr));
    assert_eq!(facts(DirectPlayMode::Forced, Some(MediaId(2))).resolution, "1080", "a choice holds in every direct-play mode");
    let first = facts(DirectPlayMode::Auto, Some(MediaId(1)));
    assert!(first.version.is_none() && first.resolution == "4k", "choosing the first is the primary part's own facts");
    let stale = facts(DirectPlayMode::Auto, Some(MediaId(99)));
    assert_eq!(stale.resolution, default.resolution, "a choice whose media is gone describes what no choice would");
    assert_eq!(d.play_rk(), "10");
}

#[test]
fn a_show_page_keys_the_choice_by_its_episode() {
    use plx_data::metadata::{Detail, Episode, EpisodeStreams};
    let ep = Episode { rk: "ep-1".into(), part: "/ep".into(), versions: versions(&[5, 6]), ..Default::default() };
    let d = Detail {
        rk: "show".into(), is_show: true, episodes: vec![ep].into(), episode_streams: EpisodeStreams::Loaded("ep-1".into()),
        ..Default::default()
    };
    assert_eq!(d.play_rk(), "ep-1", "the choice belongs to the episode Play starts, not to the show");
    assert_eq!(d.versions_known("ep-1").map(<[_]>::len), Some(2));
    assert_eq!(d.versions_known("show"), None, "a show has no versions of its own");
    assert_eq!(d.versions_known("other"), None, "an item not on the page is not known");
}

/// **Resume and Play, through the real request**: a play that names no version asks for the one the
/// profile selected for THAT item; the next item asks for none; a play that names one (the player's
/// own switch, or a hold menu's) is remembered for next time.
#[test]
fn a_request_carries_the_remembered_version_and_remembers_a_named_one() {
    let (_g, _session, sid) = seated();
    let mut ps = PlaybackSession::IDLE;
    let mut meta = plx_data::stores::metadata::MetadataStore::default();
    let asked = |ps: &PlaybackSession| ps.request.as_ref().map(|r| r.version);
    let play = |ps: &mut PlaybackSession, meta: &mut plx_data::stores::metadata::MetadataStore, rk: &str, version| {
        let started = request_play(ps, meta, plx_os::devcaps::caps(), sid, rk, "/library/parts/1/file.mkv", "h264", "aac", "T", "", version, "");
        let seen = asked(ps);
        cancel_play(ps);
        assert!(wait_resolve_workers_for_test(std::time::Duration::from_secs(15)), "the resolve worker ended");
        assert!(started, "the request was admitted");
        seen
    };
    assert!(choose_version(sid, "10", MediaId(3086)));
    assert_eq!(play(&mut ps, &mut meta, "10", None), Some(Some(MediaId(3086))), "Resume plays the chosen media");
    assert_eq!(play(&mut ps, &mut meta, "11", None), Some(None), "the next episode takes the server's first file");
    assert_eq!(play(&mut ps, &mut meta, "12", Some(MediaId(3082))), Some(Some(MediaId(3082))), "a named version stands");
    assert_eq!(chosen_version(sid, "12"), Some(MediaId(3082)), "and is remembered for that item");
    assert_eq!(play(&mut ps, &mut meta, "12", None), Some(Some(MediaId(3082))), "the next play of it takes it");
    tidy();
}

/// **Select Version on the failure read-out** (`Recovery::PickVersion`): the retry asks for the picked file, and the
/// pick is remembered once it is playing (either backend), not when it is asked for. A version that fails must not
/// become what Play starts next time.
#[test]
fn a_version_picked_on_the_failure_screen_is_remembered_once_it_plays() {
    use crate::player::{FailureKind, FrozenOffers, LiveOffers, PlaybackFailure, Recovery};
    use std::time::Duration;
    let (_g, _session, sid) = seated();
    let mut ps = PlaybackSession::IDLE;
    let mut meta = plx_data::stores::metadata::MetadataStore::default();
    seed_request_for_test(&mut ps, "10");
    ps.request.as_mut().expect("seeded").sid = sid;
    record_failure(&mut ps, PlaybackFailure::for_test(
        FailureKind::MediaSource, DirectPlayMode::Auto, FrozenOffers { can_retry: true, quality_carriable: true }));
    let live = LiveOffers { repair_idle: true, tracks_available: false, versions_available: true };

    // The door answers the pick as an override: the retry's request names that file.
    let overrides = begin_recovery(&mut ps, live, None, &Recovery::PickVersion(MediaId(3086))).expect("two files: the row offers it");
    assert_eq!(overrides.version, Some(MediaId(3086)));
    assert!(retry_current_play(&mut ps, &mut meta, 0, overrides), "the pick starts a new attempt");
    assert_eq!(attempt_version(&ps), Some(MediaId(3086)), "the attempt asks for the picked file");
    assert_eq!(chosen_version(sid, "10"), None, "asking for a version remembers nothing");

    // Playing remembers it, once.
    remember_version_that_played(&mut ps);
    assert_eq!(chosen_version(sid, "10"), Some(MediaId(3086)), "it played: that is the file Play starts next");
    assert!(choose_version(sid, "10", MediaId(3082)));
    assert!(!version_on_trial_pending(&ps));
    remember_version_that_played(&mut ps);
    assert_eq!(chosen_version(sid, "10"), Some(MediaId(3082)), "the trial is spent: later frames write nothing");
    // The retry's resolve is withdrawn before its worker lands, the way the request test above does it, so no stop for
    // it can reach a later test's server.
    cancel_play(&mut ps);
    assert!(wait_resolve_workers_for_test(Duration::from_secs(10)));
    tidy();
}

/// A picked version whose attempt never reaches a frame is not remembered: the next request replaced it (or a failure
/// ended the attempt), so a frame of ANOTHER file finds the trial and drops it.
#[test]
fn a_picked_version_that_never_plays_is_not_remembered() {
    use crate::player::{FailureKind, FrozenOffers, LiveOffers, PlaybackFailure, Recovery};
    use std::time::Duration;
    let (_g, _session, sid) = seated();
    let mut ps = PlaybackSession::IDLE;
    let mut meta = plx_data::stores::metadata::MetadataStore::default();
    seed_request_for_test(&mut ps, "10");
    ps.request.as_mut().expect("seeded").sid = sid;
    record_failure(&mut ps, PlaybackFailure::for_test(
        FailureKind::MediaSource, DirectPlayMode::Auto, FrozenOffers { can_retry: true, quality_carriable: true }));
    let live = LiveOffers { repair_idle: true, tracks_available: false, versions_available: true };
    let overrides = begin_recovery(&mut ps, live, None, &Recovery::PickVersion(MediaId(3086))).expect("offered");
    assert!(retry_current_play(&mut ps, &mut meta, 0, overrides));
    // The request in force is now another file of the item (the viewer picked again, or Try again named none).
    ps.request.as_mut().expect("in force").version = Some(MediaId(3082));
    remember_version_that_played(&mut ps);
    assert_eq!(chosen_version(sid, "10"), None, "the file that was asked for never played");
    cancel_play(&mut ps);
    assert!(wait_resolve_workers_for_test(Duration::from_secs(10)));
    tidy();
}

/// **A refusal repeats, so picking the file that just failed is refused at the door** (`Refused::SameVersion`): the row
/// withholds *Try again* there, and *Select Version* of the same file would be that retry. Another file is accepted, and a
/// transient failure keeps the same file selectable (it is *Try again* there). "The same file" is the one the attempt
/// LANDED on (`attempt_media`), not the one the request named: with no version named the server's default played.
#[test]
fn a_refusal_does_not_take_the_same_version_again_but_a_stall_does() {
    use crate::player::{FailureKind, FrozenOffers, LiveOffers, PlaybackFailure, Recovery, Refused};
    let mut ps = PlaybackSession::IDLE;
    let _g = fresh_registry(&mut ps);
    seed_request_for_test(&mut ps, "10");
    ps.cur_rk = "10".into();
    let versions = [MediaVersion { id: MediaId(3082), ..Default::default() }, MediaVersion { id: MediaId(3086), ..Default::default() }];
    let live = LiveOffers { repair_idle: true, tracks_available: false, versions_available: true };
    let offers = FrozenOffers { can_retry: true, quality_carriable: true };

    // No version named: the plan took the default (position 0), and that is the file a refusal repeats.
    assert_eq!(attempt_version(&ps), None, "the request names no file");
    assert_eq!(attempt_media(&ps, &versions), Some(MediaId(3082)), "the default file is the one that landed");
    let landed = attempt_media(&ps, &versions);
    record_failure(&mut ps, PlaybackFailure::for_test(FailureKind::DecisionRefused, DirectPlayMode::Auto, offers));
    assert!(matches!(begin_recovery(&mut ps, live, landed, &Recovery::PickVersion(MediaId(3082))), Err(Refused::SameVersion)),
        "the server-default file the refused request landed on repeats it");
    assert!(begin_recovery(&mut ps, live, landed, &Recovery::PickVersion(MediaId(3086))).is_ok(), "another file is a different request");
    record_failure(&mut ps, PlaybackFailure::for_test(FailureKind::MediaSource, DirectPlayMode::Auto, offers));
    assert!(begin_recovery(&mut ps, live, landed, &Recovery::PickVersion(MediaId(3082))).is_ok(), "a stall may be tried again, on the same file too");

    // A named version that landed at its own position is the same file.
    ps.request.as_mut().expect("seeded").version = Some(MediaId(3086));
    ps.cur_media_index = 1;
    assert_eq!(attempt_media(&ps, &versions), Some(MediaId(3086)));
    // Nothing landed for this request's item (another item's plan, or a position the list does not hold): no file is "the same".
    ps.cur_media_index = 5;
    assert_eq!(attempt_media(&ps, &versions), None, "a position the list does not hold");
    ps.cur_media_index = 1;
    ps.cur_rk = "11".into();
    assert_eq!(attempt_media(&ps, &versions), None, "the plan is another item's");
}
