//! **Library order on Home** (Settings, LO-2): with a record the merge stable-sorts every source's
//! shelves, across sources, by their library's rank; with none, Home is exactly what it was. The
//! ring (`published_order`), the hero pool and a released row all have to agree with that.

use super::*;
use super::test_support::*;
use crate::captured_store_fixture::{key, roster};

/// Own server: a library-less whole-server hub, then library 1's and library 2's Recently Added, then
/// an unrecognised hub whose cards all sit in library 2. The friend's: library 7's Recently Added.
fn sources() -> [Src; 2] {
    let recent = |slot: u16, sec: i64, id: &str| {
        let mut shelf = shelf_in(slot, sec, id, id, &[&format!("{id}-a"), &format!("{id}-b")]);
        for card in &mut shelf.items { Arc::make_mut(card).art = "/art".into(); }
        shelf
    };
    [
        src(0, "", HubState::Ready, Some(built(0, &[], vec![
            shelf(0, "Playlists", "home.playlists", &["pl"]),
            recent(0, 1, "movie.recentlyadded.1"),
            recent(0, 2, "tv.recentlyadded.2"),
            shelf_in(0, 2, "Mixed", "mock.shelf.0", &["m0", "m1"]),
        ]))),
        src(1, "friend", HubState::Ready, Some(built(1, &[], vec![recent(1, 7, "movie.recentlyadded.7")]))),
    ]
}

fn ids(hubs: &[HubRow]) -> Vec<String> { hubs.iter().map(|hub| hub.hub_id.clone()).collect() }

const SERVER_ORDER: [&str; 5] = ["home.playlists", "movie.recentlyadded.1", "tv.recentlyadded.2", "mock.shelf.0", "movie.recentlyadded.7"];

/// The server's own order of `effective`'s libraries in these fixtures: by server, then library key.
fn defaults(effective: &[(ServerId, i64)]) -> Vec<(ServerId, i64)> {
    let mut default = effective.to_vec();
    default.sort_by_key(|(sid, key)| (sid.raw(), *key));
    default
}

fn scope<'a>(effective: &'a [(ServerId, i64)], default: &'a [(ServerId, i64)]) -> BrowseScope<'a> {
    BrowseScope { order: HomeOrder::Moved { effective, default }, ..BrowseScope::standalone() }
}

/// No record: the merge is the server order, source by source, exactly what it was.
#[test]
fn without_a_record_home_is_the_server_order() {
    let srcs = sources();
    let (_, hubs, _) = merge_with_scope(&srcs, &BrowseScope::standalone());
    assert_eq!(ids(&hubs), SERVER_ORDER);
    assert_eq!(published_order(&srcs, &BrowseScope::standalone()), natural_order(&srcs, &[]));
}

/// A record ranks shelves across sources: the friend's library between the owner's own, a
/// library-less shelf staying with the first shelf of ITS source, and a shelf with no recognised hub
/// id following its cards' one library (decided at projection).
#[test]
fn a_record_ranks_shelves_across_sources() {
    let srcs = sources();
    let order = [(sid(1), 7), (sid(0), 2), (sid(0), 1)];
    let (cat, hubs, _) = merge_with_scope(&srcs, &scope(&order, &defaults(&order)));
    assert_eq!(ids(&hubs), ["movie.recentlyadded.7", "home.playlists", "tv.recentlyadded.2", "mock.shelf.0", "movie.recentlyadded.1"],
        "the owner's Playlists lead the owner's own shelves, not the page");
    assert_eq!(cat[hubs[0].start].sid, sid(1), "the friend's row holds the friend's cards");
    assert_eq!(cat[hubs[4].start].rk, "movie.recentlyadded.1-a", "and a row still points at its own cards");
}

/// A shelf with no library of its own inherits the rank of the shelf before it in ITS source.
#[test]
fn a_library_less_shelf_follows_the_shelf_before_it_in_its_source() {
    let mut srcs = sources();
    srcs[0].last.as_mut().unwrap().shelves.swap(0, 1); // recentlyadded.1, then the playlists
    let order = [(sid(0), 2), (sid(1), 7), (sid(0), 1)];
    let (_, hubs, _) = merge_with_scope(&srcs, &scope(&order, &defaults(&order)));
    assert_eq!(ids(&hubs), ["tv.recentlyadded.2", "mock.shelf.0", "movie.recentlyadded.7", "movie.recentlyadded.1", "home.playlists"],
        "Playlists rides behind library 1, the shelf before it");
}

/// The hero pool is built in the server's order whatever order Home shows.
#[test]
fn the_hero_pool_does_not_move_with_the_order() {
    let srcs = sources();
    let pool = |scope: &BrowseScope| {
        let (cat, _, pool) = merge_with_scope(&srcs, scope);
        pool.iter().map(|slot| cat[slot.idx].rk.clone()).collect::<Vec<_>>()
    };
    let order = [(sid(1), 7), (sid(0), 2), (sid(0), 1)];
    assert!(!pool(&BrowseScope::standalone()).is_empty());
    assert_eq!(pool(&scope(&order, &defaults(&order))), pool(&BrowseScope::standalone()));
}

