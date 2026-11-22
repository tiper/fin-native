//! The EGL context the renderer draws into: display, ES2 window config, window surface, context.
//!
//! `plx_gfx::egl` only PROBES a context that already exists (it finds `libEGL` already mapped and asks it
//! for the extensions); a platform that owns no SDL window makes the context itself, and this is that
//! code. It runs on the LOOP thread, from `Shell::open`, so the context is current on the thread that
//! renders and no other (the thread rule `eglMakeCurrent` imposes, E6 option B): the window it draws into
//! was made on the host thread and arrives through the hand-off ([`crate::surface`]).
//!
//! `libEGL.so.1` is bound through `plx_base::dynlib!`, all-required: a library that lacks any of these
//! calls is not an EGL 1.4 the renderer can use, and the table is refused as a whole. GL itself is not
//! bound here: `plx_gfx` imports `gl*` at link time (`libGLESv2.so.2`, `docs/platform-ports.md`).
//!
//! The config asks for what the emulator run proved and the renderer needs: an RGBA8888 window config
//! that supports ES2, an ES2 context. UNVERIFIED on a retail set: whether `EGL_ALPHA_SIZE` 8 is offered
//! (the first config the driver returns that matches is used, and the log names the attributes asked).
use plx_base::eventlog::log;
use std::ffi::c_void;
use std::ptr::null_mut;

type Display = *mut c_void;
type Config = *mut c_void;
type Surface = *mut c_void;
type Context = *mut c_void;

plx_base::dynlib! {
    /// EGL 1.4: the calls a window context needs.
    libegl: ["libEGL.so.1"] {
        fn eglGetDisplay(native: *mut c_void) -> Display;
        fn eglInitialize(dpy: Display, major: *mut i32, minor: *mut i32) -> u32;
        fn eglBindAPI(api: u32) -> u32;
        fn eglChooseConfig(dpy: Display, attribs: *const i32, configs: *mut Config, size: i32, count: *mut i32) -> u32;
        fn eglCreateWindowSurface(dpy: Display, config: Config, window: *mut c_void, attribs: *const i32) -> Surface;
        fn eglCreateContext(dpy: Display, config: Config, share: Context, attribs: *const i32) -> Context;
        fn eglMakeCurrent(dpy: Display, draw: Surface, read: Surface, ctx: Context) -> u32;
        fn eglSwapBuffers(dpy: Display, surface: Surface) -> u32;
        fn eglSwapInterval(dpy: Display, interval: i32) -> u32;
        fn eglDestroySurface(dpy: Display, surface: Surface) -> u32;
        fn eglDestroyContext(dpy: Display, ctx: Context) -> u32;
        fn eglTerminate(dpy: Display) -> u32;
        fn eglGetError() -> i32;
    }
}

/// Load the EGL table (idempotent): the window layer asks it on the host thread, before any window exists.
pub(crate) fn load() -> plx_base::dynlib::Loaded {
    libegl::load(None)
}

const EGL_OPENGL_ES_API: u32 = 0x30A0;
const EGL_NONE: i32 = 0x3038;
const EGL_SURFACE_TYPE: i32 = 0x3033;
const EGL_WINDOW_BIT: i32 = 0x0004;
const EGL_RENDERABLE_TYPE: i32 = 0x3040;
const EGL_OPENGL_ES2_BIT: i32 = 0x0004;
const EGL_RED_SIZE: i32 = 0x3024;
const EGL_GREEN_SIZE: i32 = 0x3023;
const EGL_BLUE_SIZE: i32 = 0x3022;
const EGL_ALPHA_SIZE: i32 = 0x3021;
const EGL_CONTEXT_CLIENT_VERSION: i32 = 0x3098;

/// The config request: a window surface, ES2, 8 bits per channel with alpha. `EGL_NONE`-terminated.
pub const CONFIG_ATTRIBS: [i32; 13] = [
    EGL_SURFACE_TYPE,
    EGL_WINDOW_BIT,
    EGL_RENDERABLE_TYPE,
    EGL_OPENGL_ES2_BIT,
    EGL_RED_SIZE,
    8,
    EGL_GREEN_SIZE,
    8,
    EGL_BLUE_SIZE,
    8,
    EGL_ALPHA_SIZE,
    8,
    EGL_NONE,
];

/// The context request: ES 2. `EGL_NONE`-terminated.
pub const CONTEXT_ATTRIBS: [i32; 3] = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];

/// A display, a window surface and an ES2 context, current on the thread that made it.
pub struct Egl {
    display: Display,
    surface: Surface,
    context: Context,
}

impl Egl {
    /// Make the context for `native_window` (a `wl_egl_window*`) on `wl_display`, and make it current on
    /// THIS thread. Every step that can fail is logged with `eglGetError`; whatever was made before the
    /// failure is released.
    pub fn create(wl_display: *mut c_void, native_window: *mut c_void) -> Result<Egl, String> {
        if !crate::ecore::loaded("egl", load()) {
            return Err("egl: libEGL.so.1 is not usable (the log line above says why)".into());
        }
        // SAFETY (whole function): EGL calls with EGL's arguments on a live table; every returned handle is
        // checked before use, and a handle made is destroyed on the error paths below.
        unsafe {
            let display = eglGetDisplay(wl_display);
            if display.is_null() {
                return Err(format!("egl: eglGetDisplay -> null (error {:#x})", eglGetError()));
            }
            let (mut major, mut minor) = (0, 0);
            if eglInitialize(display, &mut major, &mut minor) == 0 {
                return Err(format!("egl: eglInitialize failed (error {:#x})", eglGetError()));
            }
            log(&format!("egl: initialised EGL {major}.{minor}"));
            let made = Self::build(display, native_window);
            if made.is_err() {
                eglTerminate(display);
            }
            made
        }
    }

