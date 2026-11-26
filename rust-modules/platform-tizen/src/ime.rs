//! The system on-screen keyboard: Ecore IMF, bound at run time.
//!
//! The application asks for the keyboard through `Shell::show_keyboard` / `hide_keyboard` on its loop thread
//! (`textinput::start` / `stop`, which keep the "have I asked" edge); the Tizen shell queues [`show`] and
//! [`hide`] to the host's main thread through `run_on_host`, because **every Ecore IMF call is on that
//! thread**, as every Ecore call in [`crate::ecore`] is. What the panel types arrives in the COMMIT callback
//! ([`commit_cb`]), which pushes it onto the input queue as [`Input::Text`]; the loop hands it to
//! `EventSink::text`, the same door an SDL `SDL_TEXTINPUT` comes through.
//!
//! The sequence is the flutter-tizen embedder's (`tizen_input_method_context.cc`): `ecore_imf_init`, a context
//! from `ecore_imf_context_default_id_get` + `ecore_imf_context_add`, the window set as the client by its
//! Ecore_Wl2 id (`ecore_wl2_window_id_get`), no autocapital and no prediction, the NORMAL layout, then
//! `focus_in` + `input_panel_show` to raise it and `input_panel_hide` + `focus_out` to dismiss it.
//!
//! **The viewer can close the panel themself** (Back, or Done on the panel), and then the application's field
//! would go on owning a keyboard that has gone, and `textinput::start` would refuse to raise it again. The
//! panel's STATE callback ([`state_cb`]) is what says so: a HIDE after a SHOW of a panel we asked for and did not
//! dismiss ourselves is pushed as [`Input::KeyboardDismissed`] ([`Panel`] is that decision, host-tested).
//!
//! The enum values are EFL's `Ecore_IMF.h` (`ECORE_IMF_CALLBACK_COMMIT` = 3, `ECORE_IMF_INPUT_PANEL_STATE_EVENT`
//! = 0, the states SHOW = 0, HIDE = 1, WILL_SHOW = 2, `ECORE_IMF_AUTOCAPITAL_TYPE_NONE` = 0,
//! `ECORE_IMF_INPUT_PANEL_LAYOUT_NORMAL` = 0). From the upstream header, not from a retail set.
use crate::hosted::Input;
use plx_base::eventlog::log;
use std::ffi::{c_char, c_int, c_void, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr::null;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicPtr, Ordering};
use std::sync::{Mutex, OnceLock};

/// `void (*Ecore_IMF_Event_Cb)(void *data, Ecore_IMF_Context *ctx, void *event_info)`.
type EventCb = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void);
/// `void (*)(void *data, Ecore_IMF_Context *ctx, int value)`, the input panel's event callback.
type PanelCb = unsafe extern "C" fn(*mut c_void, *mut c_void, c_int);

plx_base::dynlib! {
    /// The keyboard: all of it or none of it.
    imf: ["libecore_imf.so.1"] {
        fn ecore_imf_init() -> c_int;
        fn ecore_imf_shutdown() -> c_int;
        fn ecore_imf_context_default_id_get() -> *const c_char;
        fn ecore_imf_context_add(id: *const c_char) -> *mut c_void;
        fn ecore_imf_context_del(ctx: *mut c_void);
        fn ecore_imf_context_client_window_set(ctx: *mut c_void, window: *mut c_void);
        fn ecore_imf_context_focus_in(ctx: *mut c_void);
        fn ecore_imf_context_focus_out(ctx: *mut c_void);
        fn ecore_imf_context_reset(ctx: *mut c_void);
        fn ecore_imf_context_input_panel_show(ctx: *mut c_void);
        fn ecore_imf_context_input_panel_hide(ctx: *mut c_void);
        fn ecore_imf_context_autocapital_type_set(ctx: *mut c_void, kind: c_int);
        fn ecore_imf_context_prediction_allow_set(ctx: *mut c_void, allow: u8);
        fn ecore_imf_context_input_panel_layout_set(ctx: *mut c_void, layout: c_int);
        fn ecore_imf_context_event_callback_add(ctx: *mut c_void, kind: c_int, cb: EventCb, data: *const c_void);
        fn ecore_imf_context_input_panel_event_callback_add(ctx: *mut c_void, kind: c_int, cb: PanelCb, data: *const c_void);
    }
}

plx_base::dynlib! {
    /// The window's id, which the context takes as its client window.
    wl2_id: ["libecore_wl2.so.1"] {
        fn ecore_wl2_window_id_get(window: *mut c_void) -> c_int;
    }
}

