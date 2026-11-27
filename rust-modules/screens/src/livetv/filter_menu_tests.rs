use super::*;
use crate::registry::PageMemory;
use plx_machine::machine::{Cx, Edge, FocusKey, FocusRead, Host, InputEvent, InputKind, InputOwner, Key, Machine, NavOp, PressRead, ScreenId, Source, Tick};
use plx_ui::a11y::{A11yPath, Role};
use plx_ui::screen::{Dir, Focusable, Screen, Step};
use plx_ui::screen::ScreenArg;

#[derive(Clone)]
struct Arg;
impl LogicalState for Arg {
    fn write(&self, _: &mut Canon) {}
    fn probe(&self, _: &mut String) {}
}
impl ScreenArg for Arg {
    fn chrome(&self) -> plx_machine::machine::Chrome {
        plx_machine::machine::Chrome::None
    }
    fn id(&self) -> ScreenId {
        ScreenId(1)
    }
    fn title(&self) -> Option<&str> {
        None
    }
    fn same_instance(&self, _: &Self) -> bool {
        true
    }
}
struct HostFixture;
impl Host for HostFixture {
    type Arg = Arg;
    type Fx = crate::registry::AppFx;
    type Msg = AppMsg;
    type Elem = u32;
    type Views<'a> = ();
    type Init = Arg;
    type Memory = PageMemory;
}

const PAGE: EntryId = EntryId(2);
const ENTRY: EntryId = EntryId(3);

fn menu(chosen: Filter) -> LiveTvFilterScreen {
    let anchor = [1500.0f32, 148.0, 270.0, 56.0].map(f32::to_bits);
    new(ENTRY, LiveTvFilterArg { host: InstanceId(9), chosen, anchor, opener: (PAGE, 5) })
}

fn cx() -> Cx<'static, HostFixture> {
    Cx {
        views: (),
        tick: Tick::default(),
        measure: &plx_ui::fixture::FixtureMeasure,
        focus: FocusRead::default(),
        press: PressRead::default(),
        modal: Default::default(), page_image: Default::default(), hold_hint: Default::default(),
        owner: InputOwner::Entry(ENTRY),
    }
}

/// What one event does to the menu: the effects it pushes.
fn drive(m: &mut LiveTvFilterScreen, ev: ScreenEvent<HostFixture>) -> Vec<Fx<HostFixture>> {
    let mut out = Vec::new();
    let mut present = plx_machine::present::Present::new();
    let mut fx: Effects<'_, HostFixture> = Effects::new(&mut out, MachineId::Instance(InstanceId(4)), &mut present);
    Machine::<HostFixture>::step(m, &ev, &cx(), &mut fx);
    out.into_iter().map(|e| e.fx).collect()
}

fn mounted(chosen: Filter) -> LiveTvFilterScreen {
    let mut m = menu(chosen);
    drive(&mut m, ScreenEvent::Mount);
    m
}

fn closes(fx: &[Fx<HostFixture>]) -> bool {
    fx.iter().any(|f| matches!(f, Fx::Nav(NavOp::Dismiss(ENTRY))))
}

fn reported(fx: &[Fx<HostFixture>]) -> Vec<Filter> {
    fx.iter().filter_map(|f| match f {
        Fx::Deliver(MachineId::Instance(InstanceId(9)), Delivery::Screen(ScreenEvent::App(AppMsg::LiveTvFilterPicked(filter)))) => Some(*filter),
        _ => None,
    }).collect()
}

/// The menu opened on a filter, run through the id conformance (`AppArg::LiveTvFilter`; the coverage table calls this).
pub(crate) fn livetv_filter_conforms() -> plx_ui::a11y::Namespaces {
    let m = mounted(Filter::OnNow);
    vec![plx_ui::a11y::conform::<HostFixture, _>(&m, &cx())]
}

/// **Four rows, in the spec's order, the filter in force checked and the menu opened on it.**
#[test]
fn the_menu_lists_the_four_filters_and_opens_on_the_one_in_force() {
    let _lang = plx_os::i18n::language_on_this_thread_for_test(plx_os::i18n::Preference::En);
    for chosen in Filter::ALL {
        let m = mounted(chosen);
        let panel = m.probe_panel();
        let labels: Vec<&str> = panel.iter().flat_map(|(_, rows)| rows.iter().map(|(label, _)| label.as_str())).collect();
        assert_eq!(labels, ["On now", "Just started", "Ending soon", "Later today"], "{chosen:?}");
        let want: Vec<bool> = Filter::ALL.iter().map(|filter| *filter == chosen).collect();
        assert_eq!(m.probe_checked(), want, "{chosen:?}");
        assert_eq!(m.highlighted(), chosen.code() as usize, "{chosen:?}: opens on its own row");
    }
}

