//! SDL's process-level lifecycle: bring-up, the GL context request, the hint door, the idle sleep
//! and the quit. Everything here runs on the SDL main thread, which the caller proves with the
//! `&MainThread` it was handed.
use crate::sys;
use plx_base::eventlog::log;
use plx_base::task::MainThread;
use std::ffi::CStr;
use std::os::raw::c_int;

/// Which SDL2 the platform links, as the one fact the byte layouts of SDL's events depend on. The
/// television's SDL is LG's fork, which lays its keyboard, text and wheel events out differently from
/// the stock SDL2 a desktop links ([`crate::event::decode_key`] has the measurements). A platform
/// states it once, in its [`InitHooks`], and every decode and encode in this crate takes it as an
/// argument, so both layouts are compiled and graded in every build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SdlFlavor {
    /// LG's fork of SDL2, on the television.
    Lg,
    /// Stock SDL2 (sdl2-compat over SDL3 on a Mac), on a desktop.
    Stock,
}

/// The GL context the platform's driver can give, which [`request_gl`] asks SDL for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlProfile {
    /// OpenGL ES 2.0: the television's driver (a shim over libmali).
    Es2,
    /// OpenGL 4.1 core: the closest thing macOS has to ES 2.0, which it does not ship at all.
    Core41,
}

/// The steps a platform runs around SDL's bring-up: two around `SDL_Init` (`before`: it reads hints
/// SDL consults while it initialises; `after`: it uses what `SDL_Init` set up) and one behind the
/// GL context (`after_context`: it sets up what its driver needs bound before the renderer's first
/// call, which a desktop core profile does with a vertex array object). Plain function pointers, so
/// the crate names nothing of the platform that supplies them: the television's are LG-fork code,
/// and a platform with nothing to do states [`InitSteps::NONE`].
#[derive(Clone, Copy)]
pub struct InitSteps {
    pub before: unsafe fn(),
    pub after: unsafe fn(),
    pub after_context: unsafe fn(),
}

impl InitSteps {
    /// A platform with nothing to do around `SDL_Init` or behind the GL context.
    pub const NONE: InitSteps = InitSteps { before: noop, after: noop, after_context: noop };
}

/// Everything a platform states about its SDL: the [`InitSteps`] it runs, the [`SdlFlavor`] its
/// events are laid out in and the [`GlProfile`] it asks for. Every field is required, so a platform
/// states each one and no default hides in this crate.
#[derive(Clone, Copy)]
pub struct InitHooks {
    pub steps: InitSteps,
    pub flavor: SdlFlavor,
    pub gl: GlProfile,
}

unsafe fn noop() {}

/// `SDL_Init` refused (the reason is the log line `SDL_Init failed`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InitFailed;

const SDL_INIT_VIDEO: u32 = 0x20;

// SDL_GLattr enum
const A_RED: c_int = 0;
const A_GREEN: c_int = 1;
const A_BLUE: c_int = 2;
const A_ALPHA: c_int = 3;
const A_BUFFER_SIZE: c_int = 4;
const A_DEPTH: c_int = 6;
const A_STENCIL: c_int = 7;
const A_CTX_MAJOR: c_int = 17;
const A_CTX_MINOR: c_int = 18;
const A_CTX_PROFILE_MASK: c_int = 21;
const CTX_PROFILE_ES: c_int = 0x0004;
/// `SDL_GL_CONTEXT_PROFILE_CORE` — a desktop's only option on macOS ([`GlProfile::Core41`]). See [`request_gl`].
const CTX_PROFILE_CORE: c_int = 0x0001;