const CALLBACK_COMMIT: c_int = 3;
const PANEL_STATE_EVENT: c_int = 0;
const AUTOCAPITAL_NONE: c_int = 0;
const LAYOUT_NORMAL: c_int = 0;

/// The input panel's state, as its STATE event reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PanelState {
    Show,
    Hide,
    WillShow,
    Other(c_int),
}

impl PanelState {
    fn of(value: c_int) -> PanelState {
        match value {
            0 => PanelState::Show,
            1 => PanelState::Hide,
            2 => PanelState::WillShow,
            other => PanelState::Other(other),
        }
    }
}

/// Whether a panel's HIDE is the viewer closing it: the decision [`state_cb`] makes, apart from Ecore.
///
/// `asked` is set by [`show`] and cleared by [`hide`] (and by the dismissal itself); `seen` is set by a SHOW
/// while asked. A HIDE is a dismissal only while both are set: a HIDE before the panel has shown (a panel still
/// rising) or after our own `hide` is not the viewer's.
///
/// **Done sends two things, in no promised order**: its `Select` key (OK, which commits the field and has the
/// application close the panel) and the panel's own HIDE. [`Panel::done`] makes the pair one decision whichever
/// arrives first: a `Select` while the panel is up is the commit, and ends the ask so the HIDE behind it is ours;
/// a `Select` after a HIDE already dismissed the field is dropped, since an OK on a field that is no longer
/// editing would raise the panel again.
#[derive(Debug, Default)]
pub(crate) struct Panel {
    asked: bool,
    seen: bool,
    /// A `Select` down was admitted and its up has not come yet.
    done_held: bool,
}

impl Panel {
    pub(crate) fn asked(&mut self) {
        (self.asked, self.seen) = (true, false);
    }
    pub(crate) fn dismissed_by_us(&mut self) {
        (self.asked, self.seen) = (false, false);
    }
    /// One edge of the panel's `Select` (its Done); `true` when the edge is delivered as the OK it is bound to.
    pub(crate) fn done(&mut self, down: bool) -> bool {
        if down {
            if self.asked && self.seen && !self.done_held {
                self.dismissed_by_us();
                self.done_held = true;
                return true;
            }
            false
        } else {
            std::mem::take(&mut self.done_held)
        }
    }
    /// One STATE event; `true` when it is the viewer's dismissal, which also ends the ask.
    pub(crate) fn state(&mut self, state: PanelState) -> bool {
        match state {
            PanelState::Show if self.asked => {
                self.seen = true;
                false
            }
            PanelState::Hide if self.asked && self.seen => {
                self.dismissed_by_us();
                true
            }
            _ => false,
        }
    }
}

/// The window's Ecore_Wl2 id, recorded on the host thread by [`bind_window`]; -1 before it.
static WINDOW_ID: AtomicI64 = AtomicI64::new(-1);
/// The context, made at the first [`show`] (host thread), deleted by [`release`].
static CONTEXT: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
/// `ecore_imf_init` ran and `ecore_imf_shutdown` has not.
static INITIALISED: AtomicBool = AtomicBool::new(false);
/// Host thread only (the callbacks and [`show`] / [`hide`]); never held across an Ecore call, since the panel
/// may report its state from inside `input_panel_show`.
static PANEL: Mutex<Panel> = Mutex::new(Panel { asked: false, seen: false, done_held: false });

fn loaded() -> bool {
    static LOADED: OnceLock<bool> = OnceLock::new();
    *LOADED.get_or_init(|| {
        crate::ecore::loaded("ecore_imf", imf::load(None)) && crate::ecore::loaded("ecore_wl2_window_id", wl2_id::load(None))
    })
}

/// Record the window the keyboard types into. HOST THREAD, after the window is made.
pub(crate) fn bind_window(window: *mut c_void) {
    if window.is_null() || !loaded() {
        return;
    }
    // SAFETY: a live window and the live table.
    let id = unsafe { ecore_wl2_window_id_get(window) };
    log(&format!("ime: window id {id}"));
    WINDOW_ID.store(i64::from(id), Ordering::SeqCst);
}

/// `Shell::keyboard_available`: the IMF library loads and the window was bound. Any thread.
pub(crate) fn available() -> bool {
    WINDOW_ID.load(Ordering::SeqCst) >= 0 && loaded()
}

