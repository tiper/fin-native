//! **TimeGrid** — every row of a list against one clock: the Live TV Guide's board (the handoff's `SPEC-3`),
//! drawn at 1:1 on the 1920 x 1080 frame. A row is a channel cell that stands still at the left and the
//! programmes of that channel laid along a time ruler; the window is [`Axis`], a start, a length and a ruler step in
//! Unix seconds (the Guide's is 90 minutes from a half hour, which moves a half hour at a time), and a programme
//! stands from where it begins (or where the window does, for one that began before it) to where it ends, less
//! [`CELL_GAP`]. The now line, its shade and its chip are drawn only while now is in the window, and the chip hides
//! the ruler label it would cover.
//!
//! **Why this is a widget of its own.** Nothing else here lays things out along a TIME axis: a card shelf
//! ([`cards`](crate::cards)) and the poster grid are lattices of equal cells, and an [`HStack`](crate::hstack)
//! packs measured items with one gap. A programme's width and left edge are its begin and end, so the cells of
//! two rows do not line up, and focus travel between rows is by how much of the time two cells share
//! ([`TimeGrid::step`]) rather than by column or by x. Anything that is a list of rows each with a timeline
//! (a recordings schedule, a "coming up" list) wants this one geometry, so it lands here and not in a screen.
//!
//! **What it is made of.** A pure model — [`Axis`] (the clock-to-pixel rule), [`TimeGrid`] (the rows' cells, the
//! page of [`PAGE_ROWS`] rows that is showing, the focus travel, the paging) — and one paint
//! ([`TimeGrid::draw`]) that reads theme tokens only. It owns no focus: a screen that hosts it as a
//! [`Kind::Custom`](crate::cards::Kind::Custom) section of a [`Stack`](crate::cards::Stack) answers the stack's
//! `plain_len`, `plain_elem`, `plain_step` and `element_rect` from [`TimeGrid::len`], [`TimeGrid::nth`],
//! [`TimeGrid::step`] and [`TimeGrid::rect`], which is the stack's own mechanism for "a list, a button row", so
//! the scroll, the header's focus group and the page's links stay the stack's and `PageRegions`'. Every cell of
//! every row is an element (the cells of a row that is not on the page are parked off screen, so they can be
//! neither pointed at nor drawn), which makes UP and DOWN between pages ordinary moves; the page follows the
//! focus ([`TimeGrid::follow`]).
//!
//! **An element is its row and its place in the row** ([`ROW_STRIDE`] apart), not its place in the whole board,
//! so a refresh that gives one channel another programme moves no other channel's elements, and the focus that
//! stands on a row stays on that row.
//!
//! **A row with no programme in the window** (a channel the guide has nothing for) is one cell across the whole
//! window, so it can be focused and pressed like any other ([`TimeGrid::is_unscheduled`]).
use std::ops::Range;

use plx_machine::machine::Measure;

use crate::fmt;
use crate::label::HAlign;
use crate::screen::Dir;
use crate::text_view::TextView;
use crate::{card_shadow_params, theme, Painter, Rect};

