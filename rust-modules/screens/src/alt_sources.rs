//! **Also available** — the same film held by more than one source, as a list you can walk into.
//!
//! Deliverable E of the Shared Sources design (`docs/shared-servers.md` §6): when a second pinned
//! source also holds the item on screen, the detail page's actions row grows an *Also available*
//! pill with a trailing chevron, and it opens this panel — a one-page `ui::menu::Menu` hung under or above that
//! pill (`Host::Sheet`: the latched field, never glass), whose rows this module declares.
//!
//! **A `Style::Compact` surface on the container tree** since restructure phase 10 (§6.2): the
//! anchored menu the Library's chips already are. The container owns its PHASE and its appear
//! spring; the surface owns input while it is `Opening | Open`; and its ANCHOR — the drawn rect of
//! the pill that opened it — rides on its [`AltSourcesArg`] exactly as `LibraryMenuArg`'s does,
//! which is what retired a `static mut ANCHOR: PanelAnchor` holding a `Rect` (a geometry static is
//! never an allowlistable render cache, §15.2).
//!
//! Its rows use all FOUR of that cell's content places — the leading mark, the label over its
//! sub-line, the trailing read-out, and the right-aligned badge:
//!
//! | mark | label | sub-line | read-out | badge |
//! |---|---|---|---|---|
//! | ✓ | `Movies`    | `This account` | `1h 57m` | `1080p` |
//! |   | `Film Club` | `friend`       | `1h 57m` | `4K`    |
//!
//! # The four calls this module implements, and why each is not the obvious thing
//!
//! **A CONTROL in the actions row, not a section of the page.** An item on several servers is a
//! rare fact; a permanent band would spend a whole row on nothing on every other page in the
//! library. So the list is behind a button, and the button itself is drawn ONLY when a second
//! pinned source holds this item — the same "with one source, none of this is drawn" rule the
//! shelf heading's annotation implements as absence rather than as a branch that draws nothing
//! visible. **That gate is `metadata::alt_available`, asked by the PAGE about its own item**, and
//! it is deliberately not a question about this panel: a screen must never have to mount a surface
//! to find out how many columns its own footer has.
//!
//! **The tick marks the copy you are on now**, and the badge is the RESOLUTION class — the
//! vocabulary the hero's badges row already speaks (`ui::fmt::resolution`, one spelling of "4K" in
//! the product). A row is `[library] [owner · runtime] [class]`: the label answers *which library*,
//! the sub-line *whose*, the badge *what you would get*.
//!
//! **OK NAVIGATES.** It opens that server's own page for the film; it does not swap the copy under
//! you. That is also what settles the per-server progress problem: **a RESUME POSITION does not
//! travel between servers** — it is about a file you are streaming from one host — so a design that
//! switched the copy in place would have had to explain a resume position that jumped. Navigating
//! makes it self-evident: you are on a different page, and it says where you are in *that* copy.
//! Like every other menu here this surface only REPORTS the choice; it delivers
//! [`AppMsg::AltSourceOpen`] to the Detail instance named by its argument and the PAGE performs the
//! navigation, which is `LibraryMenu`'s shape and keeps one owner of "what a press means here".
//!
//! This paragraph said "watch state does not travel between servers" flat, and half of that stopped
//! being true when [`plx_data::viewstate`] began fanning a Mark as Watched out over every source
//! holding the guid. The two halves are not the same claim and the split is the whole point: the
//! **watched flag follows the TITLE** (having seen a film is not a fact about a host), while the
//! **offset stays with the copy**. Nothing here changed — this panel still navigates rather than
//! swaps, for the reason above, which is about the offset.
//!
//! **Ordering is the copy that PLAYS first**, then what a viewer would prefer: higher quality
//! first, and at equal quality yours before a friend's, because yours cannot go offline mid-film.
//! The design's own example pins the first clause against the second — a 1080p copy you are
//! standing on sorts above a friend's 4K one.
//!
//! Explicitly rejected, and not to be re-introduced: merging the copies invisibly and playing the
//! best one (it is what makes a film disappear when a friend goes offline, with no way to see why);
//! a permanent section on the page; switching the copy silently on OK.
//!
//! # Where the copies come from
//!
//! **The identity is the `guid` (`plex://movie/…`), never the `ratingKey`.** Every item-shaped
//! integer in Plex is server-local and starts at 1, so both servers in this household have a
//! `ratingKey` 4 and a section 1 (`docs/shared-servers.md` §1). Matching on one would confidently
//! offer a different film — which is why the STORE is ADDRESSED on the `(server, ratingKey)` PAIR:
//! a resolve outliving the page that asked for it is the normal case, not the exotic one. The
//! store, the cross-source resolve that fills it and the headless `/tmp/plxnative-shared` stand-in
//! all live in `plx_data::metadata`, beside the detail fetch whose landing kicks them; this module
//! ORDERS, MARKS and DRAWS and owns nothing else.
//!
//! **Rows are a SNAPSHOT, refreshed against a stamp.** `TableView` is built once and drawn from,
//! so a correction reaching the store — a re-described source's credit, a revoked share — has to
//! be carried the last step by rebuilding. The stamp is the row list itself, so a rebuild happens
//! exactly when the drawn content would differ, and the ORDER moves with the text (the credit is
//! the own-before-a-friend's tiebreak, so a corrected panel can legitimately swap two of its rows).
//! The cursor is KEPT across a rebuild rather than re-seated on the tick: the rows can re-order
//! beneath it, which is a smaller surprise than the cursor jumping mid-interaction.

