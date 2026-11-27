//! Detail's continuous shade and text contrast over bright and white artwork.

use super::*;
#[allow(unused_imports)]
use super::test_support::*;

#[test]
fn the_trailer_shade_clears_the_space_left_by_the_title() {
    use crate::landing_hero::{detail_shade, PREVIEW_FIELD};

    let still = detail_shade(1.0, 1.0, 1.0, 0.0);
    assert!(still.alpha(400.0, 480.0) > 0.5);
    let mut previous = detail_shade(1.0, PREVIEW_FIELD, 1.0, 0.0).alpha(400.0, 480.0);
    for progress in [0.25, 0.5, 0.75, 1.0] {
        let alpha = detail_shade(1.0, PREVIEW_FIELD, 1.0, progress).alpha(400.0, 480.0);
        assert!(alpha <= previous, "the vacated title area should lighten as the logo moves");
        previous = alpha;
    }
    let preview = detail_shade(1.0, PREVIEW_FIELD, 1.0, 1.0);
    for y in [260.0, 360.0, 480.0, 520.0] {
        assert_eq!(preview.alpha(400.0, y), 0.0, "the gap under the compact logo at y={y}");
    }
    let logo_shade = preview.alpha(300.0, 85.0);
    for x in [0.0, 576.0, 960.0, 1920.0] {
        assert_eq!(preview.alpha(x, 85.0), logo_shade, "the top fade has no patch behind the logo");
    }
    assert!((0.4..0.55).contains(&logo_shade), "the shade supports the compact logo without a dark patch");
    assert!(preview.alpha(300.0, 175.0) < logo_shade);
    assert!(preview.alpha(1200.0, 700.0) > 0.6, "the lower text keeps its shade");
}

/// Weakest contrast over a text box, sampled through the same shade the page uses.
fn detail_worst(
    shade: &WashScrim,
    ink: [f32; 4],
    art: f32,
    (x0, x1): (f32, f32),
    (y0, y1): (f32, f32),
) -> (f32, f32, f32, f32) {
    let mut worst = (f32::INFINITY, x0, y0, 0.0);
    for ix in 0..=16 {
        for iy in 0..=4 {
            let (x, y) = (x0 + (x1 - x0) * ix as f32 / 16.0, y0 + (y1 - y0) * iy as f32 / 4.0);
            let total = shade.alpha(x, y);
            let c = contrast_over_art(ink, art, total);
            if c < worst.0 {
                worst = (c, x, y, total);
            }
        }
    }
    worst
}

