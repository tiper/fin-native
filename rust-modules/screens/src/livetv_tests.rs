use super::*;

use plx_data::livetv::{Channel, LiveTvView};
use plx_data::stores::livetv::LiveTvStore;
use plx_machine::machine::{Chrome, FocusRead, Host, InputOwner, InstanceId, PressId, PressRead, ScreenId, Stamped};
use plx_ui::fixture::FixtureMeasure;
use plx_ui::fmt::Wall;
use plx_ui::screen::{At, ScreenArg};

use crate::registry::PlayIntent;

#[derive(Clone, PartialEq, Eq)]
struct TestArg;

impl LogicalState for TestArg {
    fn write(&self, c: &mut Canon) {
        c.u32(0);
    }
    fn probe(&self, _: &mut String) {}
}

impl ScreenArg for TestArg {
    fn chrome(&self) -> Chrome {
        Chrome::TabBar
    }
    fn id(&self) -> ScreenId {
        ScreenId(801)
    }
    fn title(&self) -> Option<&str> {
        None
    }
    fn same_instance(&self, other: &Self) -> bool {
        self == other
    }
}

struct TestHost;

impl Host for TestHost {
    type Arg = TestArg;
    type Fx = AppFx;
    type Msg = crate::registry::AppMsg;
    type Elem = u32;
    type Views<'a> = LiveTvView<'a>;
    type Init = crate::registry::NoInit;
    type Memory = crate::registry::PageMemory;
}

thread_local! {
    /// The wall time the test host publishes (`None`: the frame has no clock).
    static WALL: std::cell::Cell<Option<Wall>> = const { std::cell::Cell::new(None) };
}

impl LiveTvLike for TestHost {
    fn livetv<'a>(cx: &Cx<'a, Self>) -> LiveTvView<'a> {
        cx.views
    }
    fn wall(_cx: &Cx<'_, Self>) -> Option<Wall> {
        WALL.with(std::cell::Cell::get)
    }
}

/// 2026-11-25 20:30:00 UTC, the wall time the caption tests run at.
const NOW: i64 = 1_795_638_600;
const MIN: i64 = 60;

const ENTRY: EntryId = EntryId(7);

fn cx<'a>(view: LiveTvView<'a>, focus: Option<u32>) -> Cx<'a, TestHost> {
    static MEASURE: FixtureMeasure = FixtureMeasure;
    Cx {
        views: view,
        tick: Tick::default(),
        measure: &MEASURE,
        press: PressRead::default(),
        modal: Default::default(), page_image: Default::default(), hold_hint: Default::default(),
        focus: FocusRead { current: focus.map(|elem| FocusKey { entry: ENTRY, elem }), ..Default::default() },
        owner: InputOwner::Entry(ENTRY),
    }
}

fn step(s: &mut LiveTvScreen, view: LiveTvView<'_>, focus: Option<u32>, ev: &ScreenEvent<TestHost>) -> Vec<Stamped<TestHost>> {
    step_handled(s, view, focus, ev).1
}

/// [`step`], also answering whether the screen handled the event.
fn step_handled(s: &mut LiveTvScreen, view: LiveTvView<'_>, focus: Option<u32>, ev: &ScreenEvent<TestHost>) -> (Handled, Vec<Stamped<TestHost>>) {
    let context = cx(view, focus);
    let mut out = Vec::new();
    let mut present = plx_machine::present::Present::new();
    let mut fx = Effects::new(&mut out, plx_machine::machine::MachineId::Instance(InstanceId(9)), &mut present);
    let handled = Machine::<TestHost>::step(s, ev, &context, &mut fx);
    (handled, out)
}

fn channel(vcn: &str, name: &str) -> Channel {
    Channel { sid: plx_plex::plex::ServerId::UNSET, dvr: "7".into(), channel: format!("C{vcn}"),
        vcn: vcn.into(), name: name.into(), thumb: format!("/logos/{vcn}.jpg"), now: "Family Show".into(),
        now_thumb: format!("/programmes/{vcn}.jpg"), now_summary: String::new(), begins_at: 20, ends_at: 100 }
}

/// A channel whose programme began `ago` minutes before [`NOW`] and runs `mins` minutes.
fn airing(vcn: &str, name: &str, ago: i64, mins: i64) -> Channel {
    Channel { begins_at: NOW - ago * MIN, ends_at: NOW - ago * MIN + mins * MIN, ..channel(vcn, name) }
}

fn seeded() -> (LiveTvStore, LiveTvScreen) {
    let mut store = LiveTvStore::default();
    let mut silent = channel("103", "Music");
    silent.now.clear();
    store.install_for_test(vec![channel("101", "Family Channel"), channel("102", "Cartoons"), silent]);
    let mut screen = LiveTvScreen::new(ENTRY);
    step(&mut screen, store.view(), None, &ScreenEvent::Enter(Enter::Fresh { focus: FocusTarget::ContainerGroup(RECENT_GROUP) }));
    (store, screen)
}

/// `store`'s view with `recent` as the profile's Recent row.
fn with_recent<'a>(store: &'a LiveTvStore, recent: &'a [Channel]) -> LiveTvView<'a> {
    store.view().for_test_with_recent(recent)
}

