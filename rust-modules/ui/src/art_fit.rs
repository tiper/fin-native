//! `art_fit` -- how a card's picture meets its tile when the two do not share a shape.
//!
//! A tile's art has always been COVERED: the picture fills the tile and the overflow is cropped
//! ([`crate::widgets::art_uv`]). That is right for a poster in a poster tile and for the art the
//! server made to size, and wrong for art that is not poster-shaped at all: a square programme image
//! in Live TV's On now row lost its sides and its baked-in title ("Space: New Frontiers" was cut off).
//! The owner's decision (2026-11-26, over PR #1557's default Q11): "square should fit with blurred bg".
//!
//! [`ArtFit`] is that choice, declared on the row ([`crate::card_row::RowStyle::fit`]) and read by the
//! shared card draw, never decided by a screen:
//!
//! - [`ArtFit::Cover`]: today's fit, for every row but Live TV's posters.
//! - [`ArtFit::ContainOnBlur`]: a picture within [`POSTER_FIT_TOLERANCE`] of the tile's own aspect is
//!   still covered (a 2:3 poster draws exactly as before). Any other picture is CONTAINED: the whole
//!   image, centred, its aspect kept, over a backdrop that is the same image cover-filled to the tile
//!   and blurred, and dimmed ([`crate::theme::ART_BACKDROP_DIM`]) so the contained image reads. **A
//!   picture with transparency (a channel's logo, a PNG with a clear ground) gets no blurred copy**: a
//!   blur of a mark on a clear ground is a grey wash with the mark's colours smeared into bands behind
//!   it. It is contained on the same flat ground the Recent channel plates stand on
//!   ([`crate::theme::CHANNEL_PLATE`]), carried as a one-texel copy so the tile draws exactly as it
//!   does over a blur. The
//!   tile's rounded corners, shadow, focus scale and resume bar are the tile's, and a FOCUSED tile
//!   is lit (rim glow, sheen crown, gloss) as one card pass around both layers, exactly as a covered
//!   picture is. At rest the picture is a second quad over the copy's card pass, so its square edge
//!   covers the constant 1px sheen along the sides where it touches the tile.
//!
//! [`place`] is that decision, pure over the source size and the tile rect.
//!
//! **The blur is made once per image, never per frame.** [`backdrop_of`] reduces the decoded
//! picture to a tiny blurred copy (a few texels on the short side) when the picture arrives, on the
//! main thread in the poster adapter's `drain_decoded`, from at most [`TAPS_PER_AXIS`]^2 taps per
//! output texel plus one 3 x 3 box pass over the tiny result. That is a few thousand taps over a
//! ~375 KB buffer: **unmeasured**, estimated at a few hundred microseconds on the television's
//! ARMv7 (the strided reads miss the cache more than the arithmetic costs), once per image.
//!
//! **A request asks for the copy** ([`Derive::Copy`], carried by the request that names the art:
//! [`crate::tex::Source::probe`]), so only art whose request declares a copy pays for one: a poster
//! under a fit that contains ([`ArtFit::derive_request`]) and the Live TV art ground's programme still
//! ([`crate::art_ground::SPEC`], a full-screen copy it draws as one quad). Never a logo or a headshot on a row, a
//! backdrop, or a Cover row's poster. The copy rides in
//! [`crate::tex::Decoded::backdrop`], is uploaded with the image and lives and dies in the same
//! cache entry ([`crate::tex::TexCache`]); drawing a tile is two textured quads (one pass when
//! focused: [`crate::Painter::tex_carded_inset`]) and no render pass. The GPU upscales the copy
//! bilinearly, which is the rest of the blur. A poster's copy is [`POSTER_BACKDROP_SIDE`] x up to
//! four times that x 4 bytes (1 KiB for a square, 1.75 KiB for 16:9, 4 KiB at the most) beside the
//! picture's own ~375 KB. The size is the request's, so another surface (a full-screen blurred
//! ground) asks for the copy size that suits it.
//!
//! **A request can also ask for the picture's INK BOX** ([`Derive::Ink`], [`Ink`]): the bounding
//! box of the texels that are not clear. A logo is often delivered on a canvas much larger than its mark (a
//! resizer's `fit=contain` pads it to the box asked for, a studio's own file keeps a margin), and a
//! size rule that reads the canvas sizes the margin, not the mark ([`crate::hero_logo::fit_ink`]). It is
//! found by one pass over the alpha channel, in [`attach_derived`] beside the copy, once per image, and
//! rides in [`crate::tex::Decoded::ink`] like the copy does. Only a Live TV channel plate's logo asks. A picture with no
//! visible texel answers [`Ink::Clear`], which is not the same as not having asked.
use crate::tex::Decoded;
use crate::{theme, Crop, Rect};
use plx_gfx::gfx::UV_FULL;

/// How far a picture's aspect may sit from its tile's and still be COVERED rather than contained:
/// the picture's aspect over the tile's lies in `1 +/- 0.08`. A 2:3 tile takes 0.61 - 0.72 as its own,
/// which holds every real poster (the server's 2:3 artwork, a 1000x1500 scan, a cover a few pixels
/// off) and nothing a viewer would call square or landscape. The edge is inclusive.
///
/// Cropping within the tolerance loses at most 8% of one axis, split evenly over both sides; beyond
/// it the crop starts to cut into titles and faces, which is the defect this exists to end.
pub const POSTER_FIT_TOLERANCE: f32 = 0.08;

