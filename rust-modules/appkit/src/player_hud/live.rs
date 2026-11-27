//! **The live banner** (Live TV PR4; handoff SPEC-4): what the player draws on a live channel in place
//! of the VOD transport, and the art ground it stands on until the first frame.
//!
//! The banner is the HUD package's (`layout.live`, HUD format 1.2, [`LiveLayout`]): where it sits and how
//! tall its scrim is are the package's, its inside is the app's, as Up Next's is. [`words`] is the one pure
//! reading of what the banner says (`Words`), from the channel it is for and the frame's wall time, so a
//! test grades the wording without a GL context; [`draw`] paints it.
//!
//! **Whose channel it shows** is the screen's decision, not this module's: the pending zap target while
//! one is being stepped to, else the channel the session is playing. The guide's raw begin and end ride
//! the channel and every clock face, "22m left" and the progress fraction are derived at draw from the
//! frame's [`Wall`] by `plx_ui::fmt` (the app's one airing wording), so nothing here caches words.
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use plx_data::livetv::Channel;
use plx_ui::consts::{SCR_H, SCR_W};
use plx_ui::fmt::{self, Airing, Wall};
use plx_ui::label::{HAlign, Label, VAlign};
use plx_ui::theme::{self, size};
use plx_ui::{Painter, Rect};

/// How long the banner takes to fade out once its time is up (SPEC-4: .35 s).
pub const FADE_MS: u32 = 350;
/// How long after the last action the banner starts to fade (SPEC-4: 4 s). A declared value of the live
/// banner: the HUD's general linger (`LINGER_MS`, 4500) is the VOD transport's and stays.
pub const SHOW_MS: u32 = 4_000;
/// The handoff's gap between the channel line and the title, the title's line and the rows under it.
const GAP: f32 = 16.0;
const TIME_GAP: f32 = 8.0;
const BADGE_H: f32 = 32.0;
const BADGE_PAD: f32 = 10.0;
const TITLE_LINE: f32 = 56.0;
const TEXT_LINE: f32 = 32.0;
const PROGRESS_H: f32 = 6.0;
/// The badge's tracking, .06em of its MICRO size.
const BADGE_TRACKING: f32 = 0.06 * size::MICRO as f32;

/// **`layout.live`**: where the banner stands. All of it is the package's choice; the defaults are
/// `builtin:classic`'s, which is also what a package that declares none draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveLayout {
    /// the block's distance from the screen's left edge, and the clock's from its right
    pub x: f32,
    /// the block's and the clock's distance above the screen's bottom edge
    pub bottom: f32,
    /// the scrim's height from the screen's bottom
    pub scrim_h: f32,
    /// the title's widest extent before it elides
    pub title_w: f32,
    /// the progress bar's width
    pub progress_w: f32,
}

impl LiveLayout {
    pub const CLASSIC: Self = Self { x: 96.0, bottom: 96.0, scrim_h: 520.0, title_w: 1200.0, progress_w: 560.0 };

    /// The package's `layout.live` (every key present: the table requires them all of a node that is there), else
    /// [`Self::CLASSIC`].
    pub fn read(node: Option<&crate::hud_format::parse::Object>) -> Self {
        let Some(node) = node else { return Self::CLASSIC };
        let px = |key: &str, default: f32| node.get(key).and_then(crate::hud_format::parse::Val::as_f64).map_or(default, |n| n as f32);
        let c = Self::CLASSIC;
        Self { x: px("x_inset", c.x), bottom: px("inset", c.bottom), scrim_h: px("scrim", c.scrim_h), title_w: px("title_width", c.title_w), progress_w: px("progress_width", c.progress_w) }
    }
}

/// What the banner says, worded for one frame ([`words`]).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Words {
    /// the channel's number (`vcn`), empty for a channel with none
    pub number: String,
    pub station: String,
    /// the programme on air; empty when the guide has none
    pub title: String,
    /// "21:37–22:39 · Started 1 min ago", else "21:00–22:00 · 22m left"; `None` without a wall time or an airing
    pub time_line: Option<String>,
    /// "Next at 22:39": the current airing's end; `None` under the same conditions
    pub next: Option<String>,
    /// how much of the airing has aired, `0.0..=1.0`
    pub progress: Option<f32>,
    /// the wall clock's face, "21:38"; `None` without a wall time
    pub clock: Option<String>,
}

