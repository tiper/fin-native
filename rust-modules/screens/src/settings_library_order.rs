//! **Settings > Libraries > Library order**: the order of the favorite libraries on Home and in the
//! Library header, moved one library at a time.
//!
//! The page lists the favourites in the profile's order (the Browse publication's
//! `DirectoryView::library_order`) and a Reset row. OK on a library PICKS IT UP ([`Mode::Moving`]):
//! Up and Down carry it through the list (the table glides the rows, `FormTable`), OK puts it down
//! (one `BrowseCmd::SetLibraryOrder` naming the libraries an order can name, in the shown order, and only when that order changed), Back puts it back and writes
//! nothing. OK on Reset asks once more ([`Mode::ConfirmReset`]); a second OK resets
//! (`BrowseCmd::ResetLibraryOrder`), any other key keeps the order. Left and Right are swallowed
//! while a library is held (`TableScreen::uncommitted`), so a stray press cannot leave the page with
//! the order half-moved.
//!
//! **One lifecycle, one enum.** [`Mode`] is everything the page is doing: nothing, holding a library,
//! asking whether to reset, and the two sentences that follow a finished action. The sentence beside
//! the table is a pure function of the mode and the focused row ([`PageState::description`]); the
//! rows' move marks are a function of the mode ([`move_of`]); nothing else keeps a flag.
//!
//! **A library is its server's machine id and its section key** ([`LibraryRef`]): the identity the
//! saved order names it by, which survives a rebuild of the section table (whose indices
//! ([`LibraryOrderRow::section`]) do not). The mode, the focus, the sentence and the rows' keys all
//! follow a library by it. A library whose server's machine id is not learned yet has no identity an
//! order can record ([`LibId::Unnamed`]): it is listed, not pickable, and says why, and a drop's
//! command leaves it out. **The page lays every order out with the store's own rule**
//! ([`PageState::laid_out`]): the named order a move leaves is handed to `plx_data::browse`'s
//! `presented_sections_named`, the call the store makes over the same table in its server order (the
//! publication's [`Published::slots`]), so a row no order names lands right after its default
//! predecessor. A drop that SENDS an order shows what the store publishes for it (the store keeps
//! no entry of a library on a present server that the order does not name: the record names, among
//! the present servers' libraries, exactly the named order, or is deleted when that is the server
//! order), and the echo moves no row. A drop that sends nothing shows the rows that
//! were picked up, as Back does: the record stays as it was, and it may name libraries that are no
//! favourites, which would place the rows differently.
//!
//! **What the store says and what the page shows are two things, and the state between them has a
//! name** ([`Shown`]). The page reads the publication through ONE adapter ([`Published`]); after a
//! drop it is [`Shown::AwaitingEcho`] of the order it sent and keeps that order on screen, however
//! many publications of OTHER orders pass first (an earlier drop's echo landing while the next
//! library is held), until the store publishes the dropped order or [`ECHO_PATIENCE_MS`] runs out
//! (a command the store refused never echoes). Reset is declared `inert` while the profile has no
//! record (`OrderState::is_recorded`), so focus skips it; it stays offered while a failed write
//! leaves a record standing, and the failed write says so in words ([`PageState::description`]).
//!
//! **The table holds the invariants for keys and pointer alike.** While a library is held the other
//! rows wait (`RowMove::Waiting`), and a waiting row is no stop (`Row::selectable`), so the engine
//! cannot move focus off the held row; and a move of focus to anything but Reset cancels the
//! confirmation ([`Mode::ConfirmReset`]). Only what the table cannot say is swallowed by hand.
//!
//! The picture under the sentence is the header preview ([`Preview`]): the real Library header's
//! names row for the kind of the library in view, laid out through the same assembly
//! (`plx_ui::page_header::names_row`, by [`crate::registry::names_preview`]), so it folds names
//! where the real header does; its ⋯ and the note about it appear only when names are folded.

use super::*;
use plx_data::browse::{presented_sections_named, LibraryOrderRow, OrderSlot, OrderState, SecKind, WriteState};
use plx_data::stores::browse::{BrowseCmd, DirectoryView};
use plx_data::stores::{StoreCmd, StoreId};
use plx_machine::machine::{Edge, InputEvent, InputKind};
use plx_ui::follow_copy::{FollowCopy, PictureSlot};
use plx_ui::page_header::{NamesRow, PageHeader};
use plx_ui::table::RowMove;
use plx_ui::widgets::{CtlPop, TabStrip};

// ---------------------------------------------------------------------------------------------
// the adapter: every read of the Library order publication
// ---------------------------------------------------------------------------------------------

/// **What the page reads of the Browse publication, in one place.** The rows are the favourites in
/// the profile's order, `state` is what the profile's record is (Reset is offered while it is
/// recorded) and `write` where the last write stands (`Failed`: the order applies until restart).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Published {
    rows: Vec<LibraryOrderRow>,
    /// The table in the server's own order, which `rows` were laid out from: the input an order a
    /// move leaves is laid out from again.
    slots: Vec<OrderSlot>,
    state: OrderState,
    write: WriteState,
}

