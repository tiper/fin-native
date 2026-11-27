//! Live TV's three pages, one screen: **What's on** (the landing page, the handoff's `SPEC-1` / `SPEC-2`), **the
//! Guide** (`SPEC-3`) and **Browse** (`SPEC-6`), chosen by the scope row they wear ([`Scope`], the Library's header row:
//! What's on | Guide | Browse, the chosen one on the selected plate). What's on is the profile's Recent
//! channels on top, the programmes airing now below, and the channels with neither under them: three adjacent
//! shared card shelves over the store's one lineup. A Recent tile, a programme poster and a Channels tile all ask
//! the content owner to play their channel. A channel is drawn as one thing wherever it stands, the handoff's
//! 250 x 140 channel plate (Recent and Channels); only a programme is a poster (On now).
//!
//! **On now has one fixed order**: the most recent start first, a tie in lineup order. **Recent never
//! reorders** (the store keeps it, newest first) and is hidden while it is empty. **Every channel stays
//! reachable**: a third shelf, the page's earlier "Channels" shelf (the same channel plates as Recent, which
//! may run past five and scroll like On now), holds the lineup channels with no programme on that are not in
//! Recent, under the other rows, and is hidden while that set is empty. It is the What's on path for a channel
//! without guide data (the Pluto channels of a server that has none; on the Guide such a channel is one cell
//! across the window, and on Browse it is a card of the programmes it has); with nothing recent and nothing on it is the page's only row. A lineup with no channel at all is the store's empty read-out.
//! Which rows the page shows is decided once, by [`Mode`] in `Page::sync`, and matched everywhere after.
//! The page reads the frame's wall time ([`LiveTvLike::wall`]) for its captions and progress bars, and asks
//! for a frame when the words of any airing change.
//!
//! **The Guide** is one board ([`plx_ui::time_grid::TimeGrid`]) under the scope row: six channels a page against a
//! 90-minute window that starts at a half hour, each channel's programmes laid along the time ruler with the
//! now line down through them. It is one focusable `Custom` section of the page's stack: the board owns the cells,
//! their travel (LEFT and RIGHT along a row, UP and DOWN to the cell that shares the most time) and the page of
//! six, and this screen owns what they mean. **OK on what is airing tunes the channel; OK on a later programme
//! does nothing**; a channel the guide has nothing for is one cell across the window that tunes it. **CH+ and CH-
//! turn the page by six** (CH+ the next page: it is the next channel, as the live player's is); a digit opens
//! number entry as on What's on; BACK from the player returns to the left channel's row (`Enter::Restored`).
//!
//! **Browse** is the scope's third page: a grid of programmes (one card per programme, the airing on now and each later
//! one the store has read) under the row, narrowed by the one filter the row's last stop opens (On now, Just started, Ending
//! soon, Later today). The rules and the decisions behind them are `browse.rs`'s; the filter's menu is `filter_menu.rs`'s.
//! Browse and its filter are one value ([`Scope::Browse`]), a card's element is its programme's ([`Idents`]) and a filter
//! that shows nothing is its own mode ([`Mode::BrowseNone`]), so no state can say two things at once.
//!
//! **The window scrolls by half hours** ([`Window`]): RIGHT past a row's last cell moves the whole window half an hour
//! later, as far as the guide has data (some programme runs past the window's end: [`Board::has_more`]); LEFT past a
//! row's first cell moves it back, never earlier than the live window (the half hour now is in). The focus lands on
//! the cell of its row that shares the most time with the one it left ([`TimeGrid::land`]), so a programme the window
//! cut stays under the focus while it is revealed. The board is read to the window's end and a step beyond it
//! ([`LiveTvCmd::Reach`], sent when the window moves and released when the page leaves), so the next shift has its
//! programmes. The window follows the clock until it is moved: a window held ahead stays where the reader put it as
//! the half hour rolls over, and goes back to live when the live window catches up to it. **BACK first returns a
//! scrolled window to the live one**, and so does choosing a scope or coming back from the player; the now line, its
//! shade and its chip are drawn only while now is in the window.
//! The window and the page are state of the screen (the [`Mode::Guide`] board, rebuilt by `Page::sync` from the store's
//! revision, the scope and the window's start), never read from a clock at draw. The board is laid against the clock's
//! instant ([`LiveTvLike::instant`]); a zone the platform cannot read costs the board its clock words (ruler labels,
//! programme times, the chip), not its programmes.
use std::collections::HashMap;
use plx_data::livetv::{Channel, LiveTvStatus, LiveTvView};
use plx_plex::plex::livetv::{window_start, WINDOW_SECONDS, WINDOW_STEP_SECONDS};
use plx_data::stores::{StoreCmd, StoreId};
use plx_data::stores::livetv::LiveTvCmd;
use plx_machine::machine::{Canon, Cx, Delivery, Edge, Effects, Enter, EntryId, FocusKey, FocusTarget,
    Fx, GroupId, Handled, InputEvent, InputKind, Key, LogicalState, Machine, Tick};
use plx_machine::present::Provenance;
use plx_ui::cards::{heading_ink_drop, CardEvent, CardSource, GridSpec, Kind, RowStyle, SectionSpec, Stack, StackEvent, StackPage, TileLabel};
use plx_ui::consts::*;
use plx_ui::page_header::{HeaderItem, HeaderRow, PageHeader, PageHeaderSpec, SharedRow};
use plx_ui::page_regions::{Arrival, BackStep, PageRegions};
use plx_ui::screen::{AxisMask, DrawFrame, EdgeRule, ElemKind, FocusSource, Focusable, GroupKind,
    GroupSpec, HitSource, Link, RenderStrategy, Screen, ScreenEvent, Seat};
use plx_ui::theme;
use plx_ui::a11y::A11y;
use plx_ui::time_grid::{self, Axis, Cell, Span, TimeGrid, Words};
use plx_ui::widgets::channel_tile;
use plx_ui::fmt::{self, Airing, AiringPhase, Wall};
use plx_ui::tile::Resume;
use plx_ui::widgets::{AmbientWash, Art, PageGround, StatusKind, StatusOverlay};
use plx_ui::{Env, Rect};
use crate::registry::{captions, tile_facts, AppFx, AppMsg, ContentPanel, ContentReq, HomeReq, HomeTab, LiveTvLike, PlayIntent};

pub(crate) mod browse;
mod idents;
use idents::Idents;
pub mod filter_menu;
use browse::{Filter, Pick};

#[cfg(test)]
pub(crate) mod cards_harness;

const FIRST_CARD_ELEM: u32 = 0x1000;
const RETRY_ELEM: u32 = 1;
const RECENT_GROUP: GroupId = GroupId(0);
const NOW_GROUP: GroupId = GroupId(1);
const HEAD_GROUP: GroupId = GroupId(2);
const STATUS_GROUP: GroupId = GroupId(3);
const CHANNELS_GROUP: GroupId = GroupId(4);
const GUIDE_GROUP: GroupId = GroupId(5);
const BROWSE_GROUP: GroupId = GroupId(6);
/// The scope row's three stops: What's on, the Guide, Browse; and Browse's filter control, the row's last stop, which
/// is on the row only while Browse shows.
const WHATS_ON_ELEM: u32 = 2;
const GUIDE_ELEM: u32 = 3;
const BROWSE_ELEM: u32 = 4;
const FILTER_ELEM: u32 = 5;
/// The Guide board's first element: above every card (`FIRST_CARD_ELEM` + two a channel), below the bar's.
const GUIDE_FIRST_ELEM: u32 = 0x0010_0000;
/// Browse's first card element: above the Guide's cells, below the accessibility namespaces' floor
/// ([`plx_ui::geom::NAMESPACE_FLOOR`]). A card's element is allocated from [`Idents`] and never reused within a page.
const BROWSE_FIRST_ELEM: u32 = 0x0100_0000;
/// The Guide section's height: from the ruler to the bottom safe edge, so the document is one screen and
/// never scrolls (the board's rows end at 932).
const GUIDE_H: f32 = SCR_H - MARGIN_Y - time_grid::RULER_Y;
/// A shelf's heading to its tiles as the handoff has it, from the top of the heading's line box (a Home
/// shelf keeps `TITLE_DY + CARD_DY`, 60). The page's shelves draw their heading at the cap-top, which is
/// [`heading_ink_drop`] below that line box's top, so each gives its heading that much less.
const BOARD_HEADING: f32 = 56.0;
/// The air from the bottom of a shelf's tiles to the heading of the shelf under it at rest (the handoff's
/// 48, to the line box's top).
const BOARD_GAP_AT_REST: f32 = 48.0;
/// The channel plate of Recent's tiles and of the Channels shelf's: a logo plate, 250 x 140 on the poster
/// pitch, so a poster stands under each Recent tile.
const PLATE_STYLE: RowStyle = RowStyle::CHANNEL;
/// On now's posters: the Home poster tile, with art that is not poster-shaped contained on its own blurred
/// copy rather than cover-cropped (`RowStyle::PROGRAMME`).
const NOW_STYLE: RowStyle = RowStyle::PROGRAMME;
/// The words a filter wears on the control and in its menu.
fn filter_label(filter: Filter) -> &'static str {
    use plx_os::i18n::msg;
    match filter {
        Filter::OnNow => msg::browse_livetv_on_now(),
        Filter::JustStarted => msg::browse_livetv_filter_just_started(),
        Filter::EndingSoon => msg::browse_livetv_filter_ending_soon(),
        Filter::LaterToday => msg::browse_livetv_filter_later_today(),
    }
}

/// **Browse's grid**: six columns of the programme poster (250 x 375 with the programme's art fit) on the Library's 280 pitch, from
/// the grid's top (244, 40 under the row). Its rows REST as the Library's and the Watchlist's do: the focused row stands where the
/// first does and the next peeks under it (`GridSpec::resting`).
fn browse_spec() -> GridSpec {
    GridSpec::resting(plx_ui::page_header::GRID_TOP).columns(plx_ui::cards::GRID_COLS, BROWSE_STYLE, MARGIN_X)
}
/// The grid's card: On now's poster (`RowStyle::PROGRAMME`) at the Library's 280 pitch.
const BROWSE_STYLE: RowStyle = RowStyle { gap: BROWSE_GAP, ..RowStyle::PROGRAMME };
/// The air between two posters: 280 less the poster.
const BROWSE_GAP: f32 = 30.0;
const STATUS_FRAME: Rect = Rect { x: MARGIN_X, y: plx_ui::page_header::GRID_TOP, w: SCR_W - 2.0 * MARGIN_X, h: SCR_H - plx_ui::page_header::GRID_TOP - MARGIN_Y };

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sec { Head, Guide, Browse, Recent, Now, Channels, Status }

impl Sec {
    /// The focus group the section's cards (or its retry read-out) stand in.
    const fn group(self) -> GroupId {
        match self {
            Sec::Head => HEAD_GROUP,
            Sec::Guide => GUIDE_GROUP,
            Sec::Browse => BROWSE_GROUP,
            Sec::Recent => RECENT_GROUP,
            Sec::Now => NOW_GROUP,
            Sec::Channels => CHANNELS_GROUP,
            Sec::Status => STATUS_GROUP,
        }
    }

    /// The tile style of a shelf section; `None` for the sections that are not shelves.
    const fn style(self) -> Option<&'static RowStyle> {
        match self {
            Sec::Recent | Sec::Channels => Some(&PLATE_STYLE),
            Sec::Now => Some(&NOW_STYLE),
            Sec::Head | Sec::Guide | Sec::Browse | Sec::Status => None,
        }
    }
}

/// **Which of Live TV's three pages shows**: the scope row's selection, the one explicit state a press on the row
/// changes (`LiveTvScreen::commit`). It is the screen's, not the store's: leaving the page and coming back
/// keeps it, a fresh page opens on What's on. **Browse carries its filter**, so the filter exists exactly while Browse
/// does and no other scope can hold a filter that disagrees with the one on show; the filter control's menu replaces
/// it through [`LiveTvScreen::pick_filter`], the one place it changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope { WhatsOn, Guide, Browse(Filter) }

impl Scope {
    /// The row's stops, left to right. Browse's is the filter it opens on (the default); a press on it while Browse is
    /// already showing keeps the filter in force ([`same_page`](Self::same_page)).
    const ALL: [Scope; 3] = [Scope::WhatsOn, Scope::Guide, Scope::Browse(Filter::OnNow)];

