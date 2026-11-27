//! **`plx_ui::menu` — the one menu component**: the glass (or sheet, or flat card) a table of rows stands
//! in, the stack of pages pushed into it, and the motion between them, as ONE machine the hosts
//! (`screens::item_menu`, `versions_panel`, `preplay_panel`, `alt_sources`, the Library's sort / filter / genre /
//! sources menu, the account menu and the Watchlist's options) take whole. It grew out of
//! [`crate::page_stack`] and stands on the same pieces the player's popovers do ([`PageStack`], the
//! focus-stop functions and [`page_stack::register_stops`], the ONE stop registration), and replaces what
//! every host wrote for itself: the push and the pop, BACK, the ground, the stop registration, the size
//! class, the radius, the sizing, and the glue between a screen's events and the machine ([`Menu::on_screen`]).
//!
//! `ui` may use only `base`, `machine`, `os` and `gfx` (`ci/module-layers.ini`), so the component takes
//! FORMS and VALUES: a host builds its rows from its own data ([`Page`]), the menu never sees a
//! session, a store or a screen. Content stays in `appkit`, hosts stay in `screens`.
//!
//! # The machine
//!
//! The state is a stack of pages over a motion, and nothing else is stored:
//!
//! ```text
//!   stack:  Root  ──push──▶  Paged(1)  ──push──▶  Paged(2) …      (`PageStack`: empty = the root)
//!   motion: Rest  ──push/pop/replace──▶  Sliding ──settles──▶  Resizing ──settles──▶  Rest
//! ```
//!
//! Every change is an [`Event`] through [`Menu::step`]; the answer is an [`Out`]:
//!
//! | event | at the root | on a page |
//! |---|---|---|
//! | `Ok(key)` on a Nav row (`RowKind::Nav(page)`) | push `page` | push `page` |
//! | `Ok(key)` on any other row | `Picked(action)` | `Picked(action)` |
//! | `Ok(key)` on a disabled row | `Stay` | `Stay` |
//! | `Ok(key)` on no row | `Dismiss` | `Dismiss` |
//! | `Ok(TITLE_KEY)` (a click on the header) | `Stay` | pop |
//! | `Right(key)` | `Stay` unless the row is Nav, then push | same |
//! | `Back` | `Dismiss` | pop; `Dismiss` when no page beneath can be built (the root's rows need the host) |
//! | `Left` | `Stay` | pop |
//! | `PopTo(depth)` | `Stay` | pop to `depth` in one slide |
//! | `ReplaceRoot { slide }` | rebuild the root | clear the stack, rebuild the root |
//! | `Refresh { keep }` | rebuild the root in place | rebuild the page in place; a page that can no longer be built is dropped with no slide and the page beneath takes focus on the row that opened it |
//! | `Opening` / `Closing` | the glass of a [`Host::Grows`] starts / drains its morph | |
//! | `Tick` | springs, the cursor, the glass | |
//! | `Focused(key)` | the cursor follows the focus engine | |
//!
//! LEFT pops and RIGHT enters a Nav row, as in the player's menus; BACK at the root is the only key
//! that closes the menu from inside. Each transition takes a `build(Option<P>) -> Option<Page>`
//! closure: `None` asks for the root, and the answer is `None` when that page can no longer be built
//! (the root's rows need the host's session, which the component never holds). A pop whose page beneath
//! cannot be built keeps going down to the first depth that can, the root last.
//!
//! # The host's side
//!
//! A host implements [`HasMenu`], takes the six `Screen` methods the menu owns (the dim, the prewarm, the
//! pointer hold, the draw, the morph's hand-over) from [`menu_screen_methods!`], and answers its `Machine::step`
//! with [`Menu::on_screen`]: Mount, Closing, Tick, FocusMoved, Activate, PressCommit and the keys all become an
//! [`Event`] in ONE place, with the answer ([`Glued`]) for the host to perform. What stays the host's is what
//! only it knows: its rows ([`Page`]), and what a pick or a dismissal does.
//!
//! # Derived, never stored
//!
//! The header (the top page's title, drawn with the back mark; the root has none; there is no way to
//! push a page without one), the [`Class`], the radius ([`RADIUS`]), the width floor, the ground
//! ([`Menu::surface`]), the pointer hold ([`Menu::pointer_held`]: the motion is not at rest), the dim, and what
//! a rebuild does when the row under the cursor is gone (an [`Class::Action`] menu opens the page afresh, a
//! [`Class::Picker`] lands on the nearest survivor).
//!
//! # Unrepresentable
//!
//! * a title without a page: a title is a field of [`Page`], and the header is read off the top page;
//! * a morph without its glass: [`Host::Grows`] is the only host that owns a `Popover`, built with it;
//! * a per-page size class: the class is a constructor argument of the menu, a [`Page`] has none, and
//!   a page pushed into an action menu takes the menu's;
//! * glass on video: [`Host::Video`] is the one host over the video plane and its surface is flat, in the
//!   match that derives it (`docs/liquid-glass.md` §7). It carries its placement ([`Placement`]), so the
//!   player's menus ([`Placement::PLAYER_MENU`]), when they move onto this, and the failure screen's picker
//!   ([`Placement::UNDER_OR_ABOVE_FIT`]) are one host, one width floor and one dim. Today the failure screen's
//!   Audio & Subtitles picker is the one that stands on it.
//!
//! # Size
//!
//! [`MenuCore`] is not generic: the placement, the draw, the stops and the motion are compiled once.
//! [`Menu`] adds the forms and the stack, generic over the host's row id, action and page.
use crate::form::{Activation, Form, FormTable};
use crate::geom::IndexElem;
use crate::page_stack::{self, leave_page, popover_groups, PageStack, PopoverPanel, TITLE_KEY};
use crate::panel_motion::PanelMotion;
use crate::popover::{self, Placement, Popover};
use crate::screen::{Activate, DrawFrame, GroupSpec, Scrim};
use crate::table::{TableView, MENU_MAX_W, MENU_MIN_W};
use crate::widgets::{panel_ground, Ground};
use crate::{theme, Rect};
use plx_machine::machine::{
    Cx, Edge, Effects, EntryId, FocusKey, GroupId, Handled, Host as MachineHost, InputKind, Key, Measure, ScreenEvent,
};

/// The ONE corner radius of a menu, whatever its host or class (it was 28, 24, 20 and a per-screen
/// constant; the Detail hosts all stood on 20, which is the value the Library's menus already have).
pub const RADIUS: f32 = 20.0;