/// A row that gave its cards up to the ring keeps its rank: the shown order is the same before and
/// after the ring takes the cards (the library is read at projection, not from the cards it holds).
#[test]
fn a_released_row_keeps_its_place() {
    let mut srcs = sources();
    for shelf in &mut srcs[0].last.as_mut().unwrap().shelves { shelf.library = ShelfLibrary::projected(&shelf.hub_id, &shelf.key, &shelf.items); }
    let order = [(sid(1), 7), (sid(0), 2), (sid(0), 1)];
    let before = ids(&merge_with_scope(&srcs, &scope(&order, &defaults(&order))).1);
    let mixed = &mut srcs[0].last.as_mut().unwrap().shelves[3];
    mixed.shown = mixed.items.len();
    mixed.items = Vec::new();
    assert!(mixed.released());
    assert_eq!(before, ids(&merge_with_scope(&srcs, &scope(&order, &defaults(&order))).1));
}

/// The ring reads the same order the merge publishes, so a row's index names its own shelf.
#[test]
fn the_ring_maps_rows_in_the_shown_order() {
    let srcs = sources();
    let order = [(sid(1), 7), (sid(0), 2), (sid(0), 1)];
    let published = published_order(&srcs, &scope(&order, &defaults(&order)));
    let (_, hubs, _) = merge_with_scope(&srcs, &scope(&order, &defaults(&order)));
    let deck = hubs.len() - published.len();
    for (at, (i, j)) in published.iter().enumerate() {
        assert_eq!(hubs[deck + at].hub_id, srcs[*i].last.as_ref().unwrap().shelves[*j].hub_id);
    }
}

/// A moved Library order is a moved Home even when no pin and no generation moved with it, so the
/// published Home is rebuilt from the sources already held, with no refetch.
#[test]
fn a_moved_order_alone_moves_the_scope() {
    let pins = [(sid(0), 1, true), (sid(1), 7, true)];
    let (moved, server) = ([(sid(1), 7), (sid(0), 1)], [(sid(0), 1), (sid(1), 7)]);
    let key = |order: HomeOrder<&[(ServerId, i64)]>| BrowseScope { pins: &pins, order, sections_gen: 4, ..BrowseScope::standalone() }.cache_key();
    let moved_order = HomeOrder::Moved { effective: &moved[..], default: &server[..] };
    assert_ne!(key(HomeOrder::Natural), key(moved_order));
    assert_ne!(key(moved_order), key(HomeOrder::Moved { effective: &server[..], default: &server[..] }));
    assert_eq!(key(moved_order), key(HomeOrder::Moved { effective: &moved[..], default: &server[..] }));
}

// ---- review findings: one definition of "natural", unranked and released rows ---------------------

/// Own server `home`: two Recently Added shelves in /hubs order `tv.2, movie.1` (the directory lists
/// library 1 first). A friend `anna`: On Deck first, then `movie.7`, `movie.8`.
fn two_sources() -> [Src; 2] {
    let recent = |slot: u16, sec: i64, id: &str| {
        let mut shelf = shelf_in(slot, sec, id, id, &[&format!("{id}-a")]);
        for card in &mut shelf.items { Arc::make_mut(card).art = "/art".into(); }
        shelf
    };
    [
        src(0, "", HubState::Ready, Some(built(0, &[], vec![recent(0, 2, "tv.recentlyadded.2"), recent(0, 1, "movie.recentlyadded.1")]))),
        src(1, "anna", HubState::Ready, Some(built(1, &[], vec![
            shelf(1, "On Deck", "home.ondeck", &["od"]), recent(1, 7, "movie.recentlyadded.7"), recent(1, 8, "movie.recentlyadded.8")]))),
    ]
}

/// F2: a reorder on the friend's libraries alone leaves the owner's shelves exactly as /hubs had
/// them, and the friend's leading On Deck stays with the friend.
#[test]
fn a_reorder_on_one_source_leaves_the_other_sources_shelves_natural() {
    let srcs = two_sources();
    // the user swapped the friend's two libraries; everything else is the server order
    let order = [(sid(0), 1), (sid(0), 2), (sid(1), 8), (sid(1), 7)];
    let (_, hubs, _) = merge_with_scope(&srcs, &scope(&order, &defaults(&order)));
    assert_eq!(ids(&hubs), ["tv.recentlyadded.2", "movie.recentlyadded.1", "home.ondeck", "movie.recentlyadded.8", "movie.recentlyadded.7"],
        "the owner's /hubs order is untouched and On Deck leads its own source, not the whole page");
}

/// F2: a reorder inside one source moves that source's libraries; a library-less shelf stays attached
/// to the shelf before it in ITS source whatever the other sources do.
#[test]
fn a_source_whose_library_order_is_unchanged_keeps_its_natural_shelves_when_another_moves_between_them() {
    let srcs = two_sources();
    // the friend's library 7 now sits between the owner's two: the owner's own order is unchanged
    let order = [(sid(0), 1), (sid(1), 7), (sid(0), 2), (sid(1), 8)];
    let (_, hubs, _) = merge_with_scope(&srcs, &scope(&order, &defaults(&order)));
    let shown = ids(&hubs);
    let own: Vec<_> = shown.iter().filter(|id| id.starts_with("tv.") || id.starts_with("movie.recentlyadded.1")).cloned().collect();
    assert_eq!(own, ["tv.recentlyadded.2", "movie.recentlyadded.1"], "the owner's own shelves keep /hubs order");
    let at = |id: &str| shown.iter().position(|s| s == id).unwrap();
    assert!(at("home.ondeck") < at("movie.recentlyadded.7"), "On Deck rides with its own source's first shelf");
}