impl Published {
    fn read(directory: DirectoryView<'_>) -> Self {
        Self {
            rows: rows_of(directory).to_vec(), slots: directory.library_order_slots().to_vec(),
            state: directory.library_order_state(), write: directory.library_order_write(),
        }
    }
    /// Does `directory` publish exactly this (no clone, for the per-frame question)?
    fn is_current(&self, directory: DirectoryView<'_>) -> bool {
        self.rows == rows_of(directory)
            && self.slots == directory.library_order_slots()
            && self.state == directory.library_order_state()
            && self.write == directory.library_order_write()
    }
    fn reset_offered(&self) -> bool {
        self.state.is_recorded()
    }
    fn write_failed(&self) -> bool {
        self.write == WriteState::Failed
    }
}

/// The favourites in the profile's order, from the publication.
pub(super) fn rows_of<'a>(directory: DirectoryView<'a>) -> &'a [LibraryOrderRow] {
    directory.library_order()
}

/// **The Settings root's value for the Library order row**: "Server order" unless a saved order is in
/// effect, else the library it puts first ("Anime first").
pub(super) fn root_value(directory: DirectoryView<'_>) -> String {
    use plx_os::i18n::msg;
    match (directory.library_order_state().in_effect(), rows_of(directory).first()) {
        (true, Some(first)) => msg::library_order_value_first(&first.title),
        _ => msg::library_order_value_server().to_string(),
    }
}

// ---------------------------------------------------------------------------------------------
// the identity of a library
// ---------------------------------------------------------------------------------------------

/// **A library the saved order can name**: its server's machine id and its section key, the pair a
/// command carries. Unlike [`LibraryOrderRow::section`] (an index into one table epoch) it is the
/// same library after the table is rebuilt.
#[derive(Clone, Debug, PartialEq, Eq)]
struct LibraryRef {
    machine_id: String,
    key: i64,
}

impl LibraryRef {
    /// `None` while the registry has not learned the row's server's machine id: nothing an order can
    /// record names such a library.
    fn of(row: &LibraryOrderRow) -> Option<Self> {
        (!row.machine_id.is_empty()).then(|| Self { machine_id: row.machine_id.clone(), key: row.key })
    }
}

/// The libraries `rows` name, in order (the identity sequence a drop sends and the store echoes).
fn named_order(rows: &[LibraryOrderRow]) -> Vec<LibraryRef> {
    rows.iter().filter_map(LibraryRef::of).collect()
}

/// The command a drop sends: the shown order of every library an order can name, by `(machine id,
/// section key)`. `None` when the page shows none (nothing is sent: the store refuses an order that
/// names no library, Reset being the command that deletes one).
fn order_command(shown: &[LibraryOrderRow]) -> Option<BrowseCmd> {
    let libs: Vec<(String, i64)> = named_order(shown).into_iter().map(|lib| (lib.machine_id, lib.key)).collect();
    (!libs.is_empty()).then_some(BrowseCmd::SetLibraryOrder(libs))
}

// ---------------------------------------------------------------------------------------------
// the rows
// ---------------------------------------------------------------------------------------------

/// A row's identity: a library, a library no order can name yet (by the table index it opens, which
/// is all it has; it is never held, so no state outlives a rebuild on it), or Reset.
#[derive(Clone, PartialEq, Eq, Debug)]
enum LibId {
    Lib(LibraryRef),
    Unnamed(usize),
    Reset,
}

impl LibId {
    fn of(row: &LibraryOrderRow) -> Self {
        LibraryRef::of(row).map_or(LibId::Unnamed(row.section), LibId::Lib)
    }
}

/// The first focus key of a library row: clear of Reset's.
const LIB_KEY_BASE: u32 = 100;

impl FormId for LibId {
    /// Reset's key; a library's is assigned by [`Keys`] (`item_keyed`), so it moves with no row.
    fn key(&self) -> RowKey {
        match self {
            LibId::Reset => RowKey(1),
            LibId::Lib(_) | LibId::Unnamed(_) => RowKey(LIB_KEY_BASE),
        }
    }
    fn a11y(&self) -> Option<&'static str> {
        match self {
            LibId::Reset => Some("reset"),
            // one row per library: the position names each (`row.<i>`)
            LibId::Lib(_) | LibId::Unnamed(_) => None,
        }
    }
}

/// **A library's focus key, first seen first served**: the key a row answers the focus engine with
/// must stay the same row's through a reorder (the table glides rows, and the lift follows one, by
/// key), so it comes from the order libraries were first listed in, never from their position.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Keys(Vec<LibId>);

impl Keys {
    /// Give every row of `rows` its key (the first time it is listed).
    fn note(&mut self, rows: &[LibraryOrderRow]) {
        for row in rows {
            let id = LibId::of(row);
            if !self.0.contains(&id) {
                self.0.push(id);
            }
        }
    }
    fn key_of(&self, id: &LibId) -> RowKey {
        match id {
            LibId::Reset => id.key(),
            _ => RowKey(LIB_KEY_BASE + self.0.iter().position(|seen| seen == id).unwrap_or(0) as u32),
        }
    }
}

/// **What the page is doing**: the one lifecycle (no flag beside it).
#[derive(Clone, Debug, PartialEq, Eq)]
enum Mode {
    /// Nothing held: the rows read as pickable, the sentence is the focused row's.
    Idle,
    /// `lifted` is held; `before` is the shown order it was picked up from, which Back restores and
    /// a drop that changed no named order is compared with.
    Moving { lifted: LibraryRef, before: Vec<LibraryOrderRow> },
    /// OK was pressed on Reset: a second OK resets, any other key (or a move of focus off it) keeps the order.
    ConfirmReset,
    /// `.0` was just put down (and an order was sent): the page says where it is now, until the next key.
    Dropped(LibraryRef),
    /// The order was just reset: the page says so, until the next key.
    ResetDone,
}

