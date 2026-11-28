//! A Detail page whose item would not load (`failed`): the page the store's `DetailSlot::Failed`
//! reads as, not a title with no body, no spinner and a Play that does nothing.
//!
//! The regression these pin: opening an item the server does not answer for settled the fetch with
//! `None`, the spinner went (`detail_loading()` false), and the page sat on the opening card's
//! title with its hero row drawn, a Play with no item to play and nothing that asked again. Every
//! assertion below reads the page through the engine, the stops it draws and the effects it emits,
//! never through the field a fix happens to keep.

use super::*;
use plx_machine::machine::{FocusTarget, InputOwner};
use plx_ui::a11y::A11yPath;
use plx_ui::focus::{FocusEngine, Outcome};

const OWNER: InputOwner = InputOwner::Entry(EntryId(7));

/// TEST ONLY: the host publishes a platform with no playback driver while this is alive.
struct NoDriver;
impl NoDriver {
    fn set(on: bool) -> Self {
        NO_DRIVER.with(|flag| flag.set(on));
        NoDriver
    }
}
impl Drop for NoDriver {
    fn drop(&mut self) {
        NO_DRIVER.with(|flag| flag.set(false));
    }
}

fn keys() -> plx_plex::plex::ClientKeys {
    plx_plex::plex::ClientKeys::default()
}

fn movie(sid: ServerId, rk: &str) -> Detail {
    Detail {
        sid, rk: rk.into(), kind: "movie".into(), part: "/library/parts/1/file.mkv".into(),
        guid: "plex://movie/5d776b1d96b655001f2a9c3e".into(), extras: vec![trailer_extra()], ..Default::default()
    }
}

/// The page's request effects, as the store receives them.
fn requests(effects: &[plx_machine::machine::Stamped<TestHost>]) -> Vec<(ServerId, String)> {
    effects.iter().filter_map(|effect| match &effect.fx {
        Fx::App(AppFx::Store(StoreId::Metadata, StoreCmd::Metadata(MetadataCmd::RequestDetail { sid, rk }))) => Some((*sid, rk.clone())),
        _ => None,
    }).collect()
}

/// Every effect that would write to the household's record or start playback.
fn writes_or_plays(effects: &[plx_machine::machine::Stamped<TestHost>]) -> bool {
    effects.iter().any(|effect| matches!(&effect.fx,
        Fx::App(AppFx::Store(StoreId::ViewState, _)) | Fx::App(AppFx::Content(ContentReq::Play { .. }))))
}

/// The store answers the request the page just made, with `landed` (`None`: the fetch failed), and
/// the pump installs it.
fn answer(sid: ServerId, rk: &str, landed: Option<Detail>) {
    let generation = plx_data::metadata::detail_generation_for_test(test_store().adapter_ref());
    let (state, adapter) = test_store().split_for_test();
    plx_data::metadata::land_detail_for_test(&keys(), false, state, adapter, sid, rk, generation, landed);
}

/// What the engine's reconcile does with focus now: the key it moves to, else the one held.
fn settle(engine: &mut FocusEngine<u32>, screen: &DetailScreen, held: u32) -> u32 {
    let measure = plx_ui::fixture::FixtureMeasure;
    let now = Cx { focus: engine.read(OWNER), ..cx(&measure, Some(held)) };
    match engine.reconcile::<TestHost>(OWNER, screen, &now) {
        Outcome::Moved { to, .. } => to.elem,
        Outcome::Nothing => held,
        other => panic!("{other:?}"),
    }
}

/// Open the page on `(sid, rk)` the way a push does (Mount, then a fresh Enter) with the engine
/// seated where the container seats it, and return it with the focus it opened on.
fn open(screen: &mut DetailScreen) -> (FocusEngine<u32>, u32, Vec<plx_machine::machine::Stamped<TestHost>>) {
    let measure = plx_ui::fixture::FixtureMeasure;
    step(screen, &ScreenEvent::Mount, None);
    let (_, entered) = step(screen, &ScreenEvent::Enter(plx_ui::screen::Enter::Fresh {
        focus: FocusTarget::FirstInGroup(hero::HERO_GROUP),
    }), None);
    apply_metadata_effects(&entered);
    let mut engine = FocusEngine::new();
    let first = match engine.enter::<TestHost>(OWNER, screen, FocusTarget::FirstInGroup(hero::HERO_GROUP), None, &cx(&measure, None)) {
        Outcome::Moved { to, .. } => to.elem,
        other => panic!("an entry moves focus onto something: {other:?}"),
    };
    (engine, first, entered)
}

