//! Conformance mounts of the real What's on page: its On now shelf (the long row, which scrolls), its
//! Recent shelf (five tiles at most, which never does), both with the other shelf standing over or under
//! them as on the page, and its Channels shelf (a lineup with no guide data, the page's only row).
use super::*;
use plx_data::livetv::MAX_RECENT;
use plx_machine::machine::{FocusRead, Host, InputOwner, PressRead};
use plx_ui::cards::conformance::{CardHarness, Landing, Nb};
use plx_ui::fixture::FixtureMeasure;
use plx_ui::screen::{At, By, Dir, Placed, Step};

struct LiveHost;
impl Host for LiveHost {
    type Arg = crate::registry::SettingsPage;
    type Fx = AppFx;
    type Msg = crate::registry::AppMsg;
    type Elem = u32;
    type Views<'a> = LiveTvView<'a>;
    type Init = crate::registry::NoInit;
    type Memory = crate::registry::PageMemory;
}
impl LiveTvLike for LiveHost {
    fn livetv<'a>(cx: &Cx<'a, Self>) -> LiveTvView<'a> { cx.views }
}
const ENTRY: EntryId = EntryId(9);
/// The shelves a mount can put under test: Recent's and the Channels shelf's cards are both plates.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shelf { Now, Recent, Channels }
struct Harness {
    store: plx_data::stores::livetv::LiveTvStore,
    /// The profile's Recent row: the first channels of the lineup, at most [`MAX_RECENT`].
    recent: Vec<plx_data::livetv::Channel>,
    /// The shelf this mount puts under test.
    row: Shelf,
    screen: LiveTvScreen,
    focus: Option<FocusKey<u32>>,
    press: f32,
    ms: u32,
    n: usize,
}
/// The On now shelf of an `n`-channel lineup, a Recent row of its first five above it.
pub(crate) fn mount(n: usize) -> Box<dyn CardHarness> { Box::new(Harness::new(n, Shelf::Now)) }
/// The Recent shelf of an `n`-channel lineup (at most five tiles), On now below it.
pub(crate) fn mount_recent(n: usize) -> Box<dyn CardHarness> { Box::new(Harness::new(n, Shelf::Recent)) }
/// The Channels shelf of an `n`-channel lineup with no programme and nothing recent: the page's only row.
pub(crate) fn mount_channels(n: usize) -> Box<dyn CardHarness> { Box::new(Harness::new(n, Shelf::Channels)) }

pub(crate) fn livetv_conforms() -> plx_ui::a11y::Namespaces {
    use plx_ui::a11y::{conform, A11yPath};
    let _guard = plx_base::testlock::serial();
    let h = Harness::new(3, Shelf::Recent);
    let cx = h.cx();
    let namespace = conform::<LiveHost, _>(&h.screen, &cx);
    for section in ["recent", "on_now"] {
        let path = A11yPath::new().name(section).name("card").index(0);
        let elem = h.screen.a11y_find(&path, &cx).expect("the first card in each shelf");
        let card = h.screen.a11y(&elem, &cx).expect("the card projects accessibility attributes");
        let subject = format!("live-{}-6c69766574763a313a6330", plx_plex::plex::ServerId::UNSET.raw());
        assert_eq!(card.subject.as_deref(), Some(subject.as_str()));
    }
    vec![namespace]
}

