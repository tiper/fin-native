//! **In what order a profile's libraries are shown — the rules, pure.**
//!
//! [`Preferences::library_order`](super::session::Preferences::library_order) is the record (on disk,
//! keyed by profile, naming libraries by `(machineIdentifier, section key)`); `browse`'s section table
//! is the live set of libraries. This module is the policy between them and holds neither, the way
//! [`pins`](super::pins) does for Favorite libraries, so each rule below is graded by `cargo test
//! --lib` rather than observed on a television.
//!
//! * **The default is the server order**: the caller hands the libraries down already in it (own
//!   servers first, then shares, each server's Directory order). An absent record is that order.
//! * **A recorded order is a projection, never a mutation.** [`effective`] answers a permutation of
//!   the libraries it was handed; no table index moves, so every index that names a library
//!   (`cur`, a saved view, a listing) stays valid.
//! * **A library the record does not name** (one created since, a share that arrived later, a
//!   library whose server has not told us its machine id) falls at its default position relative to
//!   the recorded ones: right after the library that precedes it in the default order.
//! * **A recorded library that is not here** (its server is absent for now, or it was deleted) costs
//!   nothing in what is shown, and what the writer does with its record is [`merged`]'s rule: the
//!   grain is the MACHINE, as in [`pins::carry_forward`](super::pins::carry_forward). A server that is
//!   present has just been answered about in full; one that is not keeps its entries, each right
//!   after the entry that preceded it, so it returns to its place with its server.
//! * **Two questions, each asked once.** *Does the record carry anything* is [`merged`]'s, over every
//!   library present: the writer deletes a record that says nothing and stores one that does.
//!   *Does it move anything the profile is shown* is [`OrderState`]'s `in_effect`, judged by the
//!   caller over the favourites every surface presents (Browse compares the favourites' effective
//!   and default order once: the header, Home and Settings all read that). A record can therefore be
//!   stored and in effect over no favourite, say once the one library it moved is unfavourited.
//!
//! [`hub_library`] is the other half of Home's use of this: which library a `/hubs` shelf belongs to,
//! from its `hubIdentifier`, else its key's `sectionID`, so the answer does not depend on whether the shelf still holds
//! its cards.
use super::session::LibraryOrder;

/// One library as these rules see it: the join key a record names it by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lib<'a> {
    /// the server's `machineIdentifier`; empty while nobody has learned it, which is a library a
    /// record can neither name nor be matched against
    pub machine_id: &'a str,
    pub key: i64,
}

/// **What a profile's Library order is now**, the one state every surface reads: the Library header,
/// Home and Settings' Reset row.
///
/// "A record exists" and "it moves a favourite" are kept apart, and so is "it exists" from "it is
/// saved": a record is *recorded* when the profile sees one (an edit pending or failed to save counts)
/// OR the session still holds one (a Reset that is pending or failed counts), so Reset stays offered
/// until the disk really has none. Whether a write reached the disk is the write's own state
/// (`browse::WriteState`), published beside this.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OrderState {
    /// No record anywhere: the server order. Reset has nothing to delete.
    #[default]
    Server,
    /// A record exists, shown or stored. `in_effect` is whether the profile is shown any favourite
    /// moved: false for a record that names only an offline server's libraries, one in the server
    /// order, or one the profile has just Reset (the disk's copy not yet deleted).
    Recorded { in_effect: bool },
}

impl OrderState {
    /// The state of `recorded` (a record is shown or stored) whose effect is `in_effect`; an effect
    /// with no record is no effect.
    pub fn of(recorded: bool, in_effect: bool) -> Self {
        if recorded { Self::Recorded { in_effect } } else { Self::Server }
    }
    /// Is a record there to delete (Settings' Reset)?
    pub fn is_recorded(self) -> bool { matches!(self, Self::Recorded { .. }) }
    /// Does the record move any favourite the profile is shown (what Home and the header project)?
    pub fn in_effect(self) -> bool { matches!(self, Self::Recorded { in_effect: true }) }
}

