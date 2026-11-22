//! The native window and the remote's key events: Ecore_Wl2, bound at run time.
//!
//! The sequence is the flutter-tizen embedder's (`tizen_window_ecore_wl2.cc`), which an emulator run of
//! this crate's prototype proved (`docs/platform-ports.md`, E6, E8, E11): `ecore_wl2_init`, connect the
//! display, sync, read the screen size, create a window of that size, make an EGL window for it, set the
//! window options, register the key and focus handlers, then show. **Every call here is on the host's main
//! thread** (Ecore is not thread-safe and its main loop is that thread): `plx_host_start` calls
//! [`Window::create`] and [`Window::show`]; the handlers run on that thread; and the window's teardown is
//! queued back to it through the host's `run_on_host` ([`Window::destroy`]). The loop thread touches only
//! the EGL surface, which it makes from [`Window::native_window`].
//!
//! The libraries are `dlopen`ed through `plx_base::dynlib!` (a `.so` linked against none of them still loads
//! on an image that lacks one, and a missing library is one logged refusal, not a loader abort). Each table
//! is all-or-nothing, so the optional symbols live in tables of their own: the embedder's EGL-window path
//! ([`ecore_wl2_egl`]) with `libwayland-egl` ([`wayland_egl`]) as the logged fallback, and the window
//! options ([`ecore_wl2_opts`]). The Ecore event ids are DATA symbols, which `dynlib!` does not bind: they
//! are read through `Handle::sym` ([`event_ids`]). The handler functions are bound from `libecore_wl2.so.1`:
//! that is the handle the emulator run resolved them from (E6), and it reaches `libecore`'s definition
//! through its own dependency. There is no second candidate: [`Window::create`] has already refused when
//! `libecore_wl2` does not open, so a fallback to `libecore.so.1` could never be reached.
//!
//! **UNVERIFIED on a retail set: the layout of `Ecore_Event_Key`.** Only its first two pointers (`keyname`,
//! `key`, first in every EFL version) and `keycode` are read, as the emulator run did; a wrong offset reads
//! a wrong number or text, it does not corrupt anything, and the key log line shows what was read.
use crate::host_abi;
use crate::keys::{self, Decoder};
use plx_base::dynlib::{Handle, Loaded};
use plx_base::eventlog::log;
use std::ffi::{c_char, c_int, c_void, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr::{null, null_mut};
use std::sync::Mutex;

/// `Eina_Bool (*Ecore_Event_Handler_Cb)(void *data, int type, void *event)`; `Eina_Bool` is an `unsigned
/// char`. A handler returns `ECORE_CALLBACK_PASS_ON` (1) so the system's own key handling still runs.
pub type EventCb = unsafe extern "C" fn(*mut c_void, c_int, *mut c_void) -> u8;

plx_base::dynlib! {
    /// The window: what the embedder cannot work without.
    ecore_wl2: ["libecore_wl2.so.1"] {
        fn ecore_wl2_init() -> c_int;
        fn ecore_wl2_display_connect(name: *const c_char) -> *mut c_void;
        fn ecore_wl2_display_get(display: *mut c_void) -> *mut c_void;
        fn ecore_wl2_sync();
        fn ecore_wl2_display_screen_size_get(display: *mut c_void, w: *mut c_int, h: *mut c_int);
        fn ecore_wl2_window_new(display: *mut c_void, parent: *mut c_void, x: c_int, y: c_int, w: c_int, h: c_int) -> *mut c_void;
        fn ecore_wl2_window_show(window: *mut c_void);
    }
}

plx_base::dynlib! {
    /// The embedder's way to a `wl_egl_window`.
    ecore_wl2_egl: ["libecore_wl2.so.1"] {
        fn ecore_wl2_egl_window_create(window: *mut c_void, w: c_int, h: c_int) -> *mut c_void;
        fn ecore_wl2_egl_window_native_get(egl_window: *mut c_void) -> *mut c_void;
        fn ecore_wl2_egl_window_destroy(egl_window: *mut c_void);
    }
}

plx_base::dynlib! {
    /// The fallback's first half: the window's `wl_surface`.
    ecore_wl2_surface: ["libecore_wl2.so.1"] {
        fn ecore_wl2_window_surface_get(window: *mut c_void) -> *mut c_void;
    }
}

plx_base::dynlib! {
    /// The fallback's second half, `libwayland-egl`: a `wl_egl_window` made from the `wl_surface`.
    wayland_egl: ["libwayland-egl.so.1"] {
        fn wl_egl_window_create(surface: *mut c_void, w: c_int, h: c_int) -> *mut c_void;
        fn wl_egl_window_destroy(window: *mut c_void);
    }
}

plx_base::dynlib! {
    /// The window options the emulator run applied (E6): the toplevel type and the alpha.
    ecore_wl2_opts: ["libecore_wl2.so.1"] {
        fn ecore_wl2_window_type_set(window: *mut c_void, kind: c_int);
        fn ecore_wl2_window_alpha_set(window: *mut c_void, alpha: u8);
    }
}

plx_base::dynlib! {
    /// The options only the embedder sets (position, the geometry aux hint, the rotations): their presence
    /// on the emulator image is unrecorded, so they are a table of their own and a missing one costs only
    /// these three, not the two above.
    ecore_wl2_opts_extra: ["libecore_wl2.so.1"] {
        fn ecore_wl2_window_position_set(window: *mut c_void, x: c_int, y: c_int);
        fn ecore_wl2_window_aux_hint_add(window: *mut c_void, id: c_int, hint: *const c_char, val: *const c_char);
        fn ecore_wl2_window_available_rotations_set(window: *mut c_void, rotations: *const c_int, count: u32);
    }
}

plx_base::dynlib! {
    /// The embedder's `DestroyWindow()`.
    ecore_wl2_end: ["libecore_wl2.so.1"] {
        fn ecore_wl2_window_free(window: *mut c_void);
        fn ecore_wl2_display_disconnect(display: *mut c_void);
        fn ecore_wl2_shutdown() -> c_int;
    }
}

plx_base::dynlib! {
    /// The event handlers, from the handle the emulator run resolved them from; see the module note.
    ecore_events: ["libecore_wl2.so.1"] {
        fn ecore_event_handler_add(kind: c_int, cb: EventCb, data: *const c_void) -> *mut c_void;
        fn ecore_event_handler_del(handler: *mut c_void) -> *mut c_void;
    }
}

plx_base::dynlib! {
    /// `ecore_event_init`, the retry when an event id still reads 0 (a one-entry table of its own: a missing
    /// symbol costs only the retry).
    ecore_input: ["libecore_input.so.1"] {
        fn ecore_event_init() -> c_int;
    }
}

/// `ECORE_WL2_WINDOW_TYPE_TOPLEVEL` in EFL's `Ecore_Wl2_Window_Type` (NONE = 0, TOPLEVEL = 1). From the
/// upstream header, not from a device: UNVERIFIED.
const WINDOW_TYPE_TOPLEVEL: c_int = 1;

/// **The window is translucent: this platform's frames go over a video plane.** The `alpha` argument of
/// `ecore_wl2_window_alpha_set`, which the window gets at creation and keeps for the process (a flag of the
/// window, so there is nothing to route per frame and `Shell::opaque_route` has no work). Measured on the TV 10.0
/// emulator (2026-11-22): with it ON the native player's overlay video shows only through the pixels the frame
/// clears to (0, 0, 0, 0) and the UI around them is intact. With it OFF (what this crate set before) the window
/// is opaque and no video can show.
///
/// **Which frames clear to (0, 0, 0, 0).** The pages with a ground clear to an opaque colour (Home, detail,
/// library, search: `frame_clear`), so they are as opaque as before. The PLAYER ROUTE clears to (0, 0, 0, 0)
/// whenever `fr.player` is set, bound plane or not (`app/run.rs`), and `PlayerScreen` paints no ground, so its
/// spinner and its "Playback failed" read-out are drawn over a transparent clear even when no plane is bound.
/// Measured on the emulator at `3d56d35b2` (no URI driver yet, so the plane is never bound): hero Play opens the
/// read-out, and behind it the emulator shows exactly (0, 0, 0) black (5778 of 5784 samples; the rest is the
/// footer text), unchanged from +0.49 s to +6.86 s, with the text and pills drawn cleanly and Back returning to an
/// opaque Home. The caveat: the EMULATOR's compositor had nothing under the window. What a retail set composites
/// under a (0, 0, 0, 0) clear with no player is unmeasured and belongs to H6.
pub const WINDOW_ALPHA: u8 = 1;

/// Load one table, saying why when it does not load; `true` when it is live.
pub(crate) fn loaded(what: &str, result: Loaded) -> bool {
    match result {
        Loaded::Ok(soname) => {
            log(&format!("ecore: {what} bound from {soname}"));
            true
        }
        Loaded::NoLibrary => {
            log(&format!("ecore: {what}: no candidate library opened"));
            false
        }
        Loaded::Incomplete(soname, missing) => {
            log(&format!("ecore: {what}: {soname} lacks {missing} symbol(s)"));
            false
        }
    }
}

/// One library table to load: its name for the log, and its loader.
type Required = (&'static str, fn() -> Loaded);

/// The tables [`Window::create`] cannot work without, and the EGL table the loop thread needs: a missing
/// `libEGL` is refused here, on the host thread, as `PLX_HOST_E_WINDOW` (the header's promise), not found
/// later by the loop thread as an `OK` start that ends in `on_stopped(1, 0)`.
fn required_libraries() -> [Required; 4] {
    [
        ("ecore_wl2", || ecore_wl2::load(None)),
        ("ecore_wl2_end", || ecore_wl2_end::load(None)),
        ("ecore_events", || ecore_events::load(None)),
        ("egl", crate::egl::load),
    ]
}

/// Load every table in `required`, logging each outcome (none is skipped after a failure, so one start says
/// everything an image lacks); `true` when all are live.
fn all_loaded(required: &[Required]) -> bool {
    let mut all = true;
    for (what, load) in required {
        all &= loaded(what, load());
    }
    all
}

/// The native window and everything Ecore made for it. Raw handles only: the struct is moved once, to the
/// loop thread's surface, which uses [`Window::native_window`] for EGL and hands the whole value back to the
/// host thread to destroy.
pub struct Window {
    display: *mut c_void,
    /// The `wl_display` `eglGetDisplay` is given.
    pub wl_display: *mut c_void,
    window: *mut c_void,
    /// What to destroy: an `Ecore_Wl2_Egl_Window*`, or on the fallback path the `wl_egl_window*`.
    egl_handle: *mut c_void,
    /// The `wl_egl_window*`: the `EGLNativeWindowType` for `eglCreateWindowSurface`.
    pub native_window: *mut c_void,
    via_ecore: bool,
    handlers: Vec<*mut c_void>,
    /// `ecore_wl2_init` succeeded and `ecore_wl2_shutdown` has not run: nothing may be called otherwise.
    initialised: bool,
    /// The screen size Ecore reported, which is the window's size.
    pub width: i32,
    pub height: i32,
}

impl crate::hosted::Made for Window {
    fn unmake(mut self) {
        self.destroy();
    }
}

// SAFETY: the handles are opaque tokens. After the one move to the loop thread they are used for EGL
// (native_window, wl_display) there, and the host thread alone calls Ecore with them (`destroy`).
unsafe impl Send for Window {}

impl Window {
    /// Steps 1-6 and the options, then the handlers. HOST THREAD. Every failure is logged where it happens
    /// and answers [`host_abi::E_WINDOW`]; what was made is released first.
    pub fn create() -> Result<Window, i32> {
        if !all_loaded(&required_libraries()) {
            return Err(host_abi::E_WINDOW);
        }
        // SAFETY (whole function): the embedder's calls with the embedder's arguments; every returned
        // handle is null-checked before use, and the tables above are live.
        let inits = unsafe { ecore_wl2_init() };
        log(&format!("ecore: ecore_wl2_init -> {inits}"));
        if inits < 1 {
            return Err(host_abi::E_WINDOW);
        }
        let mut window = Window {
            display: null_mut(),
            wl_display: null_mut(),
            window: null_mut(),
            egl_handle: null_mut(),
            native_window: null_mut(),
            via_ecore: false,
            handlers: Vec::new(),
            initialised: true,
            width: 0,
            height: 0,
        };
        if let Err(code) = window.open() {
            window.destroy();
            return Err(code);
        }
        window.set_options();
        if let Err(code) = window.register_handlers() {
            window.destroy();
            return Err(code);
        }
        Ok(window)
    }

    /// A window that holds nothing and calls nothing when destroyed: the stand-in a test hands the loop
    /// where the platform libraries are absent.
    #[cfg(test)]
    pub fn detached() -> Window {
        Window {
            display: null_mut(),
            wl_display: null_mut(),
            window: null_mut(),
            egl_handle: null_mut(),
            native_window: null_mut(),
            via_ecore: false,
            handlers: Vec::new(),
            initialised: false,
            width: 1920,
            height: 1080,
        }
    }

    fn open(&mut self) -> Result<(), i32> {
        // SAFETY: see `create`.
        unsafe {
            self.display = ecore_wl2_display_connect(null());
            log(&format!("ecore: display_connect -> {:p}", self.display));
            if self.display.is_null() {
                return Err(host_abi::E_WINDOW);
            }
            self.wl_display = ecore_wl2_display_get(self.display);
            if self.wl_display.is_null() {
                log("ecore: display_get -> null");
                return Err(host_abi::E_WINDOW);
            }
            ecore_wl2_sync();
            let (mut w, mut h) = (0, 0);
            ecore_wl2_display_screen_size_get(self.display, &mut w, &mut h);
            log(&format!(
                "ecore: screen size {w}x{h}{}",
                if (w, h) == (1920, 1080) { "" } else { " (NOT 1920x1080: the drawable size is not published from here, UNHANDLED)" }
            ));
            if w <= 0 || h <= 0 {
                return Err(host_abi::E_WINDOW);
            }
            (self.width, self.height) = (w, h);
            self.window = ecore_wl2_window_new(self.display, null_mut(), 0, 0, w, h);
            log(&format!("ecore: window_new({w}x{h}) -> {:p}", self.window));
            if self.window.is_null() {
                return Err(host_abi::E_WINDOW);
            }
        }
        self.make_egl_window()
    }

    /// The embedder's path first (`ecore_wl2_egl_window_create` + `_native_get`), then `libwayland-egl`'s
    /// `wl_egl_window_create(wl_surface, w, h)` as a logged fallback.
    fn make_egl_window(&mut self) -> Result<(), i32> {
        // SAFETY: see `create`.
        unsafe {
            if loaded("ecore_wl2_egl", ecore_wl2_egl::load(None)) {
                let handle = ecore_wl2_egl_window_create(self.window, self.width, self.height);
                let native = if handle.is_null() { null_mut() } else { ecore_wl2_egl_window_native_get(handle) };
                log(&format!("ecore: egl_window_create -> {handle:p}, native_get -> {native:p}"));
                if !native.is_null() {
                    (self.egl_handle, self.native_window, self.via_ecore) = (handle, native, true);
                    return Ok(());
                }
                if !handle.is_null() {
                    ecore_wl2_egl_window_destroy(handle);
                }
                log("ecore: the Ecore EGL-window path failed; trying libwayland-egl");
            }
            if !(loaded("ecore_wl2_surface", ecore_wl2_surface::load(None)) && loaded("wayland_egl", wayland_egl::load(None))) {
                return Err(host_abi::E_WINDOW);
            }
            let surface = ecore_wl2_window_surface_get(self.window);
            let native = if surface.is_null() { null_mut() } else { wl_egl_window_create(surface, self.width, self.height) };
            log(&format!("ecore: window_surface_get -> {surface:p}, wl_egl_window_create -> {native:p}"));
            if native.is_null() {
                return Err(host_abi::E_WINDOW);
            }
            (self.egl_handle, self.native_window, self.via_ecore) = (native, native, false);
        }
        Ok(())
    }

    /// The embedder's `SetWindowOptions()` minus the phone-only indicator calls. A missing table is one
    /// logged line: the window works without the options.
    fn set_options(&self) {
        // SAFETY: a live window and the live tables; the strings outlive the calls.
        unsafe {
            if loaded("ecore_wl2_opts", ecore_wl2_opts::load(None)) {
                ecore_wl2_window_type_set(self.window, WINDOW_TYPE_TOPLEVEL);
                ecore_wl2_window_alpha_set(self.window, WINDOW_ALPHA);
            }
            if loaded("ecore_wl2_opts_extra", ecore_wl2_opts_extra::load(None)) {
                let hint = CString::new("wm.policy.win.user.geometry").unwrap_or_default();
                let value = CString::new("1").unwrap_or_default();
                let rotations: [c_int; 1] = [0];
                ecore_wl2_window_position_set(self.window, 0, 0);
                ecore_wl2_window_aux_hint_add(self.window, 0, hint.as_ptr(), value.as_ptr());
                ecore_wl2_window_available_rotations_set(self.window, rotations.as_ptr(), 1);
            }
        }
    }

    /// Key down and up (required) and focus in and out (a log line each, for the diagnostics).
    fn register_handlers(&mut self) -> Result<(), i32> {
        let Some(ids) = event_ids() else { return Err(host_abi::E_WINDOW) };
        for (what, kind, cb, data) in [
            ("key down", Some(ids.key_down), key_cb as EventCb, 1usize),
            ("key up", Some(ids.key_up), key_cb as EventCb, 0),
            ("focus in", ids.focus_in, focus_cb as EventCb, 1),
            ("focus out", ids.focus_out, focus_cb as EventCb, 0),
        ] {
            let Some(kind) = kind else { continue }; // a focus id that was absent: logged by `ids_from`
            // SAFETY: the live events table; `data` is a tag the callbacks read, never dereferenced.
            let handler = unsafe { ecore_event_handler_add(kind, cb, data as *const c_void) };
            log(&format!("ecore: handler {what} (event {kind}) -> {handler:p}"));
            if handler.is_null() {
                // The key handlers are the point; the focus ones are a diagnostic.
                if what.starts_with("key") {
                    return Err(host_abi::E_WINDOW);
                }
            } else {
                self.handlers.push(handler);
            }
        }
        Ok(())
    }

    /// Show the window, as the embedder does last. HOST THREAD.
    pub fn show(&self) {
        // SAFETY: a live window.
        unsafe { ecore_wl2_window_show(self.window) };
        log("ecore: window shown");
    }

    /// The embedder's `DestroyWindow()`: the handlers, the EGL window, the window, the display, Ecore.
    /// HOST THREAD, after the EGL surface is gone. Idempotent: every handle is cleared after its use.
    pub fn destroy(&mut self) {
        if !self.initialised {
            return;
        }
        self.initialised = false;
        // SAFETY: handles made by `create`, each cleared after use; the tables were live to make them.
        unsafe {
            for handler in self.handlers.drain(..) {
                ecore_event_handler_del(handler);
            }
            if !self.egl_handle.is_null() {
                if self.via_ecore {
                    ecore_wl2_egl_window_destroy(self.egl_handle);
                } else {
                    wl_egl_window_destroy(self.egl_handle);
                }
                (self.egl_handle, self.native_window) = (null_mut(), null_mut());
            }
            if !self.window.is_null() {
                ecore_wl2_window_free(self.window);
                self.window = null_mut();
            }
            if !self.display.is_null() {
                ecore_wl2_display_disconnect(self.display);
                self.display = null_mut();
            }
            log(&format!("ecore: window torn down; ecore_wl2_shutdown -> {}", ecore_wl2_shutdown()));
        }
    }
}

/// The four Ecore event ids the handlers register for.
struct EventIds {
    key_down: c_int,
    key_up: c_int,
    /// Diagnostic only: a missing one is logged and its handler is not registered.
    focus_in: Option<c_int>,
    focus_out: Option<c_int>,
}

/// Read an event id, a DATA symbol (`int ECORE_EVENT_KEY_DOWN`), which `dynlib!` does not bind.
fn id_of(handles: &[Handle], name: &str) -> Option<c_int> {
    handles.iter().find_map(|h| h.sym(name).filter(|p| !p.is_null())).map(|p| {
        // SAFETY: the symbol is an `int` variable in a mapped library.
        unsafe { p.cast::<c_int>().read() }
    })
}

/// The ids, as `libecore_input` and `libecore_wl2` allocated them. `ecore_wl2_init` normally initialises
/// the input library; if a KEY id still reads 0, `ecore_event_init` is called once and the ids are read again.
fn event_ids() -> Option<EventIds> {
    let Some((input, _)) = Handle::open(&["libecore_input.so.1"]) else {
        log("ecore: libecore_input.so.1 did not open");
        return None;
    };
    let Some((wl2, _)) = Handle::open(&["libecore_wl2.so.1"]) else {
        log("ecore: libecore_wl2.so.1 did not open (event ids)");
        return None;
    };
    ids_from(
        |name| id_of(&[if name.starts_with("ECORE_WL2_") { wl2 } else { input }], name),
        || {
            if !loaded("ecore_input", ecore_input::load(None)) {
                return None;
            }
            // SAFETY: `int ecore_event_init(void)` from a live table, called on the host thread that owns Ecore.
            Some(unsafe { ecore_event_init() })
        },
    )
}

/// [`event_ids`] over the two readers: `read` answers a data symbol's value, `init` runs `ecore_event_init`
/// (`None`: it could not be called). The key ids are required: a missing or 0 one runs `init` once, then they
/// are read again, and a start that still has no key id is refused. The focus ids are not: each missing one is
/// logged by name and never causes the retry or the refusal.
fn ids_from(read: impl Fn(&str) -> Option<c_int>, init: impl FnOnce() -> Option<c_int>) -> Option<EventIds> {
    let keys = |say: bool| {
        let one = |name: &str| {
            let id = read(name);
            if id.is_none() && say {
                log(&format!("ecore: symbol {name} not found"));
            }
            id
        };
        match (one("ECORE_EVENT_KEY_DOWN"), one("ECORE_EVENT_KEY_UP")) {
            (Some(down), Some(up)) if down != 0 && up != 0 => Some((down, up)),
            _ => None,
        }
    };
    let (key_down, key_up) = match keys(false) {
        Some(ids) => ids,
        None => {
            match init() {
                Some(n) => log(&format!("ecore: a key event id was absent or 0; ecore_event_init -> {n}")),
                None => log("ecore: a key event id was absent or 0, and ecore_event_init could not be called"),
            }
            let Some(ids) = keys(true) else {
                log("ecore: the key event ids could not be read");
                return None;
            };
            ids
        }
    };
    let focus = |name: &str| {
        let id = read(name);
        if id.is_none() {
            log(&format!("ecore: symbol {name} not found (no focus diagnostics for it)"));
        }
        id
    };
    Some(EventIds {
        key_down,
        key_up,
        focus_in: focus("ECORE_WL2_EVENT_FOCUS_IN"),
        focus_out: focus("ECORE_WL2_EVENT_FOCUS_OUT"),
    })
}

/// Mirror of EFL's `Ecore_Event_Key` as far as `keycode` (see the module's UNVERIFIED note).
#[repr(C)]
struct EcoreEventKey {
    keyname: *const c_char,
    key: *const c_char,
    string: *const c_char,
    compose: *const c_char,
    window: usize,
    root_window: usize,
    event_window: usize,
    timestamp: u32,
    modifiers: u32,
    same_screen: c_int,
    keycode: u32,
}

/// The longest key name read through an event's pointer (key names are a few characters).
const NAME_LIMIT: usize = 64;

/// A `const char *` field as text: `None` for null or empty. Bounded: at most [`NAME_LIMIT`] bytes are read.
///
/// # Safety
/// `p` is null or points at a readable NUL-terminated string, or at [`NAME_LIMIT`] readable bytes.
unsafe fn text_of(p: *const c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let mut bytes = Vec::new();
    for i in 0..NAME_LIMIT {
        // SAFETY: by the caller's contract, one byte at a time, stopping at the NUL.
        let b = unsafe { p.add(i).cast::<u8>().read() };
        if b == 0 {
            break;
        }
        bytes.push(b);
    }
    (!bytes.is_empty()).then(|| String::from_utf8_lossy(&bytes).into_owned())
}

/// Held keys, for the repeat edge. The handlers run on the host thread; the lock is only ever contended by
/// nobody.
static DECODER: Mutex<Decoder> = Mutex::new(Decoder::new());

fn on_key(down: bool, event: *mut c_void) {
    if event.is_null() {
        return;
    }
    // SAFETY: Ecore's event, laid out as `EcoreEventKey` as far as read (the module's UNVERIFIED note);
    // the strings are read bounded.
    let (name, code) = unsafe {
        let ev = &*event.cast::<EcoreEventKey>();
        (text_of(ev.keyname).or_else(|| text_of(ev.key)), ev.keycode)
    };
    let mut decoder = DECODER.lock().unwrap_or_else(|e| e.into_inner());
    keys::feed(&mut decoder, name.as_deref().unwrap_or(""), code, down, |action, edge| {
        // A full queue or a stopped library drops the press: nothing here may wait.
        let _ = crate::HOSTED.push(crate::hosted::Input::Key(action, edge));
    });
}

/// `ECORE_EVENT_KEY_DOWN` (`data` = 1) and `ECORE_EVENT_KEY_UP` (`data` = 0). Never unwinds into Ecore.
unsafe extern "C" fn key_cb(data: *mut c_void, _kind: c_int, event: *mut c_void) -> u8 {
    let down = !data.is_null();
    if catch_unwind(AssertUnwindSafe(|| on_key(down, event))).is_err() {
        log("ecore: PANIC in the key callback (swallowed)");
    }
    1
}

/// Focus in (`data` = 1) and out (0): whether the window has focus decides whether keys arrive at all, so
/// the log says so. The event body is not read.
unsafe extern "C" fn focus_cb(data: *mut c_void, _kind: c_int, _event: *mut c_void) -> u8 {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if data.is_null() {
            log("ecore: window focus OUT");
            // The up of a held key goes to whoever took the focus: release them here (see `keys::Decoder`).
            let released = DECODER.lock().unwrap_or_else(|e| e.into_inner()).release_all();
            for (action, edge) in released {
                let _ = crate::HOSTED.push(crate::hosted::Input::Key(action, edge));
            }
        } else {
            log("ecore: window focus IN");
        }
    }));
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The window is created translucent, and by the named value.** The video plane shows through a window
    /// whose alpha is on (E-measurement 2026-11-22, `docs/tizen.md`); the call is made in `set_options`, from
    /// [`WINDOW_ALPHA`], and not from a literal that could read 0 again.
    #[test]
    fn the_window_is_created_translucent_from_the_named_value() {
        assert_eq!(WINDOW_ALPHA, 1, "alpha ON: the overlay video shows through a (0, 0, 0, 0) clear");
        let src = include_str!("ecore.rs");
        let call = "ecore_wl2_window_alpha_set"; // built apart, so this test's own text is not what it finds
        assert_eq!(src.matches(&format!("{call}(self.window, WINDOW_ALPHA);")).count(), 1);
        assert!(!src.contains(&format!("{call}(self.window, 0);")), "no opaque window");
    }

    /// **No Ecore, no window, no crash**: on a host without `libecore_wl2` (every host this runs on) the window
    /// layer answers one clean refusal, `E_WINDOW`, and logs which library did not open. This is the path a
    /// device without the libraries takes, and the one every other test relies on staying inert.
    #[test]
    fn without_the_libraries_the_window_layer_refuses_cleanly() {
        if ecore_wl2::load(None).ok() {
            return; // a host that has EFL is not the absent-library case this tests
        }
        let log = plx_base::eventlog::with_private_log(|| {
            assert!(matches!(Window::create(), Err(code) if code == host_abi::E_WINDOW));
            std::fs::read_to_string(plx_base::eventlog::events_log()).unwrap_or_default()
        });
        assert!(log.contains("ecore: ecore_wl2: no candidate library opened"), "{log}");
        assert!(log.contains("ecore: egl"), "the EGL table is part of the same refusal: {log}");
        assert_eq!(log.lines().count(), 4, "one line per table, and no Ecore call was attempted: {log}");
    }

    /// **A missing libEGL is refused on the host thread**, in the same chain as the Ecore tables, so the start
    /// answers `E_WINDOW` (the header's promise) instead of `OK` and an `on_stopped(1, 0)` from the loop thread.
    /// Every table is tried and logged even after one fails; only a fully live set passes.
    #[test]
    fn a_missing_egl_library_fails_the_window_chain_and_names_itself() {
        let names: Vec<&str> = required_libraries().iter().map(|(n, _)| *n).collect();
        assert_eq!(names, ["ecore_wl2", "ecore_wl2_end", "ecore_events", "egl"]);
        let live: Required = ("a", || Loaded::Ok("liba.so.1"));
        let no_egl: Required = ("egl", || Loaded::NoLibrary);
        let log = plx_base::eventlog::with_private_log(|| {
            assert!(all_loaded(&[live, live]));
            assert!(!all_loaded(&[live, no_egl, live]));
            std::fs::read_to_string(plx_base::eventlog::events_log()).unwrap_or_default()
        });
        assert!(log.contains("ecore: egl: no candidate library opened"), "{log}");
        assert_eq!(log.matches("ecore: a bound from liba.so.1").count(), 4, "a failure skips nothing: {log}");
    }

    fn reader(table: &'static [(&'static str, c_int)]) -> impl Fn(&str) -> Option<c_int> {
        move |name| table.iter().find(|(n, _)| *n == name).map(|(_, v)| *v)
    }

    /// Ids that read non-zero never call `ecore_event_init`.
    #[test]
    fn event_ids_that_read_fine_need_no_retry() {
        let ids = ids_from(
            reader(&[
                ("ECORE_EVENT_KEY_DOWN", 7),
                ("ECORE_EVENT_KEY_UP", 8),
                ("ECORE_WL2_EVENT_FOCUS_IN", 9),
                ("ECORE_WL2_EVENT_FOCUS_OUT", 10),
            ]),
            || panic!("no retry"),
        )
        .expect("ids");
        assert_eq!((ids.key_down, ids.key_up, ids.focus_in, ids.focus_out), (7, 8, Some(9), Some(10)));
    }

    /// **A missing focus symbol is not a key problem**: no retry, no refusal, the symbol is named in the log
    /// and only its handler is skipped (`register_handlers` treats focus as a diagnostic).
    #[test]
    fn a_missing_focus_id_is_logged_by_name_and_never_retries_or_refuses() {
        let log = plx_base::eventlog::with_private_log(|| {
            let ids = ids_from(
                reader(&[("ECORE_EVENT_KEY_DOWN", 7), ("ECORE_EVENT_KEY_UP", 8), ("ECORE_WL2_EVENT_FOCUS_IN", 9)]),
                || panic!("a focus id is not a reason to retry"),
            )
            .expect("the keys are enough");
            assert_eq!((ids.key_down, ids.key_up, ids.focus_in, ids.focus_out), (7, 8, Some(9), None));
            std::fs::read_to_string(plx_base::eventlog::events_log()).unwrap_or_default()
        });
        assert!(log.contains("ECORE_WL2_EVENT_FOCUS_OUT not found"), "{log}");
        assert!(!log.contains("key event ids could not be read"), "{log}");
    }

    /// A key id that reads 0 runs `ecore_event_init` once and is read again; one that stays 0 or absent is
    /// refused, with the missing symbol named.
    #[test]
    fn a_zero_key_id_retries_once_and_a_missing_one_is_refused_by_name() {
        use std::cell::Cell;
        let calls = Cell::new(0);
        let after = Cell::new(false);
        let read = |name: &str| match name {
            "ECORE_EVENT_KEY_DOWN" => Some(if after.get() { 5 } else { 0 }),
            "ECORE_EVENT_KEY_UP" => Some(6),
            _ => Some(1),
        };
        let ids = ids_from(read, || {
            calls.set(calls.get() + 1);
            after.set(true);
            Some(1)
        })
        .expect("the retry made the id");
        assert_eq!((calls.get(), ids.key_down, ids.key_up), (1, 5, 6));

        let log = plx_base::eventlog::with_private_log(|| {
            let calls = Cell::new(0);
            let refused = ids_from(reader(&[("ECORE_EVENT_KEY_DOWN", 7)]), || {
                calls.set(calls.get() + 1);
                Some(0)
            });
            assert!(refused.is_none());
            assert_eq!(calls.get(), 1, "one retry, no more");
            std::fs::read_to_string(plx_base::eventlog::events_log()).unwrap_or_default()
        });
        assert!(log.contains("symbol ECORE_EVENT_KEY_UP not found"), "{log}");
        assert!(log.contains("the key event ids could not be read"), "{log}");
    }

    /// Each way a table can fail to load is one log line that names the table and the reason, and only a
    /// complete table is live. (The mapper the EGL layer shares.)
    #[test]
    fn a_load_result_is_one_line_and_only_a_complete_table_is_live() {
        let log = plx_base::eventlog::with_private_log(|| {
            assert!(loaded("t", Loaded::Ok("libx.so.1")));
            assert!(!loaded("t", Loaded::NoLibrary));
            assert!(!loaded("t", Loaded::Incomplete("libx.so.1", 2)));
            std::fs::read_to_string(plx_base::eventlog::events_log()).unwrap_or_default()
        });
        assert_eq!(
            log.lines().collect::<Vec<_>>(),
            [
                "ecore: t bound from libx.so.1",
                "ecore: t: no candidate library opened",
                "ecore: t: libx.so.1 lacks 2 symbol(s)"
            ]
        );
    }

    /// A key name is read bounded and a null or empty one is no name.
    #[test]
    fn a_key_name_is_read_bounded() {
        let name = CString::new("XF86AudioPlay").unwrap();
        // SAFETY: live CStrings.
        unsafe {
            assert_eq!(text_of(name.as_ptr()).as_deref(), Some("XF86AudioPlay"));
            assert_eq!(text_of(null()), None);
            assert_eq!(text_of(c"".as_ptr()), None);
            let long = CString::new("k".repeat(500)).unwrap();
            assert_eq!(text_of(long.as_ptr()).map(|s| s.len()), Some(NAME_LIMIT));
        }
    }

    /// The key callback reads the event the way the emulator run's layout does and queues the press.
    #[test]
    fn the_key_callback_queues_the_press_of_the_event_it_is_given() {
        let name = CString::new("Return").unwrap();
        let event = EcoreEventKey {
            keyname: name.as_ptr(),
            key: name.as_ptr(),
            string: null(),
            compose: null(),
            window: 0,
            root_window: 0,
            event_window: 0,
            timestamp: 1,
            modifiers: 0,
            same_screen: 0,
            keycode: 36,
        };
        // The process-wide `HOSTED` is not running in a unit test, so the push is refused: what is proven
        // is that the callback decodes without faulting, passes the event on (1) and holds no key.
        plx_base::eventlog::with_private_log(|| {
            let ev = std::ptr::from_ref(&event).cast_mut().cast::<c_void>();
            // SAFETY: a live event of the mirrored layout; data 1 = key down.
            assert_eq!(unsafe { key_cb(1 as *mut c_void, 0, ev) }, 1);
            assert_eq!(unsafe { key_cb(null_mut(), 0, ev) }, 1);
            let log = std::fs::read_to_string(plx_base::eventlog::events_log()).unwrap_or_default();
            assert_eq!(
                log.lines().collect::<Vec<_>>(),
                ["key: down name=\"Return\" code=36 -> Ok Down", "key: up name=\"Return\" code=36 -> Ok Up"]
            );
        });
    }
}