/// Channels in a page of the board (`SPEC-3`: "six channels a page").
const PAGE_ROWS: usize = 6;
/// The channel cell's left edge, width and radius.
const CHANNEL_X: f32 = 96.0;
const CHANNEL_W: f32 = 320.0;
const CHANNEL_RADIUS: f32 = 14.0;
/// A row's height, the pitch from one row to the next, and the first row's top.
const CELL_H: f32 = 100.0;
const ROW_PITCH: f32 = 108.0;
const FIRST_ROW_Y: f32 = 292.0;
/// A programme cell's radius, and the air its right edge keeps from the next.
const CELL_RADIUS: f32 = 10.0;
const CELL_GAP: f32 = 8.0;
/// The time ruler's line (the page indicator stands on it too): top and height.
pub const RULER_Y: f32 = 236.0;
const RULER_H: f32 = 40.0;
/// The time axis' left and right edges on the frame: 1392 px for the window.
const AXIS_X0: f32 = 432.0;
const AXIS_X1: f32 = 1824.0;
/// A ruler tick (2 x 32) and the air between it and its label.
const TICK_W: f32 = 2.0;
const TICK_H: f32 = 32.0;
const TICK_LABEL_GAP: f32 = 12.0;
/// The now chip: 88 x 36, fully rounded, on the ruler's line; wider only when its words need more.
const CHIP_W: f32 = 88.0;
const CHIP_H: f32 = 36.0;
const CHIP_PAD: f32 = 12.0;
/// The now line's width.
const NOW_W: f32 = 2.0;
/// A programme's padding: vertical 18, horizontal 20 (`SPEC-3` item 6).
const PAD_Y: f32 = 18.0;
const PAD_X: f32 = 20.0;
/// The channel number's left inset in the channel cell, and where the logo's box starts and how big it is.
const NUMBER_X: f32 = 20.0;
const LOGO_X: f32 = 128.0;
const LOGO_W: f32 = 175.0;
const LOGO_H: f32 = 98.0;
/// The logo box and its content are the Recent tile's, at 70%.
pub const LOGO_SCALE: f32 = 0.7;
/// A cell narrower than this draws no text: a title that is all ellipsis says nothing.
const MIN_TEXT_W: f32 = 64.0;
/// How far apart the elements of two rows are: more than the seconds in any window the board has, so no row runs
/// into the next whatever the guide sends.
const ROW_STRIDE: u32 = 8_192;
/// Where a cell that is not on the page is parked: above the frame, so it is neither drawn nor hit.
const PARKED_Y: f32 = -10_000.0;

/// A stretch of time, in Unix seconds: a programme's airing, or the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub begins: i64,
    pub ends: i64,
}

impl Span {
    pub const fn new(begins: i64, ends: i64) -> Self {
        Self { begins, ends }
    }

    /// Whether `t` is within the span (its start included, its end not).
    pub const fn contains(self, t: i64) -> bool {
        self.begins <= t && t < self.ends
    }

    /// How much time `self` and `other` share, in seconds.
    fn overlap(self, other: Span) -> i64 {
        (self.ends.min(other.ends) - self.begins.max(other.begins)).max(0)
    }

    /// The empty time between them, in seconds: 0 when they touch or overlap.
    fn gap(self, other: Span) -> i64 {
        (self.begins.max(other.begins) - self.ends.min(other.ends)).max(0)
    }
}

/// **The clock-to-pixel rule**: a window of `seconds` from `start`, laid across [`AXIS_X0`]..[`AXIS_X1`], its ruler
/// ticking every `step` seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Axis {
    start: i64,
    seconds: i64,
    step: i64,
}

impl Axis {
    /// The window `[start, start + seconds)` with a tick every `step`: the screen's one step (the Guide's
    /// `WINDOW_STEP_SECONDS`, which is also what the window moves by). A window of no length is one second long
    /// (there is no 0 to divide by), and so is a step.
    pub const fn new(start: i64, seconds: i64, step: i64) -> Self {
        Self { start, seconds: if seconds < 1 { 1 } else { seconds }, step: if step < 1 { 1 } else { step } }
    }

    pub const fn start(self) -> i64 {
        self.start
    }

    pub const fn end(self) -> i64 {
        self.start + self.seconds
    }

    /// The window as a span.
    pub const fn window(self) -> Span {
        Span::new(self.start, self.end())
    }

    /// Where time `t` stands, clamped to the axis (a time before the window is its left edge).
    pub fn x_of(self, t: i64) -> f32 {
        let along = (t - self.start).clamp(0, self.seconds) as f64 / self.seconds as f64;
        AXIS_X0 + (along * f64::from(AXIS_X1 - AXIS_X0)) as f32
    }

    /// **A programme's cell on the axis**, `(x, width)`: from the later of its begin and the window's start to its end
    /// (clipped to the window's end), less [`CELL_GAP`]. `None` for a programme that lies wholly outside the window
    /// or leaves no room for the gap.
    pub fn cell(self, span: Span) -> Option<(f32, f32)> {
        if span.ends <= self.start || span.begins >= self.end() || span.ends <= span.begins {
            return None;
        }
        let (x, right) = (self.x_of(span.begins), self.x_of(span.ends));
        let w = right - x - CELL_GAP;
        (w > 0.0).then_some((x, w))
    }

