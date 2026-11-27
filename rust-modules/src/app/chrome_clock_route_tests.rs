//! **The clock on the shared bar, on the real pages** (Live TV handoff SPEC-1 item 2: "the clock goes in
//! the TopBar on every screen"): the app-owned chrome draws it on Home, the Library and the Watchlist at
//! the bar's right end, it is no focus stop and no hit target, it follows the bar's tuck, and the player
//! (which wears no bar) draws none. What a person reads is what the page DREW (`World::drawn`).

use super::*;
use super::test_support::World;

/// 2026-11-26 19:50:00 UTC, a whole minute.
const AT: i64 = 1_795_636_200;
/// UTC+3: the face is 22:50 (or 10:50 PM) whatever the style.
const MOSCOW: i32 = 3 * 3_600;

/// The face the clock must read at `AT` in `MOSCOW`, worded by the one formatter.
fn face() -> String {
    plx_ui::fmt::clock_face(AT, i64::from(MOSCOW)).expect("a face")
}

/// Publish the loop's wall time as `publish_clock` receives it; returns whether the face changed.
fn publish(w: &mut World, unix_s: i64, offset_s: Option<i32>) -> bool {
    w.bridge.bridge().publish_clock(Some(crate::app::clock::Reading { unix_ms: unix_s * 1_000, utc_offset_s: offset_s }))
}

/// The clock's runs in `w`'s frame: the ink (TEXT_SECONDARY, which no other run on a bar page wears at
/// BODY in the bar's row) of the face's words.
fn clock_runs(w: &mut World, face: &str) -> Vec<plx_ui::draw_census::Run> {
    let secondary = plx_ui::theme::TEXT_SECONDARY;
    w.drawn().runs.into_iter()
        .filter(|run| run.text == face && run.size == plx_ui::theme::size::BODY && run.ink[..3] == secondary[..3])
        .collect()
}

/// The clock stands where the handoff puts it, on every page that wears the bar.
fn assert_on_the_bar(w: &mut World, page: &str) {
    let face = face();
    let runs = clock_runs(w, &face);
    assert_eq!(runs.len(), 1, "{page}: exactly one clock face {face:?} in the bar's row, got {runs:?}");
    let run = &runs[0];
    assert!(!run.bold, "{page}: regular");
    assert_eq!(run.align, 2, "{page}: right-aligned");
    assert_eq!(run.at.x + run.at.w, plx_ui::consts::SCR_W - plx_ui::consts::MARGIN_X, "{page}: its right edge is the margin, 1824");
    let (cap_top, baseline) = plx_gfx::text::text_cap_band(run.size, 0);
    assert_eq!(run.at.y + (cap_top + baseline) * 0.5, plx_ui::widgets::TOP_BAR_Y + plx_ui::widgets::TAB_PILL_H * 0.5,
        "{page}: centred on the bar (y 62, h 60)");
}

#[test]
fn home_draws_the_clock_at_the_bars_right_end() {
    let mut w = World::home_with_disc("clock-home", [7; 12]);
    assert!(clock_runs(&mut w, &face()).is_empty(), "no wall time published yet: no clock");
    publish(&mut w, AT, Some(MOSCOW));
    assert_on_the_bar(&mut w, "Home");
}

#[test]
fn the_library_draws_it_too() {
    let mut w = World::movies_library("clock-library", false);
    publish(&mut w, AT, Some(MOSCOW));
    assert_on_the_bar(&mut w, "Library");
}

#[test]
fn and_so_does_the_watchlist() {
    let mut w = World::watchlist("clock-watchlist");
    publish(&mut w, AT, Some(MOSCOW));
    assert_on_the_bar(&mut w, "Watchlist");
}

/// Never focusable: the bar's focus stops are the chip and the strip's pills, and a walk along them
/// never lands on anything the clock is; a click on the clock lands on nothing and moves no focus.
#[test]
fn the_clock_is_no_focus_stop_and_no_hit_target() {
    let mut w = World::home_with_disc("clock-focus", [7; 12]);
    publish(&mut w, AT, Some(MOSCOW));
    w.frames(2);
    for _ in 0..6 {
        if w.on_nav() { break; }
        w.press(Key::Up);
    }
    assert!(w.on_nav(), "premise: Up from the page reaches the bar");
    let mut elems = Vec::new();
    for _ in 0..12 {
        elems.extend(w.elem());
        w.press(Key::Right);
    }
    elems.extend(w.elem());
    assert!(!elems.is_empty() && elems.iter().all(|elem| *elem >= plx_ui::dispatch::STRIP_BASE),
        "every stop the walk reached is the chip or a pill of the strip: {elems:?}");
    let strip: Vec<u32> = (0..8).map(|i| plx_ui::dispatch::STRIP_BASE + i).collect();
    assert!(elems.iter().all(|elem| strip.contains(elem)), "{elems:?}");

    let before = (w.focus(), w.on(&AppArg::Home));
    w.present_bar();
    let frame = plx_ui::widgets::top_bar_clock_frame(plx_ui::fixture::FixtureMeasure.width(
        std::ffi::CString::new(face()).unwrap().as_c_str(), plx_ui::widgets::TOP_BAR_CLOCK_SIZE, false));
    w.click(frame.cx(), frame.cy());
    assert_eq!((w.focus(), w.on(&AppArg::Home)), before, "a click on the clock does nothing");
}

