//! Per-profile Recent channels, the data layer: the SPEC-1 rule over the memory door (join at the
//! front, a known channel never moves, a sixth drops the oldest), the soft parse, and the row.
use super::recent_channels::*;
use super::{MemoryEntry, Parse, Preferences, ProfileList, ProfileMemory, Scope, DESCRIPTORS};
use crate::plex::session::test_support::STAMP;
use serde_json::json;

fn entry(channel: &str) -> RecentChannelEntry {
    RecentChannelEntry::new("0123456789abcdef0123456789abcdef01234567", "7", channel)
}

/// Note a tune of `channel` for `user` the way the app does: through the entry's own merge rule.
fn note(list: &mut Vec<ProfileList<RecentChannelEntry>>, user: &str, channel: &str) -> bool {
    let e = entry(channel);
    list.upsert_with(user, STAMP, &e.key(), e.noted())
}

/// The profile's channels as the row shows them: newest first (the list is stored oldest first).
fn row_of(list: &[ProfileList<RecentChannelEntry>], user: &str) -> Vec<String> {
    list.iter().find(|r| r.user == user)
        .map(|r| r.items.iter().rev().map(|e| e.channel.clone()).collect()).unwrap_or_default()
}

#[test]
fn a_newly_tuned_channel_joins_at_the_front() {
    let mut list = Vec::new();
    assert!(note(&mut list, "u1", "A"));
    assert!(note(&mut list, "u1", "B"));
    assert!(note(&mut list, "u1", "C"));
    assert_eq!(row_of(&list, "u1"), ["C", "B", "A"]);
}

#[test]
fn a_channel_already_in_the_list_never_moves() {
    let mut list = Vec::new();
    for c in ["A", "B", "C"] { note(&mut list, "u1", c); }
    assert!(!note(&mut list, "u1", "A"), "re-tuning the oldest is no change");
    assert!(!note(&mut list, "u1", "C"), "nor the newest");
    assert!(!note(&mut list, "u1", "B"), "nor one in the middle");
    assert_eq!(row_of(&list, "u1"), ["C", "B", "A"]);
}

#[test]
fn a_sixth_channel_drops_the_oldest_which_is_the_last_of_the_row() {
    assert_eq!(MAX_RECENT, 5);
    let mut list = Vec::new();
    for c in ["A", "B", "C", "D", "E"] { note(&mut list, "u1", c); }
    assert_eq!(row_of(&list, "u1"), ["E", "D", "C", "B", "A"]);
    assert!(note(&mut list, "u1", "F"));
    assert_eq!(row_of(&list, "u1"), ["F", "E", "D", "C", "B"], "A, the last, went");
    assert!(!note(&mut list, "u1", "B"), "a known channel at the end still does not move");
    assert_eq!(row_of(&list, "u1"), ["F", "E", "D", "C", "B"]);
}

#[test]
fn each_profile_keeps_its_own_row() {
    let mut list = Vec::new();
    note(&mut list, "u1", "A");
    note(&mut list, "u2", "B");
    note(&mut list, "u1", "C");
    assert_eq!(row_of(&list, "u1"), ["C", "A"]);
    assert_eq!(row_of(&list, "u2"), ["B"]);
}

/// The same channel on two servers is two entries: the machine id is part of the key.
#[test]
fn the_server_is_part_of_the_identity() {
    let mut list = Vec::new();
    let other = RecentChannelEntry::new("ffffffffffffffffffffffffffffffffffffffff", "7", "A");
    note(&mut list, "u1", "A");
    assert!(list.upsert_with("u1", STAMP, &other.key(), other.noted()));
    assert_eq!(list[0].items.len(), 2);
}

#[test]
fn the_list_row_names_its_own_field_and_is_account_bound() {
    let default_json = serde_json::to_value(Preferences::default()).unwrap();
    assert!(default_json.get("recent_channels").is_none(), "an empty list writes no key: {default_json}");
    let mut p = Preferences::default();
    note(&mut p.recent_channels, "u1", "A");
    let body = serde_json::to_value(&p).unwrap();
    assert!(body["recent_channels"].is_array(), "{body}");
    let back: Preferences = serde_json::from_value(body).unwrap();
    assert_eq!(back, p);
    let row = DESCRIPTORS.iter().find(|d| d.key == "recent_channels").expect("the row is in the table");
    assert_eq!(row.scope, Scope::Profile);
    assert!(row.account_bound && !row.live && row.settings.is_none());
    assert_eq!(row.parse, Parse::SoftVec);
    assert!(plx_os::storage::state::ACCOUNT_BOUND_PREFERENCES.contains(&"recent_channels"),
        "sign-out forgets a household member's channels with the account");
}

/// **Soft parse, round trip.** An unknown field never costs the entry or the list; an entry that
/// cannot be read costs only itself; a value that is not a list is no list; and what is written
/// back is exactly the documented shape.
#[test]
fn an_unknown_field_never_costs_the_list_and_the_stored_form_round_trips() {
    let stored = json!([{ "user": "u1", "account": STAMP, "items": [
        { "machine_id": "m1", "dvr": "7", "channel": "A", "added_by_a_newer_build": { "x": 1 } },
        { "machine_id": "m1", "dvr": "7" },
        { "machine_id": "m1", "dvr": 7, "channel": "B" },
        { "machine_id": "m1", "dvr": "7", "channel": "x".repeat(ID_MAX + 1) },
        "junk",
        { "machine_id": "m1", "dvr": "7", "channel": "C" },
    ], "extra_record_field": true }]);
    let p: Preferences = serde_json::from_value(json!({ "recent_channels": stored })).unwrap();
    let kept: Vec<_> = p.recent_channels[0].items.iter().map(|e| e.channel.as_str()).collect();
    assert_eq!(kept, ["A", "C"], "the unreadable entries went, the readable ones stayed, in order");
    let again = serde_json::to_value(&p).unwrap();
    assert_eq!(again["recent_channels"][0]["items"],
        json!([{ "machine_id": "m1", "dvr": "7", "channel": "A" }, { "machine_id": "m1", "dvr": "7", "channel": "C" }]),
        "an unknown field inside an entry is not written back");
    let back: Preferences = serde_json::from_value(again).unwrap();
    assert_eq!(back, p, "the stored form reads back as itself");
    for junk in [json!(null), json!("zzz"), json!(17), json!({ "unexpected": 1 })] {
        let p: Preferences = serde_json::from_value(json!({ "recent_channels": junk })).unwrap();
        assert!(p.recent_channels.is_empty());
    }
}

#[test]
fn an_over_long_or_blank_id_is_not_an_entry() {
    assert!(RecentChannelEntry::new("", "7", "A").is_blank());
    assert!(RecentChannelEntry::new("m", "7", &"x".repeat(ID_MAX + 1)).is_blank());
    assert!(!RecentChannelEntry::new("m", "7", &"x".repeat(ID_MAX)).is_blank());
    let mut list = Vec::new();
    let blank = RecentChannelEntry::default();
    assert!(!list.upsert_with("u1", STAMP, &blank.key(), blank.clone().noted()), "a blank entry records nothing");
    assert!(list.is_empty());
}