/// **The size class of a glass**, fixed when the menu is built: an `Action` menu is a short list of
/// one-line commands (BODY labels, the lightest dim); a `Picker` lists choices with sub-lines (the track
/// menu's HEADLINE rows, the panel dim). One class per glass: a picker pushed into an action menu takes
/// the menu's class, so the hero `…` menu's Select Version page is BODY regular while the same page on
/// its standalone panel is HEADLINE bold.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    Action,
    Picker,
}

impl Class {
    /// `TableView::compact`: the class as the table reads it.
    const fn compact(self) -> bool {
        matches!(self, Class::Action)
    }

    /// The modal dim of this class (`theme::underlay`): the card and the shelf stay legible behind a
    /// compact action menu; a picker is the panel of the moment.
    const fn dim(self) -> f32 {
        match self {
            Class::Action => theme::underlay::DIM_COMPACT,
            Class::Picker => theme::underlay::DIM_PANEL,
        }
    }
}

/// **Where a menu stands, how it enters and what it stands on**: the legal combinations of ground,
/// entrance and placement as one enum, so an illegal one (a morph with no opener, glass over the video
/// plane, a sheet that grows) is not a value. Anchors are the drawn rect of what opened the menu as
/// `[x, y, w, h]` bits, like every panel argument (a canonical argument holds them without float
/// equality).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Host {
    /// **Grows out of its control** (a hero control, a header pill): the glass is the control's own,
    /// morphing from `pill` and back into it (`Popover::morphing` + `open_from`), laid OVER it
    /// ([`popover::cover_opener`]; clear of `clear_y` when the panel fits below it). `Ground::Menu`.
    /// `fill` is the control's focused face, which the close's blob whitens toward (`None`: the control
    /// is itself glass, as the profile chip is). `morph` says where the morph starts ([`Morph`]); `width` is
    /// the panel's width rule ([`GrowWidth`]).
    ///
    /// **Its height** is the table's, between 120 px and the room below the pill's top (to the foot of
    /// [`crate::consts::SAFE`]) when there is no `clear_y`: a page taller than that SCROLLS in a panel that
    /// stays top-flush with its pill, and never rides up over what stands above it (the tab bar, the page's
    /// header). With a `clear_y` (the Detail hero's: text above the pill) a panel too tall for the room below
    /// slides up inside the frame by its shortfall, as it always has.
    Grows { pill: Rect, clear_y: Option<f32>, fill: Option<[f32; 4]>, morph: Morph, width: GrowWidth },
    /// **Glass beside a card** (the tile's hold menu): fades in beside `anchor`, the card whole above
    /// the dim. `Ground::Menu`.
    BesideGlass { anchor: [u32; 4] },
    /// **A sheet hung under or above its opener** (Also available; a Detail page's Select Version or Audio &
    /// Subtitles with no control to grow from): fades and rises in, on the latched field. `Ground::Sheet`,
    /// never glass.
    Sheet { anchor: [u32; 4] },
    /// **A card over the video plane** (the playback failure screen's Audio & Subtitles picker today; the player's menus
    /// move onto it later): flat,
    /// rising in from the side of its opener it took, placed by `placement`. The player stays flat over video
    /// (`docs/liquid-glass.md` §7), with the player's width floor and the neutral dim that casts no artwork.
    Video { anchor: [u32; 4], placement: Placement },
}

/// **Where a growing glass starts its morph.** `FromPill` is the rect the host stood on when it opened
/// (a hero control: it never moves while the menu is up); `Fade` is `Popover::open_from(None)`, the
/// fade-and-scale entrance, for a control whose rect is read again every frame ([`Menu::track_opener`]:
/// the profile chip rides the bar's tuck), so the morph starts and ends on where the control IS drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Morph {
    FromPill,
    Fade,
}

/// **How wide a growing panel is**, a choice of its [`Host::Grows`] alone (no other host has a width rule to
/// name). Both are never narrower than the pill ([`TableView::menu_panel_width_from`]).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum GrowWidth {
    /// The table's own menu width, up to [`MENU_MAX_W`]: the localization catch for rows of OUR words.
    Catch,
    /// As wide as the widest row up to `ceiling`, so a name is read whole: for rows of USER-NAMED text (a list
    /// of libraries), which no catch can bound ([`TableView::menu_panel_width_to`]).
    Names { ceiling: f32 },
}

/// What a menu stands on, derived from its [`Host`] by one `match`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Surface {
    /// Glass on the frozen ground ([`Ground::Menu`]).
    Glass,
    /// The latched field under the frost ([`Ground::Sheet`]), never glass.
    Sheet,
    /// A flat card, no glass and no field: over the video plane.
    Flat,
}

impl Host {
    /// **Where a growing host's panel of `size` stands**: OVER its pill ([`popover::cover_opener`]), clear of
    /// `clear_y` when it fits below it ([`popover::cover_opener_clear`]). The one rule [`MenuCore`] lays a
    /// [`Host::Grows`] out by, public so a host's own rule (the Detail hero's) is tested on the production
    /// path and not on a copy. `None` for a host that does not grow.
    pub fn grown(&self, size: (f32, f32)) -> Option<Rect> {
        match self {
            Host::Grows { pill, clear_y, .. } => Some(grow_over(*pill, *clear_y, size)),
            _ => None,
        }
    }

    /// The narrowest a panel of this host is drawn ([`TableView::min_panel_w`]): the in-player menus are
    /// never a sliver of their labels.
    const fn width_floor(&self) -> f32 {
        match self {
            Host::Video { .. } => theme::layout::PLAYER_MENU_MIN_W,
            _ => MENU_MIN_W,
        }
    }

    /// Do the panel's edges all spring toward a new layout? Everything standing in the page does (a pushed
    /// page resizes the card from any side); the player's panels track their bottom and right anchors.
    const fn springs_all_edges(&self) -> bool {
        !matches!(self, Host::Video { .. })
    }
}

/// **A page ready to be shown**: its rows, its header and where focus opens. Not its size class: that is
/// the menu's. The root has no header, so a title is read only off a PUSHED page.
pub struct Page<Id, A, P> {
    pub form: Form<Id, A, P>,
    /// The header band's words (the back mark is drawn with them; there is no way to have none).
    pub title: String,
    /// The row focus opens on (the checked one), else the form's opening row.
    pub initial: Option<Id>,
}

