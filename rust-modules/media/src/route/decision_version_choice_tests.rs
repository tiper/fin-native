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