/// Bring SDL's video subsystem up: tell SDL the entry point is ours, state the screensaver intent,
/// run `hooks.steps.before`, `SDL_Init(VIDEO)`, run `hooks.steps.after`, and log the driver it picked.
///
/// # Safety
/// Once, before any other SDL call (the `&MainThread` is the proof of the thread); and `hooks`
/// must be sound to call on that thread at those two points (the third step, `after_context`, is
/// [`crate::window::Window::open`]'s).
pub unsafe fn init(_mt: &MainThread, hooks: &InitHooks) -> Result<(), InitFailed> {
    sys::SDL_SetMainReady();
    // DEAD END, measured 2026-07-31 — do not re-try this. The obvious answer to "a parked TV
    // should blank itself" is to stop inhibiting the platform screensaver here (and re-allow it
    // per route, since webOS BACKGROUNDS the app to run one and `0x103` suspends the
    // buffer-feed, so it could never be on during playback). It does not work, for a reason
    // upstream of this app: the TV's SDL 2.0.4 fork carries the
    // `SDL_VIDEO_ALLOW_SCREENSAVER` hint STRING but implements no wayland idle-inhibit
    // (`strings libSDL2-2.0.so.0` finds no `idle_inhibit`/`suspend_screensaver` symbol), so
    // this call and `SDL_EnableScreenSaver` are both no-ops. Soaked 34 min on Home with the
    // TV's own `screenSaverEnabled: on`: no screensaver, no `LIFECYCLE: background`, CPU flat,
    // our UI still at full brightness on the panel. webOS does not blank a foreground native
    // app, and nothing reachable from SDL changes that. The line stays because it costs
    // nothing and states the intent; it is not what keeps the screensaver away.
    sys::SDL_SetHint(c"SDL_VIDEO_ALLOW_SCREENSAVER".as_ptr(), c"0".as_ptr());
    (hooks.steps.before)();
    if sys::SDL_Init(SDL_INIT_VIDEO) != 0 {
        log("SDL_Init failed");
        return Err(InitFailed);
    }
    (hooks.steps.after)();
    {
        let d = sys::SDL_GetCurrentVideoDriver();
        if !d.is_null() {
            log(&format!(
                "video driver: {}",
                std::ffi::CStr::from_ptr(d).to_string_lossy()
            ));
        }
    }
    Ok(())
}

/// Ask SDL for the GL context [`crate::window::Window::open`] will create. It is its own call
/// because the application reads the stored session between the request and the window (the
/// window size is a setting), and that order is pinned.
///
pub fn request_gl(_mt: &MainThread, gl: GlProfile) {
    unsafe { request_gl_attributes(gl) }
}

unsafe fn request_gl_attributes(gl: GlProfile) {
    // The television has a real GLES2 driver (a shim over libmali). macOS has none at all —
    // Apple ships desktop GL only, capped at 4.1 core — so asking for ES here fails context
    // creation outright. 4.1 core is the closest thing that exists, and it is a superset for
    // everything this renderer does: a real VBO (never client arrays) and RGBA/UNSIGNED_BYTE
    // textures, both core-profile-legal. The shader sources are adapted at compile time by
    // `gfx::glsl_preamble`, which reads the driver's GLSL version rather than assuming.
    match gl {
        GlProfile::Core41 => {
            sys::SDL_GL_SetAttribute(A_CTX_PROFILE_MASK, CTX_PROFILE_CORE);
            sys::SDL_GL_SetAttribute(A_CTX_MAJOR, 4);
            sys::SDL_GL_SetAttribute(A_CTX_MINOR, 1);
        }
        GlProfile::Es2 => {
            sys::SDL_GL_SetAttribute(A_CTX_PROFILE_MASK, CTX_PROFILE_ES);
            sys::SDL_GL_SetAttribute(A_CTX_MAJOR, 2);
            sys::SDL_GL_SetAttribute(A_CTX_MINOR, 0);
        }
    }
    // full 32-bit RGBA so the video plane shows through
    sys::SDL_GL_SetAttribute(A_RED, 8);
    sys::SDL_GL_SetAttribute(A_GREEN, 8);
    sys::SDL_GL_SetAttribute(A_BLUE, 8);
    sys::SDL_GL_SetAttribute(A_ALPHA, 8);
    sys::SDL_GL_SetAttribute(A_BUFFER_SIZE, 32);
    // ...and NO depth or stencil, which SDL would otherwise give us anyway: its defaults are
    // 16 bits of depth and 0 of stencil, and asking for neither had simply never been written
    // down. **This renderer has no use for either.** There is no `GL_DEPTH_TEST`, no
    // `glDepthFunc`, no `glDepthMask` and no `glClear(GL_DEPTH_BUFFER_BIT)` anywhere in the
    // crate — every screen is painter's-algorithm 2-D, drawn back to front — and the one
    // scissor user (`gfx::clip_set`) is a scissor, not a stencil.
    //
    // On a TILER this is not merely 4 MB of address space. Midgard allocates the depth buffer
    // per tile alongside colour and, unless the driver proves it dead, RESOLVES it to memory at
    // end-of-frame: 1920x1080x2 bytes written per presented frame for a buffer nothing ever
    // reads. The webOS platform crate's `system.rs` logs what the config actually came back with — a
    // request is not a grant, and the only honest confirmation is `FB bits: … depth=0`.
    sys::SDL_GL_SetAttribute(A_DEPTH, 0);
    sys::SDL_GL_SetAttribute(A_STENCIL, 0);
}