impl<Id, A, P> Page<Id, A, P> {
    /// A pushed page.
    pub fn new(form: Form<Id, A, P>, title: impl Into<String>) -> Self {
        Self { form, title: title.into(), initial: None }
    }

    /// The root's rows (its header is never drawn).
    pub fn root(form: Form<Id, A, P>) -> Self {
        Self { form, title: String::new(), initial: None }
    }

    /// Focus opens on `id` when the page has it.
    pub fn initial(mut self, id: Option<Id>) -> Self {
        self.initial = id;
        self
    }
}

/// The page builder every transition takes: `None` asks for the root; the answer is `None` when that
/// page can no longer be built.
pub type Build<'a, P, Id, A> = dyn FnMut(Option<P>) -> Option<Page<Id, A, P>> + 'a;

/// Where the stack stands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stack {
    Root,
    /// `n >= 1` pages pushed over the root.
    Paged(usize),
}

/// What the panel is doing, read off its [`PanelMotion`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Motion {
    Rest,
    /// A page slide is running (the card resizes meanwhile).
    Sliding,
    /// Only the card is still moving toward its layout.
    Resizing,
}

/// The machine's whole state: [`Menu::state`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct State {
    pub stack: Stack,
    pub motion: Motion,
}

/// One input to the machine ([`Menu::step`]).
pub enum Event<'a, Id> {
    /// The container began to present the menu: a [`Host::Grows`] glass starts its morph from its pill.
    Opening,
    /// The surface was dismissed, however (BACK, a pick, the host's own press): the glass drains back.
    Closing,
    /// One frame: the springs, the cursor (`focus` is the focus engine's row key on this menu, if any)
    /// and the glass.
    Tick { dt: f32, measure: &'a dyn Measure, focus: Option<u32> },
    /// The focus engine moved to the row `key`: the cursor follows.
    Focused(u32),
    /// OK on the row `key`, or a click on it (the header's [`TITLE_KEY`] pops).
    Ok(u32),
    /// RIGHT on the row `key`: enters it when it is a Nav row.
    Right(u32),
    Back,
    Left,
    /// Replace the root's rows (the host's session changed what they are, or the player's tab switched):
    /// the stack is cleared and the root rebuilt, sliding in the direction `slide` says (`+1` / `-1`) or
    /// not at all.
    ReplaceRoot { slide: Option<f32> },
    /// Pop every page above `depth` in one slide.
    PopTo(usize),
    /// The data a page was built from changed: rebuild it in place, the cursor kept by identity (`keep`,
    /// else where it stands).
    Refresh { keep: Option<Id> },
}

/// **What a key means to a menu**: BACK, and LEFT / RIGHT (the menu's one group hands both to its host,
/// [`EdgeRule::Screen`](crate::screen::EdgeRule), so they arrive re-delivered with `at_edge` set). The
/// ONE place a host reads them: it answers `Handled::Yes` for [`Keyed::Event`] and [`Keyed::Swallow`]
/// (an unhandled re-delivery is a dead press), and passes [`Keyed::Pass`] on.
pub enum Keyed<Id> {
    /// Step the menu with this.
    Event(Event<'static, Id>),
    /// Consumed with no effect: a held LEFT or RIGHT must not walk through every page (only the press
    /// acts), and RIGHT with no row focused has nothing to enter.
    Swallow,
    /// Not the menu's key.
    Pass,
}

/// Read `kind` for a menu whose focused row key is `focus`.
pub fn keyed<K, Id>(kind: &InputKind<K>, focus: Option<u32>) -> Keyed<Id> {
    let InputKind::Key { key, edge, at_edge, .. } = kind else { return Keyed::Pass };
    match (key, at_edge) {
        (Key::Back, _) if *edge == Edge::Down => Keyed::Event(Event::Back),
        (Key::Left | Key::Right, true) if *edge != Edge::Down => Keyed::Swallow,
        (Key::Left, true) => Keyed::Event(Event::Left),
        (Key::Right, true) => focus.map_or(Keyed::Swallow, |row| Keyed::Event(Event::Right(row))),
        _ => Keyed::Pass,
    }
}

/// **The page builder of a menu with one page** (the version list, Also available): its root is installed by
/// [`Event::ReplaceRoot`] / [`Event::Refresh`] from the host's own closure, and no push or pop ever asks for another
/// (no row is a Nav row, and `P` is [`std::convert::Infallible`]), so every other step passes this.
pub fn no_pages<Id, A>(_: Option<std::convert::Infallible>) -> Option<Page<Id, A, std::convert::Infallible>> {
    None
}

/// What a step asks of the host.
#[derive(Clone, PartialEq, Debug)]
pub enum Out<A> {
    /// Nothing for the host to do.
    Stay,
    /// A page was pushed, popped or replaced (the host extends whatever it keeps alive while the viewer
    /// reads, e.g. the player's HUD timer).
    Moved,
    /// OK on a row that is neither Nav nor disabled: the host performs `A`.
    Picked(A),
    /// The menu asks to close.
    Dismiss,
}

/// **The compiled-once half of a menu**: where it stands (the host), what it stands on, its glass when
/// it has one, and the motion of its card. No generics, so a host's row types never multiply it.
pub struct MenuCore {
    mount: Mount,
    class: Class,
    motion: PanelMotion,
}

/// `size` over `pill`, clear of `clear_y` when there is one.
fn grow_over(pill: Rect, clear_y: Option<f32>, size: (f32, f32)) -> Rect {
    match clear_y {
        Some(y) => popover::cover_opener_clear(pill, size, crate::consts::SAFE, y),
        None => popover::cover_opener(pill, size, crate::consts::SAFE),
    }
}

/// A [`Host`] and the state that belongs to it alone: the glass of a [`Host::Grows`] is owned by that
/// variant, so a morph without a popover cannot be built.
enum Mount {
    Grows { pill: Rect, clear_y: Option<f32>, morph: Morph, width: GrowWidth, glass: Popover },
    BesideGlass { anchor: [u32; 4] },
    Sheet { anchor: [u32; 4] },
    /// `side` is the side of the anchor the layout chose, kept for the rise: a card hung above its opener
    /// rises from it, and the entrance reads the side the placer took.
    Video { anchor: [u32; 4], placement: Placement, side: std::cell::Cell<popover::Side> },
}

impl MenuCore {
    fn new(host: Host, class: Class) -> Self {
        let motion = if host.springs_all_edges() { PanelMotion::new().spring_all_edges() } else { PanelMotion::new() };
        let mount = match host {
            Host::Grows { pill, clear_y, fill, morph, width } => {
                let glass = Popover::new().morphing();
                let glass = match fill {
                    Some(fill) => glass.opener_fill(fill),
                    None => glass,
                };
                Mount::Grows { pill, clear_y, morph, width, glass }
            }
            Host::BesideGlass { anchor } => Mount::BesideGlass { anchor },
            Host::Sheet { anchor } => Mount::Sheet { anchor },
            Host::Video { anchor, placement } => Mount::Video { anchor, placement, side: std::cell::Cell::new(popover::Side::Above) },
        };
        Self { mount, class, motion }
    }