    /// The ruler's ticks: every step from the window's start, while they are inside it.
    pub fn ticks(self) -> impl Iterator<Item = i64> {
        let (start, end, step) = (self.start, self.end(), self.step);
        (0..).map(move |n| start + n * step).take_while(move |t| *t < end)
    }
}

/// One cell of the board: the row (an index into the lineup) and the cell's place in it, in time order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub row: usize,
    pub index: usize,
}

/// **What a screen supplies to the paint**: the face of a row's sticky cell and the words of a programme.
pub trait Content {
    /// The channel cell's face (its number and logo, see [`number_at`] and [`logo_rect`]) in `at`, on the plate
    /// [`TimeGrid::draw`] has drawn; `focused` while the row has focus.
    fn channel(&self, p: Painter, row: usize, at: Rect, focused: bool);
    /// The title of programme `index` of `row`.
    fn title(&self, row: usize, index: usize) -> &str;
}

/// The words the board draws itself, from the screen.
#[derive(Clone, Copy)]
pub struct Words<'a> {
    /// The page indicator ("Channels 1–6 of 16").
    pub indicator: &'a str,
    /// What the one cell of a row with nothing in the window says.
    pub unscheduled: &'a str,
}

/// **The board**: the rows' programmes, the page showing and the focus travel. See the module doc.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimeGrid {
    axis: Axis,
    /// Per row, its programmes in time order; empty is an unscheduled row.
    rows: Vec<Vec<Span>>,
    /// How many cells come before each row, and the total last (`rows.len() + 1` entries): the board's cells in
    /// row then time order, which is the order [`nth`](Self::nth) walks.
    starts: Vec<usize>,
    /// The element of row 0, cell 0: the screen's namespace for the board.
    base: u32,
    /// Which page of [`PAGE_ROWS`] rows shows.
    page: usize,
}

impl TimeGrid {
    /// A board of `rows` over `axis` on its first page. `base` is the element of the first cell; row `r`'s cells
    /// are `base + r * ROW_STRIDE + n`, so the screen keeps the range from `base` for the board.
    pub fn new(axis: Axis, rows: Vec<Vec<Span>>, base: u32) -> Self {
        let mut starts = Vec::with_capacity(rows.len() + 1);
        let mut total = 0;
        for row in &rows {
            starts.push(total);
            total += row.len().max(1);
        }
        starts.push(total);
        Self { axis, rows, starts, base, page: 0 }
    }

    /// This board on page `page`, or on its last page when it has fewer: a board rebuilt from a refresh keeps
    /// the page the reader was on.
    pub fn at_page(mut self, page: usize) -> Self {
        self.page = page.min(self.pages().saturating_sub(1));
        self
    }

    pub fn axis(&self) -> Axis {
        self.axis
    }

    /// The row an element is in, whether or not that row still has a cell at the element's place (a refresh that
    /// gave the row fewer programmes): where a focus that lost its cell goes to stay on its channel.
    pub fn row_of(&self, elem: u32) -> Option<usize> {
        let row = (elem.checked_sub(self.base)? / ROW_STRIDE) as usize;
        (row < self.rows.len()).then_some(row)
    }

    /// How many rows the board has.
    pub fn rows(&self) -> usize {
        self.rows.len()
    }

