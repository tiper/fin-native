//! The Tizen [`Shell`], [`Surface`] and [`FrameProbe`]: the window the application draws into and the
//! queue its input arrives through.
//!
//! **Who does what, on which thread.** The window is made on the host's main thread (`plx_host_start`,
//! [`crate::ecore::Window::create`]: Ecore is not thread-safe) and moved once, through [`hand_over`], to the
//! loop thread, which is where [`Shell::open`] runs. `open` takes the window from that hand-off cell and
//! makes the ES2 context current on the loop thread ([`crate::egl::Egl`], E6 option B), so the thread that
//! renders is the thread that owns the context. [`Surface::quit`] releases the context on the loop thread
//! first, and only then asks the host's main thread to take the window down (`run_on_host`, without
//! waiting): the loop never blocks on the host thread, and the host thread never touches a context it
//! does not own. After `plx_host_stop` it is the library's own gate that refuses that request, without
//! calling the host, and the window then lives to the end of the process; a host that refuses a request made
//! before the stop gets the other log line (`TizenShell::teardown`).
//!
//! **The window handle is null, and that is safe** ([`Surface::window`], D1). The SDL-shaped calls that read
//! it differ: `textinput`'s `has_focus` returns `None` for a null window before any SDL call (the log's
//! `focus=?`), and only the on-screen keyboard probe (`keyboard_probe(win)`) takes SDL's "no window" answer,
//! which holds because SDL was never initialised, as `Shell::init` here never does; the tests that pin it are
//! `plx_shell_sdl::text` and the application's `textinput`.
//!
//! Everything SDL-shaped that [`Shell`] asks for is a stated no-op: there is no SDL window to bind, no
//! surface to release, no opaque region to route (the window's mode is set once at its creation,
//! [`crate::ecore::WindowMode`], so the video plane shows through wherever the frame clears to alpha 0
//! and there is nothing to switch per frame; the player route clears that way whether or not a plane is
//! bound) and no platform bus to pump. The drawable size is not probed: the window is the screen's size, which
//! [`crate::ecore`] logs, and anything other than 1920x1080 is UNHANDLED (the renderer's canvas is that).
use crate::ecore::Window;
use crate::egl::Egl;
use crate::hosted::Hosted;
use plx_base::eventlog::log;
use plxnative_modules::host::{
    EventSink, Exit, FrameProbe, FrameSeam, GlProfile, InitHooks, InitSteps, MainThread, OpenAsk, Placement, SdlFlavor, Shell,
    Surface, WindowPlan,
};
use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::Mutex;

/// The window on its way from the host thread to the loop thread.
static HANDOFF: Mutex<Option<Window>> = Mutex::new(None);

/// Put the window where [`Shell::open`] will find it. Host or loop thread, before `host::run` is called.
pub fn hand_over(window: Window) {
    *HANDOFF.lock().unwrap_or_else(|e| e.into_inner()) = Some(window);
}