    /// # Safety
    /// `display` is an initialised EGL display and the table is live.
    unsafe fn build(display: Display, native_window: *mut c_void) -> Result<Egl, String> {
        // SAFETY: by the caller's contract; the attribute arrays are `EGL_NONE`-terminated statics.
        unsafe {
            if eglBindAPI(EGL_OPENGL_ES_API) == 0 {
                return Err(format!("egl: eglBindAPI(ES) failed (error {:#x})", eglGetError()));
            }
            let mut config: Config = null_mut();
            let mut count = 0;
            if eglChooseConfig(display, CONFIG_ATTRIBS.as_ptr(), &mut config, 1, &mut count) == 0 || count < 1 {
                return Err(format!(
                    "egl: no RGBA8888 ES2 window config ({count} matched, error {:#x})",
                    eglGetError()
                ));
            }
            let surface = eglCreateWindowSurface(display, config, native_window, std::ptr::null());
            if surface.is_null() {
                return Err(format!("egl: eglCreateWindowSurface -> null (error {:#x})", eglGetError()));
            }
            let context = eglCreateContext(display, config, null_mut(), CONTEXT_ATTRIBS.as_ptr());
            if context.is_null() {
                let error = eglGetError();
                eglDestroySurface(display, surface);
                return Err(format!("egl: eglCreateContext -> null (error {error:#x})"));
            }
            if eglMakeCurrent(display, surface, surface, context) == 0 {
                let error = eglGetError();
                eglDestroyContext(display, context);
                eglDestroySurface(display, surface);
                return Err(format!("egl: eglMakeCurrent failed (error {error:#x})"));
            }
            log("egl: ES2 window context created and current on the loop thread");
            Ok(Egl { display, surface, context })
        }
    }

    /// A context that holds nothing: every call on it is inert. The stand-in a test gives a surface where
    /// there is no `libEGL`.
    #[cfg(test)]
    pub fn destroyed() -> Egl {
        Egl { display: null_mut(), surface: null_mut(), context: null_mut() }
    }

    /// Present the frame.
    pub fn swap(&self) {
        if self.display.is_null() {
            return;
        }
        // SAFETY: the live display and surface this value owns, on the thread the context is current on.
        unsafe { eglSwapBuffers(self.display, self.surface) };
    }

    /// 1 locks to the panel's refresh, 0 uncaps. A driver that refuses is not an error (the Mali's is
    /// inert, `eglSwapInterval` there changes nothing); the answer is not read.
    pub fn set_interval(&self, interval: i32) {
        if self.display.is_null() {
            return;
        }
        // SAFETY: as `swap`.
        unsafe { eglSwapInterval(self.display, interval) };
    }

    /// Release the context from this thread and destroy the surface, the context and the display
    /// connection. LOOP thread, before the window's own teardown. Idempotent.
    pub fn destroy(&mut self) {
        if self.display.is_null() {
            return;
        }
        // SAFETY: the handles `build` made, destroyed once and cleared.
        unsafe {
            eglMakeCurrent(self.display, null_mut(), null_mut(), null_mut());
            eglDestroySurface(self.display, self.surface);
            eglDestroyContext(self.display, self.context);
            eglTerminate(self.display);
        }
        (self.display, self.surface, self.context) = (null_mut(), null_mut(), null_mut());
        log("egl: context and surface destroyed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both attribute lists are `EGL_NONE`-terminated and the config list is key/value pairs, which is what
    /// `eglChooseConfig` reads: a missing terminator walks off the array.
    #[test]
    fn the_attribute_lists_are_pairs_ending_in_none() {
        assert_eq!(CONFIG_ATTRIBS.last(), Some(&EGL_NONE));
        assert_eq!((CONFIG_ATTRIBS.len() - 1) % 2, 0);
        assert!(CONFIG_ATTRIBS[..CONFIG_ATTRIBS.len() - 1].chunks(2).all(|p| p[0] != EGL_NONE));
        assert_eq!(CONTEXT_ATTRIBS, [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE]);
    }

    /// No `libEGL` on the host (or none this crate's table is complete against): one refusal that says so,
    /// and nothing was called.
    #[test]
    fn without_the_library_the_context_is_refused_with_a_reason() {
        if libegl::load(None).ok() {
            return; // a host with a complete EGL is not the absent-library case; it is never driven from a test
        }
        let result = plx_base::eventlog::with_private_log(|| Egl::create(null_mut(), null_mut()));
        match result {
            Err(why) => assert!(why.starts_with("egl: "), "{why}"),
            // A host that has a complete EGL and a null display answers some other error, never success.
            Ok(_) => panic!("a null native window cannot make a context"),
        }
    }
}
