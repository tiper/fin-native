//! The hero row's overflow rule (VER-8): after Play/Resume at most [`ROUND_SLOTS`] round buttons —
//! Watchlist, the watch toggle, then the movable actions — and a More (…) button, always last, that
//! holds the rest and never a single item.

use super::*;
use HeroCtl::*;

fn set(watchlist: bool, mark: PosterMark, go_show: bool, restart: bool, version: bool) -> HeroSet {
    HeroSet { watchlist, go_show, version, restart, mark, ..Default::default() }
}

fn row(s: HeroSet) -> Vec<HeroCtl> {
    let (v, n) = hero_ctls(s);
    v[..n].to_vec()
}

fn under_more(s: HeroSet) -> Vec<HeroCtl> {
    let (v, n) = overflow_ctls(s);
    v[..n].to_vec()
}

/// The generic rule, on stand-in actions: 3 or fewer are all round, 4 or more keep the first two and
/// move the rest, and the moved list is never a single item.
#[test]
fn the_rule_is_generic_over_the_action_list() {
    for n in 0..=8usize {
        let all: Vec<usize> = (0..n).collect();
        let (round, moved) = split_actions(&all);
        if n <= ROUND_SLOTS {
            assert_eq!((round.len(), moved.len()), (n, 0), "{n} actions are all round, no …");
        } else {
            assert_eq!(round, &all[..ROUND_SLOTS - 1], "{n} actions keep the first two");
            assert_eq!(moved, &all[ROUND_SLOTS - 1..], "{n} actions move the rest, in order");
        }
        assert_ne!(moved.len(), 1, "{n} actions: … never holds a single item");
    }
}

/// An unwatched movie with one version has Watchlist and Watched only: no restart (nothing to
/// restart), no version. A third action fills the last slot without a …; a 4th (a stand-in for the
/// Audio & Subtitles action another change adds) makes the row Watchlist, Watched, ….
#[test]
fn an_unwatched_one_version_movie_is_two_round_buttons_and_a_fourth_action_brings_the_more_button() {
    let s = set(true, PosterMark::None, false, false, false);
    assert_eq!(row(s), [Play, Watchlist, MarkWatched]);
    assert!(under_more(s).is_empty());

    // Three actions: still all round.
    let three = set(true, PosterMark::None, false, true, false);
    assert_eq!(row(three), [Play, Watchlist, MarkWatched, Restart]);

    // Four (stand-in): the first two stay, … takes the third slot and holds the other two.
    let stand_in = [Watchlist, MarkWatched, Restart, Version, Version];
    let (round, moved) = split_actions(&stand_in[..4]);
    assert_eq!((round, moved), (&stand_in[..2], &stand_in[2..4]));
    let four = set(true, PosterMark::None, false, true, true);
    assert_eq!(row(four), [Play, Watchlist, MarkWatched, More]);
    assert_eq!(under_more(four), [Restart, Version]);
}

/// Part-watched, one version: Resume · Watchlist · Watched · Play from Start.
#[test]
fn a_part_watched_one_version_movie_keeps_three_round_buttons() {
    let s = set(true, PosterMark::InProgress, false, true, false);
    assert_eq!(row(s), [Play, Watchlist, MarkWatched, Restart]);
    assert!(under_more(s).is_empty(), "three actions need no …");
}

/// Part-watched with two versions: four actions, so Play from Start and Play Version go under ….
#[test]
fn a_part_watched_two_version_movie_moves_restart_and_version_under_more() {
    let s = set(true, PosterMark::InProgress, false, true, true);
    assert_eq!(row(s), [Play, Watchlist, MarkWatched, More]);
    assert_eq!(under_more(s), [Restart, Version]);
}

/// A part-watched episode with two versions (no Watchlist on an episode): Watched, Go to Show, Play
/// from Start, Play Version — four actions, so the toggle and Go to Show stay round and … holds the
/// other two in the pinned order.
#[test]
fn a_part_watched_episode_with_two_versions_moves_go_to_show_restart_and_version() {
    let s = set(false, PosterMark::InProgress, true, true, true);
    assert_eq!(row(s), [Play, MarkWatched, GoToShow, More], "four actions: first two, then …");
    assert_eq!(under_more(s), [Restart, Version]);

    // With a bookmark as well, Go to Show moves too.
    let with_bookmark = set(true, PosterMark::InProgress, true, true, true);
    assert_eq!(row(with_bookmark), [Play, Watchlist, MarkWatched, More]);
    assert_eq!(under_more(with_bookmark), [GoToShow, Restart, Version]);
}

