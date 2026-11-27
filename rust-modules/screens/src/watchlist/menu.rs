//! **The Watchlist's Sort & Filter menu** (plan §2.4 W45; boards WatchlistOptions, Motion): a
//! `Style::Compact` surface presented over the page, a glass `Ground::Menu` panel that MORPHS out
//! of the header pill (DS6, DS7). It is a host of the shared menu ([`plx_ui::menu::Menu`]): the menu owns the
//! open/morph state, the table, the ground, the size class and the stops, and the engine owns the highlight:
//! every row is a focus stop keyed by its position in [`OptionsRow::ALL`].
//!
//! OK on a row applies it ([`OptionsRow::apply`]) and reports the new value to the application
//! (`AppFx::Watchlist(WatchlistOut::SetOptions)`), which hands it to the store; the menu stays up
//! with the check marks moved, the Library sort menu's idiom. BACK dismisses.
use std::borrow::Cow;

use plx_machine::machine::{Canon, Cx, Effects, EntryId, FocusKey, Fx, GroupId, Handled, LogicalState, Machine, NavOp};
use plx_ui::a11y::{A11y, A11yPath, Role};
use plx_ui::menu::{Event, Out, Page};
use plx_ui::screen::{At, Dir, Focusable, GroupSpec, Placed, RenderStrategy, Screen, ScreenEvent, Step};
use plx_ui::Rect;
use std::convert::Infallible;

use super::options::{options_form, options_menu, opening_row, OptionsMenu, OptionsRow, WatchlistOptionsState};
use super::page::WatchlistOut;
use crate::registry::{AppFx, ContentLike, WatchlistMenuArg};

pub const SHAPE: &str = "WatchlistMenu{arg:WatchlistMenuArg,options:{type:u32,sort:u32,descending:bool,show:u32},sel:u32,table:TableViewMotion}";

const GROUP: GroupId = GroupId(0);

pub struct WatchlistMenuScreen {
    entry: EntryId,
    arg: WatchlistMenuArg,
    /// The value the rows show; the application's store is the owner, this follows the presses.
    options: WatchlistOptionsState,
    menu: OptionsMenu,
    opened: bool,
}

/// The pill rect a menu argument carries.
fn anchor_rect(arg: &WatchlistMenuArg) -> Rect {
    let [x, y, w, h] = arg.anchor.map(f32::from_bits);
    Rect::new(x, y, w, h)
}

/// The only page there is: the rows of `options`, opening on the first row it has chosen.
fn root(options: WatchlistOptionsState) -> impl FnMut(Option<Infallible>) -> Option<Page<OptionsRow, OptionsRow, Infallible>> {
    move |page| match page {
        Some(never) => match never {},
        None => Some(Page::root(options_form(&options)).initial(opening_row(&options))),
    }
}

impl WatchlistMenuScreen {
    /// The panel as it reads: each section's heading and its rows' `(label, value)`, for a route
    /// test that compares the Watchlist's words with the Library's.
    #[cfg(any(test, feature = "test-support"))]
    pub fn probe_panel(&self) -> Vec<(String, Vec<(String, Option<String>)>)> {
        self.menu.form().table.sections.iter()
            .map(|section| (section.header.clone(), section.rows.iter().map(|row| (row.label.clone(), row.value.clone())).collect()))
            .collect()
    }

    /// The rect the glass is morphing from (the pill as last handed in).
    #[cfg(test)]
    pub(crate) fn glass_opener(&self) -> Option<Rect> {
        self.menu.glass().and_then(plx_ui::popover::Popover::opener)
    }

    pub fn new(entry: EntryId, arg: WatchlistMenuArg) -> Self {
        let mut menu = options_menu(anchor_rect(&arg));
        menu.step(Event::ReplaceRoot { slide: None }, &mut root(arg.options));
        Self { entry, arg, options: arg.options, menu, opened: false }
    }

    /// Perform what the menu answered: a pick edits the value (and the rows are rebuilt in place under the
    /// cursor), BACK closes.
    fn run<H: ContentLike>(&mut self, out: Out<OptionsRow>, fx: &mut Effects<'_, H>) {
        match out {
            Out::Picked(row) => {
                if row.apply(&mut self.options) {
                    fx.push(Fx::App(AppFx::Watchlist(WatchlistOut::SetOptions(self.options))));
                    self.menu.step(Event::Refresh { keep: Some(row) }, &mut root(self.options));
                }
            }
            Out::Dismiss => fx.push(Fx::Nav(NavOp::Dismiss(self.entry))),
            Out::Stay | Out::Moved => {}
        }
    }
}