    /// **What the menu stands on: the ONE derivation**, by an exhaustive `match` on the mount (a fifth host
    /// does not compile until it says), read by [`Menu::surface`] and by the draw, which paints exactly the
    /// ground this names.
    fn surface(&self) -> Surface {
        match self.mount {
            Mount::Grows { .. } | Mount::BesideGlass { .. } => Surface::Glass,
            Mount::Sheet { .. } => Surface::Sheet,
            Mount::Video { .. } => Surface::Flat,
        }
    }

    fn glass(&self) -> Option<&Popover> {
        match &self.mount {
            Mount::Grows { glass, .. } => Some(glass),
            _ => None,
        }
    }

    fn glass_mut(&mut self) -> Option<&mut Popover> {
        match &mut self.mount {
            Mount::Grows { glass, .. } => Some(glass),
            _ => None,
        }
    }

    /// **The ONE width rule**: the table's own menu width ([`TableView::menu_panel_width`], clamped between
    /// its floor and [`MENU_MAX_W`]); a panel that grows out of a control is never narrower than the control's
    /// pill ([`TableView::menu_panel_width_from`]), and its [`GrowWidth`] may let a list of names grow past the
    /// catch. The height is the table's, between 120 px and the room the placement leaves; a growing panel's
    /// room is what lies below its pill's top ([`Host::Grows`]).
    fn layout(&self, table: &TableView, measure: &dyn Measure) -> Rect {
        let h = table.measured_height();
        match &self.mount {
            Mount::Grows { pill, clear_y, width, .. } => {
                let w = match width {
                    GrowWidth::Catch => table.menu_panel_width_from(measure, *pill),
                    GrowWidth::Names { ceiling } => table.menu_panel_width_to(measure, *pill, *ceiling),
                };
                let safe = crate::consts::SAFE;
                // a host with a `clear_y` has text above its pill that a panel too tall for the room below covers
                // by exactly its shortfall (the Detail hero's); every other one stays top-flush and scrolls
                let room = match clear_y {
                    Some(_) => safe.h,
                    None => (safe.y + safe.h - pill.y).clamp(120.0, safe.h),
                };
                grow_over(*pill, *clear_y, (w, h.clamp(120.0, room)))
            }
            Mount::BesideGlass { anchor } => beside(rect_of(*anchor), table.menu_panel_width(measure), h),
            Mount::Sheet { anchor } => popover::under_or_above_fit(rect_of(*anchor), table.menu_panel_width(measure), h),
            Mount::Video { anchor, placement, side } => {
                let h = h.clamp(120.0, crate::consts::SCR_H - 2.0 * popover::PANEL_EDGE);
                let placed = popover::anchor_panel(rect_of(*anchor), (table.menu_panel_width(measure), h), h, popover::PANEL_GAP, *placement);
                side.set(placed.side);
                placed.rect
            }
        }
    }

    /// The table's natural rect for the page showing, measured once per layout revision.
    fn natural(&self, table: &TableView, measure: &dyn Measure) -> Rect {
        self.motion.natural(table.layout_rev(), || self.layout(table, measure))
    }

    fn motion_kind(&self) -> Motion {
        if self.motion.sliding() {
            Motion::Sliding
        } else if self.motion.transitioning() {
            Motion::Resizing
        } else {
            Motion::Rest
        }
    }

    /// **The modal dim**: a growing glass's FOLLOWS its morph (`Popover::dim`); every other host's is the
    /// container's, at the class's weight (over the video plane: the neutral ink that carries no artwork).
    fn scrim(&self) -> Scrim {
        let weight = self.class.dim();
        match &self.mount {
            Mount::Grows { glass, .. } => glass.dim(weight),
            Mount::BesideGlass { .. } | Mount::Sheet { .. } => Scrim::dim(weight),
            Mount::Video { .. } => Scrim::over_video(theme::underlay::DIM_SHEET),
        }
    }