/// F3: a share whose /hubs landed before its directory has no rank yet. It sits where it naturally
/// does (behind the owner's shelves) instead of jumping to the top and back when the directory lands.
#[test]
fn a_source_with_no_ranked_library_holds_its_natural_position() {
    let srcs = two_sources();
    let order = [(sid(0), 1), (sid(0), 2)]; // anna's libraries have not landed
    let (_, hubs, _) = merge_with_scope(&srcs, &scope(&order, &defaults(&order)));
    assert_eq!(ids(&hubs)[2..], ["home.ondeck", "movie.recentlyadded.7", "movie.recentlyadded.8"]);
}

/// An unranked source sits behind EVERYTHING the source before it shows, not behind that source's last
/// shelf: when the record puts the owner's library 2 first, library 1's shelf is the owner's last on
/// the page and a share whose directory has not landed must not slip in between the two.
#[test]
fn an_unranked_source_sits_behind_the_whole_of_a_reordered_source() {
    let mut srcs = two_sources();
    // the owner's /hubs lists library 1 first, so the record's move of library 2 puts the LAST shelf
    // /hubs listed at the front, and the key the source ends on is not its largest
    srcs[0].last.as_mut().unwrap().shelves.swap(0, 1);
    // the owner's libraries swapped (2 before 1); anna's directory has not landed
    let order = [(sid(0), 2), (sid(0), 1)];
    let default = [(sid(0), 1), (sid(0), 2)];
    let (_, hubs, _) = merge_with_scope(&srcs, &scope(&order, &default));
    assert_eq!(ids(&hubs), ["tv.recentlyadded.2", "movie.recentlyadded.1", "home.ondeck", "movie.recentlyadded.7", "movie.recentlyadded.8"],
        "anna's shelves stay behind both of the owner's, whichever order the owner put them in");
    // and when her directory lands and ranks her last, nothing of hers jumps
    let landed = [(sid(0), 2), (sid(0), 1), (sid(1), 7), (sid(1), 8)];
    let landed_default = [(sid(0), 1), (sid(0), 2), (sid(1), 7), (sid(1), 8)];
    let (_, after, _) = merge_with_scope(&srcs, &scope(&landed, &landed_default));
    assert_eq!(ids(&after), ids(&hubs), "ranking her last moves nothing she already showed");
}

/// Browse's verdict on a source as the fixtures spell it: household, and whether it has an owner to name.
fn verdict(sid: ServerId, household: bool, has_handle: bool) -> (ServerId, crate::browse::SourceVerdict) {
    (sid, crate::browse::SourceVerdict { household, has_handle })
}

/// The default source order is Browse's (`browse::source_rank`): a managed profile's family server is
/// the household's though plex.tv sends it a handle, so it leads a friend's share even from the higher
/// slot. This CHANGES Home for a managed profile with no record: it used to list the friend first.
#[test]
fn home_lists_the_household_before_a_share_whichever_slot_holds_it() {
    let keys = roster(&[(61, Some(key(1)), "anna"), (62, Some(key(2)), "mum"), (63, Some(key(3)), "")], 1, 1);
    let verdicts = [verdict(sid(61), false, true), verdict(sid(62), true, true), verdict(sid(63), true, false)];
    let scope = BrowseScope { verdicts: &verdicts, clients: std::borrow::Cow::Borrowed(&keys), ..BrowseScope::standalone() };
    let rank: Vec<u16> = roster_with_scope(&scope).iter().map(|(sid, _)| sid.raw()).collect();
    assert_eq!(rank, [63, 62, 61], "no owner, then the family's, then the friend's share");
    // a source Browse has not classified (the step after a reset empties its table, while the registry
    // still lists every slot) is graded by no second rule, a handle included: it ranks behind the
    // classified ones, in slot order
    let unknown = BrowseScope { verdicts: &[], clients: std::borrow::Cow::Borrowed(&keys), ..BrowseScope::standalone() };
    let rank: Vec<u16> = roster_with_scope(&unknown).iter().map(|(sid, _)| sid.raw()).collect();
    assert_eq!(rank, [61, 62, 63], "no verdict, no grouping: slot order");
    let partly = [verdict(sid(62), true, true)];
    let scope = BrowseScope { verdicts: &partly, clients: std::borrow::Cow::Borrowed(&keys), ..BrowseScope::standalone() };
    let rank: Vec<u16> = roster_with_scope(&scope).iter().map(|(sid, _)| sid.raw()).collect();
    assert_eq!(rank, [62, 61, 63], "the classified source leads, the unclassified follow in slot order");
    // a verdict that moves, on either half, is a moved roster
    let moved = [verdict(sid(61), true, true), verdict(sid(62), true, true), verdict(sid(63), true, false)];
    let named = [verdict(sid(61), false, true), verdict(sid(62), true, true), verdict(sid(63), true, true)];
    let pins = [(sid(61), 1, true)];
    let key_of = |verdicts: &[(ServerId, crate::browse::SourceVerdict)]| BrowseScope { verdicts, pins: &pins, ..BrowseScope::standalone() }.cache_key();
    assert_ne!(key_of(&verdicts), key_of(&moved));
    assert_ne!(key_of(&verdicts), key_of(&named), "a source gaining an owner to name moves the roster too");
    // and with no pin table yet: a reclassification landing before any directory still re-sorts Home
    let no_pins = |verdicts: &[(ServerId, crate::browse::SourceVerdict)]| BrowseScope { verdicts, ..BrowseScope::standalone() }.cache_key();
    assert_ne!(no_pins(&verdicts), no_pins(&moved));
    assert_ne!(no_pins(&verdicts), no_pins(&named));
    assert_eq!(no_pins(&[]), 0, "nothing known, nothing to republish");
}