use std::borrow::Cow;
use std::convert::Infallible;

use plx_data::metadata::AltCopy;
use plx_plex::plex::ServerId;
use crate::registry::{AppLike, AppMsg, ContentArg, PageMemory};
use plx_ui::a11y::{A11y, A11yPath};
use plx_machine::machine::{
    Canon, Cx, Delivery, Effects, EntryId, FocusKey, Fx, GroupId, Handled, InstanceId, LogicalState, Machine,
    MachineId, NavOp,
};
use plx_ui::screen::{
    At, Dir, FocusSource, Focusable, GroupSpec, HitSource, Placed, RenderStrategy, Screen,
    ScreenEvent, Step,
};
use plx_ui::form::{Form, FormSection, RowKey, RowKind};
use plx_ui::menu::{no_pages, Class, Event, Host, Menu, Out, Page};
use plx_ui::table::{Badge, Row, TableView};
use plx_ui::Rect;

/// The fields [`AltSourcesScreen::write`] canonicalises, for the recorder's shape pin (§5.4). The
/// selected ROW is in it deliberately: this panel's UP/DOWN changes nothing else in the app, so
/// without it a replay grades the panel opening and closing and nothing between.
pub const SHAPE: [&str; 2] = [
    "AltSourcesScreen{arg:AltSourcesArg{host:u32,sid:u32,rk:str,anchor:[u32;4]},rows:[{sid:u32,rk:str,label:str,detail:str,value:opt<str>,badge:opt<str>,checked:bool}],sel:i32,table:TableViewMotion}",
    TableView::MOTION_SHAPE,
];

/// What the container is asked to present.
///
/// `host` is the Detail instance the choice is reported back to; it rides on the argument rather
/// than being looked up, for `LibraryMenuArg`'s reason — the entry outlives any one frame's idea of
/// which page is on top. `sid`/`rk` are the copy the page is STANDING ON: the tick's other half,
/// and the pair the addressed store is read with.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AltSourcesArg {
    pub host: InstanceId,
    pub sid: ServerId,
    pub rk: String,
    /// Bit-preserving rest rectangle of the pill that opened it; valid in canonical arguments
    /// without float equality.
    pub anchor: [u32; 4],
}

impl LogicalState for AltSourcesArg {
    fn write(&self, c: &mut Canon) {
        c.u32(self.host.0).u32(u32::from(self.sid.raw())).str(&self.rk);
        for value in self.anchor {
            c.u32(value);
        }
    }
    fn probe(&self, out: &mut String) {
        out.push_str("alt_sources_arg");
    }
}