    /// The stop's element.
    const fn elem(self) -> u32 {
        match self { Scope::WhatsOn => WHATS_ON_ELEM, Scope::Guide => GUIDE_ELEM, Scope::Browse(_) => BROWSE_ELEM }
    }

    fn of_elem(elem: u32) -> Option<Scope> {
        Self::ALL.into_iter().find(|scope| scope.elem() == elem)
    }

    /// Is `other` the same page, whatever filter either holds?
    fn same_page(self, other: Scope) -> bool {
        std::mem::discriminant(&self) == std::mem::discriminant(&other)
    }

    /// The stop's accessibility name (`livetv.header.whats_on`).
    const fn name(self) -> &'static str {
        match self { Scope::WhatsOn => "whats_on", Scope::Guide => "guide", Scope::Browse(_) => "browse" }
    }

    /// The canon's discriminant of the page, and of its filter (`0` for a scope with none).
    const fn code(self) -> (u32, u32) {
        match self { Scope::WhatsOn => (0, 0), Scope::Guide => (1, 0), Scope::Browse(filter) => (2, filter.code()) }
    }

    /// Browse's filter, while Browse shows.
    const fn filter(self) -> Option<Filter> {
        match self { Scope::Browse(filter) => Some(filter), Scope::WhatsOn | Scope::Guide => None }
    }
}

/// **Which rows the page shows**, decided once per store revision ([`Page::sync`], the one place the store's
/// status is read) and matched everywhere after: the sections, the focus regions, the air under each shelf,
/// the cards, the headings and the status read-out. The page keeps no status beside it. A row has its own [`Sec`], and so its own shelf body and row style, in every
/// mode; the modes only say which of the rows are present. The rows come top to bottom: Recent, On now,
/// Channels.
#[derive(Debug, PartialEq)]
enum Mode {
    /// The store is loading: no row, the working read-out.
    Loading,
    /// The lineup is empty: no row, the store's empty read-out.
    Empty,
    /// The store failed: the retry read-out.
    Failed,
    /// Recent tiles over the programmes on now.
    RecentNow,
    /// Recent tiles alone: no programme on, and every other channel is in Recent.
    RecentOnly,
    /// The programmes on now alone.
    NowOnly,
    /// Recent tiles over the lineup channels that have no programme on.
    RecentChannels,
    /// Recent tiles, the programmes on now, then the lineup channels that have none.
    RecentNowChannels,
    /// The programmes on now, then the lineup channels that have none.
    NowChannels,
    /// Nothing recent and nothing on: the lineup as its Channels shelf, the page's only row.
    ChannelsOnly,
    /// The Guide's board under the scope row ([`Scope::Guide`] with a lineup and a clock to lay it against), which
    /// holds its window ([`Window`]) and the page of rows showing.
    Guide(Board),
    /// Browse's grid under the scope row and the filter ([`Scope::Browse`] with a lineup and a clock): the programmes the
    /// filter shows, at least one ([`Mode::browse`] decides this variant from the set, so it never holds an empty grid).
    Browse(Browse),
    /// Browse's filter shows nothing: the empty read-out under the row, whose filter control is the one stop that can be
    /// reached. The same [`Browse`] as the grid's, with no card.
    BrowseNone(Browse),
}

impl Mode {
    /// The mode of a store that is `Ready` with a lineup, from which rows have channels.
    fn of(recent: bool, now: bool, channels: bool) -> Self {
        match (recent, now, channels) {
            (true, true, true) => Mode::RecentNowChannels,
            (true, true, false) => Mode::RecentNow,
            (true, false, true) => Mode::RecentChannels,
            (true, false, false) => Mode::RecentOnly,
            (false, true, true) => Mode::NowChannels,
            (false, true, false) => Mode::NowOnly,
            (false, false, true) => Mode::ChannelsOnly,
            // every channel of a lineup is on now, in Recent or in Channels: no channel, no row
            (false, false, false) => Mode::Empty,
        }
    }

    /// The sections that take room, top to bottom.
    fn rows(&self) -> &'static [Sec] {
        match self {
            Mode::Loading | Mode::Empty => &[],
            Mode::Failed => &[Sec::Status],
            Mode::RecentNow => &[Sec::Recent, Sec::Now],
            Mode::RecentOnly => &[Sec::Recent],
            Mode::NowOnly => &[Sec::Now],
            Mode::RecentChannels => &[Sec::Recent, Sec::Channels],
            Mode::RecentNowChannels => &[Sec::Recent, Sec::Now, Sec::Channels],
            Mode::NowChannels => &[Sec::Now, Sec::Channels],
            Mode::ChannelsOnly => &[Sec::Channels],
            Mode::Guide(_) => &[Sec::Guide],
            Mode::Browse(_) => &[Sec::Browse],
            // the read-out is the status section's; the row above it is all there is to reach
            Mode::BrowseNone(_) => &[],
        }
    }

    /// Browse's set while Browse shows, with a card or none.
    fn browse_set(&self) -> Option<&Browse> {
        match self {
            Mode::Browse(set) | Mode::BrowseNone(set) => Some(set),
            _ => None,
        }
    }

    /// The mode of Browse's `set`: the grid when it holds a programme, else the empty read-out.
    fn browse(set: Browse) -> Self {
        if set.picks.is_empty() { Mode::BrowseNone(set) } else { Mode::Browse(set) }
    }

    fn has(&self, k: Sec) -> bool {
        self.rows().contains(&k)
    }

    /// Whether the page is a status read-out (the working, empty and failed ones) rather than rows.
    fn reads_out(&self) -> bool {
        matches!(self, Mode::Loading | Mode::Empty | Mode::Failed | Mode::BrowseNone(_))
    }

    /// The first row: where the arrival is seated.
    fn head(&self) -> Option<Sec> {
        self.rows().first().copied()
    }

    /// Is another shelf drawn under row `k`?
    fn shelf_under(&self, k: Sec) -> bool {
        self.rows().iter().position(|&r| r == k).is_some_and(|i| i + 1 < self.rows().len())
    }
}

/// **Where a Guide cell's words come from**: the channel's airing now, or one of its later programmes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CellSrc { Now, Later(usize) }

/// **Where the Guide's window stands**: on the live half hour (it follows the clock as the half hour rolls over) or held
/// `Ahead` of it at the half hour the reader scrolled to. It is the one state a shift changes, and the window's start
/// follows from it and the clock alone: a held window never lies at or before the live one (the live window catching up to
/// it makes it live again, [`settled`](Self::settled)), and never moves earlier than the live one
/// ([`earlier`](Self::earlier)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Window {
    Live,
    /// Held at this start (Unix seconds, a half hour), later than the live window's.
    Ahead(i64),
}

impl Window {
    /// The window's start at the clock's `now`.
    fn start(self, now: i64) -> i64 {
        match self {
            Window::Live => window_start(now),
            Window::Ahead(start) => start,
        }
    }

    /// The window after the clock has moved to `now`: a held start the live window has reached is the live window.
    fn settled(self, now: i64) -> Self {
        match self {
            Window::Ahead(start) if start <= window_start(now) => Window::Live,
            other => other,
        }
    }

    /// One step later.
    fn later(self, now: i64) -> Self {
        Window::Ahead(self.start(now) + WINDOW_STEP_SECONDS)
    }

    /// One step earlier, and no earlier than the live window.
    fn earlier(self, now: i64) -> Self {
        match self {
            Window::Live => Window::Live,
            Window::Ahead(start) => Window::Ahead(start - WINDOW_STEP_SECONDS).settled(now),
        }
    }
}

/// **The Guide's board and what its cells are**: the [`Window`] it is laid over, the [`TimeGrid`] (one row a lineup
/// channel, in the lineup's order, so a row is a channel's index) and, per row, the source of each of its cells. A row
/// with no cell is a channel the guide has nothing for in the window (the grid's one unscheduled cell).
#[derive(Debug, PartialEq)]
struct Board {
    window: Window,
    grid: TimeGrid,
    cells: Vec<Vec<CellSrc>>,
    /// The clock's instant at the frame this board was last synced at: what an unscheduled row's neighbour is
    /// entered by (the programme on now), which a move reads without a frame to ask.
    now: i64,
}

impl Board {
    /// The board for `channels` over `window`, which starts at `start`, on page `page` (clamped): each channel's
    /// airing now and its later programmes that stand in the window, in time order.
    fn build(channels: &[Channel], window: Window, start: i64, now: i64, page: usize) -> Self {
        let axis = Axis::new(start, WINDOW_SECONDS, WINDOW_STEP_SECONDS);
        fn push(axis: Axis, span: Span, src: CellSrc, spans: &mut Vec<Span>, cells: &mut Vec<CellSrc>) {
            if axis.cell(span).is_some() {
                spans.push(span);
                cells.push(src);
            }
        }
        let (mut rows, mut cells) = (Vec::with_capacity(channels.len()), Vec::with_capacity(channels.len()));
        for channel in channels {
            let (mut spans, mut srcs) = (Vec::new(), Vec::new());
            if !channel.now.is_empty() {
                push(axis, Span::new(channel.begins_at, channel.ends_at), CellSrc::Now, &mut spans, &mut srcs);
            }
            for (i, later) in channel.later.iter().enumerate() {
                push(axis, Span::new(later.begins_at, later.ends_at), CellSrc::Later(i), &mut spans, &mut srcs);
            }
            rows.push(spans);
            cells.push(srcs);
        }
        Self { window, grid: TimeGrid::new(axis, rows, GUIDE_FIRST_ELEM).at_page(page), cells, now }
    }

    /// **Does the guide go on past the window?** Some programme runs past its end (one the window cuts, or one that
    /// begins at the end), so a shift has something to show. When none does, the guide's data has ended and the window
    /// stops. It reads the store's channels, which hold what the last read answered: a shift made faster than the read
    /// lands waits for it.
    fn has_more(&self, channels: &[Channel]) -> bool {
        let end = self.grid.axis().end();
        channels.iter().any(|c| (!c.now.is_empty() && c.ends_at > end) || c.later.iter().any(|later| later.ends_at > end))
    }

    /// The cell's words: its programme's title, or the channel's name for the unscheduled cell.
    fn title<'a>(&self, channels: &'a [Channel], at: Cell) -> &'a str {
        let Some(channel) = channels.get(at.row) else { return "" };
        match self.cells.get(at.row).and_then(|row| row.get(at.index)) {
            Some(CellSrc::Now) => &channel.now,
            Some(CellSrc::Later(i)) => channel.later.get(*i).map_or("", |later| later.title.as_str()),
            None => &channel.name,
        }
    }

    /// Does pressing the cell tune the channel? What is airing does, and so does the unscheduled cell; a later
    /// programme does not. "Airing" is the wall time's, not the read's: a programme that has begun since the read is on.
    fn tunes(&self, at: Cell, now: i64) -> bool {
        match self.cells.get(at.row).and_then(|row| row.get(at.index)) {
            None => self.grid.is_unscheduled(at.row),
            Some(CellSrc::Now) => true,
            Some(CellSrc::Later(_)) => self.grid.span(at).is_some_and(|span| span.contains(now)),
        }
    }
}

/// **What Browse shows**: the programmes the filter admits at the clock's instant ([`browse::shown`]), each with its element
/// ([`Idents`]), the total the header's count is read against, the end of the local day the set was made for, and the first
/// instant at which the set may change with no new data ([`browse::next_change`]): the page rebuilds it when the clock reaches it.
#[derive(Debug, PartialEq)]
struct Browse {
    picks: Vec<Pick>,
    /// The element of card `i`.
    elems: Vec<u32>,
    /// The card an element stands for: the inverse of `elems`.
    index: HashMap<u32, usize>,
    /// How many programmes are on air: the "of" of the narrowing filters' count.
    on_now: usize,
    day_end: i64,
    until: i64,
}

impl Browse {
    fn build(channels: &[Channel], filter: Filter, now: i64, day_end: i64, idents: &mut Idents) -> Self {
        let picks = browse::shown(channels, filter, now, day_end);
        let elems: Vec<u32> = picks.iter().map(|pick| idents.elem(&channels[pick.channel], pick.begins_at)).collect();
        let index: HashMap<u32, usize> = elems.iter().enumerate().map(|(i, elem)| (*elem, i)).collect();
        // a day that turned lets go of the programmes a page no longer shows or can reach
        idents.turn_day(day_end, |elem| index.contains_key(&elem));
        Self { picks, elems, index, on_now: browse::on_now(channels, now), day_end, until: browse::next_change(channels, now, day_end) }
    }

