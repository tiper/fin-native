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
    /// The clock instant the host publishes when it has no wall: a frame whose zone offset could not be read.
    static INSTANT: std::cell::Cell<Option<i64>> = const { std::cell::Cell::new(None) };
}

impl LiveTvLike for TestHost {
    fn livetv<'a>(cx: &Cx<'a, Self>) -> LiveTvView<'a> {
        cx.views
    }
    fn wall(_cx: &Cx<'_, Self>) -> Option<Wall> {
        WALL.with(std::cell::Cell::get)
    }
    fn instant(_cx: &Cx<'_, Self>) -> Option<i64> {
        WALL.with(std::cell::Cell::get).map(|wall| wall.unix_s()).or_else(|| INSTANT.with(std::cell::Cell::get))
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
        now_thumb: format!("/programmes/{vcn}.jpg"), now_summary: String::new(), begins_at: 20, ends_at: 100, later: Vec::new() }
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
    assert_eq!(sections.iter().map(|s| s.key).collect::<Vec<_>>(), [Sec::Head, Sec::Guide, Sec::Browse, Sec::Recent, Sec::Now, Sec::Channels, Sec::Status]);
    assert!(sections[3..6].iter().all(|s| matches!(s.kind, Kind::Shelf { .. })));
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
    let Art::Channel { logo, number, name, .. } = CardSource::<TestHost>::art(&channels, 0) else { panic!("a channel plate") };
    assert_eq!((logo, number, name), ("/logos/103.jpg", "103", "Music"), "the same plate as Recent's");
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
        assert_eq!(specs.iter().map(|s| s.key).collect::<Vec<_>>(), [Sec::Head, Sec::Guide, Sec::Browse, Sec::Recent, Sec::Now, Sec::Channels, Sec::Status], "{case}");
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
                assert!(matches!(specs.iter().find(|s| s.key == Sec::Channels).map(|s| &s.kind), Some(Kind::Shelf { style, .. }) if **style == PLATE_STYLE), "{case}: the plate style, Recent's");
                let cards = first.as_ref().expect("lineup cards");
                assert!(matches!(CardSource::<TestHost>::art(cards, 0), Art::Channel { .. }), "{case}: a channel plate, not a poster tile");
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
/// lineup's tiles as plates, never as posters.
#[test]
fn a_bare_lineup_that_arrives_after_loading_is_drawn_in_channel_plates() {
    let mut screen = LiveTvScreen::new(ENTRY);
    let loading = LiveTvView::for_test(LiveTvStatus::Loading, &[], &[], 7_777);
    step(&mut screen, loading, None, &ScreenEvent::Enter(Enter::Fresh { focus: FocusTarget::ContainerGroup(RECENT_GROUP) }));
    step(&mut screen, loading, None, &ScreenEvent::Tick(Tick { ms: 16, dt_us: 16_667 }));
    let mut store = LiveTvStore::default();
    store.install_for_test(bare_lineup());
    let view = store.view();
    step(&mut screen, view, None, &ScreenEvent::StoreChanged(StoreId::LiveTv.ord(), 1));
    let tile = settled_rect(&mut screen, view, FIRST_CARD_ELEM);
    assert_eq!((tile.w, tile.h), (RowStyle::CHANNEL.w, RowStyle::CHANNEL.h), "the lineup's tile is a 250 x 140 plate, not a poster");
}