// ---- the model (pure) ------------------------------------------------------------------------

/// A copy's quality, as SCAN LINES — the ordering's second key.
///
/// It reads the same three fields `ui::fmt::resolution` badges the row with, in the same
/// precedence (the server's own class first, the stored frame size only as its fallback), so the
/// ladder the eye sees on the badges and the order the rows are in cannot disagree. Saturating,
/// because these are wire values through the lenient `de_i64` and a garbage 19-digit height must
/// not overflow into a top-of-list sort key.
fn scan_lines(c: &AltCopy) -> i64 {
    let r = c.res.trim().to_ascii_lowercase();
    match r.as_str() {
        "" => {}
        "8k" => return 4320,
        "4k" => return 2160,
        "sd" => return 480,
        _ if r.chars().all(|ch| ch.is_ascii_digit()) => return r.parse::<i64>().unwrap_or(0),
        _ => {}
    }
    if c.height > 0 {
        c.height
    } else {
        c.width.saturating_mul(9) / 16
    }
}

/// One drawn row, resolved: what it says and where OK goes. The panel builds its `TableView` from
/// these and nothing else, so the ordering, the tick and the destination map are ONE list.
#[derive(Clone, Debug, PartialEq)]
pub struct AltRow {
    /// the library, on that source
    pub label: String,
    /// whose copy it is: `"This account"` / `"friend"`
    pub detail: String,
    /// `detail` is this app's own "This account" word (`true`), not another person's name (`false`,
    /// user text) — which of the two the sub-line's text-fit check may hold to account.
    pub own_detail: bool,
    /// the runtime, for the row's trailing read-out — `None` for a copy the server sent no
    /// duration for, which then says nothing rather than "0m"
    pub value: Option<String>,
    /// the resolution class, or `None` for a copy the server described no video for
    pub badge: Option<String>,
    /// the copy the page is on NOW — the picker's tick
    pub checked: bool,
    pub sid: ServerId,
    pub rk: String,
}

/// What the sub-line calls a source with no owner: the signed-in account's own server. A PERSON in
/// every case, which is the design's rule for every browsing surface — the machine name never
/// appears outside the Sources list.
fn own_account() -> &'static str { plx_os::i18n::msg::browse_account_own() }

/// Build the panel's rows from `list`, given the copy the page is standing on (`here_sid` +
/// `here_rk`). PURE — every ordering and marking decision in this module is here, and the host
/// suite drives it directly.
///
/// **Exactly one row is ticked**, by construction: the mark is placed on the FIRST copy matching
/// the page's own (server, ratingKey), so a producer that listed the same copy twice still yields
/// one "you are here" rather than two. A list with no such copy is ticked nowhere at all — which
/// is the honest answer while a landing is being reconciled, and never two.
///
/// The order is the design's: the copy that plays first, then higher quality, then yours before a
/// friend's, then by library name so the list cannot reshuffle between two frames that resolved
/// the same facts.
pub fn rows(list: &[AltCopy], here_sid: ServerId, here_rk: &str) -> Vec<AltRow> {
    // `same_item`, not a hand-rolled `&&`: it is the one place the pair rule lives, and its own doc
    // promises every stored-item lookup routes through it. These two sites were the last that did
    // not — and they are the ones that decide which row wears the tick and which press does nothing.
    let here = list
        .iter()
        .position(|c| plx_plex::plex::same_item((c.sid, &c.rk), (here_sid, here_rk)));
    let mut idx: Vec<usize> = (0..list.len()).collect();
    idx.sort_by(|&a, &b| {
        let (ca, cb) = (&list[a], &list[b]);
        // `false` sorts before `true`, so each key is written as "the one that loses"
        (
            here != Some(a),
            std::cmp::Reverse(scan_lines(ca)),
            ca.owner.is_some(),
            &ca.library,
        )
            .cmp(&(
                here != Some(b),
                std::cmp::Reverse(scan_lines(cb)),
                cb.owner.is_some(),
                &cb.library,
            ))
    });
    idx.into_iter()
        .map(|i| {
            let c = &list[i];
            let who = c.owner.as_deref().unwrap_or(own_account());
            AltRow {
                label: c.library.clone(),
                detail: who.to_string(),
                own_detail: c.owner.is_none(),
                // a copy the server sent no duration for leaves the read-out slot EMPTY rather
                // than claiming "0m" — the same rule the sub-line's dangling separator followed
                // while the runtime was part of it
                value: (c.dur_ms > 0).then(|| plx_ui::fmt::duration(c.dur_ms)),
                badge: plx_ui::fmt::resolution(&c.res, c.width, c.height),
                checked: here == Some(i),
                sid: c.sid,
                rk: c.rk.clone(),
            }
        })
        .collect()
}

