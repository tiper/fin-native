//! The crop an art tile samples (`art_uv`): every picture keeps its own aspect in its tile.

use super::*;
use crate::card_row::RowStyle;

/// The crop never reads which server a picture came from, so any raw id will do.
const SRC: u16 = 0;

/// The cast row's circle, at rest and popped. The shape is what matters; the position is not.
fn cast_circle(scale: f32) -> Rect {
    Rect::new(96.0, 700.0, RowStyle::CAST.w, RowStyle::CAST.h).scaled(scale)
}

fn headshot(key: &str) -> Art<'_> {
    Art::Person { sid: SRC, key, res: (300, 300) }
}

/// Texels per drawn pixel on each axis — equal ⇔ an even scale, i.e. no distortion.
fn texel_scale(uv: [f32; 4], tw: f32, th: f32, r: Rect) -> (f32, f32) {
    (uv[2] * tw / r.w, uv[3] * th / r.h)
}

/// **The reported bug.** A cast headshot is requested at 300×300 with `minSize=1`, which COVERS the
/// box, so a portrait photo decodes at 300×450. The tile used to sample the whole texture into the
/// 190×190 circle — a vertical squash to two-thirds. It must now sample an even-scaled window that
/// keeps the full width and rides high on the photo, at rest and through the focus pop.
#[test]
fn a_portrait_headshot_on_the_cast_row_is_cropped_not_squashed() {
    let (tw, th) = (300.0, 450.0);
    for scale in [1.0, RowStyle::CAST.focus_scale] {
        let r = cast_circle(scale);
        let uv = art_uv(&headshot("/library/metadata/1/thumb"), tw, th, r);
        let (sx, sy) = texel_scale(uv, tw, th, r);
        assert!((sx - sy).abs() < 1e-4, "cast headshot drawn with uneven scale {sx} x {sy} (uv {uv:?})");
        assert_eq!(uv[2], 1.0, "the full width of a portrait survives");
        assert!(
            uv[1] < (1.0 - uv[3]) * 0.5,
            "a headshot's crop must keep more of the top than the bottom: {uv:?}"
        );
    }
}

/// Every surface that draws a person's photo draws it through `Art::Person` — the cast row, the
/// person page's portrait and Search's person results — so all of them get the headshot crop.
#[test]
fn every_person_photo_takes_the_headshot_crop_and_other_art_is_even() {
    assert_eq!(art_crop(&headshot("k")), crate::Crop::Headshot);
    assert_eq!(art_crop(&Art::Poster(None)), crate::Crop::Centre);
    assert_eq!(art_crop(&Art::Still(None)), crate::Crop::Centre);
    assert_eq!(
        art_crop(&Art::Thumb { sid: SRC, key: "k", res: (300, 300) }),
        crate::Crop::Centre
    );
}

/// Art already at its tile's aspect is sampled whole, so posters in poster tiles and 16:9 stills in
/// the episode filmstrip draw exactly as before; only art of ANOTHER aspect changes, and it
/// changes from squashed to cropped — here the poster a landscape tile falls back to.
#[test]
fn art_at_its_tiles_aspect_is_untouched_and_a_poster_in_a_landscape_tile_is_cropped() {
    let poster = Rect::new(0.0, 0.0, 250.0, 375.0);
    assert_eq!(art_uv(&Art::Poster(None), 250.0, 375.0, poster), plx_gfx::gfx::UV_FULL);
    let still = Rect::new(0.0, 0.0, RowStyle::EPISODE.w, RowStyle::EPISODE.h);
    let uv = art_uv(&Art::Still(None), 250.0, 375.0, still);
    let (sx, sy) = texel_scale(uv, 250.0, 375.0, still);
    assert!((sx - sy).abs() < 1e-4, "poster fallback drawn with uneven scale {sx} x {sy}");
    assert!(uv[3] < 1.0 && uv[2] == 1.0, "a tall poster in a wide tile loses top and bottom: {uv:?}");
    // an undecoded texture answers the whole window, never a NaN
    assert_eq!(art_uv(&headshot("k"), 0.0, 0.0, cast_circle(1.0)), plx_gfx::gfx::UV_FULL);
}

/// **Only Live TV's programme posters contain their art.** Every other row's style keeps the cover
/// fit it always had (Home, Library, Search, the person page, the strips of stills); the contained
/// fit is a row's declaration, never a global.
#[test]
fn only_the_programme_poster_style_contains_its_art() {
    use crate::art_fit::ArtFit;
    assert_eq!(RowStyle::PROGRAMME.fit, ArtFit::ContainOnBlur);
    assert!(RowStyle { fit: ArtFit::Cover, ..RowStyle::PROGRAMME } == RowStyle::HOME, "otherwise HOME's tile and motion");
    for style in [RowStyle::HOME, RowStyle::CHANNEL, RowStyle::EPISODE, RowStyle::STILL, RowStyle::CAST, RowStyle::PROFILES, crate::poster_grid::STYLE] {
        assert_eq!(style.fit, ArtFit::Cover);
    }
}

