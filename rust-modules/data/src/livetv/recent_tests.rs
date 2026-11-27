//! Recent channels in the Live TV store: the row as a pure function of what the memory door holds,
//! the roster and the lineup; the one cap; the session-only list of a run nobody is seated in, and
//! what a seat arriving or leaving does to it.
use super::*;
use crate::stores::Stores;
use plx_plex::plex::session::prefs::RecentChannelEntry;
use plx_plex::plex::ClientFacts;

fn sid(n: u16) -> ServerId { ServerId::from_raw(n) }

fn channel(server: u16, number: &str) -> Channel {
    Channel { sid: sid(server), dvr: "7".into(), channel: format!("C{number}"), vcn: number.into(),
        name: format!("Channel {number}"), thumb: String::new(), now: String::new(), now_thumb: String::new(),
        now_summary: String::new(), begins_at: 0, ends_at: 0, later: Vec::new() }
}
fn key(server: u16, number: &str) -> RecentKey {
    RecentKey { sid: sid(server), dvr: "7".into(), channel: format!("C{number}") }
}
fn entry(machine: &str, number: &str) -> RecentChannelEntry {
    RecentChannelEntry::new(machine, "7", &format!("C{number}"))
}
fn lineup(server: u16, numbers: &[&str]) -> Vec<Channel> { numbers.iter().map(|n| channel(server, n)).collect() }

/// A registry as the Bridge captures it: slot `n` holds the server whose machine id is `machines[n]`.
fn roster(roster_gen: u32, machines: &[(u16, &str)]) -> ClientKeys {
    ClientKeys::captured(ServerId::UNSET, roster_gen, 0, machines.iter().map(|(n, machine)| {
        (sid(*n), Some(ClientKey { instance: u32::from(*n) + 1, token: 1, grant: 1 }),
            ClientFacts { machine_id: (*machine).into(), described: None, source: None, stream_allowed: false })
    }))
}

fn vcns(channels: &[Channel]) -> Vec<&str> { channels.iter().map(|c| c.vcn.as_str()).collect() }
fn row(stores: &Stores) -> Vec<String> { stores.livetv_view().recent.iter().map(|c| c.vcn.clone()).collect() }

fn unseated<'a>(keys: &'a ClientKeys) -> LiveTvInput<'a> {
    LiveTvInput { keys, now: Some(100), recent: RecentSeat::Unseated }
}
fn seated<'a>(keys: &'a ClientKeys, stored: &[RecentChannelEntry]) -> LiveTvInput<'a> {
    LiveTvInput { keys, now: Some(100), recent: RecentSeat::Seated(stored.to_vec()) }
}
/// One step under `input`, then `lineup` as the store's published lineup (the roster the step
/// registered owns no guide of its own here), then one more step: the row the input resolves to.
fn show(stores: &mut Stores, input: &LiveTvInput<'_>, lineup: Vec<Channel>) -> Vec<String> {
    stores.livetv_pump(input);
    stores.livetv.install_for_test(lineup);
    stores.livetv_pump(input);
    row(stores)
}
fn with_lineup(numbers: &[&str]) -> Stores {
    let mut stores = Stores::default();
    stores.livetv.install_for_test(lineup(0, numbers));
    stores
}

#[test]
fn an_empty_recent_is_published_as_an_empty_list() {
    let stores = with_lineup(&["1", "2", "3"]);
    assert!(stores.livetv_view().recent.is_empty(), "PR3 hides the row on an empty list");
    assert!(LiveTvView::LOADING.recent.is_empty());
}

/// **The row is the door's list, resolved**: newest first (the door keeps oldest first), each entry
/// the lineup's own channel, five at most, and nothing else decides it.
#[test]
fn the_seated_row_is_the_stored_list_newest_first() {
    let keys = roster(1, &[(0, "machine-a")]);
    let stored: Vec<_> = ["1", "2", "3"].iter().map(|n| entry("machine-a", n)).collect();
    let lineup = lineup(0, &["1", "2", "3", "4"]);
    assert_eq!(vcns(&resolve(&stored, &keys, &lineup)), ["3", "2", "1"]);
    let mut stores = Stores::default();
    assert_eq!(show(&mut stores, &seated(&keys, &stored), lineup.clone()), ["3", "2", "1"]);
    assert!(!stores.livetv_pump(&seated(&keys, &stored)), "the same input twice is no change");
}

#[test]
fn at_most_five_are_shown_whatever_a_list_holds() {
    let keys = roster(1, &[(0, "machine-a")]);
    let stored: Vec<_> = ["1", "2", "3", "4", "5", "6", "7"].iter().map(|n| entry("machine-a", n)).collect();
    let row = resolve(&stored, &keys, &lineup(0, &["1", "2", "3", "4", "5", "6", "7"]));
    assert_eq!(vcns(&row), ["7", "6", "5", "4", "3"]);
    assert_eq!(MAX_RECENT, 5);
}