fn asked_plays(out: &[Stamped<TestHost>]) -> Vec<PlayIntent> {
    out.iter().filter_map(|s| match &s.fx {
        Fx::App(AppFx::Content(ContentReq::Play { play, resume_ns: 0 })) => Some(play.clone()), _ => None,
    }).collect()
}

#[test]
fn own_store_supplies_adjacent_recent_and_on_now_shelves_with_distinct_art() {
    let _policy = captions::PolicyPin::default_setting();
    let _language = plx_os::i18n::language_on_this_thread_for_test(plx_os::i18n::Preference::En);
    let (store, mut screen) = seeded();
    let recent = [channel("102", "Cartoons"), channel("101", "Family Channel")];
    let view = with_recent(&store, &recent);
    step(&mut screen, view, None, &ScreenEvent::StoreChanged(StoreId::LiveTv.ord(), 1));
    let context = cx(view, None);
    let mut sections = Vec::new();
    StackPage::<TestHost>::sections(&screen.page, &context, &mut sections);
    assert_eq!(sections.iter().map(|s| s.key).collect::<Vec<_>>(), [Sec::Head, Sec::Recent, Sec::Now, Sec::Channels, Sec::Status]);
    assert!(sections[1..4].iter().all(|s| matches!(s.kind, Kind::Shelf { .. })));
    assert_eq!(screen.page.mode, Mode::RecentNowChannels, "the silent channel is in neither row above");
    // the air under a shelf that has another under it: the handoff's 48 from the tiles to the next heading's line
    // box at rest, less the 18 of the collapsed label band, plus the 8 of ink drop (38); with a tile focused the
    // band's own air, 22, plus the drop (30). The last shelf keeps none.
    for k in [Sec::Recent, Sec::Now] {
        assert_eq!((StackPage::<TestHost>::shelf_foot(&screen.page, &context, k), StackPage::<TestHost>::shelf_foot_open(&screen.page, &context, k)), (38.0, 30.0), "{k:?}");
    }
    assert_eq!((StackPage::<TestHost>::shelf_foot(&screen.page, &context, Sec::Channels), StackPage::<TestHost>::shelf_foot_open(&screen.page, &context, Sec::Channels)), (0.0, 0.0));
    let channels = StackPage::<TestHost>::cards(&screen.page, &context, Sec::Channels).unwrap();
    assert_eq!(CardSource::<TestHost>::len(&channels), 1, "the channel without a programme that is not in Recent");
    assert!(matches!(CardSource::<TestHost>::art(&channels, 0), Art::Poster(Some(_))), "the lineup's own tile");
    assert_eq!(CardSource::<TestHost>::elem(&channels, 0), FIRST_CARD_ELEM + 4, "channel 103, its tile on the even elements");
    let recent_cards = StackPage::<TestHost>::cards(&screen.page, &context, Sec::Recent).unwrap();
    let now = StackPage::<TestHost>::cards(&screen.page, &context, Sec::Now).unwrap();
    assert_eq!(CardSource::<TestHost>::len(&recent_cards), 2, "the profile's own channels, newest first");
    assert_eq!(CardSource::<TestHost>::len(&now), 2, "the channel without programme data has no poster");
    let Art::Channel { logo, number, name, .. } = CardSource::<TestHost>::art(&recent_cards, 0) else { panic!("a logo plate") };
    assert_eq!((logo, number, name), ("/logos/102.jpg", "102", "Cartoons"));
    let Art::Poster(Some(airing)) = CardSource::<TestHost>::art(&now, 0) else { panic!("programme artwork") };
    assert_eq!((airing.thumb, airing.title), ("/programmes/101.jpg", "Family Show"));
    assert_ne!(CardSource::<TestHost>::elem(&recent_cards, 0), CardSource::<TestHost>::elem(&now, 0), "one channel, two shelves, two elements");
}

/// On now's one order: the most recent start first, a tie in lineup order; Recent is the store's order.
#[test]
fn on_now_lists_the_most_recent_start_first_and_a_tie_in_lineup_order() {
    let (mut store, mut screen) = seeded();
    let mut silent = channel("109", "Silent");
    silent.now.clear();
    store.install_for_test(vec![
        airing("101", "Old", 50, 60), airing("102", "Newest", 2, 60), airing("103", "Tie A", 20, 60),
        silent, airing("105", "Tie B", 20, 30), airing("106", "Mid", 35, 60),
    ]);
    step(&mut screen, store.view(), None, &ScreenEvent::Tick(Tick { ms: 16, dt_us: 16_667 }));
    let names: Vec<&str> = screen.page.airings.iter().map(|&i| store.view().channels[i].name.as_str()).collect();
    assert_eq!(names, ["Newest", "Tie A", "Tie B", "Mid", "Old"]);
}