/// ONE verdict per publication: Home is handed Browse's WHOLE verdict with the directory and computes
/// no half of it. A share that gains a handle in the registry's facts is graded by the directory it is
/// published with, so Home and the Library header (which ranks by the same verdicts) cannot disagree
/// in that publication; the roster follows the facts only when the next directory carries them.
#[test]
fn home_and_the_header_rank_by_one_verdict_inside_a_publication() {
    use crate::browse::source_rank;
    // two household servers: 62 has no owner to name in Browse's table; 61 has one
    let table = [verdict(sid(61), true, true), verdict(sid(62), true, false)];
    // the registry's facts have since given 62 a handle too (a facts refresh Browse has not yet absorbed)
    let refreshed = roster(&[(61, Some(key(1)), "anna"), (62, Some(key(2)), "mum")], 2, 2);
    let directory = crate::stores::browse::DirectorySnapshot::fixture_with_sources(1, 0, Vec::new(), Vec::new()).with_source_verdicts(table.to_vec());
    let scope = BrowseScope::retained(directory.view(), &refreshed);
    let home: Vec<u16> = roster_with_scope(&scope).iter().map(|(sid, _)| sid.raw()).collect();
    let header: Vec<u16> = {
        let mut ranked: Vec<_> = table.iter().map(|(sid, v)| (source_rank(Some(*v), *sid), sid.raw())).collect();
        ranked.sort();
        ranked.into_iter().map(|(_, raw)| raw).collect()
    };
    assert_eq!(header, [62, 61], "no owner to name leads, in Browse's table");
    assert_eq!(home, header, "Home takes the verdict as published, not the fresher facts' handle");
    // the next publication carries the refreshed verdict, and both surfaces move together
    let absorbed = [verdict(sid(61), true, true), verdict(sid(62), true, true)];
    let directory = directory.with_source_verdicts(absorbed.to_vec());
    let scope = BrowseScope::retained(directory.view(), &refreshed);
    let home: Vec<u16> = roster_with_scope(&scope).iter().map(|(sid, _)| sid.raw()).collect();
    assert_eq!(home, [61, 62], "both have an owner to name now: slot order");
}

/// PMS mints one `home.television.recent` hub PER TV library when a household owns two
/// (`twin_tv_libraries_home`), so the hub id does not span libraries: each row ranks by the one
/// library its cards name, and reordering the two libraries swaps the two rows.
#[test]
fn a_reorder_of_two_same_type_libraries_swaps_their_shared_identifier_rows() {
    let build = project(&twin_tv_libraries_home(), &plx_plex::plex::MediaContainer::default(), sid(0));
    let srcs = [src(0, "", HubState::Ready, Some(build))];
    let sections = |hubs: &[HubRow], cat: &[Arc<PmsMovie>]| hubs.iter().map(|hub| cat[hub.start].sec).collect::<Vec<_>>();
    let natural = [(sid(0), 1), (sid(0), 2)];
    let (cat, hubs, _) = merge_with_scope(&srcs, &scope(&natural, &defaults(&natural)));
    assert_eq!(sections(&hubs, &cat), [1, 2]);
    let swapped = [(sid(0), 2), (sid(0), 1)];
    let (cat, hubs, _) = merge_with_scope(&srcs, &scope(&swapped, &defaults(&swapped)));
    assert_eq!(ids(&hubs), ["home.television.recent", "home.television.recent"]);
    assert_eq!(sections(&hubs, &cat), [2, 1], "TV HDR now leads TV");
}

/// A `home.<type>.recent` hub with one library's cards is that library's shelf (the household's only
/// library of the type); a whole-server hub is nobody's, whatever its cards say.
#[test]
fn a_lone_type_hub_is_its_librarys_shelf_and_a_whole_server_hub_is_nobodys() {
    let cards = shelf_in(0, 3, "x", "x", &["a", "b"]).items;
    assert_eq!(ShelfLibrary::projected("home.movies.recent", "", &cards), ShelfLibrary::One(3));
    for id in ["home.playlists", "home.ondeck", "home.continue"] {
        assert_eq!(ShelfLibrary::projected(id, "", &cards), ShelfLibrary::Mixed, "{id}");
    }
}

/// Both surfaces order the sources by the slot number (`ServerId::raw`), not by the order the
/// registry's roster listed the slots in: the roster key of Home is the same whatever that order was.
#[test]
fn home_orders_the_roster_by_slot_number_whatever_order_the_registry_listed_them() {
    let rows = |order: &[u16]| roster(&order.iter().map(|&raw| (raw, Some(key(raw as u32)), if raw == 62 { "anna" } else { "" })).collect::<Vec<_>>(), 1, 1);
    let verdicts = [verdict(sid(61), true, false), verdict(sid(62), false, true), verdict(sid(63), true, false)];
    let order = |listed: &[u16]| {
        let keys = rows(listed);
        let scope = BrowseScope { verdicts: &verdicts, clients: std::borrow::Cow::Borrowed(&keys), ..BrowseScope::standalone() };
        roster_with_scope(&scope).iter().map(|(sid, _)| sid.raw()).collect::<Vec<u16>>()
    };
    assert_eq!(order(&[61, 62, 63]), [61, 63, 62]);
    assert_eq!(order(&[63, 62, 61]), [61, 63, 62], "listed descending, ranked by slot");
    assert_eq!(order(&[62, 63, 61]), [61, 63, 62]);
}