/// A library row's part in the reorder, from the mode ([`RowMove`] is the table's one language for it).
///
/// A library moves among the libraries an order can name (`rank` of `named`), so every move changes
/// the order a drop sends; where a row no order names lies is the store's rule's business
/// ([`PageState::laid_out`]).
fn move_of(mode: &Mode, rank: Option<usize>, named: usize, row: &LibraryOrderRow) -> RowMove {
    let (can_up, can_down) = rank.map_or((false, false), |rank| (rank > 0, rank + 1 < named));
    match (mode, LibraryRef::of(row)) {
        (Mode::Moving { lifted, .. }, Some(lib)) if lib == *lifted => RowMove::Lifted { can_up, can_down },
        (Mode::Moving { .. }, _) => RowMove::Waiting,
        (_, Some(_)) => RowMove::Pickable { can_up, can_down },
        // no order can name it: it does not offer to move
        (_, None) => RowMove::Off,
    }
}

/// The page's rows: the shown favourites, then Reset (inert while there is no record to delete, and
/// while a library is held, when it is no action at all). A library no order can name yet is a
/// disabled row: focus lands on it so the sentence can say why, and OK does nothing.
fn order_form(shown: &[LibraryOrderRow], mode: &Mode, reset_offered: bool, keys: &Keys) -> Form<LibId, (), SettingsPage> {
    use plx_os::i18n::msg;
    let mut libraries = FormSection::new(msg::settings_libraries_title()).visible(!shown.is_empty());
    let named = shown.iter().filter(|row| LibraryRef::of(row).is_some()).count();
    let mut rank = 0;
    for row in shown {
        let at = LibraryRef::of(row).map(|_| { rank += 1; rank - 1 });
        let label = Row::new(&row.title).moving(move_of(mode, at, named, row));
        let label = if row.handle.is_empty() { label } else { label.value(&row.handle) };
        let id = LibId::of(row);
        let key = keys.key_of(&id);
        libraries = libraries.item_keyed(id.clone(), key, RowKind::Button, (), label).disabled(matches!(id, LibId::Unnamed(_)));
    }
    let reset = FormSection::new("")
        .item(LibId::Reset, RowKind::Button, (), Row::new(msg::library_order_reset()))
        .inert(!reset_offered || matches!(mode, Mode::Moving { .. }));
    Form::new().section(libraries).section(reset)
}

/// The plural kind word of a library's header ("Movies", "TV Shows").
fn kind_word(kind: SecKind) -> &'static str {
    match kind {
        SecKind::Movie => plx_os::i18n::msg::browse_kind_movies(),
        SecKind::Show => plx_os::i18n::msg::browse_kind_tv_shows(),
    }
}

// ---------------------------------------------------------------------------------------------
// the page state
// ---------------------------------------------------------------------------------------------

/// How long the page keeps a dropped order on screen waiting for the store to publish it, in
/// milliseconds of the tick clock. The store republishes within its next step, so this only ends the
/// wait for a command it refused (and so will never echo): the page then shows what the store says.
const ECHO_PATIENCE_MS: u32 = 3_000;

/// **Where the rows the page shows come from**: one named state, not an inference from a snapshot
/// that goes stale while a library is held.
#[derive(Clone, Debug, PartialEq)]
enum Shown {
    /// The store's: re-read whenever it publishes something other than what was last read.
    Published,
    /// A drop sent this order (the libraries an order can name, as the shown rows had them) and the
    /// store has not published it yet: the page keeps its own rows, whatever else the store
    /// publishes meanwhile (an earlier drop's echo, say), until the store says this order (of the
    /// libraries it names; one that gained a machine id meanwhile does not make it another order)
    /// or [`ECHO_PATIENCE_MS`] of the tick clock have passed since `since`, the first tick after the
    /// drop (`None` until one lands): an absolute reading of the clock, never an accumulated delta.
    AwaitingEcho { order: Vec<LibraryRef>, since: Option<u32> },
}

struct PageState {
    mode: Mode,
    /// The focused row's identity.
    sel: LibId,
    /// The order the page SHOWS (the held library's moving order while one is held).
    shown: Vec<LibraryOrderRow>,
    /// Where `shown` comes from ([`Shown`]).
    basis: Shown,
    /// The last publication the page took in: its rows only while [`Shown::Published`], its record
    /// state and write state always.
    seen: Published,
    /// Every library's focus key ([`Keys`]).
    keys: Keys,
}

impl PageState {
    fn position(&self, lib: &LibraryRef) -> Option<usize> {
        self.shown.iter().position(|row| LibraryRef::of(row).as_ref() == Some(lib))
    }

    fn row_of(&self, lib: &LibraryRef) -> Option<&LibraryOrderRow> {
        self.shown.iter().find(|row| LibraryRef::of(row).as_ref() == Some(lib))
    }

    /// **The shown rows for a named order**, laid out by the store's rule: the libraries of the
    /// table in the order `presented_sections_named` puts them were `named` recorded, over the table
    /// the shown rows were read with ([`Published::slots`]). A row no order can name lands where the
    /// store will place it, so the page shows what the store publishes after the drop. `None` when
    /// the rows and the table disagree (the publication moved under the page): the caller leaves the
    /// rows as they are.
    fn laid_out(&self, named: &[LibraryRef]) -> Option<Vec<LibraryOrderRow>> {
        let named: Vec<(String, i64)> = named.iter().map(|lib| (lib.machine_id.clone(), lib.key)).collect();
        let laid: Vec<LibraryOrderRow> = presented_sections_named(&self.seen.slots, &named).into_iter()
            .filter_map(|section| self.shown.iter().find(|row| row.section == section).cloned()).collect();
        (laid.len() == self.shown.len()).then_some(laid)
    }