    /// Has the clock reached the instant the set may change at?
    fn due(&self, now: i64) -> bool {
        now >= self.until
    }

    /// The card of a programme of `channel`, in lineup order: its own (the one airing now first), else where it would stand
    /// ([`browse::seat_for`]), as its element.
    fn seat(&self, channel: usize, now: i64) -> Option<u32> {
        self.elems.get(browse::seat_for(&self.picks, channel, now)?).copied()
    }
}

/// What a Guide row's sticky cell and cells say: the board's [`time_grid::Content`] over the lineup.
struct Faces<'a> {
    channels: &'a [Channel],
    board: &'a Board,
}

impl time_grid::Content for Faces<'_> {
    fn channel(&self, p: plx_ui::Painter, row: usize, at: Rect, _focused: bool) {
        let Some(channel) = self.channels.get(row) else { return };
        channel_tile::draw_guide_face(p, at, &channel_tile::Face { sid: channel.sid.raw(), logo: &channel.thumb, number: &channel.vcn, name: &channel.name });
    }

    fn title(&self, row: usize, index: usize) -> &str {
        self.board.title(self.channels, Cell { row, index })
    }
}

/// What a sync was made from: the store's revision, the scope, and, while a clock-dependent page shows, its anchor: the
/// Guide's window start or Browse's end of the local day. What's on has none, so it does not rebuild every thirty minutes.
type SyncKey = (u32, Scope, Option<i64>);

struct Page {
    entry: EntryId,
    /// Which page shows ([`Scope`]).
    scope: Scope,
    /// How many times a sync or a page turn changed what the sections read: mixed into the stack's revision, so a
    /// page the board turned is laid out again.
    epoch: u32,
    /// The lineup, one `(server, key)` per channel in the store's order (the cards' stable identities).
    channels: Vec<(plx_plex::plex::ServerId, String)>,
    /// The channels with a programme on, as indexes into the lineup, in On now's one order: the most recent
    /// start first, a tie in lineup order.
    airings: Vec<usize>,
    /// Recent's channels, as indexes into the lineup, in the store's order (newest first).
    recent: Vec<usize>,
    /// The lineup channels with no programme on that are not in Recent, as indexes into the lineup, in the
    /// lineup's order: the Channels shelf, which keeps a channel without guide data reachable.
    rest: Vec<usize>,
    /// The rows the page shows ([`Page::sync`] decides it from the store's status and the three lists).
    mode: Mode,
    /// The focus groups of the rows, top to bottom, from [`Mode::rows`].
    content: Vec<GroupId>,
    synced: Option<SyncKey>,
    /// What the airings' words and bars were last drawn from ([`Page::clock_stamp`]); not state, a
    /// frame's reason.
    clock: u64,
    header_capsules: plx_ui::widgets::TabStrip,
    header_pop: plx_ui::widgets::CtlPop<1>,
    handoff: plx_ui::scroll_chrome::Handoff,
    header_rows: SharedRow<HeaderRow>,
    /// What the row carries over from the row it replaced ([`HeaderSwap`]): a filter picked, a count that moved, a control that
    /// comes or goes dissolves instead of cutting. Begun from the rebuild itself ([`Page::header_row`], `SharedRow::get_replacing`),
    /// as the Watchlist's is.
    header_swap: std::cell::RefCell<plx_ui::page_header::HeaderSwap>,
    /// The elements of Browse's cards ([`Idents`]).
    idents: Idents,
}

impl Page {
    /// The Guide's board while [`Mode::Guide`] shows.
    fn board(&self) -> Option<&Board> {
        match &self.mode {
            Mode::Guide(board) => Some(board),
            _ => None,
        }
    }

    /// Follow the store, the scope and the clock's `instant`: rebuild what the page shows when what it was made from moved.
    /// `true` when it did (the board, if any, is a new one).
    fn sync(&mut self, view: LiveTvView<'_>, instant: Option<i64>, offset: Option<i32>) -> bool {
        let guide = self.scope == Scope::Guide;
        let browse = self.scope.filter();
        // The Guide's window is held where the reader put it (it follows the clock until then); a frame with no clock
        // has none to lay it against.
        let window = match &mut self.mode {
            Mode::Guide(board) if guide => {
                if let Some(now) = instant { board.now = now; }
                instant.map_or(board.window, |now| board.window.settled(now))
            }
            _ => Window::Live,
        };
        let start = instant.filter(|_| guide).map(|now| window.start(now));
        // Browse is made for the end of the local day, which a zone change moves; it has no window
        let day = instant.filter(|_| browse.is_some()).map(|now| browse::day_end(now, offset));
        let key = (view.revision(), self.scope, start.or(day));
        // a held window the live one has caught up to has the same start and is another window; a Browse set whose
        // next boundary the clock has passed (a programme began, or is ten minutes in, or fifteen from its end, or ended) is another set
        let settled = matches!(&self.mode, Mode::Guide(board) if board.window != window)
            || matches!((self.mode.browse_set(), instant), (Some(set), Some(now)) if set.due(now));
        if self.synced == Some(key) && !settled { return false; }
        self.synced = Some(key);
        self.epoch = self.epoch.wrapping_add(1);
        self.channels = view.tiles.iter().map(|m| (m.sid, m.rk.clone())).collect();
        self.airings = on_now_order(view.channels);
        // An empty Recent is a row to hide, whatever else the page holds.
        self.recent = view.recent.iter().filter_map(|r| lineup_index(view.channels, r)).collect();
        // Every channel stays reachable: one with no programme on that is not in Recent has no tile above, so
        // it is in the Channels shelf.
        self.rest = (0..view.channels.len()).filter(|i| view.channels[*i].now.is_empty() && !self.recent.contains(i)).collect();
        // a rebuilt board keeps the page the reader was on
        let page = self.board().map_or(0, |board| board.grid.page());
        self.mode = match view.status {
            LiveTvStatus::Loading => Mode::Loading,
            LiveTvStatus::Empty => Mode::Empty,
            LiveTvStatus::Failed => Mode::Failed,
            LiveTvStatus::Ready if view.channels.is_empty() => Mode::Empty,
            // the Guide without a clock is still loading: its window has nothing to start from
            LiveTvStatus::Ready if guide => match (start, instant) {
                (Some(start), Some(now)) => Mode::Guide(Board::build(view.channels, window, start, now, page)),
                _ => Mode::Loading,
            },
            // Browse without a clock is still loading: what is on now cannot be told
            LiveTvStatus::Ready if browse.is_some() => match (browse, instant, day) {
                (Some(filter), Some(now), Some(day_end)) => Mode::browse(Browse::build(view.channels, filter, now, day_end, &mut self.idents)),
                _ => Mode::Loading,
            },
            LiveTvStatus::Ready => Mode::of(!self.recent.is_empty(), !self.airings.is_empty(), !self.rest.is_empty()),
        };
        self.content = self.mode.rows().iter().map(|k| k.group()).collect();
        true
    }

    /// **Move the Guide's window one step** later (when the guide goes on past it) or earlier (never before the live
    /// window), rebuilding the board over the new window on the page the reader was on. `false` when it cannot move.
    fn shift(&mut self, view: LiveTvView<'_>, instant: Option<i64>, later: bool) -> bool {
        let (Some(now), Mode::Guide(board)) = (instant, &self.mode) else { return false };
        if later && !board.has_more(view.channels) { return false; }
        let window = if later { board.window.later(now) } else { board.window.earlier(now) };
        self.set_window(view, now, window)
    }

    /// Return the Guide's window to the live one. `false` when it is there.
    fn go_live(&mut self, view: LiveTvView<'_>, instant: Option<i64>) -> bool {
        let Some(now) = instant else { return false };
        self.set_window(view, now, Window::Live)
    }

    /// Rebuild the board over `window`, if it is not the one it has.
    fn set_window(&mut self, view: LiveTvView<'_>, now: i64, window: Window) -> bool {
        let Mode::Guide(board) = &self.mode else { return false };
        if window == board.window { return false; }
        let start = window.start(now);
        self.mode = Mode::Guide(Board::build(view.channels, window, start, now, board.grid.page()));
        self.synced = Some((view.revision(), self.scope, Some(start)));
        self.epoch = self.epoch.wrapping_add(1);
        true
    }

    /// How far ahead the store must read the guide for what shows: the window's end and a step more, so the next shift has
    /// its programmes, or the end of the day under Browse's Later today ([`LiveTvCmd::Reach`]); `0` when neither shows.
    fn reach(&self) -> i64 {
        match (&self.scope, self.mode.browse_set()) {
            // Later today reads to the end of the local day: the Guide's own re-read rule, asked for further
            (Scope::Browse(Filter::LaterToday), Some(set)) => set.day_end,
            _ => self.board().map_or(0, |board| board.grid.axis().end() + WINDOW_STEP_SECONDS),
        }
    }

    /// **The page follows the focus**: show the page `elem`'s row is on. `true` when that turned the page.
    fn follow(&mut self, elem: u32) -> bool {
        let Mode::Guide(board) = &mut self.mode else { return false };
        let Some(cell) = board.grid.locate(elem) else { return false };
        let turned = board.grid.follow(cell.row);
        self.epoch = self.epoch.wrapping_add(u32::from(turned));
        turned
    }

    /// The page indicator's words, "Channels 1-6 of 16".
    fn indicator(&self) -> String {
        let Some(board) = self.board() else { return String::new() };
        let (first, last, total) = board.grid.indicator();
        let locale = plx_os::i18n::current();
        plx_os::i18n::msg::browse_livetv_guide_range(&locale.number(first as i64), &locale.number(last as i64), &locale.number(total as i64))
    }

    /// One number that moves exactly when the words or the bar of any airing the page shows does
    /// ([`AiringPhase::stamp`]), or, on the Guide and Browse, the minute (the now line and chip, the minutes left and the bar) and whether it has a zone: what the
    /// page asks a frame for. `0` without a wall time (the Guide needs only the instant).
    fn clock_stamp(&self, view: LiveTvView<'_>, instant: Option<i64>, wall: Option<Wall>) -> u64 {
        // The Guide's now line and chip move with the minute, and its clock words come with the zone
        if let (Mode::Guide(_) | Mode::Browse(_), Some(now)) = (&self.mode, instant) {
            return now.div_euclid(60) as u64 | u64::from(wall.is_some()) << 62 | 1 << 63;
        }
        let Some(wall) = wall else { return 0 };
        self.recent.iter().chain(&self.airings).filter_map(|&i| view.channels.get(i))
            .fold(0xcbf2_9ce4_8422_2325_u64, |acc, c| {
                (acc ^ fmt::phase(c.begins_at, c.ends_at, wall.unix_s()).stamp()).wrapping_mul(0x0100_0000_01b3)
            })
    }

    /// The first row's pitch: the scroll at which what stands under the top chrome at rest has gone (a plate
    /// row's is shorter than a poster row's, so the chrome tucks sooner). `0` with no row, which the handoff
    /// floors.
    fn first_row_pitch(&self) -> f32 {
        match self.mode.head() {
            Some(Sec::Browse) => browse_spec().pitch(),
            head => head.and_then(Sec::style).map_or(0.0, RowStyle::pitch_fixed),
        }
    }

    /// How many stops the scope row has: the scopes, and Browse's filter control after them while it shows.
    fn head_stops(&self) -> usize {
        Scope::ALL.len() + usize::from(self.browsing())
    }

    /// The `n`-th stop of the scope row, left to right.
    fn head_elem(&self, n: usize) -> Option<u32> {
        Scope::ALL.get(n).map(|scope| scope.elem()).or_else(|| (n == Scope::ALL.len() && self.browsing()).then_some(FILTER_ELEM))
    }

    /// Is Browse showing? Its scope row carries the filter control as its last stop. The row rides the page here as on every
    /// page (the Library's: it scrolls away with the grid and the top chrome tucks through the one `Handoff`).
    fn browsing(&self) -> bool {
        self.mode.browse_set().is_some()
    }