    /// **Paint the card and the page(s) in it, then register the focus stops**, once, whatever the host.
    /// The rows come from `table`; `key_at(i)` is the focus key of row `i` (`None`: not a stop).
    fn draw<H: MachineHost>(
        &self,
        f: &mut DrawFrame<'_, '_, H>,
        entry: EntryId,
        table: &TableView,
        key_at: &dyn Fn(usize) -> Option<u32>,
    ) where
        H::Elem: IndexElem,
    {
        let natural = self.natural(table, f.measure);
        let shown = self.motion.shown(natural);
        let appear = f.page_alpha;
        // What the card stands on is the surface's alone (`Self::surface`): glass and sheet name their ground, flat
        // paints none (the video plane is behind it).
        let ground = match self.surface() {
            Surface::Glass => Some(Ground::Menu),
            Surface::Sheet => Some(Ground::Sheet),
            Surface::Flat => None,
        };
        match &self.mount {
            // The glass and the rows ride the MORPH (`Popover::panel` / `morph_rows`), not the container's
            // appear: the panel begins and ends as the control it grows from.
            Mount::Grows { glass, .. } => {
                let m = glass.content_painter_on(f.painter.alpha(f.nav_page_alpha), 0.0);
                if let Some(ground) = ground {
                    glass.panel(m, shown, RADIUS, f.underlay, ground);
                }
                glass.morph_rows(m, shown, RADIUS, |m| self.motion.draw_rows(m, natural, table, f.measure));
            }
            mount => {
                // beside a card the glass fades in where it stands; every other card rises into place, from the
                // side of its opener it hangs on (a card over the opener's side the placer chose)
                let p = match mount {
                    Mount::Video { side, .. } => {
                        let (dx, dy) = Popover::rise_offset(side.get(), Popover::RISE, appear);
                        f.painter.alpha(appear).translate(dx, dy)
                    }
                    Mount::Sheet { .. } => f.painter.alpha(appear).translate(0.0, Popover::RISE * (1.0 - appear)),
                    _ => f.painter.alpha(appear),
                };
                match ground {
                    Some(ground) => {
                        panel_ground(p, shown, RADIUS, f.underlay, ground);
                        self.motion.draw_rows(p, natural, table, f.measure);
                    }
                    None => self.motion.draw(p, natural, RADIUS, table, f.measure),
                }
            }
        }
        // the stops: one registration for every popover (`page_stack::register_stops`), through the page's own
        // alpha; only the ACTIVE page registers, where it is drawn this frame
        page_stack::register_stops(
            f,
            entry,
            f.painter.alpha(f.page_alpha),
            Activate::Immediate,
            table,
            natural,
            shown,
            self.motion.live_dx(),
            key_at,
        );
    }
}

/// A transition that did something is [`Out::Moved`].
fn moved<A>(did: bool) -> Out<A> {
    if did {
        Out::Moved
    } else {
        Out::Stay
    }
}

/// `[x, y, w, h]` bits as a rect.
fn rect_of(bits: [u32; 4]) -> Rect {
    let [x, y, w, h] = bits.map(f32::from_bits);
    Rect::new(x, y, w, h)
}

/// Keep-out from the screen edges, so a menu beside a card low on the screen still gets a whole panel:
/// per AXIS, the overscan frame's (vertically `space::XL` clears `MARGIN_Y`; across, the panel opened over
/// the last column lands on `MARGIN_X`).
const EDGE: f32 = theme::space::XL;
const EDGE_X: f32 = crate::consts::MARGIN_X;
/// Air between a card's drawn edge and the panel beside it: one `space` rung, like every other gap.
const CARD_GAP: f32 = theme::space::MD;

/// **Beside the card, never over it**: to its RIGHT by default, flipped to its LEFT when that would run
/// past the screen's keep-out; vertically it hangs off the card's top edge, pulled back inside the safe
/// band. Pure (anchor + size in, rect out), which is what makes the rule host-testable.
pub fn beside(a: Rect, content_w: f32, content_h: f32) -> Rect {
    use crate::consts::{SCR_H, SCR_W};
    let width = content_w.clamp(MENU_MIN_W, MENU_MAX_W);
    let h = content_h.clamp(120.0, SCR_H - 2.0 * EDGE);
    let right = a.x + a.w + CARD_GAP;
    let x = if right + width <= SCR_W - EDGE_X { right } else { a.x - CARD_GAP - width };
    let x = x.clamp(EDGE_X, (SCR_W - EDGE_X - width).max(EDGE_X));
    let y = a.y.clamp(EDGE, (SCR_H - EDGE - h).max(EDGE));
    Rect::new(x, y, width, h)
}

/// **A menu**: the [`MenuCore`], the form of the page showing and the stack of pages pushed over the
/// root. `P` names a page (the row kind `RowKind::Nav(P)` carries it), `Id` a row, `A` what a row does.
pub struct Menu<P: Copy, Id, A> {
    core: MenuCore,
    form: FormTable<Id, A, P>,
    stack: PageStack<P, Id>,
}

impl<P: Copy, Id: PartialEq + Clone, A: Clone> Menu<P, Id, A> {
    /// A menu of `class` standing at `host`, whose rows are keyed below `band` (the page's key ceiling),
    /// with an empty root: install it with [`Event::ReplaceRoot`].
    pub fn new(host: Host, class: Class, band: u32) -> Self {
        let mut form = FormTable::new(band);
        form.table.compact = class.compact();
        form.table.min_panel_w = host.width_floor();
        Self { core: MenuCore::new(host, class), form, stack: PageStack::new() }
    }

    /// **The menu a page header's choice control opens** (the Watchlist's Sort & Filter, Live TV Browse's filter): an
    /// [`Class::Action`] menu of compact one-word choices whose glass grows out of the control's pill at `pill` and covers it
    /// top edge to top edge, the close's blob whitening toward the pill's own focused fill. It draws no pill: the control is the
    /// host page's, painted by its opener lift. `width` is the panel's width rule: [`GrowWidth::Catch`] for rows of OUR
    /// words, [`GrowWidth::Names`] for rows of user-named text (the Library's "+N" list of libraries).
    pub fn from_header_pill(pill: Rect, band: u32, width: GrowWidth) -> Self {
        let host = Host::Grows {
            pill,
            clear_y: None,
            fill: Some(crate::page_header::OPENER_FILL),
            morph: Morph::FromPill,
            width,
        };
        Self::new(host, Class::Action, band)
    }

    // ---- what is derived ------------------------------------------------------------------------

    /// The machine's whole state.
    pub fn state(&self) -> State {
        State {
            stack: if self.stack.is_empty() { Stack::Root } else { Stack::Paged(self.stack.len()) },
            motion: self.core.motion_kind(),
        }
    }

    /// The pages pushed over the root, outermost first, each with the row that opened it and the scroll it was
    /// left at (for a host whose replay canon writes them).
    pub fn pages(&self) -> impl Iterator<Item = &page_stack::Saved<P, Id>> {
        self.stack.iter()
    }

    /// How many pages are pushed over the root.
    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    /// The page showing, `None` at the root.
    pub fn top(&self) -> Option<P> {
        self.stack.top()
    }

    pub fn class(&self) -> Class {
        self.core.class
    }

    /// What the panel stands on.
    pub fn surface(&self) -> Surface {
        self.core.surface()
    }

    /// The header the page showing wears (its title, drawn with the back mark); none at the root.
    pub fn header(&self) -> Option<&str> {
        self.form.table.title()
    }

    /// The pointer waits while the card or the pages move (`Screen::pointer_held`).
    pub fn pointer_held(&self) -> bool {
        self.core.motion_kind() != Motion::Rest
    }

    /// The modal dim (`Screen::scrim`).
    pub fn scrim(&self) -> Scrim {
        self.core.scrim()
    }

    /// The control the glass grew out of is CONSUMED by the morph: gone while the panel stands, back as it
    /// drains (`Screen::opener_alpha`). Whole for a host with no morph.
    pub fn opener_alpha(&self) -> f32 {
        self.core.glass().map_or(1.0, Popover::opener_alpha)
    }