    /// The library row the focus is on, `None` on Reset.
    fn focused(&self) -> Option<&LibraryOrderRow> {
        self.shown.iter().find(|row| LibId::of(row) == self.sel)
    }

    /// **The library the preview follows**: the one held, else the one just put down, else the one in
    /// focus; with Reset in focus, the last library.
    fn previewed(&self) -> Option<&LibraryOrderRow> {
        let held = match &self.mode {
            Mode::Moving { lifted, .. } => self.row_of(lifted),
            Mode::Dropped(lib) => self.row_of(lib),
            _ => self.focused(),
        };
        held.or_else(|| self.shown.last())
    }

    /// **The sentence beside the table**: a pure function of the mode, the focused row and (for the
    /// one thing it adds) whether the last write failed.
    fn description(&self) -> String {
        use plx_os::i18n::{msg, ordinal};
        let said = match &self.mode {
            Mode::ConfirmReset => return msg::library_order_state_confirm().to_string(),
            Mode::Moving { lifted, .. } => {
                return self.row_of(lifted).map_or_else(String::new, |row| msg::library_order_state_moving(&row.title));
            }
            Mode::ResetDone => msg::library_order_state_reset_done().to_string(),
            Mode::Dropped(lib) => match (self.row_of(lib), self.position(lib)) {
                (Some(row), Some(at)) => {
                    let place = self.shown[..at].iter().filter(|other| other.kind == row.kind).count();
                    // the generated function takes its arguments in name order
                    msg::library_order_state_dropped(&ordinal(place + 1), &ordinal(at + 1), kind_word(row.kind), &row.title)
                }
                _ => String::new(),
            },
            Mode::Idle => match self.focused() {
                Some(row) if LibraryRef::of(row).is_none() => msg::library_order_state_unnamed(&row.title),
                Some(row) => msg::library_order_state_idle(kind_word(row.kind), &row.title),
                None => msg::library_order_state_reset().to_string(),
            },
        };
        match (self.seen.write_failed(), said.is_empty()) {
            (true, false) => format!("{said} {}", msg::library_order_state_unsaved()),
            (true, true) => msg::library_order_state_unsaved().to_string(),
            (false, _) => said,
        }
    }
}

fn write_lib(w: &mut Canon, lib: &LibraryRef) {
    w.str(&lib.machine_id).u64(lib.key as u64);
}

fn write_id(w: &mut Canon, id: &LibId) {
    match id {
        LibId::Lib(lib) => { w.u8(0); write_lib(w, lib); }
        LibId::Unnamed(section) => { w.u8(1).u32(*section as u32); }
        LibId::Reset => { w.u8(2); }
    }
}

fn write_rows(w: &mut Canon, rows: &[LibraryOrderRow]) {
    w.seq(rows.len());
    for row in rows { write_id(w, &LibId::of(row)); }
}

impl LogicalState for PageState {
    fn write(&self, w: &mut Canon) {
        write_id(w, &self.sel);
        match &self.mode {
            Mode::Idle => { w.u8(0); }
            Mode::Moving { lifted, before } => {
                w.u8(1);
                write_lib(w, lifted);
                write_rows(w, before);
            }
            Mode::ConfirmReset => { w.u8(2); }
            Mode::Dropped(lib) => { w.u8(3); write_lib(w, lib); }
            Mode::ResetDone => { w.u8(4); }
        }
        write_rows(w, &self.shown);
        match &self.basis {
            Shown::Published => { w.u8(0); }
            Shown::AwaitingEcho { order, since } => {
                w.u8(1).seq(order.len());
                for lib in order { write_lib(w, lib); }
                w.bool(since.is_some()).u32(since.unwrap_or(0));
            }
        }
        w.bool(self.seen.reset_offered()).bool(self.seen.write_failed());
        w.seq(self.keys.0.len());
        for id in &self.keys.0 { write_id(w, id); }
    }
    fn probe(&self, out: &mut String) {
        let name = |lib: &LibraryRef| format!("{}/{}", lib.machine_id, lib.key);
        let order: Vec<String> = self.shown.iter().map(|row| LibraryRef::of(row).map_or_else(|| format!("?{}", row.section), |lib| name(&lib))).collect();
        out.push_str(&format!(
            "library_order sel={:?} mode={} order={order:?} basis={} recorded={} write_failed={}",
            self.sel,
            match &self.mode {
                Mode::Idle => "idle".to_string(),
                Mode::Moving { lifted, .. } => format!("moving({})", name(lifted)),
                Mode::ConfirmReset => "confirm-reset".to_string(),
                Mode::Dropped(lib) => format!("dropped({})", name(lib)),
                Mode::ResetDone => "reset-done".to_string(),
            },
            match &self.basis {
                Shown::Published => "published".to_string(),
                Shown::AwaitingEcho { order, since } => format!("awaiting-echo({} libraries, since={since:?})", order.len()),
            },
            self.seen.reset_offered(), self.seen.write_failed(),
        ));
    }
}

// ---------------------------------------------------------------------------------------------
// the preview
// ---------------------------------------------------------------------------------------------

