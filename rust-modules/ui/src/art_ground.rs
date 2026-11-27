//! **The art ground: a programme's picture, full screen, standing in for video that has not arrived.**
//!
//! The Live TV player draws it over the video plane's hole from a zap's first press until the new
//! stream's first frame (`docs/hud-plugins.md`, "The live banner"; handoff SPEC-4): the target airing's
//! art, scaled [`ZOOM`], at [`BRIGHTNESS`], heavily blurred, never black. A channel with no art, or whose
//! art has not decoded yet, stands on the app's own ground ([`theme::SURFACE_APP`]).
//!
//! **The blur is made once per image, not drawn.** The request asks for the art with its own
//! [`Derive`] ([`SPEC`]): the poster adapter makes a small blurred copy of the picture when it
//! arrives (`art_fit::backdrop_of`, [`SHORT_SIDE`] texels on the short side, brightness baked in at
//! [`BRIGHTNESS`]) and uploads it beside the picture, and this draws that copy as ONE plain textured quad over
//! the screen. There is no render pass and no per-frame filter, and nothing here depends on the live glass,
//! which the player route never has (the video plane is bound, `live_blur_available` is false), so the art is
//! as soft over a playing channel as over a stopped one. The GPU's bilinear upscale of the copy is the rest of
//! the blur. Art with a clear ground (a channel's logo) has no blur worth drawing: the spec's flat ground
//! stands in for it, the app's own. A picture too small to have a copy is drawn as it is, at the same
//! brightness.
use crate::art_fit::{percent, CopySpec, Derive};
use crate::{theme, Crop, Painter, Rect};

/// The handoff's scale of the art over the screen.
pub const ZOOM: f32 = 1.08;
/// The handoff's brightness of the art: baked into the blurred copy ([`CopySpec::dim`]), and the tint of a
/// picture drawn as it is.
pub const BRIGHTNESS: f32 = 0.85;
/// Texels on the copy's short side: over 1080 px of screen each texel is spread over ~34 px by the GPU's
/// bilinear upscale, a blur heavy enough that no shape of the picture survives and its colours do.
pub const SHORT_SIDE: u8 = 32;

/// What the art ground asks of the source with its art: a copy of every picture (no shape is skipped), the app's
/// own ground for art with a clear ground, and the handoff's brightness.
pub const SPEC: Derive = Derive::Copy(CopySpec {
    short_side: SHORT_SIDE,
    skip_shape: None,
    ground: Some(theme::SURFACE_APP_RGB8),
    dim: percent(BRIGHTNESS),
});

/// The size the art is requested at (the programme still's own).
const ART_W: i32 = 480;
const ART_H: i32 = 270;

/// The texture window of a `tw` x `th` picture that covers the screen, zoomed [`ZOOM`] about its centre.
pub fn zoomed_uv(tw: f32, th: f32) -> [f32; 4] {
    let [u, v, w, h] = Rect::FULL.cover_uv(tw, th, Crop::Centre);
    let (zw, zh) = (w / ZOOM, h / ZOOM);
    [u + (w - zw) * 0.5, v + (h - zh) * 0.5, zw, zh]
}