    /// The droplet has drained into its control: the surface and the lift end on this frame
    /// (`Screen::handed_back`). Never for a host with no morph.
    pub fn handed_back(&self) -> bool {
        self.core.glass().is_some_and(Popover::handed_back)
    }

    /// The stack's part of the host's replay canon, whole: the depth, then each pushed page's stable `code`
    /// and the key of the row that opened it. Byte for byte what `PageStack::canon` writes, the player's
    /// popovers' own (so a host that moves over from one keeps its recorded canon).
    pub fn stack_canon(&self, c: &mut plx_machine::machine::Canon, code: impl Fn(P) -> u32, key: impl Fn(&Id) -> u32) {
        self.stack.canon(c, code, key);
    }

    /// The depth alone, for a host whose canon never wrote what is pushed (its own page ids are its canon's
    /// business): the Detail hosts', whose recorded form is that.
    pub fn depth_canon(&self, c: &mut plx_machine::machine::Canon) {
        c.u32(self.stack.len() as u32);
    }

    /// **Where the control the glass grows out of is drawn NOW**: a [`Host::Grows`] whose control moves while
    /// the menu is up (the profile chip rides the bar's tuck) starts and ends its morph on the live rect, and
    /// the panel is laid over it. Call before every draw; inert for a host that does not grow.
    pub fn track_opener(&mut self, drawn: Rect) {
        let MenuCore { mount, motion, .. } = &mut self.core;
        if let Mount::Grows { pill, glass, .. } = mount {
            // the panel is laid over the pill, so the rect measured for the old one is stale
            if *pill != drawn {
                *pill = drawn;
                motion.forget_natural();
            }
            glass.track_opener(drawn);
        }
    }

    /// The side of its opener a video host's card took (`None` for any other host).
    #[cfg(test)]
    pub(crate) fn core_side(&self) -> Option<popover::Side> {
        match &self.core.mount {
            Mount::Video { side, .. } => Some(side.get()),
            _ => None,
        }
    }

    /// The rect the glass is morphing from, if it has one.
    #[cfg(test)]
    pub(crate) fn glass_opener(&self) -> Option<Rect> {
        self.core.glass().and_then(Popover::opener)
    }

    /// The growing glass, for the tests that hold its morph to the scrim's clock (`None`: a host that does not grow).
    pub fn glass(&self) -> Option<&Popover> {
        self.core.glass()
    }

    /// The form of the page showing.
    pub fn form(&self) -> &FormTable<Id, A, P> {
        &self.form
    }

    /// The form, for a test that arranges rows without a build.
    #[cfg(any(test, feature = "test-support"))]
    pub fn form_mut(&mut self) -> &mut FormTable<Id, A, P> {
        &mut self.form
    }

    /// The panel's natural (layout) rect; the cached measure.
    pub fn panel_rect(&self, measure: &dyn Measure) -> Rect {
        self.core.natural(&self.form.table, measure)
    }

    /// The card as drawn this frame: the layout, followed by the card's resize motion.
    pub fn shown_rect(&self, measure: &dyn Measure) -> Rect {
        self.core.motion.shown(self.panel_rect(measure))
    }

    // ---- the frame's seams ----------------------------------------------------------------------

    /// **Text prewarm, on every page**: the strings of the page showing are queued once per layout
    /// (`PanelMotion::warm_open`), so a freshly pushed or popped page meets no cold string in its draw.
    /// Call from `Screen::prepare`, which asks every presenting frame (a layout already walked costs one
    /// `Cell` read).
    pub fn prepare(&self, measure: &dyn Measure) {
        self.core.motion.warm_open(&self.form.table, || self.panel_rect(measure), measure);
    }

    /// **Paint the card, its page(s) and the stops** (`Screen::draw`).
    pub fn draw<H: MachineHost>(&self, f: &mut DrawFrame<'_, '_, H>, entry: EntryId)
    where
        H::Elem: IndexElem,
    {
        self.core.draw(f, entry, &self.form.table, &|i| self.form.key_at(i).map(|k| k.0));
    }

    /// The menu's one `Column` group: UP/DOWN stop at the ends, LEFT (pop a page) and RIGHT (enter a Nav
    /// row) are the host's, delivered as [`Event::Left`] / [`Event::Right`].
    pub fn groups(&self, group: GroupId, measure: &dyn Measure, out: &mut Vec<GroupSpec>) {
        popover_groups(self, group, measure, out);
    }

    /// Where focus seats on entering the menu: the cursor's row, else the opening row.
    pub fn seat(&self, entry: EntryId) -> FocusKey<u32> {
        crate::page_stack::popover_seat(self, entry)
    }

    /// `want` when it is on the page showing, else the seat (a page that was just left for another: the
    /// ring goes to the row the form restored, the one that opened the page).
    pub fn reconcile(&self, entry: EntryId, want: FocusKey<u32>, group: GroupId) -> FocusKey<u32> {
        if self.group_of(&want.elem, group).is_some() {
            want
        } else {
            self.seat(entry)
        }
    }

    /// The group a row key belongs to (none for the header's pointer-only key).
    pub fn group_of(&self, key: &u32, group: GroupId) -> Option<GroupId> {
        crate::page_stack::popover_group_of(self, key, group)
    }

    /// UP/DOWN step through the form's focusable rows; LEFT/RIGHT are the host's.
    pub fn neighbour(&self, entry: EntryId, key: FocusKey<u32>, dir: crate::screen::Dir) -> crate::screen::Step<u32> {
        crate::page_stack::popover_neighbour(self, entry, key, dir)
    }

    /// Where a stop is DRAWN (replay compares it with what [`Self::draw`] registers).
    pub fn place(&self, key: &u32, measure: &dyn Measure) -> Option<crate::screen::Placed> {
        crate::page_stack::popover_place(self, key, measure)
    }

    /// The identity of the row under focus key `key`, on the page showing.
    pub fn id_of(&self, key: u32) -> Option<&Id> {
        self.form.index_of_key(crate::form::RowKey(key)).and_then(|i| self.form.id_at(i))
    }

    /// The rows a focus stop exists for, in row order.
    pub fn focusable(&self) -> impl Iterator<Item = u32> + '_ {
        (0..self.form.table.n_rows() as usize).filter_map(|i| self.form.key_at(i).map(|k| k.0))
    }