    /// How many cells the board has, in all.
    pub fn len(&self) -> usize {
        self.starts.last().copied().unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The number of pages.
    pub fn pages(&self) -> usize {
        self.rows.len().div_ceil(PAGE_ROWS)
    }

    /// The page showing, from 0.
    pub fn page(&self) -> usize {
        self.page
    }

    /// The rows on the page showing.
    pub fn shown(&self) -> Range<usize> {
        let first = (self.page * PAGE_ROWS).min(self.rows.len());
        first..(first + PAGE_ROWS).min(self.rows.len())
    }

    /// **The page indicator's numbers**, 1-based: the first and last row showing and the total.
    pub fn indicator(&self) -> (usize, usize, usize) {
        let shown = self.shown();
        (shown.start + 1, shown.end, self.rows.len())
    }

    /// How many cells `row` has: its programmes, or the one cell of an unscheduled row.
    pub fn cells(&self, row: usize) -> usize {
        self.rows.get(row).map_or(0, |spans| spans.len().max(1))
    }

    /// Does `row` have nothing in the window (so its one cell spans the whole window)?
    pub fn is_unscheduled(&self, row: usize) -> bool {
        self.rows.get(row).is_some_and(Vec::is_empty)
    }

    /// A cell's time: the programme's, or the whole window for an unscheduled row.
    pub fn span(&self, at: Cell) -> Option<Span> {
        let spans = self.rows.get(at.row)?;
        if spans.is_empty() { (at.index == 0).then(|| self.axis.window()) } else { spans.get(at.index).copied() }
    }

    /// A cell's element.
    pub fn elem(&self, at: Cell) -> Option<u32> {
        let row = u32::try_from(at.row).ok()?;
        (at.index < self.cells(at.row)).then(|| self.base + row * ROW_STRIDE + at.index as u32)
    }

    /// The cell an element is, `None` for one that is not the board's.
    pub fn locate(&self, elem: u32) -> Option<Cell> {
        let offset = elem.checked_sub(self.base)?;
        let at = Cell { row: (offset / ROW_STRIDE) as usize, index: (offset % ROW_STRIDE) as usize };
        (at.index < self.cells(at.row)).then_some(at)
    }

    /// The `n`th cell of the board, in row then time order (`len()` of them): the order a stack walks a section's
    /// elements in.
    pub fn nth(&self, n: usize) -> Option<Cell> {
        if n >= self.len() {
            return None;
        }
        let row = self.starts.partition_point(|start| *start <= n) - 1;
        Some(Cell { row, index: n - self.starts[row] })
    }

    /// The position of `at` in the board's run (the inverse of [`nth`](Self::nth)).
    pub fn position(&self, at: Cell) -> Option<usize> {
        (at.index < self.cells(at.row)).then(|| self.starts[at.row] + at.index)
    }

    /// **The page follows the focus**: show the page `row` is on. `true` when that turned the page.
    pub fn follow(&mut self, row: usize) -> bool {
        let page = row / PAGE_ROWS;
        let moved = page != self.page && row < self.rows.len();
        if moved {
            self.page = page;
        }
        moved
    }

    /// Where a cell stands on the frame while its row is on the page showing; `None` while it is not.
    /// The top of `row` on the frame, while it is on the page showing.
    fn row_top(&self, row: usize) -> Option<f32> {
        let shown = self.shown();
        shown.contains(&row).then(|| FIRST_ROW_Y + (row - shown.start) as f32 * ROW_PITCH)
    }

    fn placed(&self, at: Cell) -> Option<Rect> {
        let y = self.row_top(at.row)?;
        let (x, w) = self.span(at).and_then(|span| self.axis.cell(span)).unwrap_or((AXIS_X0, 0.0));
        Some(Rect::new(x, y, w, CELL_H))
    }

    /// A cell's rect on the frame, or one parked above it when its row is not on the page showing (so it can be
    /// neither drawn nor pointed at, and focus can still stand on it for the frame the page takes to follow).
    pub fn rect(&self, at: Cell) -> Rect {
        self.placed(at).unwrap_or(Rect::new(AXIS_X0, PARKED_Y, 0.0, CELL_H))
    }

    /// The row's channel cell, while it is on the page.
    pub fn channel_rect(&self, row: usize) -> Option<Rect> {
        Some(Rect::new(CHANNEL_X, self.row_top(row)?, CHANNEL_W, CELL_H))
    }

    /// **What time a move leaves from**: the cell's own, or, for the one cell of an unscheduled row (which spans the
    /// whole window and says nothing about where the reader was), the instant `now` (held inside the window: before it,
    /// the window's first second), so the move lands on what is airing then and not on the longest cell.
    fn leaving(&self, at: Cell, now: i64) -> Option<Span> {
        if self.is_unscheduled(at.row) { Some(self.instant(now)) } else { self.span(at) }
    }

    /// The instant `now` as a span of one second, held inside the window (before it, the window's first second).
    fn instant(&self, now: i64) -> Span {
        let window = self.axis.window();
        let at = now.clamp(window.begins, window.ends - 1);
        Span::new(at, at + 1)
    }

    /// **Where a focus lands when the board under it is rebuilt or its window moves**: the cell of `row` that shares the
    /// most time with the cell the focus stood on (`from`, its span; the same programme where it is still there), or, for
    /// a focus that stood on an unscheduled row (`None`, which says nothing about where the reader was), the cell on `now`.
    pub fn land(&self, row: usize, from: Option<Span>, now: i64) -> Option<Cell> {
        self.nearest(row, from.unwrap_or_else(|| self.instant(now)))
    }

    /// The cell of `row` that shares the most time with `from` (clipped to the window), the earliest on a tie; when
    /// `from` shares none with any of them (a gap in the guide), the nearest in time.
    fn nearest(&self, row: usize, from: Span) -> Option<Cell> {
        let window = self.axis.window();
        let from = Span::new(from.begins.max(window.begins), from.ends.min(window.ends));
        (0..self.cells(row))
            .filter_map(|index| {
                let span = self.span(Cell { row, index })?;
                Some((std::cmp::Reverse(from.overlap(span)), from.gap(span), index))
            })
            .min()
            .map(|(_, _, index)| Cell { row, index })
    }

    /// **A move**: LEFT and RIGHT to the cell before or after in the row (`None` at its ends); UP and DOWN to the
    /// cell of the row above or below that shares the most time with this one (from an unscheduled row, the one on
    /// `now`). `None` at the first and last row.
    pub fn step(&self, at: Cell, dir: Dir, now: i64) -> Option<Cell> {
        match dir {
            Dir::Left => at.index.checked_sub(1).map(|index| Cell { row: at.row, index }),
            Dir::Right => (at.index + 1 < self.cells(at.row)).then_some(Cell { row: at.row, index: at.index + 1 }),
            Dir::Up | Dir::Down => {
                let row = if dir == Dir::Up { at.row.checked_sub(1)? } else { at.row + 1 };
                if row >= self.rows.len() {
                    return None;
                }
                self.nearest(row, self.leaving(at, now)?)
            }
        }
    }

    /// **Turn the page by `pages`** (CH+ and CH-): the cell of the same place on the page `pages` over, by the same
    /// overlap rule as UP and DOWN. The slot stands at the last row of a last page that is short. `None` with no such
    /// page (the board does not wrap).
    pub fn turn(&self, at: Cell, pages: isize, now: i64) -> Option<Cell> {
        let page = at.row / PAGE_ROWS;
        let want = page.checked_add_signed(pages)?;
        if want >= self.pages() || want == page {
            return None;
        }
        let row = (want * PAGE_ROWS + at.row % PAGE_ROWS).min(self.rows.len() - 1);
        self.nearest(row, self.leaving(at, now)?)
    }

    /// **The cell a board is entered on** from outside it (DOWN from the header): the first row of the page showing, at
    /// the programme on `now`, else its first cell.
    pub fn entry(&self, now: i64) -> Option<Cell> {
        let row = self.shown().start;
        if row >= self.rows.len() {
            return None;
        }
        let on_now = (0..self.cells(row)).find(|&index| {
            self.span(Cell { row, index }).is_some_and(|span| span.contains(now))
        });
        Some(Cell { row, index: on_now.unwrap_or(0) })
    }

    /// **Paint the board** at the frame's own coordinates (translate `p` for a page that scrolls): the indicator and the
    /// ruler, the page's rows (the channel cell, then the programmes, the focused one lit), the shade over the time
    /// already aired, and the now line and chip, the last two while `now` is in the window. `offset` is the zone's
    /// UTC offset: without it (the platform could not say) there are no clock words, no ruler labels, programme
    /// times or chip, and the board is otherwise the same. `focus` is the focused cell, if it is the board's.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&self, p: Painter, measure: &dyn Measure, now: i64, offset: Option<i32>, words: Words<'_>, content: &dyn Content, focus: Option<Cell>) {
        let offset = offset.map(i64::from);
        let in_window = self.axis.window().contains(now);
        let chip = in_window.then(|| self.chip(measure, offset, now)).flatten();
        self.draw_ruler(p, measure, offset, words.indicator, chip.as_ref().map(|(rect, _)| *rect));
        let shown = self.shown();
        for row in shown.clone() {
            let Some(at) = self.channel_rect(row) else { continue };
            let focused_row = focus.is_some_and(|cell| cell.row == row);
            p.rrect(at, CHANNEL_RADIUS, CHANNEL_RADIUS, if focused_row { theme::TAB_PLATE_SELECTED } else { theme::SURFACE_PANEL });
            content.channel(p, row, at, focused_row);
            for index in 0..self.cells(row) {
                let cell = Cell { row, index };
                let Some(rect) = self.placed(cell).filter(|r| r.w > 0.0) else { continue };
                self.draw_cell(p, measure, offset, words, content, cell, rect, focus == Some(cell));
            }
        }
        if shown.is_empty() {
            return;
        }
        let rows_bottom = FIRST_ROW_Y + (shown.len() - 1) as f32 * ROW_PITCH + CELL_H;
        let now_x = self.axis.x_of(now);
        // time already aired: shaded over every row, from the axis' left edge to the now line (none while the window is
        // still ahead of now)
        if now_x > AXIS_X0 {
            p.rrect(Rect::new(AXIS_X0, FIRST_ROW_Y, now_x - AXIS_X0, rows_bottom - FIRST_ROW_Y), 0.0, 0.0, theme::GUIDE_AIRED_SHADE);
        }
        if !in_window {
            return;
        }
        p.rrect(Rect::new(now_x - NOW_W * 0.5, RULER_Y + RULER_H * 0.5, NOW_W, rows_bottom - RULER_Y - RULER_H * 0.5), 0.0, 0.0, theme::GUIDE_NOW);
        if let Some((rect, label)) = chip {
            self.draw_chip(p, measure, rect, &label);
        }
    }