fn status_retry() -> A11yPath {
    A11yPath::new().name("status").name("retry")
}

/// The path the page names `elem` by, which is what a harness (and a reader of the log) sees.
fn named(screen: &DetailScreen, elem: u32) -> Option<A11yPath> {
    let measure = plx_ui::fixture::FixtureMeasure;
    Focusable::<TestHost>::a11y(screen, &elem, &cx(&measure, Some(elem))).map(|a| a.path)
}

/// **The page of an item that would not load offers Try again, seats on it, and Try again asks
/// again.** With a driver and without one: with none the page's first seat is the watched toggle,
/// which writes to the account once an item is there, and that is exactly the control a failure
/// must not leave focus on.
#[test]
fn a_page_whose_item_failed_to_load_offers_try_again_and_asks_again() {
    let sid = ServerId::UNSET;
    for no_driver in [false, true] {
        test_store().run(MetadataCmd::Clear);
        let guard = install(movie(sid, "other"));
        let _driver = NoDriver::set(no_driver);
        let mut screen = bare(&guard, sid, "gone");
        let (mut engine, first, entered) = open(&mut screen);
        assert_eq!(requests(&entered), vec![(sid, "gone".to_string())], "no_driver={no_driver}: the open asks once");
        assert!(test_store().view().detail_loading(), "no_driver={no_driver}: and the spinner is up");

        // the server does not answer for the item
        answer(sid, "gone", None);
        assert!(!test_store().view().detail_loading(), "no_driver={no_driver}: the fetch settled");
        step(&mut screen, &ScreenEvent::StoreChanged(StoreId::Metadata.ord(), 1), Some(first));
        let held = settle(&mut engine, &screen, first);
        assert_eq!(named(&screen, held), Some(status_retry()),
            "no_driver={no_driver}: the seat is Try again, not a control of the hero row");

        // the page draws that read-out and nothing else: one stop, which is the seat
        let measure = plx_ui::fixture::FixtureMeasure;
        let stops = drawn_stops(&mut screen, &cx(&measure, Some(held)));
        assert_eq!(stops.iter().map(|stop| stop.key.elem).collect::<Vec<_>>(), vec![held],
            "no_driver={no_driver}: no Play, no watched toggle, nothing to click but Try again");
        let words = plx_ui::draw_census::capture_runs(|| { drawn_stops(&mut screen, &cx(&measure, Some(held))); });
        let (retry, play) = (plx_os::i18n::msg::browse_action_retry(), plx_os::i18n::msg::browse_detail_play());
        assert!(words.iter().any(|run| run.text == retry), "no_driver={no_driver}: Try again is drawn");
        assert!(!words.iter().any(|run| run.text == play), "no_driver={no_driver}: no Play pill");

        // a press that reaches a hero control in the frame before focus moved presses nothing
        for stale in [hero::ELEM_PLAY, hero::ELEM_MARK_WATCHED, hero::ELEM_WATCHLIST] {
            let (_, effects) = step(&mut screen, &ScreenEvent::Activate(stale), Some(stale));
            assert!(!writes_or_plays(&effects) && requests(&effects).is_empty(),
                "no_driver={no_driver}: the hero control {stale} acts on an item that is not there");
        }

        // Try again re-sends the page's own request
        let (_, pressed) = step(&mut screen, &ScreenEvent::Activate(held), Some(held));
        assert_eq!(requests(&pressed), vec![(sid, "gone".to_string())], "no_driver={no_driver}");
        assert!(!writes_or_plays(&pressed));
        apply_metadata_effects(&pressed);
        assert!(test_store().view().detail_loading(), "no_driver={no_driver}: the retry is in flight");
        assert!(matches!(test_store().view().detail_slot(sid, "gone"), plx_data::metadata::DetailSlot::Waiting),
            "no_driver={no_driver}: and the page is back to loading");

        // the retry lands: the item is the page, and focus is off anything that writes
        answer(sid, "gone", Some(movie(sid, "gone")));
        step(&mut screen, &ScreenEvent::StoreChanged(StoreId::Metadata.ord(), 2), Some(held));
        let after = settle(&mut engine, &screen, held);
        assert_ne!(named(&screen, after), Some(status_retry()), "no_driver={no_driver}: Try again is gone with the failure");
        assert!(hero::HeroCtl::of_elem(after).map_or(true, |ctl| !ctl.writes_account()),
            "no_driver={no_driver}: the landing leaves focus on a control that writes to the account ({after})");
        test_store().run(MetadataCmd::Clear);
        clear();
    }
}