fn take() -> Option<Window> {
    HANDOFF.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// The Tizen shell: what the loop drives each frame, over the process-wide [`Hosted`].
pub struct TizenShell {
    pub hosted: &'static Hosted,
}

impl TizenShell {
    /// Ask the host's main thread to take `window` down. Dropped unrun (and the window left to the
    /// process's end) when the host has stopped the library or refuses the request.
    fn teardown(&self, mut window: Window) {
        if self.hosted.on_host(Box::new(move || window.destroy())) {
            return;
        }
        if self.hosted.stopping() {
            // Not the host being slow: `plx_host_stop` closed the library's gate the moment it was called, and the
            // contract is that `run_on_host` is never called after a stop (`plx_host.h`).
            log("surface: the host asked the library to stop, so the window teardown is not requested; it lives until the process ends");
        } else {
            log("surface: the host refused the window teardown; it lives until the process ends");
        }
    }
}

impl Shell for TizenShell {
    /// What Tizen states about its SDL, which it does not use: no SDL window, no SDL event and no step. The
    /// values are the ones a Tizen build compiled before the platform stated them (the television's
    /// layout, an ES 2.0 context and the nothing-to-do steps: the `not(sim)` arms every build without
    /// the simulator took), and the ES2 context is the one [`crate::egl::Egl`] makes.
    fn sdl(&self) -> InitHooks {
        InitHooks { steps: InitSteps::NONE, flavor: SdlFlavor::Lg, gl: GlProfile::Es2 }
    }

    /// What Tizen asks for, which [`open`](Shell::open) ignores (the window is the host's, and the screen's
    /// size): the title and the canvas-sized panel a Tizen build computed before the platform stated them (the
    /// webOS Interface resolution setting does not exist here, so it is the canvas, with no retry).
    fn window(&self, _stored: Option<(std::os::raw::c_int, std::os::raw::c_int)>) -> WindowPlan {
        WindowPlan {
            title: c"plxnative",
            placement: Placement::Panel {
                size: (plx_base::surface::LOGICAL_W as std::os::raw::c_int, plx_base::surface::LOGICAL_H as std::os::raw::c_int),
                fallback: None,
            },
        }
    }

    /// Windowing is up already (the host thread made the window): nothing to initialise, and no SDL.
    fn init(&self, _mt: &MainThread) -> Result<(), Exit> {
        Ok(())
    }

    fn open(&self, _mt: &MainThread, _ask: &OpenAsk) -> Result<Box<dyn Surface>, Exit> {
        let Some(window) = take() else {
            log("surface: no window was handed over");
            return Err(Exit::refused(1));
        };
        match Egl::create(window.wl_display, window.native_window) {
            Ok(egl) => Ok(Box::new(TizenSurface {
                egl: RefCell::new(egl),
                window: RefCell::new(Some(window)),
                shell: TizenShell { hosted: self.hosted },
            })),
            Err(why) => {
                log(&why);
                // The context never existed, so the window can go straight back to the host.
                self.teardown(window);
                Err(Exit::refused(1))
            }
        }
    }

    fn bind_window(&self, _win: *mut c_void) {}
    fn grab_surface(&self, _win: *mut c_void) {}
    fn release_surface(&self) -> bool {
        false
    }
    fn arm_opaque_region(&self) {}
    /// Nothing to route: the window's mode is fixed at creation ([`crate::ecore::WindowMode`]), and whether the
    /// video shows is the frame's own alpha (the player route's clear, which the loop makes on that route whether
    /// or not a plane is bound), so `_player` has no second effect here.
    fn opaque_route(&self, _player: bool) {}
    fn clear_opaque_region(&self) {}
    fn pump_bus(&self) {}
    fn hide_pointer(&self) {}
    fn show_pointer_after_motion(&self) {}
    fn hide_is_cursor_swap(&self) -> bool {
        false
    }
    /// The panel's EGL swap paces the frame and there is no simulator compositor to stand in for: nothing to state.
    fn frame(&self) -> &'static FrameSeam {
        &FrameSeam::NONE
    }
    /// Ecore IMF's keyboard ([`crate::ime`]), not SDL's: the library loads and the window was bound.
    fn keyboard_available(&self) -> bool {
        crate::ime::available()
    }
    /// Queued to the host's main thread, where every Ecore IMF call is made.
    fn show_keyboard(&self) {
        if !self.hosted.on_host(Box::new(crate::ime::show)) {
            log("surface: the keyboard show was not queued (the host is stopping or refused it)");
        }
    }
    fn hide_keyboard(&self) {
        if !self.hosted.on_host(Box::new(crate::ime::hide)) {
            log("surface: the keyboard hide was not queued (the host is stopping or refused it)");
        }
    }
}

/// The open window: the EGL context the loop presents through, and the native window it was made for.
pub struct TizenSurface {
    egl: RefCell<Egl>,
    window: RefCell<Option<Window>>,
    shell: TizenShell,
}

impl Surface for TizenSurface {
    /// Null: there is no SDL window (see the module note).
    fn window(&self) -> *mut c_void {
        null_mut()
    }

    fn swap(&self) {
        self.egl.borrow().swap();
    }

