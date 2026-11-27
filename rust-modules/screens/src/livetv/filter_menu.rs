//! **Live TV Browse's filter menu** (PR7; handoff `SPEC-6`): the surface the filter control opens, a host of the shared menu
//! ([`plx_ui::menu::Menu`], built by [`Menu::from_header_pill`] as the Watchlist's Sort & Filter menu is) over the four
//! filters: a `Style::Compact` panel presented over the page, a glass `Ground::Menu` panel that MORPHS out of the control, the
//! filter in force marked. Every row is a focus stop keyed by its position in [`Filter::ALL`], the control that opened it is the
//! host's opener lift (`Screen::opener`), BACK drains the glass back into it.
//!
//! **A pick is REPORTED, not applied**: OK on a row delivers [`AppMsg::LiveTvFilterPicked`] to the Live TV instance named on the
//! argument and, as every header menu does, the menu STAYS UP with the check moved; the page reflows under it. The page owns its
//! filter (`Scope::Browse(Filter)`) and changes it on that message alone, so the filter on show and the set under it cannot
//! disagree. OK on the row that is already checked reports nothing.
use std::borrow::Cow;
use std::convert::Infallible;

use plx_machine::machine::{Canon, Cx, Delivery, Effects, EntryId, FocusKey, Fx, GroupId, Handled, InstanceId, LogicalState, Machine, MachineId, NavOp};
use plx_ui::a11y::{A11y, A11yPath, Role};
use plx_ui::form::{Form, FormId, FormSection, RowKey, RowKind};
use plx_ui::menu::{Event, GrowWidth, Menu, Out, Page};
use plx_ui::screen::{At, Dir, Focusable, GroupSpec, Placed, RenderStrategy, Screen, ScreenEvent, Step};
use plx_ui::table::Row;
use plx_ui::Rect;

use super::browse::Filter;
use crate::registry::{AppLike, AppMsg, PageMemory};

pub const SHAPE: &str =
    "LiveTvFilter{arg:LiveTvFilterArg{host:u32,chosen:u32,anchor:[u32;4],opener:{entry:u32,elem:u32}},chosen:u32,sel:u32,table:TableViewMotion}";

const GROUP: GroupId = GroupId(0);

/// **What the container is asked to present**: the Live TV instance the pick is reported to, the filter in force (the row
/// the menu opens on, wearing the check mark), the drawn rect of the control it grows out of (bit-preserving, so a canonical
/// argument holds it without float equality) and that control: the page's entry and its element, which the page leaves out of its
/// own pass and the opener lift draws at the morph's alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveTvFilterArg {
    pub host: InstanceId,
    pub chosen: Filter,
    pub anchor: [u32; 4],
    pub opener: (EntryId, u32),
}

impl LogicalState for LiveTvFilterArg {
    fn write(&self, c: &mut Canon) {
        c.u32(self.host.0).u32(self.chosen.code());
        for value in self.anchor {
            c.u32(value);
        }
        c.u32(self.opener.0 .0).u32(self.opener.1);
    }
    fn probe(&self, out: &mut String) {
        out.push_str("livetv_filter_arg");
    }
}

/// A row's identity IS the filter it chooses; its focus key is its position in [`Filter::ALL`] (draw order), so a key names the
/// same choice before and after the form is rebuilt in place under a press.
impl FormId for Filter {
    fn key(&self) -> RowKey {
        RowKey(self.code())
    }
    /// The row's accessibility name (`livetv_filter.<name>`): what it chooses, never its place.
    fn a11y(&self) -> Option<&'static str> {
        Some(match self {
            Filter::OnNow => "on_now",
            Filter::JustStarted => "just_started",
            Filter::EndingSoon => "ending_soon",
            Filter::LaterToday => "later_today",
        })
    }
}

/// The menu's rows, the filter in force checked.
fn filter_form(chosen: Filter) -> Form<Filter, Filter, Infallible> {
    let mut section = FormSection::new("");
    for filter in Filter::ALL {
        section = section.item(filter, RowKind::Choice, filter, Row::new(super::filter_label(filter)).checked(filter == chosen));
    }
    Form::new().section(section)
}