/// Texels on the short side of a POSTER's blurred copy. The GPU's bilinear upscale of this to a
/// 250 x 375 tile spreads each texel over ~15 px, which is the blur's radius: 16 reads as a colour
/// field with no recognisable shape, which is what a backdrop behind a contained picture should be.
pub const POSTER_BACKDROP_SIDE: u8 = 16;

/// How many times the short side the long side of a copy may be: a panorama is not given a wider
/// copy than it can use.
const LONG_SIDE_RATIO: usize = 4;

/// The most taps [`backdrop_of`] takes per output texel along one axis. A 667 x 375 picture reduced
/// to 28 x 16 spans ~24 source texels per output texel; sampling 4 x 4 of them on an even grid is
/// plenty for a picture that is blurred twice more (the 3 x 3 pass, then the GPU's upscale), and
/// bounds the work to `28 * 16 * 16` taps whatever the picture's size.
const TAPS_PER_AXIS: usize = 4;

/// How a tile's art meets its box ([`crate::card_row::RowStyle::fit`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ArtFit {
    /// Fill the tile and crop the overflow: every row but Live TV's posters.
    #[default]
    Cover,
    /// Cover a picture of the tile's own shape; contain any other over its own blurred copy, or over a
    /// flat ground if it has transparency.
    ContainOnBlur,
}

impl ArtFit {
    /// What a request for art drawn under this fit asks for: nothing under
    /// [`Cover`](Self::Cover), a poster's blurred copy under [`ContainOnBlur`](Self::ContainOnBlur) (the only
    /// tile that contains is a poster tile).
    pub const fn derive_request(self) -> Option<Derive> {
        match self {
            Self::Cover => None,
            Self::ContainOnBlur => Some(Derive::POSTER),
        }
    }
}

/// **What a request for a picture asks the source to derive from it**: a blurred copy ([`CopySpec`]) or its ink
/// box ([`Ink`]). The request declares it ([`crate::tex::Source::probe`]), so the producer never guesses from a
/// size which consumers will draw it; it is part of the art's identity in the source (the same art asked for with
/// and without is two entries), and `Copy`/`Eq` so it can be. A request that asks for nothing carries `None`
/// instead of a `Derive`: there is no value of this type that asks for nothing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Derive {
    /// A blurred copy of the picture, or a flat ground for one with transparency.
    Copy(CopySpec),
    /// The picture's [`Ink`] box ([`ink_of`]), carried in [`crate::tex::Decoded::ink`]: a Live TV channel plate's
    /// logo, which the plate sizes by its mark and not by the canvas it came on ([`crate::hero_logo::fit_ink`]).
    Ink,
}

impl Derive {
    /// A poster tile's copy ([`CopySpec::POSTER`]).
    pub const POSTER: Self = Self::Copy(CopySpec::POSTER);
    /// A channel plate's logo asks for the ink box ([`Derive::Ink`]).
    pub const INK: Self = Self::Ink;

    /// The copy this asks for, if it asks for one.
    pub const fn copy(self) -> Option<CopySpec> {
        match self {
            Self::Copy(spec) => Some(spec),
            Self::Ink => None,
        }
    }
}

/// The blurred copy ([`Derive::Copy`]) a request asks the source to make of a picture.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CopySpec {
    /// Texels on the copy's short side; the long side keeps the picture's aspect, at most
    /// [`LONG_SIDE_RATIO`] times as long.
    pub short_side: u8,
    /// A picture within [`POSTER_FIT_TOLERANCE`] of this shape (`w`, `h`) is one the tile covers
    /// whatever the copy, so it is given none; `None` makes a copy of every picture.
    pub skip_shape: Option<(u16, u16)>,
    /// What a picture WITH TRANSPARENCY ([`is_see_through`]) gets instead of a blur: this flat ground
    /// (RGB, drawn as it is), or `None` for no copy at all (it is then covered, as without a spec).
    /// The blur of a mark on a clear ground is a grey wash, so the policy is the request's, not the
    /// blur's.
    pub ground: Option<[u8; 3]>,
    /// The brightness the blurred copy keeps, in percent of the picture's own colours (baked into the copy when it
    /// is made, so the tile draws it with no tint): a poster's backdrop sits under the contained picture at
    /// [`theme::ART_BACKDROP_DIM`]; another surface asks for the brightness that suits it. A flat ground is
    /// drawn as it is, whatever this says.
    pub dim: u8,
}

/// `fraction` (0 to 1) as a whole percent, for [`CopySpec::dim`].
pub const fn percent(fraction: f32) -> u8 {
    (fraction * 100.0 + 0.5) as u8
}

impl CopySpec {
    /// A poster tile's: [`POSTER_BACKDROP_SIDE`] texels, none for a picture within the tolerance of
    /// 2:3 (a poster-tile fit covers it, and so does [`place`] for any 2:3 tile), and for a picture
    /// with transparency the Recent channel plates' own ground ([`theme::CHANNEL_PLATE_RGB8`]).
    pub const POSTER: Self = Self {
        short_side: POSTER_BACKDROP_SIDE,
        skip_shape: Some((2, 3)),
        ground: Some(theme::CHANNEL_PLATE_RGB8),
        dim: percent(theme::ART_BACKDROP_DIM),
    };