    /// **The page's column**: the scope row, then the rows the mode shows. The page arrives in its content; while the
    /// lineup is still loading it seats nothing (the row is one Up away, as on the Library and the Watchlist).
    fn regions(&self) -> PageRegions<'_> {
        PageRegions {
            header: Some(HEAD_GROUP),
            content: &self.content,
            rail: None,
            back_walks_header: false,
            arrival: Arrival::Content,
            pending: (self.mode == Mode::Loading).then_some(RECENT_GROUP),
        }
    }

    /// **Browse's header read-out**, "3 of 16": while the filter narrows what is on now ([`browse::count`]), in the Library's own
    /// words (`library_header.filtered_of`) and ink. Read from the scope's filter and the set made for it: the two cannot
    /// differ, for a filter changes only through [`LiveTvScreen::pick_filter`], which syncs the set at once.
    fn readout(&self) -> Option<String> {
        let (filter, set) = (self.scope.filter()?, self.mode.browse_set()?);
        let (shown, total) = browse::count(filter, set.picks.len(), set.on_now)?;
        let locale = plx_os::i18n::current();
        Some(plx_os::i18n::msg::library_header_filtered_of(&locale.number(shown as i64), &locale.number(total as i64)))
    }

    fn header_row(&self, measure: &dyn plx_machine::machine::Measure) -> std::rc::Rc<HeaderRow> {
        use std::hash::{Hash, Hasher};
        let readout = self.readout();
        // one row per locale, scope (and filter), whether the filter control stands on it and its read-out: the chosen scope wears the selected plate
        let key = {
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            (std::ptr::from_ref(plx_os::i18n::current()) as usize, self.scope.code(), self.browsing(), &readout).hash(&mut hash);
            hash.finish()
        };
        self.header_rows.get_replacing(key, || {
            use plx_os::i18n::msg;
            let item = |scope: Scope, label: &str| HeaderItem { label: label.to_owned(), selected_plate: self.scope.same_page(scope), focusable: true, ..Default::default() };
            let left = vec![
                item(Scope::WhatsOn, msg::browse_livetv_whats_on()),
                item(Scope::Guide, msg::browse_livetv_guide()),
                item(Scope::Browse(Filter::OnNow), msg::browse_livetv_browse()),
            ];
            // Browse's filter is the row's last stop, drawn as the Library's sort control, with the count before it
            let mut right = Vec::new();
            if let Some(filter) = self.scope.filter().filter(|_| self.browsing()) {
                if let Some(readout) = readout {
                    right.push(HeaderItem { label: readout, readout: true, ..Default::default() });
                }
                right.push(HeaderItem { a11y: Some("filter"), ..HeaderItem::sort_control(msg::browse_library_filter(), filter_label(filter)) });
            }
            let page = PageHeader::new(PageHeaderSpec { left, right, ..Default::default() }, &[], measure);
            let layout = page.layout(SCR_W, measure);
            // the n-th laid-out left item is the n-th scope; the one focusable right item is the filter
            let stops = layout.items.iter().enumerate()
                .filter_map(|(index, rect)| match rect.slot {
                    plx_ui::page_header::Slot::Left(n) => Scope::ALL.get(n).map(|scope| (scope.elem(), index)),
                    plx_ui::page_header::Slot::Right(_) if rect.focusable => Some((FILTER_ELEM, index)),
                    _ => None,
                })
                .collect();
            HeaderRow::new(page, layout, stops)
        }, |old, new| self.header_swap.borrow_mut().begin(old, new))
    }

    /// The filter control as the focused pill draws it, where the menu's glass grows out of and drains back into (in the
    /// row's document place: the caller shifts it by the page's scroll).
    fn filter_pill(&self, measure: &dyn plx_machine::machine::Measure) -> Option<Rect> {
        self.header_row(measure).plate(FILTER_ELEM).map(plx_ui::widgets::focus_pill)
    }

    fn status_overlay(&self, tick: u32) -> StatusOverlay<'static> {
        match self.mode {
            // placeholder-exempt: built here, counted by StatusOverlay's Working draw.
            Mode::Loading => StatusOverlay::new(STATUS_FRAME, plx_os::i18n::msg::browse_livetv_loading_c(), StatusKind::Working).phase(tick),
            Mode::Empty => StatusOverlay::new(STATUS_FRAME, plx_os::i18n::msg::browse_livetv_empty_c(), StatusKind::Empty),
            Mode::BrowseNone(_) => StatusOverlay::new(STATUS_FRAME, plx_os::i18n::msg::browse_livetv_browse_empty_c(), StatusKind::Empty),
            Mode::Failed => StatusOverlay::new(Rect::FULL, plx_os::i18n::msg::browse_home_failed_c(), StatusKind::Failed)
                .page(plx_ui::icons::Icon::ServerBadgeMinus).action(plx_os::i18n::msg::browse_action_retry_c()),
            // rows are showing: nothing to read out (never drawn)
            _ => StatusOverlay::new(STATUS_FRAME, c"", StatusKind::Empty),
        }
    }
}

/// On now's one order, as indexes into `channels` (the lineup, in number order): the channels with a
/// programme, the most recent start first; a tie keeps the lineup's order (the sort is stable).
fn on_now_order(channels: &[Channel]) -> Vec<usize> {
    let mut order: Vec<usize> = channels.iter().enumerate().filter_map(|(i, c)| (!c.now.is_empty()).then_some(i)).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(channels[i].begins_at));
    order
}

/// Where a Recent channel stands in the lineup: a channel is its server, DVR and key, never its number.
fn lineup_index(channels: &[Channel], of: &Channel) -> Option<usize> {
    channels.iter().position(|c| c.sid == of.sid && c.dvr == of.dvr && c.channel == of.channel)
}

/// Which kind of tile a [`Cards`] serves: it decides the cards' art, words and element ids.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Row {
    /// A channel plate: Recent's, and the Channels shelf's (the lineup channels with no programme on that are
    /// not in Recent).
    Plates,
    /// A programme poster: On now's.
    Now,
}

struct Cards<'a> { view: LiveTvView<'a>, order: &'a [usize], row: Row, wall: Option<Wall> }
impl Cards<'_> {
    /// The lineup index of card `i`, in the shelf's own order.
    fn index(&self, i: usize) -> Option<usize> {
        self.order.get(i).copied()
    }
    fn channel(&self, i: usize) -> Option<(usize, &Channel)> {
        let index = self.index(i)?;
        Some((index, self.view.channels.get(index)?))
    }
    /// The amber bar: how much of the programme has aired. `Resume`'s own rule (present exactly while
    /// in progress) is the airing's phase.
    fn resume(&self, channel: &Channel) -> Option<Resume> {
        let w = self.wall?;
        if !matches!(fmt::phase(channel.begins_at, channel.ends_at, w.unix_s()), AiringPhase::JustStarted { .. } | AiringPhase::Running { .. }) {
            return None;
        }
        Some(Resume { frac: fmt::frac(channel.begins_at, channel.ends_at, w)?, left_ms: (channel.ends_at - w.unix_s()).max(0) * 1_000 })
    }
    /// The low bit of a card's element id: the plates (Recent's and the Channels shelf's, no channel is in both)
    /// are even, On now's posters odd.
    fn parity(&self) -> u32 { u32::from(self.row == Row::Now) }
}
impl<H: LiveTvLike> CardSource<H> for Cards<'_> {
    fn len(&self) -> usize { self.order.len() }
    fn elem(&self, i: usize) -> u32 { FIRST_CARD_ELEM + self.index(i).unwrap_or(0) as u32 * 2 + self.parity() }
    fn index_of(&self, e: &u32) -> Option<usize> {
        let offset = e.checked_sub(FIRST_CARD_ELEM)?;
        if offset % 2 != self.parity() { return None; }
        let i = offset as usize / 2;
        self.order.iter().position(|&n| n == i)
    }
    fn art(&self, i: usize) -> Art<'_> {
        let Some((index, channel)) = self.channel(i) else { return Art::Poster(None) };
        if self.row == Row::Plates {
            return Art::Channel { sid: channel.sid.raw(), logo: &channel.thumb, number: &channel.vcn, name: &channel.name };
        }
        Art::Poster(self.view.tiles.get(index).map(|tile| {
            let mut facts = tile_facts::of(tile);
            facts.owner = channel as *const _ as usize;
            facts.title = &channel.now;
            facts.show_title = "";
            facts.thumb = &channel.now_thumb;
            facts.still = "";
            facts.art = "";
            facts.resume = self.resume(channel);
            facts
        }))
    }
    /// The poster's bar; a channel plate has none.
    fn progress(&self, i: usize) -> Option<f32> {
        if self.row != Row::Now { return None; }
        self.channel(i).and_then(|(_, c)| self.resume(c)).map(|r| r.frac)
    }
    fn label(&self, i: usize) -> TileLabel {
        let Some((_, channel)) = self.channel(i) else { return TileLabel::default() };
        use plx_os::i18n::msg;
        match self.row {
            // "On now: <programme>" over when it started and how long is left; a channel with no
            // programme (every Channels plate) is its name alone.
            Row::Plates if channel.now.is_empty() => captions::live_label(channel.name.clone(), None),
            Row::Plates => captions::live_label(msg::browse_livetv_recent_on_now(&channel.now),
                self.wall.and_then(|w| fmt::airing_line(Airing::Tile, channel.begins_at, channel.ends_at, w))),
            // The programme over its channel, number first ("Ch 2009 · Station · 42m left").
            Row::Now => airing_caption(channel.now.clone(), channel, self.wall.and_then(|w| fmt::left(channel.begins_at, channel.ends_at, w))),
        }
    }

    fn subject(&self, i: usize) -> Option<std::borrow::Cow<'_, str>> {
        channel_subject(self.view, self.channel(i)?.0).map(Into::into)
    }
}

/// **Browse's cards**: one per programme the filter shows ([`Browse::picks`]), each a poster of the programme with the DS amber bar of
/// how much has aired (a programme not begun has none), and, focused, the label: the programme, then
/// "Ch 2000 · Pluto TV Spotlight · 2h 22m left" (a later programme says when it begins in place of the time left). The label is
/// placed by the grid, under the card, through `card_row::anchor_label` as On now's is.
struct BrowseCards<'a> { view: LiveTvView<'a>, set: &'a Browse, wall: Option<Wall> }
impl BrowseCards<'_> {
    fn channel(&self, i: usize) -> Option<(&Channel, Pick)> {
        let pick = *self.set.picks.get(i)?;
        Some((self.view.channels.get(pick.channel)?, pick))
    }
    /// The programme's words and art: its title and its thumb.
    fn programme<'c>(channel: &'c Channel, pick: Pick) -> (&'c str, &'c str) {
        match pick.src {
            CellSrc::Now => (&channel.now, &channel.now_thumb),
            CellSrc::Later(n) => channel.later.get(n).map_or(("", ""), |later| (later.title.as_str(), later.thumb.as_str())),
        }
    }
    /// The amber bar: how much has aired, while the guide knows both ends and the programme is on.
    fn resume(&self, pick: Pick) -> Option<Resume> {
        let w = self.wall?;
        if !matches!(browse::stand(pick.begins_at, pick.ends_at, w.unix_s()), browse::Stand::OnAir { left: Some(_), .. }) {
            return None;
        }
        Some(Resume { frac: fmt::frac(pick.begins_at, pick.ends_at, w)?, left_ms: (pick.ends_at - w.unix_s()).max(0) * 1_000 })
    }
}
impl<H: LiveTvLike> CardSource<H> for BrowseCards<'_> {
    fn len(&self) -> usize { self.set.picks.len() }
    fn elem(&self, i: usize) -> u32 { self.set.elems.get(i).copied().unwrap_or(BROWSE_FIRST_ELEM) }
    fn index_of(&self, e: &u32) -> Option<usize> { self.set.index.get(e).copied() }
    fn art(&self, i: usize) -> Art<'_> {
        let Some((channel, pick)) = self.channel(i) else { return Art::Poster(None) };
        Art::Poster(self.view.tiles.get(pick.channel).map(|tile| {
            let (title, thumb) = Self::programme(channel, pick);
            let mut facts = tile_facts::of(tile);
            // a programme's own identity: its address stands for it as the channel's stood for the airing on now
            facts.owner = match pick.src {
                CellSrc::Now => channel as *const _ as usize,
                CellSrc::Later(n) => &channel.later[n] as *const _ as usize,
            };
            facts.title = title;
            facts.show_title = "";
            facts.thumb = thumb;
            facts.still = "";
            facts.art = "";
            facts.resume = self.resume(pick);
            facts
        }))
    }
    fn progress(&self, i: usize) -> Option<f32> {
        self.resume(self.channel(i)?.1).map(|r| r.frac)
    }
    fn label(&self, i: usize) -> TileLabel {
        let Some((channel, pick)) = self.channel(i) else { return TileLabel::default() };
        // the time left while on, when it begins before it has: with no zone offset the identity says it alone
        let when = self.wall.and_then(|w| match browse::stand(pick.begins_at, pick.ends_at, w.unix_s()) {
            browse::Stand::OnAir { left: Some(_), .. } => fmt::left(pick.begins_at, pick.ends_at, w),
            browse::Stand::Upcoming => fmt::clock_face(pick.begins_at, i64::from(w.utc_offset_s)),
            _ => None,
        });
        airing_caption(Self::programme(channel, pick).0.to_owned(), channel, when)
    }
    fn subject(&self, i: usize) -> Option<std::borrow::Cow<'_, str>> {
        channel_subject(self.view, self.channel(i)?.1.channel).map(Into::into)
    }
}