    /// The ruler's line: the indicator, then a tick and its clock label at every step of the window. A label the now chip
    /// (`chip`) would cover is not drawn (its tick stays): the chip's own words say the time there.
    fn draw_ruler(&self, p: Painter, measure: &dyn Measure, offset: Option<i64>, indicator: &str, chip: Option<Rect>) {
        let caption = theme::size::CAPTION;
        let top = RULER_Y + (RULER_H - measure.cap_h(caption)) * 0.5;
        TextView::new(indicator, caption, theme::TEXT_TERTIARY).with_measure(measure).max_lines(1)
            .draw(p, Rect::new(CHANNEL_X, top, AXIS_X0 - CHANNEL_X - 16.0, 0.0));
        for t in self.axis.ticks() {
            let x = self.axis.x_of(t);
            p.rrect(Rect::new(x, RULER_Y + (RULER_H - TICK_H) * 0.5, TICK_W, TICK_H), 0.0, 0.0, theme::TAB_PLATE_SELECTED);
            let Some(label) = offset.and_then(|offset| fmt::clock_face(t, offset)) else { continue };
            let left = x + TICK_W + TICK_LABEL_GAP;
            if chip.is_some_and(|chip| left < chip.x + chip.w && left + measure.width_str(&label, caption, false) > chip.x) {
                continue;
            }
            TextView::new(&label, caption, theme::TEXT_SECONDARY).with_measure(measure).max_lines(1)
                .draw(p, Rect::new(left, top, 200.0, 0.0));
        }
    }