/// The picture and its blurred copy come back from ONE resolve: `resolve_card_art` hands the tile the
/// copy's texture beside the picture's, and a picture that arrived without one answers 0.
#[test]
fn a_card_resolves_the_blurred_copy_with_its_picture() {
    use crate::tex::{Decoded, PosterReady, Source, Warm};
    use plx_machine::machine::PosterKey;
    struct Always;
    impl Source for Always {
        fn probe(&self, _: u16, _: &str, _: i32, _: i32, _: bool, _: Option<crate::art_fit::Derive>) -> Option<PosterKey> { Some(PosterKey(7)) }
        fn warm(&self, _: u16, _: &str, _: i32, _: i32, _: bool, _: Option<crate::art_fit::Derive>) -> Warm { Warm::Known }
        fn logo(&self, _: u16, _: &str) -> Option<PosterKey> { None }
        fn logo_warm(&self, _: u16, _: &str) -> Warm { Warm::Known }
        fn unresident(&self, _: PosterKey, _: bool) {}
        fn idle(&self) -> bool { true }
    }
    struct Up(u32);
    impl crate::tex::Uploader for Up {
        fn upload(&mut self, d: &Decoded) -> crate::tex::Tex { self.0 += 1; crate::tex::Tex { id: self.0, w: d.w, h: d.h } }
        fn warm(&mut self, _: crate::tex::Tex) {}
        fn free(&mut self, _: crate::tex::Tex) {}
    }
    static SOURCE: Always = Always;
    let _guard = plx_base::testlock::serial();
    crate::tex::install(&SOURCE);
    for (w, h, expect_copy) in [(375u16, 375u16, true), (250, 375, false)] {
        crate::tex::reset_for_test(usize::MAX);
        let picture = Decoded { w, h, rgba: [40, 80, 120, 255].repeat(w as usize * h as usize).into_boxed_slice(), backdrop: None, ink: None };
        crate::tex::accept(PosterReady { key: PosterKey(7), result: Ok(crate::art_fit::attach_derived(picture, crate::art_fit::Derive::POSTER)) });
        let mut budget = crate::frame::Budget::new();
        budget.begin_frame(0);
        let mut present = plx_machine::present::Present::new();
        let mut handle = plx_machine::machine::PresentHandle::of(&mut present);
        crate::tex::prepare(&mut budget, &mut Up(10), &mut handle, || 0);
        let facts = crate::tile::TileFacts { owner: 880_000 + w as usize, thumb: "/library/metadata/7/thumb/1", ..Default::default() };
        let image = resolve_card_art(Painter::root(), Rect::new(0.0, 0.0, 250.0, 375.0), &Art::Poster(Some(facts)));
        assert_eq!((image.tex, image.w, image.h), (11, w as f32, h as f32), "{w}x{h}");
        assert_eq!(image.backdrop != 0, expect_copy, "{w}x{h}: {image:?}");
    }
}

/// What a poster tile's art paints, read from the recording painter's census: the card passes
/// (`(tag, rect)`; 14 is `tex_carded`, 27 `tex_carded_inset`) and the picture quads (`(texture,
/// rect, alpha)`).
fn painted(placement: Placement, f: f32) -> (Vec<(u64, Rect)>, Vec<(u32, Rect, f32)>) {
    painted_at(placement, f, 1.0)
}

/// [`painted`] for art that has only `shown` of its late-arrival fade ([`ArtImage::shown`]).
fn painted_at(placement: Placement, f: f32, shown: f32) -> (Vec<(u64, Rect)>, Vec<(u32, Rect, f32)>) {
    const TILE: Rect = Rect { x: 100.0, y: 200.0, w: 250.0, h: 375.0 };
    let mut quads = Vec::new();
    let cards = crate::draw_census::capture(|| {
        let p = Painter::recording();
        quads = crate::draw_census::capture_tex(|| paint_poster_art(p, p.alpha(shown), placement, 7, 8, TILE, 14.0, f));
    });
    (cards, quads)
}

/// **A contained picture is two layers; a covered one is one.** At rest the blurred copy is the
/// tile's card pass and the picture is its own quad, whole, in the box `place` centred it in; a
/// covered picture is the card pass alone.
#[test]
fn a_contained_tile_paints_the_card_and_the_picture_and_a_covered_one_only_the_card() {
    use crate::art_fit::{place, ArtFit};
    let tile = Rect::new(100.0, 200.0, 250.0, 375.0);
    let contained = place(ArtFit::ContainOnBlur, crate::Crop::Centre, 375.0, 375.0, tile, true);
    let Placement::Contained { inner, .. } = contained else { panic!("a square is contained") };
    let (cards, quads) = painted(contained, 0.0);
    assert_eq!(cards.iter().map(|c| c.0).collect::<Vec<_>>(), vec![14, 12], "the copy's card pass, then the picture's own quad (12)");
    assert_eq!(quads, vec![(7, inner, 1.0)], "and the picture whole in its box");
    let covered = place(ArtFit::ContainOnBlur, crate::Crop::Centre, 250.0, 375.0, tile, true);
    let (cards, quads) = painted(covered, 0.0);
    assert_eq!(cards.iter().map(|c| c.0).collect::<Vec<_>>(), vec![14]);
    assert!(quads.is_empty(), "a covered picture is the card pass alone: {quads:?}");
}