/// **The banner's words** for `channel` (`name` stands in for the station of a channel the lineup does
/// not list, from the request's own title) at `wall`.
pub fn words(channel: Option<&Channel>, name: &str, wall: Option<Wall>) -> Words {
    let mut out = Words { station: name.to_owned(), ..Default::default() };
    if let Some(ch) = channel {
        out.number.clone_from(&ch.vcn);
        out.station.clone_from(&ch.name);
        out.title.clone_from(&ch.now);
    }
    let Some(wall) = wall else { return out };
    out.clock = fmt::clock_face(wall.unix_s(), i64::from(wall.utc_offset_s));
    if let Some(ch) = channel.filter(|ch| !ch.now.is_empty()) {
        out.time_line = fmt::airing_line(Airing::Banner, ch.begins_at, ch.ends_at, wall);
        out.progress = fmt::frac(ch.begins_at, ch.ends_at, wall);
        // "Next at": only while the airing is known, and only until it is over (a programme that is
        // over has no "next" until the guide re-reads)
        out.next = out.time_line.as_ref()
            .and_then(|_| fmt::clock_face(ch.ends_at, i64::from(wall.utc_offset_s)))
            .filter(|_| out.progress.is_some_and(|f| f < 1.0))
            .map(|face| plx_os::i18n::msg::widgets_live_next_at(&face));
    }
    out
}

fn c(text: &str) -> CString {
    CString::new(text.replace('\0', "")).unwrap_or_default()
}

fn label(p: Painter, text: &CStr, sz: i32, bold: bool, ink: [f32; 4], frame: Rect, h: HAlign, v: VAlign, tracking: f32) -> f32 {
    let l: Label = Label::new(text.as_ptr() as *const c_char, sz, ink).h(h).v(v).tracking(tracking);
    (if bold { l.bold() } else { l }).draw(p, frame)
}

/// Paint the scrim and the banner at `alpha` (1 is fully shown, 0 draws nothing).
pub fn draw(p: Painter, layout: &LiveLayout, words: &Words, alpha: f32, measure: &dyn plx_machine::machine::Measure) {
    if alpha <= 0.0 {
        return;
    }
    let p = p.alpha(alpha.min(1.0));
    // Scrim: 0 at the top, .55 at 45% of its height, .88 at the bottom.
    let top = SCR_H - layout.scrim_h;
    let mid = layout.scrim_h * 0.45;
    p.rect(Rect::new(0.0, top, SCR_W, mid), 0.0, theme::live_scrim(0.0), theme::live_scrim(0.55), 0.0);
    p.rect(Rect::new(0.0, top + mid, SCR_W, layout.scrim_h - mid), 0.0, theme::live_scrim(0.55), theme::live_scrim(0.88), 0.0);

    // The rows, bottom up from the block's foot, so a channel with no guide data keeps its block seated.
    let has_title = !words.title.is_empty();
    let has_time = words.time_line.is_some();
    let mut rows: Vec<f32> = vec![BADGE_H];
    if has_title { rows.push(GAP); rows.push(TITLE_LINE); }
    if has_time { rows.push(TIME_GAP); rows.push(TEXT_LINE); }
    if words.progress.is_some() { rows.push(GAP); rows.push(PROGRESS_H); }
    if words.next.is_some() { rows.push(GAP); rows.push(TEXT_LINE); }
    let mut y = SCR_H - layout.bottom - rows.iter().sum::<f32>();
    let mut at = |h: f32| { let r = Rect::new(layout.x, y, layout.title_w, h); y += h; r };

    // Channel line: LIVE badge, the number (bold), the station (secondary), 16 apart.
    let line = at(BADGE_H);
    let badge_text = plx_os::i18n::msg::widgets_live_badge();
    let badge = c(&badge_text);
    let badge_w = measure.width_str(&badge_text, size::MICRO, true)
        + BADGE_TRACKING * (badge_text.chars().count().saturating_sub(1)) as f32
        + 2.0 * BADGE_PAD;
    let chip = Rect::new(line.x, line.y, badge_w, BADGE_H);
    p.rrect(chip, theme::RADIUS_CHIP, theme::RADIUS_CHIP, theme::LIVE_BADGE_FILL);
    label(p, &badge, size::MICRO, true, theme::LIVE_BADGE_INK, chip, HAlign::Center, VAlign::Middle, BADGE_TRACKING);
    let mut x = chip.x + chip.w + GAP;
    if !words.number.is_empty() {
        let number = c(&words.number);
        let w = label(p, &number, size::BODY, true, theme::TEXT_PRIMARY, Rect::new(x, line.y, 0.0, BADGE_H), HAlign::Left, VAlign::Middle, 0.0);
        x += w + GAP;
    }
    if !words.station.is_empty() {
        let station = measure.fit_line(&words.station, (layout.x + layout.title_w - x).max(0.0), size::BODY, false);
        label(p, &station, size::BODY, false, theme::TEXT_SECONDARY, Rect::new(x, line.y, 0.0, BADGE_H), HAlign::Left, VAlign::Middle, 0.0);
    }

    if has_title {
        at(GAP);
        let r = at(TITLE_LINE);
        let title = measure.fit_line(&words.title, layout.title_w, size::DISPLAY, true);
        label(p, &title, size::DISPLAY, true, theme::TEXT_PRIMARY, r, HAlign::Left, VAlign::Middle, 0.0);
    }
    if let Some(time) = &words.time_line {
        at(TIME_GAP);
        let r = at(TEXT_LINE);
        label(p, &c(time), size::CAPTION, false, theme::TEXT_SECONDARY, r, HAlign::Left, VAlign::Middle, 0.0);
    }
    if let Some(frac) = words.progress {
        at(GAP);
        let r = at(PROGRESS_H);
        let track = Rect::new(r.x, r.y, layout.progress_w, PROGRESS_H);
        // the shared played-progress rail: the fill and the track are split at the fraction and never share a pixel
        plx_ui::widgets::progress_bar(p, track, PROGRESS_H * 0.5, PROGRESS_H, frac);
    }
    if let Some(next) = &words.next {
        at(GAP);
        let r = at(TEXT_LINE);
        label(p, &c(next), size::CAPTION, false, theme::TEXT_TERTIARY, r, HAlign::Left, VAlign::Middle, 0.0);
    }
    // The clock: right edge and foot on the block's own margins; it shows with the banner only.
    if let Some(clock) = &words.clock {
        let r = Rect::new(SCR_W - layout.x - 400.0, SCR_H - layout.bottom - TEXT_LINE, 400.0, TEXT_LINE);
        label(p, &c(clock), size::BODY, false, theme::TEXT_SECONDARY, r, HAlign::Right, VAlign::Baseline, 0.0);
    }
}

