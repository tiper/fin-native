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
    // [0,45) / [45,90) of row 0 the first (45 shared) wins on a tie by the earliest
    let g = board();
    assert_eq!(g.step(cell(1, 0), Dir::Up, NOW), Some(cell(0, 0)));
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

#[test]
fn time_already_aired_is_shaded_from_the_axis_edge_to_the_now_line() {
    let g = board();
    let now = START + 30 * MIN;
    let (_, inks) = paint(&g, now, None);
    let shade = inks.iter().find(|(tag, _, a)| *tag == 2 && near(*a, 0.22)).expect("the shade");
    assert!(near(shade.1.x, 432.0));
    assert!(near(shade.1.x + shade.1.w, AXIS.x_of(now)));
    assert!(near(shade.1.y, 292.0));
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