/// **A programme's caption wherever a poster shows one** (What's on's On now and Browse): the programme over its channel's
/// identity, number first, then when ("Ch 2009 · Station · 42m left"), through the one live caption policy
/// ([`captions::live_label`]). A line longer than its band is the shared caption's to hold: it glides on the shared marquee
/// from its head, as every caption line does.
fn airing_caption(programme: String, channel: &Channel, when: Option<String>) -> TileLabel {
    use plx_os::i18n::msg;
    let identity = if channel.vcn.is_empty() { channel.name.clone() } else { msg::browse_livetv_channel(&channel.name, &channel.vcn) };
    let line = match when {
        Some(when) => msg::core_airing_pair(&identity, &when),
        None => identity,
    };
    captions::live_label(programme, Some(line))
}

/// What a card section reads its cards from: a shelf's channels and programmes, or Browse's programmes.
enum Source<'a> { Shelf(Cards<'a>), Browse(BrowseCards<'a>) }
impl<H: LiveTvLike> CardSource<H> for Source<'_> {
    fn len(&self) -> usize { match self { Source::Shelf(c) => CardSource::<H>::len(c), Source::Browse(c) => CardSource::<H>::len(c) } }
    fn elem(&self, i: usize) -> u32 { match self { Source::Shelf(c) => CardSource::<H>::elem(c, i), Source::Browse(c) => CardSource::<H>::elem(c, i) } }
    fn index_of(&self, e: &u32) -> Option<usize> { match self { Source::Shelf(c) => CardSource::<H>::index_of(c, e), Source::Browse(c) => CardSource::<H>::index_of(c, e) } }
    fn art(&self, i: usize) -> Art<'_> { match self { Source::Shelf(c) => CardSource::<H>::art(c, i), Source::Browse(c) => CardSource::<H>::art(c, i) } }
    fn progress(&self, i: usize) -> Option<f32> { match self { Source::Shelf(c) => CardSource::<H>::progress(c, i), Source::Browse(c) => CardSource::<H>::progress(c, i) } }
    fn label(&self, i: usize) -> TileLabel { match self { Source::Shelf(c) => CardSource::<H>::label(c, i), Source::Browse(c) => CardSource::<H>::label(c, i) } }
    fn subject(&self, i: usize) -> Option<std::borrow::Cow<'_, str>> { match self { Source::Shelf(c) => CardSource::<H>::subject(c, i), Source::Browse(c) => CardSource::<H>::subject(c, i) } }
}

/// **The accessibility subject of lineup channel `index`**, which every card and Guide cell of it acts on. Two servers
/// may advertise the same DVR and channel, so a press names the exact source; hex preserves the key without
/// punctuation the remote subject grammar rejects.
fn channel_subject(view: LiveTvView<'_>, index: usize) -> Option<String> {
    use std::fmt::Write as _;
    let tile = view.tiles.get(index)?;
    let mut subject = format!("live-{}-", tile.sid.raw());
    for byte in tile.rk.bytes() {
        write!(subject, "{byte:02x}").expect("writing to a String succeeds");
    }
    Some(subject)
}

impl<H: LiveTvLike> StackPage<H> for Page {
    type Key = Sec;
    type Cards<'a> = Source<'a>;
    fn revision(&self, cx: &Cx<'_, H>) -> u64 { u64::from(H::livetv(cx).revision()) | u64::from(self.epoch) << 32 }
    /// A focus whose cell a refresh took away stays on its channel: the last cell of its row.
    fn recover(&self, cx: &Cx<'_, H>, want: &u32) -> Option<u32> {
        // A Browse card that left the set (its programme ended or began, or the filter changed) is followed to its channel: that
        // channel's card in the set, else the card where the channel would stand ([`browse::seat_for`]). Nothing shown leaves it to the
        // fallback, which seats the filter control.
        if let Some(set) = self.mode.browse_set() {
            let key = self.idents.programme(*want)?;
            let index = H::livetv(cx).channels.iter().position(|c| c.sid == key.sid && c.dvr == key.dvr && c.channel == key.channel)?;
            return set.seat(index, H::instant(cx)?);
        }
        let board = self.board()?;
        let row = board.grid.row_of(*want)?;
        board.grid.elem(Cell { row, index: board.grid.cells(row).checked_sub(1)? })
    }
    fn sections(&self, cx: &Cx<'_, H>, out: &mut Vec<SectionSpec<Sec>>) {
        let heading = BOARD_HEADING - heading_ink_drop(cx.measure);
        let guide = matches!(self.mode, Mode::Guide(_));
        // The scope row is on every page and is focusable on every one; the Guide's ruler follows it directly,
        // What's on's first shelf under the air its heading keeps.
        let head = if guide {
            time_grid::RULER_Y
        } else if self.browsing() {
            // Browse's grid begins at the grid's top, the row above it
            plx_ui::page_header::GRID_TOP
        } else {
            plx_ui::page_header::GRID_TOP + heading_ink_drop(cx.measure)
        };
        let head = SectionSpec::new(Sec::Head, Kind::Custom { height: head, focusable: true }, Sec::Head.group()).ranked(1).named("header");
        out.push(head);
        out.push(SectionSpec::new(Sec::Guide, Kind::Custom { height: if guide { GUIDE_H } else { 0.0 }, focusable: guide }, Sec::Guide.group()).named("guide"));
        // Browse's grid: the programmes the filter shows, under the row, a row at a time; the first row of the content when it shows
        let browse = SectionSpec::new(Sec::Browse, Kind::Grid { spec: browse_spec() }, Sec::Browse.group()).named("browse");
        out.push(if self.mode.head() == Some(Sec::Browse) { browse.seated(plx_ui::page_regions::CONTENT_HEAD_SEAT) } else { browse });
        // Each row is its own section, with its own style: a shelf keeps its style from the body it was made
        // with, so a key never stands for two row styles. The head of the content is seated by its own rule
        // (`CONTENT_HEAD_SEAT`): the first row there is.
        for (k, name) in [(Sec::Recent, "recent"), (Sec::Now, "on_now"), (Sec::Channels, "channels")] {
            let style = k.style().expect("a shelf has a style");
            let spec = SectionSpec::new(k, Kind::Shelf { style, heading }, k.group()).named(name);
            out.push(if self.mode.head() == Some(k) { spec.seated(plx_ui::page_regions::CONTENT_HEAD_SEAT) } else { spec });
        }
        let retry = (self.mode == Mode::Failed).then(|| self.status_overlay(cx.tick.ms).action_frame_measured(cx.measure)).flatten();
        out.push(SectionSpec::new(Sec::Status, Kind::Overlay { rect: retry.unwrap_or(STATUS_FRAME), focusable: retry.is_some() }, Sec::Status.group()).named("status"));
    }
    /// A shelf with another under it keeps the handoff's 48 from its tiles to that shelf's heading at rest
    /// (the label band's collapsed 18, then this) and the label band's own room (the 552 under Recent) with a
    /// tile focused ([`shelf_foot_open`](Self::shelf_foot_open)); the last shelf keeps nothing.
    fn shelf_foot(&self, cx: &Cx<'_, H>, k: Sec) -> f32 {
        if self.mode.shelf_under(k) { BOARD_GAP_AT_REST + heading_ink_drop(cx.measure) - plx_ui::cards::LABEL_BAND_COLLAPSED } else { 0.0 }
    }
    fn shelf_foot_open(&self, cx: &Cx<'_, H>, k: Sec) -> f32 {
        if self.mode.shelf_under(k) { UNDER_LABEL_AIR + heading_ink_drop(cx.measure) } else { 0.0 }
    }
    /// The air under a shelf is the room of the heading of the shelf below, which that shelf's own reveal brings
    /// on: focusing a shelf scrolls for its tiles and open label band only.
    fn reveal_foot(&self, _k: Sec) -> bool { false }
    fn fallback(&self, _cx: &Cx<'_, H>, out: &mut Vec<Sec>) { out.extend([Sec::Guide, Sec::Browse, Sec::Recent, Sec::Now, Sec::Channels, Sec::Status, Sec::Head]); }
    fn cards<'a>(&'a self, cx: &'a Cx<'_, H>, k: Sec) -> Option<Source<'a>> {
        if k == Sec::Browse {
            return match &self.mode {
                Mode::Browse(set) => Some(Source::Browse(BrowseCards { view: H::livetv(cx), set, wall: H::wall(cx) })),
                _ => None,
            };
        }
        let (order, row) = match k { Sec::Recent => (&self.recent, Row::Plates), Sec::Now => (&self.airings, Row::Now), Sec::Channels => (&self.rest, Row::Plates), _ => return None };
        self.mode.has(k).then(|| Source::Shelf(Cards { view: H::livetv(cx), order, row, wall: H::wall(cx) }))
    }
    fn card_has_menu(&self, _cx: &Cx<'_, H>, _k: Sec, _elem: &u32) -> bool { false }
    fn caption_policy(&self, _cx: &Cx<'_, H>, k: Sec) -> Option<plx_ui::caption_policy::CaptionPolicy> {
        matches!(k, Sec::Recent | Sec::Now | Sec::Channels | Sec::Browse).then(captions::live_tv_policy)
    }
    fn elem_of(&self, k: Sec) -> Option<u32> { (k == Sec::Status).then_some(RETRY_ELEM) }
    /// The scope row has its three stops, and Browse's filter control after them; the Guide its board's cells, the status
    /// read-out its retry.
    fn plain_len(&self, k: Sec) -> usize {
        match k {
            Sec::Head => self.head_stops(),
            Sec::Guide => self.board().map_or(0, |board| board.grid.len()),
            _ => usize::from(k == Sec::Status),
        }
    }
    fn plain_elem(&self, k: Sec, n: usize) -> Option<u32> {
        match k {
            Sec::Head => self.head_elem(n),
            Sec::Guide => self.board().and_then(|board| board.grid.elem(board.grid.nth(n)?)),
            _ => (k == Sec::Status && n == 0).then_some(RETRY_ELEM),
        }
    }
    fn plain_name(&self, k: Sec, i: usize) -> Option<&'static str> {
        match k {
            Sec::Head => Scope::ALL.get(i).map(|scope| scope.name()).or_else(|| (i == Scope::ALL.len() && self.browsing()).then_some("filter")),
            Sec::Status => (i == 0).then_some("retry"),
            _ => None,
        }
    }
    /// A cell says what it is (its programme, or its channel's name) and acts on its channel, as a card does: the
    /// subject is the channel's, the one `press` plays.
    fn plain_a11y<'a>(&'a self, cx: &Cx<'_, H>, k: Sec, n: usize, mut a11y: A11y<'a>) -> A11y<'a> {
        let Some(board) = self.board().filter(|_| k == Sec::Guide) else { return a11y };
        let Some(cell) = board.grid.nth(n) else { return a11y };
        let view = H::livetv(cx);
        let title = board.title(view.channels, cell);
        if !title.is_empty() {
            a11y = a11y.with_label(title.to_owned());
        }
        match channel_subject(view, cell.row) {
            Some(subject) => a11y.with_subject(subject),
            None => a11y,
        }
    }
    fn plain_step(&self, k: Sec, at: usize, dir: plx_ui::screen::Dir) -> Option<usize> {
        use plx_ui::screen::Dir;
        match k {
            Sec::Head => match dir {
                Dir::Left => at.checked_sub(1),
                Dir::Right => Some(at + 1).filter(|next| *next < self.head_stops()),
                Dir::Up | Dir::Down => None,
            },
            Sec::Guide => {
                let board = self.board()?;
                board.grid.position(board.grid.step(board.grid.nth(at)?, dir, board.now)?)
            }
            _ => None,
        }
    }
    /// The scope stops are the row's plates; a Guide cell is the board's rect, set where the section stands (the
    /// board is laid out on the frame, its ruler at [`time_grid::RULER_Y`], which is the section's top).
    fn element_rect(&self, cx: &Cx<'_, H>, k: Sec, n: usize, section: Rect) -> Rect {
        match k {
            Sec::Head => {
                let plate = self.head_elem(n).and_then(|elem| self.header_row(cx.measure).plate(elem)).unwrap_or_default();
                // the section's rect is where it is drawn: its place in the document, scrolled with the page
                Rect::new(plate.x, plate.y + section.y, plate.w, plate.h)
            }
            Sec::Guide => {
                let cell = self.board().and_then(|board| Some(board.grid.rect(board.grid.nth(n)?))).unwrap_or_default();
                Rect::new(cell.x, cell.y + section.y - time_grid::RULER_Y, cell.w, cell.h)
            }
            _ => section,
        }
    }
    /// The scope row seats the scope on view; the Guide's board seats the airing on now of the first row showing.
    fn seat_override(&self, cx: &Cx<'_, H>, k: Sec, _from: plx_ui::screen::Placed) -> Option<u32> {
        match k {
            // a filter that shows nothing leaves the filter control the one stop worth seating
            Sec::Head if matches!(self.mode, Mode::BrowseNone(_)) => Some(FILTER_ELEM),
            Sec::Head => Some(self.scope.elem()),
            Sec::Guide => {
                let board = self.board()?;
                board.grid.elem(board.grid.entry(H::instant(cx)?)?)
            }
            _ => None,
        }
    }
    fn plain_group(&self, cx: &Cx<'_, H>, k: Sec, id: GroupId, extent: Rect) -> GroupSpec {
        match k {
            // the row's own group: a Control row, Left and Right stopping at its ends; the section's top places it
            Sec::Head => match self.header_row(cx.measure).focus(self.entry, id, (0.0, 0.0), Rect::FULL).group_spec() {
                Some(spec) => GroupSpec { extent: Rect { y: spec.extent.y + extent.y, ..spec.extent }, ..spec },
                None => GroupSpec { id, kind: GroupKind::Free, seat: Seat::First, reachable: AxisMask::VERTICAL, edge: [EdgeRule::Stop; 4], extent, len: 0, elem: ElemKind::Control },
            },
            // the board: its own cells move focus (`plain_step`); Up from the first row is the header's link, Up and Down
            // otherwise stop at the board's ends, and Left and Right past a row's ends are the screen's: they move the
            // window ([`LiveTvScreen::shift`]). Down from the header seats the cell the last Up left from, else the
            // airing on now.
            Sec::Guide => GroupSpec {
                id, kind: GroupKind::Free, seat: plx_ui::page_regions::CONTENT_HEAD_SEAT, reachable: AxisMask::VERTICAL,
                edge: [EdgeRule::Stop, EdgeRule::Stop, EdgeRule::Screen, EdgeRule::Screen], extent,
                len: StackPage::<H>::plain_len(self, k), elem: ElemKind::Control,
            },
            _ => GroupSpec { id, kind: GroupKind::Free, seat: Seat::First, reachable: AxisMask::VERTICAL,
                edge: [EdgeRule::Geometric, EdgeRule::Geometric, EdgeRule::Stop, EdgeRule::Stop], extent, len: 1, elem: ElemKind::Bare },
        }
    }
    /// While the lineup loads the content's head has no group yet: the page declares it empty (the shared rule),
    /// which an arrival names, so a page that has nothing to seat seats nothing.
    fn extra_groups(&self, _cx: &Cx<'_, H>, out: &mut Vec<GroupSpec>) { self.regions().declare_pending(out); }
    fn draw_heading(&self, k: Sec, f: &mut DrawFrame<'_, '_, H>, r: Rect, lift: f32) {
        let title = match k { Sec::Recent => plx_os::i18n::msg::browse_livetv_recent_c(), Sec::Now => plx_os::i18n::msg::browse_livetv_on_now_c(), Sec::Channels => plx_os::i18n::msg::browse_livetv_channels_c(), _ => return };
        plx_ui::cards::draw_heading_c(f.painter.alpha(f.page_alpha).translate(0.0, r.y), title, c"", c"", MARGIN_X, -lift, 1.0, f.measure);
    }
    fn custom_draw(&self, k: Sec, f: &mut DrawFrame<'_, '_, H>, r: Rect, _scroll: f32) {
        if k == Sec::Guide {
            let (Some(board), Some(now)) = (self.board(), H::instant(f.cx)) else { return };
            let channels = H::livetv(f.cx).channels;
            let focus = f.focus.current.filter(|key| key.entry == self.entry).and_then(|key| board.grid.locate(key.elem));
            let indicator = self.indicator();
            let words = Words { indicator: &indicator, unscheduled: plx_os::i18n::msg::browse_livetv_no_guide() };
            // the board is laid out on the frame, its ruler at the section's top
            let p = f.painter.alpha(f.page_alpha).translate(0.0, r.y - time_grid::RULER_Y);
            // without a zone offset the board is laid out and has no clock words
            let offset = H::wall(f.cx).map(|wall| wall.utc_offset_s);
            board.grid.draw(p, f.measure, now, offset, words, &Faces { channels, board }, focus);
        } else if k == Sec::Head {
            // The row rides the page and leaves with the chrome and returns with it (`Handoff::head_alpha`), on Browse as on every
            // page. The control a menu grew out of is the lift's to draw ([`LiveTvScreen::redraw_opener`]), not the row's.
            let p = f.painter.alpha(f.page_alpha).alpha(self.handoff.head_alpha(r.y));
            let lifted = f.lifted.filter(|key| key.entry == self.entry).map(|key| key.elem);
            self.header_row(f.measure).draw_with(p, -r.y, &self.header_capsules, self.header_pop.scale_with(0, f.press), lifted, &self.header_swap.borrow());
        } else if k == Sec::Status && self.mode.reads_out() {
            self.status_overlay(f.cx.tick.ms).focused(f.focus.current.is_some_and(|k| k.entry == self.entry && k.elem == RETRY_ELEM))
                .draw_measured(&Env::inert(), f.painter.alpha(f.page_alpha), f.measure);
        }
    }
}