    // ---- the machine ----------------------------------------------------------------------------

    /// **The one transition function.** `build` answers a page (`None` asks for the root).
    pub fn step(&mut self, ev: Event<'_, Id>, build: &mut Build<'_, P, Id, A>) -> Out<A> {
        match ev {
            Event::Opening => {
                if let Mount::Grows { pill, morph, glass, .. } = &mut self.core.mount {
                    glass.open_from((*morph == Morph::FromPill).then_some(*pill));
                }
                Out::Stay
            }
            Event::Closing => {
                if let Some(glass) = self.core.glass_mut() {
                    glass.dismiss();
                }
                Out::Stay
            }
            Event::Tick { dt, measure, focus } => {
                self.tick(dt, measure, focus);
                Out::Stay
            }
            Event::Focused(key) => {
                if let Some(i) = self.form.index_of_key(crate::form::RowKey(key)) {
                    self.form.table.sel = i as i32;
                }
                Out::Stay
            }
            Event::Ok(key) => self.ok(key, false, build),
            Event::Right(key) => self.ok(key, true, build),
            // BACK pops, and closes where there is nothing to pop to: at the root, and on a page whose every
            // page beneath (the root too) can no longer be built, where the menu has nowhere to stand
            Event::Back => match self.stack.len() {
                0 => Out::Dismiss,
                n if self.pop_to(n - 1, build) => Out::Moved,
                _ => Out::Dismiss,
            },
            Event::Left => match self.stack.len() {
                0 => Out::Stay,
                n => moved(self.pop_to(n - 1, build)),
            },
            Event::PopTo(depth) => moved(depth < self.stack.len() && self.pop_to(depth, build)),
            Event::ReplaceRoot { slide } => moved(self.replace_root(slide, build)),
            Event::Refresh { keep } => {
                self.refresh(keep, build);
                Out::Stay
            }
        }
    }

    fn tick(&mut self, dt: f32, measure: &dyn Measure, focus: Option<u32>) {
        let natural = self.panel_rect(measure);
        self.core.motion.step(dt, natural);
        if let Some(glass) = self.core.glass_mut() {
            glass.update(dt);
        }
        if let Some(i) = focus.and_then(|key| self.form.index_of_key(crate::form::RowKey(key))) {
            self.form.table.sel = i as i32;
        }
        self.form.table.update(dt, self.panel_rect(measure).h);
    }

    /// OK on a row, a click on it, or RIGHT (`enter_only`: only a Nav row responds).
    fn ok(&mut self, key: u32, enter_only: bool, build: &mut Build<'_, P, Id, A>) -> Out<A> {
        if key == TITLE_KEY {
            return match self.stack.len() {
                0 => Out::Stay,
                n => moved(self.pop_to(n - 1, build)),
            };
        }
        let Some(index) = self.form.index_of_key(crate::form::RowKey(key)) else {
            // a key that names no row (the separator has none) only closes the menu
            return if enter_only { Out::Stay } else { Out::Dismiss };
        };
        // a DISABLED row does nothing and the menu stays up, for OK, RIGHT and a click alike
        let Some(activation) = self.form.activate(index) else { return Out::Stay };
        match activation {
            Activation::Push(page) => {
                let Some(opener) = self.form.id_at(index).cloned() else { return Out::Stay };
                moved(self.push(page, opener, build))
            }
            Activation::Action(_) if enter_only => Out::Stay,
            Activation::Action(action) => Out::Picked(action),
        }
    }

    /// **The ONE push**: remember the opener and the scroll, move the page that is showing out of the form
    /// (the slide draws it once more), open `page` on its initial row and start the slide. The card resizes
    /// toward the page's rect meanwhile.
    fn push(&mut self, page: P, opener: Id, build: &mut Build<'_, P, Id, A>) -> bool {
        let Some(built) = build(Some(page)) else { return false };
        self.stack.push(page, opener, self.form.table.scroll_pos());
        let leaving = leave_page(&mut self.form);
        self.form.open(built.form, built.initial.as_ref());
        self.form.table.set_title(Some(built.title));
        self.core.motion.begin_slide(leaving, 1.0);
        true
    }

    /// **The ONE pop**: restore the page at `depth` (`0` = the root) with focus on the row that opened the
    /// page above it and the scroll it was left at, sliding back. A page that can no longer be built (its data
    /// went) is not stood on: the pop keeps going down to the first depth that can, the root last; `false`
    /// when none can, and the menu is left as it was (the host closes it).
    fn pop_to(&mut self, depth: usize, build: &mut Build<'_, P, Id, A>) -> bool {
        let mut depth = depth.min(self.stack.len());
        let (built, saved) = loop {
            let below = depth.checked_sub(1).and_then(|i| self.stack.page_at(i));
            if let Some(built) = build(below) {
                break (built, self.stack.truncate(depth));
            }
            match depth {
                0 => return false,
                _ => depth -= 1,
            }
        };
        let Some(saved) = saved else { return false };
        let leaving = leave_page(&mut self.form);
        self.form.restore(built.form, Some(&saved.return_id), saved.scroll);
        self.form.table.set_title((depth > 0).then_some(built.title));
        self.core.motion.begin_slide(leaving, -1.0);
        true
    }

    fn replace_root(&mut self, slide: Option<f32>, build: &mut Build<'_, P, Id, A>) -> bool {
        let Some(root) = build(None) else { return false };
        self.stack.clear();
        let leaving = leave_page(&mut self.form);
        self.form.open(root.form, root.initial.as_ref());
        self.form.table.set_title(None);
        match slide {
            Some(dir) => self.core.motion.begin_slide(leaving, dir),
            None => self.core.motion.cancel_slide(),
        }
        true
    }