#[test]
fn recent_keeps_its_order_hides_while_empty_and_never_leaves_the_page_without_a_row() {
    let (mut store, mut screen) = seeded();
    let recent = [channel("102", "Cartoons"), channel("101", "Family Channel")];
    step(&mut screen, with_recent(&store, &recent), None, &ScreenEvent::StoreChanged(StoreId::LiveTv.ord(), 1));
    assert_eq!(screen.page.recent, [1, 0], "newest first, as the store keeps it, by lineup position");
    assert_eq!(screen.page.regions().content, &[RECENT_GROUP, NOW_GROUP, CHANNELS_GROUP], "the silent channel keeps a tile, in Channels");
    // no Recent channel and a programme on: the row is hidden
    store.install_for_test(vec![channel("101", "Family Channel"), channel("102", "Cartoons")]);
    step(&mut screen, store.view(), None, &ScreenEvent::StoreChanged(StoreId::LiveTv.ord(), 1));
    assert!(screen.page.recent.is_empty(), "Recent is empty while there are programmes to show");
    assert_eq!(screen.page.regions().content, &[NOW_GROUP]);
    assert_eq!(screen.page.mode, Mode::NowOnly);
    // nothing recent and nothing on: still no Recent row; the lineup is the page's earlier Channels shelf
    let mut dark = channel("101", "Family Channel");
    dark.now.clear();
    let mut dark2 = channel("102", "Cartoons");
    dark2.now.clear();
    store.install_for_test(vec![dark, dark2]);
    step(&mut screen, store.view(), None, &ScreenEvent::StoreChanged(StoreId::LiveTv.ord(), 1));
    assert!(screen.page.recent.is_empty() && screen.page.airings.is_empty());
    assert_eq!(screen.page.mode, Mode::ChannelsOnly);
    assert_eq!(screen.page.regions().content, &[CHANNELS_GROUP]);
}

/// An empty Recent means no Recent shelf, and never a "Recent channels" heading over other channels:
/// with programmes on now the page is On now alone; with a lineup but nothing on it is the page's earlier
/// Channels shelf (the whole lineup, the existing "Channels" heading); with no lineup it is the store's
/// empty read-out.
#[test]
fn an_empty_recent_is_no_recent_shelf_with_programmes_on_with_a_bare_lineup_or_with_none() {
    let _policy = captions::PolicyPin::default_setting();
    let _language = plx_os::i18n::language_on_this_thread_for_test(plx_os::i18n::Preference::En);
    // (programmes on, channels in the lineup)
    for (programmes, channels) in [(true, 2), (false, 2), (false, 0)] {
        let (mut store, mut screen) = seeded();
        let mut lineup = vec![channel("101", "Family Channel"), channel("102", "Cartoons")];
        lineup.truncate(channels);
        if !programmes { for c in &mut lineup { c.now.clear(); } }
        store.install_for_test(lineup);
        step(&mut screen, store.view(), None, &ScreenEvent::StoreChanged(StoreId::LiveTv.ord(), 1));
        let case = format!("programmes {programmes}, channels {channels}");
        assert!(screen.page.recent.is_empty(), "{case}");
        let context = cx(store.view(), None);
        let mut specs = Vec::new();
        StackPage::<TestHost>::sections(&screen.page, &context, &mut specs);
        assert_eq!(specs.iter().map(|s| s.key).collect::<Vec<_>>(), [Sec::Head, Sec::Recent, Sec::Now, Sec::Channels, Sec::Status], "{case}");
        let len = |k: Sec| StackPage::<TestHost>::cards(&screen.page, &context, k).map_or(0, |c| CardSource::<TestHost>::len(&c));
        let first = StackPage::<TestHost>::cards(&screen.page, &context, Sec::Channels);
        assert_eq!(len(Sec::Recent), 0, "{case}: no Recent shelf");
        match (programmes, channels) {
            (true, _) => {
                assert_eq!((len(Sec::Now), len(Sec::Channels)), (2, 0), "{case}");
                assert_eq!(screen.page.regions().content, &[NOW_GROUP], "{case}");
                assert_eq!(screen.page.mode, Mode::NowOnly, "{case}");
                assert!(!screen.page.mode.reads_out(), "{case}: rows, not a read-out");
            }
            (false, 2) => {
                assert_eq!((len(Sec::Now), len(Sec::Channels)), (0, 2), "{case}: the lineup shelf");
                assert_eq!(screen.page.regions().content, &[CHANNELS_GROUP], "{case}");
                assert_eq!(screen.page.mode, Mode::ChannelsOnly, "{case}");
                assert!(matches!(specs[3].kind, Kind::Shelf { style, .. } if *style == NOW_STYLE), "{case}: the earlier shelf's own row style");
                let cards = first.as_ref().expect("lineup cards");
                assert!(matches!(CardSource::<TestHost>::art(cards, 0), Art::Poster(Some(_))), "{case}: a poster tile, not a plate");
                assert!(CardSource::<TestHost>::progress(cards, 0).is_none(), "{case}");
                assert_eq!(CardSource::<TestHost>::elem(cards, 1), FIRST_CARD_ELEM + 2, "{case}");
                assert_eq!(CardSource::<TestHost>::index_of(cards, &(FIRST_CARD_ELEM + 2)), Some(1), "{case}");
            }
            _ => {
                assert_eq!((len(Sec::Now), len(Sec::Channels)), (0, 0), "{case}: no shelf at all");
                assert!(screen.page.regions().content.is_empty(), "{case}");
                assert_eq!(screen.page.mode, Mode::Empty, "{case}");
                assert!(screen.page.mode.reads_out(), "{case}: the store's empty read-out");
                assert_eq!(screen.page.status_overlay(0).caption, plx_os::i18n::msg::browse_livetv_empty_c());
            }
        }
    }
}