/// One row's drawing: its four content places. The tick is WHICH copy you are on, the label WHICH
/// LIBRARY (a server's name), the sub-line WHOSE (another person's name, or this app's "This
/// account"), the read-out HOW LONG and the badge WHAT YOU GET.
fn row_for(r: &AltRow) -> Row {
    let mut row = Row::new(r.label.clone()).server_label().detail(r.detail.clone()).checked(r.checked);
    if !r.own_detail {
        row = row.server_detail();
    }
    if let Some(v) = &r.value {
        row = row.value(v.clone());
    }
    if let Some(b) = &r.badge {
        row = row.badge(Badge::Text(b.clone()));
    }
    row
}

/// What OK on the highlighted row does.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    None,
    /// Open `rk` on server `sid` — that copy's own page. Never a swap of the copy in place.
    Open { sid: ServerId, rk: String },
}

/// What one row's OK means, decided once when the row is declared, so the "the row you are on is
/// not a destination" rule is host-testable and no press ever indexes a list.
pub fn action_for(row: &AltRow, here_sid: ServerId, here_rk: &str) -> Action {
    if plx_plex::plex::same_item((row.sid, &row.rk), (here_sid, here_rk)) || row.rk.is_empty() {
        return Action::None;
    }
    Action::Open {
        sid: row.sid,
        rk: row.rk.clone(),
    }
}

/// A row's identity: the copy it names (server + ratingKey) and, for a producer that listed the
/// same copy twice, which occurrence — a duplicate is a second row, never a second identity
/// collision. Selection across a rebuild follows this, not a position.
#[derive(Clone, Debug, PartialEq)]
pub struct AltId {
    sid: ServerId,
    rk: String,
    nth: usize,
}

impl AltId {
    /// The identity of the row at `pos` of `rows`.
    fn of(rows: &[AltRow], pos: usize) -> Self {
        let r = &rows[pos];
        let nth = rows[..pos].iter().filter(|o| o.sid == r.sid && o.rk == r.rk).count();
        AltId { sid: r.sid, rk: r.rk.clone(), nth }
    }
}

/// The panel's one section, declared from the resolved rows alone (pure, so the text-fit suite
/// drives it): every row carries its own destination ([`action_for`]). The dynamic rows' focus key
/// is their position in the list (docs/settings-form.md, "Dynamic rows"): the list is one
/// unheaded section whose length and order the store decides.
pub fn form_for(rows: &[AltRow], here_sid: ServerId, here_rk: &str) -> Form<AltId, Action, Infallible> {
    let mut sec = FormSection::new("");
    for (pos, r) in rows.iter().enumerate() {
        let id = AltId::of(rows, pos);
        sec = sec.item_keyed(id, RowKey(pos as u32), RowKind::Choice, action_for(r, here_sid, here_rk), row_for(r));
    }
    Form::new().section(sec)
}

// ---- the surface -----------------------------------------------------------------------------

/// Where the selection lands when the table is rebuilt.
enum Sel {
    /// Opening: on the copy you are on — the row the tick is against — because that is where the
    /// eye already is, and one DOWN from it is the alternative.
    OnTheCopyYouAreOn,
    /// Rebuilding under an OPEN panel: leave the cursor where the user put it.
    Keep,
}

