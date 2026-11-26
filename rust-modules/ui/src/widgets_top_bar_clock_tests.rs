//! **The bar's clock** (Live TV handoff SPEC-1 item 2: "right edge on --margin-x (96), centred on the
//! top bar. BODY 28 regular, --text-secondary, tabular figures, soft text shadow. Never focusable"):
//! where its box stands, what the draw paints, and how the strip yields to it.

use super::*;
use crate::draw_census::capture_runs;

const FACE: &CStr = c"21:38";

fn drawn(clock: ClockRead<'_>) -> Vec<crate::draw_census::Run> {
    let _serial = plx_base::testlock::serial();
    capture_runs(|| draw_top_bar_clock(Painter::recording(), clock))
}

/// The box: its right edge is the safe frame's margin (96 in from 1920), and it is as tall as, and
/// stands on, the bar's pill row (y 62, h 60), so its centre is the bar's: 92.
#[test]
fn the_clock_box_ends_on_the_margin_and_is_centred_on_the_bar() {
    let frame = top_bar_clock_frame(76.0);
    assert_eq!(frame.x + frame.w, 1824.0, "the right edge is SCR_W - MARGIN_X");
    assert_eq!(frame.x + frame.w, crate::consts::SCR_W - crate::consts::MARGIN_X);
    assert_eq!((frame.y, frame.h), (62.0, 60.0), "the bar's pill row: --top-bar-y, --control-h");
    assert_eq!(frame.y + frame.h * 0.5, 92.0, "centred on the bar");
    assert_eq!((frame.y, frame.h), (TOP_BAR_Y, TAB_PILL_H));
}

/// What the draw paints: BODY 28, regular, `TEXT_SECONDARY`, anchored RIGHT on 1824, its cap band
/// centred on 92, with a soft shadow run (the caption's token, one pixel low) under it.
#[test]
fn the_clock_is_body_regular_text_secondary_right_aligned_with_a_soft_shadow() {
    let runs = drawn(ClockRead { text: FACE, w: 76.0 });
    assert_eq!(runs.len(), 2, "the shadow and the ink: {runs:?}");
    let (shadow, ink) = (&runs[0], &runs[1]);
    assert_eq!(ink.text, "21:38");
    assert_eq!(ink.size, 28);
    assert_eq!(ink.size, crate::theme::size::BODY);
    assert!(!ink.bold, "regular");
    assert_eq!(ink.ink, crate::theme::TEXT_SECONDARY);
    assert_eq!(ink.align, 2, "right-aligned");
    assert_eq!(ink.at.x, 1824.0, "its right edge is the margin");
    let (cap_top, baseline) = plx_gfx::text::text_cap_band(28, 0);
    assert_eq!(ink.at.y + (cap_top + baseline) * 0.5, 92.0, "the cap band is centred on the bar");

    assert_eq!(shadow.text, ink.text);
    assert_eq!(shadow.ink, crate::caption_policy::CAPTION_SHADOW, "the caption's soft shadow token");
    assert_eq!((shadow.size, shadow.align, shadow.bold), (ink.size, ink.align, ink.bold));
    assert_eq!(shadow.at.x, ink.at.x);
    assert_eq!(shadow.at.y - ink.at.y, crate::caption_policy::CAPTION_SHADOW_DY);
}

/// The face's digits share one advance (the face's tabular figures), which is what keeps a
/// right-aligned run still when the minute changes: the draw anchors on the RIGHT edge whatever the
/// measured width, so two faces of one width stand on the same left edge, and the box follows the width.
#[test]
fn the_right_edge_does_not_depend_on_the_faces_width() {
    for (text, w) in [(c"9:05", 62.0), (c"21:38", 76.0), (c"12:00\u{202f}AM", 130.0)] {
        let runs = drawn(ClockRead { text, w });
        assert!(runs.iter().all(|run| run.at.x == 1824.0), "{text:?}: {runs:?}");
        assert_eq!(top_bar_clock_frame(w).x + w, 1824.0);
    }
}

/// **The strip yields to the clock, never the clock to the strip.** The track is centred, so the widest
/// face the clock can wear (`TOP_BAR_CLOCK_MAX_W`, measured 146 px for Spanish and Catalan "12:00 p. m."
/// with the shipped font; English 12 h is 129, Portuguese 141, any 24 h face 80) is folded into the
/// clearance behind `TAB_VIEW_MAX`: the widest track ends `BAND_AIR` short of the widest clock's left
/// edge, so there is always room for the face and no room rule that could hide it.
#[test]
fn the_strip_yields_to_the_widest_face_the_clock_can_wear() {
    assert_eq!(TOP_BAR_CLOCK_MAX_W, 146.0, "the measured width of the widest shipped 12-hour face");
    assert_eq!(TAB_CLOCK_CLEAR, 96.0 + 146.0 + BAND_AIR, "margin + widest face + air");
    assert!(TAB_CHIP_CLEAR < TAB_CLOCK_CLEAR, "the clock's end is the wider one, so it sets the symmetric clearance");
    assert_eq!(TAB_SIDE_CLEAR, TAB_CLOCK_CLEAR);
    let widest_track = TAB_VIEW_MAX + 2.0 * TAB_TRACK_PAD;
    let track_right = (crate::consts::SCR_W + widest_track) * 0.5;
    assert_eq!(track_right, 1662.0, "the widest track's right edge");
    assert_eq!(top_bar_clock_frame(TOP_BAR_CLOCK_MAX_W).x - track_right, BAND_AIR, "exactly one rung of air to the widest face");
    for face_w in [80.0, 129.0, 141.0, 146.0] {
        assert!(track_right + BAND_AIR <= top_bar_clock_frame(face_w).x, "a {face_w} px face clears the widest track");
    }
    // the strip is what gave way: it scrolls past a viewport 140 px narrower than before the clock
    assert_eq!(widest_track, 1404.0);
}