#[test]
fn a_recent_tile_says_what_is_on_and_when_it_started_and_a_poster_says_which_channel_and_how_long_is_left() {
    let _policy = captions::PolicyPin::default_setting();
    let _language = plx_os::i18n::language_on_this_thread_for_test(plx_os::i18n::Preference::En);
    let (mut store, mut screen) = seeded();
    let mut silent = channel("103", "Music");
    silent.now.clear();
    // started 3 min ago; started 40 min ago (an hour long); no programme
    store.install_for_test(vec![airing("101", "Family Channel", 3, 60), airing("102", "Cartoons", 40, 60), silent]);
    let recent = store.view().channels.to_vec();
    let view = with_recent(&store, &recent);
    step(&mut screen, view, None, &ScreenEvent::StoreChanged(StoreId::LiveTv.ord(), 1));
    WALL.with(|w| w.set(Some(Wall::from_secs(NOW, 0))));
    let context = cx(view, None);
    let words = |label: &TileLabel| (label.title.as_ref().map(|t| t.to_str().unwrap().to_owned()), label.caption.as_ref().map(|c| c.to_str().unwrap().to_owned()));
    let recent_cards = StackPage::<TestHost>::cards(&screen.page, &context, Sec::Recent).unwrap();
    // just started: how long ago, then what is left
    assert_eq!(words(&CardSource::<TestHost>::label(&recent_cards, 0)),
        (Some("On now: Family Show".into()), Some("Started 3 min ago \u{b7} 57m left".into())));
    // later: the clock times, then what is left
    let (title, caption) = words(&CardSource::<TestHost>::label(&recent_cards, 1));
    assert_eq!(title.as_deref(), Some("On now: Family Show"));
    let caption = caption.unwrap();
    assert!(caption.contains('\u{2013}') && caption.ends_with(" \u{b7} 20m left"), "{caption}");
    // a channel with no programme is its name alone
    assert_eq!(words(&CardSource::<TestHost>::label(&recent_cards, 2)), (Some("Music".into()), None));
    // the poster: the programme over `Ch <number> · <station> · <left>`
    let now = StackPage::<TestHost>::cards(&screen.page, &context, Sec::Now).unwrap();
    assert_eq!(words(&CardSource::<TestHost>::label(&now, 0)),
        (Some("Family Show".into()), Some("Ch 101 \u{b7} Family Channel \u{b7} 57m left".into())));
    // and the bar: how much has aired, from the same airing
    let Art::Poster(Some(facts)) = CardSource::<TestHost>::art(&now, 1) else { panic!("poster") };
    let resume = facts.resume.expect("a programme on air is in progress");
    assert!((resume.frac - 40.0 / 60.0).abs() < 1e-3 && resume.left_ms == 20 * MIN * 1_000, "{resume:?}");
    // the shelf paints that bar: the card source's own progress, and none on the Recent tiles
    assert!((CardSource::<TestHost>::progress(&now, 1).unwrap() - 40.0 / 60.0).abs() < 1e-3);
    assert_eq!(CardSource::<TestHost>::progress(&recent_cards, 1), None);
    // no wall time (a replay): no clock words and no bar, the channel's identity alone
    WALL.with(|w| w.set(None));
    let context = cx(view, None);
    let now = StackPage::<TestHost>::cards(&screen.page, &context, Sec::Now).unwrap();
    assert_eq!(words(&CardSource::<TestHost>::label(&now, 0)), (Some("Family Show".into()), Some("Ch 101 \u{b7} Family Channel".into())));
    let Art::Poster(Some(facts)) = CardSource::<TestHost>::art(&now, 0) else { panic!("poster") };
    assert_eq!(facts.resume, None);
}

#[test]
fn the_page_asks_for_a_frame_when_the_words_of_an_airing_change_and_not_before() {
    let (store, mut screen) = seeded();
    let view = store.view();
    // One tick; what the step asked of the presenter. A fresh `Present` per tick starts dirty but names no
    // cause: a cause is a damage the step itself raised, and the page's own ground animates (motion, not
    // damage) for a while after its first tick, so the cause is what is read.
    let tick = |screen: &mut LiveTvScreen, ms: u32| -> Option<Provenance> {
        let context = cx(view, None);
        let mut out = Vec::new();
        let mut present = plx_machine::present::Present::new();
        {
            let mut fx = Effects::new(&mut out, plx_machine::machine::MachineId::Instance(InstanceId(9)), &mut present);
            Machine::<TestHost>::step(screen, &ScreenEvent::Tick(Tick { ms, dt_us: 16_667 }), &context, &mut fx);
        }
        present.why()
    };
    WALL.with(|w| w.set(None));
    assert_eq!(tick(&mut screen, 16), None, "a page with no wall time has no clock words to redraw");
    assert_eq!(tick(&mut screen, 32), None);
    // the airings begin at 20 s and end at 100 s: at 0 they have not begun, at 30 they are on air
    WALL.with(|w| w.set(Some(Wall::from_secs(0, 0))));
    assert_eq!(tick(&mut screen, 48), Some(Provenance::Input), "the first wall time is read");
    assert_eq!(tick(&mut screen, 64), None, "the same words: nothing to draw");
    WALL.with(|w| w.set(Some(Wall::from_secs(30, 0))));
    assert_eq!(tick(&mut screen, 80), Some(Provenance::Input), "they began: the words and the bar change");
    WALL.with(|w| w.set(Some(Wall::from_secs(31, 0))));
    assert_eq!(tick(&mut screen, 96), None, "the same minute: the same words");
    WALL.with(|w| w.set(None));
}