/// A page-less menu: its root is the only page there is, so no push or pop ever asks for another.
type AltMenu = Menu<Infallible, AltId, Action>;

pub struct AltSourcesScreen {
    entry: EntryId,
    arg: AltSourcesArg,
    /// The rows the table and its actions were built from — the rebuild stamp (module doc).
    rows: Vec<AltRow>,
    /// The panel: a sheet hung under or above the pill that opened it, one root page, declared from
    /// the resolved rows alone ([`form_for`]).
    menu: AltMenu,
}

impl AltSourcesScreen {
    pub fn new(entry: EntryId, arg: AltSourcesArg, meta: plx_data::metadata::MetadataView<'_>) -> Self {
        // rows carry a sub-line — the track menu's size class, not the account popover's
        let menu = Menu::new(Host::Sheet { anchor: arg.anchor }, Class::Picker, plx_ui::table_screen::BAND_BASE);
        let mut screen = Self { entry, arg, rows: Vec::new(), menu };
        screen.rebuild(Sel::OnTheCopyYouAreOn, meta);
        screen
    }

    /// The panel's table and cursor.
    pub fn form(&self) -> &plx_ui::form::FormTable<AltId, Action, Infallible> {
        self.menu.form()
    }

    /// The rows the store would produce right now, marked against **the server the open PAGE
    /// belongs to** and the item it is showing.
    ///
    /// The page's server, not `plex::current_server()`. Those were the same thing only while
    /// browsing a shared library also re-pointed `current`; once that stopped, opening a borrowed
    /// film left `current` on our own server, no copy matched the pair, and the panel drew **no
    /// tick at all** — owner-reported. The tick answers "which of these am I looking at", and only
    /// the page knows; since phase 10 it says so on the argument.
    fn live_rows(&self, meta: plx_data::metadata::MetadataView<'_>) -> Vec<AltRow> {
        rows(
            meta.alt_copies(self.arg.sid, &self.arg.rk),
            self.arg.sid,
            &self.arg.rk,
        )
    }

    /// Materialise the store into the table, when and only when the drawn content would differ.
    fn refresh(&mut self, meta: plx_data::metadata::MetadataView<'_>) -> bool {
        let next = self.live_rows(meta);
        if next == self.rows {
            return false;
        }
        self.rows = next;
        self.rebuild(Sel::Keep, meta);
        true
    }

    fn rebuild(&mut self, sel_mode: Sel, meta: plx_data::metadata::MetadataView<'_>) {
        if matches!(sel_mode, Sel::OnTheCopyYouAreOn) {
            self.rows = self.live_rows(meta);
        }
        let (rows, sid, rk) = (&self.rows, self.arg.sid, &self.arg.rk);
        match sel_mode {
            // leave the cursor where the user put it — on the same copy, found by identity
            Sel::Keep => {
                let mut build = |page: Option<Infallible>| page.is_none().then(|| Page::root(form_for(rows, sid, rk)));
                self.menu.step(Event::Refresh { keep: None }, &mut build);
            }
            // on the copy you are on (the tick), else the first row
            Sel::OnTheCopyYouAreOn => {
                let here = rows.iter().position(|r| r.checked).map(|pos| AltId::of(rows, pos));
                let mut build =
                    |page: Option<Infallible>| page.is_none().then(|| Page::root(form_for(rows, sid, rk)).initial(here.clone()));
                self.menu.step(Event::ReplaceRoot { slide: None }, &mut build);
            }
        }
    }

    pub fn frame(&self, measure: &dyn plx_machine::machine::Measure) -> Rect {
        self.menu.panel_rect(measure)
    }

    fn dismiss<H: AppLike>(&self, fx: &mut Effects<'_, H>) {
        fx.push(Fx::Nav(NavOp::Dismiss(self.entry)));
    }