    fn set_swap_interval(&self, interval: i32) {
        self.egl.borrow().set_interval(interval);
    }

    fn deliver(&self, sink: &mut dyn EventSink) {
        self.shell.hosted.deliver(sink);
    }

    /// The context first, here; then the host's thread takes the window down.
    unsafe fn quit(&self) {
        self.egl.borrow_mut().destroy();
        if let Some(window) = self.window.borrow_mut().take() {
            self.shell.teardown(window);
        }
    }
}

/// The frame probe of a platform with no presentation-callback clock: unarmed, every call does nothing and
/// the log fields are empty.
pub struct NoProbe;

impl FrameProbe for NoProbe {
    fn request(&self) {}
    fn prepare_begin(&self) {}
    fn prepare_end(&self) {}
    fn waiting(&self) {}
    fn acquired(&self) {}
    fn fields(&self) -> String {
        String::new()
    }
    fn heartbeat_tag(&self) -> &'static str {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hosted::testing::*;
    use crate::launch::Launch;
    use plxnative_modules::host::{Action, Edge, Placement, Release};
    use std::sync::mpsc;
    use std::time::Duration;

    /// A loop that does nothing until the test lets it end: it never drains the queue, so the test owns it.
    fn idle_loop() -> (impl FnOnce(Launch, ()) -> Exit + Send + 'static, mpsc::Sender<()>) {
        let (release, wait) = mpsc::channel::<()>();
        (move |_, ()| {
            let _ = wait.recv_timeout(Duration::from_secs(10));
            complete()
        }, release)
    }

    /// End the run the test started: stop, let the loop return, and see the one clean report.
    fn finish(w: &World, release: mpsc::Sender<()>) {
        w.hosted.stop(Duration::from_secs(5));
        release.send(()).unwrap();
        assert_eq!(w.wait_stopped(1), [(0, 1)]);
    }