/// **The Library header's names row for one kind, as a picture.** Built from the kind's favourites in
/// the order shown (title and owner as the real header words them) through the one assembly the real
/// header runs ([`crate::registry::names_preview`]), so it folds the names the real header folds; the
/// held [`TabStrip`] glides the plate between libraries as focus moves.
pub(super) struct Preview {
    /// What `row` was built from: the pills (identity, label) and which one is plated, and whether the
    /// header reserves a visit slot.
    built: Option<(Vec<(LibId, String)>, LibId, bool)>,
    kind: SecKind,
    row: Option<NamesRow<LibId>>,
    strip: TabStrip,
    pop: CtlPop<1>,
}

impl Preview {
    pub(super) fn new() -> Self {
        Self { built: None, kind: SecKind::Movie, row: None, strip: TabStrip::new(), pop: CtlPop::new() }
    }

    /// Show `of` (a library among `rows`, the shown order) in the header of its kind. A change of
    /// pills or plate rebuilds the row; the same ones keep it.
    pub(super) fn sync(
        &mut self, rows: &[LibraryOrderRow], of: Option<&LibraryOrderRow>, directory: DirectoryView<'_>,
        measure: &dyn plx_machine::machine::Measure,
    ) {
        let Some(of) = of else {
            self.built = None;
            self.row = None;
            return;
        };
        let pills: Vec<(LibId, String)> = rows.iter().filter(|row| row.kind == of.kind)
            .map(|row| (LibId::of(row), plx_ui::page_header::joined_name(&row.title, &row.handle))).collect();
        let current = LibId::of(of);
        let visit = crate::registry::visit_slot(directory, of.kind, pills.len());
        if self.built.as_ref().is_some_and(|(held, at, held_visit)| *held == pills && *at == current && *held_visit == visit) {
            return;
        }
        let ids: Vec<LibId> = pills.iter().map(|(id, _)| id.clone()).collect();
        let label_at = |id: &LibId, room: f32| {
            let row = rows.iter().find(|row| LibId::of(row) == *id);
            row.map_or_else(String::new, |row| plx_ui::page_header::fit_name(&row.title, &row.handle, room, measure))
        };
        self.row = Some(crate::registry::names_preview(&ids, &current, of.kind, directory, measure, label_at));
        self.built = Some((pills, current, visit));
        self.kind = of.kind;
    }

    /// One frame of the plate's motion.
    pub(super) fn step(&mut self, dt: f32) {
        if let Some(row) = &self.row {
            let selected = row.page.selected(&row.layout);
            PageHeader::step(&row.layout, selected, -1, &mut self.strip, &mut self.pop, dt);
        }
    }

    /// Did the row the page would draw fold any name behind the ⋯?
    #[cfg(test)]
    pub(super) fn overflowed(&self) -> bool {
        self.row.as_ref().is_some_and(NamesRow::overflowed)
    }

    /// The labels of the names the row draws, in order (the ⋯ has none).
    #[cfg(test)]
    pub(super) fn shown_names(&self) -> Vec<String> {
        self.row.as_ref().map_or_else(Vec::new, |row| {
            row.page.spec().left.iter().filter(|item| item.glyph.is_none()).map(|item| item.label.clone()).collect()
        })
    }

    /// Does the row draw a ⋯ item?
    #[cfg(test)]
    pub(super) fn draws_more(&self) -> bool {
        self.row.as_ref().is_some_and(|row| row.page.spec().left.iter().any(|item| item.glyph.is_some()))
    }

    #[cfg(test)]
    pub(super) fn plated(&self) -> Option<String> {
        self.row.as_ref().and_then(|row| row.page.spec().left.iter().find(|item| item.selected_plate).map(|item| item.label.clone()))
    }

    pub(super) fn slot(&self) -> Option<PreviewSlot<'_>> {
        self.row.as_ref().map(|row| PreviewSlot { row, strip: &self.strip, kind: self.kind })
    }
}

/// The preview drawn into the picture slot: the kind's caption, the names row, and (only when names
/// are folded behind the ⋯, the same answer that draws it) the note about the ⋯.
pub(super) struct PreviewSlot<'a> {
    row: &'a NamesRow<LibId>,
    strip: &'a TabStrip,
    kind: SecKind,
}

/// The names row's top under the slot's caption (the mockup: a 24 px caption, then `--space-md`).
const CAPTION_H: f32 = 24.0;

impl PictureSlot for PreviewSlot<'_> {
    fn draw(&self, p: Painter, slot: Rect, measure: &dyn plx_machine::machine::Measure) {
        use plx_os::i18n::msg;
        use plx_ui::label::HAlign;
        use plx_ui::text_view::TextView;
        let size = theme::size::CAPTION;
        let caption_cap_top = plx_gfx::text::css_cap_drop(size, CAPTION_H, measure.cap_h(size));
        plx_ui::table::draw_caps_label_aligned(
            p, Rect::new(slot.x, slot.y + caption_cap_top, slot.w, plx_ui::table::caps_band_h()),
            &msg::library_order_preview_caption(kind_word(self.kind)), slot.w,
            theme::TEXT_TERTIARY, plx_ui::table::BOARD_CAPS_TRACK_PX, HAlign::Left, measure,
        );
        // the row stands where the real header stands: its layout is absolute, so slide it under the caption
        let layout = &self.row.layout;
        let Some(first) = layout.items.first() else { return };
        let row_top = slot.y + CAPTION_H + theme::space::MD;
        let moved = p.translate(slot.x - first.plate.x, row_top - layout.top());
        self.row.page.draw_names(moved, layout, self.strip, 1.0);
        if self.row.overflowed() {
            let note = msg::library_order_preview_more();
            let top = row_top + plx_ui::page_header::HEIGHT + theme::space::MD;
            TextView::new(note, size, theme::TEXT_TERTIARY).with_measure(measure).h(HAlign::Left).leading(32.0).max_lines(2)
                .draw(p, Rect::new(slot.x, top, slot.w, 64.0));
        }
    }
}