/// The effective order of `default` (libraries in the server's own order) under `record`: a
/// permutation, each entry an index into `default`, first shown first.
pub fn effective(default: &[Lib<'_>], record: Option<&LibraryOrder>) -> Vec<usize> {
    place(default, record.map(|record| record.libs.iter().map(|lib| (lib.machine_id.as_str(), lib.key))).into_iter().flatten())
}

fn place<'a>(default: &[Lib<'_>], named: impl Iterator<Item = (&'a str, i64)>) -> Vec<usize> {
    let mut placed: Vec<usize> = Vec::with_capacity(default.len());
    let mut is_placed = vec![false; default.len()];
    let sequence: Vec<usize> = (0..default.len()).collect();
    for (machine_id, key) in named.filter(|(machine_id, _)| !machine_id.is_empty()) {
        let found = default.iter().position(|lib| lib.machine_id == machine_id && lib.key == key);
        if let Some(index) = found.filter(|index| !is_placed[*index]) {
            is_placed[index] = true;
            placed.push(index);
        }
    }
    // In default order, so a run of unnamed libraries chains behind the nearest named one: each
    // one's predecessor (`index - 1`) is already placed when it is reached.
    for index in 0..default.len() {
        if !is_placed[index] {
            insert_behind_predecessor(&mut placed, &sequence, index, index);
            is_placed[index] = true;
        }
    }
    placed
}

/// **The one insertion both orderings use**: puts `item`, which stands at `sequence[at]`, into `list`
/// right after the nearest entry before it in `sequence` that `list` already holds, and at the front
/// when it holds none. [`place`] files an unnamed library behind its default-order predecessor (always
/// held by then, so the nearest is the immediate one) and [`merged`] files a carried entry behind
/// whichever of its recorded predecessors survived.
fn insert_behind_predecessor<T: PartialEq>(list: &mut Vec<T>, sequence: &[T], at: usize, item: T) {
    let after = sequence[..at].iter().rev().find_map(|before| list.iter().position(|held| held == before));
    list.insert(after.map_or(0, |after| after + 1), item);
}

/// **What the writer stores** when the profile chose `arrangement` (the favourites as the page showed
/// them, `(machine id, key)` first shown first) over `previous` (the record the write meets) while
/// `present` (every library the table holds, in the server's own order) is what it can see.
///
/// `None` is "no record": the merged record says nothing, so it is deleted. Otherwise the new list:
///
/// * the arrangement, minus libraries no record can name (no machine id) and duplicates;
/// * plus every entry of `previous` whose MACHINE is not in `present` — silence is not an answer —
///   each placed right after the entry that preceded it in `previous` (and at the front when none
///   did), so an offline server's libraries return to where they were.
///
/// The decision is on that MERGED list: a record that still carries an absent server's entries says
/// something even when the present libraries are in the server order, so only a list with nothing
/// carried and no effect among `present` is `None`.
pub fn merged(previous: Option<&LibraryOrder>, arrangement: &[(String, i64)], present: &[(String, i64)])
    -> Option<Vec<(String, i64)>> {
    let mut list: Vec<(String, i64)> = Vec::with_capacity(arrangement.len());
    for (machine_id, key) in arrangement.iter().filter(|(machine_id, _)| !machine_id.is_empty()) {
        if !list.iter().any(|(m, k)| m == machine_id && k == key) { list.push((machine_id.clone(), *key)); }
    }
    let fresh = list.len();
    if let Some(previous) = previous {
        let sequence: Vec<(String, i64)> = previous.libs.iter().map(|lib| (lib.machine_id.clone(), lib.key)).collect();
        for (at, entry) in sequence.iter().enumerate()
            .filter(|(_, (machine_id, _))| super::pins::is_absent_machine(machine_id, present.iter().map(|(m, _)| m.as_str()))) {
            if !list.contains(entry) { insert_behind_predecessor(&mut list, &sequence, at, entry.clone()); }
        }
    }
    let carried = list.len() > fresh;
    let default: Vec<Lib<'_>> = present.iter().map(|(machine_id, key)| Lib { machine_id, key: *key }).collect();
    let moves = place(&default, list.iter().map(|(machine_id, key)| (machine_id.as_str(), *key)))
        .iter().enumerate().any(|(at, index)| at != *index);
    (carried || moves).then_some(list)
}