    /// Rebuild the page showing in place. A page that can no longer be built is dropped with NO slide, and
    /// the page beneath takes focus on the row that opened it (found by identity in the rebuilt form).
    fn refresh(&mut self, keep: Option<Id>, build: &mut Build<'_, P, Id, A>) {
        let mut depth = self.stack.len();
        while depth > 0 && self.stack.page_at(depth - 1).is_some_and(|page| build(Some(page)).is_none()) {
            depth -= 1;
        }
        if depth < self.stack.len() {
            let below = depth.checked_sub(1).and_then(|i| self.stack.page_at(i));
            let Some(built) = build(below) else { return };
            let Some(saved) = self.stack.truncate(depth) else { return };
            self.core.motion.cancel_slide();
            self.form.restore(built.form, Some(&saved.return_id), saved.scroll);
            self.form.table.set_title((depth > 0).then_some(built.title));
            return;
        }
        let Some(built) = build(self.stack.top()) else { return };
        let held = keep.or_else(|| self.form.selected_id().cloned());
        match self.core.class {
            // an action menu must never slide onto a neighbour that may be destructive (*Sign out* once
            // *Change profile* is hidden): a row that is gone opens the page afresh on its initial row
            Class::Action => self.form.set_sliding_or_open(built.form, held.as_ref(), built.initial.as_ref()),
            Class::Picker => self.form.set_sliding(built.form, held.as_ref()),
        }
        self.form.table.set_title((!self.stack.is_empty()).then_some(built.title));
    }
}

// ---- the screen glue: what every host of a menu would otherwise write for itself -----------------------

/// What [`Menu::on_screen`] made of a screen event: the menu's answer, the row key the event named (an OK or a
/// click: the host reads the row a pick was on from it), and whether the event was consumed.
pub struct Glued<A> {
    pub out: Out<A>,
    pub elem: Option<u32>,
    pub handled: Handled,
}

impl<P: Copy, Id: PartialEq + Clone, A: Clone> Menu<P, Id, A> {
    /// **The ONE adapter from a screen's events to the menu's** (Mount, Closing, Tick, FocusMoved, an OK or a
    /// click, a pressed key): a host calls it first, performs [`Glued::out`] (a dismissal closes the surface, a
    /// pick acts) and answers [`Glued::handled`]. `None` is an event that is not the menu's (a key it passes, any
    /// other event): the host's own. The menu's focus is the engine's current key on `entry`; the Tick carries
    /// it so the cursor follows a focus change the screen was not told about, and a moved focus damages the
    /// frame, so no host can forget either.
    pub fn on_screen<H: MachineHost>(
        &mut self,
        ev: &ScreenEvent<H>,
        cx: &Cx<'_, H>,
        entry: EntryId,
        fx: &mut Effects<'_, H>,
        build: &mut Build<'_, P, Id, A>,
    ) -> Option<Glued<A>>
    where
        H::Elem: IndexElem,
    {
        let focus = cx.focus.current.filter(|key| key.entry == entry).and_then(|key| key.elem.index());
        let (menu_ev, elem, handled) = match ev {
            ScreenEvent::Mount => (Event::Opening, None, Handled::No),
            ScreenEvent::Closing => (Event::Closing, None, Handled::No),
            ScreenEvent::Tick(tick) => (Event::Tick { dt: tick.dt(), measure: cx.measure, focus }, None, Handled::No),
            ScreenEvent::FocusMoved { to, .. } => {
                fx.invalidate(plx_machine::present::Provenance::Input);
                (Event::Focused(to.elem.index()?), None, Handled::Yes)
            }
            ScreenEvent::Activate(elem) => (Event::Ok(elem.index()?), elem.index(), Handled::Yes),
            ScreenEvent::PressCommit(_) => {
                let key = cx.focus.current.and_then(|key| key.elem.index());
                match key {
                    Some(key) => (Event::Ok(key), Some(key), Handled::Yes),
                    None => return Some(Glued { out: Out::Stay, elem: None, handled: Handled::Yes }),
                }
            }
            ScreenEvent::Input(input) => match keyed(&input.kind, focus) {
                Keyed::Event(ev) => (ev, None, Handled::Yes),
                Keyed::Swallow => return Some(Glued { out: Out::Stay, elem: None, handled: Handled::Yes }),
                Keyed::Pass => return None,
            },
            _ => return None,
        };
        let out = self.step(menu_ev, build);
        Some(Glued { out, elem, handled })
    }
}

/// **A screen that is a menu's host**: the menu and the entry it registers its stops under. With
/// [`menu_screen_methods!`] it is the whole of a host's `Screen` surface for the menu (the modal dim, the
/// prewarm, the pointer hold, the draw and the morph's hand-over), so a host does not forward them one by
/// one, and cannot forget one (an unforwarded `pointer_held` is a pointer that acts on a card in motion).
pub trait HasMenu {
    type Page: Copy;
    type Row: PartialEq + Clone;
    type Act: Clone;
    fn menu(&self) -> &Menu<Self::Page, Self::Row, Self::Act>;
    fn menu_entry(&self) -> EntryId;
}

/// The `Screen` methods a menu's host takes from the menu, in the `impl Screen<H> for Host` block
/// (`H` the impl's host type parameter, `$phase` the draw's profile label): `opener_alpha`, `handed_back`,
/// `scrim`, `prepare`, `pointer_held`, `draw`. The host implements [`HasMenu`].
#[macro_export]
macro_rules! menu_screen_methods {
    ($H:ident, $phase:literal) => {
        fn opener_alpha(&self) -> f32 {
            $crate::menu::HasMenu::menu(self).opener_alpha()
        }
        fn handed_back(&self) -> bool {
            $crate::menu::HasMenu::menu(self).handed_back()
        }
        fn scrim(&self) -> $crate::screen::Scrim {
            $crate::menu::HasMenu::menu(self).scrim()
        }
        fn prepare(&mut self, _: &mut $crate::frame::Budget, cx: &plx_machine::machine::Cx<'_, $H>) {
            $crate::menu::HasMenu::menu(self).prepare(cx.measure);
        }
        fn pointer_held(&self) -> bool {
            $crate::menu::HasMenu::menu(self).pointer_held()
        }
        fn draw(&mut self, f: &mut $crate::screen::DrawFrame<'_, '_, $H>) {
            let entry = $crate::menu::HasMenu::menu_entry(self);
            $crate::profile::phase($phase, || $crate::menu::HasMenu::menu(self).draw(f, entry))
        }
    };
}

impl<P: Copy, Id: PartialEq + Clone, A: Clone> PopoverPanel for Menu<P, Id, A> {
    type Id = Id;
    type Act = A;
    type Dest = P;
    fn form(&self) -> &FormTable<Id, A, P> {
        &self.form
    }
    fn motion(&self) -> &PanelMotion {
        &self.core.motion
    }
    fn panel_rect(&self, measure: &dyn Measure) -> Rect {
        Menu::panel_rect(self, measure)
    }
    fn shown_rect(&self, measure: &dyn Measure) -> Rect {
        Menu::shown_rect(self, measure)
    }
}