fn panel() -> std::sync::MutexGuard<'static, Panel> {
    PANEL.lock().unwrap_or_else(|e| e.into_inner())
}

/// The key handler's question for the panel's `Select` ([`Panel::done`]). HOST THREAD.
pub(crate) fn admit_done(down: bool) -> bool {
    panel().done(down)
}

/// The context, made on first use. HOST THREAD.
fn context() -> Option<*mut c_void> {
    let ctx = CONTEXT.load(Ordering::SeqCst);
    if !ctx.is_null() {
        return Some(ctx);
    }
    let id = WINDOW_ID.load(Ordering::SeqCst);
    if id < 0 || !loaded() {
        log("ime: no window or no IMF library; the keyboard cannot rise");
        return None;
    }
    // SAFETY (block): the embedder's calls on the host thread, with the live table; the context is
    // null-checked before use and the id string is Ecore's own, read for the call.
    unsafe {
        // The count is raised once, and marked raised only when init says it was, so `release` never
        // shuts down a count it did not raise.
        if !INITIALISED.load(Ordering::SeqCst) {
            let count = ecore_imf_init();
            log(&format!("ime: ecore_imf_init -> {count}"));
            if count <= 0 {
                return None;
            }
            INITIALISED.store(true, Ordering::SeqCst);
        }
        let default_id = ecore_imf_context_default_id_get();
        if default_id.is_null() {
            log("ime: no default IMF context id");
            return None;
        }
        let ctx = ecore_imf_context_add(default_id);
        log(&format!("ime: context_add({}) -> {ctx:p}", CStr::from_ptr(default_id).to_string_lossy()));
        if ctx.is_null() {
            return None;
        }
        // The client window is the id cast to a pointer, as the embedder passes it.
        ecore_imf_context_client_window_set(ctx, id as usize as *mut c_void);
        ecore_imf_context_autocapital_type_set(ctx, AUTOCAPITAL_NONE);
        ecore_imf_context_prediction_allow_set(ctx, 0);
        ecore_imf_context_input_panel_layout_set(ctx, LAYOUT_NORMAL);
        ecore_imf_context_event_callback_add(ctx, CALLBACK_COMMIT, commit_cb, null());
        ecore_imf_context_input_panel_event_callback_add(ctx, PANEL_STATE_EVENT, state_cb, null());
        CONTEXT.store(ctx, Ordering::SeqCst);
        Some(ctx)
    }
}

/// Raise the panel. HOST THREAD (queued by the shell).
pub(crate) fn show() {
    let Some(ctx) = context() else { return };
    panel().asked();
    // SAFETY: a live context on the host thread.
    unsafe {
        ecore_imf_context_focus_in(ctx);
        ecore_imf_context_input_panel_show(ctx);
    }
    log("ime: panel show");
}

/// Dismiss the panel. HOST THREAD (queued by the shell).
pub(crate) fn hide() {
    panel().dismissed_by_us();
    let ctx = CONTEXT.load(Ordering::SeqCst);
    if ctx.is_null() {
        return;
    }
    // SAFETY: a live context on the host thread.
    unsafe {
        ecore_imf_context_reset(ctx);
        ecore_imf_context_input_panel_hide(ctx);
        ecore_imf_context_focus_out(ctx);
    }
    log("ime: panel hide");
}

/// The context and the library, at the window's end. HOST THREAD, before the window is freed.
///
/// The ask is forgotten and the panel hidden BEFORE the context is deleted, as the embedder does: a HIDE the
/// module reports while the context goes is then ours, never the viewer's, and nothing is pushed for it.
pub(crate) fn release() {
    panel().dismissed_by_us();
    let ctx = CONTEXT.swap(std::ptr::null_mut(), Ordering::SeqCst);
    // SAFETY: the context this module made, hidden and deleted once; the library initialised once.
    unsafe {
        if !ctx.is_null() {
            ecore_imf_context_input_panel_hide(ctx);
            ecore_imf_context_focus_out(ctx);
            ecore_imf_context_del(ctx);
        }
        if INITIALISED.swap(false, Ordering::SeqCst) {
            ecore_imf_shutdown();
        }
    }
    WINDOW_ID.store(-1, Ordering::SeqCst);
}

/// The longest commit read through the callback's pointer: a commit is a character or a word.
const COMMIT_LIMIT: usize = 4096;