/// **A focused contained tile is composed INSIDE its one card pass.** The rim glow, the sheen crown and
/// the gloss ramp are that pass's; a second quad for the picture would paint over them inside its box
/// (the side glow lost over two thirds of each edge, the gloss stepping at the picture's top). So with
/// `f > 0` the census holds the inset pass (27) and not the plain card (14), and still the picture.
#[test]
fn a_focused_contained_tile_composes_its_picture_inside_the_card_pass() {
    use crate::art_fit::{place, ArtFit};
    let tile = Rect::new(100.0, 200.0, 250.0, 375.0);
    let contained = place(ArtFit::ContainOnBlur, crate::Crop::Centre, 667.0, 375.0, tile, true);
    let Placement::Contained { inner, .. } = contained else { panic!("16:9 is contained") };
    for f in [0.05, 1.0] {
        let (cards, quads) = painted(contained, f);
        assert_eq!(cards.iter().map(|c| c.0).collect::<Vec<_>>(), vec![27], "f={f}: the inset pass, not a card and a quad over it");
        assert_eq!(quads, vec![(7, inner, 1.0)], "f={f}: the picture is the pass's, in its box");
    }
}

/// **A contained tile that is still fading in does not double its picture's alpha.** The skeleton it
/// replaces is under it. Two layers each at the fade would composite to `1 - (1 - shown)^2` inside the
/// picture's box (0.64 at a fade of 0.4), ahead of the backdrop the rest of the tile has (0.4). A
/// resting tile fades the BACKDROP card and lays the picture at full alpha; a focused one is a single
/// pass, which fades as one surface, so the picture it composes carries the fade.
#[test]
fn a_contained_tile_fading_in_fades_its_card_once() {
    use crate::art_fit::{place, ArtFit};
    let tile = Rect::new(100.0, 200.0, 250.0, 375.0);
    let contained = place(ArtFit::ContainOnBlur, crate::Crop::Centre, 375.0, 375.0, tile, true);
    let (cards, quads) = painted_at(contained, 0.0, 0.4);
    assert_eq!(cards.iter().map(|c| c.0).collect::<Vec<_>>(), vec![14, 12]);
    assert_eq!(quads.iter().map(|q| q.2).collect::<Vec<_>>(), vec![1.0], "the picture is laid at full alpha over the fading card");
    let (cards, quads) = painted_at(contained, 1.0, 0.4);
    assert_eq!(cards.iter().map(|c| c.0).collect::<Vec<_>>(), vec![27]);
    assert_eq!(quads.iter().map(|q| q.2).collect::<Vec<_>>(), vec![0.4], "one pass: the picture fades with the card it is part of");
}

/// **A led tile resolves with its row's fit.** The grid and the sample row ask for the focused tile's
/// art ahead of painting it ([`crate::card_row::lead_focused`]) and draw with that answer: if the ask
/// named a Cover key, a contained tile would be handed a picture without its backdrop (and so cropped)
/// and the draw would not find it again.
#[test]
fn a_led_tile_resolves_the_key_its_rows_fit_draws() {
    use crate::art_fit::Derive;
    use crate::tex::{Source, Warm};
    use plx_machine::machine::PosterKey;
    use std::sync::Mutex;
    static ASKED: Mutex<Vec<Option<Derive>>> = Mutex::new(Vec::new());
    struct Spy;
    impl Source for Spy {
        fn probe(&self, _: u16, _: &str, _: i32, _: i32, _: bool, backdrop: Option<Derive>) -> Option<PosterKey> {
            ASKED.lock().unwrap().push(backdrop);
            None
        }
        fn warm(&self, _: u16, _: &str, _: i32, _: i32, _: bool, _: Option<Derive>) -> Warm { Warm::Known }
        fn logo(&self, _: u16, _: &str) -> Option<PosterKey> { None }
        fn logo_warm(&self, _: u16, _: &str) -> Warm { Warm::Known }
        fn unresident(&self, _: PosterKey, _: bool) {}
        fn idle(&self) -> bool { true }
    }
    static SPY: Spy = Spy;
    let _guard = plx_base::testlock::serial();
    crate::tex::install(&SPY);
    let facts = || crate::tile::TileFacts { owner: 880_500, thumb: "/library/metadata/9/thumb/1", ..Default::default() };
    for (style, expect) in [(RowStyle::PROGRAMME, Some(Derive::POSTER)), (RowStyle::HOME, None)] {
        ASKED.lock().unwrap().clear();
        crate::card_row::lead_focused(Painter::root(), &Art::Poster(Some(facts())), Rect::new(0.0, 0.0, 250.0, 375.0), 1.0, &style);
        assert_eq!(*ASKED.lock().unwrap(), vec![expect], "the lead asks for the copy exactly when the row contains");
    }
}