/// A started show page: the hero plays the on-deck episode, so it has a restart point; its … is the
/// row's overflow like everywhere else (it no longer opens the episode's hold menu).
#[test]
fn a_started_show_page_has_the_ordinary_overflow() {
    let one_version = set(true, PosterMark::InProgress, false, true, false);
    assert_eq!(row(one_version), [Play, Watchlist, MarkWatched, Restart]);
    let two_versions = set(true, PosterMark::InProgress, false, true, true);
    assert_eq!(row(two_versions), [Play, Watchlist, MarkWatched, More]);
    assert_eq!(under_more(two_versions), [Restart, Version]);
    // A show not yet started has no restart: Watchlist and Watched only.
    assert_eq!(row(set(true, PosterMark::None, false, false, false)), [Play, Watchlist, MarkWatched]);
}

/// Over EVERY set: … is last among the round buttons (only Also available may follow), appears
/// exactly when at least two actions moved, holds in the pinned order, and never holds one item.
#[test]
fn more_is_never_a_single_item_and_always_ends_the_round_buttons() {
    for bits in 0..128u32 {
        for mark in [PosterMark::None, PosterMark::InProgress, PosterMark::Watched] {
            let s = HeroSet {
                watchlist: bits & 1 != 0, go_show: bits & 2 != 0, version: bits & 4 != 0, restart: bits & 8 != 0,
                trailer: bits & 16 != 0, alt: bits & 32 != 0, tracks: bits & 64 != 0, mark,
            };
            let moved = under_more(s);
            let r = row(s);
            assert_ne!(moved.len(), 1, "{s:?}: … never holds a single item");
            assert_eq!(r.contains(&More), !moved.is_empty(), "{s:?}: … exactly when something moved");
            let round = r.iter().filter(|c| !matches!(c, Play | Trailer | Alt | More)).count();
            assert!(round <= ROUND_SLOTS - usize::from(!moved.is_empty()), "{s:?}: at most {ROUND_SLOTS} slots with …");
            if let Some(at) = r.iter().position(|c| *c == More) {
                let after = &r[at + 1..];
                assert!(after.is_empty() || after == [Alt], "{s:?}: … ends the row (Also available may trail it)");
            }
            // Movable actions keep their pinned order wherever they sit.
            let all: Vec<_> = {
                let (v, n) = actions(s);
                v[..n].to_vec()
            };
            let movable: Vec<_> = all.iter().filter(|c| matches!(c, GoToShow | Restart | Version | AudioSubtitles)).collect();
            let want = [GoToShow, Restart, Version, AudioSubtitles];
            assert!(movable.windows(2).all(|w| want.iter().position(|c| c == w[0]) < want.iter().position(|c| c == w[1])), "{s:?}");
            // Alt and the hidden Trailer never count and never move.
            assert!(!moved.contains(&Alt) && !moved.contains(&Trailer), "{s:?}");
        }
    }
}

/// Audio & Subtitles is the LAST movable action: an unwatched one-version movie with tracks is
/// Watchlist, Watched, Audio & Subtitles — three round buttons, no …; a part-watched one tips into …
/// with Play from Start and Audio & Subtitles under it, in that order, and a second version adds
/// Select Version between them. No special case: the rule is `split_actions` over `actions`.
#[test]
fn audio_and_subtitles_is_the_last_movable_action_and_the_rule_places_it() {
    let with_tracks = |watchlist, mark, go_show, restart, version| HeroSet { tracks: true, ..set(watchlist, mark, go_show, restart, version) };

    let unwatched = with_tracks(true, PosterMark::None, false, false, false);
    assert_eq!(row(unwatched), [Play, Watchlist, MarkWatched, AudioSubtitles]);
    assert!(under_more(unwatched).is_empty());

    let part_watched = with_tracks(true, PosterMark::InProgress, false, true, false);
    assert_eq!(row(part_watched), [Play, Watchlist, MarkWatched, More]);
    assert_eq!(under_more(part_watched), [Restart, AudioSubtitles]);

    let versions = with_tracks(true, PosterMark::InProgress, false, true, true);
    assert_eq!(under_more(versions), [Restart, Version, AudioSubtitles]);

    let episode = with_tracks(true, PosterMark::None, true, false, false);
    assert_eq!(row(episode), [Play, Watchlist, MarkWatched, More]);
    assert_eq!(under_more(episode), [GoToShow, AudioSubtitles]);

    // the a11y word round-trips and the ELEM id is its own, below the section block
    assert_eq!(HeroCtl::of_a11y_name(AudioSubtitles.a11y_name()), Some(AudioSubtitles));
    assert_eq!(HeroCtl::of_elem(AudioSubtitles.elem()), Some(AudioSubtitles));
    assert!(AudioSubtitles.elem() < HERO_ELEM_RANGE_END);
    assert!(disc_slot(AudioSubtitles).is_some_and(|slot| slot < DISCS));
}