/// A retry that fails again is the same page again, with Try again as the seat.
#[test]
fn a_retry_that_fails_again_is_the_failed_page_again() {
    let sid = ServerId::UNSET;
    test_store().run(MetadataCmd::Clear);
    let guard = install(movie(sid, "other"));
    let mut screen = bare(&guard, sid, "gone");
    let (mut engine, first, _) = open(&mut screen);
    answer(sid, "gone", None);
    step(&mut screen, &ScreenEvent::StoreChanged(StoreId::Metadata.ord(), 1), Some(first));
    let held = settle(&mut engine, &screen, first);
    let (_, pressed) = step(&mut screen, &ScreenEvent::Activate(held), Some(held));
    apply_metadata_effects(&pressed);
    answer(sid, "gone", None);
    step(&mut screen, &ScreenEvent::StoreChanged(StoreId::Metadata.ord(), 2), Some(held));
    let again = settle(&mut engine, &screen, held);
    assert_eq!(named(&screen, again), Some(status_retry()));
    let (_, pressed) = step(&mut screen, &ScreenEvent::Activate(again), Some(again));
    assert_eq!(requests(&pressed), vec![(sid, "gone".to_string())], "it can be asked again, as often as the viewer likes");
    test_store().run(MetadataCmd::Clear);
    clear();
}

/// **A refresh of a page that has its item that fails keeps the page**: the Failed read-out is for a
/// page with nothing to show.
#[test]
fn a_failed_refresh_of_a_loaded_page_keeps_its_hero_and_shows_no_read_out() {
    let sid = ServerId::UNSET;
    test_store().run(MetadataCmd::Clear);
    let guard = install(movie(sid, "kept"));
    let mut screen = bare(&guard, sid, "kept");
    let (_, entered) = step(&mut screen, &ScreenEvent::Enter(plx_ui::screen::Enter::Fresh {
        focus: FocusTarget::FirstInGroup(hero::HERO_GROUP),
    }), None);
    apply_metadata_effects(&entered);
    answer(sid, "kept", None);
    let measure = plx_ui::fixture::FixtureMeasure;
    let mut groups = Vec::new();
    Focusable::<TestHost>::groups(&screen, &cx(&measure, None), &mut groups);
    assert!(groups.iter().any(|g| g.id == hero::HERO_GROUP), "the hero row is still the page's");
    let words = plx_ui::draw_census::capture_runs(|| { drawn_stops(&mut screen, &cx(&measure, None)); });
    let retry = plx_os::i18n::msg::browse_action_retry();
    assert!(!words.iter().any(|run| run.text == retry), "a page with its item shows no failure");
    test_store().run(MetadataCmd::Clear);
    clear();
}

/// The failure belongs to the page that asked: a page for another item, covered or about to be
/// pushed, does not read it.
#[test]
fn another_pages_failure_is_not_this_pages() {
    let sid = ServerId::UNSET;
    test_store().run(MetadataCmd::Clear);
    let guard = install(movie(sid, "other"));
    let mut screen = bare(&guard, sid, "gone");
    let (_, _, _) = open(&mut screen);
    answer(sid, "gone", None);
    let elsewhere = bare(&guard, sid, "elsewhere");
    let measure = plx_ui::fixture::FixtureMeasure;
    let mut groups = Vec::new();
    Focusable::<TestHost>::groups(&elsewhere, &cx(&measure, None), &mut groups);
    assert!(groups.iter().any(|g| g.id == hero::HERO_GROUP), "the page for another item keeps its own row");
    test_store().run(MetadataCmd::Clear);
    clear();
}
