//! The webOS package links Sentry Native and its crash daemon. The capture transport is offline.

use plx_os::tv::crash_capture::{Breadcrumb, Hooks, Start};

pub(super) const HOOKS: Hooks = Hooks { start: Some(start), stop, set_user, record_window };

use std::ffi::{c_char, c_int, c_void, CString};
use std::sync::atomic::{AtomicBool, Ordering};

static ACTIVE: AtomicBool = AtomicBool::new(false);

extern "C" {
    fn sentry_options_new() -> *mut c_void;
    fn sentry_options_set_dsn(options: *mut c_void, value: *const c_char);
    fn sentry_options_set_database_path(options: *mut c_void, value: *const c_char);
    fn sentry_options_set_handler_path(options: *mut c_void, value: *const c_char);
    fn sentry_options_set_external_crash_reporter_path(
        options: *mut c_void,
        value: *const c_char,
    );
    fn sentry_options_set_release(options: *mut c_void, value: *const c_char);
    fn sentry_options_set_environment(options: *mut c_void, value: *const c_char);
    fn sentry_options_set_dist(options: *mut c_void, value: *const c_char);
    fn sentry_options_set_auto_session_tracking(options: *mut c_void, value: c_int);
    fn sentry_options_set_max_breadcrumbs(options: *mut c_void, value: usize);
    fn sentry_options_set_debug(options: *mut c_void, value: c_int);
    fn sentry_options_set_crash_reporting_mode(options: *mut c_void, value: c_int);
    fn sentry_init(options: *mut c_void) -> c_int;
    fn sentry_close() -> c_int;
    fn plx_sentry_set_webos_context(
        name: *const c_char,
        release: *const c_char,
        codename: *const c_char,
        api: *const c_char,
        model: *const c_char,
        soc: *const c_char,
        hardware_revision: *const c_char,
        rtkmem: *const c_char,
        install: *const c_char,
    );
    fn plx_sentry_set_user_id(id: *const c_char);
    fn plx_sentry_window_breadcrumb(stage: *const c_char, playing: c_int,
        major: c_int, minor: c_int, patch: c_int, display: c_int, surface: c_int);
}

/// Put the crash-report identifier on the SDK scope as `user.id`, or clear it. Each call makes
/// the native backend rewrite the daemon's base-event file, so the value is on disk before any
/// fault can happen. No-op while the backend is not running.
fn set_user(id: Option<&str>) {
    if !ACTIVE.load(Ordering::Acquire) {
        return;
    }
    let id = id.filter(|id| !id.is_empty()).and_then(cstring);
    unsafe {
        plx_sentry_set_user_id(id.as_ref().map_or(std::ptr::null(), |id| id.as_ptr()));
    }
}

fn record_window(observation: Breadcrumb) {
    if !ACTIVE.load(Ordering::Acquire) { return; }
    let Some(stage) = cstring(observation.stage) else { return; };
    let version = observation.version.map(|v| v.map(c_int::from)).unwrap_or([-1; 3]);
    let bit = |v: Option<bool>| v.map(c_int::from).unwrap_or(-1);
    unsafe {
        plx_sentry_window_breadcrumb(stage.as_ptr(), bit(observation.playing),
            version[0], version[1], version[2], bit(observation.display), bit(observation.surface));
    }
}

fn cstring(value: impl Into<Vec<u8>>) -> Option<CString> {
    CString::new(value).ok()
}

fn start(config: Start<'_>) {
    if ACTIVE.load(Ordering::Acquire) {
        return;
    }
    let Some(dsn) = cstring(config.dsn) else {
        return;
    };
    let Some(database) = cstring(config.database.as_os_str().as_encoded_bytes()) else {
        return;
    };
    let Some(handler) = cstring(
        plx_base::paths::app_dir()
            .join("sentry-crash")
            .as_os_str()
            .as_encoded_bytes(),
    ) else {
        return;
    };
    let Ok(executable_path) = std::env::current_exe() else {
        return;
    };
    let Some(executable) = cstring(executable_path.as_os_str().as_encoded_bytes()) else {
        return;
    };
    let Some(release) = cstring(config.release) else {
        return;
    };
    let Some(environment) = cstring(config.environment) else {
        return;
    };
    let Some(dist) = cstring(config.build_id) else {
        return;
    };
    let webos = plx_os::tv::device::info();
    let webos_name = cstring(webos.webos.name.as_bytes());
    let webos_release = cstring(webos.release.as_bytes());
    let webos_codename = cstring(webos.webos.codename.as_bytes());
    let webos_api = cstring(webos.webos.api.as_bytes());
    let hardware = plx_os::tv::device::device();
    let model = cstring(hardware.model.as_bytes());
    let soc = cstring(hardware.board.as_bytes());
    let hardware_revision = cstring(hardware.hw_revision.as_bytes());
    // issue #74: the same two closed-enum sandbox facts the PostHog envelope carries, so a
    // native crash report can be graded by chassis AND sandbox without a second dashboard.
    let rtkmem = cstring(plx_os::tv::native_video::context().as_bytes());
    let install = cstring(plx_base::paths::install_kind().as_bytes());
    let ptr = |value: &Option<CString>| {
        value
            .as_ref()
            .map_or(std::ptr::null(), |value| value.as_ptr())
    };

    unsafe {
        let options = sentry_options_new();
        if options.is_null() {
            return;
        }
        sentry_options_set_dsn(options, dsn.as_ptr());
        sentry_options_set_database_path(options, database.as_ptr());
        sentry_options_set_handler_path(options, handler.as_ptr());
        sentry_options_set_external_crash_reporter_path(options, executable.as_ptr());
        sentry_options_set_release(options, release.as_ptr());
        sentry_options_set_environment(options, environment.as_ptr());
        if !dist.as_bytes().is_empty() {
            sentry_options_set_dist(options, dist.as_ptr());
        }
        sentry_options_set_auto_session_tracking(options, 0);
        sentry_options_set_max_breadcrumbs(options, config.max_breadcrumbs);
        sentry_options_set_debug(options, 0);
        sentry_options_set_crash_reporting_mode(options, 1); // NATIVE, no minidump
        if sentry_init(options) == 0 {
            plx_sentry_set_webos_context(
                ptr(&webos_name),
                ptr(&webos_release),
                ptr(&webos_codename),
                ptr(&webos_api),
                ptr(&model),
                ptr(&soc),
                ptr(&hardware_revision),
                ptr(&rtkmem),
                ptr(&install),
            );
            ACTIVE.store(true, Ordering::Release);
            // After ACTIVE, because `set_user` refuses to touch a backend that is not running;
            // still inside `start`, so no caller can observe an active backend with no id.
            set_user(config.user);
            plx_base::eventlog::log("telemetry: native ARM crash capture active");
        } else {
            // `sentry_init` takes ownership even when backend startup fails.
            plx_base::eventlog::log(
                "telemetry: native crash capture unavailable; C fallback remains active",
            );
        }
    }
}

fn stop() {
    if ACTIVE.swap(false, Ordering::AcqRel) {
        unsafe {
            let _ = sentry_close();
        }
    }
}