    /// Does a `w` x `h` picture get a copy under this spec?
    pub fn wanted_for(self, w: u16, h: u16) -> bool {
        self.skip_shape.is_none_or(|(sw, sh)| !within_tolerance(sw as f32, sh as f32, w as f32, h as f32))
    }
}

/// Where a picture goes in its tile: what [`place`] answers and the card draw paints.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Placement {
    /// One quad filling the tile, sampling the `uv` window ([`crate::widgets::art_uv`]'s answer).
    Cover { uv: [f32; 4] },
    /// Two quads: the backdrop (a blurred copy, or a flat ground) over the whole tile (its `backdrop_uv` window), then the picture
    /// whole in `inner`, centred in the tile with its aspect kept.
    Contained { inner: Rect, backdrop_uv: [f32; 4] },
}

/// Is a `w` x `h` picture within [`POSTER_FIT_TOLERANCE`] of the shape `rw` x `rh`? Cross-multiplied
/// (`|w / h - rw / rh| <= tol * rw / rh`), so the edge is decided on products of the sizes and not
/// on two divisions' rounding. An empty picture is within it (the undecoded answer is cover's whole
/// texture).
fn within_tolerance(rw: f32, rh: f32, w: f32, h: f32) -> bool {
    if w <= 0.0 || h <= 0.0 || rw <= 0.0 || rh <= 0.0 {
        return true;
    }
    (w * rh - h * rw).abs() <= POSTER_FIT_TOLERANCE * h * rw
}

/// **The placement of a `tw` x `th` picture in the tile `r`** under `fit`. `crop` is the cover's
/// ([`crate::widgets::art_crop`]); `has_backdrop` says whether the picture came with its blurred
/// copy or ground ([`crate::widgets::ArtImage::backdrop`] is not 0): without one a picture is covered,
/// as it was before this existed, and never drawn contained on a bare ground.
pub fn place(fit: ArtFit, crop: Crop, tw: f32, th: f32, r: Rect, has_backdrop: bool) -> Placement {
    let cover = r.cover_uv(tw, th, crop);
    if fit != ArtFit::ContainOnBlur || !has_backdrop || r.w <= 0.0 || r.h <= 0.0 || within_tolerance(r.w, r.h, tw, th) {
        return Placement::Cover { uv: cover };
    }
    Placement::Contained { inner: r.contain(tw, th), backdrop_uv: cover }
}

/// The size of the blurred copy of a `w` x `h` picture: `short` texels on the short side, the same
/// aspect on the long one (at most [`LONG_SIDE_RATIO`] times `short`).
fn backdrop_size(w: usize, h: usize, short: usize) -> (usize, usize) {
    let long = ((short * w.max(h)) as f32 / w.min(h) as f32).round() as usize;
    let long = long.clamp(short, short * LONG_SIDE_RATIO);
    if w >= h { (long, short) } else { (short, long) }
}

/// The share of a picture's sampled texels (percent) that must be translucent for it to count as one
/// with transparency: a few anti-aliased edge texels on an opaque picture are not a clear ground.
const SEE_THROUGH_PERCENT: usize = 5;

/// An alpha below this is translucent (a texel that is not quite opaque is still a plain picture's).
const OPAQUE_ALPHA: u8 = 250;

/// The most texels per axis [`is_see_through`] reads: an even grid, so the cost is bounded
/// (`64 * 64`) whatever the picture's size.
const SEE_THROUGH_TAPS_PER_AXIS: usize = 64;

/// Does this picture have transparency: at least [`SEE_THROUGH_PERCENT`] of an even grid of its
/// texels (at most [`SEE_THROUGH_TAPS_PER_AXIS`] per axis) are translucent? A channel's logo on a
/// clear ground does; a JPEG never does. An empty or short buffer does not.
pub fn is_see_through(d: &Decoded) -> bool {
    let (w, h) = (d.w as usize, d.h as usize);
    if w == 0 || h == 0 || d.rgba.len() < w * h * 4 {
        return false;
    }
    let (nx, ny) = (w.min(SEE_THROUGH_TAPS_PER_AXIS), h.min(SEE_THROUGH_TAPS_PER_AXIS));
    let mut clear = 0;
    for j in 0..ny {
        let y = (j * 2 + 1) * h / (ny * 2);
        for i in 0..nx {
            let x = (i * 2 + 1) * w / (nx * 2);
            clear += (d.rgba[(y * w + x) * 4 + 3] < OPAQUE_ALPHA) as usize;
        }
    }
    clear * 100 >= nx * ny * SEE_THROUGH_PERCENT
}

/// An alpha above this is ink: a texel a viewer can see. A resizer's anti-aliased fringe and a faint
/// glow on a clear canvas stay below it, so they do not stretch the box.
const INK_ALPHA: u8 = 16;

/// **The bounding box of a picture's ink**, in texels: the smallest rectangle holding every texel whose
/// alpha is above [`INK_ALPHA`] ([`ink_of`]). A fully opaque picture's is the whole picture.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InkBox {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

/// **What a request for the ink box found** ([`Derive::Ink`], [`crate::tex::Decoded::ink`]; `None` there means the
/// request did not ask, or the buffer was unreadable). A picture with nothing in it is its own answer, not a box of
/// zero: a plate draws the channel's name for it, where a missing answer draws the whole texture.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ink {
    /// The bounding box of the visible texels (a fully opaque picture's is the whole picture).
    Box(InkBox),
    /// No texel is visible: the picture is entirely clear.
    Clear,
}