    /// What the menu answered: a dismissal closes the panel, and a pick closes it and names the copy to open. The
    /// row you are ALREADY on reports nothing: there is nowhere to navigate to, and the tick has already answered
    /// the question the press was asking.
    ///
    /// The row comes from the event's own element (`Menu::on_screen`), not the table's cursor: the engine's OK arm
    /// reads it off the current focus key and a pointer click's `Activate` names the STOP that was clicked, which
    /// need not be the row the cursor was already resting on.
    fn run<H: AppLike>(&mut self, out: Out<Action>, fx: &mut Effects<'_, H>) {
        let action = match out {
            Out::Stay | Out::Moved | Out::Edge => return,
            Out::Picked(action) => action,
            Out::Dismiss => Action::None,
        };
        self.dismiss(fx);
        if let Action::Open { sid, rk } = action {
            // The PAGE navigates. This surface names the destination and nothing else — the
            // `LibraryMenu` shape, and what keeps "what a press means on the Detail page" in one
            // place instead of two.
            fx.push(Fx::Deliver(
                MachineId::Instance(self.arg.host),
                Delivery::Screen(ScreenEvent::App(AppMsg::AltSourceOpen(
                    ContentArg::Detail { sid, rk },
                ))),
            ));
        }
    }
}

impl<H: AppLike<Memory = PageMemory> + crate::registry::MetadataLike> Machine<H> for AltSourcesScreen {
    type Ev = ScreenEvent<H>;
    fn step(&mut self, ev: &Self::Ev, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) -> Handled {
        match ev {
            ScreenEvent::Mount | ScreenEvent::Enter(_) => {
                self.refresh(H::metadata(cx));
                Handled::Yes
            }
            ScreenEvent::StoreChanged(ord, _) => {
                if *ord == plx_data::stores::StoreId::Metadata.ord() && self.refresh(H::metadata(cx)) {
                    fx.invalidate(plx_machine::present::Provenance::Landing(fx.from()));
                }
                Handled::Yes
            }
            _ => {
                // The rest is the menu's (`Menu::on_screen`): the tick re-derives the drawn selection from the
                // engine's own tracked focus (the belt to `FocusMoved`'s suspenders, for a focus change this screen
                // was not told about directly: a remembered-cursor seat on `Enter`), OK and a click (the engine's OK
                // arm delivers `Activate` for a `Bare` element, a pointer click on a registered row stop reaches the
                // same arm), BACK (at its one page it closes) and LEFT (inert here, never a close). A click BESIDE
                // every registered row stop is a HIT-MAP MISS, and `Style::Compact`'s `on_miss`
                // (`OnMiss::Dismiss`) closes the panel.
                match self.menu.on_screen(ev, cx, self.entry, fx, &mut no_pages) {
                    Some(glued) => {
                        self.run(glued.out, fx);
                        glued.handled
                    }
                    None => Handled::No,
                }
            }
        }
    }
}

impl plx_ui::menu::HasMenu for AltSourcesScreen {
    type Page = Infallible;
    type Row = AltId;
    type Act = Action;
    fn menu(&self) -> &Menu<Infallible, AltId, Action> {
        &self.menu
    }
    fn menu_entry(&self) -> EntryId {
        self.entry
    }
}

/// **A single-column `Focusable` over the table's own cursor** (phase 12, D2) — the same shape
/// `AccountMenuScreen::groups` uses for its own `TableView`: one `GroupKind::Column` of
/// one `Bare` element per row (OK/click activates on the down edge, never arms a hold —
/// there is nothing here to hold), UP/DOWN stopping at the ends, and `place`/`neighbour` reading
/// straight off the menu's own row geometry rather than a second copy of it.
impl<H: AppLike<Memory = PageMemory>> Focusable<H> for AltSourcesScreen {
    fn groups(&self, cx: &Cx<'_, H>, out: &mut Vec<GroupSpec>) {
        self.menu.groups(GroupId(0), cx.measure, out);
    }
    fn group_of(&self, elem: &u32, _cx: &Cx<'_, H>) -> Option<GroupId> {
        self.menu.group_of(elem, GroupId(0))
    }
    fn neighbour(&self, key: FocusKey<u32>, dir: Dir, _cx: &Cx<'_, H>) -> Step<u32> {
        self.menu.neighbour(self.entry, key, dir)
    }
    fn place(&self, elem: &u32, cx: &Cx<'_, H>, _at: At) -> Option<Placed> {
        self.menu.place(elem, cx.measure)
    }
    fn reconcile(&self, want: FocusKey<u32>, _cx: &Cx<'_, H>) -> FocusKey<u32> {
        self.menu.reconcile(self.entry, want, GroupId(0))
    }
    fn seat(&self, _g: GroupId, _from: Placed, _cx: &Cx<'_, H>) -> FocusKey<u32> {
        self.menu.seat(self.entry)
    }