/// A refresh re-projects every row, and the latest projection decides the library of a row that gave
/// its cards up just as it does for one that holds them: the old decision is only what a row projection
/// left undecided (a recording that predates the decision) falls back to.
#[test]
fn a_refresh_decides_a_released_row_from_its_fresh_first_page() {
    let mut was = shelf_in(0, 2, "Mixed", "mock.shelf.0", &["m0", "m1"]);
    was.library = ShelfLibrary::One(2);
    was.shown = was.items.len();
    was.items = Vec::new();
    let old = built(0, &[], vec![was]);
    let mut fresh = shelf_in(0, 5, "Mixed", "mock.shelf.0", &["m2", "m3"]);
    fresh.library = ShelfLibrary::projected(&fresh.hub_id, &fresh.key, &fresh.items);
    let mut new = built(0, &[], vec![fresh]);
    carry_descriptors(&old, &mut new);
    assert_eq!(new.shelves[0].library, ShelfLibrary::One(5), "the fresh first page decided it");
    assert!(new.shelves[0].released());
}

/// A refresh's preview is evidence for a row that still holds cards: a row whose cards sat in one
/// library, refreshed with a preview that spans two, is `Mixed` now, not stuck on the first library
/// for the source's whole life; a row that gave its cards up is decided the same way (above).
#[test]
fn a_refresh_re_decides_a_row_that_holds_a_full_preview() {
    let old = built(0, &[], vec![shelf_in(0, 2, "Mixed", "mock.shelf.0", &["m0", "m1"])]);
    assert_eq!(old.shelves[0].library, ShelfLibrary::One(2), "one library's preview");
    assert!(!old.shelves[0].released());
    let mut fresh = shelf_in(0, 2, "Mixed", "mock.shelf.0", &["m2", "m3"]);
    Arc::make_mut(&mut fresh.items[1]).sec = 5;
    fresh.library = ShelfLibrary::projected(&fresh.hub_id, &fresh.key, &fresh.items);
    let mut new = built(0, &[], vec![fresh]);
    carry_descriptors(&old, &mut new);
    assert_eq!(new.shelves[0].library, ShelfLibrary::Mixed, "two libraries' preview");
    // and back: a later preview of one library is that library's again
    let mut again = built(0, &[], vec![shelf_in(0, 5, "Mixed", "mock.shelf.0", &["m4", "m5"])]);
    carry_descriptors(&new, &mut again);
    assert_eq!(again.shelves[0].library, ShelfLibrary::One(5));
}

/// F7: a window read builds the replacement with no library of its own: the row keeps the projection's.
#[test]
fn a_window_read_keeps_the_library_of_the_row_it_replaces() {
    let mut shelf = shelf_in(0, 2, "Mixed", "mock.shelf.0", &["m0"]);
    shelf.library = ShelfLibrary::One(2);
    let old = shelf.clone();
    let mut refreshed = shelf_in(0, 5, "", "mock.shelf.0", &["m1", "m2"]);
    refreshed.library = ShelfLibrary::Unknown; // what `fetch_window` builds: it decided nothing
    adopt_window(&mut shelf, refreshed, &old);
    assert_eq!(shelf.library, ShelfLibrary::One(2));
    assert_eq!(shelf.title, "Mixed");
}

/// F7, the replay case: a row decoded from a recording keeps the decision it was recorded with, and
/// gives the same place before and after it gives its cards up.
#[test]
fn a_replayed_row_ranks_the_same_before_and_after_it_gives_its_cards_up() {
    let mut srcs = two_sources();
    let replay = |shelf: &Shelf| -> Shelf { serde_json::from_str(&serde_json::to_string(shelf).unwrap()).unwrap() };
    for source in &mut srcs {
        for shelf in &mut source.last.as_mut().unwrap().shelves { *shelf = replay(shelf); }
    }
    let order = [(sid(0), 2), (sid(0), 1), (sid(1), 8), (sid(1), 7)];
    let before = ids(&merge_with_scope(&srcs, &scope(&order, &defaults(&order))).1);
    for source in &mut srcs {
        for shelf in &mut source.last.as_mut().unwrap().shelves {
            shelf.shown = shelf.items.len();
            shelf.items = Vec::new();
        }
    }
    assert_eq!(before, ids(&merge_with_scope(&srcs, &scope(&order, &defaults(&order))).1));
}

/// F7, the live-versus-replay case: a row whose hub id names no library is ranked by the library its
/// first-page cards share. Projection decided it and the recording KEEPS the decision, so the replayed
/// run reads what the live run decided and shows the same order, and keeps it when the row is released.
#[test]
fn a_card_derived_row_ranks_the_same_replayed_as_live() {
    let recent = |sec: i64, id: &str| shelf_in(0, sec, id, id, &[&format!("{id}-a")]);
    let mut live = [src(0, "", HubState::Ready, Some(built(0, &[], vec![
        recent(1, "movie.recentlyadded.1"), shelf_in(0, 2, "Mixed", "mock.shelf.0", &["m0"]), recent(2, "tv.recentlyadded.2"),
    ])))];
    let order = [(sid(0), 2), (sid(0), 1)];
    let show = |srcs: &[Src; 1]| ids(&merge_with_scope(srcs, &scope(&order, &defaults(&order))).1);
    let before = show(&live);
    assert_eq!(before, ["mock.shelf.0", "tv.recentlyadded.2", "movie.recentlyadded.1"], "the mixed row travels with library 2");
    // what a replay hands the landing: every row decoded, the decision it was recorded with intact
    let build = live[0].last.as_mut().unwrap();
    let decided: Vec<ShelfLibrary> = build.shelves.iter().map(|shelf| shelf.library).collect();
    for shelf in &mut build.shelves { *shelf = round_trip(shelf); }
    assert_eq!(build.shelves.iter().map(|shelf| shelf.library).collect::<Vec<_>>(), decided);
    admit(None, build);
    assert_eq!(show(&live), before, "the replayed run ranks it as the live run did");
    let mixed = &mut live[0].last.as_mut().unwrap().shelves[1];
    mixed.shown = mixed.items.len();
    mixed.items = Vec::new();
    assert_eq!(show(&live), before, "and it keeps its place when it gives its cards up");
}