#[test]
fn channel_and_airing_ok_use_the_same_content_play_without_a_page_tuning_guard() {
    let (store, mut screen) = seeded();
    let recent = store.view().channels.to_vec();
    let view = with_recent(&store, &recent);
    step(&mut screen, view, None, &ScreenEvent::StoreChanged(StoreId::LiveTv.ord(), 1));
    for (press, elem) in [FIRST_CARD_ELEM + 2, FIRST_CARD_ELEM + 3, FIRST_CARD_ELEM + 3].into_iter().enumerate() {
        let focus = FocusKey { entry: ENTRY, elem };
        step(&mut screen, view, Some(elem), &ScreenEvent::FocusMoved { from: None, to: focus, by: plx_ui::screen::By::Dir });
        let out = step(&mut screen, view, Some(elem), &ScreenEvent::PressCommit(PressId(press as u32 + 1)));
        assert_eq!(asked_plays(&out), [PlayIntent::Live { sid: store.view().channels[1].sid, dvr: "7".into(), channel: "C102".into(), title: "Cartoons".into() }]);
    }
}

#[test]
fn strip_and_profile_use_home_requests_and_back_at_the_top_selects_home() {
    use plx_ui::widgets::{TAB_KEY_CHIP, TAB_KEY_HOME, TAB_KEY_LIVETV, TAB_KEY_WATCHLIST};
    let (store, mut screen) = seeded();
    for (elem, expected) in [(TAB_KEY_HOME, HomeReq::Tab(HomeTab::Home)), (TAB_KEY_WATCHLIST, HomeReq::Tab(HomeTab::Watchlist)), (TAB_KEY_CHIP, HomeReq::Account)] {
        let out = step(&mut screen, store.view(), Some(elem), &ScreenEvent::Activate(elem));
        assert!(out.iter().any(|s| matches!(&s.fx, Fx::App(AppFx::Home(req)) if *req == expected)));
    }
    assert_eq!(LiveTvScreen::strip_press(TAB_KEY_LIVETV), Some(None));
    let back = ScreenEvent::Input(InputEvent { at: Tick::default(), source: plx_machine::machine::Source::Script,
        kind: InputKind::Key { key: Key::Back, sym: 0, wcode: 0, edge: Edge::Down, at_edge: false } });
    let out = step(&mut screen, store.view(), Some(FIRST_CARD_ELEM), &back);
    assert!(out.iter().any(|s| matches!(&s.fx, Fx::App(AppFx::Home(HomeReq::Tab(HomeTab::Home))))));
}

#[test]
fn store_revisions_and_notices_refresh_programmes_and_status_retry_targets_live_tv() {
    let (mut store, mut screen) = seeded();
    let mut changed = channel("101", "Family Channel");
    changed.now.clear();
    store.install_for_test(vec![changed]);
    step(&mut screen, store.view(), None, &ScreenEvent::Tick(Tick { ms: 16, dt_us: 16_667 }));
    assert!(screen.page.airings.is_empty(), "the store revision is read on Tick");
    assert!(screen.page.recent.is_empty(), "a page with nothing on has no Recent row of lineup channels");
    assert_eq!(screen.page.mode, Mode::ChannelsOnly, "it shows the lineup as its Channels shelf");
    // a store that says Ready with no channel is the empty read-out too (the page never shows a bare Ready)
    step(&mut screen, LiveTvView::for_test(LiveTvStatus::Ready, &[], &[], 150), None, &ScreenEvent::StoreChanged(StoreId::LiveTv.ord(), 1));
    assert_eq!(screen.page.mode, Mode::Empty);
    for status in [LiveTvStatus::Loading, LiveTvStatus::Empty, LiveTvStatus::Failed] {
        let view = LiveTvView::for_test(status, &[], &[], 100 + status as u32);
        step(&mut screen, view, None, &ScreenEvent::StoreChanged(StoreId::LiveTv.ord(), 1));
        let mode = match status { LiveTvStatus::Loading => Mode::Loading, LiveTvStatus::Empty => Mode::Empty, _ => Mode::Failed };
        assert_eq!(screen.page.mode, mode, "{status:?}");
        assert!(screen.page.mode.reads_out() && screen.page.mode.rows().len() == usize::from(mode == Mode::Failed), "{status:?}: a read-out, with no row but the retry");
        let overlay = screen.page.status_overlay(0);
        assert_eq!(overlay.kind, match status { LiveTvStatus::Loading => StatusKind::Working, LiveTvStatus::Empty => StatusKind::Empty, _ => StatusKind::Failed });
        if status == LiveTvStatus::Empty { assert_eq!(overlay.caption, plx_os::i18n::msg::browse_livetv_empty_c()); }
        assert_eq!(overlay.action.is_some(), status == LiveTvStatus::Failed);
    }
    let failed = LiveTvView::for_test(LiveTvStatus::Failed, &[], &[], 200);
    let out = step(&mut screen, failed, Some(RETRY_ELEM), &ScreenEvent::Activate(RETRY_ELEM));
    assert!(out.iter().any(|s| matches!(&s.fx, Fx::App(AppFx::Store(StoreId::LiveTv, StoreCmd::LiveTv(LiveTvCmd::Retry))))));
}