/// Scenario B: a lineup with no guide data, then a channel is tuned: it becomes Recent while the others stay
/// reachable. The Recent plate is a plate (140 tall), whatever the row held before it, and the lineup's
/// other channels stay in plates, in the Channels shelf under it.
#[test]
fn tuning_a_channel_in_a_bare_lineup_makes_a_recent_plate_and_leaves_the_rest_in_channel_plates() {
    let (mut store, mut screen) = (LiveTvStore::default(), LiveTvScreen::new(ENTRY));
    store.install_for_test(bare_lineup());
    step(&mut screen, store.view(), None, &ScreenEvent::Enter(Enter::Fresh { focus: FocusTarget::ContainerGroup(RECENT_GROUP) }));
    let first = settled_rect(&mut screen, store.view(), FIRST_CARD_ELEM);
    assert_eq!((first.w, first.h), (RowStyle::CHANNEL.w, RowStyle::CHANNEL.h), "before the tune: the lineup in plates");
    // channel 102 is tuned: the store bumps its revision and Recent holds it
    store.install_for_test(bare_lineup());
    let recent = [store.view().channels[1].clone()];
    let view = with_recent(&store, &recent);
    step(&mut screen, view, None, &ScreenEvent::StoreChanged(StoreId::LiveTv.ord(), 1));
    let plate = settled_rect(&mut screen, view, FIRST_CARD_ELEM + 2);
    assert_eq!((plate.w, plate.h), (RowStyle::CHANNEL.w, RowStyle::CHANNEL.h), "Recent's tile is a 250 x 140 plate");
    for index in [0_u32, 2] {
        let tile = settled_rect(&mut screen, view, FIRST_CARD_ELEM + index * 2);
        assert_eq!((tile.w, tile.h), (RowStyle::CHANNEL.w, RowStyle::CHANNEL.h), "channel {index} is still reachable, in a plate");
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
        assert_eq!((&screen.page.mode, screen.page.mode.rows()), (&mode, rows), "{case}");
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

/// **The posters contain their art on a backdrop; the plates do not read the fit.** On now stands on
/// `RowStyle::PROGRAMME` (owner 2026-11-26, "square should fit with blurred bg", over the cover-crop default
/// of #1557), the plates of Recent and Channels on the cover-fit `CHANNEL` style, and a 2:3 poster draws as it
/// does on Home (`art_fit`'s tests pin the placement itself).
#[test]
fn the_poster_row_contains_its_art_and_the_plates_keep_the_cover_fit() {
    use plx_ui::art_fit::ArtFit;
    assert_eq!(NOW_STYLE.fit, ArtFit::ContainOnBlur);
    assert_eq!(PLATE_STYLE.fit, ArtFit::Cover);
    let tile = |s: &RowStyle| (s.w, s.h, s.gap, s.focus_scale, s.radius);
    assert_eq!(tile(&NOW_STYLE), tile(&RowStyle::HOME), "the tile, its pitch and its pop are Home's");
}

/// **The declared sections' art fits**, read off the sections the page actually declares (not the consts the
/// page is meant to use): On now's posters are `ContainOnBlur`; the plates of Recent and Channels are `Cover`,
/// which the plate's logo (contained in its own box) does not read.
#[test]
fn the_declared_on_now_section_contains_its_art_and_the_plate_sections_keep_the_cover_fit() {
    use plx_ui::art_fit::ArtFit;
    let _policy = captions::PolicyPin::default_setting();
    let (mut store, mut screen) = seeded();
    store.install_for_test(vec![channel("101", "Family Channel"), channel("102", "Cartoons")]);
    let recent = store.view().channels.to_vec();
    let view = with_recent(&store, &recent);
    step(&mut screen, view, None, &ScreenEvent::StoreChanged(StoreId::LiveTv.ord(), 1));
    let context = cx(view, None);
    let mut specs = Vec::new();
    StackPage::<TestHost>::sections(&screen.page, &context, &mut specs);
    let fit = |k: Sec| specs.iter().find(|s| s.key == k).and_then(|s| match s.kind { Kind::Shelf { style, .. } => Some(style.fit), _ => None });
    assert_eq!(fit(Sec::Channels), Some(ArtFit::Cover), "the Channels shelf's plates");
    assert_eq!(fit(Sec::Now), Some(ArtFit::ContainOnBlur), "the On now shelf");
    assert_eq!(fit(Sec::Recent), Some(ArtFit::Cover), "Recent's logo plates keep the cover fit");
}

/// **The Channels shelf draws channel plates, not posters** (owner 2026-11-26, "Why channels are vertical"):
/// the shelf's declared section style is the Recent plate's (`RowStyle::CHANNEL`), its tile is 250 x 140
/// at the 280 pitch, and its cards are `Art::Channel`.
#[test]
fn the_channels_shelf_declares_the_plate_style_and_draws_250_by_140_plates() {
    let _policy = captions::PolicyPin::default_setting();
    let (store, mut screen) = seeded();
    let view = store.view();
    step(&mut screen, view, None, &ScreenEvent::StoreChanged(StoreId::LiveTv.ord(), 1));
    assert_eq!(screen.page.mode, Mode::NowChannels, "the silent channel is in Channels");
    let context = cx(view, None);
    let mut specs = Vec::new();
    StackPage::<TestHost>::sections(&screen.page, &context, &mut specs);
    let style = specs.iter().find(|s| s.key == Sec::Channels).and_then(|s| match s.kind { Kind::Shelf { style, .. } => Some(*style), _ => None }).expect("a shelf");
    assert!(style == RowStyle::CHANNEL, "the Channels shelf is Recent's plate");
    let channels = StackPage::<TestHost>::cards(&screen.page, &context, Sec::Channels).unwrap();
    assert!(matches!(CardSource::<TestHost>::art(&channels, 0), Art::Channel { .. }), "a channel plate");
    let tile = settled_rect(&mut screen, view, FIRST_CARD_ELEM + 4);
    assert_eq!((tile.w, tile.h), (250.0, 140.0), "the Channels tile is the Recent plate's size");
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
    assert!(specs.iter().find(|s| s.key == Sec::Recent).is_some_and(|s| matches!(s.kind, Kind::Shelf { style, .. } if *style == PLATE_STYLE)));
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
/// posters 544 (heading ink 496), and its plates, 250 x 140 like Recent's, below On now's at 1023 (heading ink
/// 975).
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
    assert_eq!((tile.w, tile.h), (RowStyle::CHANNEL.w, RowStyle::CHANNEL.h), "a plate, like Recent's");
    assert_eq!((tile.x, tile.y), (poster.x, 1023.0), "the first Channels plate stands in On now's first column");
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
    // Down to On now again, then onto Channels: the page scrolls to show the plate, which ends within the safe
    // frame. The plates stand at 1023, 140 tall, so the page ends 227 up
    rest_with_focus(&mut screen, view, FIRST_CARD_ELEM + 1);
    let (scroll, _, _, tile) = rest_with_focus(&mut screen, view, FIRST_CARD_ELEM + 4);
    assert!((scroll - 227.0).abs() < 0.1, "the Channels plate is brought up to the end of the page: scroll {scroll}");
    assert!(tile.y + tile.h <= SCR_H - MARGIN_Y, "the focused Channels tile ends at {}", tile.y + tile.h);
    // Up to On now from there: the plates' label band closes, which ends the page 8 sooner (219), and On now's
    // posters, which stood in the safe frame already, stay in it; BACK or an unfocused page is what homes it
    let (back, _, poster, _) = rest_with_focus(&mut screen, view, FIRST_CARD_ELEM + 1);
    assert!((back - 219.0).abs() < 0.1, "On now's poster focused: the page rests at its end, 219 (not at the top margin 442 it took when the Channels tiles were posters): scroll {back} from {scroll}");
    assert!(poster.y >= MARGIN_Y && poster.y + poster.h <= SCR_H - MARGIN_Y, "On now's poster is in the safe frame: {}", poster.y);
    // and the page comes home with focus gone
    for frame in 1..=90 {
        step(&mut screen, view, None, &ScreenEvent::Tick(Tick { ms: 2_000 + frame * 16, dt_us: 16_667 }));
    }
    assert!(screen.stack.scroll().abs() < 0.01, "unfocused, the page is home: {}", screen.stack.scroll());
}

/// The `Enter::Fresh` seats an event pushed, as the elements they name.
fn seated_elems(out: &[Stamped<TestHost>]) -> Vec<u32> {
    out.iter().filter_map(|s| match &s.fx {
        Fx::Deliver(_, Delivery::Screen(ScreenEvent::Enter(Enter::Fresh { focus: FocusTarget::Elem(key) }))) => Some(key.elem),
        _ => None,
    }).collect()
}

fn left(channel: &str) -> ScreenEvent<TestHost> {
    ScreenEvent::App(crate::registry::AppMsg::LiveTvSeat { sid: plx_plex::plex::ServerId::UNSET, dvr: "7".into(), channel: channel.into() })
}

/// **BACK from the live player lands on the channel left, not the card that was pressed.** The player's exit tells the
/// covered page; on its restore it seats that channel's Recent tile, else the row the channel belongs to.
#[test]
fn the_restored_page_seats_the_channel_the_player_left() {
    let (store, mut screen) = seeded();
    // 101 and 102 are on now; 103 has no programme. Recent holds 102 only.
    let recent = [channel("102", "Cartoons")];
    let view = with_recent(&store, &recent);
    step(&mut screen, view, None, &ScreenEvent::StoreChanged(StoreId::LiveTv.ord(), 1));
    for (left_channel, want, why) in [
        ("C102", FIRST_CARD_ELEM + 2, "a Recent channel: its Recent tile (even)"),
        ("C101", FIRST_CARD_ELEM + 1, "a channel only On now holds: its poster (odd)"),
        ("C103", FIRST_CARD_ELEM + 4, "a channel with no programme and not in Recent: the Channels shelf's tile (even)"),
    ] {
        let mut screen = LiveTvScreen::new(ENTRY);
        step(&mut screen, view, None, &ScreenEvent::Enter(Enter::Fresh { focus: FocusTarget::ContainerGroup(RECENT_GROUP) }));
        assert!(step(&mut screen, view, None, &left(left_channel)).is_empty(), "the message only remembers");
        let out = step(&mut screen, view, Some(FIRST_CARD_ELEM), &ScreenEvent::Enter(Enter::Restored));
        assert_eq!(seated_elems(&out), vec![want], "{why}");
        // spent: a second restore seats nothing
        assert!(seated_elems(&step(&mut screen, view, Some(want), &ScreenEvent::Enter(Enter::Restored))).is_empty());
    }
}

fn digit_key(sym: u32, edge: Edge) -> ScreenEvent<TestHost> {
    ScreenEvent::Input(InputEvent { at: Tick::default(), source: plx_machine::machine::Source::Script,
        kind: InputKind::Key { key: Key::Other, sym, wcode: 0, edge, at_edge: false } })
}

fn picked(channel: &str) -> ScreenEvent<TestHost> {
    ScreenEvent::App(crate::registry::AppMsg::LiveChannelPicked { sid: plx_plex::plex::ServerId::UNSET, dvr: "7".into(), channel: channel.into() })
}

/// **A digit typed on the What's on page opens number entry** (Live TV PR5): one request for the panel carrying the digit
/// and nothing else, taken only on the key's press, and only when the lineup has a number to type.
#[test]
fn a_digit_on_the_page_opens_number_entry() {
    let (store, mut screen) = seeded();
    for (sym, digit) in [(48, 0), (53, 5), (57, 9)] {
        let (handled, out) = step_handled(&mut screen, store.view(), Some(FIRST_CARD_ELEM), &digit_key(sym, Edge::Down));
        assert_eq!(handled, Handled::Yes, "sym {sym}");
        let panels: Vec<_> = out.iter().filter_map(|s| match &s.fx {
            Fx::App(AppFx::Content(ContentReq::Panel(panel))) => Some(panel.clone()), _ => None,
        }).collect();
        assert_eq!(panels, vec![ContentPanel::ChannelEntry { first: digit }], "sym {sym}");
        assert!(asked_plays(&out).is_empty(), "opening the panel plays nothing");
    }
    // a held key or its release opens nothing
    for edge in [Edge::Repeat, Edge::Up] {
        let (_, out) = step_handled(&mut screen, store.view(), Some(FIRST_CARD_ELEM), &digit_key(53, edge));
        assert!(!out.iter().any(|s| matches!(&s.fx, Fx::App(AppFx::Content(ContentReq::Panel(_))))), "{edge:?}");
    }
    // a page with no lineup has no number to type: the digit is not taken
    let mut empty = LiveTvScreen::new(ENTRY);
    let (handled, out) = step_handled(&mut empty, LiveTvView::LOADING, None, &digit_key(53, Edge::Down));
    assert_eq!(handled, Handled::No);
    assert!(out.is_empty());
}

/// **The pick plays through the page's own play path**, the request a card's OK makes (named by identity); a channel
/// the lineup no longer lists plays nothing.
#[test]
fn a_pick_from_number_entry_plays_the_channel_like_a_card() {
    let (store, mut screen) = seeded();
    let out = step(&mut screen, store.view(), None, &picked("C102"));
    assert_eq!(asked_plays(&out), vec![PlayIntent::Live {
        sid: plx_plex::plex::ServerId::UNSET, dvr: "7".into(), channel: "C102".into(), title: "Cartoons".into(),
    }]);
    assert!(asked_plays(&step(&mut screen, store.view(), None, &picked("C999"))).is_empty());
}

/// A restore with nobody told, or told of a channel the lineup no longer lists, keeps the focus the engine restored.
#[test]
fn a_restore_with_no_left_channel_seats_nothing() {
    let (store, mut screen) = seeded();
    let recent = [channel("102", "Cartoons")];
    let view = with_recent(&store, &recent);
    assert!(seated_elems(&step(&mut screen, view, Some(FIRST_CARD_ELEM), &ScreenEvent::Enter(Enter::Restored))).is_empty());
    step(&mut screen, view, None, &left("C999"));
    assert!(seated_elems(&step(&mut screen, view, Some(FIRST_CARD_ELEM), &ScreenEvent::Enter(Enter::Restored))).is_empty());
}

/// **The top chrome tucks when the first row has scrolled up under it, whatever that row is.** The handoff's
/// `gone_at` is the first row's pitch: Recent's plates are 140 tall, not a poster's 375, so with the page
/// resting 227 down (a Channels plate focused) the plates stand at 73..213 under the top bar and the chrome
/// has to be gone; at scroll 0 it stands shown. (With the poster pitch, 457, it tucked from a target of 397.)
#[test]
fn the_chrome_tucks_at_the_channels_rest_under_a_plate_first_page_and_stands_at_the_top() {
    use plx_ui::screen::Screen;
    let (mut store, mut screen) = seeded();
    let recent = [store.view().channels[0].clone()];
    store.install_for_test(store.view().channels.to_vec());
    let view = with_recent(&store, &recent);
    step(&mut screen, view, None, &ScreenEvent::StoreChanged(StoreId::LiveTv.ord(), 1));
    assert_eq!(screen.page.mode, Mode::RecentNowChannels);
    let (scroll, ..) = rest_with_focus(&mut screen, view, FIRST_CARD_ELEM);
    assert_eq!((scroll, Screen::<TestHost>::chrome_tuck(&screen)), (0.0, 0.0), "at the top: the chrome shown");
    rest_with_focus(&mut screen, view, FIRST_CARD_ELEM + 1);
    let (scroll, ..) = rest_with_focus(&mut screen, view, FIRST_CARD_ELEM + 4);
    assert!((scroll - 227.0).abs() < 0.1, "the Channels rest: {scroll}");
    assert_eq!(Screen::<TestHost>::chrome_tuck(&screen), 1.0, "the plates are under the top bar: the chrome is gone");
    let (scroll, ..) = rest_with_focus(&mut screen, view, FIRST_CARD_ELEM + 1);
    assert!((scroll - 219.0).abs() < 0.1, "back on On now: {scroll}");
    assert_eq!(Screen::<TestHost>::chrome_tuck(&screen), 1.0, "still tucked (it returns below 102)");
}

// ---- the Guide (SPEC-3) ------------------------------------------------------------------------------------------

use plx_data::livetv::LiveAiring;
use plx_ui::screen::{By, Dir};

/// The Guide's wall time: ten minutes into the half hour that starts at [`NOW`] (20:30), so the window is 20:30 to
/// 22:00 and a minute is 15.47 px along the ruler.
const GUIDE_NOW: i64 = NOW + 10 * MIN;

fn later(title: &str, from: i64, to: i64) -> LiveAiring {
    LiveAiring { title: title.into(), begins_at: NOW + from * MIN, ends_at: NOW + to * MIN, thumb: String::new() }
}

/// A channel airing `from`..`to` minutes after [`NOW`], with the programmes that follow it.
fn guide_channel(vcn: &str, from: i64, to: i64, after: Vec<LiveAiring>) -> Channel {
    Channel { begins_at: NOW + from * MIN, ends_at: NOW + to * MIN, later: after, ..channel(vcn, &format!("Station {vcn}")) }
}

/// Eight channels: 101 began before the window and has two programmes after it (the second runs past the window's
/// end); 102 is on with nothing after it; 103 has no guide data at all; 104 to 108 split the window in two.
fn guide_lineup() -> Vec<Channel> {
    let mut lineup = vec![
        guide_channel("101", -30, 30, vec![later("News at Nine", 30, 60), later("Late Film", 60, 120)]),
        guide_channel("102", 0, 60, Vec::new()),
    ];
    let mut dark = guide_channel("103", 0, 0, Vec::new());
    dark.now.clear();
    lineup.push(dark);
    lineup.extend((4..=8).map(|n| guide_channel(&format!("10{n}"), 0, 30, vec![later("Second Half", 30, 90)])));
    lineup
}

/// A screen on the Guide over [`guide_lineup`], at [`GUIDE_NOW`].
fn guide() -> (LiveTvStore, LiveTvScreen) {
    guide_over(guide_lineup())
}

/// [`guide_lineup`] with a guide that goes on for two hours more: 101's programmes run to 180 minutes, so the window can
/// scroll three half hours before the data ends.
fn long_guide_lineup() -> Vec<Channel> {
    let mut lineup = guide_lineup();
    lineup[0].later.push(later("Epilogue", 120, 180));
    lineup
}

/// A screen on the Guide over `lineup`, at [`GUIDE_NOW`].
fn guide_over(lineup: Vec<Channel>) -> (LiveTvStore, LiveTvScreen) {
    WALL.with(|w| w.set(Some(Wall::from_secs(GUIDE_NOW, 0))));
    let mut store = LiveTvStore::default();
    store.install_for_test(lineup);
    let mut screen = LiveTvScreen::new(ENTRY);
    step(&mut screen, store.view(), None, &ScreenEvent::Enter(Enter::Fresh { focus: FocusTarget::ContainerGroup(RECENT_GROUP) }));
    choose(&mut screen, store.view(), GUIDE_ELEM);
    (store, screen)
}

/// Press the scope row's stop `elem`.
fn choose(screen: &mut LiveTvScreen, view: LiveTvView<'_>, elem: u32) -> Vec<Stamped<TestHost>> {
    let focus = FocusKey { entry: ENTRY, elem };
    step(screen, view, Some(elem), &ScreenEvent::FocusMoved { from: None, to: focus, by: By::Dir });
    let out = step(screen, view, Some(elem), &ScreenEvent::PressCommit(PressId(1)));
    // the next frame lays the page out for the scope the press chose
    step(screen, view, Some(elem), &ScreenEvent::Tick(Tick { ms: 16, dt_us: 16_667 }));
    out
}

fn board(screen: &LiveTvScreen) -> &Board {
    screen.page.board().expect("the Guide's board")
}

/// The element of the Guide's cell `row`, `index`.
fn cell_elem(screen: &LiveTvScreen, row: usize, index: usize) -> u32 {
    board(screen).grid.elem(Cell { row, index }).expect("a cell")
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

/// The rect the stack stands a Guide cell at, with the page at rest (the section's top is the ruler's).
fn rect_of(screen: &LiveTvScreen, view: LiveTvView<'_>, row: usize, index: usize) -> Rect {
    let elem = cell_elem(screen, row, index);
    let context = cx(view, Some(elem));
    Focusable::<TestHost>::place(screen, &elem, &context, At::Drawn).expect("placed").rect
}

fn move_from(screen: &LiveTvScreen, view: LiveTvView<'_>, from: u32, dir: Dir) -> Option<u32> {
    let context = cx(view, Some(from));
    match Focusable::<TestHost>::neighbour(screen, FocusKey { entry: ENTRY, elem: from }, dir, &context) {
        plx_ui::screen::Step::Move(key) => Some(key.elem),
        plx_ui::screen::Step::Edge => None,
    }
}

/// **The scope row chooses the page, and focus stays on its stop.** A fresh page is What's on, with the row's
/// What's on stop on the selected plate; pressing Guide shows the board (and nothing of What's on's shelves),
/// pressing What's on brings the shelves back.
#[test]
fn the_scope_row_switches_between_whats_on_and_the_guide() {
    let _language = plx_os::i18n::language_on_this_thread_for_test(plx_os::i18n::Preference::En);
    WALL.with(|w| w.set(Some(Wall::from_secs(GUIDE_NOW, 0))));
    let (store, mut screen) = seeded();
    assert_eq!((screen.page.scope, &screen.page.mode), (Scope::WhatsOn, &Mode::NowChannels));
    let context = cx(store.view(), None);
    assert_eq!(screen.page.header_row(context.measure).selected_stop(), Some(WHATS_ON_ELEM));
    assert!(screen.page.board().is_none(), "What's on lays out no board");

    choose(&mut screen, store.view(), GUIDE_ELEM);
    assert!(screen.page.scope == Scope::Guide && matches!(screen.page.mode, Mode::Guide(_)));
    assert_eq!(screen.page.header_row(context.measure).selected_stop(), Some(GUIDE_ELEM), "the Guide wears the selected plate");
    assert_eq!(screen.page.regions().content, &[GUIDE_GROUP]);
    let mut sections = Vec::new();
    StackPage::<TestHost>::sections(&screen.page, &context, &mut sections);
    let guide = sections.iter().find(|s| s.key == Sec::Guide).expect("the Guide section");
    assert!(matches!(guide.kind, Kind::Custom { height, focusable: true } if near(height, GUIDE_H)));
    assert!(matches!(sections[0].kind, Kind::Custom { height, focusable: true } if near(height, time_grid::RULER_Y)), "the scope row, then the ruler");
    for k in [Sec::Recent, Sec::Now, Sec::Channels] {
        assert!(StackPage::<TestHost>::cards(&screen.page, &context, k).is_none(), "{k:?} is not on the Guide");
    }

    choose(&mut screen, store.view(), WHATS_ON_ELEM);
    assert_eq!((screen.page.scope, &screen.page.mode), (Scope::WhatsOn, &Mode::NowChannels));
    assert!(screen.page.board().is_none());
    assert_eq!(screen.page.header_row(context.measure).selected_stop(), Some(WHATS_ON_ELEM));
}

#[test]
fn the_scope_stops_are_named_and_the_row_walks_between_them() {
    let (store, screen) = guide();
    let context = cx(store.view(), None);
    let a11y = |elem: u32| Focusable::<TestHost>::a11y(&screen, &elem, &context).map(|a| a.path.to_string());
    assert_eq!(a11y(WHATS_ON_ELEM).as_deref(), Some("header.whats_on"));
    assert_eq!(a11y(GUIDE_ELEM).as_deref(), Some("header.guide"));
    assert_eq!(move_from(&screen, store.view(), WHATS_ON_ELEM, Dir::Right), Some(GUIDE_ELEM));
    assert_eq!(move_from(&screen, store.view(), GUIDE_ELEM, Dir::Left), Some(WHATS_ON_ELEM));
    assert_eq!(move_from(&screen, store.view(), GUIDE_ELEM, Dir::Right), Some(BROWSE_ELEM));
    assert_eq!(move_from(&screen, store.view(), BROWSE_ELEM, Dir::Left), Some(GUIDE_ELEM));
    assert_eq!(move_from(&screen, store.view(), BROWSE_ELEM, Dir::Right), None, "the filter control is on the row only while Browse shows");
    assert_eq!(move_from(&screen, store.view(), WHATS_ON_ELEM, Dir::Left), None);
    assert_eq!(a11y(BROWSE_ELEM).as_deref(), Some("header.browse"));
    // the stops are the row's plates: 96 x 148 of the Library's own row
    let one = Focusable::<TestHost>::place(&screen, &WHATS_ON_ELEM, &context, At::Drawn).unwrap().rect;
    let two = Focusable::<TestHost>::place(&screen, &GUIDE_ELEM, &context, At::Drawn).unwrap().rect;
    assert!(one.x < two.x && near(one.y, two.y) && one.y >= 140.0 && one.y < 160.0, "{one:?} {two:?}");
}

/// **The window starts at the last half hour and the page shows six channels from row 1.**
#[test]
fn the_guide_is_a_90_minute_window_from_the_last_half_hour_six_channels_a_page() {
    let (_, screen) = guide();
    let axis = board(&screen).grid.axis();
    assert_eq!((axis.start(), axis.end()), (NOW, NOW + 90 * MIN), "20:30 to 22:00 at 20:40");
    assert_eq!(board(&screen).grid.rows(), 8, "a row a lineup channel");
    assert_eq!(board(&screen).grid.indicator(), (1, 6, 8));
    // the now line stands ten minutes in
    assert!(near(axis.x_of(GUIDE_NOW), 432.0 + 10.0 * 1_392.0 / 90.0));
}

/// Without a clock the Guide has no window to start from: it is the loading read-out, not a board at the wrong time.
#[test]
fn a_guide_with_no_clock_is_loading_not_a_board() {
    let (store, mut screen) = guide();
    WALL.with(|w| w.set(None));
    step(&mut screen, store.view(), Some(GUIDE_ELEM), &ScreenEvent::Tick(Tick { ms: 16, dt_us: 16_667 }));
    assert_eq!((screen.page.scope, &screen.page.mode), (Scope::Guide, &Mode::Loading));
    assert!(screen.page.board().is_none());
    WALL.with(|w| w.set(Some(Wall::from_secs(GUIDE_NOW, 0))));
    step(&mut screen, store.view(), Some(GUIDE_ELEM), &ScreenEvent::Tick(Tick { ms: 32, dt_us: 16_667 }));
    assert!(matches!(screen.page.mode, Mode::Guide(_)), "the clock came back");
}

/// **A zone the platform cannot read costs the Guide its clock words, not its programmes**: the frame has the instant and
/// no offset, so the board is laid out against the instant (no permanent "Loading channels"), the airing is on, and the
/// frame's clock stamp says there is no zone.
#[test]
fn a_guide_with_the_instant_but_no_zone_is_laid_out_and_tunes() {
    let (store, mut screen) = guide();
    let with_zone = screen.page.clock_stamp(store.view(), Some(GUIDE_NOW), Some(Wall::from_secs(GUIDE_NOW, 0)));
    WALL.with(|w| w.set(None));
    INSTANT.with(|i| i.set(Some(GUIDE_NOW)));
    step(&mut screen, store.view(), Some(GUIDE_ELEM), &ScreenEvent::Tick(Tick { ms: 16, dt_us: 16_667 }));
    assert!(matches!(screen.page.mode, Mode::Guide(_)), "the board stands on the instant alone");
    assert_eq!(board(&screen).grid.axis().start(), NOW);
    assert_ne!(screen.page.clock_stamp(store.view(), Some(GUIDE_NOW), None), with_zone, "the words come and go with the zone");
    let tune = PlayIntent::Live { sid: plx_plex::plex::ServerId::UNSET, dvr: "7".into(), channel: "C101".into(), title: "Station 101".into() };
    assert_eq!(asked_plays(&press_cell(&mut screen, store.view(), 0, 0)), [tune], "what is airing tunes");
    INSTANT.with(|i| i.set(None));
}

/// **A cell stands from where its programme begins (or the window does) to where it ends, less the 8 px gap,
/// 100 tall, on the rows at 292 + 108 r.**
#[test]
fn guide_cells_stand_at_their_times_on_rows_108_apart() {
    let (store, screen) = guide();
    let view = store.view();
    let px = 1_392.0 / 90.0;
    // 101 began 30 minutes before the window and ends 30 minutes in: it starts at the axis' left edge
    let a = rect_of(&screen, view, 0, 0);
    assert!(near(a.x, 432.0) && near(a.w, 30.0 * px - 8.0) && near(a.y, 292.0) && near(a.h, 100.0), "{a:?}");
    // its next, 30 to 60 minutes
    let b = rect_of(&screen, view, 0, 1);
    assert!(near(b.x, 432.0 + 30.0 * px) && near(b.w, 30.0 * px - 8.0), "{b:?}");
    // and the one after, which runs past the window's end: cut at the right edge
    let c = rect_of(&screen, view, 0, 2);
    assert!(near(c.x + c.w + 8.0, 1_824.0), "{c:?}");
    for row in 0..6 {
        assert!(near(rect_of(&screen, view, row, 0).y, 292.0 + 108.0 * row as f32), "row {row}");
    }
    // the second page's rows are not on the frame
    assert!(rect_of(&screen, view, 6, 0).y < 0.0);
}

/// **The next programme is the airing after the current one; a channel with none has one cell.**
#[test]
fn a_channel_with_nothing_after_the_airing_has_one_cell_and_a_channel_with_no_data_has_the_unscheduled_one() {
    let (store, screen) = guide();
    let grid = &board(&screen).grid;
    assert_eq!(grid.cells(0), 3, "the airing and the two after it");
    assert_eq!(grid.cells(1), 1, "no next on 102");
    assert!(!grid.is_unscheduled(1));
    assert_eq!(move_from(&screen, store.view(), cell_elem(&screen, 1, 0), Dir::Right), None, "nothing to the right of the only cell");
    assert!(grid.is_unscheduled(2), "103 has no guide data");
    let full = rect_of(&screen, store.view(), 2, 0);
    assert!(near(full.x, 432.0) && near(full.x + full.w + 8.0, 1_824.0), "one cell across the window: {full:?}");
}

/// **UP and DOWN land on the cell that shares the most time; LEFT and RIGHT walk the row.**
#[test]
fn up_and_down_follow_the_time_and_left_and_right_walk_the_row() {
    let (store, screen) = guide();
    let view = store.view();
    let e = |row, index| cell_elem(&screen, row, index);
    // 101's cell 1 (30-60) down to 102's only cell (0-60)
    assert_eq!(move_from(&screen, view, e(0, 1), Dir::Down), Some(e(1, 0)));
    // from 102's cell to 103's whole-window cell and on to 104's first half (0-30 shares 30 of the 60)
    assert_eq!(move_from(&screen, view, e(1, 0), Dir::Down), Some(e(2, 0)));
    assert_eq!(move_from(&screen, view, e(2, 0), Dir::Down), Some(e(3, 0)), "out of the row with no guide: what is on now (20:40), not the longer cell");
    assert_eq!(move_from(&screen, view, e(3, 0), Dir::Down), Some(e(4, 0)));
    assert_eq!(move_from(&screen, view, e(3, 1), Dir::Up), Some(e(2, 0)));
    assert_eq!(move_from(&screen, view, e(0, 0), Dir::Right), Some(e(0, 1)));
    assert_eq!(move_from(&screen, view, e(0, 1), Dir::Left), Some(e(0, 0)));
    assert_eq!(move_from(&screen, view, e(0, 0), Dir::Left), None);
    // the board's last row has nothing below it
    assert_eq!(move_from(&screen, view, e(7, 0), Dir::Down), None);
}

/// **UP from the first row is the scope row (a link), and DOWN from it seats the airing on now.**
#[test]
fn up_from_the_first_row_goes_to_the_scope_row_and_down_comes_back() {
    let (store, screen) = guide();
    let view = store.view();
    assert_eq!(move_from(&screen, view, cell_elem(&screen, 0, 0), Dir::Up), None, "the board's own edge: the link decides");
    let mut links = Vec::new();
    Screen::<TestHost>::links(&screen, &mut links);
    assert!(links.contains(&Link { from: GUIDE_GROUP, dir: Dir::Up, to: HEAD_GROUP }));
    assert!(links.contains(&Link { from: HEAD_GROUP, dir: Dir::Down, to: GUIDE_GROUP }));
    // the Guide group declares the board's cells, a Control group the board seats the airing on now in
    let context = cx(view, None);
    let mut groups = Vec::new();
    Focusable::<TestHost>::groups(&screen, &context, &mut groups);
    let spec = groups.iter().find(|g| g.id == GUIDE_GROUP).expect("the Guide's group");
    assert_eq!(spec.len, board(&screen).grid.len());
    assert_eq!(spec.elem, ElemKind::Control);
    // first row's airing on now at 20:40: 101's first cell (began 20:00, ends 21:00)
    let from = Focusable::<TestHost>::place(&screen, &GUIDE_ELEM, &context, At::Drawn).unwrap();
    let seated = Focusable::<TestHost>::seat(&screen, GUIDE_GROUP, from, &context);
    assert_eq!(seated.elem, cell_elem(&screen, 0, 0));
}

/// **CH+ and CH- turn the page by six, keeping the slot, and the page follows focus.**
#[test]
fn ch_plus_and_ch_minus_turn_the_page_by_six() {
    let (store, mut screen) = guide();
    let view = store.view();
    let ch_up = ScreenEvent::Input(InputEvent { at: Tick::default(), source: plx_machine::machine::Source::Script,
        kind: InputKind::Key { key: Key::Other, sym: 0, wcode: plx_ui::consts::WCODE_CH_UP_KEY, edge: Edge::Down, at_edge: false } });
    let ch_down = ScreenEvent::Input(InputEvent { at: Tick::default(), source: plx_machine::machine::Source::Script,
        kind: InputKind::Key { key: Key::Other, sym: 0, wcode: plx_ui::consts::WCODE_CH_DOWN_KEY, edge: Edge::Down, at_edge: false } });
    let from = cell_elem(&screen, 1, 0);
    let (handled, out) = step_handled(&mut screen, view, Some(from), &ch_up);
    assert_eq!(handled, Handled::Yes);
    // row 1 + 6 = 7, the last of 8 channels
    assert_eq!(seated_elems(&out), vec![cell_elem(&screen, 7, 0)]);
    assert_eq!(board(&screen).grid.page(), 1, "the page is showing before focus lands on it");
    assert_eq!(board(&screen).grid.indicator(), (7, 8, 8));
    // and back
    let (_, out) = step_handled_at(&mut screen, view, 7, 0, &ch_down);
    assert_eq!(seated_elems(&out), vec![cell_elem(&screen, 1, 0)]);
    assert_eq!(board(&screen).grid.page(), 0);
    // the first page has none before it, the last none after it
    let (handled, out) = step_handled_at(&mut screen, view, 1, 0, &ch_down);
    assert_eq!(handled, Handled::Yes);
    assert!(seated_elems(&out).is_empty());
}

/// **The page follows a move of focus** (DOWN past the sixth row, UP back): the board shows the page focus is on.
#[test]
fn the_page_follows_focus_down_and_up() {
    let (store, mut screen) = guide();
    let view = store.view();
    let (last, next) = (cell_elem(&screen, 5, 0), cell_elem(&screen, 6, 0));
    assert_eq!(move_from(&screen, view, last, Dir::Down), Some(next));
    step(&mut screen, view, Some(next), &ScreenEvent::FocusMoved { from: Some(FocusKey { entry: ENTRY, elem: last }), to: FocusKey { entry: ENTRY, elem: next }, by: By::Dir });
    assert_eq!(board(&screen).grid.page(), 1);
    assert!(near(rect_of(&screen, view, 6, 0).y, 292.0), "the seventh channel is the first row of the second page");
    step(&mut screen, view, Some(last), &ScreenEvent::FocusMoved { from: Some(FocusKey { entry: ENTRY, elem: next }), to: FocusKey { entry: ENTRY, elem: last }, by: By::Dir });
    assert_eq!(board(&screen).grid.page(), 0);
}

fn press_cell(screen: &mut LiveTvScreen, view: LiveTvView<'_>, row: usize, index: usize) -> Vec<Stamped<TestHost>> {
    step_at(screen, view, row, index, &ScreenEvent::PressCommit(PressId(7)))
}

/// [`step`] with focus on the Guide's cell `row`, `index`.
fn step_at(screen: &mut LiveTvScreen, view: LiveTvView<'_>, row: usize, index: usize, ev: &ScreenEvent<TestHost>) -> Vec<Stamped<TestHost>> {
    let elem = cell_elem(screen, row, index);
    step(screen, view, Some(elem), ev)
}

/// [`step_handled`] with focus on the Guide's cell `row`, `index`.
fn step_handled_at(screen: &mut LiveTvScreen, view: LiveTvView<'_>, row: usize, index: usize, ev: &ScreenEvent<TestHost>) -> (Handled, Vec<Stamped<TestHost>>) {
    let elem = cell_elem(screen, row, index);
    step_handled(screen, view, Some(elem), ev)
}

/// **OK on what is airing tunes the channel; OK on a later programme does nothing; the unscheduled cell tunes.**
#[test]
fn ok_tunes_what_is_airing_and_ignores_a_later_programme() {
    let (store, mut screen) = guide();
    let view = store.view();
    let tune = |channel: &str, title: &str| PlayIntent::Live { sid: plx_plex::plex::ServerId::UNSET, dvr: "7".into(), channel: channel.into(), title: title.into() };
    let airing = press_cell(&mut screen, view, 0, 0);
    assert_eq!(asked_plays(&airing), [tune("C101", "Station 101")]);
    assert!(asked_plays(&press_cell(&mut screen, view, 0, 1)).is_empty(), "a later programme is not on");
    assert!(asked_plays(&press_cell(&mut screen, view, 0, 2)).is_empty());
    assert_eq!(asked_plays(&press_cell(&mut screen, view, 1, 0)), [tune("C102", "Station 102")]);
    assert_eq!(asked_plays(&press_cell(&mut screen, view, 2, 0)), [tune("C103", "Station 103")], "no guide data: the cell tunes the channel");
    // the same programme once its time has come is on: a clock past 21:00 makes 101's second cell the airing
    WALL.with(|w| w.set(Some(Wall::from_secs(NOW + 35 * MIN, 0))));
    assert_eq!(asked_plays(&press_cell(&mut screen, view, 0, 1)), [tune("C101", "Station 101")]);
}

/// **A cell has an id, a label and the channel's subject, like a card.**
#[test]
fn guide_cells_are_addressable_with_a_label_and_the_channels_subject() {
    let _language = plx_os::i18n::language_on_this_thread_for_test(plx_os::i18n::Preference::En);
    let (store, screen) = guide();
    let context = cx(store.view(), None);
    let a11y = |row, index| Focusable::<TestHost>::a11y(&screen, &cell_elem(&screen, row, index), &context).expect("an element");
    let first = a11y(0, 1);
    assert!(first.path.to_string().starts_with("guide."), "{}", first.path);
    assert_eq!(first.label.as_deref(), Some("News at Nine"));
    let subject = first.subject.as_deref().expect("a subject").to_owned();
    assert!(subject.starts_with("live-"), "{subject}");
    // every cell of a channel acts on that channel
    assert_eq!(a11y(0, 0).subject.as_deref(), Some(subject.as_str()));
    assert_ne!(a11y(1, 0).subject.as_deref(), Some(subject.as_str()));
    assert_eq!(a11y(2, 0).label.as_deref(), Some("Station 103"), "the unscheduled cell says its channel");
    // the id finds the element back
    let id = a11y(1, 0).path.clone();
    assert_eq!(Focusable::<TestHost>::a11y_find(&screen, &id, &context), Some(cell_elem(&screen, 1, 0)));
}

/// **A digit on the Guide opens number entry**, as on What's on.
#[test]
fn a_digit_on_the_guide_opens_number_entry() {
    let (store, mut screen) = guide();
    let (handled, out) = step_handled_at(&mut screen, store.view(), 0, 0, &digit_key(50, Edge::Down));
    assert_eq!(handled, Handled::Yes);
    assert!(out.iter().any(|s| matches!(&s.fx, Fx::App(AppFx::Content(ContentReq::Panel(ContentPanel::ChannelEntry { first: 2 }))))));
}

/// **BACK from the player returns to the left channel's row**, on its page.
#[test]
fn the_restored_guide_seats_the_row_of_the_channel_the_player_left() {
    let (store, mut screen) = guide();
    let view = store.view();
    step(&mut screen, view, None, &left("C108"));
    let out = step_at(&mut screen, view, 0, 0, &ScreenEvent::Enter(Enter::Restored));
    assert_eq!(seated_elems(&out), vec![cell_elem(&screen, 7, 0)], "the channel's row, its airing cell");
    assert_eq!(board(&screen).grid.page(), 1, "and its page is the one showing");
    // 101 was left: its airing cell (the first), on the first page again
    step(&mut screen, view, None, &left("C101"));
    let out = step_at(&mut screen, view, 7, 0, &ScreenEvent::Enter(Enter::Restored));
    assert_eq!(seated_elems(&out), vec![cell_elem(&screen, 0, 0)]);
    assert_eq!(board(&screen).grid.page(), 0);
}

/// **A refresh keeps the page and the focus on its channel**: the board is rebuilt from the store's revision keeping the
/// page, and a focused cell the refresh took away goes to the last cell of its row.
#[test]
fn a_refresh_keeps_the_page_and_the_focus_on_its_channel() {
    let (mut store, mut screen) = guide();
    let seventh = cell_elem(&screen, 6, 0);
    step(&mut screen, store.view(), Some(seventh), &ScreenEvent::FocusMoved { from: None, to: FocusKey { entry: ENTRY, elem: seventh }, by: By::Dir });
    assert_eq!(board(&screen).grid.page(), 1);
    // 101 loses its programmes after the airing: a refresh
    let mut lineup = guide_lineup();
    lineup[0].later.clear();
    store.install_for_test(lineup);
    step(&mut screen, store.view(), Some(seventh), &ScreenEvent::StoreChanged(StoreId::LiveTv.ord(), 2));
    assert_eq!(board(&screen).grid.page(), 1, "the page the reader was on");
    assert_eq!(board(&screen).grid.cells(0), 1);
    // a focus on 101's third cell, which no longer exists, stays on its row
    let gone = FocusKey { entry: ENTRY, elem: GUIDE_FIRST_ELEM + 2 };
    let context = cx(store.view(), Some(gone.elem));
    assert_eq!(Focusable::<TestHost>::reconcile(&screen, gone, &context).elem, GUIDE_FIRST_ELEM);
}

/// **The now line and chip move with the minute**: the page asks for a frame when the minute changes, and not within it.
#[test]
fn the_guide_asks_for_a_frame_each_minute_for_its_now_line() {
    let (store, mut screen) = guide();
    let view = store.view();
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
    tick(&mut screen, 16);
    tick(&mut screen, 32);
    assert_eq!(tick(&mut screen, 48), None, "the same minute: nothing to draw");
    WALL.with(|w| w.set(Some(Wall::from_secs(GUIDE_NOW + 30, 0))));
    assert_eq!(tick(&mut screen, 64), None, "thirty seconds on: still the same minute");
    WALL.with(|w| w.set(Some(Wall::from_secs(GUIDE_NOW + MIN, 0))));
    assert_eq!(tick(&mut screen, 80), Some(Provenance::Input), "the next minute: the line and the chip move");
}

/// **The canon says the scope and the board's page and window**, so a replay that differs in either differs in state.
#[test]
fn the_canon_carries_the_scope_the_page_and_the_window() {
    let (store, mut screen) = guide();
    let canon = |screen: &LiveTvScreen| { let mut c = Canon::new(); screen.write(&mut c); c.finish() };
    let before = canon(&screen);
    let seventh = cell_elem(&screen, 6, 0);
    step(&mut screen, store.view(), Some(seventh), &ScreenEvent::FocusMoved { from: None, to: FocusKey { entry: ENTRY, elem: seventh }, by: By::Dir });
    assert_ne!(canon(&screen), before, "the page");
    let on_page = canon(&screen);
    WALL.with(|w| w.set(Some(Wall::from_secs(NOW + 40 * MIN, 0))));
    step(&mut screen, store.view(), Some(seventh), &ScreenEvent::Tick(Tick { ms: 16, dt_us: 16_667 }));
    assert_ne!(canon(&screen), on_page, "the window rolled to the next half hour");
    choose(&mut screen, store.view(), WHATS_ON_ELEM);
    assert_ne!(canon(&screen), on_page, "the scope");
}

// ---- the Guide's window scrolls by half hours ----------------------------------------------------------------------

fn clock(now: i64) {
    WALL.with(|w| w.set(Some(Wall::from_secs(now, 0))));
}

/// LEFT or RIGHT that the board's edge hands to the screen (`EdgeRule::Screen`).
fn edge_key(key: Key) -> ScreenEvent<TestHost> {
    ScreenEvent::Input(InputEvent { at: Tick::default(), source: plx_machine::machine::Source::Script,
        kind: InputKind::Key { key, sym: 0, wcode: 0, edge: Edge::Down, at_edge: true } })
}

/// The reaches a step sent to the store, in order.
fn reaches(out: &[Stamped<TestHost>]) -> Vec<i64> {
    out.iter().filter_map(|s| match &s.fx {
        Fx::App(AppFx::Store(StoreId::LiveTv, StoreCmd::LiveTv(LiveTvCmd::Reach(until)))) => Some(*until),
        _ => None,
    }).collect()
}

fn window_of(screen: &LiveTvScreen) -> Window {
    board(screen).window
}

/// **The board's edge hands LEFT and RIGHT to the screen** (so a press past a row's end moves the window), and UP and DOWN
/// stay the engine's.
#[test]
fn the_boards_left_and_right_edges_are_the_screens() {
    let (store, screen) = guide();
    let context = cx(store.view(), None);
    let mut groups = Vec::new();
    Focusable::<TestHost>::groups(&screen, &context, &mut groups);
    let spec = groups.iter().find(|g| g.id == GUIDE_GROUP).expect("the Guide's group");
    assert_eq!(spec.edge, [EdgeRule::Stop, EdgeRule::Stop, EdgeRule::Screen, EdgeRule::Screen]);
}

/// **RIGHT past a row's last cell moves the whole window half an hour later**: the window is held ahead, the ruler follows it,
/// the now line leaves it, the focus lands on the cell of its row that shares the most time with the one it left, and the
/// store is asked to read to the new window's end and a step beyond.
#[test]
fn right_past_the_last_cell_scrolls_the_window_half_an_hour_later() {
    let (store, mut screen) = guide();
    let view = store.view();
    assert_eq!(window_of(&screen), Window::Live);
    // 101's last cell, "Late Film" 60-120, which the live window cuts at 90
    let (handled, out) = step_handled_at(&mut screen, view, 0, 2, &edge_key(Key::Right));
    assert_eq!(handled, Handled::Yes);
    assert_eq!(window_of(&screen), Window::Ahead(NOW + 30 * MIN));
    let axis = board(&screen).grid.axis();
    assert_eq!((axis.start(), axis.end()), (NOW + 30 * MIN, NOW + 120 * MIN));
    assert_eq!(axis.ticks().collect::<Vec<_>>(), [NOW + 30 * MIN, NOW + 60 * MIN, NOW + 90 * MIN], "the ruler's labels are the later half hours");
    assert!(!axis.window().contains(GUIDE_NOW), "now is before the window: no now line");
    // "News at Nine" 30-60 and "Late Film" 60-120: the programme the focus left, whole now
    assert_eq!(seated_elems(&out), vec![cell_elem(&screen, 0, 1)]);
    assert_eq!(board(&screen).grid.span(Cell { row: 0, index: 1 }).map(|s| (s.begins, s.ends)), Some((NOW + 60 * MIN, NOW + 120 * MIN)));
    assert_eq!(reaches(&out), [NOW + 120 * MIN + 30 * MIN], "read to the window's end and a step more");
}

/// **LEFT past a row's first cell moves it back, and never earlier than the live window.**
#[test]
fn left_scrolls_back_and_stops_at_the_live_window() {
    let (store, mut screen) = guide_over(long_guide_lineup());
    let view = store.view();
    // at the live window LEFT past the first cell does nothing (and is spent)
    let (handled, out) = step_handled_at(&mut screen, view, 0, 0, &edge_key(Key::Left));
    assert_eq!((handled, window_of(&screen)), (Handled::Yes, Window::Live));
    assert!(seated_elems(&out).is_empty() && reaches(&out).is_empty());
    step_handled_at(&mut screen, view, 0, 2, &edge_key(Key::Right));
    step_handled_at(&mut screen, view, 0, 1, &edge_key(Key::Right));
    assert_eq!(window_of(&screen), Window::Ahead(NOW + 60 * MIN), "two steps on");
    // 101's first cell at 60 is "Late Film": back a step, where it is the second cell and the focus lands on it
    let (_, out) = step_handled_at(&mut screen, view, 0, 0, &edge_key(Key::Left));
    assert_eq!(window_of(&screen), Window::Ahead(NOW + 30 * MIN));
    assert_eq!(seated_elems(&out), vec![cell_elem(&screen, 0, 1)], "the focus lands again, on the programme it left");
    let (_, out) = step_handled_at(&mut screen, view, 0, 0, &edge_key(Key::Left));
    assert_eq!(window_of(&screen), Window::Live, "back at the live window");
    assert_eq!(board(&screen).grid.axis().start(), NOW);
    assert_eq!(reaches(&out), [NOW + 90 * MIN + 30 * MIN]);
    // and no further
    let (_, out) = step_handled_at(&mut screen, view, 0, 0, &edge_key(Key::Left));
    assert_eq!(window_of(&screen), Window::Live);
    assert!(seated_elems(&out).is_empty() && reaches(&out).is_empty());
}

/// **The end of the guide's data stops the shift**: no programme runs past the window's end, so there is nothing to show.
#[test]
fn the_end_of_the_guide_data_stops_the_scroll() {
    let (mut store, mut screen) = guide();
    // nothing runs past the live window's end (90): "Late Film" now ends at 90 and no other channel is longer
    let mut lineup = guide_lineup();
    lineup[0].later = vec![later("News at Nine", 30, 60), later("Late Film", 60, 90)];
    store.install_for_test(lineup);
    step(&mut screen, store.view(), None, &ScreenEvent::StoreChanged(StoreId::LiveTv.ord(), 2));
    let (handled, out) = step_handled_at(&mut screen, store.view(), 0, 2, &edge_key(Key::Right));
    assert_eq!(handled, Handled::Yes, "the press is spent");
    assert_eq!(window_of(&screen), Window::Live, "the guide ends where the window does");
    assert!(seated_elems(&out).is_empty() && reaches(&out).is_empty());
    // one programme to 120: the window moves once, then the data has ended again
    let mut lineup = guide_lineup();
    lineup[0].later = vec![later("News at Nine", 30, 60), later("Late Film", 60, 120)];
    store.install_for_test(lineup);
    step(&mut screen, store.view(), None, &ScreenEvent::StoreChanged(StoreId::LiveTv.ord(), 3));
    step_handled_at(&mut screen, store.view(), 0, 2, &edge_key(Key::Right));
    assert_eq!(window_of(&screen), Window::Ahead(NOW + 30 * MIN));
    step_handled_at(&mut screen, store.view(), 0, 1, &edge_key(Key::Right));
    assert_eq!(window_of(&screen), Window::Ahead(NOW + 30 * MIN), "the last programme ends at the window's end: no more");
}

/// **Where the focus lands**: a row whose programme the window left is the row's cell that shares the most with it, and a row
/// with nothing in the new window is its one unscheduled cell.
#[test]
fn the_focus_lands_on_the_cell_that_overlaps_the_one_it_left() {
    let (store, mut screen) = guide_over(long_guide_lineup());
    let view = store.view();
    // 102 airs 0-60 and nothing after: one step on its cell is cut at the left edge and still the only one, which the
    // focus already stands on
    let (_, out) = step_handled_at(&mut screen, view, 1, 0, &edge_key(Key::Right));
    assert!(seated_elems(&out).is_empty(), "the same cell: nothing to move");
    assert_eq!(board(&screen).grid.cells(1), 1);
    assert!(!board(&screen).grid.is_unscheduled(1));
    // two steps on it is over: the row has nothing in the window, and its one cell is where the focus stands
    step_handled_at(&mut screen, view, 1, 0, &edge_key(Key::Right));
    assert!(board(&screen).grid.is_unscheduled(1));
    // a row the shift moves under: 104 (0-30, then 30-90) from its last cell at the live window: "Second Half" stays
    let (store, mut screen) = guide();
    let (_, out) = step_handled_at(&mut screen, store.view(), 3, 1, &edge_key(Key::Right));
    let kept = board(&screen).grid.span(Cell { row: 3, index: 0 }).map(|s| (s.begins, s.ends));
    assert_eq!(kept, Some((NOW + 30 * MIN, NOW + 90 * MIN)), "the second half is the first cell of the moved window");
    assert_eq!(seated_elems(&out), vec![cell_elem(&screen, 3, 0)]);
}

/// **The window follows the clock until it is moved**: a held window stays where the reader put it as the half hour rolls
/// over, and is the live window again once the live one reaches it.
#[test]
fn a_held_window_stays_through_the_half_hour_roll_and_goes_live_when_caught_up() {
    let (store, mut screen) = guide_over(long_guide_lineup());
    let view = store.view();
    step_handled_at(&mut screen, view, 0, 2, &edge_key(Key::Right));
    step_handled_at(&mut screen, view, 0, 1, &edge_key(Key::Right));
    assert_eq!(window_of(&screen), Window::Ahead(NOW + 60 * MIN));
    // 21:35: the live window is 21:30-23:00, the held one (21:30) is the live one
    clock(NOW + 65 * MIN);
    step(&mut screen, view, None, &ScreenEvent::Tick(Tick { ms: 16, dt_us: 16_667 }));
    assert_eq!(window_of(&screen), Window::Live, "caught up: the live window");
    assert_eq!(board(&screen).grid.axis().start(), NOW + 60 * MIN);
    // a window held further ahead stays put as the live window rolls
    clock(GUIDE_NOW);
    step(&mut screen, view, None, &ScreenEvent::Tick(Tick { ms: 24, dt_us: 16_667 }));
    step_handled_at(&mut screen, view, 0, 2, &edge_key(Key::Right));
    step_handled_at(&mut screen, view, 0, 1, &edge_key(Key::Right));
    step_handled_at(&mut screen, view, 0, 1, &edge_key(Key::Right));
    assert_eq!(window_of(&screen), Window::Ahead(NOW + 90 * MIN));
    clock(NOW + 35 * MIN);
    step(&mut screen, view, None, &ScreenEvent::Tick(Tick { ms: 32, dt_us: 16_667 }));
    assert_eq!(window_of(&screen), Window::Ahead(NOW + 90 * MIN), "the held window did not move with the clock");
}

/// **The window returns to the live one** on BACK (first), on choosing a scope again and on coming back from the player.
#[test]
fn back_a_scope_and_a_restore_return_the_window_to_live() {
    let (store, mut screen) = guide();
    let view = store.view();
    let back = ScreenEvent::Input(InputEvent { at: Tick::default(), source: plx_machine::machine::Source::Script,
        kind: InputKind::Key { key: Key::Back, sym: 0, wcode: 0, edge: Edge::Down, at_edge: false } });
    step_handled_at(&mut screen, view, 0, 2, &edge_key(Key::Right));
    assert_ne!(window_of(&screen), Window::Live);
    let (handled, out) = step_handled_at(&mut screen, view, 0, 1, &back);
    assert_eq!(handled, Handled::Yes, "BACK first backs out of the scroll");
    assert_eq!(window_of(&screen), Window::Live);
    assert!(out.iter().all(|s| !matches!(&s.fx, Fx::App(AppFx::Home(_)))), "and does not leave the page");
    // a scope chosen opens on the live window
    step_handled_at(&mut screen, view, 0, 2, &edge_key(Key::Right));
    choose(&mut screen, view, WHATS_ON_ELEM);
    choose(&mut screen, view, GUIDE_ELEM);
    assert_eq!(window_of(&screen), Window::Live);
    // and the player's BACK
    step_handled_at(&mut screen, view, 0, 2, &edge_key(Key::Right));
    step(&mut screen, view, None, &left("C101"));
    step_at(&mut screen, view, 0, 1, &ScreenEvent::Enter(Enter::Restored));
    assert_eq!(window_of(&screen), Window::Live);
}

/// **The store's reach follows the page**: asked for when the Guide shows (the window's end and a step), sent again only
/// when it changes, and released when the page leaves (a scope without a board, or the screen going).
#[test]
fn the_reach_follows_the_board_and_is_released_when_the_page_leaves() {
    let (store, mut screen) = guide();
    let view = store.view();
    assert_eq!(screen.reached, NOW + 120 * MIN, "asked for when the Guide opened");
    let out = step(&mut screen, view, None, &ScreenEvent::Tick(Tick { ms: 16, dt_us: 16_667 }));
    assert!(reaches(&out).is_empty(), "not again while it is unchanged");
    let out = step(&mut screen, view, None, &ScreenEvent::WillLeave(plx_machine::machine::Leave::Deeper));
    assert_eq!(reaches(&out), [0], "covered: released");
    let out = step(&mut screen, view, None, &ScreenEvent::WillLeave(plx_machine::machine::Leave::ForGood));
    assert!(reaches(&out).is_empty(), "once");
    let out = step(&mut screen, view, None, &ScreenEvent::Tick(Tick { ms: 32, dt_us: 16_667 }));
    assert_eq!(reaches(&out), [NOW + 120 * MIN], "a page restored asks again");
    let out = choose(&mut screen, view, WHATS_ON_ELEM);
    assert_eq!(reaches(&out), [0], "What's on shows no more than the live window");
    let out = choose(&mut screen, view, GUIDE_ELEM);
    assert_eq!(reaches(&out), [NOW + 120 * MIN]);
}

/// **A read that lands under the focus keeps it on its programme**: cells are numbered by their place in the row, so when
/// the airing ends and the next programme takes the first place the focus follows the programme, not the number.
#[test]
fn a_landing_read_keeps_the_focus_on_its_programme() {
    let (mut store, mut screen) = guide();
    let view = store.view();
    // focus on 101's second cell, "News at Nine"
    let elem = cell_elem(&screen, 0, 1);
    step(&mut screen, view, Some(elem), &ScreenEvent::FocusMoved { from: None, to: FocusKey { entry: ENTRY, elem }, by: By::Dir });
    // the airing ended: the guide's answer has the programme that was next as the one on now, and the cells renumber
    let mut lineup = guide_lineup();
    lineup[0] = guide_channel("101", 30, 60, vec![later("Late Film", 60, 120)]);
    lineup[0].now = "News at Nine".into();
    store.install_for_test(lineup);
    let out = step(&mut screen, store.view(), Some(elem), &ScreenEvent::StoreChanged(StoreId::LiveTv.ord(), 2));
    assert_eq!(seated_elems(&out), vec![cell_elem(&screen, 0, 0)], "News at Nine is now the first cell: the focus follows it");
}

// ---- Browse (PR7, SPEC-6) ---------------------------------------------------------------------------------------

use crate::registry::AppMsg;
use super::browse::Filter;

/// Three channels on air and one with no data, at [`NOW`] (20:30 UTC), with programmes after them:
/// 101 began 5 minutes ago and runs 30 (just started), with two programmes after it and a third that begins after the day's end;
/// 102 began 40 minutes ago and ends in 10 (ending soon); 103 began 20 minutes ago and ends in 40; 104 has nothing.
fn browse_lineup() -> Vec<Channel> {
    let mut a = airing("101", "One", 5, 30);
    a.later = vec![later("Next 101", 25, 85), later("Evening 101", 85, 145), later("Past midnight 101", 230, 260)];
    let b = airing("102", "Two", 40, 50);
    let mut c = airing("103", "Three", 20, 60);
    c.later = vec![later("Next 103", 40, 100)];
    let mut d = channel("104", "Four");
    d.now.clear();
    vec![a, b, c, d]
}

/// English, and the default caption policy: what the words and the label band's height are measured in.
fn browse_guards() -> (captions::PolicyPin, plx_os::i18n::ThreadLocaleGuard) {
    (captions::PolicyPin::default_setting(), plx_os::i18n::language_on_this_thread_for_test(plx_os::i18n::Preference::En))
}

/// A screen on Browse over `lineup` at [`NOW`], on the default filter.
fn browse_over(lineup: Vec<Channel>) -> (LiveTvStore, LiveTvScreen) {
    clock(NOW);
    let mut store = LiveTvStore::default();
    store.install_for_test(lineup);
    let mut screen = LiveTvScreen::new(ENTRY);
    step(&mut screen, store.view(), None, &ScreenEvent::Enter(Enter::Fresh { focus: FocusTarget::ContainerGroup(RECENT_GROUP) }));
    choose(&mut screen, store.view(), BROWSE_ELEM);
    (store, screen)
}

fn pick(screen: &mut LiveTvScreen, view: LiveTvView<'_>, filter: Filter) -> Vec<Stamped<TestHost>> {
    let out = step(screen, view, Some(FILTER_ELEM), &ScreenEvent::App(AppMsg::LiveTvFilterPicked(filter)));
    step(screen, view, Some(FILTER_ELEM), &ScreenEvent::Tick(Tick { ms: 16, dt_us: 16_667 }));
    out
}

/// OK on card `elem`: the focus arrives, then the press commits (the engine's own order).
fn press_card(screen: &mut LiveTvScreen, view: LiveTvView<'_>, elem: u32) -> Vec<Stamped<TestHost>> {
    step(screen, view, Some(elem), &ScreenEvent::FocusMoved { from: None, to: FocusKey { entry: ENTRY, elem }, by: By::Dir });
    step(screen, view, Some(elem), &ScreenEvent::PressCommit(PressId(5)))
}

fn browse_set(screen: &LiveTvScreen) -> &Browse {
    screen.page.mode.browse_set().expect("Browse is showing")
}

/// The channels' indices the grid shows, in order.
fn shown_channels(screen: &LiveTvScreen) -> Vec<usize> {
    browse_set(screen).picks.iter().map(|p| p.channel).collect()
}

/// The titles the grid shows, in order.
fn shown_titles(screen: &LiveTvScreen, view: LiveTvView<'_>) -> Vec<String> {
    browse_set(screen).picks.iter().map(|pick| {
        let channel = &view.channels[pick.channel];
        match pick.src { CellSrc::Now => channel.now.clone(), CellSrc::Later(n) => channel.later[n].title.clone() }
    }).collect()
}

/// **Each filter shows the programmes its rule admits, in channel then start order**, one card per programme.
#[test]
fn each_filter_shows_what_its_rule_admits_in_channel_then_start_order() {
    let _guards = browse_guards();
    let (store, mut screen) = browse_over(browse_lineup());
    let view = store.view();
    assert_eq!(screen.page.scope, Scope::Browse(Filter::OnNow), "On now is the default");
    assert_eq!(shown_channels(&screen), [0, 1, 2], "101, 102 and 103 are on; 104 has nothing");
    pick(&mut screen, view, Filter::JustStarted);
    assert_eq!(shown_channels(&screen), [0], "101 began 5 minutes ago");
    pick(&mut screen, view, Filter::EndingSoon);
    assert_eq!(shown_channels(&screen), [1], "102 ends in 10");
    pick(&mut screen, view, Filter::LaterToday);
    assert_eq!(shown_titles(&screen, view), ["Next 101", "Evening 101", "Next 103"], "channel then start order; the one past midnight is another day's");
}

/// **The count reads "shown of on now" while a filter narrows what is on, and not otherwise.**
#[test]
fn the_count_reads_for_the_narrowing_filters_only() {
    let _guards = browse_guards();
    let (store, mut screen) = browse_over(browse_lineup());
    let view = store.view();
    assert_eq!(screen.page.readout(), None, "On now is the whole");
    pick(&mut screen, view, Filter::JustStarted);
    assert_eq!(screen.page.readout().as_deref(), Some("1 of 3"));
    pick(&mut screen, view, Filter::EndingSoon);
    assert_eq!(screen.page.readout().as_deref(), Some("1 of 3"));
    pick(&mut screen, view, Filter::LaterToday);
    assert_eq!(screen.page.readout(), None, "Later today is another set, not a part of what is on");
    // the readout is on the header row, before the filter control
    pick(&mut screen, view, Filter::JustStarted);
    let row = screen.page.header_row(&FixtureMeasure);
    assert_eq!(row.readout(), Some("1 of 3"));
}

/// **A filter that shows nothing is an explicit state**: the empty read-out under the row, the filter control the one stop to reach.
#[test]
fn a_filter_with_no_programme_reads_out_and_leaves_the_control_the_only_stop() {
    let _guards = browse_guards();
    let (store, mut screen) = browse_over(vec![airing("101", "One", 25, 60)]);
    let view = store.view();
    pick(&mut screen, view, Filter::JustStarted);
    assert!(matches!(screen.page.mode, Mode::BrowseNone(_)));
    assert_eq!(screen.page.readout().as_deref(), Some("0 of 1"));
    let context = cx(view, Some(FILTER_ELEM));
    assert_eq!(StackPage::<TestHost>::plain_len(&screen.page, Sec::Head), 4, "the three scopes and the filter control");
    assert_eq!(StackPage::<TestHost>::plain_elem(&screen.page, Sec::Head, 3), Some(FILTER_ELEM));
    assert!(StackPage::<TestHost>::cards(&screen.page, &context, Sec::Browse).is_none(), "no grid");
    assert_eq!(
        StackPage::<TestHost>::seat_override(&screen.page, &context, Sec::Head, plx_ui::screen::Placed { rect: Rect::FULL, rest_rect: Rect::FULL, clip: Rect::FULL, index: None }),
        Some(FILTER_ELEM), "focus lands on the filter control");
    // a card that is gone: focus goes to the control
    let gone = BROWSE_FIRST_ELEM;
    assert_eq!(Focusable::<TestHost>::reconcile(&screen, FocusKey { entry: ENTRY, elem: gone }, &context).elem, FILTER_ELEM);
    pick(&mut screen, view, Filter::OnNow);
    assert!(matches!(screen.page.mode, Mode::Browse(_)));
}

/// **A card whose programme leaves the set hands its focus to its channel's card, else the control.**
#[test]
fn a_focused_card_that_leaves_the_set_is_followed_to_its_channels_card_or_the_control() {
    let _guards = browse_guards();
    let (store, mut screen) = browse_over(browse_lineup());
    let view = store.view();
    pick(&mut screen, view, Filter::JustStarted);
    let elem = browse_set(&screen).elems[0];
    // 7 minutes on, 101 is 12 minutes in: not just started any more
    clock(NOW + 7 * MIN);
    step(&mut screen, view, Some(elem), &ScreenEvent::Tick(Tick { ms: 32, dt_us: 16_667 }));
    assert!(matches!(screen.page.mode, Mode::BrowseNone(_)), "the set was rebuilt when the clock crossed 11 minutes");
    let context = cx(view, Some(elem));
    assert_eq!(Focusable::<TestHost>::reconcile(&screen, FocusKey { entry: ENTRY, elem }, &context).elem, FILTER_ELEM);
    // On now: the same programme is on, and keeps its element across the rebuild
    pick(&mut screen, view, Filter::OnNow);
    assert_eq!(browse_set(&screen).elems[0], elem, "a programme keeps its element across rebuilds");
    // the airing ends (101 at +25): the focus lands on the channel's next programme, now on
    clock(NOW + 26 * MIN);
    step(&mut screen, view, Some(elem), &ScreenEvent::Tick(Tick { ms: 48, dt_us: 16_667 }));
    let context = cx(view, Some(elem));
    let to = Focusable::<TestHost>::reconcile(&screen, FocusKey { entry: ENTRY, elem }, &context).elem;
    let titles = shown_titles(&screen, view);
    let at = browse_set(&screen).index.get(&to).copied().expect("the focus lands on a card of the set");
    assert_eq!(titles[at], "Next 101", "101's next programme took over: {titles:?}");
}

/// **OK on an airing programme tunes its channel; OK on a later one does nothing** (the Guide's rule).
#[test]
fn ok_tunes_an_airing_programme_and_ignores_a_later_one() {
    let _guards = browse_guards();
    let (store, mut screen) = browse_over(browse_lineup());
    let view = store.view();
    let tune = |channel: &str, title: &str| PlayIntent::Live { sid: plx_plex::plex::ServerId::UNSET, dvr: "7".into(), channel: channel.into(), title: title.into() };
    let first = browse_set(&screen).elems[0];
    let out = press_card(&mut screen, view, first);
    assert_eq!(asked_plays(&out), [tune("C101", "One")]);
    pick(&mut screen, view, Filter::LaterToday);
    let later = browse_set(&screen).elems[0];
    let out = press_card(&mut screen, view, later);
    assert!(asked_plays(&out).is_empty(), "a later programme has nothing to tune");
    // the clock reaches it: now it is on
    clock(NOW + 26 * MIN);
    step(&mut screen, view, Some(later), &ScreenEvent::Tick(Tick { ms: 64, dt_us: 16_667 }));
    pick(&mut screen, view, Filter::OnNow);
    let elem = browse_set(&screen).elems.iter().copied().find(|e| *e == later).expect("on now now");
    let out = press_card(&mut screen, view, elem);
    assert_eq!(asked_plays(&out), [tune("C101", "One")]);
}

/// **BACK from the player seats the card of the programme tuned from Browse.**
#[test]
fn back_from_the_player_seats_the_browse_card_the_tune_came_from() {
    let _guards = browse_guards();
    let (store, mut screen) = browse_over(browse_lineup());
    let view = store.view();
    let card = browse_set(&screen).elems[2];
    let out = press_card(&mut screen, view, card);
    assert_eq!(asked_plays(&out).len(), 1, "tuned from the card of 103");
    assert!(step(&mut screen, view, None, &left("C103")).is_empty());
    let out = step(&mut screen, view, Some(card), &ScreenEvent::Enter(Enter::Restored));
    assert_eq!(seated_elems(&out), vec![card], "the same programme's card");
    // while the player was up the programme ended and the next one began: the channel's card that stands there
    pick(&mut screen, view, Filter::LaterToday);
    let next = browse_set(&screen).elems[2];
    pick(&mut screen, view, Filter::OnNow);
    clock(NOW + 41 * MIN);
    step(&mut screen, view, None, &left("C103"));
    let out = step(&mut screen, view, Some(card), &ScreenEvent::Enter(Enter::Restored));
    assert_eq!(seated_elems(&out), vec![next], "103's next programme is on, and its card is the one seated");
}

/// **A programme keeps its element while the lineup around it changes**, and no element names two programmes.
#[test]
fn a_programme_keeps_its_element_across_refreshes_and_no_element_names_two() {
    let _guards = browse_guards();
    let (mut store, mut screen) = browse_over(browse_lineup());
    let before = browse_set(&screen).elems.clone();
    let mut lineup = browse_lineup();
    lineup.insert(0, airing("100", "Zero", 3, 60));
    store.install_for_test(lineup);
    step(&mut screen, store.view(), None, &ScreenEvent::StoreChanged(StoreId::LiveTv.ord(), 2));
    let elems = &browse_set(&screen).elems;
    assert_eq!(elems.len(), 4);
    let mut unique = elems.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), elems.len(), "distinct");
    assert_eq!(&elems[1..], before.as_slice(), "the three programmes already known stand on the elements they had");
}

/// **Every airing is reachable**: a lineup of seven hundred channels is seven hundred cards, each with its own element
/// that finds its card back (no cap on what can be reached; the grid pages what it draws).
#[test]
fn a_lineup_of_seven_hundred_channels_is_seven_hundred_reachable_cards() {
    let _guards = browse_guards();
    let lineup: Vec<Channel> = (0..700).map(|n| airing(&n.to_string(), &format!("C{n}"), 5, 60)).collect();
    let (store, screen) = browse_over(lineup);
    let set = browse_set(&screen);
    assert_eq!(set.picks.len(), 700);
    let context = cx(store.view(), None);
    let last = *set.elems.last().expect("a last card");
    let cards = StackPage::<TestHost>::cards(&screen.page, &context, Sec::Browse).expect("the grid");
    assert_eq!(CardSource::<TestHost>::len(&cards), 700);
    assert_eq!(CardSource::<TestHost>::index_of(&cards, &last), Some(699));
    assert!(Focusable::<TestHost>::group_of(&screen, &last, &context).is_some(), "the last card is in the grid's group");
}

/// **The filter control opens the menu** carrying the filter in force; the menu reports its pick and the page applies it.
#[test]
fn ok_on_the_filter_control_asks_for_the_menu_and_a_pick_changes_the_filter() {
    let _guards = browse_guards();
    let (store, mut screen) = browse_over(browse_lineup());
    let view = store.view();
    pick(&mut screen, view, Filter::EndingSoon);
    let out = step(&mut screen, view, Some(FILTER_ELEM), &ScreenEvent::PressCommit(PressId(3)));
    let panel = out.iter().find_map(|s| match &s.fx {
        Fx::App(AppFx::Content(ContentReq::Panel(ContentPanel::LiveTvFilter { chosen, opener, .. }))) => Some((*chosen, *opener)),
        _ => None,
    });
    assert_eq!(panel, Some((Filter::EndingSoon, (ENTRY, FILTER_ELEM))), "the menu opens on the filter in force, from the control");
    // a pick for the filter already in force is spent
    let same = pick(&mut screen, view, Filter::EndingSoon);
    assert!(reaches(&same).is_empty());
    assert_eq!(screen.page.scope, Scope::Browse(Filter::EndingSoon));
    // while the Guide shows, a late pick from the menu changes nothing
    choose(&mut screen, view, GUIDE_ELEM);
    step(&mut screen, view, None, &ScreenEvent::App(AppMsg::LiveTvFilterPicked(Filter::LaterToday)));
    assert_eq!(screen.page.scope, Scope::Guide, "a pick that finds no Browse is spent");
}

/// **Later today asks the store to read to the end of the day**, through the Guide's own reach path; leaving Browse lets go.
#[test]
fn later_today_reads_to_the_end_of_the_local_day_and_leaving_lets_go() {
    let _guards = browse_guards();
    let (store, mut screen) = browse_over(browse_lineup());
    let view = store.view();
    let day_end = browse::day_end(NOW, Some(0));
    let out = pick(&mut screen, view, Filter::LaterToday);
    assert_eq!(reaches(&out), [day_end]);
    let out = pick(&mut screen, view, Filter::OnNow);
    assert_eq!(reaches(&out), [0], "the other filters read nothing more");
    pick(&mut screen, view, Filter::LaterToday);
    let out = choose(&mut screen, view, WHATS_ON_ELEM);
    assert_eq!(reaches(&out), [0], "What's on needs no more than the live window");
}

/// **The canon says the filter**, so a replay that differs in it differs in state.
#[test]
fn the_canon_carries_the_filter() {
    let _guards = browse_guards();
    let (store, mut screen) = browse_over(browse_lineup());
    let canon = |screen: &LiveTvScreen| { let mut c = Canon::new(); screen.write(&mut c); c.finish() };
    let on_now = canon(&screen);
    pick(&mut screen, store.view(), Filter::EndingSoon);
    assert_ne!(canon(&screen), on_now);
}

/// **The grid's geometry** (SPEC-6): six 250 x 375 posters to a row on the 280 pitch from y 244, and the next row under the focused row's
/// label band. The board draws that at y 735 (619 + the 92 label block + 24); the shipped grids' two-line band is
/// `caption_policy::BAND_TWO_LINES` (112, the label block and its air), so the row stands at 731, as the Library's does: the one band, not a Browse copy of it.
#[test]
fn the_grid_is_six_posters_a_row_on_the_280_pitch_from_244_and_the_next_row_peeks_under_the_label_band() {
    let _guards = browse_guards();
    let lineup: Vec<Channel> = (0..14).map(|n| airing(&n.to_string(), &format!("C{n}"), 5, 60)).collect();
    let (store, mut screen) = browse_over(lineup);
    let view = store.view();
    let elems = browse_set(&screen).elems.clone();
    let at = |screen: &LiveTvScreen, focus: Option<u32>, elem: u32| {
        Focusable::<TestHost>::place(screen, &elem, &cx(view, focus), At::Drawn).expect("placed").rect
    };
    for frame in 1..=90 {
        step(&mut screen, view, None, &ScreenEvent::Tick(Tick { ms: frame * 16, dt_us: 16_667 }));
    }
    let row: Vec<Rect> = elems[..7].iter().map(|e| at(&screen, None, *e)).collect();
    for (i, rect) in row[..6].iter().enumerate() {
        assert!(near(rect.x, 96.0 + 280.0 * i as f32) && near(rect.y, 244.0) && near(rect.w, 250.0) && near(rect.h, 375.0), "{i}: {rect:?}");
    }
    assert!(near(row[6].x, 96.0) && row[6].y > 244.0, "the seventh poster begins the next row: {:?}", row[6]);
    // with the first poster focused the next row stands 24 under the label band
    let first = elems[0];
    step(&mut screen, view, Some(first), &ScreenEvent::FocusMoved { from: None, to: FocusKey { entry: ENTRY, elem: first }, by: By::Dir });
    for frame in 1..=90 {
        step(&mut screen, view, Some(first), &ScreenEvent::Tick(Tick { ms: 2000 + frame * 16, dt_us: 16_667 }));
    }
    let next = at(&screen, Some(first), elems[6]);
    assert!(near(next.y, 619.0 + plx_ui::caption_policy::BAND_TWO_LINES), "the next row starts {} (619 + the shipped two-line band)", next.y);
}

/// **The grid's moves**: RIGHT and LEFT along a row, DOWN and UP between rows, UP from the first row to the header row (the filter control's
/// neighbours on the row: the scopes to its left), and the row rides the page as the Library's does (it scrolls away with the grid).
#[test]
fn the_grid_moves_along_and_between_rows_and_up_from_the_first_row_reaches_the_header() {
    let lineup: Vec<Channel> = (0..14).map(|n| airing(&n.to_string(), &format!("C{n}"), 5, 60)).collect();
    let (store, mut screen) = browse_over(lineup);
    let view = store.view();
    let elems = browse_set(&screen).elems.clone();
    let go = |screen: &LiveTvScreen, from: u32, dir: Dir| move_from(screen, view, from, dir);
    assert_eq!(go(&screen, elems[0], Dir::Right), Some(elems[1]));
    assert_eq!(go(&screen, elems[1], Dir::Left), Some(elems[0]));
    assert_eq!(go(&screen, elems[0], Dir::Down), Some(elems[6]), "a row down, the same column");
    assert_eq!(go(&screen, elems[6], Dir::Up), Some(elems[0]));
    assert_eq!(go(&screen, elems[0], Dir::Up), None, "the grid's own edge: the link decides");
    let mut links = Vec::new();
    Screen::<TestHost>::links(&screen, &mut links);
    assert!(links.contains(&Link { from: BROWSE_GROUP, dir: Dir::Up, to: HEAD_GROUP }), "up from the first row is the header row");
    assert!(links.contains(&Link { from: HEAD_GROUP, dir: Dir::Down, to: BROWSE_GROUP }));
    // the header row's own walk: the scopes, then the filter control as its last stop
    assert_eq!(move_from(&screen, view, GUIDE_ELEM, Dir::Right), Some(BROWSE_ELEM));
    assert_eq!(move_from(&screen, view, BROWSE_ELEM, Dir::Right), Some(FILTER_ELEM));
    assert_eq!(move_from(&screen, view, FILTER_ELEM, Dir::Right), None);
    // the grid scrolls a row and the header row goes up with it
    let header = |screen: &LiveTvScreen| Focusable::<TestHost>::place(screen, &FILTER_ELEM, &cx(view, None), At::Drawn).expect("the control is placed").rect;
    for frame in 1..=60 {
        step(&mut screen, view, None, &ScreenEvent::Tick(Tick { ms: frame * 16, dt_us: 16_667 }));
    }
    let at_rest = header(&screen);
    let second = elems[6];
    step(&mut screen, view, Some(second), &ScreenEvent::FocusMoved { from: None, to: FocusKey { entry: ENTRY, elem: second }, by: By::Dir });
    for frame in 1..=90 {
        step(&mut screen, view, Some(second), &ScreenEvent::Tick(Tick { ms: 2000 + frame * 16, dt_us: 16_667 }));
    }
    assert!(screen.stack.scroll() > 100.0, "the grid scrolled a row: {}", screen.stack.scroll());
    assert!(header(&screen).y < at_rest.y - 100.0, "the header row rides the page: {:?} from {at_rest:?}", header(&screen));
    let rect = Focusable::<TestHost>::place(&screen, &second, &cx(view, Some(second)), At::Drawn).expect("placed").rect;
    assert!(rect.y >= 0.0 && rect.y + rect.h <= SCR_H, "the focused row stands inside the screen: {rect:?}");
}

/// Scroll Browse's grid to the second row: focus its first card and let the page settle.
fn scrolled_to_second_row(screen: &mut LiveTvScreen, view: LiveTvView<'_>) -> Vec<u32> {
    let elems = browse_set(screen).elems.clone();
    let second = elems[6];
    step(screen, view, Some(second), &ScreenEvent::FocusMoved { from: None, to: FocusKey { entry: ENTRY, elem: second }, by: By::Dir });
    for frame in 1..=120 {
        step(screen, view, Some(second), &ScreenEvent::Tick(Tick { ms: frame * 16, dt_us: 16_667 }));
    }
    assert!(screen.stack.scroll() > 100.0, "the grid scrolled a row: {}", screen.stack.scroll());
    elems
}

/// A Browse lineup of fourteen channels on air: three rows of the six-wide grid.
fn browse_fourteen() -> (LiveTvStore, LiveTvScreen) {
    browse_over((0..14).map(|n| airing(&n.to_string(), &format!("C{n}"), 5, 60)).collect())
}

/// **Browse's header row rides the page as the Library's does** (owner decision 2026-11-27): scrolled to the second row the row has
/// left with the page and the top bar has tucked through the one `Handoff`, as on What's on. Red while the row stood on a mask and
/// the bar stayed.
#[test]
fn the_header_row_scrolls_away_with_browses_grid_and_the_top_bar_tucks_with_it() {
    let _guards = browse_guards();
    let (store, mut screen) = browse_fourteen();
    let view = store.view();
    let header = |screen: &LiveTvScreen| Focusable::<TestHost>::place(screen, &FILTER_ELEM, &cx(view, None), At::Drawn).expect("the control is placed").rect;
    let at_rest = header(&screen);
    assert_eq!(Screen::<TestHost>::chrome_tuck(&screen), 0.0, "at the top: the chrome shown");
    scrolled_to_second_row(&mut screen, view);
    assert!(header(&screen).y < at_rest.y - 100.0, "the row went up with the page: {:?} from {at_rest:?}", header(&screen));
    assert_eq!(Screen::<TestHost>::chrome_tuck(&screen), 1.0, "row two: the bar has tucked with the row");
}

/// **UP from the first row reaches the filter and the scope row, and the page scrolls back** as the Library's does.
#[test]
fn focus_on_the_header_row_scrolls_the_grid_home() {
    let _guards = browse_guards();
    let (store, mut screen) = browse_fourteen();
    let view = store.view();
    scrolled_to_second_row(&mut screen, view);
    step(&mut screen, view, Some(FILTER_ELEM), &ScreenEvent::FocusMoved { from: None, to: FocusKey { entry: ENTRY, elem: FILTER_ELEM }, by: By::Dir });
    for frame in 1..=240 {
        step(&mut screen, view, Some(FILTER_ELEM), &ScreenEvent::Tick(Tick { ms: 3000 + frame * 16, dt_us: 16_667 }));
    }
    assert!(screen.stack.scroll() < 1.0, "the page went home to show the row: {}", screen.stack.scroll());
    assert_eq!(Screen::<TestHost>::chrome_tuck(&screen), 0.0, "and the bar is back");
}

/// **A filter picked dissolves the header's words instead of cutting them** (the row's `HeaderSwap`, as the Watchlist's):
/// the row rebuilt for a new filter and count begins a swap that runs out. Red while the row was built with no swap.
#[test]
fn a_filter_pick_swaps_the_header_words_and_the_swap_runs_out() {
    let _guards = browse_guards();
    let (store, mut screen) = browse_over(browse_lineup());
    let view = store.view();
    screen.page.header_row(&FixtureMeasure);
    assert!(!screen.page.header_swap.borrow().is_active(), "a row first built has nothing to swap from");
    pick(&mut screen, view, Filter::JustStarted);
    screen.page.header_row(&FixtureMeasure);
    assert!(screen.page.header_swap.borrow().is_active(), "the filter word and the count dissolve");
    for frame in 1..=120 {
        step(&mut screen, view, Some(FILTER_ELEM), &ScreenEvent::Tick(Tick { ms: frame * 16, dt_us: 16_667 }));
    }
    assert!(!screen.page.header_swap.borrow().is_active(), "and the swap ends");
}

/// **A programme's caption is one composition wherever a poster shows it**: Browse's card and What's on's On now card word the same
/// programme alike, a long channel name included, whole and in order on one line (the line a band cannot hold is the shared
/// caption's to glide from its head, not Browse's to cut).
#[test]
fn a_programme_caption_is_one_composition_on_browse_and_on_now() {
    let _guards = browse_guards();
    let mut long = airing("2006", "Pluto TV Star Trek: The Next Generation", 49, 60);
    long.now = "Star Trek: The Next Generation".into();
    let (store, mut screen) = browse_over(vec![long]);
    let view = store.view();
    WALL.with(|w| w.set(Some(Wall::from_secs(NOW, 0))));
    let words = |label: &TileLabel| (label.title.as_ref().map(|t| t.to_str().unwrap().to_owned()), label.caption.as_ref().map(|c| c.to_str().unwrap().to_owned()));
    let context = cx(view, None);
    let browse = StackPage::<TestHost>::cards(&screen.page, &context, Sec::Browse).unwrap();
    let on_browse = words(&CardSource::<TestHost>::label(&browse, 0));
    assert_eq!(on_browse, (Some("Star Trek: The Next Generation".into()), Some("Ch 2006 \u{b7} Pluto TV Star Trek: The Next Generation \u{b7} 11m left".into())));
    choose(&mut screen, view, WHATS_ON_ELEM);
    let context = cx(view, None);
    let now = StackPage::<TestHost>::cards(&screen.page, &context, Sec::Now).unwrap();
    assert_eq!(words(&CardSource::<TestHost>::label(&now, 0)), on_browse, "the same words as On now's");
}