/// `SDL_SetHint`: the one way a platform's own boot code states a hint to SDL (LG's
/// `SDL_WEBOS_CURSOR_SLEEP_TIME`, read in `SDL_Init`). `true` when SDL took it.
pub fn set_hint(name: &CStr, value: &CStr) -> bool {
    unsafe { sys::SDL_SetHint(name.as_ptr(), value.as_ptr()) != 0 }
}

/// `SDL_Quit`. The caller decides WHEN (the application's `shutdown` skips it while a native
/// worker is still inside firmware): this only does it.
///
/// # Safety
/// Main thread, after SDL's last use; nothing may call into SDL afterwards.
pub unsafe fn quit() {
    sys::SDL_Quit();
}

/// `SDL_Delay`: the idle loop's sleep.
pub fn delay_ms(ms: u32) {
    unsafe { sys::SDL_Delay(ms) };
}

#[cfg(test)]
mod tests {
    /// A hostile platform step is the caller's to get right, but the shape the crate promises is
    /// that `init` runs `before` ahead of `SDL_Init` and `after` behind it; the source order is the
    /// proof, because no host test can run `SDL_Init` on the video subsystem.
    #[test]
    fn the_before_hook_runs_ahead_of_sdl_init_and_the_after_hook_behind_it() {
        let source = include_str!("init.rs");
        let body = source
            .split_once(concat!("pub unsafe fn ", "init("))
            .expect("init")
            .1
            .split_once(concat!("pub fn ", "request_gl("))
            .expect("the end of init")
            .0;
        let before = body.find(concat!("(hooks.steps.", "before)()")).expect("init runs the before hook");
        let call = body.find(concat!("sys::SDL_Init(", "SDL_INIT_VIDEO)")).expect("init calls SDL_Init");
        let after = body.find(concat!("(hooks.steps.", "after)()")).expect("init runs the after hook");
        assert!(before < call && call < after, "before < SDL_Init < after");
        assert_eq!(body.matches(concat!("sys::SDL_Init(", "SDL_INIT_VIDEO)")).count(), 1);
    }

    /// `SDL_Init` is the only video bring-up this crate makes, and the GL request must not ride
    /// inside it: the application reads the stored session between the two.
    #[test]
    fn the_gl_request_is_not_part_of_init() {
        let source = include_str!("init.rs");
        let body = source
            .split_once(concat!("pub unsafe fn ", "init("))
            .expect("init")
            .1
            .split_once(concat!("pub fn ", "request_gl("))
            .expect("the end of init")
            .0;
        assert!(!body.contains(concat!("SDL_GL_Set", "Attribute")), "init must not request the GL context");
    }
}