pub struct LiveTvScreen {
    page: Page,
    stack: Stack<Sec>,
    /// The page's own ground: the page's backdrop and the colour the tucked chrome's top fade lays over it.
    ground: PageGround,
    ground_seeded: bool,
    /// The page's own arrival seat while nobody has chosen another (the Watchlist's rule): `None`
    /// once the reader acts or the page is restored.
    seat: Option<Option<GroupId>>,
    /// Seats this page asked for whose `Enter` has not come back yet.
    own_enters: u8,
    /// How many of those were queued before the reader acted.
    stale_enters: u8,
    /// **The channel the player left**, told by the player's exit while this page is covered (`AppMsg::LiveTvSeat`),
    /// seated when the page is restored and then forgotten: server, DVR and key.
    left: Option<(plx_plex::plex::ServerId, String, String)>,
    /// How far ahead the store was last told to read the guide ([`LiveTvCmd::Reach`]); `0` when it holds none. What the
    /// page asks for follows from the board ([`Page::reach`]), so this is only what the store has been sent.
    reached: i64,
}

/// **Where the focus stood in the Guide before the board under it changed**: its row and its time (`None`: it stood on an
/// unscheduled row, which says nothing about where the reader was), so it can land on the same programme in a rebuilt or
/// shifted board ([`TimeGrid::land`]).
#[derive(Clone, Copy)]
struct Held {
    row: usize,
    span: Option<Span>,
}

impl LiveTvScreen {
    /// The heartbeat word, byte-identical to the route word.
    pub const WORD: &'static str = "livetv";

    pub const SHAPE: &'static str = "LiveTvScreen{entry:u32,status:u32,scope:u32,filter:u32,channels:[{sid:u32,rk:str}],airings:[u32],recent:[u32],guide:Option<(page:u32,window:u64,ahead:bool)>,stack:Stack{scroll:{pos:f32,vel:f32},target:f32,sections:[Shelf{row,pop},Shelf{row,pop},Shelf{row,pop},Grid{bands,pop}]},handoff:Handoff{tucked:bool,chrome:f32,capsule:f32},left:Option<(u32,str,str)>}";

    pub fn new(entry: EntryId) -> Self {
        Self {
            page: Page {
                entry,
                scope: Scope::WhatsOn,
                epoch: 0,
                channels: Vec::new(),
                airings: Vec::new(),
                recent: Vec::new(),
                rest: Vec::new(),
                mode: Mode::Loading,
                content: Vec::new(),
                synced: None,
                clock: 0,
                header_capsules: plx_ui::widgets::TabStrip::new(),
                header_pop: plx_ui::widgets::CtlPop::new(),
                handoff: plx_ui::scroll_chrome::Handoff::new(),
                header_rows: SharedRow::new(),
                header_swap: std::cell::RefCell::new(plx_ui::page_header::HeaderSwap::new()),
                idents: Idents::default(),
            },
            stack: Stack::new(entry).home_when_unfocused(true),
            ground: PageGround::new(),
            ground_seeded: false,
            seat: None,
            own_enters: 0,
            stale_enters: 0,
            left: None,
            reached: 0,
        }
    }

    /// Every shelf names the channel that supplies its card (Recent's tiles and the Channels shelf's on the even
    /// elements, On now's poster on the odd ones); a Guide cell names its row's channel, and plays it only where it
    /// tunes ([`Board::tunes`]).
    fn press<H: LiveTvLike>(&self, elem: u32, cx: &Cx<'_, H>) -> Option<ContentReq> {
        // a Browse card tunes the programme on air and nothing else: a later programme has nothing to tune
        if let Some(set) = self.page.mode.browse_set() {
            let pick = set.picks.get(*set.index.get(&elem)?)?;
            let airing = matches!(browse::stand(pick.begins_at, pick.ends_at, H::instant(cx)?), browse::Stand::OnAir { .. });
            return H::livetv(cx).channels.get(pick.channel).filter(|_| airing).map(Self::play);
        }
        if let Some((board, cell)) = self.page.board().and_then(|board| Some((board, board.grid.locate(elem)?))) {
            let tunes = board.tunes(cell, H::instant(cx)?);
            return H::livetv(cx).channels.get(cell.row).filter(|_| tunes).map(Self::play);
        }
        let index = elem.checked_sub(FIRST_CARD_ELEM)? as usize / 2;
        let view = H::livetv(cx);
        let channel = view.channels.get(index)?;
        if elem % 2 == 1 && channel.now.is_empty() { return None; }
        Some(Self::play(channel))
    }

    /// Play a channel: the one request a card's OK and number entry's pick both make.
    fn play(channel: &Channel) -> ContentReq {
        ContentReq::Play { play: PlayIntent::Live {
            sid: channel.sid, dvr: channel.dvr.clone(), channel: channel.channel.clone(), title: channel.name.clone(),
        }, resume_ns: 0 }
    }

    /// **A press on an element the page owns beyond its cards**: a stop of the scope row chooses that page (the
    /// row stays under focus, the page under it is rebuilt), a Guide cell tunes its channel where it tunes. `false`
    /// for any other element.
    fn commit<H: LiveTvLike>(&mut self, elem: u32, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) -> bool {
        if elem == FILTER_ELEM && self.page.browsing() {
            if let Some(filter) = self.page.scope.filter() {
                if let Some(pill) = self.page.filter_pill(cx.measure) {
                    // the control as it is drawn now; the menu is handed where it is drawn on every frame after
                    // (`Screen::opener_rect`), so the page moving under it moves the glass with it
                    let anchor = [pill.x, pill.y - self.stack.scroll(), pill.w, pill.h].map(f32::to_bits);
                    let panel = ContentPanel::LiveTvFilter { chosen: filter, anchor, opener: (self.page.entry, FILTER_ELEM) };
                    fx.push(Fx::App(AppFx::Content(ContentReq::Panel(panel))));
                }
                fx.invalidate(Provenance::Input);
            }
            return true;
        }
        if let Some(scope) = Scope::of_elem(elem) {
            // a press on the Browse stop while Browse shows keeps the filter in force
            if !self.page.scope.same_page(scope) {
                self.page.scope = scope;
                // a page chosen opens on the live window (the board it leaves, if any, is dropped with its window)
                self.resync(cx);
                self.reach(fx);
                fx.invalidate(Provenance::Input);
            }
            return true;
        }
        if !self.page.board().is_some_and(|board| board.grid.locate(elem).is_some()) {
            return false;
        }
        if let Some(out) = self.press(elem, cx) {
            fx.push(Fx::App(AppFx::Content(out)));
            fx.invalidate(Provenance::Input);
        }
        true
    }