// ---------------------------------------------------------------------------------------------
// the page
// ---------------------------------------------------------------------------------------------

pub(super) struct LibraryOrderPage {
    entry: EntryId,
    form: FormTable<LibId, (), SettingsPage>,
    state: PageState,
    /// The sentence beside the table, which fades to the mode's and the focused row's.
    follow: FollowCopy,
    preview: Preview,
}

impl LibraryOrderPage {
    pub(super) fn new(entry: EntryId, directory: DirectoryView<'_>) -> Self {
        let seen = Published::read(directory);
        let shown = seen.rows.clone();
        let mut keys = Keys::default();
        keys.note(&shown);
        let initial = shown.first().map_or(LibId::Reset, LibId::of);
        let mut page = Self {
            entry,
            form: FormTable::page(super::super::registry::BAND),
            state: PageState { mode: Mode::Idle, sel: initial.clone(), shown, basis: Shown::Published, seen, keys },
            follow: FollowCopy::new(""),
            preview: Preview::new(),
        };
        let form = order_form(&page.state.shown, &page.state.mode, page.state.seen.reset_offered(), &page.state.keys);
        page.form.open(form, Some(&initial));
        page.form.table.list_focused = true;
        page.follow = FollowCopy::new(&page.state.description());
        page
    }

    /// The focus key of the focused row.
    fn sel_key(&self) -> RowKey {
        self.state.keys.key_of(&self.state.sel)
    }

    /// Re-derive the rows from the mode and the shown order, keeping the cursor on its row by
    /// identity (the held library's while it is held), and the sentence with them.
    fn rebuild(&mut self) {
        let keep = match &self.state.mode {
            Mode::Moving { lifted, .. } => Some(LibId::Lib(lifted.clone())),
            _ => self.form.selected_id().cloned(),
        };
        self.state.keys.note(&self.state.shown);
        let form = order_form(&self.state.shown, &self.state.mode, self.state.seen.reset_offered(), &self.state.keys);
        // whether the list or the band holds focus is the engine's to say (a pointer over the band
        // moved it off the list): a rebuild keeps it as it was
        let list_focused = self.form.table.list_focused;
        self.form.set_sliding(form, keep.as_ref());
        self.form.table.list_focused = list_focused;
        if let Some(id) = self.form.selected_id() {
            self.state.sel = id.clone();
        }
        self.follow.follow(&self.state.description());
    }