/// A recording from before the decision was recorded has no `library` field: it decodes `Unknown`, and
/// admission decides it from what the hub names, else the cards the row holds (what projection did).
#[test]
fn a_recording_without_the_decision_is_decided_at_admission() {
    let mut json = serde_json::to_value(shelf_in(0, 2, "Mixed", "mock.shelf.0", &["m0", "m1"])).unwrap();
    assert!(json.as_object_mut().unwrap().remove("library").is_some(), "the decision is recorded");
    let mut build = built(0, &[], vec![serde_json::from_value(json).unwrap()]);
    assert_eq!(build.shelves[0].library, ShelfLibrary::Unknown);
    admit(None, &mut build);
    assert_eq!(build.shelves[0].library, ShelfLibrary::One(2));
}

fn round_trip(shelf: &Shelf) -> Shelf { serde_json::from_str(&serde_json::to_string(shelf).unwrap()).unwrap() }

/// F7, live versus replay for a held row paged past its first page: its first page was mixed, the
/// window it holds now has cards of one library. Live, the refresh's window read keeps the first-page
/// projection's `Mixed` ([`adopt_window`]); the recording keeps that decision, so the replayed run
/// publishes the same Home order as the live one.
#[test]
fn a_held_row_paged_past_its_first_page_ranks_the_same_replayed_as_live() {
    let recent = |sec: i64, id: &str| shelf_in(0, sec, id, id, &[&format!("{id}-a")]);
    // the first page the refresh projected: cards of two libraries, so `Mixed`
    let mut first = shelf_in(0, 2, "Mixed", "mock.shelf.0", &["m0", "m1"]);
    Arc::make_mut(&mut first.items[1]).sec = 1;
    first.library = ShelfLibrary::projected(&first.hub_id, &first.key, &first.items);
    assert_eq!(first.library, ShelfLibrary::Mixed);
    let old = first.clone();
    // the window read of the row the user had paged to: past the head, every card in library 2
    let mut window = shelf_in(0, 2, "", "mock.shelf.0", &["m40", "m41"]);
    window.offset = 40;
    window.library = ShelfLibrary::Unknown; // what the window read builds: it decided nothing
    let mut live_row = first;
    adopt_window(&mut live_row, window, &old);
    assert_eq!(live_row.offset, 40);
    let mut live = built(0, &[], vec![recent(1, "movie.recentlyadded.1"), live_row, recent(2, "tv.recentlyadded.2")]);
    admit(None, &mut live); // the live landing is admitted like the replayed one
    let mut replayed = live.clone();
    for shelf in &mut replayed.shelves { *shelf = round_trip(shelf); }
    admit(None, &mut replayed);
    assert_eq!(replayed.shelves[1].library, ShelfLibrary::Mixed, "the recording holds what the live run decided");
    let order = [(sid(0), 2), (sid(0), 1)];
    let show = |build: SourceBuild| {
        let srcs = [src(0, "", HubState::Ready, Some(build))];
        ids(&merge_with_scope(&srcs, &scope(&order, &defaults(&order))).1)
    };
    assert_eq!(show(live), show(replayed.clone()));
    assert_eq!(show(replayed), ["tv.recentlyadded.2", "movie.recentlyadded.1", "mock.shelf.0"],
        "a mixed row ranks nowhere, so it follows the shelf before it");
}

/// Two libraries of one type: PMS answers a `home.television.recent` hub per library, each told
/// apart only by the `sectionID` of its key (`twin_tv_libraries_home`). Paged past their first pages
/// (windows at offset 40 whose cards are all one library's anyway), the hub id names no library: the
/// key does, so both still swap under a reorder, live and replayed. The decision is recorded with the
/// key, which is what makes that the same answer in both.
#[test]
fn twin_recent_rows_paged_past_their_first_page_still_swap_live_and_replayed() {
    let twin = |sec: i64, rk: &str| {
        let mut shelf = shelf_in(0, sec, &format!("TV {sec}"), "home.television.recent", &[rk]);
        shelf.key = format!("/hubs/home/recentlyAdded?type=2&sectionID={sec}");
        shelf.offset = 40;
        shelf.library = ShelfLibrary::projected(&shelf.hub_id, &shelf.key, &shelf.items);
        shelf
    };
    let keys = |hubs: &[HubRow]| hubs.iter().map(|hub| hub.key.clone()).collect::<Vec<_>>();
    let mut live = built(0, &[], vec![twin(1, "t1"), twin(2, "t2")]);
    admit(None, &mut live);
    let mut replayed = live.clone();
    for shelf in &mut replayed.shelves { *shelf = round_trip(shelf); }
    admit(None, &mut replayed);
    assert_eq!(replayed.shelves.iter().map(|shelf| shelf.library).collect::<Vec<_>>(), [ShelfLibrary::One(1), ShelfLibrary::One(2)]);
    let show = |build: &SourceBuild, order: &[(ServerId, i64)]| {
        let srcs = [src(0, "", HubState::Ready, Some(build.clone()))];
        keys(&merge_with_scope(&srcs, &scope(order, &defaults(order))).1)
    };
    let natural = [(sid(0), 1), (sid(0), 2)];
    let swapped = [(sid(0), 2), (sid(0), 1)];
    for build in [&live, &replayed] {
        assert_eq!(show(build, &natural), ["/hubs/home/recentlyAdded?type=2&sectionID=1", "/hubs/home/recentlyAdded?type=2&sectionID=2"]);
        assert_eq!(show(build, &swapped), ["/hubs/home/recentlyAdded?type=2&sectionID=2", "/hubs/home/recentlyAdded?type=2&sectionID=1"]);
    }
}