impl InkBox {
    /// The texture window (`u`, `v`, `w`, `h`, each 0 to 1; [`crate::Painter::tex_uv`]'s) of this box in a
    /// `tw` x `th` texture. The whole texture for an empty one.
    pub fn uv(self, tw: f32, th: f32) -> [f32; 4] {
        if tw <= 0.0 || th <= 0.0 {
            return UV_FULL;
        }
        [self.x as f32 / tw, self.y as f32 / th, self.w as f32 / tw, self.h as f32 / th]
    }
}

/// The [`Ink`] of `d`: its box, or [`Ink::Clear`] for a picture with no visible texel; `None` for no size or a short buffer. One
/// pass over the alpha channel, so it costs the picture's texel count in byte reads, once per
/// picture: 40-75 us for a 436 x 654 PNG on an Apple M4 at `-O` (a standalone copy of the loop, 2026-11-26);
/// **unmeasured on the television**, where an ARMv7 core is likely 10x slower, so under a millisecond. The
/// poster adapter runs it where it makes the copy ([`attach_derived`]).
pub fn ink_of(d: &Decoded) -> Option<Ink> {
    let (w, h) = (d.w as usize, d.h as usize);
    if w == 0 || h == 0 || d.rgba.len() < w * h * 4 {
        return None;
    }
    let (mut x0, mut x1, mut y0, mut y1) = (usize::MAX, 0, usize::MAX, 0);
    for (y, row) in d.rgba[..w * h * 4].chunks_exact(w * 4).enumerate() {
        let mut ink = row.chunks_exact(4).enumerate().filter(|(_, p)| p[3] > INK_ALPHA).map(|(x, _)| x);
        let Some(first) = ink.next() else { continue };
        let last = ink.last().unwrap_or(first);
        (x0, x1) = (x0.min(first), x1.max(last));
        (y0, y1) = (y0.min(y), y1.max(y));
    }
    Some(if x0 == usize::MAX {
        Ink::Clear
    } else {
        Ink::Box(InkBox { x: x0 as u16, y: y0 as u16, w: (x1 - x0 + 1) as u16, h: (y1 - y0 + 1) as u16 })
    })
}

/// The one-texel copy that is a flat `ground`: what a picture with transparency is contained on.
fn ground_copy(ground: [u8; 3]) -> Decoded {
    Decoded { w: 1, h: 1, rgba: Box::new([ground[0], ground[1], ground[2], 255]), backdrop: None, ink: None }
}

/// `picture` with what `derive` asks for. A copy ([`Derive::Copy`]), when the picture gets one
/// ([`CopySpec::wanted_for`]): the blurred copy of an opaque picture ([`backdrop_of`]), and for one with transparency
/// ([`is_see_through`]) the spec's flat [`ground`](CopySpec::ground), or nothing if it has none. Or the [`Ink`]
/// ([`Derive::Ink`]). The producer's one call.
pub fn attach_derived(mut picture: Decoded, derive: Derive) -> Decoded {
    match derive {
        Derive::Ink => picture.ink = ink_of(&picture),
        Derive::Copy(spec) if spec.wanted_for(picture.w, picture.h) => {
            picture.backdrop = if is_see_through(&picture) {
                spec.ground.map(ground_copy)
            } else {
                backdrop_of(&picture, spec.short_side as usize, spec.dim)
            }
            .map(Box::new);
        }
        Derive::Copy(_) => {}
    }
    picture
}