    fn view(&self) -> TableScreen<'_> {
        use plx_os::i18n::msg;
        TableScreen::new(
            Header::new(RouteLayout::screen(), Some(msg::settings_title()), msg::library_order_title(), msg::library_order_row_detail())
                .with_follow(&self.follow),
            &self.form.table, GroupId(0), self.entry,
        ).keyed(&self.form).uncommitted(matches!(self.state.mode, Mode::Moving { .. }))
    }

    /// Take the store's word in: the rows follow the publication (never while a library is held, and
    /// not while a dropped order waits for its echo, [`Shown::AwaitingEcho`]), the record and write
    /// state always.
    fn adopt(&mut self, directory: DirectoryView<'_>, now_ms: u32) -> bool {
        if matches!(self.state.mode, Mode::Moving { .. }) {
            return false;
        }
        let mut gave_up = false;
        let take_rows = match &mut self.state.basis {
            Shown::Published => {
                if self.state.seen.is_current(directory) {
                    return false;
                }
                true
            }
            Shown::AwaitingEcho { order, since } => {
                let since = *since.get_or_insert(now_ms);
                // the order sent, among the libraries it named: a library learned meanwhile (a machine
                // id arriving) joins the publication's named set without making the echo another order
                let echoed = named_order(rows_of(directory)).into_iter().filter(|lib| order.contains(lib)).eq(order.iter().cloned());
                gave_up = !echoed && now_ms.wrapping_sub(since) >= ECHO_PATIENCE_MS;
                if echoed || gave_up {
                    true
                } else {
                    // the rows hold; what the record is and whether it saved still follow the store
                    let (state, write) = (directory.library_order_state(), directory.library_order_write());
                    if (self.state.seen.state, self.state.seen.write) == (state, write) {
                        return false;
                    }
                    self.state.seen.state = state;
                    self.state.seen.write = write;
                    false
                }
            }
        };
        if take_rows {
            self.state.seen = Published::read(directory);
            self.state.shown = self.state.seen.rows.clone();
            self.state.basis = Shown::Published;
        }
        // Reset can turn inert under a confirmation; a question about an action that is gone is not asked
        if matches!(self.state.mode, Mode::ConfirmReset) && !self.state.seen.reset_offered() {
            self.state.mode = Mode::Idle;
        }
        // "put down here" is true of an order the store took in; a refused one (given up on) or a
        // library that left the publication has no place to name
        let placed = match &self.state.mode {
            Mode::Dropped(lib) => self.state.row_of(lib).is_some(),
            _ => true,
        };
        if gave_up || !placed {
            if matches!(self.state.mode, Mode::Dropped(_)) {
                self.state.mode = Mode::Idle;
            }
        }
        self.rebuild();
        true
    }

    fn set_mode(&mut self, mode: Mode) {
        self.state.mode = mode;
        self.rebuild();
    }

    /// OK (or a click) on a row, by what the page is doing.
    fn activate(&mut self, key: u32, fx: &mut Effects<'_, InnerHost>) {
        // an inert Reset (nothing recorded, or a library held) and a library no order can name answer nothing
        if form_activate(&self.form, key, fx).is_none() {
            return;
        }
        let Some(id) = self.form.index_of_key(RowKey(key)).and_then(|i| self.form.id_at(i)).cloned() else { return };
        match (&self.state.mode, id) {
            (Mode::Moving { .. }, _) => self.drop_held(fx),
            (Mode::ConfirmReset, LibId::Reset) => self.reset(fx),
            (Mode::ConfirmReset, _) => self.set_mode(Mode::Idle),
            (_, LibId::Lib(lifted)) => {
                let before = self.state.shown.clone();
                self.set_mode(Mode::Moving { lifted, before });
            }
            (_, LibId::Unnamed(_)) => {}
            (_, LibId::Reset) => self.set_mode(Mode::ConfirmReset),
        }
    }

    /// Put the held library down: one write of the named order, and only when it changed. A
    /// drop that sends nothing says nothing of a place and puts back the rows picked up (the page is
    /// idle again, as after Back).
    fn drop_held(&mut self, fx: &mut Effects<'_, InnerHost>) {
        let Mode::Moving { lifted, before } = std::mem::replace(&mut self.state.mode, Mode::Idle) else { return };
        match order_command(&self.state.shown).filter(|_| named_order(&before) != named_order(&self.state.shown)) {
            Some(command) => {
                fx.push(Fx::App(AppFx::Store(StoreId::Browse, StoreCmd::Browse(command))));
                self.state.basis = Shown::AwaitingEcho { order: named_order(&self.state.shown), since: None };
                self.set_mode(Mode::Dropped(lifted));
            }
            // nothing was written: the store publishes nothing new, so the page shows what it picked up
            // (a round trip lays the named order out as if exactly it were recorded, which a record
            // that also names libraries that are no favourites need not have published)
            None => {
                self.state.shown = before;
                self.set_mode(Mode::Idle);
            }
        }
    }

    /// Put the held library back where it was picked up from and write nothing.
    fn restore_held(&mut self) {
        let Mode::Moving { before, .. } = std::mem::replace(&mut self.state.mode, Mode::Idle) else { return };
        self.state.shown = before;
        self.rebuild();
    }

    fn reset(&mut self, fx: &mut Effects<'_, InnerHost>) {
        fx.push(Fx::App(AppFx::Store(StoreId::Browse, StoreCmd::Browse(BrowseCmd::ResetLibraryOrder))));
        // whatever order was waiting for its echo is superseded: the store's next word is the reset's
        self.state.basis = Shown::Published;
        self.set_mode(Mode::ResetDone);
    }

    /// Move the held library one place among the libraries an order can name (`-1` up, `1` down); at
    /// either end nothing moves. Only the NAMED order is permuted: the rows are then laid out by the
    /// store's rule ([`PageState::laid_out`]), unnamed ones included.
    fn carry(&mut self, delta: isize) {
        let Mode::Moving { lifted, .. } = &self.state.mode else { return };
        let lifted = lifted.clone();
        let mut named = named_order(&self.state.shown);
        let Some(rank) = named.iter().position(|lib| *lib == lifted) else { return };
        let Some(to) = rank.checked_add_signed(delta).filter(|to| *to < named.len()) else { return };
        named.swap(rank, to);
        let Some(laid) = self.state.laid_out(&named) else { return };
        self.state.shown = laid;
        self.rebuild();
    }

    /// **The page's own answer to a key**, before the focus engine sees it. `Handled::Yes` keeps the
    /// engine (and the surface's Back) from acting on it.
    fn on_key(&mut self, key: Key, edge: Edge) -> Handled {
        match &self.state.mode {
            Mode::Moving { .. } => match key {
                Key::Up => { self.carry(-1); Handled::Yes }
                Key::Down => { self.carry(1); Handled::Yes }
                Key::Left | Key::Right => Handled::Yes,
                Key::Back if edge == Edge::Down => { self.restore_held(); Handled::Yes }
                Key::Back => Handled::Yes,
                _ => Handled::No,
            },
            // OK goes on to the press that activates Reset (the second OK); anything else keeps the order
            Mode::ConfirmReset if key == Key::Ok => Handled::No,
            Mode::ConfirmReset => {
                self.set_mode(Mode::Idle);
                if matches!(key, Key::Up | Key::Down | Key::Left | Key::Right | Key::Back) { Handled::Yes } else { Handled::No }
            }
            // the sentence about what just happened lasts until the next key
            Mode::Dropped(_) | Mode::ResetDone => {
                self.set_mode(Mode::Idle);
                Handled::No
            }
            Mode::Idle => Handled::No,
        }
    }

    /// **Focus moved** (a key through the engine, or a pointer over a row). The table keeps the other
    /// rows from taking it while a library is held; what it cannot say is handled here: focus
    /// leaving the held library (a pointer over the band) puts the library back, as Back does, and
    /// focus leaving Reset withdraws the question.
    fn focus_moved(&mut self, elem: u32) {
        form_focus(&mut self.form, elem);
        if let Some(id) = self.form.selected_id() {
            self.state.sel = id.clone();
        }
        let away = match &self.state.mode {
            // a row of the table that waits is no place focus lands (the form ignored it); only focus
            // OFF the table's rows (the band) leaves the held library
            Mode::Moving { .. } => self.form.index_of_key(RowKey(elem)).is_none(),
            Mode::ConfirmReset => RowKey(elem) != LibId::Reset.key(),
            _ => false,
        };
        if away && matches!(self.state.mode, Mode::Moving { .. }) {
            self.restore_held();
        } else if away {
            self.set_mode(Mode::Idle);
        } else {
            self.follow.follow(&self.state.description());
        }
    }
}

