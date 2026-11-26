//! **A Live TV channel's tile** ([`Art::Channel`](super::Art::Channel)): the face the What's on page's
//! Recent row stands on, from the Live TV handoff's `SPEC-1` item 5. A flat plate
//! ([`theme::CHANNEL_PLATE`]), the channel's logo contained and centred on it, and the channel's number
//! at the top-left corner.
//!
//! **The logo is sized like every other logo here**: [`hero_logo::fit`] at [`LogoRung::Channel`], a target
//! ink area contained in a 218 x 84 slot, so a wide wordmark and a square emblem carry comparable weight
//! (`hero_logo`'s module doc has the argument). The handoff also gives a dark mark a light backing; that
//! needs a darkness fact per logo, and nothing in the image path reads one, so it is not drawn.
//!
//! **A channel with no logo** (its `logo` path is empty, or the source settled the fetch as a miss) shows
//! its NAME in type in the logo's slot ([`hero_logo::draw_title`]): a plate with only a number reads as a
//! broken tile. A logo still on its way shows the plate alone, counted as a wait
//! ([`placeholder::note`]) like every other tile's skeleton.
//!
//! Everything is laid out on the tile's RESTING rect and grown with the card ([`grown`]), so a focus pop
//! magnifies the plate, logo and number as one object.
use super::{ArtImage, LegacyMeasure};
use crate::collection_tile::grown;
use crate::hero_logo::{self, LogoRung};
use crate::label::HAlign;
use crate::placeholder::{self, Reason};
use crate::text_view::TextView;
use crate::{card_shadow_params, theme, Painter, Rect};
use std::os::raw::c_int;

/// The size a channel's logo is asked for at: twice the 218 x 84 slot it is drawn in, the same margin
/// the other art keeps for the focus pop and the TV's own scaling. The request COVERS this box, so the
/// texture keeps the logo's own aspect ([`hero_logo::fit`] contains it again).
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
    let ArtImage { tex, w: tw, h: th, shown, .. } = image;
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
        if tex != 0 {
            let (w, h) = hero_logo::fit(LogoRung::Channel, tw, th, theme::logo::CHANNEL_W_MAX);
            let at = Rect::new(rest.cx() - w * 0.5, rest.cy() - h * 0.5, w, h);
            p.alpha(shown).tex(tex, at, 0.0, theme::TINT_WHITE);
        } else if !waiting {
            draw_name(p, rest, face.name);
        }
        if !face.number.is_empty() {
            let at = Rect::new(rest.x + NUMBER_INSET.0, rest.y + NUMBER_INSET.1, rest.w - 2.0 * NUMBER_INSET.0, 0.0);
            TextView::new(face.number, theme::size::MICRO, theme::TEXT_TERTIARY).draw(p, at);
        }
    });
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
