//! `ui/menu.rs`: every transition of the machine, and the proof that the illegal combinations are not
//! values. Table in the module doc; one test per row of it.

use super::menu::*;
use crate::fixture::FixtureMeasure as M;
use crate::form::{Form, FormId, FormSection, RowKey, RowKind};
use crate::page_stack::TITLE_KEY;
use crate::table::Row;
use crate::Rect;
use plx_machine::machine::EntryId;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Id {
    Go,
    Other,
    Off,
    Leaf(u8),
    Deep,
}
impl FormId for Id {
    fn key(&self) -> RowKey {
        RowKey(match self {
            Id::Go => 1,
            Id::Other => 2,
            Id::Off => 3,
            Id::Leaf(n) => 10 + u32::from(*n),
            Id::Deep => 4,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pg {
    One,
    Two,
}
type Mn = Menu<Pg, Id, &'static str>;
type Pgs = Page<Id, &'static str, Pg>;

fn pill() -> Rect {
    Rect::new(1200.0, 800.0, 120.0, 60.0)
}

fn anchor() -> [u32; 4] {
    [100.0f32, 300.0, 200.0, 300.0].map(f32::to_bits)
}

fn grows() -> Host {
    Host::Grows { pill: pill(), clear_y: None, fill: Some(crate::theme::ACCENT), morph: Morph::FromPill }
}

/// The root: a Nav row into page One, a plain action, a disabled one.
fn root() -> Pgs {
    Page::root(Form::new().section(
        FormSection::new("")
            .item(Id::Go, RowKind::Nav(Pg::One), "go", Row::new("Go"))
            .item(Id::Other, RowKind::Button, "other", Row::new("Other"))
            .item(Id::Off, RowKind::Button, "off", Row::new("Off")).disabled(true),
    ))
}

fn leaves(n: u8, deep: bool) -> Form<Id, &'static str, Pg> {
    let mut sec = FormSection::new("");
    for i in 0..n {
        sec = sec.item(Id::Leaf(i), RowKind::Choice, "leaf", Row::new(format!("Leaf {i}")));
    }
    if deep {
        sec = sec.item(Id::Deep, RowKind::Nav(Pg::Two), "deep", Row::new("Deeper"));
    }
    Form::new().section(sec)
}

/// The host's builder: root for `None`, two pages for the others; `gone` makes page Two unbuildable.
fn builder(gone: bool) -> impl FnMut(Option<Pg>) -> Option<Pgs> {
    move |p| match p {
        None => Some(root()),
        Some(Pg::One) => Some(Page::new(leaves(3, true), "One").initial(Some(Id::Leaf(1)))),
        Some(Pg::Two) => (!gone).then(|| Page::new(leaves(2, false), "Two")),
    }
}

fn menu(host: Host, class: Class) -> Mn {
    let mut m = Mn::new(host, class, crate::table_screen::BAND_BASE);
    m.step(Event::ReplaceRoot { slide: None }, &mut builder(false));
    m
}

fn at(m: &Mn) -> Option<u32> {
    m.form().selected_key().map(|k| k.0)
}

/// Settle the motion: tick until it rests.
fn settle(m: &mut Mn) {
    for _ in 0..600 {
        m.step(Event::Tick { dt: 1.0 / 60.0, measure: &M, focus: None }, &mut builder(false));
        if m.state().motion == Motion::Rest {
            return;
        }
    }
    panic!("the motion never settled: {:?}", m.state());
}

fn step(m: &mut Mn, ev: Event<'_, Id>) -> Out<&'static str> {
    m.step(ev, &mut builder(false))
}

#[test]
fn a_new_menu_is_at_the_root_and_at_rest() {
    let m = menu(grows(), Class::Action);
    assert_eq!(m.state(), State { stack: Stack::Root, motion: Motion::Rest });
    assert_eq!(m.header(), None, "the root has no header");
    assert_eq!(m.depth(), 0);
}

#[test]
fn ok_on_a_nav_row_pushes_its_page_and_the_header_appears() {
    let mut m = menu(grows(), Class::Action);
    assert_eq!(step(&mut m, Event::Ok(Id::Go.key().0)), Out::Moved);
    assert_eq!((m.top(), m.depth()), (Some(Pg::One), 1));
    assert_eq!(m.header(), Some("One"), "a pushed page wears its title: there is no page without one");
    assert_eq!(m.state().stack, Stack::Paged(1));
    assert_eq!(m.state().motion, Motion::Sliding, "the pages slide");
    assert_eq!(at(&m), Some(Id::Leaf(1).key().0), "focus opens on the page's initial row");
}

#[test]
fn ok_on_any_other_row_picks_its_action_and_moves_nothing() {
    let mut m = menu(grows(), Class::Action);
    assert_eq!(step(&mut m, Event::Ok(Id::Other.key().0)), Out::Picked("other"));
    assert_eq!(m.state().stack, Stack::Root);
}

#[test]
fn ok_on_a_disabled_row_does_nothing_and_on_no_row_closes() {
    let mut m = menu(grows(), Class::Action);
    assert_eq!(step(&mut m, Event::Ok(Id::Off.key().0)), Out::Stay);
    assert_eq!(step(&mut m, Event::Right(Id::Off.key().0)), Out::Stay);
    assert_eq!(step(&mut m, Event::Ok(999)), Out::Dismiss, "a key that names no row only closes");
    assert_eq!(step(&mut m, Event::Right(999)), Out::Stay, "RIGHT on nothing is inert");
}

#[test]
fn right_enters_a_nav_row_and_is_inert_on_any_other() {
    let mut m = menu(grows(), Class::Action);
    assert_eq!(step(&mut m, Event::Right(Id::Other.key().0)), Out::Stay);
    assert_eq!(m.depth(), 0);
    assert_eq!(step(&mut m, Event::Right(Id::Go.key().0)), Out::Moved);
    assert_eq!(m.depth(), 1);
}

#[test]
fn back_pops_a_page_with_focus_on_the_row_that_opened_it_and_closes_at_the_root() {
    let mut m = menu(grows(), Class::Action);
    step(&mut m, Event::Ok(Id::Go.key().0));
    settle(&mut m);
    assert_eq!(step(&mut m, Event::Back), Out::Moved);
    assert_eq!((m.depth(), m.header()), (0, None));
    assert_eq!(at(&m), Some(Id::Go.key().0), "the ring is on the row that opened the page");
    assert_eq!(step(&mut m, Event::Back), Out::Dismiss, "BACK at the root is the only key that closes");
}

#[test]
fn left_pops_a_page_and_is_inert_at_the_root() {
    let mut m = menu(grows(), Class::Action);
    assert_eq!(step(&mut m, Event::Left), Out::Stay, "LEFT at the root closes nothing");
    step(&mut m, Event::Ok(Id::Go.key().0));
    assert_eq!(step(&mut m, Event::Left), Out::Moved);
    assert_eq!(m.depth(), 0);
}

#[test]
fn a_click_on_the_header_pops() {
    let mut m = menu(grows(), Class::Action);
    assert_eq!(step(&mut m, Event::Ok(TITLE_KEY)), Out::Stay, "no header at the root");
    step(&mut m, Event::Ok(Id::Go.key().0));
    assert_eq!(step(&mut m, Event::Ok(TITLE_KEY)), Out::Moved);
    assert_eq!(m.depth(), 0);
}

#[test]
fn pop_to_returns_several_pages_in_one_slide() {
    let mut m = menu(grows(), Class::Action);
    step(&mut m, Event::Ok(Id::Go.key().0));
    step(&mut m, Event::Ok(Id::Deep.key().0));
    assert_eq!((m.depth(), m.top()), (2, Some(Pg::Two)));
    assert_eq!(step(&mut m, Event::PopTo(1)), Out::Moved);
    assert_eq!((m.depth(), m.header()), (1, Some("One")));
    assert_eq!(at(&m), Some(Id::Deep.key().0), "back on the row that opened Two");
    assert_eq!(step(&mut m, Event::PopTo(1)), Out::Stay, "already there");
    assert_eq!(step(&mut m, Event::PopTo(0)), Out::Moved);
    assert_eq!((m.depth(), m.header()), (0, None));
    assert_eq!(at(&m), Some(Id::Go.key().0));
}

#[test]
fn replace_root_clears_the_stack_and_slides_or_does_not() {
    let mut m = menu(grows(), Class::Action);
    step(&mut m, Event::Ok(Id::Go.key().0));
    settle(&mut m);
    assert_eq!(step(&mut m, Event::ReplaceRoot { slide: None }), Out::Moved);
    assert_eq!((m.depth(), m.header()), (0, None));
    assert_eq!(m.state().motion, Motion::Rest, "no slide: the content just changes");
    assert_eq!(step(&mut m, Event::ReplaceRoot { slide: Some(1.0) }), Out::Moved);
    assert_eq!(m.state().motion, Motion::Sliding, "a tab switch slides");
}

#[test]
fn replace_root_that_cannot_build_changes_nothing() {
    let mut m = menu(grows(), Class::Action);
    step(&mut m, Event::Ok(Id::Go.key().0));
    let out = m.step(Event::ReplaceRoot { slide: None }, &mut |_| None);
    assert_eq!((out, m.depth()), (Out::Stay, 1));
}

#[test]
fn refresh_rebuilds_the_page_in_place_keeping_the_cursor_by_identity() {
    let mut m = menu(grows(), Class::Action);
    step(&mut m, Event::Ok(Id::Go.key().0));
    step(&mut m, Event::Focused(Id::Leaf(2).key().0));
    // the page now has FIVE leaves: the cursor stays on Leaf(2) by identity
    let mut five = |p: Option<Pg>| match p {
        Some(Pg::One) => Some(Page::new(leaves(5, true), "One")),
        None => Some(root()),
        Some(Pg::Two) => None,
    };
    assert_eq!(m.step(Event::Refresh { keep: None }, &mut five), Out::Stay);
    assert_eq!(m.form().table.n_rows(), 6);
    assert_eq!(at(&m), Some(Id::Leaf(2).key().0));
    assert_eq!((m.depth(), m.header()), (1, Some("One")));
}

#[test]
fn refresh_drops_a_page_that_can_no_longer_be_built_with_no_slide() {
    let mut m = menu(grows(), Class::Action);
    step(&mut m, Event::Ok(Id::Go.key().0));
    step(&mut m, Event::Ok(Id::Deep.key().0));
    settle(&mut m);
    assert_eq!(m.depth(), 2);
    // page Two vanished (its data is gone): the page beneath returns on the row that opened Two
    m.step(Event::Refresh { keep: None }, &mut builder(true));
    assert_eq!((m.depth(), m.top(), m.header()), (1, Some(Pg::One), Some("One")));
    assert_eq!(at(&m), Some(Id::Deep.key().0), "the opener is re-resolved by identity");
    assert_eq!(m.state().motion, Motion::Rest, "dropped, not slid");
    // and a root-only collapse
    let mut m = menu(grows(), Class::Action);
    step(&mut m, Event::Ok(Id::Go.key().0));
    let mut none_but_root = |p: Option<Pg>| p.is_none().then(root);
    m.step(Event::Refresh { keep: None }, &mut none_but_root);
    assert_eq!((m.depth(), m.header()), (0, None));
    assert_eq!(at(&m), Some(Id::Go.key().0));
}

#[test]
fn a_push_whose_page_cannot_be_built_leaves_the_menu_alone() {
    let mut m = menu(grows(), Class::Action);
    assert_eq!(m.step(Event::Ok(Id::Go.key().0), &mut |p| p.is_none().then(root)), Out::Stay);
    assert_eq!(m.depth(), 0);
}

#[test]
fn a_pop_that_cannot_rebuild_any_page_beneath_keeps_the_page_showing_and_back_closes() {
    let mut m = menu(grows(), Class::Action);
    step(&mut m, Event::Ok(Id::Go.key().0));
    assert_eq!(m.step(Event::Left, &mut |_| None), Out::Stay, "LEFT never closes");
    assert_eq!(m.step(Event::Back, &mut |_| None), Out::Dismiss, "no depth can be built: the menu has nowhere to stand");
    assert_eq!(m.depth(), 1);
}

#[test]
fn tick_settles_the_motion_and_the_cursor_follows_the_focus_engine() {
    let mut m = menu(grows(), Class::Action);
    step(&mut m, Event::Ok(Id::Go.key().0));
    assert!(m.pointer_held(), "a slide holds the pointer");
    settle(&mut m);
    assert!(!m.pointer_held());
    m.step(Event::Tick { dt: 1.0 / 60.0, measure: &M, focus: Some(Id::Leaf(0).key().0) }, &mut builder(false));
    assert_eq!(at(&m), Some(Id::Leaf(0).key().0), "the cursor follows the engine");
    step(&mut m, Event::Focused(Id::Leaf(2).key().0));
    assert_eq!(at(&m), Some(Id::Leaf(2).key().0));
}

#[test]
fn the_glass_of_a_growing_host_opens_and_drains_and_no_other_host_has_one() {
    let mut m = menu(grows(), Class::Action);
    assert_eq!(m.opener_alpha(), 1.0, "before it opens the control is whole");
    step(&mut m, Event::Opening);
    for _ in 0..30 {
        m.step(Event::Tick { dt: 1.0 / 60.0, measure: &M, focus: None }, &mut builder(false));
    }
    assert!(m.opener_alpha() < 1.0, "the opener is consumed while the glass stands");
    assert!(!m.handed_back());
    step(&mut m, Event::Closing);
    for _ in 0..120 {
        m.step(Event::Tick { dt: 1.0 / 60.0, measure: &M, focus: None }, &mut builder(false));
    }
    assert!(m.handed_back(), "the droplet drained into its control");
    for host in [Host::BesideGlass { anchor: anchor() }, Host::Sheet { anchor: anchor() }, Host::Video { anchor: anchor(), placement: crate::popover::Placement::PLAYER_MENU }] {
        let mut m = menu(host, Class::Picker);
        step(&mut m, Event::Opening);
        step(&mut m, Event::Closing);
        assert_eq!((m.opener_alpha(), m.handed_back()), (1.0, false), "{host:?} has no morph");
    }
}

// ---- derived, never stored ---------------------------------------------------------------------

/// The ground is a function of the host, by one exhaustive match: a fifth host does not compile here
/// until it says what it stands on, and no host can ask for another ground.
#[test]
fn every_host_derives_exactly_one_ground_and_glass_never_stands_on_video() {
    let hosts = [
        grows(),
        Host::BesideGlass { anchor: anchor() },
        Host::Sheet { anchor: anchor() },
        Host::Video { anchor: anchor(), placement: crate::popover::Placement::PLAYER_MENU },
    ];
    for host in hosts {
        let want = match host {
            Host::Grows { .. } | Host::BesideGlass { .. } => Surface::Glass,
            Host::Sheet { .. } => Surface::Sheet,
            Host::Video { .. } => Surface::Flat,
        };
        assert_eq!(menu(host, Class::Action).surface(), want, "the menu derives it from its host");
    }
}

#[test]
fn the_class_is_the_menus_and_a_pushed_page_takes_it() {
    for (class, compact) in [(Class::Action, true), (Class::Picker, false)] {
        let mut m = menu(grows(), class);
        assert_eq!(m.form().table.compact, compact);
        step(&mut m, Event::Ok(Id::Go.key().0));
        assert_eq!(m.form().table.compact, compact, "a page has no class of its own");
        assert_eq!(m.class(), class);
        step(&mut m, Event::Back);
        assert_eq!(m.form().table.compact, compact);
    }
}

#[test]
fn the_width_floor_the_radius_and_the_dim_are_derived() {
    let video = Host::Video { anchor: anchor(), placement: crate::popover::Placement::PLAYER_MENU };
    assert_eq!(menu(video, Class::Action).form().table.min_panel_w, crate::theme::layout::PLAYER_MENU_MIN_W);
    for host in [grows(), Host::BesideGlass { anchor: anchor() }, Host::Sheet { anchor: anchor() }] {
        assert_eq!(menu(host, Class::Action).form().table.min_panel_w, crate::table::MENU_MIN_W);
    }
    assert_eq!(RADIUS, 20.0);
    let dim = |host, class| menu(host, class).scrim().alpha;
    assert_eq!(dim(Host::BesideGlass { anchor: anchor() }, Class::Action), crate::theme::underlay::DIM_COMPACT);
    assert_eq!(dim(Host::Sheet { anchor: anchor() }, Class::Picker), crate::theme::underlay::DIM_PANEL);
    assert_eq!(dim(video, Class::Action), crate::theme::underlay::DIM_SHEET);
}

/// ONE width rule for every host: the table's menu width, and a growing glass is never narrower than its
/// pill. The placements differ; the width does not.
#[test]
fn one_width_rule_for_every_host() {
    let wide = Rect::new(1200.0, 800.0, 400.0, 60.0);
    let hosts = [
        Host::Grows { pill: wide, clear_y: None, fill: Some(crate::theme::ACCENT), morph: Morph::FromPill },
        Host::BesideGlass { anchor: anchor() },
        Host::Sheet { anchor: anchor() },
        Host::Video { anchor: anchor(), placement: crate::popover::Placement::PLAYER_MENU },
    ];
    for host in hosts {
        let m = menu(host, Class::Action);
        let table = &m.form().table;
        let want = match host {
            Host::Grows { pill, .. } => table.menu_panel_width_from(&M, pill),
            _ => table.menu_panel_width(&M),
        };
        assert_eq!(m.panel_rect(&M).w, want, "{host:?}");
    }
    assert!(menu(hosts[0], Class::Action).panel_rect(&M).w >= 400.0, "a growing glass covers its pill");
}

#[test]
fn a_growing_panel_covers_its_opener_and_a_card_menu_stands_beside_its_card() {
    let g = menu(grows(), Class::Action).panel_rect(&M);
    assert!(g.x <= pill().x + 0.5 || g.x + g.w >= pill().x + pill().w - 0.5, "hangs from the pill's edge");
    assert!(g.y <= pill().y + 0.5 && g.y + g.h >= pill().y + pill().h - 0.5, "covers the pill");
    let card = rect_of(anchor());
    let b = menu(Host::BesideGlass { anchor: anchor() }, Class::Action).panel_rect(&M);
    assert!(b.x >= card.x + card.w, "to the right of the card, never over it");
    // flipped to the left when the right would run off the screen
    let edge = beside(Rect::new(1700.0, 300.0, 200.0, 300.0), 400.0, 300.0);
    assert!(edge.x + edge.w <= 1700.0, "flipped left of the card");
}

fn rect_of(bits: [u32; 4]) -> Rect {
    let [x, y, w, h] = bits.map(f32::from_bits);
    Rect::new(x, y, w, h)
}

/// The depth alone is what the Detail hosts' recorded canon holds; `stack_canon` is the whole of
/// `PageStack::canon` (the player's popovers'), byte for byte, so a host that moves over from one keeps its pins.
#[test]
fn the_canons_are_the_depth_alone_and_page_stacks_whole_stack_byte_for_byte() {
    use crate::page_stack::PageStack;
    use plx_machine::machine::Canon;
    let mut m = menu(grows(), Class::Action);
    let depth = |m: &Mn| {
        let mut c = Canon::default();
        m.depth_canon(&mut c);
        c.finish()
    };
    let code = |p: Pg| match p {
        Pg::One => 7,
        Pg::Two => 9,
    };
    let key = |id: &Id| id.key().0;
    let whole = |m: &Mn| {
        let mut c = Canon::default();
        m.stack_canon(&mut c, code, key);
        c.finish()
    };
    let mut want = Canon::default();
    want.u32(0);
    assert_eq!((depth(&m), whole(&m)), (want.finish(), want.finish()), "the root: the depth, no pages");
    step(&mut m, Event::Ok(Id::Go.key().0));
    step(&mut m, Event::Ok(Id::Deep.key().0));
    let mut only_depth = Canon::default();
    only_depth.u32(2);
    assert_eq!(depth(&m), only_depth.finish());
    // the same two pushes on a bare `PageStack`: the openers are Go (on the root) and Deep (on page One)
    let mut stack: PageStack<Pg, Id> = PageStack::new();
    stack.push(Pg::One, Id::Go, 0.0);
    stack.push(Pg::Two, Id::Deep, 0.0);
    let mut by_stack = Canon::default();
    stack.canon(&mut by_stack, code, key);
    assert_eq!(whole(&m), by_stack.finish(), "page codes and opener keys, in order");
    assert_ne!(whole(&m), depth(&m));
}

/// **A page that can no longer be built is not stood on.** The pop keeps going down to the first depth that can,
/// the root last, in one slide; BACK closes the menu when no depth can (the root's rows need the host's session).
#[test]
fn a_pop_keeps_going_down_to_a_buildable_page_and_back_closes_when_none_is() {
    let mut m = menu(grows(), Class::Action);
    step(&mut m, Event::Ok(Id::Go.key().0));
    step(&mut m, Event::Ok(Id::Deep.key().0));
    settle(&mut m);
    assert_eq!((m.top(), m.depth()), (Some(Pg::Two), 2));
    // page One (the one beneath) is gone: BACK lands on the root, not on nothing
    let mut no_one = |p: Option<Pg>| match p {
        Some(Pg::One) => None,
        other => builder(false)(other),
    };
    assert_eq!(m.step(Event::Back, &mut no_one), Out::Moved);
    assert_eq!((m.top(), m.depth(), m.header()), (None, 0, None), "down to the root in one pop");
    assert_eq!(m.state().motion, Motion::Sliding, "one slide");
    assert_eq!(at(&m), Some(Id::Go.key().0), "focus is on the row that opened the first page");
    // nothing can be built: BACK has nowhere to stand and closes; the stack is left as it was
    settle(&mut m);
    step(&mut m, Event::Ok(Id::Go.key().0));
    step(&mut m, Event::Ok(Id::Deep.key().0));
    let mut nothing = |_: Option<Pg>| -> Option<Pgs> { None };
    assert_eq!(m.step(Event::Back, &mut nothing), Out::Dismiss);
    assert_eq!(m.depth(), 2, "nothing was popped");
    assert_eq!(m.step(Event::Left, &mut nothing), Out::Stay, "LEFT is never a close");
    assert_eq!(m.step(Event::PopTo(0), &mut nothing), Out::Stay);
}

/// A video host keeps the side of its opener the placer chose, so the card rises from it (a card hung above its
/// opener rises from below), and every page after the first is laid out by the same rule.
#[test]
fn a_video_host_keeps_the_placed_side_for_its_rise() {
    use crate::popover::{Placement, Side};
    for (anchor, side) in [
        ([100.0f32, 80.0, 200.0, 60.0], Side::Below),
        ([100.0f32, 900.0, 200.0, 60.0], Side::Above),
    ] {
        let host = Host::Video { anchor: anchor.map(f32::to_bits), placement: Placement::UNDER_OR_ABOVE_FIT };
        let m = menu(host, Class::Picker);
        let rect = m.panel_rect(&M);
        assert_eq!(m.core_side(), Some(side), "{anchor:?}");
        let opener = Rect::new(anchor[0], anchor[1], anchor[2], anchor[3]);
        match side {
            Side::Below => assert!(rect.y >= opener.y + opener.h, "below the opener: {rect:?}"),
            _ => assert!(rect.y + rect.h <= opener.y, "above the opener: {rect:?}"),
        }
    }
}

/// The failure screen's picker is the video host's: flat, the player's width floor and its dim, placed under or above
/// its opener by the fit rule. Not a variant of its own (there is one video host).
#[test]
fn a_video_host_under_or_above_fit_is_the_sheets_placement_on_a_flat_card() {
    use crate::popover::Placement;
    let a = anchor();
    let sheet = menu(Host::Sheet { anchor: a }, Class::Picker);
    let video = menu(Host::Video { anchor: a, placement: Placement::UNDER_OR_ABOVE_FIT }, Class::Picker);
    assert_eq!(video.surface(), Surface::Flat);
    assert_eq!(video.form().table.min_panel_w, crate::theme::layout::PLAYER_MENU_MIN_W);
    let (r_video, r_sheet) = (video.panel_rect(&M), sheet.panel_rect(&M));
    assert_eq!((r_video.x, r_video.y), (r_sheet.x, r_sheet.y), "the same side and the same edge");
    let scrim = video.scrim();
    assert_eq!(scrim.alpha, crate::theme::underlay::DIM_SHEET);
    assert_eq!(scrim.source, crate::screen::UnderlaySource::Flat, "the neutral ink: no artwork cast on the picture");
}

/// A growing glass with no pill to start from (the profile chip: its rect is read again every frame) opens by the
/// fade-and-scale entrance, and follows the control from then on.
#[test]
fn a_pill_less_morph_opens_from_nothing_and_follows_the_opener() {
    let fade = Host::Grows { pill: pill(), clear_y: None, fill: None, morph: Morph::Fade };
    let mut m = menu(fade, Class::Action);
    step(&mut m, Event::Opening);
    assert_eq!(m.glass_opener(), None, "the morph has no rect to grow from");
    let moved = Rect::new(1180.0, 700.0, 120.0, 60.0);
    m.track_opener(moved);
    assert_eq!(m.glass_opener(), Some(moved), "from here the control's drawn rect");
    let r = m.panel_rect(&M);
    assert_eq!((r.y, r.x + r.w), (moved.y, moved.x + moved.w), "and the panel is laid over it: top edges flush, hung from the near edge");
    let mut from_pill = menu(grows(), Class::Action);
    step(&mut from_pill, Event::Opening);
    assert_eq!(from_pill.glass_opener(), Some(pill()), "a hero control's morph starts on its pill");
}

/// Text prewarm happens on EVERY page: a push and a pop each queue the strings of the page they show,
/// once per layout.
#[test]
fn every_page_is_prewarmed_once() {
    let _serial = plx_base::testlock::serial();
    let mut m = menu(grows(), Class::Action);
    for depth in 0..2 {
        plx_gfx::text::reset_prewarm_for_test();
        m.prepare(&M);
        assert!(plx_gfx::text::prewarm_pending(), "depth {depth}: the page showing was queued");
        plx_gfx::text::clear_prewarm();
        m.prepare(&M);
        assert!(!plx_gfx::text::prewarm_pending(), "depth {depth}: a layout already walked is not walked again");
        step(&mut m, Event::Ok(Id::Go.key().0));
    }
    step(&mut m, Event::Back);
    plx_gfx::text::reset_prewarm_for_test();
    m.prepare(&M);
    assert!(plx_gfx::text::prewarm_pending(), "the page a pop brings back is queued too");
}

/// LEFT and RIGHT are the host's: the menu's one group declares them `EdgeRule::Screen`, UP and DOWN stop.
#[test]
fn the_group_hands_left_and_right_to_the_host() {
    use crate::screen::EdgeRule;
    let m = menu(grows(), Class::Action);
    let mut out = Vec::new();
    m.groups(plx_machine::machine::GroupId(0), &M, &mut out);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].edge, [EdgeRule::Stop, EdgeRule::Stop, EdgeRule::Screen, EdgeRule::Screen]);
    assert_eq!(out[0].len, 3);
}

#[test]
fn focus_seats_on_the_cursor_and_the_header_is_never_a_row() {
    let mut m = menu(grows(), Class::Action);
    let entry = EntryId(7);
    let g = plx_machine::machine::GroupId(0);
    step(&mut m, Event::Ok(Id::Go.key().0));
    assert_eq!(m.seat(entry).elem, Id::Leaf(1).key().0);
    assert_eq!(m.group_of(&TITLE_KEY, g), None, "never in the D-pad column");
    assert!(m.place(&TITLE_KEY, &M).is_some(), "but replay can place it");
    // a key of the page that was left reconciles to the seat
    let stale = plx_machine::machine::FocusKey { entry, elem: Id::Go.key().0 };
    assert_eq!(m.reconcile(entry, stale, g).elem, Id::Leaf(1).key().0);
    let live = plx_machine::machine::FocusKey { entry, elem: Id::Leaf(2).key().0 };
    assert_eq!(m.reconcile(entry, live, g), live);
}

fn key(key: plx_machine::machine::Key, edge: plx_machine::machine::Edge, at_edge: bool) -> plx_machine::machine::InputKind<()> {
    plx_machine::machine::InputKind::Key { key, sym: 0, wcode: 0, edge, at_edge }
}

/// `keyed`: BACK's down edge is `Back`; LEFT and RIGHT are acted on only when re-delivered at the edge and on the
/// down edge, and swallowed on the up edge; RIGHT with no row focused is swallowed; every other input passes.
#[test]
fn keyed_maps_back_left_right_and_passes_the_rest() {
    use plx_machine::machine::{Edge::*, Key};
    let ev = |k: Keyed<Id>| match k {
        Keyed::Event(Event::Back) => "back",
        Keyed::Event(Event::Left) => "left",
        Keyed::Event(Event::Right(_)) => "right",
        Keyed::Event(_) => "other",
        Keyed::Swallow => "swallow",
        Keyed::Pass => "pass",
    };
    let k = |key_: Key, edge, at_edge, focus| ev(keyed::<(), Id>(&key(key_, edge, at_edge), focus));
    assert_eq!(k(Key::Back, Down, false, None), "back");
    assert_eq!(k(Key::Back, Up, false, None), "pass", "only the down edge of BACK is the menu's");
    assert_eq!(k(Key::Left, Down, true, Some(1)), "left");
    assert_eq!(k(Key::Right, Down, true, Some(1)), "right");
    assert_eq!(k(Key::Right, Down, true, None), "swallow", "no row to enter: answered, not dead");
    assert_eq!(k(Key::Left, Up, true, Some(1)), "swallow");
    assert_eq!(k(Key::Right, Up, true, Some(1)), "swallow");
    assert_eq!(k(Key::Left, Down, false, Some(1)), "pass", "a LEFT the engine moved focus with is not ours");
    assert_eq!(k(Key::Up, Down, true, Some(1)), "pass");
}