impl Harness {
    fn new(n: usize, row: Shelf) -> Self {
        let mut store = plx_data::stores::livetv::LiveTvStore::default();
        // the Channels shelf is a lineup with no guide data and nothing tuned
        let programmes = row != Shelf::Channels;
        store.install_for_test((0..n).map(|i| plx_data::livetv::Channel {
            sid: plx_plex::plex::ServerId::UNSET, dvr: "1".into(), channel: format!("c{i}"),
            vcn: i.to_string(), name: format!("Channel {i}"), thumb: String::new(),
            airings: if programmes {
                plx_data::livetv::Airings::on_then(plx_data::livetv::LiveAiring::new(format!("Programme {i}"), 0, 0), Vec::new())
            } else { plx_data::livetv::Airings::default() },
        }).collect());
        let recent = if programmes { store.view().channels.iter().take(MAX_RECENT).cloned().collect() } else { Vec::new() };
        let mut h = Self { store, recent, row, screen: LiveTvScreen::new(ENTRY), focus: None, press: 1.0, ms: 0, n };
        h.step(ScreenEvent::Tick(Tick { ms: 0, dt_us: 16_667 }));
        h
    }
    fn view(&self) -> LiveTvView<'_> { self.store.view().for_test_with_recent(&self.recent) }
    fn cx(&self) -> Cx<'_, LiveHost> {
        Cx { views: self.view(), tick: Tick { ms: self.ms, dt_us: 16_667 }, measure: &FixtureMeasure,
            press: PressRead { scale: self.press, owner: self.focus, ..Default::default() },
            modal: Default::default(), page_image: Default::default(), hold_hint: Default::default(),
            focus: FocusRead { current: self.focus, ..Default::default() }, owner: InputOwner::Entry(ENTRY) }
    }
    fn key(&self, elem: u32) -> FocusKey<u32> { FocusKey { entry: ENTRY, elem } }
    fn step(&mut self, ev: ScreenEvent<LiveHost>) -> bool {
        let mut present = plx_machine::present::Present::new();
        let mut out = Vec::new();
        let (_, moving) = plx_machine::motion::report::scoped_motion(|| {
            let cx = Cx { views: self.store.view().for_test_with_recent(&self.recent), tick: Tick { ms: self.ms, dt_us: 16_667 }, measure: &FixtureMeasure,
                press: PressRead { scale: self.press, owner: self.focus, ..Default::default() },
                modal: Default::default(), page_image: Default::default(), hold_hint: Default::default(),
                focus: FocusRead { current: self.focus, ..Default::default() }, owner: InputOwner::Entry(ENTRY) };
            let mut fx = Effects::new(&mut out, plx_machine::machine::MachineId::Instance(plx_machine::machine::InstanceId(9)), &mut present);
            Machine::<LiveHost>::step(&mut self.screen, &ev, &cx, &mut fx);
        });
        moving || present.page_moving()
    }
}
impl CardHarness for Harness {
    fn cards(&self) -> Vec<u32> {
        let (n, odd) = match self.row { Shelf::Now => (self.n, 1), Shelf::Recent => (self.n.min(MAX_RECENT), 0), Shelf::Channels => (self.n, 0) };
        (0..n).map(|i| FIRST_CARD_ELEM + i as u32 * 2 + odd).collect()
    }
    fn focused(&self) -> Option<u32> { self.focus.map(|k| k.elem) }
    fn focus(&mut self, elem: u32, by: By) {
        let from = self.focus;
        self.focus = Some(self.key(elem));
        self.step(ScreenEvent::FocusMoved { from, to: self.key(elem), by });
    }
    fn tick(&mut self, frames: u32) -> bool {
        let mut moving = false;
        for _ in 0..frames { self.ms += 16; moving |= self.step(ScreenEvent::Tick(Tick { ms: self.ms, dt_us: 16_667 })); }
        moving
    }
    fn cover(&mut self) { self.step(ScreenEvent::Cover); }
    fn uncover(&mut self) { self.step(ScreenEvent::Uncover); }
    fn set_press(&mut self, scale: f32) { self.press = scale; }
    fn neighbour(&self, elem: u32, dir: Dir) -> Nb {
        match Focusable::<LiveHost>::neighbour(&self.screen, self.key(elem), dir, &self.cx()) {
            Step::Move(k) => Nb::To(k.elem), Step::Edge => Nb::Edge,
        }
    }
    fn place(&self, elem: u32, at: At) -> Option<Placed> { Focusable::<LiveHost>::place(&self.screen, &elem, &self.cx(), at) }
    fn drawn_rect(&self, elem: u32, press: f32) -> Option<Rect> {
        let cx = Cx { press: PressRead { scale: press, owner: self.focus, ..Default::default() }, ..self.cx() };
        let mut frame = DrawFrame::new(&cx, plx_ui::Painter::root());
        self.screen.stack.view(&self.screen.page).record_stops(&mut frame);
        frame.stops().iter().find(|s| s.key.elem == elem).map(|s| s.rect)
    }
    fn scale(&self, elem: u32) -> Option<f32> { self.screen.stack.view(&self.screen.page).scale_of(&self.cx(), &elem) }
    fn focus_scale(&self) -> f32 { NOW_STYLE.focus_scale }
    fn canon(&self) -> u64 { self.screen.hash() }
    fn identity(&self, elem: u32) -> String { format!("c{}", (elem - FIRST_CARD_ELEM) / 2) }
    fn scroll(&self) -> Option<f32> { Some(self.screen.stack.scroll()) }
    fn landing(&mut self, _landing: Landing) -> Result<(), &'static str> { Err("channel lineup uses guide order") }
    fn memory_roundtrip(&mut self) -> Result<Box<dyn CardHarness>, &'static str> {
        let mut fresh = Self::new(self.n, self.row);
        let elem = self.focus.ok_or("nothing focused")?.elem;
        fresh.focus(elem, By::Restore);
        Ok(Box::new(fresh))
    }
}

/// Where the two rows stand and how focus travels between them (SPEC-1, SPEC-2), read through the same
/// placements the focus engine navigates by, and moved by the engine itself.
mod geometry {
    use super::*;
    use plx_ui::focus::{FocusEngine, Outcome};