/// A lineup of three channels, none with a programme: no guide data (the owner's Pluto channels).
fn bare_lineup() -> Vec<Channel> {
    ["101", "102", "103"].into_iter().map(|vcn| Channel { now: String::new(), ..channel(vcn, &format!("Channel {vcn}")) }).collect()
}

/// Where card `elem` is DRAWN once the page has been ticking a while (so the pop, the scroll and the
/// layout are at rest): the placed rect the focus engine and the painter both use, never the spec.
fn settled_rect(screen: &mut LiveTvScreen, view: LiveTvView<'_>, elem: u32) -> Rect {
    for frame in 1..=60 {
        step(screen, view, None, &ScreenEvent::Tick(Tick { ms: frame * 16, dt_us: 16_667 }));
    }
    Focusable::<TestHost>::place(screen, &elem, &cx(view, None), At::Drawn).expect("the card is placed").rect
}

/// Scenario A: the tab opens while the lineup is still loading, then a lineup with no guide data arrives. A
/// shelf's style is copied once when its body is made, so a row that keeps a key across the two draws the
/// lineup's tiles at the Recent row's size.
#[test]
fn a_bare_lineup_that_arrives_after_loading_is_drawn_in_poster_tiles() {
    let mut screen = LiveTvScreen::new(ENTRY);
    let loading = LiveTvView::for_test(LiveTvStatus::Loading, &[], &[], 7_777);
    step(&mut screen, loading, None, &ScreenEvent::Enter(Enter::Fresh { focus: FocusTarget::ContainerGroup(RECENT_GROUP) }));
    step(&mut screen, loading, None, &ScreenEvent::Tick(Tick { ms: 16, dt_us: 16_667 }));
    let mut store = LiveTvStore::default();
    store.install_for_test(bare_lineup());
    let view = store.view();
    step(&mut screen, view, None, &ScreenEvent::StoreChanged(StoreId::LiveTv.ord(), 1));
    let tile = settled_rect(&mut screen, view, FIRST_CARD_ELEM);
    assert_eq!((tile.w, tile.h), (RowStyle::HOME.w, RowStyle::HOME.h), "the lineup's tile is a poster, not a 250 x 140 plate");
}

/// Scenario B: a lineup with no guide data, then a channel is tuned: it becomes Recent while the others stay
/// reachable. The Recent plate is a plate (140 tall), whatever the row held before it, and the lineup's
/// other channels stay in poster tiles.
#[test]
fn tuning_a_channel_in_a_bare_lineup_makes_a_plate_and_leaves_the_rest_in_posters() {
    let (mut store, mut screen) = (LiveTvStore::default(), LiveTvScreen::new(ENTRY));
    store.install_for_test(bare_lineup());
    step(&mut screen, store.view(), None, &ScreenEvent::Enter(Enter::Fresh { focus: FocusTarget::ContainerGroup(RECENT_GROUP) }));
    let first = settled_rect(&mut screen, store.view(), FIRST_CARD_ELEM);
    assert_eq!((first.w, first.h), (RowStyle::HOME.w, RowStyle::HOME.h), "before the tune: the lineup in poster tiles");
    // channel 102 is tuned: the store bumps its revision and Recent holds it
    store.install_for_test(bare_lineup());
    let recent = [store.view().channels[1].clone()];
    let view = with_recent(&store, &recent);
    step(&mut screen, view, None, &ScreenEvent::StoreChanged(StoreId::LiveTv.ord(), 1));
    let plate = settled_rect(&mut screen, view, FIRST_CARD_ELEM + 2);
    assert_eq!((plate.w, plate.h), (RowStyle::CHANNEL.w, RowStyle::CHANNEL.h), "Recent's tile is a 250 x 140 plate");
    for index in [0_u32, 2] {
        let tile = settled_rect(&mut screen, view, FIRST_CARD_ELEM + index * 2);
        assert_eq!((tile.w, tile.h), (RowStyle::HOME.w, RowStyle::HOME.h), "channel {index} is still reachable, in a poster tile");
        assert!(tile.y > plate.y + plate.h, "under Recent");
    }
}