    /// **Row `n` is `row.<n>`**, the shared table convention
    /// ([`plx_ui::geom::table_a11y`]): `n` is the row's ordinal among the focusable rows (the same
    /// count `GroupSpec::len` states), so a separator or note never shifts an id. The label is the
    /// row's own text, for the state file only. The elements and the inverse are the shared ones.
    fn a11y(&self, elem: &u32, _cx: &Cx<'_, H>) -> Option<A11y<'_>> {
        let form = self.menu.form();
        let slot = form.index_of_key(RowKey(*elem))?;
        plx_ui::geom::table_a11y(&form.table, Some(form), slot)
    }
    fn a11y_elems(&self, g: GroupId, _cx: &Cx<'_, H>, out: &mut Vec<u32>) {
        if g == GroupId(0) {
            let form = self.menu.form();
            out.extend(
                plx_ui::geom::table_elems(&form.table, Some(form))
                    .filter_map(|i| form.key_at(i))
                    .map(|k| k.0),
            );
        }
    }
    fn a11y_find(&self, path: &A11yPath, _cx: &Cx<'_, H>) -> Option<u32> {
        let form = self.menu.form();
        let slot = plx_ui::geom::table_a11y_find(&form.table, Some(form), path)?;
        form.key_at(slot).map(|k| k.0)
    }
}

impl LogicalState for AltSourcesScreen {
    fn write(&self, c: &mut Canon) {
        self.arg.write(c);
        c.seq(self.rows.len());
        for r in &self.rows {
            c.u32(u32::from(r.sid.raw()))
                .str(&r.rk)
                .str(&r.label)
                .str(&r.detail)
                .option(r.value.as_deref(), |c, v| {
                    c.str(v);
                })
                .option(r.badge.as_deref(), |c, b| {
                    c.str(b);
                })
                .bool(r.checked);
        }
        let table = &self.menu.form().table;
        c.u32(table.sel as u32);
        table.write_motion(c);
    }
    fn probe(&self, out: &mut String) {
        out.push_str("alt");
    }
}

impl<H: AppLike<Memory = PageMemory> + crate::registry::MetadataLike> Screen<H> for AltSourcesScreen {
    fn name(&self) -> &'static str {
        "alt"
    }
    fn state(&self) -> &dyn LogicalState {
        self
    }
    fn crumb(&self, _cx: &Cx<'_, H>) -> Option<Cow<'_, str>> {
        None
    }
    // The menu's own (`plx_ui::menu_screen_methods!`). The modal dim is asked for rather than drawn, at the menu's
    // weight: the PANEL role (a picker in the middle of the frame; the page recedes, it is not blanked, and the
    // hero behind stays readable). Nothing is lifted back out of it: this picker hangs under the detail page's
    // Source chip, a control that says which copy is playing rather than the subject of the panel. The draw is
    // named for `/tmp/plxnative-cpuprof` beside the page's own phases.
    plx_ui::menu_screen_methods!(H, "dt.alt");
    fn render(&self) -> RenderStrategy {
        RenderStrategy::Page
    }
    fn focus_source(&self) -> FocusSource {
        FocusSource::Engine
    }
    fn hit_source(&self) -> HitSource {
        HitSource::Engine
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
#[path = "alt_sources_tests.rs"]
pub(crate) mod tests;