/// An entry whose server is not registered, or whose channel the lineup does not list, is skipped
/// when the row is read and stays in the list: it comes back when the roster or the lineup has it.
#[test]
fn a_hidden_entry_is_skipped_and_comes_back_when_it_can() {
    let stored = vec![entry("machine-a", "1"), entry("machine-gone", "2"), entry("machine-a", "9"), entry("machine-a", "3")];
    let mut stores = Stores::default();
    let before = roster(1, &[(0, "machine-a")]);
    assert_eq!(show(&mut stores, &seated(&before, &stored), lineup(0, &["1", "2", "3"])), ["3", "1"],
        "machine-gone is not registered and channel 9 is not in the lineup");
    let mut gone_back = lineup(0, &["1", "2", "3", "9"]);
    gone_back.push(channel(4, "2"));
    let after = roster(2, &[(0, "machine-a"), (4, "machine-gone")]);
    assert_eq!(show(&mut stores, &seated(&after, &stored), gone_back), ["3", "9", "2", "1"],
        "both are shown again, each in the place it was stored");
}

/// **The failing scenario of one list under one cap.** Stored (oldest first) `[A1, A2, A3, B1, B2]`
/// with server B not registered reads `[A3, A2, A1]`. Tuning A4 is a write to the door, whose cap of
/// five counts the hidden entries too, so it holds `[A2, A3, B1, B2, A4]`; the row the store shows
/// once it is handed that list is the row any later read of it shows.
#[test]
fn a_hidden_server_counts_toward_the_one_cap() {
    let keys = roster(1, &[(0, "machine-a")]);
    let mut lineup = lineup(0, &["1", "2", "3", "4"]);
    lineup.push(channel(9, "1"));
    let before: Vec<_> = vec![entry("machine-a", "1"), entry("machine-a", "2"), entry("machine-a", "3"),
        entry("machine-b", "1"), entry("machine-b", "2")];
    let mut stores = Stores::default();
    assert_eq!(show(&mut stores, &seated(&keys, &before), lineup.clone()), ["3", "2", "1"]);
    let after = vec![entry("machine-a", "2"), entry("machine-a", "3"), entry("machine-b", "1"),
        entry("machine-b", "2"), entry("machine-a", "4")];
    stores.livetv_pump(&seated(&keys, &after));
    assert_eq!(row(&stores), ["4", "3", "2"], "A1 went at the tune");
    assert_eq!(vcns(&resolve(&after, &keys, &lineup)), ["4", "3", "2"], "and a fresh read of the list says the same");
}

/// The store keeps no list of its own for a seated profile: a tune it is told of changes nothing, and
/// what is shown is exactly what the input says.
#[test]
fn a_seated_store_notes_nothing_and_shows_what_the_input_says() {
    let keys = roster(1, &[(0, "machine-a")]);
    let stored = vec![entry("machine-a", "1")];
    let mut stores = Stores::default();
    show(&mut stores, &seated(&keys, &stored), lineup(0, &["1", "2"]));
    assert!(!stores.livetv_note_unseated(key(0, "2"), &seated(&keys, &stored)), "a seated tune is the door's to keep");
    assert_eq!(row(&stores), ["1"]);
    stores.livetv_pump(&seated(&keys, &[]));
    assert!(row(&stores).is_empty(), "the door emptied (a sign-out, a cleared list): so is the row");
}

/// **The stored list survives a different sid assignment.** What is stored names the server by its
/// machine identifier; a registry that numbered the servers the other way round names the same channels.
#[test]
fn the_stored_list_survives_a_different_sid_assignment() {
    let stored = vec![entry("machine-a", "1"), entry("machine-b", "2")];
    for (a, b) in [(0u16, 1u16), (1, 0), (4, 2)] {
        let keys = roster(1, &[(a, "machine-a"), (b, "machine-b")]);
        let lineup = vec![channel(a, "1"), channel(b, "2"), channel(a, "2"), channel(b, "1")];
        let row = resolve(&stored, &keys, &lineup);
        assert_eq!(row.iter().map(|c| (c.sid, c.vcn.as_str())).collect::<Vec<_>>(), [(sid(b), "2"), (sid(a), "1")]);
    }
}

#[test]
fn a_tuned_channel_has_a_durable_entry_only_while_its_server_is_registered() {
    let keys = roster(1, &[(0, "machine-a"), (2, "")]);
    assert_eq!(entry_of(sid(0), "7", "C1", &keys), Some(entry("machine-a", "1")));
    assert_eq!(entry_of(sid(3), "7", "C1", &keys), None, "a slot the roster does not hold");
    assert_eq!(entry_of(sid(2), "7", "C1", &keys), None, "a slot with no machine id is no server");
}