/// The rows the page shows, whatever the lineup holds: every mode, the shelves it has, and that every channel
/// of the lineup has a tile in one of them (a channel without guide data is in Channels, never nowhere).
#[test]
fn every_mode_lists_its_rows_and_every_channel_has_a_tile_somewhere() {
    use Mode::*;
    // 101 and 102 have a programme on; 103 and 104 have none
    let lineup = |programmes: bool| -> Vec<Channel> {
        let mut lineup = vec![channel("101", "One"), channel("102", "Two"), channel("103", "Three"), channel("104", "Four")];
        for (i, c) in lineup.iter_mut().enumerate() { if !programmes || i >= 2 { c.now.clear(); } }
        lineup
    };
    // (programmes on, the Recent channels by lineup position, the mode, the rows)
    let cases: [(bool, &[usize], Mode, &[Sec]); 9] = [
        (true, &[], NowChannels, &[Sec::Now, Sec::Channels]),
        (true, &[0], RecentNowChannels, &[Sec::Recent, Sec::Now, Sec::Channels]),
        (true, &[2], RecentNowChannels, &[Sec::Recent, Sec::Now, Sec::Channels]),
        (true, &[2, 3], RecentNow, &[Sec::Recent, Sec::Now]),
        (true, &[0, 1, 2, 3], RecentNow, &[Sec::Recent, Sec::Now]),
        (false, &[], ChannelsOnly, &[Sec::Channels]),
        (false, &[2], RecentChannels, &[Sec::Recent, Sec::Channels]),
        (false, &[0, 1, 2, 3], RecentOnly, &[Sec::Recent]),
        (false, &[3, 0], RecentChannels, &[Sec::Recent, Sec::Channels]),
    ];
    for (programmes, recent, mode, rows) in cases {
        let (mut store, mut screen) = seeded();
        store.install_for_test(lineup(programmes));
        let recent: Vec<Channel> = recent.iter().map(|&i| store.view().channels[i].clone()).collect();
        let view = with_recent(&store, &recent);
        step(&mut screen, view, None, &ScreenEvent::StoreChanged(StoreId::LiveTv.ord(), 1));
        let case = format!("programmes {programmes}, recent {:?}", recent.iter().map(|c| c.vcn.as_str()).collect::<Vec<_>>());
        assert_eq!((screen.page.mode, screen.page.mode.rows()), (mode, rows), "{case}");
        let context = cx(view, None);
        let mut tiles = std::collections::BTreeSet::new();
        for k in [Sec::Recent, Sec::Now, Sec::Channels] {
            let cards = StackPage::<TestHost>::cards(&screen.page, &context, k);
            assert_eq!(cards.is_some(), rows.contains(&k), "{case}: {k:?}");
            let Some(cards) = cards else { continue };
            for i in 0..CardSource::<TestHost>::len(&cards) {
                let elem = CardSource::<TestHost>::elem(&cards, i);
                assert_eq!(CardSource::<TestHost>::index_of(&cards, &elem), Some(i), "{case}: {k:?} {i}");
                assert!(tiles.insert(elem), "{case}: an element in two shelves");
            }
        }
        // each channel: some tile names it (the element's index is the channel's lineup position)
        for position in 0..4_u32 {
            assert!(tiles.iter().any(|e| (e - FIRST_CARD_ELEM) / 2 == position), "{case}: channel {position} has no tile");
        }
    }
}

/// Recent only: a stored Recent that holds every channel, with no programme on, is one row of plates.
#[test]
fn a_recent_row_with_no_programme_on_is_one_row_of_plates() {
    let _policy = captions::PolicyPin::default_setting();
    let (mut store, mut screen) = seeded();
    let mut lineup = vec![channel("101", "Family Channel"), channel("102", "Cartoons")];
    for c in &mut lineup { c.now.clear(); }
    store.install_for_test(lineup);
    let recent = store.view().channels.to_vec();
    let view = with_recent(&store, &recent);
    step(&mut screen, view, None, &ScreenEvent::StoreChanged(StoreId::LiveTv.ord(), 1));
    assert_eq!(screen.page.mode, Mode::RecentOnly);
    assert_eq!(screen.page.regions().content, &[RECENT_GROUP]);
    let context = cx(view, None);
    let mut specs = Vec::new();
    StackPage::<TestHost>::sections(&screen.page, &context, &mut specs);
    assert!(specs.iter().find(|s| s.key == Sec::Recent).is_some_and(|s| matches!(s.kind, Kind::Shelf { style, .. } if *style == RECENT_STYLE)));
    let plates = StackPage::<TestHost>::cards(&screen.page, &context, Sec::Recent).unwrap();
    assert_eq!(CardSource::<TestHost>::len(&plates), 2);
    assert!(matches!(CardSource::<TestHost>::art(&plates, 0), Art::Channel { .. }));
    assert!(StackPage::<TestHost>::cards(&screen.page, &context, Sec::Now).is_none() && StackPage::<TestHost>::cards(&screen.page, &context, Sec::Channels).is_none());
    // nothing is under it, so it keeps no air
    assert_eq!(StackPage::<TestHost>::shelf_foot(&screen.page, &context, Sec::Recent), 0.0);
    let plate = settled_rect(&mut screen, view, FIRST_CARD_ELEM);
    assert_eq!((plate.x, plate.y, plate.w, plate.h), (96.0, 300.0, 250.0, 140.0));
}