/// **A pick is reported and the menu STAYS UP with the check moved**, as the Library's and the Watchlist's menus do (the page
/// reflows under it); the row that is already checked reports nothing. Red while the menu closed on a pick.
#[test]
fn a_row_reports_its_filter_to_the_host_page_and_stays_up_with_the_check_moved() {
    for (row, filter) in Filter::ALL.into_iter().enumerate() {
        let mut m = mounted(Filter::OnNow);
        let fx = drive(&mut m, ScreenEvent::Activate(row as u32));
        assert!(!closes(&fx), "row {row}: the menu stays up");
        let want = if filter == Filter::OnNow { Vec::new() } else { vec![filter] };
        assert_eq!(reported(&fx), want, "row {row}: the page is told the filter, and nothing for the one in force");
        let checked: Vec<bool> = Filter::ALL.iter().map(|f| *f == filter).collect();
        assert_eq!(m.probe_checked(), checked, "row {row}: the check follows the pick");
        assert!(reported(&drive(&mut m, ScreenEvent::Activate(row as u32))).is_empty(), "row {row} again reports nothing");
    }
}

/// A pick shows its effect at once: the host follows the page's reflow under the open menu, as the Watchlist's menu asks.
#[test]
fn the_menu_lets_the_host_reflow_under_it() {
    assert!(Screen::<HostFixture>::lets_host_reflow(&menu(Filter::OnNow)));
}

/// BACK closes the menu and reports nothing.
#[test]
fn back_closes_and_reports_nothing() {
    let mut m = mounted(Filter::EndingSoon);
    let fx = drive(&mut m, ScreenEvent::Input(InputEvent {
        at: Tick::default(),
        source: Source::Script,
        kind: InputKind::Key { key: Key::Back, sym: 0, wcode: 0, edge: Edge::Down, at_edge: false },
    }));
    assert!(closes(&fx) && reported(&fx).is_empty());
}

/// The control the glass grew out of is the page's: the menu names it as its opener, so the page leaves it out of its pass.
#[test]
fn the_menus_opener_is_the_pages_filter_control() {
    let m = menu(Filter::OnNow);
    assert_eq!(
        Screen::<HostFixture>::opener(&m),
        Some((PAGE, Some(FocusKey { entry: PAGE, elem: 5 }))),
    );
}

/// Each row is named by the filter it chooses, never by its place, and finds itself again by that name.
#[test]
fn each_row_answers_the_name_of_its_filter() {
    let m = mounted(Filter::OnNow);
    let cx = cx();
    for (row, name) in ["on_now", "just_started", "ending_soon", "later_today"].into_iter().enumerate() {
        let path = A11yPath::parse_relative(name).expect("a plain row name");
        assert_eq!(Focusable::<HostFixture>::a11y_find(&m, &path, &cx), Some(row as u32), "{name}");
        let a = Focusable::<HostFixture>::a11y(&m, &(row as u32), &cx).expect("every row projects");
        assert_eq!(a.role, Role::Row);
    }
    assert!(!livetv_filter_conforms().is_empty());
}

/// UP and DOWN move the highlight along the four rows and stop at the ends.
#[test]
fn up_and_down_walk_the_rows_and_stop_at_the_ends() {
    let m = menu(Filter::OnNow);
    let cx = cx();
    let key = |row: usize| FocusKey { entry: ENTRY, elem: row as u32 };
    assert!(matches!(Focusable::<HostFixture>::neighbour(&m, key(0), Dir::Up, &cx), Step::Edge));
    assert!(matches!(Focusable::<HostFixture>::neighbour(&m, key(0), Dir::Down, &cx), Step::Move(k) if k == key(1)));
    assert!(matches!(Focusable::<HostFixture>::neighbour(&m, key(3), Dir::Down, &cx), Step::Edge));
}

/// **The filter menu follows its control**: the control rides the page, and a page that glides home under the open menu (the
/// control takes focus, or the wheel moved it) moves it, so the menu takes the control's drawn plate from the dispatcher each frame
/// (`Screen::opener_rect` -> `Menu::track_opener`), whatever the anchor it was opened with. Red while the menu held that anchor.
#[test]
fn the_menu_follows_its_control_as_the_page_moves() {
    let mut m = menu(Filter::OnNow);
    let plate = Rect::new(1600.0, 96.0, 188.0, 50.0);
    Screen::<HostFixture>::opener_rect(&mut m, plate);
    assert_eq!(m.menu.glass().and_then(plx_ui::popover::Popover::opener), Some(plx_ui::widgets::focus_pill(plate)));
    let moved = Rect { y: 140.0, ..plate };
    Screen::<HostFixture>::opener_rect(&mut m, moved);
    assert_eq!(m.menu.glass().and_then(plx_ui::popover::Popover::opener), Some(plx_ui::widgets::focus_pill(moved)), "and again as it moves on");
}