/// **No seat, no storage**: the run's tunes are the store's, session-only, by the same rule and cap.
#[test]
fn unseated_tunes_join_at_the_front_never_move_and_stop_at_five() {
    let keys = roster(1, &[]);
    let mut stores = with_lineup(&["1", "2", "3", "4", "5", "6", "7"]);
    for n in ["1", "2", "3"] { assert!(stores.livetv_note_unseated(key(0, n), &unseated(&keys))); }
    assert_eq!(row(&stores), ["3", "2", "1"]);
    let revision = stores.livetv_view().revision();
    for n in ["1", "3", "2"] { assert!(!stores.livetv_note_unseated(key(0, n), &unseated(&keys)), "re-tuning {n} is no change"); }
    assert_eq!(row(&stores), ["3", "2", "1"]);
    assert_eq!(stores.livetv_view().revision(), revision, "no publication for a no-op");
    for n in ["4", "5", "6"] { stores.livetv_note_unseated(key(0, n), &unseated(&keys)); }
    assert_eq!(row(&stores), ["6", "5", "4", "3", "2"], "1, the last, went");
}

/// A session tune of a channel the lineup does not list is skipped and kept, like a stored one.
#[test]
fn an_unseated_tune_the_lineup_does_not_list_waits_for_it() {
    let keys = roster(1, &[]);
    let mut stores = with_lineup(&["1", "2"]);
    stores.livetv_note_unseated(key(0, "1"), &unseated(&keys));
    stores.livetv_note_unseated(key(0, "9"), &unseated(&keys));
    stores.livetv_note_unseated(key(0, "2"), &unseated(&keys));
    assert_eq!(row(&stores), ["2", "1"], "9 is not in the lineup");
    stores.livetv.install_for_test(lineup(0, &["1", "2", "9"]));
    assert_eq!(row(&stores), ["2", "9", "1"]);
}

/// **A seat arriving replaces the session's tunes** (they were nobody's), and a seat leaving starts
/// the next session empty (the profile's tunes are the profile's, and are in the door).
#[test]
fn a_seat_arriving_replaces_the_session_and_leaving_starts_it_empty() {
    let keys = roster(1, &[(0, "machine-a")]);
    let mut stores = Stores::default();
    show(&mut stores, &unseated(&keys), lineup(0, &["1", "2", "3"]));
    stores.livetv_note_unseated(key(0, "1"), &unseated(&keys));
    stores.livetv_note_unseated(key(0, "2"), &unseated(&keys));
    assert_eq!(row(&stores), ["2", "1"]);

    let stored = vec![entry("machine-a", "3")];
    assert_eq!(show(&mut stores, &seated(&keys, &stored), lineup(0, &["1", "2", "3"])), ["3"],
        "the seat's own list, and none of the session's");

    assert!(stores.livetv_pump(&unseated(&keys)));
    assert!(row(&stores).is_empty(), "the profile's row does not stay on the screen of nobody");
    stores.livetv_note_unseated(key(0, "2"), &unseated(&keys));
    assert_eq!(row(&stores), ["2"], "a new session, a new list");
}

/// A roster move with nobody seated keeps the session's row (it is a list of slots, not of servers).
#[test]
fn an_unseated_roster_move_keeps_the_session_row() {
    let mut stores = with_lineup(&["1", "2"]);
    stores.livetv_note_unseated(key(0, "1"), &unseated(&roster(1, &[])));
    let moved = roster(2, &[(0, "machine-a")]);
    assert_eq!(show(&mut stores, &unseated(&moved), lineup(0, &["1", "2"])), ["1"]);
}

/// A server re-registration (`Reset`) is not a profile switch: the mode, and so the row, survives it.
#[test]
fn a_reset_keeps_recent() {
    let keys = roster(1, &[(0, "machine-a")]);
    let stored = vec![entry("machine-a", "1")];
    let mut stores = Stores::default();
    assert_eq!(show(&mut stores, &seated(&keys, &stored), lineup(0, &["1"])), ["1"]);
    stores.livetv_run(LiveTvCmd::Reset, &seated(&keys, &stored));
    assert!(row(&stores).is_empty(), "the lineup was dropped, and a row is made of the lineup");
    assert_eq!(show(&mut stores, &seated(&keys, &stored), lineup(0, &["1"])), ["1"], "the mode survived it");
}

/// **No clock is not the epoch.** A frame with no wall time asks the guide nothing (a due refresh
/// cannot be judged), and still resolves the row from what the store already holds.
#[test]
fn a_frame_with_no_time_asks_nothing_and_still_resolves_the_row() {
    let keys = roster(1, &[(0, "machine-a")]);
    let stored = vec![entry("machine-a", "1")];
    let mut stores = Stores::default();
    let dark = LiveTvInput { keys: &keys, now: None, recent: RecentSeat::Seated(stored.clone()) };
    assert_eq!(show(&mut stores, &dark, lineup(0, &["1"])), ["1"]);
    assert!(!stores.has_jobs(), "no time, no request");
    let lit = LiveTvInput { now: Some(100), ..dark.clone() };
    stores.livetv_pump(&lit);
    assert!(stores.has_jobs(), "the next frame with a time asks");
}