/// The only page there is: the rows for `chosen`, opening on it.
fn root(chosen: Filter) -> impl FnMut(Option<Infallible>) -> Option<Page<Filter, Filter, Infallible>> {
    move |page| match page {
        Some(never) => match never {},
        None => Some(Page::root(filter_form(chosen)).initial(Some(chosen))),
    }
}

/// The filter menu: the shared [`Menu`] over the filters.
type FilterMenu = Menu<Infallible, Filter, Filter>;

pub struct LiveTvFilterScreen {
    entry: EntryId,
    arg: LiveTvFilterArg,
    /// The filter the rows check; the page is the owner, this follows the presses.
    chosen: Filter,
    menu: FilterMenu,
    opened: bool,
}

/// The menu for `arg`, opened on the filter chosen.
pub fn new(entry: EntryId, arg: LiveTvFilterArg) -> LiveTvFilterScreen {
    let [x, y, w, h] = arg.anchor.map(f32::from_bits);
    let mut menu = Menu::from_header_pill(Rect::new(x, y, w, h), crate::registry::BAND, GrowWidth::Catch);
    menu.step(Event::ReplaceRoot { slide: None }, &mut root(arg.chosen));
    LiveTvFilterScreen { entry, arg, chosen: arg.chosen, menu, opened: false }
}

impl LiveTvFilterScreen {
    /// The panel as it reads: each section's heading and its rows' `(label, value)`, for a route test.
    #[cfg(any(test, feature = "test-support"))]
    pub fn probe_panel(&self) -> Vec<(String, Vec<(String, Option<String>)>)> {
        self.menu.form().table.sections.iter()
            .map(|section| (section.header.clone(), section.rows.iter().map(|row| (row.label.clone(), row.value.clone())).collect()))
            .collect()
    }

    /// Which rows wear the check mark, in draw order.
    #[cfg(test)]
    pub fn probe_checked(&self) -> Vec<bool> {
        self.menu.form().table.sections.iter().flat_map(|section| section.rows.iter().map(|row| row.checked)).collect()
    }

    /// The row the cursor stands on.
    #[cfg(test)]
    pub fn highlighted(&self) -> usize {
        self.menu.form().selected_key().map_or(0, |key| key.0 as usize)
    }

    /// Perform what the menu answered: a pick reports the filter (and the rows are rebuilt in place under the cursor), BACK
    /// closes.
    fn run<H: AppLike<Memory = PageMemory>>(&mut self, out: Out<Filter>, fx: &mut Effects<'_, H>) {
        match out {
            Out::Picked(filter) => {
                if filter != self.chosen {
                    self.chosen = filter;
                    fx.push(Fx::Deliver(
                        MachineId::Instance(self.arg.host),
                        Delivery::Screen(ScreenEvent::App(AppMsg::LiveTvFilterPicked(filter))),
                    ));
                    self.menu.step(Event::Refresh { keep: Some(filter) }, &mut root(self.chosen));
                }
            }
            Out::Dismiss => fx.push(Fx::Nav(NavOp::Dismiss(self.entry))),
            Out::Stay | Out::Moved => {}
        }
    }
}

impl plx_ui::menu::HasMenu for LiveTvFilterScreen {
    type Page = Infallible;
    type Row = Filter;
    type Act = Filter;
    fn menu(&self) -> &FilterMenu {
        &self.menu
    }
    fn menu_entry(&self) -> EntryId {
        self.entry
    }
}

impl<H: AppLike<Memory = PageMemory>> Machine<H> for LiveTvFilterScreen {
    type Ev = ScreenEvent<H>;

    fn step(&mut self, ev: &Self::Ev, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) -> Handled {
        if matches!(ev, ScreenEvent::Mount) {
            self.opened = true;
        }
        match self.menu.on_screen(ev, cx, self.entry, fx, &mut root(self.chosen)) {
            Some(glued) => {
                self.run(glued.out, fx);
                glued.handled
            }
            None => Handled::No,
        }
    }
}

