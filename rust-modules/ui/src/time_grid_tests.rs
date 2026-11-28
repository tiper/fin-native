use super::*;
use std::ffi::CStr;

struct Fixed;
impl Measure for Fixed {
    fn width(&self, s: &CStr, _: i32, bold: bool) -> f32 { s.to_bytes().len() as f32 * if bold { 10.0 } else { 8.0 } }
    fn cap_h(&self, _: i32) -> f32 { 20.0 }
    fn line_h(&self, _: i32) -> f32 { 36.0 }
}

/// A window on a half hour: 90 minutes from 20:30 UTC, so a minute is `1392 / 90` px.
const START: i64 = 1_792_524_600 - 1_792_524_600 % 1_800;
const AXIS: Axis = Axis::new(START, 5_400, 1_800);
const MIN: i64 = 60;
/// Ten minutes into the window: what the moves of a board with nothing unscheduled in the way do not care about.
const NOW: i64 = START + 10 * MIN;

fn span(from_min: i64, to_min: i64) -> Span {
    Span::new(START + from_min * MIN, START + to_min * MIN)
}

/// A clock face as the text run records it: the formatter's narrow no-break space is drawn as a plain one.
fn drawn(face: String) -> String {
    face.replace('\u{202f}', " ")
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

#[test]
fn the_axis_runs_432_to_1824_over_90_minutes() {
    assert!(near(AXIS.x_of(START), 432.0));
    assert!(near(AXIS.x_of(START + 5_400), 1_824.0));
    // 15.47 px a minute (SPEC-3 item 3)
    assert!(near(AXIS.x_of(START + 60) - AXIS.x_of(START), 1_392.0 / 90.0));
    // before and after the window clamp to its edges
    assert!(near(AXIS.x_of(START - 3_600), 432.0));
    assert!(near(AXIS.x_of(START + 9_000), 1_824.0));
}

#[test]
fn a_cell_runs_from_its_start_to_its_end_less_the_gap() {
    let (x, w) = AXIS.cell(span(30, 60)).unwrap();
    assert!(near(x, 432.0 + 30.0 * 1_392.0 / 90.0));
    assert!(near(w, 30.0 * 1_392.0 / 90.0 - CELL_GAP));
}

#[test]
fn a_cell_that_began_before_the_window_starts_at_its_left_edge() {
    // began 40 minutes before the window, ends 20 minutes in: from x = 432, 20 minutes wide, less the gap
    let (x, w) = AXIS.cell(span(-40, 20)).unwrap();
    assert!(near(x, 432.0));
    assert!(near(w, 20.0 * 1_392.0 / 90.0 - CELL_GAP));
}

#[test]
fn a_cell_that_ends_after_the_window_stops_at_its_right_edge() {
    let (x, w) = AXIS.cell(span(60, 200)).unwrap();
    assert!(near(x + w + CELL_GAP, 1_824.0));
}

#[test]
fn a_programme_wholly_outside_the_window_has_no_cell() {
    assert!(AXIS.cell(span(-60, -30)).is_none());
    assert!(AXIS.cell(span(-60, 0)).is_none(), "ending as the window opens");
    assert!(AXIS.cell(span(90, 120)).is_none());
}

#[test]
fn the_ruler_ticks_every_half_hour_and_stops_inside_the_window() {
    let ticks: Vec<i64> = AXIS.ticks().collect();
    assert_eq!(ticks, vec![START, START + 1_800, START + 3_600]);
}

fn board() -> TimeGrid {
    // 8 rows: row 0 has two programmes split at minute 45; row 1 one programme from before the window;
    // row 2 nothing (a channel the guide has nothing for); the rest a programme each, split at minute 30.
    let mut rows = vec![vec![span(0, 45), span(45, 90)], vec![span(-30, 90)], vec![]];
    rows.extend((3..8).map(|_| vec![span(0, 30), span(30, 90)]));
    TimeGrid::new(AXIS, rows, 100)
}

fn cell(row: usize, index: usize) -> Cell {
    Cell { row, index }
}

#[test]
fn an_element_is_its_row_and_its_place_in_it() {
    let g = board();
    // 2 + 1 + 1 + 5 * 2 cells in all
    assert_eq!(g.len(), 14);
    assert_eq!(g.elem(cell(0, 0)), Some(100));
    assert_eq!(g.elem(cell(0, 1)), Some(101));
    assert_eq!(g.elem(cell(1, 0)), Some(100 + ROW_STRIDE));
    assert_eq!(g.elem(cell(2, 0)), Some(100 + 2 * ROW_STRIDE));
    assert_eq!(g.elem(cell(3, 1)), Some(100 + 3 * ROW_STRIDE + 1));
    assert_eq!(g.elem(cell(3, 2)), None, "a row has no cell it does not hold");
    assert_eq!(g.elem(cell(8, 0)), None);
    for n in 0..g.len() {
        let at = g.nth(n).unwrap();
        assert_eq!(g.position(at), Some(n));
        assert_eq!(g.locate(g.elem(at).unwrap()), Some(at));
    }
    assert_eq!(g.nth(14), None);
    assert_eq!(g.locate(99), None);
    assert_eq!(g.locate(102), None, "row 0 has two cells");
    assert_eq!(g.locate(100 + 8 * ROW_STRIDE), None);
}

#[test]
fn a_row_gaining_a_programme_moves_no_other_rows_elements() {
    let before = board();
    let mut rows = vec![vec![span(0, 30), span(30, 60), span(60, 90)], vec![span(-30, 90)], vec![]];
    rows.extend((3..8).map(|_| vec![span(0, 30), span(30, 90)]));
    let after = TimeGrid::new(AXIS, rows, 100);
    for row in 1..8 {
        for index in 0..before.cells(row) {
            assert_eq!(before.elem(cell(row, index)), after.elem(cell(row, index)), "row {row} cell {index}");
        }
    }
}

#[test]
fn a_row_with_nothing_is_one_cell_across_the_whole_window() {
    let g = board();
    assert!(g.is_unscheduled(2));
    assert!(!g.is_unscheduled(1));
    assert_eq!(g.span(cell(2, 0)), Some(AXIS.window()));
    let r = g.rect(cell(2, 0));
    assert!(near(r.x, 432.0));
    assert!(near(r.x + r.w + CELL_GAP, 1_824.0));
}

#[test]
fn rows_stand_at_292_plus_108_a_row_six_to_a_page() {
    let g = board();
    for slot in 0..PAGE_ROWS {
        assert!(near(g.rect(cell(slot, 0)).y, 292.0 + 108.0 * slot as f32), "row {slot}");
        assert!(near(g.rect(cell(slot, 0)).h, 100.0));
        assert!(near(g.channel_rect(slot).unwrap().y, 292.0 + 108.0 * slot as f32));
    }
    let channel = g.channel_rect(0).unwrap();
    assert!(near(channel.x, 96.0) && near(channel.w, 320.0) && near(channel.h, 100.0));
    // rows of the other page are parked off the frame, and have no channel cell
    assert!(g.rect(cell(6, 0)).y < 0.0);
    assert!(g.channel_rect(6).is_none());
}

#[test]
fn the_page_indicator_counts_the_rows_showing() {
    let mut g = board();
    assert_eq!(g.indicator(), (1, 6, 8));
    assert_eq!(g.pages(), 2);
    assert!(g.follow(6));
    assert_eq!(g.indicator(), (7, 8, 8));
    assert!(near(g.rect(cell(6, 0)).y, 292.0), "the second page's first row is at the top");
    assert!(g.rect(cell(0, 0)).y < 0.0, "and the first page's are parked");
}

#[test]
fn the_page_follows_the_focus_and_only_moves_when_the_row_is_on_another_page() {
    let mut g = board();
    assert!(!g.follow(5));
    assert_eq!(g.page(), 0);
    assert!(g.follow(6));
    assert_eq!(g.page(), 1);
    assert!(!g.follow(7));
    assert!(g.follow(0));
    assert_eq!(g.page(), 0);
    assert!(!g.follow(99), "a row the board does not have turns nothing");
}

#[test]
fn left_and_right_walk_a_row_and_stop_at_its_ends() {
    let g = board();
    assert_eq!(g.step(cell(0, 0), Dir::Right, NOW), Some(cell(0, 1)));
    assert_eq!(g.step(cell(0, 1), Dir::Left, NOW), Some(cell(0, 0)));
    assert_eq!(g.step(cell(0, 0), Dir::Left, NOW), None);
    assert_eq!(g.step(cell(0, 1), Dir::Right, NOW), None);
    assert_eq!(g.step(cell(1, 0), Dir::Right, NOW), None, "one programme: no next");
}

#[test]
fn up_and_down_land_on_the_cell_sharing_the_most_time() {
    let g = board();
    // row 0: [0,45) [45,90). Row 3: [0,30) [30,90).
    // from row 0's first cell (0-45): overlap with row 3's first is 30, with its second 15
    assert_eq!(g.step(cell(0, 0), Dir::Down, NOW), Some(cell(1, 0)), "row 1 has one programme");
    assert_eq!(g.step(cell(2, 0), Dir::Down, NOW), Some(cell(3, 0)), "out of the unscheduled row: what is on now, not the longest cell");
    assert_eq!(g.step(cell(3, 0), Dir::Up, NOW), Some(cell(2, 0)), "into the unscheduled row");
    // from row 3's first cell [0,30): row 4's overlaps are 30 and 0, so its first
    assert_eq!(g.step(cell(3, 0), Dir::Down, NOW), Some(cell(4, 0)));
    // from row 3's second cell [30,90): row 4's overlaps are 0 and 60, so its second
    assert_eq!(g.step(cell(3, 1), Dir::Down, NOW), Some(cell(4, 1)));
    // from row 0's second cell [45,90) into row 1: the only one
    assert_eq!(g.step(cell(0, 1), Dir::Down, NOW), Some(cell(1, 0)));
    // from row 1's whole-window cell down to row 2 (nothing): its one
    assert_eq!(g.step(cell(1, 0), Dir::Down, NOW), Some(cell(2, 0)));
}

#[test]
fn up_and_down_use_the_overlap_inside_the_window_for_a_cell_that_began_before_it() {
    // row 1's programme began 30 minutes before the window: only its 90 minutes inside count, so against
    // [0,45) / [45,90) of row 0 the first (45 shared) wins on a tie by the earliest. A window ahead (`now` before it):
    // nothing in it has aired, so the whole window's overlap is the rule
    let g = board();
    assert_eq!(g.step(cell(1, 0), Dir::Up, START - MIN), Some(cell(0, 0)));
    // with `now` ten minutes in, the part from now on ([10,90): 35 and 45 shared) decides: the later cell
    assert_eq!(g.step(cell(1, 0), Dir::Up, NOW), Some(cell(0, 1)));
}

#[test]
fn up_from_the_first_row_and_down_from_the_last_have_nowhere_to_go() {
    let g = board();
    assert_eq!(g.step(cell(0, 0), Dir::Up, NOW), None);
    assert_eq!(g.step(cell(7, 0), Dir::Down, NOW), None);
}

#[test]
fn a_gap_in_the_guide_goes_to_the_nearest_cell_in_time() {
    // row 0 has a hole at [30,60); row 1 has [0,30) and [60,90)
    let g = TimeGrid::new(AXIS, vec![vec![span(0, 30), span(60, 90)], vec![span(40, 50)]], 0);
    // [40,50) shares nothing with either, and is nearer [60,90) (10 minutes) than [0,30) (10 minutes): the
    // earlier wins the tie
    assert_eq!(g.step(cell(1, 0), Dir::Up, NOW), Some(cell(0, 0)));
    let g = TimeGrid::new(AXIS, vec![vec![span(0, 30), span(60, 90)], vec![span(45, 55)]], 0);
    assert_eq!(g.step(cell(1, 0), Dir::Up, NOW), Some(cell(0, 1)), "5 minutes from the second, 15 from the first");
}

#[test]
fn the_page_turns_by_six_keeping_the_slot() {
    let g = board();
    // 3 + 6 = 9 is past row 7: the slot stands at the last row of the short page
    assert_eq!(g.turn(cell(3, 1), 1, NOW).map(|c| c.row), Some(7));
    // row 0 + 6 = 6
    assert_eq!(g.turn(cell(0, 0), 1, NOW).map(|c| c.row), Some(6));
    // and back, to the same slot
    assert_eq!(g.turn(cell(6, 0), -1, NOW).map(|c| c.row), Some(0));
    assert_eq!(g.turn(cell(7, 0), -1, NOW).map(|c| c.row), Some(1));
}

#[test]
fn the_page_does_not_wrap() {
    let g = board();
    assert_eq!(g.turn(cell(0, 0), -1, NOW), None, "no page before the first");
    assert_eq!(g.turn(cell(6, 0), 1, NOW), None, "no page after the last");
}

#[test]
fn a_page_turn_keeps_the_time_it_was_on() {
    // [30,90) of row 3 turns to the cell of row 9's shared time: the second of two
    let rows: Vec<Vec<Span>> = (0..12).map(|_| vec![span(0, 30), span(30, 90)]).collect();
    let g = TimeGrid::new(AXIS, rows, 0);
    assert_eq!(g.turn(cell(3, 1), 1, NOW), Some(cell(9, 1)));
    assert_eq!(g.turn(cell(3, 0), 1, NOW), Some(cell(9, 0)));
}

#[test]
fn entry_is_the_airing_on_now_of_the_first_row_showing() {
    let mut g = board();
    // now = minute 50: row 0's second cell
    assert_eq!(g.entry(START + 50 * MIN), Some(cell(0, 1)));
    // now = minute 10: its first
    assert_eq!(g.entry(START + 10 * MIN), Some(cell(0, 0)));
    // on page 2, the first row of that page
    g.follow(6);
    assert_eq!(g.entry(START + 50 * MIN), Some(cell(6, 1)));
}

#[test]
fn entry_with_nothing_on_now_is_the_rows_first_cell() {
    let g = TimeGrid::new(AXIS, vec![vec![span(30, 60), span(60, 90)]], 0);
    assert_eq!(g.entry(START + 10 * MIN), Some(cell(0, 0)));
    assert_eq!(TimeGrid::new(AXIS, vec![], 0).entry(START), None);
}

#[test]
fn an_empty_board_has_no_elements_and_no_pages() {
    let g = TimeGrid::new(AXIS, vec![], 7);
    assert!(g.is_empty());
    assert_eq!((g.len(), g.pages(), g.shown()), (0, 0, 0..0));
    assert_eq!(g.nth(0), None);
}

struct Faces;
impl Content for Faces {
    fn channel(&self, _: Painter, _: usize, _: Rect, _: bool) {}
    fn title(&self, row: usize, index: usize) -> &str {
        match (row, index) {
            (0, 0) => "News",
            (0, 1) => "Weather",
            _ => "Film",
        }
    }
}

const WORDS: Words<'static> = Words { indicator: "Channels 1\u{2013}6 of 8", unscheduled: "No guide data" };

fn paint(g: &TimeGrid, now: i64, focus: Option<Cell>) -> (Vec<crate::draw_census::Run>, Vec<(u64, Rect, f32)>) {
    paint_in(g, now, Some(0), focus)
}

/// [`paint`] in a zone: `None` is a frame whose zone could not be read.
fn paint_in(g: &TimeGrid, now: i64, offset: Option<i32>, focus: Option<Cell>) -> (Vec<crate::draw_census::Run>, Vec<(u64, Rect, f32)>) {
    let runs = crate::draw_census::capture_runs(|| g.draw(Painter::recording(), &Fixed, now, offset, WORDS, &Faces, focus));
    let inks = crate::draw_census::capture_inks(|| g.draw(Painter::recording(), &Fixed, now, offset, WORDS, &Faces, focus));
    (runs, inks)
}

#[test]
fn the_now_line_stands_at_the_wall_time_on_the_axis() {
    let g = board();
    let now = START + 30 * MIN;
    let (_, inks) = paint(&g, now, None);
    let x = AXIS.x_of(now);
    // the 2 px line, white .95, down through every row
    let line = inks.iter().find(|(tag, r, a)| *tag == 2 && near(r.w, 2.0) && near(r.x, x - 1.0) && near(*a, 0.95)).expect("the now line");
    assert!(line.1.y + line.1.h >= 292.0 + 5.0 * 108.0 + 100.0 - 0.01, "it runs through the sixth row");
    // a minute later it is a minute further on
    let (_, later) = paint(&g, now + MIN, None);
    let moved = later.iter().find(|(tag, r, a)| *tag == 2 && near(r.w, 2.0) && near(*a, 0.95) && r.h > 400.0).expect("the line");
    assert!(near(moved.1.x - line.1.x, 1_392.0 / 90.0), "15.47 px a minute");
}

use crate::draw_census::RRectFill;

/// Every aired-time shade of a frame in draw order: the fills of exactly the shade's colour (nothing else on the
/// board is that translucent black), with the rounded clip each was drawn under.
fn shades(g: &TimeGrid, now: i64) -> Vec<RRectFill> {
    let fills = crate::draw_census::capture_rrects(|| g.draw(Painter::recording(), &Fixed, now, Some(0), WORDS, &Faces, None));
    fills.into_iter().filter(|f| f.col.iter().zip(theme::GUIDE_AIRED_SHADE).all(|(a, b)| near(*a, b))).collect()
}

/// A rect inside another, to a hair.
fn within(inner: Rect, outer: Rect) -> bool {
    inner.x >= outer.x - 0.01 && inner.y >= outer.y - 0.01
        && inner.x + inner.w <= outer.x + outer.w + 0.01 && inner.y + inner.h <= outer.y + outer.h + 0.01
}

/// Every cell of the page showing, as the board places it.
fn page_cells(g: &TimeGrid) -> Vec<Rect> {
    g.shown().flat_map(|row| (0..g.cells(row)).map(move |index| g.rect(cell(row, index)))).collect()
}

/// A rounded rect's coverage at a point, as the rect program's SDF takes it (`fs_src.frag`: the radius is the right one
/// on the right of the middle and the left one on the left, held to the half extents).
fn own_cover(r: Rect, rl: f32, rr: f32, x: f32, y: f32) -> f32 {
    let rad = (if x > r.cx() { rr } else { rl }).min(r.w * 0.5).min(r.h * 0.5).max(0.0);
    let q = ((x - r.cx()).abs() - (r.w * 0.5 - rad), (y - r.cy()).abs() - (r.h * 0.5 - rad));
    let d = (q.0.max(0.0).powi(2) + q.1.max(0.0).powi(2)).sqrt() + q.0.max(q.1).min(0.0) - rad;
    (0.5 - d).clamp(0.0, 1.0)
}

/// What a fill leaves on a point: its own corners, times the rounded clip it was drawn under (the shared
/// `rclip_cover` mask, the one the GPU's masked rect program is graded against).
fn cover(f: &RRectFill, x: f32, y: f32) -> f32 {
    // a fill with no radius at either side is a plain quad (`draw_rrect`'s flat path): no antialiased edge of its own
    let own = if f.rl < 0.5 && f.rr < 0.5 { 1.0 } else { own_cover(f.rect, f.rl, f.rr, x, y) };
    let clip = f.round.map_or(1.0, |(r, radius)| {
        plx_gfx::gfx::rclip_cover([x, y], plx_gfx::gfx::RClip { x0: r.x, y0: r.y, x1: r.x + r.w, y1: r.y + r.h, radius })
    });
    // the fill is a rect: nothing outside it, whatever the clip says
    let inside = x >= f.rect.x && x <= f.rect.x + f.rect.w && y >= f.rect.y && y <= f.rect.y + f.rect.h;
    if inside { own * clip } else { 0.0 }
}

#[test]
fn the_aired_shade_stays_inside_the_cells_and_never_touches_a_gap() {
    let g = board();
    let cells = page_cells(&g);
    for minutes in [10, 30, 50, 90, 120] {
        let shades = shades(&g, START + minutes * MIN);
        assert!(!shades.is_empty(), "{minutes} min in: something has aired");
        for shade in &shades {
            // inside one cell: so not in the 8 px between two rows, not between two cells, and not left of the first
            assert!(cells.iter().any(|c| within(shade.rect, *c)), "{minutes} min in: the shade {:?} stands outside every cell", shade.rect);
            assert!(near(shade.rect.h, 100.0), "{minutes} min in: a shade is a cell tall, not a board tall");
        }
    }
}

#[test]
fn a_cell_the_now_line_cuts_is_shaded_flat_to_the_line_under_the_cells_own_rounded_clip() {
    let g = board();
    let now = START + 30 * MIN;
    let shades = shades(&g, now);
    // row 0's first programme runs 0..45 min: aired up to minute 30
    let cut = g.rect(cell(0, 0));
    let shade = shades.iter().find(|f| near(f.rect.y, cut.y) && near(f.rect.x, cut.x)).expect("the cut cell's shade");
    assert!(near(shade.rect.x + shade.rect.w, AXIS.x_of(now)), "it ends exactly at the now line");
    assert!(near(shade.rect.h, cut.h));
    assert!(near(shade.rl, 0.0) && near(shade.rr, 0.0), "no radius is chosen for the shade: it is flat");
    let (round, radius) = shade.round.expect("under the rounded clip");
    assert!(near(round.x, cut.x) && near(round.y, cut.y) && near(round.w, cut.w) && near(round.h, cut.h), "the clip is the cell's rect, whole");
    assert!(near(radius, 10.0), "and the cell's radius");
    // a cell that began before the window and runs on past now is cut the same way, from the axis' left edge
    let long = g.rect(cell(1, 0));
    let shade = shades.iter().find(|f| near(f.rect.y, long.y)).expect("row 1's shade");
    assert!(near(shade.rect.x, 432.0) && near(shade.rect.x + shade.rect.w, AXIS.x_of(now)));
    assert_eq!(shade.round.map(|(r, radius)| (r.x, r.w, radius)), Some((long.x, long.w, 10.0)));
    // the programme that begins at the line has nothing aired
    let next = g.rect(cell(0, 1));
    assert!(!shades.iter().any(|f| near(f.rect.y, next.y) && f.rect.x >= next.x - 0.01), "no shade on a cell the line has not reached");
}

#[test]
fn a_cell_wholly_aired_is_the_cells_own_rounded_rect() {
    let g = board();
    // rows 3..: a programme 0..30 min, then 30..90; at minute 40 the first is over
    let shades = shades(&g, START + 40 * MIN);
    let aired = g.rect(cell(3, 0));
    let shade = shades.iter().find(|f| near(f.rect.y, aired.y) && near(f.rect.x, aired.x)).expect("the aired cell's shade");
    assert!(near(shade.rect.w, aired.w) && near(shade.rect.h, aired.h), "the shade is the cell");
    assert!(near(shade.rl, 10.0) && near(shade.rr, 10.0), "round on both sides, as the cell is");
    assert!(shade.round.is_none(), "no clip: the cell's own outline is the shade's");
    // and the one the line cuts, in the same row, is the clipped flat kind
    let cut = g.rect(cell(3, 1));
    let shade = shades.iter().find(|f| near(f.rect.y, cut.y) && near(f.rect.x, cut.x)).expect("the cut cell's shade");
    assert!(shade.round.is_some() && near(shade.rl, 0.0) && near(shade.rr, 0.0));
}

/// **The outline the shade actually leaves**, for a now line `inset` px inside each edge of a programme: wherever the
/// cell is drawn, the shade is drawn left of the line and nowhere else, so it never leaves the cell's corners and
/// never misses its own. Graded point by point against the cell's own rounded rect.
fn assert_the_shade_is_the_cell_left_of_the_line(shades: &[RRectFill], cell_rect: Rect, now_x: f32, what: &str) {
    let mine: Vec<&RRectFill> = shades.iter().filter(|f| near(f.rect.y, cell_rect.y) && f.rect.x >= cell_rect.x - 0.01 && f.rect.x + f.rect.w <= cell_rect.x + cell_rect.w + 0.01).collect();
    let mut x = cell_rect.x - 2.0;
    while x <= cell_rect.x + cell_rect.w + 2.0 {
        let mut y = cell_rect.y - 2.0;
        while y <= cell_rect.y + cell_rect.h + 2.0 {
            // a point within a pixel of the line is its own antialiasing's business
            if (x - now_x).abs() > 1.0 {
                let want = if x < now_x { own_cover(cell_rect, 10.0, 10.0, x, y) } else { 0.0 };
                let got = mine.iter().map(|f| cover(f, x, y)).fold(0.0f32, f32::max);
                assert!((want - got).abs() < 0.02, "{what}: at ({x}, {y}) the cell leaves {want} of the shade and it leaves {got}");
            }
            y += 1.0;
        }
        x += 1.0;
    }
}

/// The line `inset` px inside a cell's edge, for 1..=9 px, for programmes that begin and end in the window and one that
/// begins before it: `edge` says which side of the cell the line is `inset` px inside of.
fn grade_the_line_near(edge: fn(Rect, f32) -> f32) {
    let g = board();
    for (row, index) in [(0usize, 0usize), (0, 1), (1, 0), (3, 0), (3, 1)] {
        let rect = g.rect(cell(row, index));
        for inset in 1..=9 {
            let want_x = edge(rect, inset as f32);
            let now = START + ((want_x - AXIS_X0) * 5_400.0 / 1_392.0).round() as i64;
            let at = AXIS.x_of(now);
            assert_the_shade_is_the_cell_left_of_the_line(&shades(&g, now), rect, at, &format!("row {row} cell {index}, line at {at}, {inset} px in"));
        }
    }
}

#[test]
fn a_line_within_ten_px_of_a_cells_start_shades_the_cells_own_corners_not_a_sliver_with_its_own() {
    grade_the_line_near(|rect, inset| rect.x + inset);
}

#[test]
fn a_line_within_ten_px_of_a_cells_end_leaves_the_cells_rounded_right_corners_alone() {
    grade_the_line_near(|rect, inset| rect.x + rect.w - inset);
}

#[test]
fn a_window_wholly_aired_shades_every_cell_whole() {
    let g = board();
    let shades = shades(&g, START + 200 * MIN);
    let cells: Vec<Rect> = page_cells(&g).into_iter().filter(|c| c.w > 0.0).collect();
    assert_eq!(shades.len(), cells.len(), "one shade a cell");
    for cell in cells {
        let shade = shades.iter().find(|f| near(f.rect.x, cell.x) && near(f.rect.y, cell.y)).expect("a shade for the cell");
        assert!(near(shade.rect.w, cell.w) && near(shade.rl, 10.0) && near(shade.rr, 10.0) && shade.round.is_none());
    }
}

#[test]
fn nothing_is_shaded_while_the_window_is_wholly_ahead_of_now() {
    let g = TimeGrid::new(Axis::new(START + 1_800, 5_400, 1_800), vec![vec![span(30, 90)], vec![]], 0);
    assert!(shades(&g, START + 10 * MIN).is_empty());
    // not even at the instant the window opens
    assert!(shades(&g, START + 30 * MIN).is_empty());
}

#[test]
fn the_chip_carries_the_clock_time_centred_on_now() {
    let g = board();
    let now = START + 30 * MIN;
    let (runs, inks) = paint(&g, now, None);
    let chip = inks.iter().find(|(tag, r, a)| *tag == 2 && near(r.h, 36.0) && near(*a, 0.95)).expect("the chip");
    assert!(near(chip.1.cx(), AXIS.x_of(now)));
    assert!(chip.1.w >= 88.0);
    let face = drawn(fmt::clock_face(now, 0).unwrap());
    assert!(runs.iter().any(|run| run.text == face && run.bold), "the chip's words are bold");
}

#[test]
fn the_ruler_says_every_half_hour_and_the_page_indicator() {
    let g = board();
    let (runs, _) = paint(&g, START + 10 * MIN, None);
    for t in [START, START + 1_800, START + 3_600] {
        let face = drawn(fmt::clock_face(t, 0).unwrap());
        assert!(runs.iter().any(|run| run.text == face), "{face} on the ruler");
    }
    let indicator = runs.iter().find(|run| run.text == WORDS.indicator).expect("the indicator");
    assert!(near(indicator.at.x, 96.0));
    assert!(indicator.at.y >= 236.0 && indicator.at.y <= 276.0, "on the ruler line");
}

#[test]
fn a_cell_says_its_title_and_its_clock_times_and_the_focused_one_is_lit() {
    let g = board();
    let (runs, inks) = paint(&g, START + 10 * MIN, Some(cell(0, 1)));
    assert!(runs.iter().any(|run| run.text == "News" && run.bold));
    assert!(runs.iter().any(|run| run.text == "Weather"));
    let range = drawn(fmt::airing_range(START + 45 * MIN, START + 90 * MIN, 0).unwrap());
    assert!(runs.iter().any(|run| run.text == range), "{range}");
    // a focused cell is white .92, with its shadow
    let rect = g.rect(cell(0, 1));
    assert!(inks.iter().any(|(tag, r, a)| *tag == 2 && near(r.x, rect.x) && near(r.y, rect.y) && near(*a, 0.92)));
    assert!(inks.iter().any(|(tag, r, _)| *tag == 4 && near(r.x, rect.x) && near(r.y, rect.y)), "the focus shadow");
    // an idle one is the idle plate
    let idle = g.rect(cell(0, 0));
    assert!(inks.iter().any(|(tag, r, a)| *tag == 2 && near(r.x, idle.x) && near(r.y, idle.y) && near(*a, 0.08)));
}

#[test]
fn the_focused_row_lights_its_channel_cell() {
    let g = board();
    let (_, inks) = paint(&g, START + 10 * MIN, Some(cell(1, 0)));
    let chan = |row: usize| g.channel_rect(row).unwrap();
    let alpha = |row: usize| inks.iter().find(|(tag, r, _)| *tag == 2 && near(r.x, 96.0) && near(r.y, chan(row).y) && near(r.w, 320.0)).map(|(_, _, a)| *a);
    assert_eq!(alpha(1), Some(theme::TAB_PLATE_SELECTED[3]));
    assert_eq!(alpha(0), Some(theme::SURFACE_PANEL[3]));
}

#[test]
fn a_row_with_nothing_says_the_words_the_screen_gave_it_and_no_times() {
    let g = board();
    let (runs, _) = paint(&g, START + 10 * MIN, None);
    assert!(runs.iter().any(|run| run.text == "No guide data"));
}

#[test]
fn the_page_not_showing_is_not_drawn() {
    let mut g = board();
    g.follow(6);
    let (runs, _) = paint(&g, START + 10 * MIN, None);
    assert!(!runs.iter().any(|run| run.text == "Weather"), "row 0 is on the first page");
}

#[test]
fn the_logo_is_the_recent_tiles_at_seventy_percent_in_a_175_by_98_box() {
    let cell = Rect::new(96.0, 292.0, 320.0, 100.0);
    let boxed = logo_rect(cell);
    assert!(near(boxed.x, 96.0 + 128.0) && near(boxed.w, 175.0) && near(boxed.h, 98.0));
    assert!(near(boxed.cy(), cell.cy()));
    assert!(near(boxed.w, 250.0 * LOGO_SCALE) && near(boxed.h, 140.0 * LOGO_SCALE), "the tile's 250 x 140 at 70%");
    let (x, y) = number_at(cell);
    assert!(near(x, 96.0 + 20.0) && near(y, cell.cy()));
}

#[test]
fn leaving_an_unscheduled_row_lands_on_what_is_on_now_whichever_way_the_move_goes() {
    // row 0 nothing, rows 1 and 2 two programmes split at minute 30, with the longer one second
    let g = TimeGrid::new(AXIS, vec![vec![], vec![span(0, 30), span(30, 90)], vec![span(0, 30), span(30, 90)]], 0);
    assert_eq!(g.step(cell(0, 0), Dir::Down, START + 10 * MIN), Some(cell(1, 0)), "on now: the first");
    assert_eq!(g.step(cell(0, 0), Dir::Down, START + 40 * MIN), Some(cell(1, 1)), "on now: the second");
    assert_eq!(g.step(cell(0, 0), Dir::Down, START + 30 * MIN), Some(cell(1, 1)), "a programme begins at its first second");
    assert_eq!(g.step(cell(0, 0), Dir::Down, START + 29 * MIN + 59), Some(cell(1, 0)));
    let g = TimeGrid::new(AXIS, vec![vec![span(0, 30), span(30, 90)], vec![]], 0);
    assert_eq!(g.step(cell(1, 0), Dir::Up, START + 10 * MIN), Some(cell(0, 0)), "Up too");
    // a window still ahead of now (the Guide scrolled): the first programme of the window
    assert_eq!(g.step(cell(1, 0), Dir::Up, START - 20 * MIN), Some(cell(0, 0)));
    // after it (nothing else is possible for a live window, but the rule holds): the last
    assert_eq!(g.step(cell(1, 0), Dir::Up, START + 200 * MIN), Some(cell(0, 1)));
}

#[test]
fn a_page_turn_out_of_an_unscheduled_row_lands_on_what_is_on_now() {
    let mut rows: Vec<Vec<Span>> = vec![vec![]];
    rows.extend((1..12).map(|_| vec![span(0, 30), span(30, 90)]));
    let g = TimeGrid::new(AXIS, rows, 0);
    assert_eq!(g.turn(cell(0, 0), 1, START + 10 * MIN), Some(cell(6, 0)));
    assert_eq!(g.turn(cell(0, 0), 1, START + 50 * MIN), Some(cell(6, 1)));
}

#[test]
fn the_ruler_follows_a_window_that_moved() {
    // the window one step on: its ticks are the later half hours
    let g = TimeGrid::new(Axis::new(START + 1_800, 5_400, 1_800), vec![vec![span(30, 90)]], 0);
    let (runs, _) = paint(&g, START + 10 * MIN, None);
    for t in [START + 1_800, START + 3_600, START + 5_400] {
        let face = drawn(fmt::clock_face(t, 0).unwrap());
        assert!(runs.iter().any(|run| run.text == face), "{face} on the ruler");
    }
    let first = drawn(fmt::clock_face(START, 0).unwrap());
    assert!(!runs.iter().any(|run| run.text == first), "the half hour before the window is not on it");
}

#[test]
fn the_now_line_shade_and_chip_are_drawn_only_while_now_is_in_the_window() {
    // a window ahead of now: no line, no shade, no chip
    let g = TimeGrid::new(Axis::new(START + 1_800, 5_400, 1_800), vec![vec![span(30, 90)]], 0);
    let now = START + 10 * MIN;
    let (runs, inks) = paint(&g, now, None);
    assert!(!inks.iter().any(|(tag, r, a)| *tag == 2 && near(r.w, 2.0) && near(*a, 0.95) && r.h > 100.0), "no now line");
    assert!(!inks.iter().any(|(tag, _, a)| *tag == 2 && near(*a, 0.22)), "no shade");
    assert!(!inks.iter().any(|(tag, r, a)| *tag == 2 && near(r.h, 36.0) && near(*a, 0.95)), "no chip");
    assert!(!runs.iter().any(|run| run.bold && run.text == drawn(fmt::clock_face(now, 0).unwrap())), "no chip words");
    // the live window has all three
    let (_, live) = paint(&board(), now, None);
    assert!(live.iter().any(|(tag, _, a)| *tag == 2 && near(*a, 0.22)));
    assert!(live.iter().any(|(tag, r, a)| *tag == 2 && near(r.h, 36.0) && near(*a, 0.95)));
}

#[test]
fn a_chip_over_a_ruler_label_hides_that_label_only() {
    let g = board();
    // 29 minutes in: the chip (88 wide, centred on 432 + 29 min) runs over the 30-minute tick's label
    let (runs, _) = paint(&g, START + 29 * MIN, None);
    let label = |t: i64| drawn(fmt::clock_face(t, 0).unwrap());
    assert!(!runs.iter().any(|run| run.text == label(START + 1_800) && !run.bold), "the covered label is not drawn");
    assert!(runs.iter().any(|run| run.text == label(START + 3_600)), "the others are");
    // and the tick itself stays: two px wide, on the ruler
    let (_, inks) = paint(&g, START + 29 * MIN, None);
    let x = AXIS.x_of(START + 1_800);
    assert!(inks.iter().any(|(tag, r, _)| *tag == 2 && near(r.w, 2.0) && near(r.x, x) && near(r.h, 32.0)));
    // ten minutes in the chip covers none
    let (runs, _) = paint(&g, START + 10 * MIN, None);
    for t in [START, START + 1_800, START + 3_600] {
        assert!(runs.iter().any(|run| run.text == label(t)), "{} on the ruler", label(t));
    }
}

#[test]
fn a_frame_with_no_zone_offset_has_no_clock_words() {
    let g = board();
    let (runs, inks) = paint_in(&g, START + 10 * MIN, None, None);
    let face = |t: i64| drawn(fmt::clock_face(t, 0).unwrap());
    for t in [START, START + 1_800, START + 3_600, START + 10 * MIN] {
        assert!(!runs.iter().any(|run| run.text == face(t)), "{} needs a zone", face(t));
    }
    assert!(!runs.iter().any(|run| run.text == drawn(fmt::airing_range(START, START + 45 * MIN, 0).unwrap())), "no programme times");
    assert!(runs.iter().any(|run| run.text == "News"), "the programmes' titles are there");
    assert!(runs.iter().any(|run| run.text == WORDS.indicator));
    assert!(inks.iter().any(|(tag, r, a)| *tag == 2 && near(r.w, 2.0) && near(*a, 0.95) && r.h > 400.0), "the now line is the instant's");
    assert!(!inks.iter().any(|(tag, r, a)| *tag == 2 && near(r.h, 36.0) && near(*a, 0.95)), "and there is no chip");
}

/// Two rows: the one the focus stands on (`from`) and the row it moves to.
fn pair(from: Span, to: Vec<Span>) -> TimeGrid {
    TimeGrid::new(AXIS, vec![vec![from], to], 100)
}

/// **A move lands on what is on air or ahead, not on a programme that has aired**: while `now` is in the window the
/// overlap with the cell left is measured over its part from `now` on, so a tie in the whole window is not a tie.
#[test]
fn a_move_into_a_row_prefers_the_cell_on_air_to_one_that_has_aired_when_they_tie() {
    let now = START + 55 * MIN;
    let g = pair(span(20, 80), vec![span(0, 50), span(50, 90)]);
    assert_eq!(g.step(cell(0, 0), Dir::Down, now), Some(cell(1, 1)));
    assert_eq!(g.land(1, Some(span(20, 80)), now), Some(cell(1, 1)));
    // the same through a page turn
    let rows = (0..PAGE_ROWS + 1).map(|r| if r == 0 { vec![span(20, 80)] } else { vec![span(0, 50), span(50, 90)] }).collect();
    let g = TimeGrid::new(AXIS, rows, 100);
    assert_eq!(g.turn(cell(0, 0), 1, now), Some(cell(PAGE_ROWS, 1)));
}

/// A programme that has aired and shares more of the whole window with the cell left than the one on air still loses
/// to it.
#[test]
fn an_aired_cell_that_overlaps_more_of_the_window_does_not_beat_the_one_on_air() {
    let now = START + 65 * MIN;
    let g = pair(span(10, 80), vec![span(0, 60), span(60, 90)]);
    assert_eq!(g.step(cell(0, 0), Dir::Down, now), Some(cell(1, 1)));
}

/// With nothing of the row on air or ahead of the cell left, the whole-window overlap decides, as it did.
#[test]
fn a_row_of_aired_cells_alone_lands_on_the_one_sharing_most_of_the_window() {
    let now = START + 55 * MIN;
    let g = pair(span(20, 80), vec![span(0, 30), span(30, 50)]);
    assert_eq!(g.step(cell(0, 0), Dir::Down, now), Some(cell(1, 1)));
    assert_eq!(g.land(1, Some(span(0, 30)), now), Some(cell(1, 0)));
}

/// With `now` before the window (a window ahead) nothing has aired in it: landing is the overlap of the whole window.
#[test]
fn landing_with_now_before_the_window_is_the_overlap_of_the_whole_window() {
    let now = START - 10 * MIN;
    let g = pair(span(20, 80), vec![span(0, 50), span(50, 90)]);
    assert_eq!(g.step(cell(0, 0), Dir::Down, now), Some(cell(1, 0)), "a tie goes to the earlier cell");
    assert_eq!(g.land(1, Some(span(40, 90)), now), Some(cell(1, 1)));
}
