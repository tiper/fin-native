//! **A Live TV channel's tile** ([`Art::Channel`](super::Art::Channel)): the face the What's on page's
//! Recent row stands on, from the Live TV handoff's `SPEC-1` item 5. A flat plate
//! ([`theme::CHANNEL_PLATE`]), the channel's logo contained and centred on it, and the channel's number
//! at the top-left corner.
//!
//! **The logo is sized by the one logo rule, on its ink**: [`hero_logo::fit_ink`] at [`LogoRung::Channel`], a target
//! ink area contained in a 218 x 84 slot, so a wide wordmark and a square emblem carry comparable weight
//! (`hero_logo`'s module doc has the argument). **By its ink, not its canvas**: a channel's logo often comes
//! on a canvas much larger than the mark (a resizer pads it), so the request asks for the picture's ink box
//! ([`Derive::INK`](crate::art_fit::Derive::INK)) and the plate sizes THAT and draws only that
//! window of the texture ([`logo_quad`]), the mark centred in the slot. A picture the request never asked a box
//! for is drawn whole, as before; one with no visible texel at all ([`Ink::Clear`]) is drawn as no logo. The handoff also gives a dark mark a light backing; that
//! needs a darkness fact per logo, and nothing in the image path reads one, so it is not drawn.
//!
//! **A channel with no logo** (its `logo` path is empty, the source settled the fetch as a miss, or the picture is
//! entirely clear) shows its NAME in type in the logo's slot ([`hero_logo::draw_title`]): a plate with only a number reads as a
//! broken tile. A logo still on its way shows the plate alone, counted as a wait
//! ([`placeholder::note`]) like every other tile's skeleton.
//!
//! Everything is laid out on the tile's RESTING rect and grown with the card ([`grown`]), so a focus pop
//! magnifies the plate, logo and number as one object.
use super::{ArtImage, LegacyMeasure};
use crate::art_fit::{Ink, InkBox};
use crate::collection_tile::grown;
use crate::hero_logo::{self, LogoRung};
use crate::label::HAlign;
use crate::placeholder::{self, Reason};
use crate::text_view::TextView;
use crate::{card_shadow_params, theme, Painter, Rect};
use std::os::raw::c_int;

/// The size a channel's logo is asked for at: twice the 218 x 84 slot it is drawn in, the same margin
/// the other art keeps for the focus pop and the TV's own scaling. The request COVERS this box, so the
/// texture keeps the logo's own aspect (and its canvas: [`hero_logo::fit_ink`] contains the mark's ink box
/// in the slot, whatever the margin around it).
pub const LOGO_RES: (c_int, c_int) = (436, 168);

/// The number's inset from the plate's top-left corner (the handoff's 14 / 10).
const NUMBER_INSET: (f32, f32) = (14.0, 10.0);

/// What a channel tile says besides its logo texture.
#[derive(Clone, Copy)]
pub struct Face<'a> {
    /// The raw id of the server that holds the logo.
    pub sid: u16,
    /// The logo's source path; empty when the channel has none.
    pub logo: &'a str,
    /// The channel number as the lineup prints it.
    pub number: &'a str,
    /// The channel's name, drawn in the logo's place when there is no logo.
    pub name: &'a str,
}

/// Draw the tile at `r` (popped when focused), laid out on `rest`, its resting rect. `f` is the card's
/// pop factor ([`card_shadow_params`]'s), `image` the logo as [`resolve_card_art`](super::resolve_card_art)
/// resolved it this frame.
pub fn draw(p: Painter, rest: Rect, r: Rect, rad: f32, f: f32, face: &Face<'_>, image: ArtImage) {
    let ArtImage { tex, w: tw, h: th, ink, shown, .. } = image;
    // A flat plate has no texture pass to fold its shadow into, so the shadow is the one a tile casts
    // (`Painter::tex_carded`'s parameters), drawn under the plate.
    let (blur, off, a, _) = card_shadow_params(r.h, f, theme::CARD_SHADOW[3]);
    p.shadow(r, rad, blur, off, theme::with_a(theme::CARD_SHADOW, a));
    let waiting = tex == 0 && !face.logo.is_empty() && !crate::tex::failed_on(face.sid, face.logo, LOGO_RES.0, LOGO_RES.1, true);
    if waiting {
        placeholder::note(p, Reason::CardGround, face.logo);
    }
    p.rrect_sheened(r, rad, theme::CHANNEL_PLATE);
    if waiting && placeholder::sentinel_fill(p, r, rad) {
        return;
    }
    grown(p, rest, r, |p| {
        if tex != 0 && ink != Some(Ink::Clear) {
            let (at, uv) = logo_quad(rest, tw, th, if let Some(Ink::Box(b)) = ink { Some(b) } else { None });
            p.alpha(shown).tex_uv(tex, uv, at, 0.0, theme::TINT_WHITE);
        } else if !waiting {
            draw_name(p, rest, face.name);
        }
        if !face.number.is_empty() {
            let at = Rect::new(rest.x + NUMBER_INSET.0, rest.y + NUMBER_INSET.1, rest.w - 2.0 * NUMBER_INSET.0, 0.0);
            TextView::new(face.number, theme::size::MICRO, theme::TEXT_TERTIARY).draw(p, at);
        }
    });
}