/// Check the full width of each text box, including the highest and widest credits.
#[test]
fn the_hero_text_reads_over_bright_artwork() {
    use crate::consts::{MARGIN_X, SCR_W};
    let hc = crate::detail_layout::hero_chain(
        // a two-line blurb — the shape `hero_chain`'s own doc is tuned on. Measuring is the one
        // thing the host cannot do, so the height is quoted, not computed.
        34.0, true, &crate::fixture::FixtureMeasure,
    );
    let det_col_r = MARGIN_X + crate::detail_layout::HERO_TEXT_W; // the synopsis' wrap edge
    let people_l = SCR_W - MARGIN_X - crate::detail_layout::PEOPLE_W;
    let people_top = crate::detail_layout::people_top(hc.btn_y, crate::detail_layout::PEOPLE_MAX_LINES, theme::size::CAPTION as f32);
    let shade = crate::landing_hero::detail_shade(1.0, 1.0, 1.0, 0.0);

    // (label, x run, y span, ink, floor over BRIGHT art, floor over BLOWN WHITE)
    let rows: [(&str, (f32, f32), (f32, f32), [f32; 4], f32, f32); 5] = [
        ("detail title", (MARGIN_X, det_col_r), (detail_title_cap_top(), crate::detail_layout::TITLE_BOTTOM), theme::TEXT_PRIMARY, 3.0, 2.5),
        ("detail meta", (MARGIN_X, 700.0), (hc.meta_y, hc.meta_y + BADGE_H), theme::TEXT_SECONDARY, 3.0, 2.5),
        ("detail synopsis", (MARGIN_X, det_col_r), (hc.syn_y, hc.syn_y + 34.0), theme::TEXT_READING, 3.0, 2.5),
        ("detail facts", (MARGIN_X, 1270.0), (hc.facts_y, hc.facts_y + theme::hero::FACTS_H), crate::detail_layout::FACTS_INK, 3.0, 2.5),
        (
            "detail people (top line)",
            (people_l, SCR_W - MARGIN_X),
            (people_top, people_top + crate::detail_layout::PEOPLE_LEAD),
            crate::detail_layout::PEOPLE_INK,
            3.0,
            2.5,
        ),
    ];
    for (label, xs, ys, ink, min_bright, min_white) in rows {
        for (art, floor) in [(0.85f32, min_bright), (1.0, min_white)] {
            let (after, x, y, total) = detail_worst(&shade, ink, art, xs, ys);
            let before = contrast_over_art(ink, art, crate::detail_layout::base_scrim_a(y, 1.0));
            eprintln!("{label:<26} art {art:.2}: {after:5.2}:1 (floor {floor}) at ({x:.0},{y:.0}) total alpha {total:.3}; ramp alone {before:.2}:1");
            assert!(
                after >= floor,
                "{label} at ({x:.0},{y:.0}) over art {art}: {after:.2}:1, under its {floor}:1 floor (total alpha {total:.3})"
            );
            assert!(after >= before, "{label} over art {art}: the shade made it WORSE ({before:.2} → {after:.2})");
        }
    }
    // Video-bound rows. The compact logo and the synopsis still need contrast, including when
    // no ratings row pushes the synopsis down.
    let ceiling = crate::landing_hero::PREVIEW_FIELD;
    let preview = crate::landing_hero::detail_shade(1.0, ceiling, 1.0, 1.0);
    let synopsis = crate::detail_layout::hero_chain(34.0, false, &crate::fixture::FixtureMeasure).syn_y;
    let corner_run = (crate::detail_layout::PREVIEW_LOGO_X, crate::detail_layout::PREVIEW_LOGO_X + crate::detail_layout::PREVIEW_LOGO_MAX_W);
    let corner_span = (crate::detail_layout::PREVIEW_LOGO_Y, crate::detail_layout::PREVIEW_LOGO_Y + crate::hero_logo::band_h(crate::hero_logo::LogoRung::Compact));
    for (art, floor) in [(0.85f32, 3.0f32), (1.0, 2.5)] {
        let (c, x, y, total) = detail_worst(&preview, theme::TEXT_PRIMARY, art, corner_run, corner_span);
        eprintln!("detail logo in the trailer corner art {art:.2}: {c:5.2}:1 (floor {floor}) at ({x:.0},{y:.0}) total alpha {total:.3}");
        assert!(c >= floor, "the risen logo over art {art}: {c:.2}:1, under its {floor}:1 floor");
        let (c, ..) = detail_worst(&preview, theme::TEXT_READING, art, (MARGIN_X, det_col_r), (synopsis, synopsis + 34.0));
        assert!(c >= floor, "the preview synopsis over art {art}: {c:.2}:1, under its {floor}:1 floor");
    }
}

/// Where, walking right along row `y`, the shade first falls below `level`, in 1 px steps.
fn contour_x(shade: &WashScrim, y: f32, level: f32) -> f32 {
    (0..=1920).map(|x| x as f32).find(|&x| shade.alpha(x, y) < level).unwrap_or(1920.0)
}

/// Where, walking down column `x` from the top, the shade first reaches `level`.
fn contour_y(shade: &WashScrim, x: f32, level: f32) -> f32 {
    (0..=1080).map(|y| y as f32).find(|&y| shade.alpha(x, y) >= level).unwrap_or(1080.0)
}

/// **The hero's side shade is a curve, not a box** (discussion #1460: the first retail tester saw
/// Detail's shade as a dark rectangle with a top edge and a right edge, "almost like there's an
/// issue with the TV's backlight"). A side shade that is a product of a horizontal and a vertical
/// ramp has iso-lines that run straight along both axes: its right edge is one vertical line all
/// the way down the copy, and its top edge one horizontal line across the left column, which the
/// eye reads as the two edges of a plate. Held here as a property of the edges:
///
/// - **the right edge leans out as it goes down.** On each row the edge is where the shade falls
///   halfway from its value at the left (the side shade) to its value at the right (the row's
///   background ramp, which on Detail already darkens the copy's lower rows). Walking down the
///   copy's rows, that place must move RIGHT. A separable field leaves it where it is, or (on
///   Detail, as the background ramp rises) moves it left, which is the box's vertical side;
/// - **the top edge drops as it goes right.** Where the shade reaches half its peak must move
///   DOWN between the left edge and the copy's right part, not stay one horizontal line.
///
/// Detail's only: Home keeps the box (`theme::shade::HOME_SIDE_X` says why).
#[test]
fn the_side_shade_falls_off_in_a_curve_not_a_box_corner() {
    use crate::landing_hero::detail_shade;
    let cases = [("detail", detail_shade(1.0, 1.0, 1.0, 0.0), theme::shade::DETAIL_SIDE_A, (420.0, 540.0))];
    for (name, shade, peak, (upper, lower)) in cases {
        let edge = |y: f32| {
            let (side, ground) = (shade.alpha(0.0, y), shade.alpha(1919.0, y));
            assert!(side - ground > peak * 0.25, "{name}: row y={y} is not on the side shade ({side} over {ground})");
            contour_x(&shade, y, (side + ground) * 0.5)
        };
        let (right_high, right_low) = (edge(upper), edge(lower));
        let half = peak * 0.5;
        let (top_left, top_inner) = (contour_y(&shade, 100.0, half), contour_y(&shade, 500.0, half));
        eprintln!("{name}: right edge x={right_high} (y={upper}) x={right_low} (y={lower}); top edge y={top_left} (x=100) y={top_inner} (x=500)");
        assert!(right_low - right_high >= 60.0,
            "{name}: the shade's right edge is a straight vertical line (x={right_high} on y={upper}, x={right_low} on y={lower})");
        assert!(top_inner - top_left >= 30.0,
            "{name}: the shade's top edge is a straight horizontal line (y={top_left} on x=100, y={top_inner} on x=500)");
    }
}