    fn ask() -> OpenAsk<'static> {
        OpenAsk { title: c"plx", placement: Placement::Panel { size: (1920, 1080), fallback: None }, high_dpi: false }
    }

    fn assert_refused(result: Result<Box<dyn Surface>, Exit>) {
        match result {
            Err(exit) => assert_eq!((exit.code, exit.release), (1, Release::Retained)),
            Ok(_) => panic!("the boot must be refused"),
        }
    }

    /// **No window, no boot**: a shell whose hand-off is empty refuses with the status every early-boot
    /// refusal has, and holds nothing.
    #[test]
    fn opening_without_a_handed_over_window_is_a_refused_boot() {
        let w = World::new();
        let _ = take();
        // SAFETY: this test thread plays the loop thread.
        let mt = unsafe { MainThread::assume() };
        let shell = TizenShell { hosted: w.hosted };
        assert_refused(shell.open(&mt, &ask()));
    }

    /// **A context that cannot be made hands the window back to the host**, once, and the boot is refused:
    /// no library here means the window the host made is not left for the process to leak while the host
    /// still runs. The queued teardown is the one `run_on_host` was asked for, and running it on the "host
    /// thread" destroys the (detached) window without calling anything.
    #[test]
    fn a_context_that_cannot_be_made_returns_the_window_to_the_host_thread() {
        let w = World::new();
        let (run, release) = idle_loop();
        assert_eq!(w.start(|_| Ok(()), run), crate::host_abi::OK);
        hand_over(Window::detached());
        // SAFETY: this test thread plays the loop thread.
        let mt = unsafe { MainThread::assume() };
        let shell = TizenShell { hosted: w.hosted };
        assert_refused(shell.open(&mt, &ask()));
        assert!(take().is_none(), "the window left the hand-off cell");
        let tasks = seen(|s| std::mem::take(&mut s.tasks));
        assert_eq!(tasks.len(), 1, "one teardown was asked of the host thread");
        // The host thread runs the task it was given, exactly once.
        // SAFETY: the task and argument `run_on_host` was handed, run once.
        unsafe { (tasks[0].0)(tasks[0].1 as *mut c_void) };
        finish(&w, release);
    }

    /// A surface over no live EGL: swap and interval are inert (they call nothing on a destroyed context),
    /// the window handle is null, `quit` hands the window to the host exactly once, and a second `quit`
    /// asks for nothing more.
    #[test]
    fn the_surface_has_a_null_window_and_quit_hands_the_window_back_once() {
        let w = World::new();
        let (run, release) = idle_loop();
        assert_eq!(w.start(|_| Ok(()), run), crate::host_abi::OK);
        let surface = TizenSurface {
            egl: RefCell::new(Egl::destroyed()),
            window: RefCell::new(Some(Window::detached())),
            shell: TizenShell { hosted: w.hosted },
        };
        assert!(surface.window().is_null());
        surface.swap();
        surface.set_swap_interval(1);
        // SAFETY: the loop thread's own, after the last use.
        unsafe { surface.quit() };
        unsafe { surface.quit() };
        assert_eq!(seen(|s| s.tasks.len()), 1, "one teardown, however many quits");
        finish(&w, release);
    }

    /// **After the host stopped the library the window teardown is not asked for, and the log says why.** The
    /// cause is the library's own gate, not a blocked host thread: `plx_host_stop` closes it at once, and
    /// `run_on_host` is never called after a stop (the header's contract), so the window lives until the
    /// process ends. The line names the stop; a refusal by the host is the other line.
    #[test]
    fn a_window_teardown_after_a_stop_is_not_asked_for_and_the_log_names_the_stop() {
        let w = World::new();
        w.hosted.stop(std::time::Duration::from_secs(1)); // closes the gate; no loop runs, so no deadline is armed
        let shell = TizenShell { hosted: w.hosted };
        let log = plx_base::eventlog::with_private_log(|| {
            shell.teardown(Window::detached());
            std::fs::read_to_string(plx_base::eventlog::events_log()).unwrap_or_default()
        });
        assert_eq!(seen(|s| s.tasks.len()), 0, "the host was asked for nothing");
        assert!(
            log.contains("surface: the host asked the library to stop, so the window teardown is not requested"),
            "{log}"
        );
        assert!(!log.contains("refused"), "{log}");
    }

    /// The shell's hooks are the stated no-ops: nothing is released, nothing is a cursor swap. (The three the
    /// frame gate pins to `app/run.rs`, `pump_bus` and the two opaque-region calls, are empty bodies and are not
    /// called from here.)
    #[test]
    fn the_sdl_shaped_hooks_are_stated_no_ops() {
        let w = World::new();
        let shell = TizenShell { hosted: w.hosted };
        // SAFETY: this test thread plays the loop thread.
        let mt = unsafe { MainThread::assume() };
        assert!(shell.init(&mt).is_ok());
        shell.bind_window(null_mut());
        shell.grab_surface(null_mut());
        assert!(!shell.release_surface());
        shell.arm_opaque_region();
        shell.hide_pointer();
        shell.show_pointer_after_motion();
        assert!(!shell.hide_is_cursor_swap());
        assert_eq!(NoProbe.fields(), "");
    }

    /// `deliver` is the hosted queue's: input pushed on another thread arrives on the loop's call, in order.
    #[test]
    fn deliver_hands_over_the_queued_input_in_order() {
        let w = World::new();
        let (run, release) = idle_loop();
        assert_eq!(w.start(|_| Ok(()), run), crate::host_abi::OK);
        let surface = TizenSurface {
            egl: RefCell::new(Egl::destroyed()),
            window: RefCell::new(None),
            shell: TizenShell { hosted: w.hosted },
        };
        w.hosted.push(crate::hosted::Input::Key(Action::Ok, Edge::Down)).unwrap();
        w.hosted.push(crate::hosted::Input::Key(Action::Ok, Edge::Up)).unwrap();
        let mut rec = Rec::default();
        surface.deliver(&mut rec);
        assert_eq!(rec.seen, ["key Some(Ok) None Down", "key Some(Ok) None Up"]);
        finish(&w, release);
    }
}
