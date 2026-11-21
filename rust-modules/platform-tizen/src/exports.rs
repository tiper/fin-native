//! The three C symbols the .NET host `dlsym`s, and the order `plx_host_start` brings things up in.
//!
//! The contract is `include/plx_host.h`; every rule it states (who may call what, on which thread, what
//! `on_stopped` promises) is enforced by [`crate::hosted::Hosted`], which these functions only drive. What
//! belongs here is the platform's own order of work on the host thread, before the loop thread exists:
//!
//! 1. [`layout::store`], FIRST: the launch values the layout hooks and the identity read, so nothing that
//!    can read them runs earlier;
//! 2. the UDP log channel, if the launch string names one ([`log_udp::install`]); its failure is a log line
//!    (the file log still carries everything);
//! 3. the dev boot's preset ([`devboot::arm`]), a no-op in a build without dev triggers;
//! 4. the window ([`Window::create`], [`Window::show`]): the one step that can refuse the start.
//!
//! The loop thread then hands the window over ([`surface::hand_over`]) and runs `host::run` on
//! [`crate::PLATFORM`]. Nothing here unwinds into the host: every export catches a panic and answers as a
//! refusal would.
use crate::ecore::Window;
use crate::host_abi::{self, PlxHostApi, PlxHostEvent};
use crate::hosted::{guarded, Hosted, STOP_DEADLINE};
use crate::launch::Launch;
use crate::{devboot, layout, log_udp, surface, HOSTED, PLATFORM};
use plx_base::eventlog::log;
use plxnative_modules::host::{self, Exit};
use std::ffi::{c_char, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Steps 1-4 above, on the host thread. Refuses with [`host_abi::E_WINDOW`] when the window or a library it
/// needs (Ecore, EGL) cannot be brought up; a panic after the window exists releases it ([`guarded`]).
fn bring_up(launch: &Launch) -> Result<Window, i32> {
    layout::store(launch);
    if let Some(target) = launch.log_udp {
        match log_udp::install(target) {
            Ok(()) => log(&format!("log_udp: sending the event log to {target}")),
            Err(why) => log(&format!("log_udp: not installed: {why}")),
        }
    }
    devboot::arm(&launch.dev);
    guarded(Window::create()?, |window| window.show())
}

/// The loop thread's body: hand the window over, then run the application.
fn run(launch: Launch, window: Window) -> Exit {
    surface::hand_over(window);
    let Ok(pms_host) = CString::new(launch.pms_host.as_str()) else {
        log("host: the launch string's pms_host holds a NUL byte");
        return Exit::refused(1);
    };
    host::run(&PLATFORM, &pms_host, launch.pms_port)
}

/// `plx_host_start` over `hosted` (the process's own, in the export). A panic while bringing up is caught in
/// [`Hosted::start`] and answers `E_WINDOW` with `on_stopped(1, 1)` on this thread, as a refusal does; this
/// outer net is for a panic in `start`'s own code, and answers the same way.
pub(crate) fn start(hosted: &'static Hosted, api: *const PlxHostApi, launch_json: *const c_char) -> i32 {
    catch_unwind(AssertUnwindSafe(|| hosted.start(api, launch_json, bring_up, run)))
        .unwrap_or_else(|_| hosted.refuse(host_abi::E_WINDOW))
}

/// Load the library's run: validate the table and the launch string, bring the window up on THIS (the host's
/// main) thread, start the loop thread, return. See `include/plx_host.h`, START.
///
/// # Safety
/// `api` is NULL or a table readable as far as its own `size` says; `launch_json` is NULL or a NUL-terminated
/// string valid for the call.
#[no_mangle]
pub unsafe extern "C" fn plx_host_start(api: *const PlxHostApi, launch_json: *const c_char) -> i32 {
    start(&HOSTED, api, launch_json)
}

/// Queue a host event (pause, resume) for the loop. Any thread; never blocks. See `include/plx_host.h`, POST.
///
/// # Safety
/// `event` is NULL or readable as far as its own `size` says.
#[no_mangle]
pub unsafe extern "C" fn plx_host_post(event: *const PlxHostEvent) -> i32 {
    catch_unwind(AssertUnwindSafe(|| HOSTED.post(event))).unwrap_or(host_abi::E_STATE)
}

/// Ask the application to end and return at once; `on_stopped` carries the outcome, within the deadline.
/// See `include/plx_host.h`, STOP.
#[no_mangle]
pub extern "C" fn plx_host_stop() {
    let _ = catch_unwind(AssertUnwindSafe(|| HOSTED.stop(STOP_DEADLINE)));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The header's three symbols are the three this file exports, with the header's signatures.** The
    /// assignments fail to compile on a changed type; the text check fails on a missing or renamed export or
    /// a dropped `#[no_mangle]` (a symbol the host's `dlsym` cannot find is a start that never happens).
    #[test]
    fn the_exports_are_the_headers_symbols_with_its_signatures() {
        let _start: unsafe extern "C" fn(*const PlxHostApi, *const c_char) -> i32 = plx_host_start;
        let _post: unsafe extern "C" fn(*const PlxHostEvent) -> i32 = plx_host_post;
        let _stop: extern "C" fn() = plx_host_stop;
        let header = include_str!("../include/plx_host.h");
        let source = include_str!("exports.rs");
        for symbol in ["plx_host_start", "plx_host_post", "plx_host_stop"] {
            assert!(header.contains(&format!("{symbol}(")), "the header declares {symbol}");
            let lines: Vec<&str> = source.lines().collect();
            let at = lines
                .iter()
                .position(|l| l.starts_with("pub ") && l.contains(&format!("extern \"C\" fn {symbol}(")))
                .expect("exported");
            assert_eq!(lines[at - 1], "#[no_mangle]", "{symbol} is #[no_mangle]");
        }
    }

    /// A table the host got wrong is refused at the export, before anything of the process's is touched
    /// (the static `HOSTED` stays unstarted, so no other test sees a change).
    #[test]
    fn a_null_table_is_refused_at_the_export_and_a_post_outside_a_run_is_refused() {
        // SAFETY: NULL pointers are the arguments under test.
        unsafe {
            assert_eq!(plx_host_start(std::ptr::null(), std::ptr::null()), host_abi::E_ARGS);
            assert_eq!(plx_host_post(std::ptr::null()), host_abi::E_ARGS);
        }
    }
}