    /// The now chip's place and words, centred on `now`; `None` without a zone offset (no words to put in it).
    fn chip(&self, measure: &dyn Measure, offset: Option<i64>, now: i64) -> Option<(Rect, String)> {
        let label = fmt::clock_face(now, offset?)?;
        let w = CHIP_W.max(measure.width_str(&label, theme::size::MICRO, true) + 2.0 * CHIP_PAD);
        Some((Rect::new(self.axis.x_of(now) - w * 0.5, RULER_Y + (RULER_H - CHIP_H) * 0.5, w, CHIP_H), label))
    }

    fn draw_chip(&self, p: Painter, measure: &dyn Measure, chip: Rect, label: &str) {
        let micro = theme::size::MICRO;
        p.rrect(chip, CHIP_H * 0.5, CHIP_H * 0.5, theme::GUIDE_NOW);
        TextView::new(label, micro, theme::GUIDE_CELL_FOCUS_INK).bold().h(HAlign::Center).with_measure(measure)
            .draw(p, Rect::new(chip.x, chip.cy() - measure.cap_h(micro) * 0.5, chip.w, 0.0));
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_cell(&self, p: Painter, measure: &dyn Measure, offset: Option<i64>, words: Words<'_>, content: &dyn Content,
        cell: Cell, rect: Rect, focused: bool) {
        if focused {
            let (blur, off, a, _) = card_shadow_params(rect.h, 1.0, theme::CARD_SHADOW[3]);
            p.shadow(rect, CELL_RADIUS, blur, off, theme::with_a(theme::CARD_SHADOW, a));
        }
        p.rrect(rect, CELL_RADIUS, CELL_RADIUS, if focused { theme::GUIDE_CELL_FOCUS } else { theme::TAB_PLATE_IDLE });
        if rect.w < MIN_TEXT_W + 2.0 * PAD_X {
            return;
        }
        let (label, micro) = (theme::size::LABEL, theme::size::MICRO);
        let at = |top: f32| Rect::new(rect.x + PAD_X, top, rect.w - 2.0 * PAD_X, 0.0);
        if self.is_unscheduled(cell.row) {
            let ink = if focused { theme::GUIDE_CELL_FOCUS_INK_QUIET } else { theme::TEXT_TERTIARY };
            TextView::new(words.unscheduled, label, ink).with_measure(measure).max_lines(1)
                .draw(p, at(rect.cy() - measure.cap_h(label) * 0.5));
            return;
        }
        let (title_ink, time_ink) = if focused {
            (theme::GUIDE_CELL_FOCUS_INK, theme::GUIDE_CELL_FOCUS_INK_QUIET)
        } else {
            (theme::TEXT_PRIMARY, theme::TEXT_SECONDARY)
        };
        TextView::new(content.title(cell.row, cell.index), label, title_ink).bold().with_measure(measure).max_lines(1)
            .draw(p, at(rect.y + PAD_Y + 3.0));
        if let Some(range) = offset.and_then(|offset| self.span(cell).and_then(|s| fmt::airing_range(s.begins, s.ends, offset))) {
            let top = rect.y + rect.h - PAD_Y - 3.0 - measure.cap_h(micro);
            TextView::new(&range, micro, time_ink).with_measure(measure).max_lines(1).draw(p, at(top));
        }
    }
}

/// **A channel logo's box in the channel cell**: the Recent tile's 250 x 140 plate at [`LOGO_SCALE`] (175 x 98), at
/// [`LOGO_X`] of the cell. What stands in it (the logo, or the name) is laid out as the tile lays it out on its plate
/// and scaled by [`LOGO_SCALE`] about the box's middle.
pub fn logo_rect(cell: Rect) -> Rect {
    Rect::new(cell.x + LOGO_X, cell.y + (cell.h - LOGO_H) * 0.5, LOGO_W, LOGO_H)
}

/// Where the channel number stands in the channel cell: its left edge and the line it is vertically centred on.
pub fn number_at(cell: Rect) -> (f32, f32) {
    (cell.x + NUMBER_X, cell.cy())
}

#[cfg(test)]
#[path = "time_grid_tests.rs"]
mod tests;