    fn settled(n: usize) -> Harness {
        let mut h = Harness::new(n, Shelf::Now);
        h.tick(60);
        h
    }
    fn recent(i: u32) -> u32 { FIRST_CARD_ELEM + i * 2 }
    fn poster(i: u32) -> u32 { FIRST_CARD_ELEM + i * 2 + 1 }
    fn drawn(h: &Harness, elem: u32) -> Rect {
        Focusable::<LiveHost>::place(&h.screen, &elem, &h.cx(), At::Drawn).expect("the card is placed").rect
    }
    /// The card's settled rect: the column it stands in, whatever its pop is doing.
    fn rest(h: &Harness, elem: u32) -> Rect {
        Focusable::<LiveHost>::place(&h.screen, &elem, &h.cx(), At::SpringTarget).expect("the card is placed").rest_rect
    }
    /// Where the focus ENGINE takes focus from `from` on `dir` (the page's links and groups, the
    /// geometry of the cards as drawn), or `None` when it stays put.
    fn engine_move(h: &mut Harness, from: u32, dir: Dir) -> Option<u32> {
        h.focus(from, By::Dir);
        h.tick(60);
        let cx = h.cx();
        let owner = InputOwner::Entry(ENTRY);
        let mut engine = FocusEngine::new();
        let group = Focusable::<LiveHost>::group_of(&h.screen, &from, &cx);
        engine.set(owner, h.key(from), group, By::Dir);
        let mut links = Vec::new();
        Screen::<LiveHost>::links(&h.screen, &mut links);
        match engine.move_dir(owner, &h.screen as &dyn Focusable<LiveHost>, &links, dir, &cx) {
            Outcome::Moved { to, .. } => Some(to.elem),
            _ => None,
        }
    }

    #[test]
    fn down_from_a_recent_tile_lands_straight_below_and_up_returns_to_the_same_column() {
        let mut h = settled(8);
        for i in 0..5 {
            assert_eq!(engine_move(&mut h, recent(i), Dir::Down), Some(poster(i)), "column {i}");
            assert_eq!(engine_move(&mut h, poster(i), Dir::Up), Some(recent(i)), "column {i}");
        }
        h.focus(poster(2), By::Dir);
        h.tick(60);
        assert_eq!(drawn(&h, recent(1)).x - drawn(&h, recent(0)).x, 280.0, "Recent keeps Home's pitch");
        assert_eq!(rest(&h, recent(2)).x, rest(&h, poster(2)).x, "one pitch, so one column");
    }

    #[test]
    fn up_from_a_poster_past_the_last_recent_tile_goes_to_the_nearest_one() {
        let mut h = settled(8);
        h.recent.truncate(3);
        h.screen.page.synced = None;
        h.tick(1);
        for i in 3..6 {
            assert_eq!(engine_move(&mut h, poster(i), Dir::Up), Some(recent(2)), "column {i}");
        }
    }

    /// The handoff's y positions at 1920 x 1080: Recent's tiles at 300 (140 tall) at a 280 pitch, On now's
    /// heading at 488 with its posters 56 under it, and 552 / 608 while a Recent tile has focus.
    #[test]
    fn the_rows_stand_where_the_handoff_puts_them_and_the_second_makes_room_for_a_label() {
        let mut h = settled(8);
        h.focus(poster(5), By::Dir);
        h.tick(120);
        let tile = drawn(&h, recent(1));
        assert_eq!((tile.y, tile.h, tile.w), (300.0, 140.0, 250.0));
        assert_eq!(drawn(&h, poster(0)).y, 544.0, "heading 488, posters 56 under it");
        h.focus(recent(2), By::Dir);
        h.tick(120);
        assert_eq!(drawn(&h, poster(0)).y, 608.0, "heading 552 while a Recent tile has focus: its label has room");
        h.focus(poster(5), By::Dir);
        h.tick(120);
        assert_eq!(drawn(&h, poster(0)).y, 544.0, "and back");
        assert_eq!(h.scroll(), Some(0.0), "nothing scrolls vertically");
    }

    /// Where a shelf's heading is INKED (its cap top): the shelf's tiles less the heading the page gives it.
    fn heading_ink(h: &Harness, k: Sec) -> f32 {
        let cx = h.cx();
        let frame = h.screen.stack.shelf_frame(&h.screen.page, &cx, k).expect("the shelf is laid out");
        let mut specs = Vec::new();
        StackPage::<LiveHost>::sections(&h.screen.page, &cx, &mut specs);
        let Some(Kind::Shelf { heading, .. }) = specs.iter().find(|s| s.key == k).map(|s| s.kind) else { panic!("{k:?} is not a shelf") };
        frame.y - heading
    }

    /// The handoff's heading lines are 40 px boxes at 244 (Recent) and 488 (On now), 552 while a Recent tile has
    /// focus, with the 32 px type centred in the box: its cap top, where a shelf here draws, is 8 below each
    /// (the fixture's caps and the shipped face's both give 8, `heading_ink_drop`). The tiles stand where the
    /// board has them, 48 under the inked heading.
    #[test]
    fn the_headings_are_inked_where_the_board_centres_them() {
        let mut h = settled(8);
        h.focus(poster(5), By::Dir);
        h.tick(120);
        assert_eq!(heading_ink(&h, Sec::Recent), 252.0);
        assert_eq!(heading_ink(&h, Sec::Now), 496.0, "488 + 8 at rest");
        h.focus(recent(2), By::Dir);
        h.tick(120);
        assert_eq!(heading_ink(&h, Sec::Now), 560.0, "552 + 8 with a Recent tile focused");
    }
}