/// Where the logo goes on the plate `rest`, and which window of its `tw` x `th` texture goes there: the
/// mark's size ([`hero_logo::fit_ink`], by its `ink` box when it has one) centred on the plate, and the
/// texture window that is the mark (with a texel or two of its margin) and not the canvas around it. Pure.
fn logo_quad(rest: Rect, tw: f32, th: f32, ink: Option<InkBox>) -> (Rect, [f32; 4]) {
    let ((w, h), uv) = hero_logo::fit_ink(LogoRung::Channel, tw, th, ink, theme::logo::CHANNEL_W_MAX);
    (Rect::new(rest.cx() - w * 0.5, rest.cy() - h * 0.5, w, h), uv)
}

/// The channel's name in the logo's slot, one line, centred, its baseline half a cap under the plate's
/// middle so the line reads as centred (`draw_title` anchors the BASELINE on the band's foot at this rung).
fn draw_name(p: Painter, rest: Rect, name: &str) {
    if name.is_empty() {
        return;
    }
    let measure = LegacyMeasure;
    let foot = rest.cy() + plx_machine::machine::Measure::cap_h(&measure, theme::size::LABEL) * 0.5;
    let band = Rect::new(rest.cx() - theme::logo::CHANNEL_W_MAX * 0.5, foot - theme::logo::CHANNEL_H_MAX,
        theme::logo::CHANNEL_W_MAX, theme::logo::CHANNEL_H_MAX);
    hero_logo::draw_title(p, LogoRung::Channel, name, band, HAlign::Center, &measure);
}

#[cfg(test)]
mod tests {
    use super::*;

    const REST: Rect = Rect { x: 100.0, y: 200.0, w: 250.0, h: 140.0 };

    /// The plate draws the mark's window of the texture, sized by its ink and centred on the plate.
    #[test]
    fn the_plate_draws_the_ink_window_centred_at_the_ink_size() {
        let ink = InkBox { x: 100, y: 175, w: 200, h: 50 };
        let (at, uv) = logo_quad(REST, 400.0, 400.0, Some(ink));
        assert_eq!(uv, [98.0 / 400.0, 173.0 / 400.0, 204.0 / 400.0, 54.0 / 400.0], "the mark and two texels of margin");
        assert!((at.w - 204.0).abs() < 0.01 && (at.h - 54.0).abs() < 0.01, "{at:?}");
        assert!((at.cx() - REST.cx()).abs() < 0.01 && (at.cy() - REST.cy()).abs() < 0.01, "centred on the plate: {at:?}");
        // a texture that came with no box is drawn whole, as before
        let (at, uv) = logo_quad(REST, 400.0, 400.0, None);
        assert_eq!(uv, plx_gfx::gfx::UV_FULL);
        assert!((at.w - 84.0).abs() < 0.01 && (at.h - 84.0).abs() < 0.01, "{at:?}");
    }

    /// Through the real draw: the one textured quad is the ink's size.
    #[test]
    fn a_drawn_channel_tile_paints_one_logo_quad_at_the_ink_size() {
        let image = ArtImage { tex: 9, w: 400.0, h: 400.0, backdrop: 0, ink: Some(Ink::Box(InkBox { x: 100, y: 175, w: 200, h: 50 })), shown: 1.0 };
        let face = Face { sid: 1, logo: "/logos/1.png", number: "", name: "One" };
        let p = Painter::recording();
        let quads = crate::draw_census::capture_tex(|| draw(p, REST, REST, 14.0, 0.0, &face, image));
        let logo: Vec<_> = quads.iter().filter(|(t, ..)| *t == 9).collect();
        assert_eq!(logo.len(), 1, "{quads:?}");
        let r = logo[0].1;
        assert!((r.w - 204.0).abs() < 0.5 && (r.h - 54.0).abs() < 0.5, "{r:?}");
    }

    /// A logo with no visible texel (`Ink::Clear`) is not drawn as an invisible quad over an unnamed plate: the
    /// channel's NAME stands in its place, as for a channel with no logo. A logo the request never asked a box
    /// for (`None`) is still drawn whole.
    #[test]
    fn a_clear_logo_draws_the_channel_name_and_no_quad() {
        let face = Face { sid: 1, logo: "/logos/1.png", number: "", name: "One" };
        let drawn = |ink| {
            let image = ArtImage { tex: 9, w: 400.0, h: 400.0, backdrop: 0, ink, shown: 1.0 };
            let p = Painter::recording();
            let mut quads = Vec::new();
            let words = crate::draw_census::capture_text(|| {
                quads = crate::draw_census::capture_tex(|| draw(p, REST, REST, 14.0, 0.0, &face, image));
            });
            (quads.iter().filter(|(t, ..)| *t == 9).count(), words.iter().any(|(s, _)| s == "One"))
        };
        assert_eq!(drawn(Some(Ink::Clear)), (0, true), "no quad, the name");
        assert_eq!(drawn(None), (1, false), "not asked: the whole texture, no name");
        assert_eq!(drawn(Some(Ink::Box(InkBox { x: 0, y: 0, w: 400, h: 400 }))), (1, false), "a mark: the quad, no name");
    }
}