    /// Follow the store, the clock and the zone: [`Page::sync`] with what the frame holds.
    fn resync<H: LiveTvLike>(&mut self, cx: &Cx<'_, H>) -> bool {
        self.page.sync(H::livetv(cx), H::instant(cx), H::wall(cx).map(|w| w.utc_offset_s))
    }

    /// **The filter menu chose** ([`AppMsg::LiveTvFilterPicked`]): the one place Browse's filter changes. The set is rebuilt at once
    /// for it, the focus stays on the control (the menu hands it back there), and the store is told what the new filter reads
    /// ahead to ([`Self::reach`]). A pick while Browse is not showing (the page was left under the menu) is spent.
    fn pick_filter<H: LiveTvLike>(&mut self, filter: Filter, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) {
        let Scope::Browse(current) = self.page.scope else { return };
        if current == filter { return; }
        self.page.scope = Scope::Browse(filter);
        self.resync(cx);
        self.reach(fx);
        fx.invalidate(Provenance::Input);
    }

    /// **CH+ and CH- on the Guide**: the page `step` pages over (+1 the next six channels), the focus on the cell of the
    /// same place there. The board does not wrap, and a press with focus off the board does nothing here.
    fn turn<H: LiveTvLike>(&mut self, step: isize, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) {
        let (Some(board), Some(now)) = (self.page.board(), H::instant(cx)) else { return };
        let Some(at) = cx.focus.current.filter(|key| key.entry == self.page.entry).and_then(|key| board.grid.locate(key.elem)) else { return };
        let Some(elem) = board.grid.turn(at, step, now).and_then(|to| board.grid.elem(to)) else { return };
        // the page is showing before focus lands on it
        self.page.follow(elem);
        self.seat_own(FocusTarget::Elem(FocusKey { entry: self.page.entry, elem }), fx);
        fx.invalidate(Provenance::Input);
    }

    /// **The Guide's window moves a step** (LEFT or RIGHT past a row's end: the group's `EdgeRule::Screen`): later when the
    /// guide goes on past the window, earlier while the window is held ahead of the live one; at either limit nothing
    /// moves, and the press is spent. The focus lands on the cell of its row that shares the most time with the one it left.
    fn shift<H: LiveTvLike>(&mut self, later: bool, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) {
        let held = self.held(cx);
        if self.page.shift(H::livetv(cx), H::instant(cx), later) {
            self.reseat(held, cx, fx);
            self.reach(fx);
            fx.invalidate(Provenance::Input);
        }
    }

    /// The Guide cell the focus stands on, as the row and the time it is ([`Held`]); `None` off the board.
    fn held<H: LiveTvLike>(&self, cx: &Cx<'_, H>) -> Option<Held> {
        let board = self.page.board()?;
        let at = cx.focus.current.filter(|key| key.entry == self.page.entry).and_then(|key| board.grid.locate(key.elem))?;
        Some(Held { row: at.row, span: board.grid.span(at).filter(|_| !board.grid.is_unscheduled(at.row)) })
    }

    /// **Keep the focus on its programme across a rebuilt or shifted board**: the board's cells are numbered by their place
    /// in the row, so after the programmes changed the element the focus holds can be another programme, or none. Seat
    /// it on the cell of its row that shares the most time with the one it left ([`TimeGrid::land`]); where the focus
    /// already stands on it, nothing moves.
    fn reseat<H: LiveTvLike>(&mut self, held: Option<Held>, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) {
        let (Some(held), Some(board)) = (held, self.page.board()) else { return };
        let Some(to) = board.grid.land(held.row, held.span, board.now).and_then(|cell| board.grid.elem(cell)) else { return };
        if cx.focus.current.is_some_and(|key| key.entry == self.page.entry && key.elem == to) { return; }
        self.page.follow(to);
        self.seat_own(FocusTarget::Elem(FocusKey { entry: self.page.entry, elem: to }), fx);
        fx.invalidate(Provenance::Input);
    }

    /// **Tell the store how far ahead the Guide reads** when what the board needs is not what it was told last
    /// ([`LiveTvCmd::Reach`]; the store acts on a growth and nothing else).
    fn reach<H: LiveTvLike>(&mut self, fx: &mut Effects<'_, H>) {
        let want = self.page.reach();
        if want != self.reached {
            self.reached = want;
            fx.push(Fx::App(AppFx::Store(StoreId::LiveTv, StoreCmd::LiveTv(LiveTvCmd::Reach(want)))));
        }
    }

    /// The page is leaving (covered by the player, or gone): the store need read no further than the live window for it. A
    /// page restored asks again ([`reach`](Self::reach)).
    fn release<H: LiveTvLike>(&mut self, fx: &mut Effects<'_, H>) {
        if self.reached != 0 {
            self.reached = 0;
            fx.push(Fx::App(AppFx::Store(StoreId::LiveTv, StoreCmd::LiveTv(LiveTvCmd::Reach(0)))));
        }
    }

    fn tick<H: LiveTvLike>(&mut self, t: Tick, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) {
        self.page.header_row(cx.measure).step(self.page.entry, cx.focus.current, &mut self.page.header_capsules, &mut self.page.header_pop, t.dt());
        self.page.header_swap.get_mut().step(t.dt());
        self.page.handoff.step(t.dt(), self.stack.scroll(), self.stack.target(), self.page.first_row_pitch(), false);
        // The page's own haze, fixed: it does not follow the focused tile.
        let haze = AmbientWash::target(theme::WASH_LIVE_TV, [theme::WASH_LIVE_TV_W; 4]);
        if self.ground_seeded {
            self.ground.key_target(haze, t.dt());
        } else {
            self.ground.jump_target(haze);
            self.ground_seeded = true;
        }
        // A programme's words and bar move with the minute, with no event to say so.
        let stamp = self.page.clock_stamp(H::livetv(cx), H::instant(cx), H::wall(cx));
        if stamp != self.page.clock {
            self.page.clock = stamp;
            fx.invalidate(Provenance::Input);
        }
    }

    fn route<H: LiveTvLike>(&self, out: HomeReq, fx: &mut Effects<'_, H>) {
        fx.push(Fx::App(AppFx::Home(out)));
        fx.invalidate(Provenance::Input);
    }

    /// Seat the arrival in the first shelf once it has a channel; until then seat nothing (the
    /// Watchlist's `seat_first`).
    fn seat_first<H: LiveTvLike>(&mut self, fx: &mut Effects<'_, H>) {
        let regions = self.page.regions();
        let want = regions.first_seat();
        let target = want.map(FocusTarget::FirstInGroupAnimated).or_else(|| regions.unseat());
        self.seat = (want != Some(RECENT_GROUP)).then_some(want);
        if let Some(target) = target {
            self.seat_own(target, fx);
        }
    }

    fn follow_seat<H: LiveTvLike>(&mut self, fx: &mut Effects<'_, H>) {
        let Some(seated) = self.seat else { return };
        if seated != self.page.regions().first_seat() {
            self.seat_first(fx);
        }
    }

    /// **The card of the channel the player left**: on Browse its programme's card; else its Recent tile (a tuned channel joins Recent at admission), else the
    /// row it belongs to, On now's poster or the Channels shelf's tile; on the Guide its row's cell. `None` for a
    /// channel the lineup no longer lists.
    fn left_elem<H: LiveTvLike>(&mut self, left: &(plx_plex::plex::ServerId, String, String), cx: &Cx<'_, H>) -> Option<u32> {
        self.resync(cx);
        let index = H::livetv(cx).channels.iter().position(|c| c.sid == left.0 && c.dvr == left.1 && c.channel == left.2)?;
        // On the Guide the channel's own row: the cell of what was airing (the one the reader pressed), else its
        // first. That row's page is the one showing when focus lands.
        if let Some(board) = self.page.board() {
            let row = board.cells.get(index)?;
            let at = Cell { row: index, index: row.iter().position(|src| *src == CellSrc::Now).unwrap_or(0) };
            let elem = board.grid.elem(at)?;
            self.page.follow(elem);
            return Some(elem);
        }
        // On Browse the channel's programme: the one on air, else where it would stand ([`Browse::seat`])
        if let Some(set) = self.page.mode.browse_set() {
            return set.seat(index, H::instant(cx)?);
        }
        let base = FIRST_CARD_ELEM + index as u32 * 2;
        if self.page.recent.contains(&index) || self.page.rest.contains(&index) {
            Some(base)
        } else {
            self.page.airings.contains(&index).then_some(base + 1)
        }
    }

    fn seat_own<H: LiveTvLike>(&mut self, focus: FocusTarget<u32>, fx: &mut Effects<'_, H>) {
        self.own_enters = self.own_enters.saturating_add(1);
        fx.push(Fx::Deliver(fx.from(), Delivery::Screen(ScreenEvent::Enter(Enter::Fresh { focus }))));
    }

    /// BACK, decided by [`PageRegions::back`]: a scrolled page first returns to its top, on the
    /// first tile; from there it leaves for Home.
    fn back<H: LiveTvLike>(&mut self, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) -> Handled {
        // a scrolled window goes back to the live one first, on the programme the focus holds
        let held = self.held(cx);
        if self.page.go_live(H::livetv(cx), H::instant(cx)) {
            self.reseat(held, cx, fx);
            self.reach(fx);
            fx.invalidate(Provenance::Input);
            return Handled::Yes;
        }
        let focus = cx.focus.current.filter(|key| key.entry == self.page.entry);
        let regions = self.page.regions();
        let head = regions.first_seat().unwrap_or(RECENT_GROUP);
        let at = focus.and_then(|key| regions.region_of(key.elem, Focusable::<H>::group_of(self, &key.elem, cx)));
        match regions.back(at, self.stack.target() > 0.5) {
            BackStep::Top => {
                self.seat_own(FocusTarget::FirstInGroupAnimated(head), fx);
            }
            BackStep::Content | BackStep::Header => self.seat_own(FocusTarget::ContainerGroup(head), fx),
            BackStep::Nav => self.seat_own(FocusTarget::ContainerGroup(plx_ui::containers::tabs::STRIP), fx),
            BackStep::Leave => self.route(HomeReq::Tab(HomeTab::Home), fx),
        }
        Handled::Yes
    }

    /// A non-card element committed: the strip's pills and the chip.
    fn activate<H: LiveTvLike>(&mut self, elem: u32, fx: &mut Effects<'_, H>) -> Handled {
        match Self::strip_press(elem) {
            Some(Some(out)) => self.route(out, fx),
            Some(None) => {}
            None => return Handled::No,
        }
        Handled::Yes
    }

    /// A press on the shared strip: the other tabs and the chip are requests; the Live TV pill is
    /// the page the reader is already on.
    fn strip_press(elem: u32) -> Option<Option<HomeReq>> {
        use plx_ui::widgets::{
            TAB_KEY_CHIP, TAB_KEY_HOME, TAB_KEY_LIVETV, TAB_KEY_MOVIES, TAB_KEY_SEARCH, TAB_KEY_SHOWS, TAB_KEY_WATCHLIST,
        };
        match elem {
            TAB_KEY_HOME => Some(Some(HomeReq::Tab(HomeTab::Home))),
            TAB_KEY_MOVIES => Some(Some(HomeReq::Tab(HomeTab::Movies))),
            TAB_KEY_SHOWS => Some(Some(HomeReq::Tab(HomeTab::Shows))),
            TAB_KEY_SEARCH => Some(Some(HomeReq::Tab(HomeTab::Search))),
            TAB_KEY_WATCHLIST => Some(Some(HomeReq::Tab(HomeTab::Watchlist))),
            TAB_KEY_CHIP => Some(Some(HomeReq::Account)),
            TAB_KEY_LIVETV => Some(None),
            _ => None,
        }
    }