/// **The banner's alpha** `ms` after `until` (the screen's banner deadline): 1 while it is up, a
/// [`FADE_MS`] linear fade after, 0 once gone. `held` is the pipeline being busy, which keeps it up.
pub fn alpha(now: u32, until: u32, held: bool) -> f32 {
    if held || now < until {
        return 1.0;
    }
    let gone = now - until;
    if gone >= FADE_MS { 0.0 } else { 1.0 - gone as f32 / FADE_MS as f32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-25 21:00:00 UTC.
    const NINE: i64 = 1_795_640_400;
    const MIN: i64 = 60;

    fn channel(begin_min: i64, end_min: i64) -> Channel {
        Channel {
            sid: plx_plex::plex::ServerId::from_raw(0),
            dvr: "7".into(),
            channel: "C101".into(),
            vcn: "101".into(),
            name: "Family Channel".into(),
            thumb: String::new(),
            now: "The Long Programme".into(),
            now_thumb: String::new(),
            now_summary: String::new(),
            begins_at: NINE + begin_min * MIN,
            ends_at: NINE + end_min * MIN,
            later: Vec::new(),
        }
    }

    fn at(min: i64) -> Option<Wall> {
        Some(Wall::from_secs(NINE + min * MIN, 0))
    }

    /// The handoff's two time lines, the progress, "Next at" and the clock, for the airing in each phase.
    #[test]
    fn the_words_follow_the_airing_and_the_wall() {
        let _language = plx_os::i18n::language_on_this_thread_for_test(plx_os::i18n::Preference::En);
        // mid-airing: "21:00–22:00 · 22m left" at 21:38 (English words its clock faces on 12 hours)
        let w = words(Some(&channel(0, 60)), "", at(38));
        assert_eq!((w.number.as_str(), w.station.as_str(), w.title.as_str()), ("101", "Family Channel", "The Long Programme"));
        assert_eq!(w.time_line.as_deref(), Some("9:00\u{202f}PM\u{2013}10:00\u{202f}PM \u{b7} 22m left"));
        assert_eq!(w.next.as_deref(), Some("Next at 10:00\u{202f}PM"));
        assert_eq!(w.clock.as_deref(), Some("9:38\u{202f}PM"));
        assert!((w.progress.unwrap() - 38.0 / 60.0).abs() < 1e-3, "{:?}", w.progress);
        // within ten minutes of its start: the start is told in place of the time left
        let w = words(Some(&channel(37, 99)), "", at(38));
        assert_eq!(w.time_line.as_deref(), Some("9:37\u{202f}PM\u{2013}10:39\u{202f}PM \u{b7} Started 1 min ago"));
        assert_eq!(w.next.as_deref(), Some("Next at 10:39\u{202f}PM"));
    }

    /// Without a wall time the banner words only what the guide says; without a programme, no time, progress or
    /// "Next at"; without a channel, only the name it was given.
    #[test]
    fn the_words_degrade_without_a_clock_or_a_programme() {
        let _language = plx_os::i18n::language_on_this_thread_for_test(plx_os::i18n::Preference::En);
        let w = words(Some(&channel(0, 60)), "", None);
        assert_eq!((w.time_line, w.next, w.progress, w.clock), (None, None, None, None));
        assert_eq!(w.title, "The Long Programme");
        let mut idle = channel(0, 60);
        idle.now.clear();
        let w = words(Some(&idle), "", at(38));
        assert_eq!((w.title.as_str(), w.time_line, w.next, w.progress), ("", None, None, None));
        assert_eq!(w.clock.as_deref(), Some("9:38\u{202f}PM"), "the clock needs no programme");
        let w = words(None, "Station", at(38));
        assert_eq!((w.station.as_str(), w.number.as_str(), w.title.as_str()), ("Station", "", ""));
        // a programme that is over has no "Next at": the guide has not moved on yet
        let w = words(Some(&channel(0, 60)), "", at(61));
        assert_eq!(w.next, None);
    }

    /// Shown whole while the deadline is ahead (or the pipeline is busy), a .35 s linear fade after, then gone.
    #[test]
    fn the_banner_is_whole_until_its_time_then_fades_in_350_ms() {
        assert_eq!(alpha(999, 1_000, false), 1.0);
        assert_eq!(alpha(1_000, 1_000, false), 1.0);
        assert!((alpha(1_175, 1_000, false) - 0.5).abs() < 1e-6);
        assert_eq!(alpha(1_000 + FADE_MS, 1_000, false), 0.0);
        assert_eq!(alpha(9_999, 1_000, true), 1.0, "held while a zap is in flight or no picture is there yet");
        assert_eq!((SHOW_MS, FADE_MS), (4_000, 350));
    }

    /// The paint, read off the recorded runs: the block's words in reading order at the block's left, the title at the
    /// handoff's DISPLAY bold, the badge a rounded plate in the LIVE red, the progress track as wide as the layout says,
    /// the clock right-aligned at the right margin; and a banner at alpha 0 paints nothing at all.
    #[test]
    fn the_banner_paints_the_block_and_nothing_at_alpha_zero() {
        let _language = plx_os::i18n::language_on_this_thread_for_test(plx_os::i18n::Preference::En);
        let w = words(Some(&channel(0, 60)), "", at(38));
        let m = plx_ui::fixture::FixtureMeasure;
        let (w, m) = (&w, &m);
        let paint = |alpha: f32| move || draw(Painter::recording(), &LiveLayout::CLASSIC, w, alpha, m);
        let runs = plx_ui::draw_census::capture_runs(paint(1.0));
        let texts: Vec<&str> = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, [
            "LIVE", "101", "Family Channel", "The Long Programme",
            "9:00\u{202f}PM\u{2013}10:00\u{202f}PM \u{b7} 22m left", "Next at 10:00\u{202f}PM", "9:38\u{202f}PM",
        ]);
        let title = &runs[3];
        assert_eq!((title.size, title.bold, title.ink, title.at.x), (size::DISPLAY, true, theme::TEXT_PRIMARY, 96.0));
        assert_eq!(runs[0].ink, theme::LIVE_BADGE_INK, "the badge's ink");
        let clock = &runs[6];
        assert_eq!((clock.align, clock.at.x), (2, SCR_W - 96.0), "right-aligned at the right margin");
        let rrects: Vec<_> = plx_ui::draw_census::capture(paint(1.0)).into_iter().filter(|(tag, _)| *tag == 2).map(|(_, r)| r).collect();
        assert_eq!((rrects[0].x, rrects[0].h), (96.0, 32.0), "the badge plate, at the block's left");
        assert!(rrects.iter().any(|r| r.w == 560.0 && r.h == 6.0), "the progress track: {rrects:?}");
        assert!(plx_ui::draw_census::capture_runs(paint(0.0)).is_empty());
        assert!(plx_ui::draw_census::capture(paint(0.0)).is_empty());
    }

    /// A package that declares no `layout.live` stands where classic's banner does, and classic declares exactly that.
    #[test]
    fn the_layout_is_the_packages_else_classics() {
        assert_eq!(LiveLayout::read(None), LiveLayout::CLASSIC);
        let classic = crate::player_hud::loaded::classic_for_test();
        assert_eq!(classic.live(), LiveLayout::CLASSIC, "classic's hud.json declares the same placement the default holds");
        assert_eq!(
            LiveLayout::CLASSIC,
            LiveLayout { x: 96.0, bottom: 96.0, scrim_h: 520.0, title_w: 1200.0, progress_w: 560.0 },
            "the handoff's block: left 96 / bottom 96, a 520 px scrim, the title at most 1200 wide, a 560 x 6 progress bar"
        );
    }
}