/// A lone `home.<type>.recent` hub: its key has no `sectionID` (`/hubs/home/recentlyAdded?type=1`,
/// `docs/pms-api.md` §3), so its first page's cards name its library, once. `with_window` pages it.
const LONE_MOVIES: &str = "/hubs/home/recentlyAdded?type=1";

fn lone_movies(first_page: &[&str]) -> Shelf {
    let mut shelf = shelf_in(0, 1, "Recently Added Movies", "home.movies.recent", first_page);
    shelf.key = LONE_MOVIES.into();
    shelf.library = ShelfLibrary::projected(&shelf.hub_id, &shelf.key, &shelf.items);
    shelf.more = true;
    shelf
}

fn library_tv() -> Shelf { shelf_in(0, 2, "TV", "tv.recentlyadded.2", &["tv-a"]) }

/// The row as a window read leaves it: cards `from..`, so offset `from`, deciding no library.
fn with_window(mut shelf: Shelf, from: usize, cards: &[&str]) -> Shelf {
    let window = shelf_in(0, 1, "", &shelf.hub_id, cards);
    shelf.items = window.items;
    shelf.positions = (0..cards.len()).collect();
    shelf.offset = from;
    shelf.end = from + cards.len();
    shelf
}

/// Home's rows for a source holding `build`, under TV then Movies.
fn tv_then_movies(build: &SourceBuild) -> Vec<String> {
    let order = [(sid(0), 2), (sid(0), 1)];
    let srcs = [src(0, "", HubState::Ready, Some(build.clone()))];
    ids(&merge_with_scope(&srcs, &scope(&order, &defaults(&order))).1)
}

/// A lone Recently Added Movies row the user paged past its first window, then a refresh lands while
/// the row is held: the refresh projects the first page again (one library's cards) and the window
/// read puts the row back where it was. The row keeps the library its first page named: Movies must
/// not jump above TV under the user's focus because the window moved.
#[test]
fn a_refresh_of_a_held_paged_lone_recent_row_keeps_its_place() {
    let paged_row = with_window(lone_movies(&["m0", "m1"]), 40, &["m40", "m41"]);
    let old = built(0, &[], vec![paged_row.clone(), library_tv()]);
    assert_eq!(old.shelves[0].library, ShelfLibrary::One(1));
    let before = tv_then_movies(&old);
    assert_eq!(before, ["tv.recentlyadded.2", "home.movies.recent"]);
    // the refresh: the first page, then the window read of the row the user held
    let mut fresh = lone_movies(&["m0", "m1"]);
    let mut window = with_window(fresh.clone(), 40, &["m40", "m41"]);
    window.library = ShelfLibrary::Unknown; // a page read decides nothing
    adopt_window(&mut fresh, window, &paged_row);
    assert_eq!(fresh.offset, 40);
    let mut new = built(0, &[], vec![fresh, library_tv()]);
    admit(Some(&old), &mut new);
    assert_eq!(new.shelves[0].library, ShelfLibrary::One(1));
    assert_eq!(tv_then_movies(&new), before, "Movies stays below TV");
}