impl<H: AppLike<Memory = PageMemory>> Focusable<H> for LiveTvFilterScreen {
    fn groups(&self, cx: &Cx<'_, H>, out: &mut Vec<GroupSpec>) {
        self.menu.groups(GROUP, cx.measure, out);
    }
    fn group_of(&self, elem: &u32, _: &Cx<'_, H>) -> Option<GroupId> {
        self.menu.group_of(elem, GROUP)
    }
    fn neighbour(&self, key: FocusKey<u32>, dir: Dir, _: &Cx<'_, H>) -> Step<u32> {
        self.menu.neighbour(self.entry, key, dir)
    }
    fn place(&self, elem: &u32, cx: &Cx<'_, H>, _: At) -> Option<Placed> {
        self.menu.place(elem, cx.measure)
    }
    fn reconcile(&self, want: FocusKey<u32>, _: &Cx<'_, H>) -> FocusKey<u32> {
        self.menu.reconcile(self.entry, want, GROUP)
    }
    fn seat(&self, _: GroupId, _: Placed, _: &Cx<'_, H>) -> FocusKey<u32> {
        self.menu.seat(self.entry)
    }
    /// `livetv_filter.<name>`: each row is named by the filter it chooses, labelled with its drawn words.
    fn a11y(&self, key: &u32, _: &Cx<'_, H>) -> Option<A11y<'_>> {
        let filter = *self.menu.id_of(*key)?;
        Some(A11y::new(A11yPath::new().name(filter.a11y()?), Role::Row).with_label(super::filter_label(filter)))
    }
    fn a11y_elems(&self, g: GroupId, _: &Cx<'_, H>, out: &mut Vec<u32>) {
        if g == GROUP {
            out.extend(self.menu.focusable());
        }
    }
}

impl<H: AppLike<Memory = PageMemory>> Screen<H> for LiveTvFilterScreen {
    /// A pick shows its effect at once: the page reflows under the open menu, and the glass stands on a host that follows it
    /// (`popover::host::set_reflow`), as the Watchlist's menu does.
    fn lets_host_reflow(&self) -> bool {
        true
    }
    /// The control the glass grew out of: the page leaves it out of its pass (`DrawFrame::lifted`) and the host's opener lift
    /// (`LiveTvScreen::redraw_opener`) is its only copy, lit above the scrim and the panel at the morph's `opener_alpha`.
    fn opener(&self) -> Option<(EntryId, Option<FocusKey<u32>>)> {
        let (entry, elem) = self.arg.opener;
        Some((entry, Some(FocusKey { entry, elem })))
    }
    /// The control's DRAWN plate, every frame (the page that carries it moves: focus on the pill brings the page home, a wheel
    /// scrolls it): the glass grows out of, and drains back into, the pill where it stands (`Menu::track_opener`).
    fn opener_rect(&mut self, drawn: plx_ui::Rect) {
        self.menu.track_opener(plx_ui::widgets::focus_pill(drawn));
    }
    // The menu's own (`plx_ui::menu_screen_methods!`): the opener is consumed by the morph, the droplet's hand-back, the compact
    // dim, the prewarm, the pointer hold, and the draw with its focus stops.
    plx_ui::menu_screen_methods!(H, "ltv.filter");
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn name(&self) -> &'static str {
        "livetv_filter"
    }
    fn state(&self) -> &dyn LogicalState {
        self
    }
    fn crumb(&self, _: &Cx<'_, H>) -> Option<Cow<'_, str>> {
        None
    }
    fn render(&self) -> RenderStrategy {
        RenderStrategy::Page
    }
}

impl LogicalState for LiveTvFilterScreen {
    fn write(&self, c: &mut Canon) {
        self.arg.write(c);
        c.u32(self.chosen.code());
        c.u32(self.menu.form().selected_key().map_or(0, |key| key.0));
        self.menu.form().table.write_motion(c);
    }
    fn probe(&self, out: &mut String) {
        out.push_str(&format!("livetv_filter sel={} opened={}", self.menu.form().selected_key().map_or(0, |key| key.0), self.opened));
    }
}

#[cfg(test)]
#[path = "filter_menu_tests.rs"]
pub(crate) mod tests;