    fn view<H: LiveTvLike>(&self) -> impl Focusable<H> + '_ {
        self.stack.view(&self.page)
    }

    /// **The filter control, lit above its menu's scrim** (the opener lift): painted as the focused control at its place on the row (which
    /// rides the page), at the morph's alpha and the head's own handoff alpha (the Watchlist's), since the row's pass leaves it out
    /// while the menu is up.
    fn redraw_opener<H: LiveTvLike>(&self, f: &mut DrawFrame<'_, '_, H>) {
        let p = f.painter.alpha(f.page_alpha).alpha(self.page.handoff.head_alpha(-self.stack.scroll()));
        self.page.header_row(f.measure).draw_opener_with(p, self.stack.scroll(), self.page.header_pop.scale_with(0, f.press), FILTER_ELEM, &self.page.header_swap.borrow());
    }

    /// The page's top edge once the chrome has tucked away (the Watchlist's).
    fn draw_chrome<H: LiveTvLike>(&self, f: &mut DrawFrame<'_, '_, H>) {
        let tuck = self.page.handoff.tuck();
        if tuck <= 0.0 {
            return;
        }
        let ground = self.ground.sample(Rect::FULL, SCR_W * 0.5, 0.0);
        plx_ui::scroll_capsule::draw_top_fade(f.painter.alpha(f.page_alpha), SCR_W, ground, tuck, None);
    }
}

plx_ui::focusable_via_view!(LiveTvScreen, H: [LiveTvLike], view);

impl LogicalState for LiveTvScreen {
    fn write(&self, c: &mut Canon) {
        c.u32(self.page.entry.0);
        // the store's status as the mode reads it: any row is `Ready` (1); a `Ready` store with no channel is the empty read-out (2)
        c.u32(match self.page.mode { Mode::Loading => 0, Mode::Empty => 2, Mode::Failed => 3, Mode::BrowseNone(_) => 4, _ => 1 });
        let (page, filter) = self.page.scope.code();
        c.u32(page).u32(filter);
        c.seq(self.page.channels.len());
        for (sid, rk) in &self.page.channels { c.u32(u32::from(sid.raw())).str(rk); }
        c.seq(self.page.airings.len());
        for &i in &self.page.airings { c.u32(i as u32); }
        c.seq(self.page.recent.len());
        for &i in &self.page.recent { c.u32(i as u32); }
        // the board's page, the half hour its window starts at and whether the reader holds it there
        c.option(self.page.board(), |c, board| {
            c.u32(board.grid.page() as u32).u64(board.grid.axis().start() as u64).bool(matches!(board.window, Window::Ahead(_)));
        });
        self.stack.write(c);
        self.page.handoff.write(c);
        c.option(self.left.as_ref(), |c, (sid, dvr, channel)| { c.u32(u32::from(sid.raw())).str(dvr).str(channel); });
    }

    fn probe(&self, out: &mut String) {
        out.push_str(&format!("livetv channels={}", self.page.channels.len()));
    }
}

impl<H: LiveTvLike> Machine<H> for LiveTvScreen {
    type Ev = ScreenEvent<H>;

    fn step(&mut self, ev: &Self::Ev, cx: &Cx<'_, H>, fx: &mut Effects<'_, H>) -> Handled {
        let held = self.held(cx);
        let rebuilt = match ev {
            ScreenEvent::Tick(_) | ScreenEvent::Enter(Enter::Fresh { .. }) => self.resync(cx),
            ScreenEvent::StoreChanged(ord, _) if *ord == StoreId::LiveTv.ord() => {
                self.page.synced = None;
                self.resync(cx)
            }
            // the board's page follows the focus, before the stack lays out for it
            ScreenEvent::FocusMoved { to, .. } if to.entry == self.page.entry => {
                self.page.follow(to.elem);
                false
            }
            _ => false,
        };
        // a board rebuilt under the focus (a read landed, a half hour rolled over): the focus stays on its programme
        if rebuilt {
            self.reseat(held, cx, fx);
        }
        match ev {
            ScreenEvent::WillLeave(_) | ScreenEvent::Unmount => self.release(fx),
            _ => self.reach(fx),
        }
        let card = match self.stack.on(&self.page, ev, cx, fx) {
            Some(StackEvent::Card(_, card)) => Some(card),
            _ => None,
        };
        // The reader acting makes the seat theirs (claimed on the event, as the Watchlist does).
        if let ScreenEvent::Input(InputEvent { kind, .. }) = ev {
            if matches!(kind, InputKind::Key { edge: Edge::Down, .. } | InputKind::Pointer { .. }
                | InputKind::Click { .. } | InputKind::Drag { .. } | InputKind::Wheel { .. }) {
                self.seat = None;
                self.stale_enters = self.own_enters;
            }
        }
        match ev {
            ScreenEvent::Tick(t) => {
                self.tick(*t, cx, fx);
                self.follow_seat(fx);
                Handled::Yes
            }
            ScreenEvent::Enter(enter) => {
                match enter {
                    Enter::Fresh { .. } if self.own_enters > 0 => {
                        self.own_enters -= 1;
                        if self.stale_enters > 0 {
                            self.stale_enters -= 1;
                            if let Some(held) = cx.focus.current.filter(|key| key.entry == self.page.entry) {
                                self.own_enters = self.own_enters.saturating_add(1);
                                fx.push(Fx::Deliver(fx.from(), Delivery::Screen(ScreenEvent::Enter(Enter::Fresh {
                                    focus: FocusTarget::Elem(held),
                                }))));
                            }
                        }
                    }
                    Enter::Fresh { .. } => {
                        self.seat_first(fx);
                    }
                    Enter::Restored => {
                        self.seat = None;
                        self.stale_enters = self.own_enters;
                        // back from the player the Guide is the live window again (the channel's own cell is the one airing)
                        self.page.go_live(H::livetv(cx), H::instant(cx));
                        self.reach(fx);
                        // the player was left on a channel: the card of that channel, not the one that was pressed
                        if let Some(elem) = self.left.take().and_then(|left| self.left_elem(&left, cx)) {
                            self.seat_own(FocusTarget::Elem(FocusKey { entry: self.page.entry, elem }), fx);
                        }
                    }
                }
                fx.invalidate(Provenance::Nav);
                Handled::Yes
            }
            ScreenEvent::App(AppMsg::LiveTvFilterPicked(filter)) => {
                self.pick_filter(*filter, cx, fx);
                Handled::Yes
            }
            ScreenEvent::App(AppMsg::LiveTvSeat { sid, dvr, channel }) => {
                self.left = Some((*sid, dvr.clone(), channel.clone()));
                Handled::Yes
            }
            // number entry chose a channel: played through the page's own play path, like a card's OK
            ScreenEvent::App(AppMsg::LiveChannelPicked { sid, dvr, channel }) => {
                let lineup = H::livetv(cx).channels;
                if let Some(c) = lineup.iter().find(|c| c.sid == *sid && c.dvr == *dvr && c.channel == *channel) {
                    fx.push(Fx::App(AppFx::Content(Self::play(c))));
                    fx.invalidate(Provenance::Input);
                }
                Handled::Yes
            }
            // CH+ and CH- turn the Guide's page by six; CH+ is the next channels (the live player's own reading)
            ScreenEvent::Input(InputEvent { kind: InputKind::Key { sym, wcode, edge: Edge::Down, at_edge: false, .. }, .. })
                if self.page.board().is_some() && page_dir(*sym, *wcode).is_some() =>
            {
                let step = page_dir(*sym, *wcode).map_or(0, |dir| -dir as isize);
                self.turn(step, cx, fx);
                Handled::Yes
            }
            // LEFT or RIGHT past a row's end (the board's `EdgeRule::Screen`) moves the window
            ScreenEvent::Input(InputEvent { kind: InputKind::Key { key: key @ (Key::Left | Key::Right), edge: Edge::Down | Edge::Repeat, at_edge: true, .. }, .. })
                if self.page.board().is_some() =>
            {
                self.shift(*key == Key::Right, cx, fx);
                Handled::Yes
            }
            // a digit opens number entry over the page (PR5); a page with no lineup has no number to type
            ScreenEvent::Input(InputEvent { kind: InputKind::Key { sym, wcode, edge: Edge::Down, at_edge: false, .. }, .. })
                if pin_digit(*sym, *wcode).is_some() && !H::livetv(cx).channels.is_empty() =>
            {
                let first = pin_digit(*sym, *wcode).map_or(0, |d| d - b'0');
                fx.push(Fx::App(AppFx::Content(ContentReq::Panel(ContentPanel::ChannelEntry { first }))));
                fx.invalidate(Provenance::Input);
                Handled::Yes
            }
            ScreenEvent::Uncover => {
                fx.invalidate(Provenance::Nav);
                Handled::Yes
            }
            ScreenEvent::FocusMoved { .. } | ScreenEvent::StoreChanged(..) => {
                fx.invalidate(Provenance::Input);
                Handled::Yes
            }
            ScreenEvent::Activate(elem) => {
                if *elem == RETRY_ELEM && self.page.mode == Mode::Failed {
                    fx.push(Fx::App(AppFx::Store(StoreId::LiveTv, StoreCmd::LiveTv(LiveTvCmd::Retry))));
                    fx.invalidate(Provenance::Input);
                    Handled::Yes
                } else if self.commit(*elem, cx, fx) { Handled::Yes } else { self.activate(*elem, fx) }
            },
            ScreenEvent::PressCommit(_) => {
                if let Some(CardEvent::Activate(elem)) = card {
                    if let Some(out) = self.press(elem, cx) {
                        fx.push(Fx::App(AppFx::Content(out)));
                        fx.invalidate(Provenance::Input);
                    }
                } else if let Some(key) = cx.focus.current.filter(|key| key.entry == self.page.entry) {
                    if key.elem == RETRY_ELEM && self.page.mode == Mode::Failed {
                        fx.push(Fx::App(AppFx::Store(StoreId::LiveTv, StoreCmd::LiveTv(LiveTvCmd::Retry))));
                        fx.invalidate(Provenance::Input);
                    } else if !self.commit(key.elem, cx, fx) { self.activate(key.elem, fx); }
                }
                Handled::Yes
            }
            ScreenEvent::Input(InputEvent { kind: InputKind::Key { key: Key::Back, edge: Edge::Down, .. }, .. }) => {
                self.back(cx, fx)
            }
            _ => Handled::No,
        }
    }
}

impl<H: LiveTvLike> Screen<H> for LiveTvScreen {
    fn focused_card_rect(&self, cx: &Cx<'_, H>, focus: Option<FocusKey<u32>>, at: plx_ui::screen::At) -> Option<Rect> {
        let key = focus.filter(|k| k.entry == self.page.entry && ((FIRST_CARD_ELEM..GUIDE_FIRST_ELEM).contains(&k.elem) || (BROWSE_FIRST_ELEM..plx_ui::geom::NAMESPACE_FLOOR).contains(&k.elem)))?;
        Focusable::<H>::place(self, &key.elem, cx, at).map(|placed| placed.rect)
    }
    fn redraw_focused(&self, f: &mut DrawFrame<'_, '_, H>, focus: Option<FocusKey<u32>>) {
        if self.page.browsing() && focus.is_some_and(|key| key.entry == self.page.entry && key.elem == FILTER_ELEM) {
            return self.redraw_opener(f);
        }
        self.stack.view(&self.page).redraw_focused(f, focus);
    }
    fn opener_departing(&self, key: FocusKey<u32>) -> bool {
        self.stack.view(&self.page).opener_departing::<H>(key)
    }
    fn name(&self) -> &'static str {
        Self::WORD
    }
    fn a11y_ns(&self) -> &'static str {
        Self::WORD
    }
    fn chrome_tuck(&self) -> f32 {
        self.page.handoff.tuck()
    }
    fn state(&self) -> &dyn LogicalState {
        self
    }
    fn crumb(&self, _cx: &Cx<'_, H>) -> Option<std::borrow::Cow<'_, str>> {
        None
    }
    fn prepare(&mut self, _budget: &mut plx_ui::frame::Budget, _cx: &Cx<'_, H>) {}
    fn draw(&mut self, f: &mut DrawFrame<'_, '_, H>) {
        plx_gfx::gfx::frame_clear(theme::CLEAR_RGB.0, theme::CLEAR_RGB.1, theme::CLEAR_RGB.2);
        let p = f.painter.alpha(f.page_alpha);
        self.ground.draw(p, Rect::FULL);
        self.stack.view(&self.page).paint(f);
        self.draw_chrome(f);
    }
    fn render(&self) -> RenderStrategy {
        RenderStrategy::Page
    }
    fn focus_source(&self) -> FocusSource {
        FocusSource::Engine
    }
    fn hit_source(&self) -> HitSource {
        HitSource::Engine
    }
    fn links(&self, out: &mut Vec<Link>) {
        self.page.regions().links(out);
    }
    fn holds_arrival(&self, dir: plx_ui::screen::Dir) -> bool {
        self.page.regions().holds(dir)
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
#[path = "livetv_tests.rs"]
mod tests;