/// The commit's text, bounded at [`COMMIT_LIMIT`] bytes.
///
/// # Safety
/// `p` is null or a NUL-terminated string, or points at [`COMMIT_LIMIT`] readable bytes.
unsafe fn commit_text(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut bytes = Vec::new();
    for i in 0..COMMIT_LIMIT {
        // SAFETY: the caller's contract, one byte at a time, stopping at the NUL.
        let b = unsafe { p.add(i).cast::<u8>().read() };
        if b == 0 {
            break;
        }
        bytes.push(b);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// `ECORE_IMF_CALLBACK_COMMIT`: `event_info` is the committed `char *`. Never logs the text (it can be a query
/// the viewer would not want in a log); the length is enough to see that a commit arrived.
unsafe extern "C" fn commit_cb(_data: *mut c_void, _ctx: *mut c_void, event_info: *mut c_void) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: Ecore's commit string for this call.
        let text = unsafe { commit_text(event_info.cast::<c_char>()) };
        log(&format!("ime: commit ({} bytes)", text.len()));
        if !text.is_empty() {
            let _ = crate::HOSTED.push(Input::Text(text));
        }
    }));
}

/// `ECORE_IMF_INPUT_PANEL_STATE_EVENT`: the panel rose or went. A HIDE that [`Panel`] reads as the viewer's own
/// is pushed as [`Input::KeyboardDismissed`], and the context leaves focus as [`hide`] would.
unsafe extern "C" fn state_cb(_data: *mut c_void, ctx: *mut c_void, value: c_int) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let state = PanelState::of(value);
        let dismissed = panel().state(state);
        log(&format!("ime: panel state {state:?}{}", if dismissed { " (closed on the panel)" } else { "" }));
        if dismissed {
            if !ctx.is_null() {
                // SAFETY: the live context the callback was registered on, on the host thread.
                unsafe { ecore_imf_context_focus_out(ctx) };
            }
            let _ = crate::HOSTED.push(Input::KeyboardDismissed);
        }
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A HIDE is the viewer's only after the panel we asked for has shown, and never after our own hide.**
    #[test]
    fn a_hide_is_a_dismissal_only_after_the_asked_panel_has_shown() {
        let mut p = Panel::default();
        assert!(!p.state(PanelState::Hide), "nothing was asked");
        p.asked();
        assert!(!p.state(PanelState::WillShow));
        assert!(!p.state(PanelState::Hide), "a hide before the show is the panel still settling, not the viewer");
        assert!(!p.state(PanelState::Show));
        assert!(p.state(PanelState::Hide), "shown, then hidden while asked: the viewer closed it");
        assert!(!p.state(PanelState::Hide), "one dismissal per ask");
        p.asked();
        assert!(!p.state(PanelState::Show));
        p.dismissed_by_us();
        assert!(!p.state(PanelState::Hide), "our own hide is not the viewer's");
        assert_eq!(PanelState::of(7), PanelState::Other(7));
    }

    /// **Done is one decision whichever of its two signals arrives first.** Key first: the `Select` is the OK
    /// (both edges), and the HIDE behind it is not the viewer's. HIDE first: the field was dismissed, and the
    /// `Select` behind it is dropped (both edges), so no OK lands on a field that stopped editing.
    #[test]
    fn done_is_one_commit_whichever_of_its_key_and_its_hide_arrives_first() {
        let mut key_first = Panel::default();
        key_first.asked();
        key_first.state(PanelState::Show);
        assert!(key_first.done(true), "Select down while the panel is up is the OK");
        assert!(!key_first.state(PanelState::Hide), "the HIDE behind it is ours, not a dismissal");
        assert!(key_first.done(false), "and its up goes with it");

        let mut hide_first = Panel::default();
        hide_first.asked();
        hide_first.state(PanelState::Show);
        assert!(hide_first.state(PanelState::Hide), "the HIDE dismisses the field");
        assert!(!hide_first.done(true), "a Select after it would re-open the panel: dropped");
        assert!(!hide_first.done(false));

        let mut idle = Panel::default();
        assert!(!idle.done(true) && !idle.done(false), "a Select with no panel up is not an OK");
    }

    /// The commit is read up to its NUL, and a null one is empty.
    #[test]
    fn a_commit_reads_to_its_nul() {
        let raw = b"wal\0ignored";
        // SAFETY: a NUL-terminated buffer.
        assert_eq!(unsafe { commit_text(raw.as_ptr().cast()) }, "wal");
        // SAFETY: null is allowed.
        assert_eq!(unsafe { commit_text(null()) }, "");
    }
}