impl Machine<InnerHost> for LibraryOrderPage {
    type Ev = ScreenEvent<InnerHost>;
    fn step(&mut self, ev: &Self::Ev, cx: &Cx<'_, InnerHost>, fx: &mut Effects<'_, InnerHost>) -> Handled {
        match ev {
            ScreenEvent::Mount => {
                self.preview.sync(&self.state.shown, self.state.previewed(), cx.views.directory, cx.measure);
                fx.push(Fx::Deliver(
                    MachineId::Instance(InstanceId(0)),
                    Delivery::Screen(ScreenEvent::Enter(Enter::Fresh {
                        focus: FocusTarget::Elem(FocusKey { entry: self.entry, elem: self.sel_key().0 }),
                    })),
                ));
                Handled::Yes
            }
            ScreenEvent::Tick(t) => {
                if self.adopt(cx.views.directory, t.ms) {
                    fx.invalidate(Provenance::Input);
                }
                self.preview.sync(&self.state.shown, self.state.previewed(), cx.views.directory, cx.measure);
                self.form.table.update(t.dt(), RouteLayout::screen().sectioned_table().h);
                self.follow.update(t.dt());
                self.preview.step(t.dt());
                Handled::Yes
            }
            ScreenEvent::FocusMoved { to, .. } => {
                self.focus_moved(to.elem);
                Handled::Yes
            }
            ScreenEvent::Activate(key) => {
                self.activate(*key, fx);
                fx.invalidate(Provenance::Input);
                Handled::Yes
            }
            ScreenEvent::Input(InputEvent { kind: InputKind::Key { key, edge, .. }, .. }) if *edge != Edge::Up => {
                let handled = self.on_key(*key, *edge);
                fx.invalidate(Provenance::Input);
                handled
            }
            _ => Handled::No,
        }
    }
}

plx_ui::focusable_via_view!(LibraryOrderPage, InnerHost, view);

impl Screen<InnerHost> for LibraryOrderPage {
    fn name(&self) -> &'static str { crate::registry::word::LIBRARY_ORDER }
    fn state(&self) -> &dyn LogicalState { &self.state }
    fn crumb(&self, _cx: &Cx<'_, InnerHost>) -> Option<Cow<'_, str>> { Some(Cow::Borrowed(plx_os::i18n::msg::settings_title())) }
    fn prepare(&mut self, _b: &mut Budget, _cx: &Cx<'_, InnerHost>) {}
    fn draw(&mut self, f: &mut DrawFrame<'_, '_, InnerHost>) {
        let picture = self.preview.slot();
        let mut v = self.view();
        v.header.picture = picture.as_ref().map(|slot| slot as &dyn PictureSlot);
        plx_ui::screen::Part::<InnerHost>::draw(&mut v, f, Rect::FULL);
    }
    fn render(&self) -> RenderStrategy { RenderStrategy::Page }
    fn focus_source(&self) -> FocusSource { FocusSource::Engine }
    fn hit_source(&self) -> HitSource { HitSource::Engine }
}

/// The `(focus key, destination)` of every `Nav` row the page lists (the structural navigation
/// test's expectation, built by the page's own form builder): none.
#[cfg(test)]
pub(super) fn nav_items_for_test() -> Vec<(u32, SettingsPage)> {
    let rows = [("Movies", SecKind::Movie), ("TV Shows", SecKind::Show)].into_iter().enumerate()
        .map(|(section, (title, kind))| LibraryOrderRow { section, machine_id: "m".into(), key: section as i64, title: title.into(), kind, handle: String::new() })
        .collect::<Vec<_>>();
    let mut keys = Keys::default();
    keys.note(&rows);
    let mut form: FormTable<LibId, (), SettingsPage> = FormTable::page(super::super::registry::BAND);
    form.set(order_form(&rows, &Mode::Idle, true, &keys), None);
    (0..form.table.n_rows() as usize)
        .filter_map(|i| match form.binding_at(i)?.kind.clone() {
            RowKind::Nav(dest) => Some((form.key_at(i)?.0, dest)),
            _ => None,
        })
        .collect()
}

impl LibraryOrderPage {
    /// The focus key of the library row that opens table index `section` (the tests address rows by it).
    #[cfg(test)]
    fn elem_of(&self, section: usize) -> u32 {
        let row = self.state.shown.iter().find(|row| row.section == section).expect("a shown library");
        self.state.keys.key_of(&LibId::of(row)).0
    }
}

/// The focus key of the Reset row.
#[cfg(test)]
pub(super) fn reset_elem_for_test() -> u32 {
    LibId::Reset.key().0
}

#[cfg(test)]
#[path = "settings_library_order_tests.rs"]
mod tests;