/// **The blurred copy of a decoded picture**, `short` texels on its short side, or `None` when it
/// has none to make: a short or empty buffer, no size, or a picture no larger than the copy. Pure
/// over the pixels; no GL.
///
/// Each output texel is the mean of an even grid of at most [`TAPS_PER_AXIS`]^2 source texels under
/// it (not `card_art::resample`'s full-area box mean, which would read every texel of a ~375 KB
/// picture on the main thread, and would need the buffer copied into its `Rgba`), and one 3 x 3 box
/// pass (edges clamped) smooths the result. The picture is taken as OPAQUE: alpha is ignored, because
/// a picture with transparency is never blurred ([`attach_derived`] gives it a flat ground instead).
/// The copy is dimmed to `dim` percent ([`CopySpec::dim`]: so the tile draws it with no tint), opaque, and
/// keeps the picture's aspect, so the tile's cover window ([`Placement::Contained::backdrop_uv`])
/// addresses it as it addresses the picture.
pub fn backdrop_of(d: &Decoded, short: usize, dim: u8) -> Option<Decoded> {
    let (w, h) = (d.w as usize, d.h as usize);
    // A copy is smaller than its picture or it is not a blur of it: a thumbnail this small is its own.
    if short == 0 || w <= short || h <= short || d.rgba.len() < w * h * 4 {
        return None;
    }
    let (bw, bh) = backdrop_size(w, h, short);
    let (span_x, span_y) = (w as f32 / bw as f32, h as f32 / bh as f32);
    let (nx, ny) = ((span_x.ceil() as usize).clamp(1, TAPS_PER_AXIS), (span_y.ceil() as usize).clamp(1, TAPS_PER_AXIS));
    // Per output texel: the colour sum (r, g, b) and the number of taps.
    let mut sum = vec![[0.0f32; 4]; bw * bh];
    for j in 0..bh {
        for i in 0..bw {
            let acc = &mut sum[j * bw + i];
            for ty in 0..ny {
                let y = (((j as f32 + (ty as f32 + 0.5) / ny as f32) * span_y) as usize).min(h - 1);
                for tx in 0..nx {
                    let x = (((i as f32 + (tx as f32 + 0.5) / nx as f32) * span_x) as usize).min(w - 1);
                    let p = &d.rgba[(y * w + x) * 4..][..4];
                    for k in 0..3 {
                        acc[k] += p[k] as f32;
                    }
                    acc[3] += 1.0;
                }
            }
        }
    }
    let mut rgba = Vec::with_capacity(bw * bh * 4);
    for j in 0..bh {
        for i in 0..bw {
            let mut m = [0.0f32; 4];
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let (x, y) = ((i as i32 + dx).clamp(0, bw as i32 - 1) as usize, (j as i32 + dy).clamp(0, bh as i32 - 1) as usize);
                    for k in 0..4 {
                        m[k] += sum[y * bw + x][k];
                    }
                }
            }
            let k = (dim as f32 / 100.0) / m[3];
            rgba.extend([m[0] * k, m[1] * k, m[2] * k].map(|c| c.round().clamp(0.0, 255.0) as u8));
            rgba.push(255);
        }
    }
    Some(Decoded { w: bw as u16, h: bh as u16, rgba: rgba.into_boxed_slice(), backdrop: None, ink: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use plx_gfx::gfx::UV_FULL;

    const POSTER_DIM: u8 = CopySpec::POSTER.dim;
    const TILE: Rect = Rect { x: 100.0, y: 200.0, w: 250.0, h: 375.0 };

    fn near(a: f32, b: f32) -> bool { (a - b).abs() < 1e-3 }
    fn near4(a: [f32; 4], b: [f32; 4]) -> bool { a.iter().zip(b).all(|(x, y)| near(*x, y)) }

    fn px(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Decoded {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h { for x in 0..w { v.extend_from_slice(&f(x, y)); } }
        Decoded { w: w as u16, h: h as u16, rgba: v.into_boxed_slice(), backdrop: None, ink: None }
    }

    #[test]
    fn a_square_is_contained_centred_over_a_full_tile_backdrop() {
        let Placement::Contained { inner, backdrop_uv } = place(ArtFit::ContainOnBlur, Crop::Centre, 375.0, 375.0, TILE, true)
        else { panic!("a square is not poster-shaped: it must be contained") };
        assert!(near(inner.w, 250.0) && near(inner.h, 250.0), "whole image, aspect kept: {inner:?}");
        assert!(near(inner.cx(), TILE.cx()) && near(inner.cy(), TILE.cy()), "centred in the tile: {inner:?}");
        assert!(near4(backdrop_uv, TILE.cover_uv(375.0, 375.0, Crop::Centre)), "the backdrop covers the whole tile: {backdrop_uv:?}");
        assert!(backdrop_uv[2] < 1.0 && near(backdrop_uv[3], 1.0), "cover of a square in a poster crops the sides: {backdrop_uv:?}");
    }

    #[test]
    fn a_16_by_9_is_contained_centred_over_a_full_tile_backdrop() {
        let Placement::Contained { inner, backdrop_uv } = place(ArtFit::ContainOnBlur, Crop::Centre, 667.0, 375.0, TILE, true)
        else { panic!("16:9 is not poster-shaped: it must be contained") };
        assert!(near(inner.w, 250.0), "the width is the limit: {inner:?}");
        assert!(near(inner.h, 250.0 * 375.0 / 667.0), "aspect kept: {inner:?}");
        assert!(near(inner.cx(), TILE.cx()) && near(inner.cy(), TILE.cy()), "centred in the tile: {inner:?}");
        assert!(near4(backdrop_uv, TILE.cover_uv(667.0, 375.0, Crop::Centre)));
    }

    #[test]
    fn a_poster_shaped_source_keeps_the_cover_fit_and_no_backdrop() {
        for (tw, th) in [(250.0, 375.0), (500.0, 750.0), (400.0, 600.0)] {
            assert_eq!(place(ArtFit::ContainOnBlur, Crop::Centre, tw, th, TILE, true), Placement::Cover { uv: UV_FULL }, "{tw}x{th}");
        }
        let slightly = place(ArtFit::ContainOnBlur, Crop::Centre, 262.0, 375.0, TILE, true);
        assert_eq!(slightly, Placement::Cover { uv: TILE.cover_uv(262.0, 375.0, Crop::Centre) }, "today's cover crop, unchanged");
    }

    /// The tolerance edge ITSELF, on exact sizes: against the 250 x 375 tile 72 x 100 is 1.08 of the
    /// tile's aspect and 920 x 1500 is 0.92 of it, the two edges, and both are covered (inclusive);
    /// one texel past either is contained.
    #[test]
    fn the_tolerance_edge_is_inclusive() {
        let covered = |w: f32, h: f32| matches!(place(ArtFit::ContainOnBlur, Crop::Centre, w, h, TILE, true), Placement::Cover { .. });
        assert!(covered(72.0, 100.0), "exactly 1.08, wide edge");
        assert!(!covered(73.0, 100.0), "past the wide edge");
        assert!(covered(920.0, 1500.0), "exactly 0.92, narrow edge");
        assert!(!covered(919.0, 1500.0), "past the narrow edge");
        assert!(covered(1080.0, 1500.0), "wide edge at another size");
        assert_eq!(POSTER_FIT_TOLERANCE, 0.08);
    }

    /// The producer's own judgement (`CopySpec::POSTER`'s `skip_shape`, 2:3) is the same edge.
    #[test]
    fn a_poster_spec_skips_exactly_what_a_poster_tile_covers() {
        let spec = CopySpec::POSTER;
        assert!(!spec.wanted_for(72, 100) && spec.wanted_for(73, 100), "wide edge");
        assert!(!spec.wanted_for(920, 1500) && spec.wanted_for(919, 1500), "narrow edge");
        assert!(!spec.wanted_for(250, 375) && spec.wanted_for(375, 375) && spec.wanted_for(667, 375));
        let any = CopySpec { short_side: 16, skip_shape: None, ground: None, dim: POSTER_DIM };
        assert!(any.wanted_for(250, 375), "a spec with no shape to skip wants every picture");
        assert_eq!(ArtFit::Cover.derive_request(), None);
        assert_eq!(ArtFit::ContainOnBlur.derive_request(), Some(Derive::POSTER));
    }

    #[test]
    fn cover_fit_never_contains_and_a_missing_backdrop_falls_back_to_cover() {
        assert!(matches!(place(ArtFit::Cover, Crop::Centre, 375.0, 375.0, TILE, true), Placement::Cover { .. }));
        assert!(matches!(place(ArtFit::ContainOnBlur, Crop::Centre, 375.0, 375.0, TILE, false), Placement::Cover { .. }));
        assert_eq!(place(ArtFit::ContainOnBlur, Crop::Centre, 0.0, 0.0, TILE, true), Placement::Cover { uv: UV_FULL }, "undecoded");
    }

    /// The decision is about SHAPE, so a focus pop (the same tile, scaled) cannot flip it.
    #[test]
    fn the_pop_scale_does_not_change_the_decision() {
        for s in [1.0, 1.09] {
            let r = TILE.scaled(s);
            assert!(matches!(place(ArtFit::ContainOnBlur, Crop::Centre, 375.0, 375.0, r, true), Placement::Contained { .. }), "{s}");
            assert!(matches!(place(ArtFit::ContainOnBlur, Crop::Centre, 250.0, 375.0, r, true), Placement::Cover { .. }), "{s}");
        }
    }

    #[test]
    fn the_backdrop_of_a_square_and_a_16_by_9_is_tiny_and_keeps_the_aspect() {
        let sq = backdrop_of(&px(375, 375, |_, _| [200, 100, 50, 255]), 16, POSTER_DIM).expect("a square needs one");
        assert_eq!((sq.w, sq.h), (16, 16));
        let wide = backdrop_of(&px(667, 375, |_, _| [200, 100, 50, 255]), 16, POSTER_DIM).expect("16:9 needs one");
        assert_eq!((wide.w, wide.h), (28, 16));
        assert_eq!(wide.rgba.len(), 28 * 16 * 4);
        assert!(wide.backdrop.is_none());
        let tall = backdrop_of(&px(200, 600, |_, _| [9, 9, 9, 255]), 16, POSTER_DIM).expect("1:3 needs one");
        assert_eq!((tall.w, tall.h), (16, 48));
        let pano = backdrop_of(&px(2000, 100, |_, _| [9, 9, 9, 255]), 16, POSTER_DIM).expect("a panorama needs one");
        assert_eq!((pano.w, pano.h), (64, 16), "the long side is capped");
    }

    #[test]
    fn a_malformed_or_tiny_picture_has_no_backdrop() {
        let mut short = px(300, 300, |_, _| [1, 2, 3, 255]);
        short.rgba = vec![0; 100].into_boxed_slice();
        assert!(backdrop_of(&short, 16, POSTER_DIM).is_none(), "a buffer shorter than w*h*4");
        assert!(backdrop_of(&px(0, 0, |_, _| [0; 4]), 16, POSTER_DIM).is_none(), "no size");
        assert!(backdrop_of(&px(2, 2, |_, _| [1, 2, 3, 255]), 16, POSTER_DIM).is_none(), "a copy no smaller than the picture is no blur of it");
        assert!(backdrop_of(&px(16, 30, |_, _| [1, 2, 3, 255]), 16, POSTER_DIM).is_none(), "the short side is already the copy's");
        assert!(backdrop_of(&px(17, 30, |_, _| [1, 2, 3, 255]), 16, POSTER_DIM).is_some(), "one texel more is a picture to reduce");
        assert!(backdrop_of(&px(300, 300, |_, _| [1, 2, 3, 255]), 0, POSTER_DIM).is_none(), "a copy of no texels");
    }

    /// The size is the request's: another consumer asks for its own, and the long side keeps its ratio.
    #[test]
    fn the_copy_size_is_the_requests() {
        let d = px(667, 375, |_, _| [5, 6, 7, 255]);
        let big = backdrop_of(&d, 32, POSTER_DIM).expect("a 32-texel copy");
        assert_eq!((big.w, big.h), (57, 32));
        let small = backdrop_of(&d, 8, POSTER_DIM).expect("an 8-texel copy");
        assert_eq!((small.w, small.h), (14, 8));
        let pano = backdrop_of(&px(3000, 100, |_, _| [5, 6, 7, 255]), 8, POSTER_DIM).unwrap();
        assert_eq!((pano.w, pano.h), (32, 8), "the long side is at most 4x the short");
    }

    /// `attach_derived` applies the spec's own skip: a 2:3 poster is handed back untouched, a square
    /// gets its copy, and a spec without a shape to skip copies the poster shape too.
    #[test]
    fn attach_derived_makes_the_copy_only_for_a_picture_the_spec_wants_one_for() {
        let poster = attach_derived(px(250, 375, |_, _| [1, 2, 3, 255]), Derive::POSTER);
        assert!(poster.backdrop.is_none(), "a poster-shaped picture is covered, never contained");
        let square = attach_derived(px(375, 375, |_, _| [1, 2, 3, 255]), Derive::POSTER);
        let copy = square.backdrop.as_deref().expect("a square gets a copy");
        assert_eq!((copy.w, copy.h), (16, 16));
        let any = attach_derived(px(250, 375, |_, _| [1, 2, 3, 255]), Derive::Copy(CopySpec { short_side: 16, skip_shape: None, ground: None, dim: POSTER_DIM }));
        assert!(any.backdrop.is_some());
    }

    /// A logo on a clear ground (a PNG, as a channel's thumb arrives): the owner's decision is NO blurred
    /// copy, the blur of such a mark being a grey wash with its colours smeared in bands. It is handed
    /// the flat ground of the Recent channel plates, one texel, undimmed.
    #[test]
    fn a_picture_with_transparency_gets_the_plate_ground_not_a_blur() {
        let logo = px(300, 300, |x, y| if (100..200).contains(&x) && (100..200).contains(&y) { [250, 200, 0, 255] } else { [0, 0, 0, 0] });
        assert!(is_see_through(&logo));
        let got = attach_derived(logo, Derive::POSTER);
        let copy = got.backdrop.as_deref().expect("a ground");
        assert_eq!((copy.w, copy.h), (1, 1), "a flat ground, not a blur");
        assert_eq!(&copy.rgba[..], &[0x22, 0x27, 0x2d, 255], "the Recent plates' own colour, undimmed");
        assert_eq!(theme::CHANNEL_PLATE_RGB8, [0x22, 0x27, 0x2d]);
        assert_eq!(plx_gfx::gfx::tokens::rgb8(0x22, 0x27, 0x2d), theme::CHANNEL_PLATE, "the bytes ARE the plate");
        assert_eq!(CopySpec::POSTER.ground, Some(theme::CHANNEL_PLATE_RGB8));
    }

    /// An opaque off-shape picture keeps the blurred copy, dimmed; a spec with no ground gives a
    /// picture with transparency nothing at all (it is covered, as without a spec).
    #[test]
    fn an_opaque_picture_keeps_its_blur_and_a_spec_without_a_ground_gives_transparency_none() {
        let opaque = attach_derived(px(375, 375, |_, _| [200, 100, 50, 255]), Derive::POSTER);
        let copy = opaque.backdrop.as_deref().expect("a blur");
        assert_eq!((copy.w, copy.h), (16, 16));
        let k = theme::ART_BACKDROP_DIM;
        assert!(copy.rgba.chunks(4).all(|p| p == [(200.0 * k).round() as u8, (100.0 * k).round() as u8, (50.0 * k).round() as u8, 255]),
            "its own colours at the dim: {:?}", &copy.rgba[..4]);
        let clear = || px(300, 300, |_, _| [250, 200, 0, 0]);
        let none = attach_derived(clear(), Derive::Copy(CopySpec { ground: None, ..CopySpec::POSTER }));
        assert!(none.backdrop.is_none());
        assert!(attach_derived(clear(), Derive::POSTER).backdrop.is_some(), "the poster spec's ground");
    }

    /// The edge of "has transparency": a few anti-aliased texels on an opaque picture are not a clear
    /// ground; a picture with at least 5% of its sampled texels translucent is.
    #[test]
    fn transparency_is_a_share_of_the_picture_not_a_single_texel() {
        // A 64 x 64 picture is sampled at every texel: clear the first `n` columns.
        let with_clear_columns = |n: u32| px(64, 64, move |x, _| if x < n { [0, 0, 0, 0] } else { [9, 9, 9, 255] });
        assert!(!is_see_through(&with_clear_columns(0)), "opaque");
        assert!(!is_see_through(&with_clear_columns(3)), "4.7% clear");
        assert!(is_see_through(&with_clear_columns(4)), "6.3% clear");
        assert!(!is_see_through(&px(64, 64, |_, _| [9, 9, 9, 251])), "alpha 251 is a plain picture's");
        assert!(is_see_through(&px(64, 64, |_, _| [9, 9, 9, 249])));
        let mut short = px(64, 64, |_, _| [0; 4]);
        short.rgba = vec![0; 16].into_boxed_slice();
        assert!(!is_see_through(&short) && !is_see_through(&px(0, 0, |_, _| [0; 4])), "a short or empty buffer");
    }

    /// A mark on a much larger clear canvas: the box is the mark's, not the canvas's. A fully opaque
    /// picture's is the whole picture, and a picture with no ink is `Clear`.
    #[test]
    fn the_ink_box_is_the_mark_on_its_canvas_and_the_whole_of_an_opaque_picture() {
        let mark = px(400, 400, |x, y| if (100..300).contains(&x) && (175..225).contains(&y) { [250, 200, 0, 255] } else { [0, 0, 0, 0] });
        assert_eq!(ink_of(&mark), Some(Ink::Box(InkBox { x: 100, y: 175, w: 200, h: 50 })));
        assert_eq!(ink_of(&px(300, 200, |_, _| [9, 9, 9, 255])), Some(Ink::Box(InkBox { x: 0, y: 0, w: 300, h: 200 })), "opaque: the whole picture");
        assert_eq!(ink_of(&px(64, 64, |_, _| [9, 9, 9, 0])), Some(Ink::Clear), "nothing to see is an answer, not a box of zero");
        // a single texel is a box of one; the corners are reachable
        assert_eq!(ink_of(&px(8, 8, |x, y| if (x, y) == (7, 0) { [1, 1, 1, 255] } else { [0; 4] })), Some(Ink::Box(InkBox { x: 7, y: 0, w: 1, h: 1 })));
        let mut short = px(64, 64, |_, _| [9, 9, 9, 255]);
        short.rgba = vec![255; 16].into_boxed_slice();
        assert!(ink_of(&short).is_none() && ink_of(&px(0, 0, |_, _| [0; 4])).is_none(), "a short or empty buffer");
    }

    /// The fringe of a resizer and a faint glow are not the mark: alpha at the threshold is clear, one above is ink.
    #[test]
    fn a_faint_fringe_does_not_stretch_the_ink_box() {
        let with_edge = |a: u8| px(100, 100, move |x, y| match (x, y) {
            (40..=59, 40..=59) => [255, 255, 255, 255],
            (30, 30) => [255, 255, 255, a],
            _ => [0; 4],
        });
        assert_eq!(ink_of(&with_edge(INK_ALPHA)), Some(Ink::Box(InkBox { x: 40, y: 40, w: 20, h: 20 })), "a faint texel is clear");
        assert_eq!(ink_of(&with_edge(INK_ALPHA + 1)), Some(Ink::Box(InkBox { x: 30, y: 30, w: 30, h: 30 })), "a visible one is ink");
    }

    /// The box is the request's: only [`Derive::Ink`] gets one, and it makes no copy (not even the plate
    /// ground); the poster's copy comes with no box. A picture with nothing visible answers `Clear`.
    #[test]
    fn attach_derived_finds_the_ink_box_only_when_the_request_asks_for_it() {
        let mark = || px(400, 400, |x, y| if (100..300).contains(&x) && (175..225).contains(&y) { [250, 200, 0, 255] } else { [0, 0, 0, 0] });
        let got = attach_derived(mark(), Derive::INK);
        assert_eq!(got.ink, Some(Ink::Box(InkBox { x: 100, y: 175, w: 200, h: 50 })));
        assert!(got.backdrop.is_none(), "the ink request makes no copy, not even the plate ground");
        let opaque = attach_derived(px(375, 375, |_, _| [200, 100, 50, 255]), Derive::INK);
        assert_eq!((opaque.ink, opaque.backdrop.is_none()), (Some(Ink::Box(InkBox { x: 0, y: 0, w: 375, h: 375 })), true), "opaque: the whole picture, no copy");
        let clear = attach_derived(px(64, 64, |_, _| [9, 9, 9, 0]), Derive::INK);
        assert_eq!((clear.ink, clear.backdrop.is_none()), (Some(Ink::Clear), true), "a clear picture is Clear, not 'not asked'");
        let poster = attach_derived(mark(), Derive::POSTER);
        assert!(poster.ink.is_none() && poster.backdrop.is_some(), "a request that did not ask pays nothing for a box");
        assert_eq!(InkBox { x: 100, y: 175, w: 200, h: 50 }.uv(400.0, 400.0), [0.25, 0.4375, 0.5, 0.125]);
        assert_eq!(InkBox { x: 1, y: 1, w: 1, h: 1 }.uv(0.0, 0.0), UV_FULL, "an undecoded texture answers the whole window");
    }

    #[test]
    fn the_backdrop_is_the_blurred_mean_not_a_crop() {
        let flat = backdrop_of(&px(300, 300, |_, _| [200, 100, 50, 255]), 16, POSTER_DIM).unwrap();
        let k = theme::ART_BACKDROP_DIM;
        let dimmed = |c: f32| (c * k).round() as u8;
        assert!(flat.rgba.chunks(4).all(|p| p == [dimmed(200.0), dimmed(100.0), dimmed(50.0), 255]), "a flat image blurs to itself, dimmed, opaque");
        let split = backdrop_of(&px(320, 320, |x, _| if x < 160 { [255, 0, 0, 255] } else { [0, 0, 255, 255] }), 16, POSTER_DIM).unwrap();
        let row = |x: usize| split.rgba[(8 * 16 + x) * 4..][..3].to_vec();
        assert_eq!(row(0)[0], dimmed(255.0), "far left stays red");
        assert_eq!(row(15)[2], dimmed(255.0), "far right stays blue");
        let (a, b) = (row(7), row(8));
        assert!(a[0] > 0 && a[2] > 0 && b[0] > 0 && b[2] > 0, "the edge is smeared across texels: {a:?} {b:?}");
    }
}