/// Which library a `/hubs` shelf belongs to, from what it NAMES, never from the cards it holds: its
/// `hubIdentifier` first: the per-library Recently Added family (`movie.recentlyadded.<id>`,
/// `show.recentlyadded.<id>`, `tv.recentlyadded.<id>`) and a promoted collection
/// (`custom.collection.<section>.…`); then, for a hub that is not [`is_whole_server_hub`], the
/// `sectionID` of its `key`: PMS mints one `home.<type>.recent` hub per library when a household
/// owns two of a type, and each one's key says which (`/hubs/home/recentlyAdded?type=2&sectionID=1`).
/// `None` for every other shelf, which the cards of its first page decide, once, at projection
/// (`data::pms::ShelfLibrary`).
pub fn hub_library(hub_identifier: &str, key: &str) -> Option<i64> {
    for prefix in super::hub_title::PER_LIBRARY_PREFIXES {
        if let Some(rest) = hub_identifier.strip_prefix(prefix) {
            return rest.parse().ok();
        }
    }
    if super::collections::is_promoted_collection_hub(hub_identifier) { return super::collections::promoted_collection_section(hub_identifier); }
    if is_whole_server_hub(hub_identifier) { return None; }
    super::paging::query_value(key, "sectionID")?.parse().ok()
}