/// Draw the art ground at `alpha` (1 covers, 0 shows the video): the art of `key` on server `srv`, else
/// the app's ground. Returns whether the picture was the art (not the stand-in), for a caller's test.
pub fn draw(p: Painter, srv: u16, key: &str, alpha: f32) -> bool {
    if alpha <= 0.0 {
        return false;
    }
    let p = p.alpha(alpha);
    let art = crate::tex::resolve_art_on(srv, key, ART_W, ART_H, false, Some(SPEC));
    if art.id == 0 {
        p.rect(Rect::FULL, 0.0, theme::SURFACE_APP, theme::SURFACE_APP, 0.0);
        return false;
    }
    if art.backdrop != 0 {
        // The copy keeps the picture's aspect (a flat ground is one texel), so the picture's window addresses it.
        p.tex_uv(art.backdrop, zoomed_uv(art.w, art.h), Rect::FULL, 0.0, theme::TINT_WHITE);
    } else {
        p.tex_uv(art.id, zoomed_uv(art.w, art.h), Rect::FULL, 0.0, [BRIGHTNESS, BRIGHTNESS, BRIGHTNESS, 1.0]);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::art_fit::attach_derived;
    use crate::tex::{Decoded, PosterReady, Source, Tex, Uploader, Warm};
    use plx_machine::machine::PosterKey;
    use std::sync::Mutex;

    static ASKED: Mutex<Vec<Option<Derive>>> = Mutex::new(Vec::new());
    struct Always;
    impl Source for Always {
        fn probe(&self, _: u16, _: &str, w: i32, h: i32, _: bool, spec: Option<Derive>) -> Option<PosterKey> {
            assert_eq!((w, h), (ART_W, ART_H));
            ASKED.lock().unwrap().push(spec);
            Some(PosterKey(7))
        }
        fn warm(&self, _: u16, _: &str, _: i32, _: i32, _: bool, _: Option<Derive>) -> Warm { Warm::Known }
        fn logo(&self, _: u16, _: &str) -> Option<PosterKey> { None }
        fn logo_warm(&self, _: u16, _: &str) -> Warm { Warm::Known }
        fn unresident(&self, _: PosterKey, _: bool) {}
        fn idle(&self) -> bool { true }
    }
    struct Up(u32);
    impl Uploader for Up {
        fn upload(&mut self, d: &Decoded) -> Tex { self.0 += 1; Tex { id: self.0, w: d.w, h: d.h } }
        fn warm(&mut self, _: Tex) {}
        fn free(&mut self, _: Tex) {}
    }
    static SOURCE: Always = Always;

    /// Arrive `picture` (with the copy [`SPEC`] asks for, as the poster adapter makes it) and upload it; the
    /// picture is texture 11, its copy 12.
    fn arrive(picture: Decoded) {
        crate::tex::install(&SOURCE);
        crate::tex::reset_for_test(usize::MAX);
        crate::tex::accept(PosterReady { key: PosterKey(7), result: Ok(attach_derived(picture, SPEC)) });
        let mut budget = crate::frame::Budget::new();
        budget.begin_frame(0);
        let mut present = plx_machine::present::Present::new();
        let mut handle = plx_machine::machine::PresentHandle::of(&mut present);
        crate::tex::prepare(&mut budget, &mut Up(10), &mut handle, || 0);
    }

    fn picture(w: u16, h: u16, px: [u8; 4]) -> Decoded {
        Decoded { w, h, rgba: px.repeat(w as usize * h as usize).into_boxed_slice(), backdrop: None, ink: None }
    }

    /// The window is the cover window shrunk by the zoom about its centre, so a 16:9 picture on the 16:9
    /// screen shows its middle 1/1.08 and nothing else.
    #[test]
    fn the_window_is_the_cover_window_zoomed_about_its_centre() {
        let uv = zoomed_uv(480.0, 270.0);
        let want = 1.0 / ZOOM;
        assert!((uv[2] - want).abs() < 1e-5 && (uv[3] - want).abs() < 1e-5, "{uv:?}");
        assert!((uv[0] - (1.0 - want) * 0.5).abs() < 1e-5 && (uv[1] - (1.0 - want) * 0.5).abs() < 1e-5, "{uv:?}");
    }

    /// **With the video plane bound the art is still blurred**: a zap from a playing channel is the case the live
    /// glass cannot serve. The frame is one textured quad of the picture's COPY over the whole screen at the art's
    /// alpha, no quad of the sharp picture and no glass pass.
    #[test]
    fn the_art_over_a_bound_video_plane_is_its_blurred_copy_and_no_glass() {
        let _g = plx_base::testlock::serial();
        arrive(picture(480, 270, [40, 80, 120, 255]));
        ASKED.lock().unwrap().clear();
        let was = plx_gfx::gfx::set_video_plane_frame(true);
        assert!(!plx_gfx::gfx::live_blur_available(), "the premise: no live blur on this frame");
        let mut drawn = false;
        let mut tags = Vec::new();
        let quads = crate::draw_census::capture_tex(|| {
            tags = crate::draw_census::capture(|| drawn = draw(Painter::recording(), 0, "/library/metadata/7/thumb/1", 0.6));
        });
        plx_gfx::gfx::set_video_plane_frame(was);
        assert!(drawn);
        assert_eq!(*ASKED.lock().unwrap(), vec![Some(SPEC)], "the art is requested with its own copy");
        assert_eq!(quads.len(), 1, "one textured quad: {quads:?}");
        assert_eq!(quads[0].0, 12, "the copy's texture, not the sharp picture's (11): {quads:?}");
        assert_eq!(quads[0].1, Rect::FULL);
        assert!((quads[0].2 - 0.6).abs() < 1e-6, "at the art's alpha: {quads:?}");
        assert!(!tags.iter().any(|(tag, _)| *tag == crate::frame::backdrop::GLASS_COMMAND), "no glass pass: {tags:?}");
    }

    /// The copy is the picture at the handoff's brightness, made once, small and soft; art with a clear ground gets
    /// the app's own ground, flat.
    #[test]
    fn the_copy_carries_the_brightness_and_a_logo_gets_the_app_ground() {
        let copy = attach_derived(picture(480, 270, [100, 200, 40, 255]), SPEC).backdrop.expect("a copy");
        assert_eq!((copy.w, copy.h), (57, 32), "the short side is SHORT_SIDE");
        assert_eq!(&copy.rgba[..4], &[85, 170, 34, 255], "the picture at BRIGHTNESS, baked in");
        let logo = attach_derived(picture(300, 300, [250, 200, 0, 0]), SPEC).backdrop.expect("a ground");
        assert_eq!((logo.w, logo.h, &logo.rgba[..]), (1, 1, &[0x2c, 0x2c, 0x2e, 255][..]), "the app ground, flat");
        assert_eq!(crate::theme::SURFACE_APP_RGB8, [0x2c, 0x2c, 0x2e]);
        assert_eq!(plx_gfx::gfx::tokens::rgb8(0x2c, 0x2c, 0x2e), theme::SURFACE_APP, "the bytes ARE the app ground");
    }

    /// A channel with no art, or art that has not decoded yet, is the app's own ground: never a hole.
    #[test]
    fn no_art_is_the_apps_ground() {
        let _g = plx_base::testlock::serial();
        crate::tex::install(&SOURCE);
        crate::tex::reset_for_test(usize::MAX);
        let mut drawn = true;
        let quads = crate::draw_census::capture_tex(|| drawn = draw(Painter::recording(), 0, "", 1.0));
        assert!(!drawn && quads.is_empty());
    }
}