impl plx_ui::menu::HasMenu for WatchlistMenuScreen {
    type Page = Infallible;
    type Row = OptionsRow;
    type Act = OptionsRow;
    fn menu(&self) -> &OptionsMenu {
        &self.menu
    }
    fn menu_entry(&self) -> EntryId {
        self.entry
    }
}

impl<H: ContentLike> Machine<H> for WatchlistMenuScreen {
    type Ev = ScreenEvent<H>;

    fn step(&mut self, ev: &Self::Ev, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) -> Handled {
        if matches!(ev, ScreenEvent::Mount) {
            self.opened = true;
        }
        match self.menu.on_screen(ev, cx, self.entry, fx, &mut root(self.options)) {
            Some(glued) => {
                self.run(glued.out, fx);
                glued.handled
            }
            None => Handled::No,
        }
    }
}

impl<H: ContentLike> Focusable<H> for WatchlistMenuScreen {
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
    /// `watchlist_menu.<row>`: each row is named by what it chooses (`OptionsRow::a11y_name`),
    /// labelled with its drawn words.
    fn a11y(&self, key: &u32, _: &Cx<'_, H>) -> Option<A11y<'_>> {
        let row = *self.menu.id_of(*key)?;
        Some(A11y::new(A11yPath::new().name(row.a11y_name()), Role::Row).with_label(row.label()))
    }
    fn a11y_elems(&self, g: GroupId, _: &Cx<'_, H>, out: &mut Vec<u32>) {
        if g == GROUP {
            out.extend(self.menu.focusable());
        }
    }
}

impl<H: ContentLike> Screen<H> for WatchlistMenuScreen {
    /// Sort & Filter shows the effect of each click: the page reflows under the open menu, and
    /// the glass stands on a host that follows it (`popover::host::set_reflow`).
    fn lets_host_reflow(&self) -> bool {
        true
    }
    /// The pill the panel grew out of: the page leaves it out of its pass (`DrawFrame::lifted`) and
    /// the host's opener lift (`WatchlistScreen::redraw_focused`) is its only copy, lit above the
    /// scrim and the panel at the morph's `opener_alpha` (gone while the menu stands, back as the
    /// droplet drains into it). Left on the page it sat dimmed under the scrim.
    fn opener(&self) -> Option<(EntryId, Option<FocusKey<u32>>)> {
        Some((self.arg.host, Some(FocusKey { entry: self.arg.host, elem: super::page::PILL_ELEM })))
    }
    /// The control's DRAWN plate, every frame (the page that carries it moves: focus on the pill brings the page home, a wheel
    /// scrolls it): the glass grows out of, and drains back into, the pill where it stands (`Menu::track_opener`).
    fn opener_rect(&mut self, drawn: plx_ui::Rect) {
        self.menu.track_opener(plx_ui::widgets::focus_pill(drawn));
    }
    // The menu's own (`plx_ui::menu_screen_methods!`): the opener is CONSUMED by the morph (the lift paints it at
    // the morph's `opener_alpha`: out as the glass detaches, back in as it merges into it), the droplet's
    // hand-back, the COMPACT dim (DS6's 30% scrim: a chip-shaped control on a live page opening a list), the
    // prewarm, the pointer hold, and the draw with its focus stops (the glass grows out of the pill's rect; drawn
    // NOT faded by the container's appear: the morph is its own entrance, and the pill above it is the opener
    // lift's).
    plx_ui::menu_screen_methods!(H, "wl.menu");
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn name(&self) -> &'static str {
        "watchlist_menu"
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

impl LogicalState for WatchlistMenuScreen {
    fn write(&self, c: &mut Canon) {
        self.arg.write(c);
        self.options.write_canon(c);
        c.u32(self.menu.form().selected_key().map_or(0, |key| key.0));
        self.menu.form().table.write_motion(c);
    }
    fn probe(&self, out: &mut String) {
        out.push_str(&format!("watchlist_menu sel={} opened={}", self.menu.form().selected_key().map_or(0, |key| key.0), self.opened));
    }
}