/// The clock rides the bar's painter: a page that tucks the bar lifts and fades the clock with it, so a
/// tucked Library shows none (a run at alpha 0 paints no pixel, and the lift has carried it off the top).
#[test]
fn the_clock_follows_the_bars_tuck() {
    let mut w = World::movies_library("clock-tuck", true);
    publish(&mut w, AT, Some(MOSCOW));
    let rest = clock_runs(&mut w, &face());
    assert_eq!(rest.len(), 1, "premise: the bar is shown on arrival and carries the clock");
    assert!(w.tuck() < 0.5, "premise: untucked");
    w.climb_into_header();
    for _ in 0..3 { w.press(Key::Down); }
    w.frames(60);
    assert!(w.tuck() > 0.99, "premise: the grid's rows have tucked the chrome (tuck {})", w.tuck());
    let tucked = clock_runs(&mut w, &face());
    assert!(tucked.iter().all(|run| run.ink[3] < 0.01), "a tucked bar shows no clock: {tucked:?}");
    // and it comes back with the bar
    for _ in 0..4 { w.press(Key::Up); }
    w.frames(60);
    assert!(w.tuck() < 0.01, "premise: the bar is back (tuck {})", w.tuck());
    assert_eq!(clock_runs(&mut w, &face()).len(), 1, "the clock returns with the bar");
}

/// Search and Live TV wear the bar, so they carry the clock where Home does: drawn runs, not a list.
#[test]
fn search_draws_it_too() {
    let mut w = World::search_results("clock-search", |_| Vec::new());
    publish(&mut w, AT, Some(MOSCOW));
    assert_on_the_bar(&mut w, "Search");
}

#[test]
fn and_so_does_live_tv() {
    let mut w = World::new("clock-livetv");
    super::show_page(&mut w.d, AppArg::LiveTv);
    w.frames(60);
    assert!(w.on(&AppArg::LiveTv), "premise: Live TV is on top");
    publish(&mut w, AT, Some(MOSCOW));
    assert_on_the_bar(&mut w, "Live TV");
}

/// **A page that wears no bar draws no clock**, and the ONE answer to "does the bar show a clock this
/// frame" says so: the detail page's drawn runs hold no face and `Bridge::bar_clock` is `None` for it,
/// where it is `Some` on Home. (It is that answer that the minute's damage and the dev fps counter's
/// step ask, so neither fires for a clock nobody can see.)
#[test]
fn a_page_without_the_bar_draws_no_clock_and_the_one_answer_says_none() {
    let mut home = World::home_with_disc("clock-answer-home", [7; 12]);
    publish(&mut home, AT, Some(MOSCOW));
    home.frames(2);
    let shown = home.bridge.bridge().bar_clock(&home.d).map(|clock| clock.w);
    assert!(shown.is_some_and(|w| w > 0.0), "Home: the bar shows a clock ({shown:?})");
    drop(home);

    let mut detail = World::detail_with_disc("clock-answer-detail", [7; 12]);
    publish(&mut detail, AT, Some(MOSCOW));
    detail.frames(2);
    assert!(detail.d.top_arg().is_some_and(|arg| !Bridge::draws_chrome_for(arg)), "premise: a page without the bar is on top");
    assert!(clock_runs(&mut detail, &face()).is_empty(), "the detail page draws no clock");
    assert!(detail.bridge.bridge().bar_clock(&detail.d).is_none(), "and the one answer is none");
}

/// A tucked bar (the Library's grid has lifted and faded it) shows no clock, in the one answer as in
/// the drawn runs; it answers again when the bar returns.
#[test]
fn a_tucked_bar_shows_no_clock_in_the_one_answer() {
    let mut w = World::movies_library("clock-answer-tuck", true);
    publish(&mut w, AT, Some(MOSCOW));
    w.frames(2);
    assert!(w.bridge.bridge().bar_clock(&w.d).is_some(), "premise: the bar is shown on arrival");
    w.climb_into_header();
    for _ in 0..3 { w.press(Key::Down); }
    w.frames(60);
    assert!(w.tuck() > 0.99, "premise: the grid's rows have tucked the chrome (tuck {})", w.tuck());
    assert!(w.bridge.bridge().bar_clock(&w.d).is_none(), "tucked: none");
    for _ in 0..4 { w.press(Key::Up); }
    w.frames(60);
    assert!(w.bridge.bridge().bar_clock(&w.d).is_some(), "and back with the bar");
}

/// The player wears no bar at all (its argument's registry answer), the other half of the same rule.
#[test]
fn the_player_wears_no_bar_and_so_carries_no_clock() {
    use plx_ui::screen::ScreenArg;
    assert_ne!(AppArg::Player.chrome(), plx_machine::machine::Chrome::TabBar);
    let mut w = World::new("clock-player");
    super::show_page(&mut w.d, AppArg::Player);
    w.frames(30);
    assert!(w.on(&AppArg::Player), "premise: the player is on top");
    publish(&mut w, AT, Some(MOSCOW));
    assert!(clock_runs(&mut w, &face()).is_empty(), "the player draws no clock");
    assert!(w.bridge.bridge().bar_clock(&w.d).is_none(), "and the one answer is none");
    for arg in [AppArg::Home, AppArg::Search, AppArg::Watchlist, AppArg::LiveTv, AppArg::LibraryRecommended { section: 0 }] {
        assert_eq!(arg.chrome(), plx_machine::machine::Chrome::TabBar, "a bar page");
    }
}

/// No wall time (the zone cannot be read, a replay) is no clock: a face is never guessed.
#[test]
fn a_zone_that_cannot_be_read_draws_no_clock() {
    let mut w = World::home_with_disc("clock-no-zone", [7; 12]);
    publish(&mut w, AT, Some(MOSCOW));
    assert_eq!(clock_runs(&mut w, &face()).len(), 1);
    assert!(publish(&mut w, AT, None), "losing the zone is a change");
    assert!(clock_runs(&mut w, &face()).is_empty());
}