/// The `home.*` hubs that span the whole server whatever they hold: Continue Watching, On Deck (both
/// spellings PMS has used) and Recent Playlists. **The only `home.*` identifiers that belong to no
/// library by their id.** `home.<type>.recent` is NOT one: PMS mints one such hub per library when a
/// household owns two of a type (`docs/pms-api.md` §3, `hub_title::is_recently_added_hub`), and a
/// lone one is that library's own shelf, so those are decided by their key's `sectionID` ([`hub_library`]), else by the library their first page's cards share.
pub fn is_whole_server_hub(hub_identifier: &str) -> bool {
    matches!(hub_identifier, "home.continue" | "home.ondeck" | "home.onDeck" | "home.playlists")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plex::session::OrderedLib;

    fn libs() -> Vec<Lib<'static>> {
        // own server A (keys 4, 1, 5 in the server's own order), then a friend's B (keys 2, 9)
        vec![
            Lib { machine_id: "A", key: 4 }, Lib { machine_id: "A", key: 1 }, Lib { machine_id: "A", key: 5 },
            Lib { machine_id: "B", key: 2 }, Lib { machine_id: "B", key: 9 },
        ]
    }
    fn record(named: &[(&str, i64)]) -> LibraryOrder {
        LibraryOrder {
            user: String::new(),
            libs: named.iter().map(|(machine_id, key)| OrderedLib {
                machine_id: (*machine_id).into(), key: *key, extensions: Default::default() }).collect(),
            extensions: Default::default(),
        }
    }

    #[test]
    fn no_record_is_the_server_order() {
        let order = effective(&libs(), None);
        assert_eq!(order, vec![0, 1, 2, 3, 4]);
                assert_eq!(effective(&libs(), Some(&record(&[]))), order);
    }

    #[test]
    fn a_recorded_order_ranks_across_servers() {
        // a friend's share sits between the owner's own libraries
        let order = effective(&libs(), Some(&record(&[("A", 1), ("B", 2), ("A", 4), ("A", 5), ("B", 9)])));
        assert_eq!(order, vec![1, 3, 0, 2, 4]);
    }

    #[test]
    fn a_library_the_record_does_not_name_falls_after_its_default_predecessor() {
        // A/5 and B/9 are new: A/5's predecessor is A/1 (which the record moved to the front), and
        // B/9's is B/2
        let order = effective(&libs(), Some(&record(&[("A", 1), ("B", 2), ("A", 4)])));
        assert_eq!(order, vec![1, 2, 3, 4, 0]);
    }

    #[test]
    fn an_unnamed_library_with_no_predecessor_leads() {
        let order = effective(&libs(), Some(&record(&[("B", 9), ("B", 2)])));
        // A/4 has no predecessor; A/1 and A/5 chain behind it; the named two keep their order
        assert_eq!(order, vec![0, 1, 2, 4, 3]);
    }

    #[test]
    fn a_recorded_library_that_is_absent_costs_nothing() {
        let present = &libs()[..3];
        let order = effective(present, Some(&record(&[("B", 2), ("A", 5), ("A", 4)])));
        assert_eq!(order, vec![2, 0, 1], "A/1 is unnamed: it follows A/4 (its predecessor)");
    }

    #[test]
    fn a_library_with_no_machine_id_is_never_matched_by_the_record() {
        let anonymous = [Lib { machine_id: "", key: 1 }, Lib { machine_id: "", key: 2 }];
        let order = effective(&anonymous, Some(&record(&[("", 2), ("", 1)])));
        assert_eq!(order, vec![0, 1], "the record cannot name them, so the default stands");
    }

    #[test]
    fn a_duplicate_entry_places_its_library_once() {
        let order = effective(&libs(), Some(&record(&[("A", 5), ("A", 5), ("A", 4)])));
        assert_eq!(order.len(), 5);
        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, vec![0, 1, 2, 3, 4], "a permutation");
    }

    /// A record is *recorded* whether or not it moves anything shown: Reset has something to delete.
    #[test]
    fn a_recorded_order_that_moves_nothing_shown_is_recorded_not_in_effect() {
        assert_eq!(OrderState::of(false, false), OrderState::Server);
        assert_eq!(OrderState::of(false, true), OrderState::Server, "an effect with no record is none");
        assert_eq!(OrderState::of(true, false), OrderState::Recorded { in_effect: false });
        assert!(OrderState::of(true, false).is_recorded() && !OrderState::of(true, false).in_effect());
        assert!(OrderState::of(true, true).in_effect());
    }

    fn pairs(list: &[(&str, i64)]) -> Vec<(String, i64)> {
        list.iter().map(|(machine, key)| ((*machine).to_owned(), *key)).collect()
    }
    fn present() -> Vec<(String, i64)> { pairs(&[("A", 4), ("A", 1), ("A", 5), ("B", 2), ("B", 9)]) }

    #[test]
    fn the_writer_deletes_a_record_that_says_nothing() {
        assert_eq!(merged(None, &present(), &present()), None, "the server order, nothing recorded before");
        let previous = record(&[("B", 9), ("B", 2)]);
        assert_eq!(merged(Some(&previous), &present(), &present()), None, "B is present: its old answer is replaced");
        assert_eq!(merged(None, &pairs(&[("", 1)]), &present()), None, "a library no record can name is not recorded");
    }

    #[test]
    fn the_writer_keeps_an_offline_servers_entries_in_their_places() {
        // B is offline: only A's libraries are present
        let here = pairs(&[("A", 4), ("A", 1), ("A", 5)]);
        let previous = record(&[("A", 5), ("B", 2), ("A", 4), ("B", 9), ("A", 1)]);
        let list = merged(Some(&previous), &pairs(&[("A", 1), ("A", 5), ("A", 4)]), &here).unwrap();
        // B/2 followed A/5 and B/9 followed A/4: each stays behind its predecessor
        assert_eq!(list, pairs(&[("A", 1), ("A", 5), ("B", 2), ("A", 4), ("B", 9)]));
        // choosing the server order of what is present still keeps B's entries: the record says
        // something about B, and deleting it would lose it
        let kept = merged(Some(&previous), &here, &here).unwrap();
        assert_eq!(kept, pairs(&[("A", 4), ("B", 9), ("A", 1), ("A", 5), ("B", 2)]), "each still behind the entry that preceded it");

    }

    #[test]
    fn an_entry_with_no_surviving_predecessor_leads() {
        let here = pairs(&[("A", 4)]);
        let previous = record(&[("B", 2), ("A", 4)]);
        assert_eq!(merged(Some(&previous), &here, &here).unwrap(), pairs(&[("B", 2), ("A", 4)]));
        // its predecessor was a present library the profile unfavoured: it follows the one before that
        let previous = record(&[("A", 4), ("A", 9), ("B", 2)]);
        assert_eq!(merged(Some(&previous), &here, &here).unwrap(), pairs(&[("A", 4), ("B", 2)]));
    }

    /// A carried entry goes behind the NEAREST predecessor that survived, which may be an entry carried
    /// just before it, and a predecessor the profile unfavoured is skipped.
    #[test]
    fn a_carried_entry_follows_its_nearest_surviving_predecessor() {
        let here = pairs(&[("A", 4)]);
        // B/2 and B/9 are offline; A/9 is present but no longer a favourite. B/9's immediate predecessor
        // B/2 was carried first, so it follows that; B/2's A/9 did not survive, so it follows A/4
        let previous = record(&[("A", 4), ("A", 9), ("B", 2), ("B", 9)]);
        assert_eq!(merged(Some(&previous), &here, &here).unwrap(), pairs(&[("A", 4), ("B", 2), ("B", 9)]));
        // and with the arrangement putting A/4 last, the carried pair rides behind it
        let two = pairs(&[("A", 4), ("A", 1)]);
        let previous = record(&[("A", 1), ("B", 2), ("B", 9), ("A", 4)]);
        assert_eq!(merged(Some(&previous), &pairs(&[("A", 4), ("A", 1)]), &two).unwrap(),
            pairs(&[("A", 4), ("A", 1), ("B", 2), ("B", 9)]));
    }

    #[test]
    fn whole_server_hubs_are_the_three_that_span_libraries() {
        for id in ["home.continue", "home.ondeck", "home.onDeck", "home.playlists"] { assert!(is_whole_server_hub(id), "{id}"); }
        for id in ["home.movies.recent", "home.television.recent", "movie.recentlyadded.3", "mock.shelf.0"] {
            assert!(!is_whole_server_hub(id), "{id}");
        }
    }

    #[test]
    fn hub_library_names_only_what_the_identifier_or_key_names() {
        assert_eq!(hub_library("movie.recentlyadded.3", ""), Some(3));
        assert_eq!(hub_library("tv.recentlyadded.12", ""), Some(12));
        assert_eq!(hub_library("show.recentlyadded.7", ""), Some(7));
        assert_eq!(hub_library("custom.collection.2.420.420", ""), Some(2));
        assert_eq!(hub_library("custom.collection.x.420.420", ""), None);
        assert_eq!(hub_library("custom.collection.x.420.420", "/hubs/x?sectionID=3"), None, "a promoted collection is decided by its id alone");
        assert_eq!(hub_library("home.movies.recent", ""), None, "a whole-server hub spans libraries");
        assert_eq!(hub_library("home.ondeck", ""), None);
        // a `home.<type>.recent` hub per library: the library is in its key
        let key = |id: i64| format!("/hubs/home/recentlyAdded?type=2&sectionID={id}");
        assert_eq!(hub_library("home.television.recent", &key(2)), Some(2));
        assert_eq!(hub_library("home.television.recent", "/hubs/home/recentlyAdded?type=2"), None);
        assert_eq!(hub_library("home.television.recent", "/hubs/home/recentlyAdded?type=2&sectionID=x"), None);
        assert_eq!(hub_library("home.ondeck", &key(2)), None, "a whole-server hub is nobody's whatever its key says");
        assert_eq!(hub_library("movie.recentlyadded.3", &key(9)), Some(3), "the id outranks the key");
        // one list of the per-library prefixes: every one `is_recently_added_hub` accepts names its library here
        for prefix in crate::plex::hub_title::PER_LIBRARY_PREFIXES {
            let id = format!("{prefix}5");
            assert!(crate::plex::hub_title::is_recently_added_hub(&id), "{id}");
            assert_eq!(hub_library(&id, ""), Some(5), "{id}");
        }
        assert_eq!(hub_library("movie.recentlyadded.x", ""), None);
    }
}