/// One row, one library, whatever its window does: card-decided, paged to offset 40, released, then
/// refreshed (a quiet read is admitted when it lands AND when it is published), reloaded and refreshed
/// again. The decision is the same at every step, so the same row at the same offset never holds
/// `One` or `Mixed` by the timing of a refresh.
#[test]
fn a_rows_library_is_constant_across_every_window_move_and_replacement() {
    let mut source = src(0, "", HubState::Ready, Some(built(0, &[], vec![lone_movies(&["m0", "m1"]), library_tv()])));
    let library = |source: &Src| find_shelf(&source.last.as_ref().unwrap().shelves, "home.movies.recent", LONE_MOVIES).unwrap().library;
    let place = |source: &Src| tv_then_movies(source.last.as_ref().unwrap());
    let natural = place(&source);
    assert_eq!(library(&source), ShelfLibrary::One(1));
    assert_eq!(natural, ["tv.recentlyadded.2", "home.movies.recent"]);
    let ask = |reload: bool| PageQuery { id: "home.movies.recent".into(), key: LONE_MOVIES.into(), start: 40, before: false, hidden: Vec::new(), reload };
    // the user pages to offset 40
    let window = |cards: &[&str]| built(0, &[], vec![with_window(lone_movies(&["m0", "m1"]), 40, cards)]);
    let mut moved = window(&["m40", "m41"]);
    moved.shelves[0].library = ShelfLibrary::Unknown;
    assert!(land_page(&mut source, &ask(false), moved));
    assert_eq!(library(&source), ShelfLibrary::One(1), "a window move decides nothing");
    // a refresh lands while the row is held: the first page again, then the window read of the row
    let held = find_shelf(&source.last.as_ref().unwrap().shelves, "home.movies.recent", LONE_MOVIES).unwrap().clone();
    let mut fresh = lone_movies(&["n0", "n1"]);
    let mut again = with_window(fresh.clone(), 40, &["m40", "m41"]);
    again.library = ShelfLibrary::Unknown;
    adopt_window(&mut fresh, again, &held);
    let mut new = built(0, &[], vec![fresh, library_tv()]);
    admit(source.last.as_ref(), &mut new);
    source.last = Some(new);
    assert_eq!(library(&source), ShelfLibrary::One(1), "a held row at offset 40 is the row it was");
    assert_eq!(place(&source), natural);
    // the ring takes its cards
    let row = &mut source.last.as_mut().unwrap().shelves[0];
    row.shown = row.items.len();
    row.items = Vec::new();
    assert!(row.released() && row.offset == 40);
    // a refresh: admitted when the quiet read lands, and AGAIN when it is published, the second time
    // over a row the first already carried to offset 40
    let mut fresh = built(0, &[], vec![lone_movies(&["n0", "n1"]), library_tv()]);
    admit(source.last.as_ref(), &mut fresh);
    assert_eq!(fresh.shelves[0].offset, 40, "the refresh returns the row where it was");
    assert_eq!(fresh.shelves[0].library, ShelfLibrary::One(1));
    admit(source.last.as_ref(), &mut fresh);
    source.last = Some(fresh);
    assert!(find_shelf(&source.last.as_ref().unwrap().shelves, "home.movies.recent", LONE_MOVIES).unwrap().released());
    assert_eq!(library(&source), ShelfLibrary::One(1));
    assert_eq!(place(&source), natural);
    // the row comes back into the ring: its window is read again where it stood
    let mut reloaded = window(&["m40", "m41"]);
    reloaded.shelves[0].library = ShelfLibrary::Unknown;
    assert!(land_reload(&mut source, &ask(true), reloaded));
    assert_eq!(library(&source), ShelfLibrary::One(1));
    // and one more refresh over the held, paged row
    let mut fresh = built(0, &[], vec![lone_movies(&["o0", "o1"]), library_tv()]);
    admit(source.last.as_ref(), &mut fresh);
    source.last = Some(fresh);
    assert_eq!(library(&source), ShelfLibrary::One(1));
    assert_eq!(place(&source), natural);
}

/// The decision is part of the recording: a landing written out and read back (what a replay does) holds
/// the library the live run decided for every row, paged or released or not, and a page read's row,
/// which decides nothing, is written without the field so its recording is what it always was.
#[test]
fn a_recorded_landing_keeps_every_rows_decided_library() {
    let mut mixed = shelf_in(0, 2, "Mixed", "mock.shelf.0", &["m0", "m1"]);
    Arc::make_mut(&mut mixed.items[1]).sec = 5;
    mixed.library = ShelfLibrary::projected(&mixed.hub_id, &mixed.key, &mixed.items);
    let paged_lone = with_window(lone_movies(&["m0"]), 40, &["m40"]);
    let mut released = lone_movies(&["r0"]);
    released.shown = released.items.len();
    released.items = Vec::new();
    let mut live = built(0, &[], vec![mixed, paged_lone, released, library_tv()]);
    admit(None, &mut live);
    let decided: Vec<ShelfLibrary> = live.shelves.iter().map(|shelf| shelf.library).collect();
    assert_eq!(decided, [ShelfLibrary::Mixed, ShelfLibrary::One(1), ShelfLibrary::One(1), ShelfLibrary::One(2)]);
    let mut replayed: SourceBuild = serde_json::from_str(&serde_json::to_string(&live).unwrap()).unwrap();
    admit(None, &mut replayed);
    assert_eq!(replayed.shelves.iter().map(|shelf| shelf.library).collect::<Vec<_>>(), decided);
    assert_eq!(tv_then_movies(&replayed), tv_then_movies(&live));
    let mut page_read = with_window(lone_movies(&["m0"]), 40, &["m40"]);
    page_read.library = ShelfLibrary::Unknown;
    assert!(!serde_json::to_string(&page_read).unwrap().contains("library"), "an undecided row writes no decision");
}

/// A quiet read's build is admitted when it lands and again when it is published. Between the two the
/// ring may release the row it replaces: the fresh projection said `Mixed` (cards of libraries 1 and 5),
/// and the second admit must say the same as it would have with the row still held.
#[test]
fn admitting_a_build_twice_gives_the_same_library_whether_the_ring_released_the_row_between() {
    let held_one = built(0, &[], vec![lone_movies(&["m0", "m1"]), library_tv()]);
    assert_eq!(held_one.shelves[0].library, ShelfLibrary::One(1));
    let mixed_page = || {
        let mut fresh = lone_movies(&["n0", "n1"]);
        Arc::make_mut(&mut fresh.items[1]).sec = 5;
        fresh.library = ShelfLibrary::projected(&fresh.hub_id, &fresh.key, &fresh.items);
        assert_eq!(fresh.library, ShelfLibrary::Mixed);
        built(0, &[], vec![fresh, library_tv()])
    };
    let mut outcomes = Vec::new();
    for released_between in [false, true] {
        let mut last = held_one.clone();
        let mut staged = mixed_page();
        admit(Some(&last), &mut staged); // the quiet read lands: staged
        if released_between {
            let row = &mut last.shelves[0];
            row.shown = row.items.len();
            row.items = Vec::new();
            assert!(row.released());
        }
        admit(Some(&last), &mut staged); // `publish_staged`
        outcomes.push((staged.shelves[0].library, tv_then_movies(&staged)));
    }
    assert_eq!(outcomes[0].0, ShelfLibrary::Mixed);
    assert_eq!(outcomes[0], outcomes[1], "a ring release between the two admits changes nothing");
}