/// The Channels shelf stands under the other rows without moving them: Recent's plates at 300, On now's
/// posters 544 (heading ink 496), and its tiles below On now's.
#[test]
fn the_channels_shelf_stands_under_on_now_and_moves_nothing_above_it() {
    let (mut store, mut screen) = seeded();
    // the seeded lineup: two programmes on, the third channel silent
    let recent = [store.view().channels[0].clone()];
    store.install_for_test(store.view().channels.to_vec());
    let view = with_recent(&store, &recent);
    step(&mut screen, view, None, &ScreenEvent::StoreChanged(StoreId::LiveTv.ord(), 1));
    assert_eq!(screen.page.mode, Mode::RecentNowChannels);
    let plate = settled_rect(&mut screen, view, FIRST_CARD_ELEM);
    let poster = settled_rect(&mut screen, view, FIRST_CARD_ELEM + 1);
    let tile = settled_rect(&mut screen, view, FIRST_CARD_ELEM + 4);
    assert_eq!((plate.y, plate.h), (300.0, 140.0));
    assert_eq!(poster.y, 544.0);
    assert_eq!((tile.w, tile.h), (RowStyle::HOME.w, RowStyle::HOME.h));
    assert!(tile.y > poster.y + poster.h, "the Channels shelf is under On now's posters");
}

/// The page with `focus` held for a second: the scroll it comes to rest at, and where the cards of Recent, On
/// now and Channels rest (`rect`, as drawn: the focused card's carries its focus pop).
fn rest_with_focus(screen: &mut LiveTvScreen, view: LiveTvView<'_>, focus: u32) -> (f32, Rect, Rect, Rect) {
    for frame in 1..=90 {
        step(screen, view, Some(focus), &ScreenEvent::Tick(Tick { ms: frame * 16, dt_us: 16_667 }));
    }
    let context = cx(view, Some(focus));
    let at = |elem: u32| Focusable::<TestHost>::place(screen, &elem, &context, At::Drawn).expect("the card is placed").rect;
    (screen.stack.scroll(), at(FIRST_CARD_ELEM), at(FIRST_CARD_ELEM + 1), at(FIRST_CARD_ELEM + 4))
}

/// **Focusing On now's poster does not scroll the page when the Channels shelf stands under it.** The reveal
/// brings the focused shelf's tiles and open label band onto the screen; the air the page keeps under the shelf
/// for the shelf below's heading is that shelf's own, not part of what has to be visible. So a Down onto On now
/// leaves Recent's plates where they were (300), and an Up to Recent leaves the page at scroll 0.
#[test]
fn focusing_on_now_with_channels_below_does_not_scroll_the_page() {
    let (mut store, mut screen) = seeded();
    let recent = [store.view().channels[0].clone()];
    store.install_for_test(store.view().channels.to_vec());
    let view = with_recent(&store, &recent);
    step(&mut screen, view, None, &ScreenEvent::StoreChanged(StoreId::LiveTv.ord(), 1));
    assert_eq!(screen.page.mode, Mode::RecentNowChannels);
    let (scroll, ..) = rest_with_focus(&mut screen, view, FIRST_CARD_ELEM);
    assert_eq!(scroll, 0.0, "Recent's plate focused: the page at rest");
    // Down onto On now: nothing scrolls, so the plates stay at 300
    let (scroll, plate, poster, _) = rest_with_focus(&mut screen, view, FIRST_CARD_ELEM + 1);
    assert_eq!((scroll, plate.y), (0.0, 300.0), "On now's poster focused: the page does not scroll (poster at {})", poster.y);
    // and an Up to Recent leaves it where it was
    let (scroll, ..) = rest_with_focus(&mut screen, view, FIRST_CARD_ELEM);
    assert_eq!(scroll, 0.0, "Up to Recent: the page stays at rest");
    // Down to On now again, then onto Channels: the page scrolls to show the tile, which ends within the safe frame
    rest_with_focus(&mut screen, view, FIRST_CARD_ELEM + 1);
    let (scroll, _, _, tile) = rest_with_focus(&mut screen, view, FIRST_CARD_ELEM + 4);
    assert!(scroll > 0.0, "the Channels tile has to be brought up");
    assert!(tile.y + tile.h <= SCR_H - MARGIN_Y, "the focused Channels tile ends at {}", tile.y + tile.h);
    // Up to On now from there: the shared minimal reveal brings the poster's block to the page's top margin
    // (it is above the screen), the same as every stacked page; BACK or an unfocused page is what homes it
    let (back, _, poster, _) = rest_with_focus(&mut screen, view, FIRST_CARD_ELEM + 1);
    assert!((back - 442.0).abs() < 0.1, "On now's heading ends at the top margin 54 (the block's top, 496, is brought to the margin): scroll {back} from {scroll}");
    assert!(poster.y >= MARGIN_Y && poster.y + poster.h <= SCR_H - MARGIN_Y, "On now's poster is in the safe frame: {}", poster.y);
    // and the page comes home with focus gone
    for frame in 1..=90 {
        step(&mut screen, view, None, &ScreenEvent::Tick(Tick { ms: 2_000 + frame * 16, dt_us: 16_667 }));
    }
    assert!(screen.stack.scroll().abs() < 0.01, "unfocused, the page is home: {}", screen.stack.scroll());
}