/// Detail's side shade on its own, with its peak: the page's field with the bottom ramp at zero,
/// which leaves exactly the side term (`EdgeShade::at` joins a zero ramp to nothing). Home keeps the
/// box (`theme::shade::HOME_SIDE_X` says why), so these two tests grade Detail only.
fn side_shades() -> [(&'static str, WashScrim, f32); 1] {
    [("detail", crate::landing_hero::detail_shade(1.0, 1.0, 0.0, 0.0), theme::shade::DETAIL_SIDE_A)]
}

/// **The side shade has no edge anywhere: it changes slowly in every direction.** Discussion #1460:
/// the tester read a crisp fall of the shade as a backlight fault, so the side term may never change
/// faster than a smoothstep across 375 px would at its steepest (`1.5 * peak / 375`, written
/// `1.6 * peak / 400`): the band is at least ~375 px wide wherever it falls. The central difference
/// below can only read under a smoothstep's true peak, so the bound is the feather, not sampling slack. Graded on the gradient's MAGNITUDE over the whole frame,
/// so a band that is wide across the copy but narrow up the left edge (the old box's 280 px
/// vertical ramp) or narrow normal to a curve (a thin feather between two ellipses) both fail.
#[test]
fn the_side_shade_has_no_edge_anywhere() {
    for (name, shade, peak) in side_shades() {
        let limit = 1.6 * peak / 400.0;
        let mut worst = (0.0f32, 0.0, 0.0);
        for y in (1..1080).step_by(4) {
            for x in (1..1920).step_by(4) {
                let (x, y) = (x as f32, y as f32);
                let gx = (shade.alpha(x + 1.0, y) - shade.alpha(x - 1.0, y)) * 0.5;
                let gy = (shade.alpha(x, y + 1.0) - shade.alpha(x, y - 1.0)) * 0.5;
                let g = (gx * gx + gy * gy).sqrt();
                if g > worst.0 {
                    worst = (g, x, y);
                }
            }
        }
        let (g, x, y) = worst;
        eprintln!("{name}: steepest {:.5}/px at ({x},{y}), limit {limit:.5} (a {:.0} px smoothstep)", g, 1.5 * peak / g);
        assert!(g <= limit, "{name}: the side shade changes {g:.5}/px at ({x},{y}), over {limit:.5}: a visible edge");
    }
}

/// **The side shade covers no more of the art's top-left than the box it replaced.** The tester's
/// complaint was a shade too strong over the picture, so softening it may not buy legibility by
/// climbing higher: over the top-left (x ≤ 600, y ≤ 250, where the title's art usually is) the
/// side term is no darker anywhere than the old separable field's (`main` before #1460's fix: a
/// smoothstep down from y0 to y1 times one across from x0 to x1, quoted here as it was).
#[test]
fn the_side_shade_covers_no_more_of_the_top_left_than_the_box_did() {
    let smooth = |t: f32| { let t = t.clamp(0.0, 1.0); t * t * (3.0 - 2.0 * t) };
    let old = |x: f32, y: f32, peak: f32, [x0, x1]: [f32; 2], [y0, y1]: [f32; 2]| {
        peak * (1.0 - smooth((x - x0) / (x1 - x0))) * smooth((y - y0) / (y1 - y0))
    };
    let boxes = [([600.0, 1700.0], [120.0, 400.0])];
    for ((name, shade, peak), (bx, by)) in side_shades().into_iter().zip(boxes) {
        for y in (0..=250).step_by(2) {
            for x in (0..=600).step_by(4) {
                let (x, y) = (x as f32, y as f32);
                let (now, was) = (shade.alpha(x, y), old(x, y, peak, bx, by));
                assert!(now <= was + 1e-6, "{name}: ({x},{y}) is {now:.3}, darker than the box's {was:.3}");
            }
        }
    }
}
