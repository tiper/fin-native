//! The hosted C ABI's types and constants: the Rust spelling of `include/plx_host.h`.
//!
//! The header is the contract's single source (the .NET host's `DllImport`s and its export check are
//! written from it); this file is its Rust twin, and `the_header_and_the_rust_types_agree` reads the
//! header's text and pins every name, value and field order against the types below. The three exports
//! (`plx_host_start`, `plx_host_post`, `plx_host_stop`) are in [`crate::exports`]; the logic behind them,
//! which a host test drives with a fake host table and no Ecore, is [`crate::hosted`].
use std::os::raw::c_void;

/// `PLX_HOST_ABI_VERSION`: the version of [`PlxHostApi`] this library implements.
pub const ABI_VERSION: u32 = 1;

/// A call that succeeded.
pub const OK: i32 = 0;
/// `PLX_HOST_E_ARGS`: a NULL or too small table or event, an `abi_version` of 0, an unknown `flags` bit or
/// event `kind`. No callback is made.
pub const E_ARGS: i32 = -1;
/// `PLX_HOST_E_STATE`: a second start, or a post when the library is not running.
pub const E_STATE: i32 = -2;
/// `PLX_HOST_E_LAUNCH`: `launch_json` was refused.
pub const E_LAUNCH: i32 = -3;
/// `PLX_HOST_E_WINDOW`: the Ecore window or the EGL libraries could not be brought up.
pub const E_WINDOW: i32 = -4;
/// `PLX_HOST_E_THREAD`: the loop thread could not be started.
pub const E_THREAD: i32 = -5;
/// `PLX_HOST_E_FULL`: the post queue was full and the event was dropped.
pub const E_FULL: i32 = -6;

/// `PLX_HOST_EVENT_PAUSE`: `OnPause`, the app went to the background.
pub const EVENT_PAUSE: u32 = 1;
/// `PLX_HOST_EVENT_RESUME`: `OnResume`, the app came back.
pub const EVENT_RESUME: u32 = 2;

/// What `on_stopped` is: the host's report sink, called exactly once with the process status and whether
/// the host may release its callbacks and surfaces (`clean` 1) or releases nothing (`clean` 0).
pub type OnStopped = unsafe extern "C" fn(code: i32, clean: i32);
/// A task the host runs once on its main thread.
pub type HostTask = unsafe extern "C" fn(arg: *mut c_void);
/// `run_on_host`: queue `task(arg)` for the host's main thread and return 0, or refuse it (non-zero).
pub type RunOnHost = unsafe extern "C" fn(task: HostTask, arg: *mut c_void) -> i32;

/// The table the host hands `plx_host_start`. `size` and `abi_version` come first so a later version can
/// grow the struct: a field is read only when `size` covers it, and a table smaller than version 1 is
/// refused. `Option` of a function pointer is the C nullable pointer, so a host that left a callback NULL
/// reads as `None` and is refused at the door instead of crashing at the call.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PlxHostApi {
    pub size: u32,
    pub abi_version: u32,
    pub flags: u32,
    pub reserved: u32,
    pub on_stopped: Option<OnStopped>,
    pub run_on_host: Option<RunOnHost>,
}

/// One event the host posts. Version 1 carries only a kind.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PlxHostEvent {
    pub size: u32,
    pub kind: u32,
}

/// What `size` must cover for a version-1 [`PlxHostApi`]: up to and including its last version-1 field.
pub const API_V1_SIZE: u32 = (std::mem::offset_of!(PlxHostApi, run_on_host) + std::mem::size_of::<Option<RunOnHost>>()) as u32;
/// What `size` must cover for a version-1 [`PlxHostEvent`].
pub const EVENT_V1_SIZE: u32 = (std::mem::offset_of!(PlxHostEvent, kind) + std::mem::size_of::<u32>()) as u32;

/// The host's callbacks, copied out of the table at the start: plain function pointers, valid until
/// `on_stopped` has been delivered clean.
#[derive(Clone, Copy)]
pub struct HostCallbacks {
    pub on_stopped: OnStopped,
    pub run_on_host: RunOnHost,
}

/// Read the table at `api` into [`HostCallbacks`], or say why it is refused ([`E_ARGS`] always: a refused
/// table has no `on_stopped` to tell).
///
/// # Safety
/// `api` is null or points at `size` readable bytes, where `size` is the table's first word, as the
/// contract's host guarantees; only the fields `size` covers are read.
pub unsafe fn read_table(api: *const PlxHostApi) -> Result<HostCallbacks, i32> {
    if api.is_null() {
        return Err(E_ARGS);
    }
    // SAFETY: the first word is `size`, readable by the contract; nothing else is read before it is checked.
    let size = unsafe { api.cast::<u32>().read_unaligned() };
    if size < API_V1_SIZE {
        return Err(E_ARGS);
    }
    // SAFETY: `size` covers every version-1 field.
    let table = unsafe { api.read_unaligned() };
    if table.abi_version < ABI_VERSION || table.flags != 0 {
        return Err(E_ARGS);
    }
    match (table.on_stopped, table.run_on_host) {
        (Some(on_stopped), Some(run_on_host)) => Ok(HostCallbacks { on_stopped, run_on_host }),
        _ => Err(E_ARGS),
    }
}

/// An event the host posted, as the queue holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostEvent {
    Pause,
    Resume,
}

/// Read the event at `event`, or [`E_ARGS`].
///
/// # Safety
/// `event` is null or points at `size` readable bytes, `size` being its first word.
pub unsafe fn read_event(event: *const PlxHostEvent) -> Result<HostEvent, i32> {
    if event.is_null() {
        return Err(E_ARGS);
    }
    // SAFETY: the first word is `size`, readable by the contract.
    let size = unsafe { event.cast::<u32>().read_unaligned() };
    if size < EVENT_V1_SIZE {
        return Err(E_ARGS);
    }
    // SAFETY: `size` covers every version-1 field.
    match unsafe { event.read_unaligned() }.kind {
        EVENT_PAUSE => Ok(HostEvent::Pause),
        EVENT_RESUME => Ok(HostEvent::Resume),
        _ => Err(E_ARGS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::offset_of;

    const HEADER: &str = include_str!("../include/plx_host.h");

    /// The text of the header with its comments removed, one logical line per statement.
    fn code() -> String {
        let mut out = String::new();
        let mut rest = HEADER;
        while let Some(at) = rest.find("/*") {
            out.push_str(&rest[..at]);
            rest = &rest[at + 2..];
            rest = &rest[rest.find("*/").expect("a comment ends") + 2..];
        }
        out.push_str(rest);
        out
    }

    /// `#define NAME value` of the header, the value with its parentheses dropped.
    fn define(name: &str) -> String {
        let text = code();
        let line = text
            .lines()
            .find_map(|l| l.trim().strip_prefix("#define ").and_then(|d| d.strip_prefix(name)).filter(|v| v.starts_with(' ')))
            .unwrap_or_else(|| panic!("the header defines {name}"));
        line.trim().trim_matches(|c| c == '(' || c == ')').to_string()
    }

    /// The field names of `typedef struct <name> { ... } <name>;`, in order.
    fn fields(name: &str) -> Vec<String> {
        let text = code();
        let head = format!("typedef struct {name} {{");
        let at = text.find(&head).unwrap_or_else(|| panic!("the header declares {name}"));
        let body = &text[at + head.len()..];
        let body = &body[..body.find('}').expect("the struct closes")];
        body.split(';')
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .map(|f| f.rsplit(|c: char| c.is_whitespace() || c == '*').next().expect("a field name").to_string())
            .collect()
    }

    /// **The header and the Rust types are one contract**: every constant has the value the header gives it,
    /// and both structs have the header's fields in the header's order (the Rust offsets ascend in that
    /// order and the last version-1 field ends the version-1 size). A renamed, re-valued or swapped field on
    /// either side fails here, before a host built from the header ever loads the library.
    #[test]
    fn the_header_and_the_rust_types_agree() {
        for (name, value) in [
            ("PLX_HOST_ABI_VERSION", i64::from(ABI_VERSION)),
            ("PLX_HOST_OK", i64::from(OK)),
            ("PLX_HOST_E_ARGS", i64::from(E_ARGS)),
            ("PLX_HOST_E_STATE", i64::from(E_STATE)),
            ("PLX_HOST_E_LAUNCH", i64::from(E_LAUNCH)),
            ("PLX_HOST_E_WINDOW", i64::from(E_WINDOW)),
            ("PLX_HOST_E_THREAD", i64::from(E_THREAD)),
            ("PLX_HOST_E_FULL", i64::from(E_FULL)),
            ("PLX_HOST_EVENT_PAUSE", i64::from(EVENT_PAUSE)),
            ("PLX_HOST_EVENT_RESUME", i64::from(EVENT_RESUME)),
        ] {
            assert_eq!(define(name).parse::<i64>().unwrap(), value, "{name}");
        }
        let api = [
            ("size", offset_of!(PlxHostApi, size)),
            ("abi_version", offset_of!(PlxHostApi, abi_version)),
            ("flags", offset_of!(PlxHostApi, flags)),
            ("reserved", offset_of!(PlxHostApi, reserved)),
            ("on_stopped", offset_of!(PlxHostApi, on_stopped)),
            ("run_on_host", offset_of!(PlxHostApi, run_on_host)),
        ];
        assert_eq!(fields("PlxHostApi"), api.map(|(n, _)| n), "PlxHostApi's fields, in order");
        assert!(api.windows(2).all(|w| w[0].1 < w[1].1), "the Rust offsets ascend in the header's order: {api:?}");
        let event = [("size", offset_of!(PlxHostEvent, size)), ("kind", offset_of!(PlxHostEvent, kind))];
        assert_eq!(fields("PlxHostEvent"), event.map(|(n, _)| n), "PlxHostEvent's fields, in order");
        assert!(event.windows(2).all(|w| w[0].1 < w[1].1));
        assert_eq!(API_V1_SIZE as usize, std::mem::size_of::<PlxHostApi>(), "version 1 ends at the last field");
        assert_eq!(EVENT_V1_SIZE as usize, std::mem::size_of::<PlxHostEvent>());
        let code = code();
        for decl in [
            "int32_t plx_host_start(const PlxHostApi *api, const char *launch_json);",
            "int32_t plx_host_post(const PlxHostEvent *event);",
            "void plx_host_stop(void);",
            "typedef void (*PlxHostTask)(void *arg);",
            "typedef void (*PlxOnStopped)(int32_t code, int32_t clean);",
            "typedef int32_t (*PlxRunOnHost)(PlxHostTask task, void *arg);",
        ] {
            assert!(code.contains(decl), "the header declares `{decl}`");
        }
    }

    unsafe extern "C" fn stopped(_: i32, _: i32) {}
    unsafe extern "C" fn run(_: HostTask, _: *mut c_void) -> i32 {
        0
    }

    fn table() -> PlxHostApi {
        PlxHostApi {
            size: std::mem::size_of::<PlxHostApi>() as u32,
            abi_version: ABI_VERSION,
            flags: 0,
            reserved: 0,
            on_stopped: Some(stopped),
            run_on_host: Some(run),
        }
    }

    /// A table smaller than version 1, version 0, with a flag bit set, or with a NULL callback is refused,
    /// and a bigger table (a later version) is read as far as version 1 covers.
    #[test]
    fn a_table_is_accepted_only_at_version_one_or_later() {
        let ok = table();
        assert!(unsafe { read_table(&ok) }.is_ok());
        assert_eq!(unsafe { read_table(std::ptr::null()) }.err(), Some(E_ARGS));
        let small = PlxHostApi { size: API_V1_SIZE - 1, ..ok };
        assert_eq!(unsafe { read_table(&small) }.err(), Some(E_ARGS), "smaller than version 1");
        let old = PlxHostApi { abi_version: 0, ..ok };
        assert_eq!(unsafe { read_table(&old) }.err(), Some(E_ARGS), "version 0");
        let flagged = PlxHostApi { flags: 1, ..ok };
        assert_eq!(unsafe { read_table(&flagged) }.err(), Some(E_ARGS), "a flag this library does not know");
        let no_stop = PlxHostApi { on_stopped: None, ..ok };
        assert_eq!(unsafe { read_table(&no_stop) }.err(), Some(E_ARGS), "no on_stopped");
        let no_run = PlxHostApi { run_on_host: None, ..ok };
        assert_eq!(unsafe { read_table(&no_run) }.err(), Some(E_ARGS), "no run_on_host");
        let later = PlxHostApi { size: API_V1_SIZE + 64, abi_version: 2, ..ok };
        assert!(unsafe { read_table(&later) }.is_ok(), "a later version's table is read as version 1");
    }

    #[test]
    fn an_event_is_decoded_by_its_kind_and_size() {
        let ev = |size, kind| PlxHostEvent { size, kind };
        assert_eq!(unsafe { read_event(&ev(EVENT_V1_SIZE, EVENT_PAUSE)) }, Ok(HostEvent::Pause));
        assert_eq!(unsafe { read_event(&ev(EVENT_V1_SIZE, EVENT_RESUME)) }, Ok(HostEvent::Resume));
        assert_eq!(unsafe { read_event(&ev(EVENT_V1_SIZE, 0)) }, Err(E_ARGS));
        assert_eq!(unsafe { read_event(&ev(EVENT_V1_SIZE, 99)) }, Err(E_ARGS));
        assert_eq!(unsafe { read_event(&ev(EVENT_V1_SIZE - 1, EVENT_PAUSE)) }, Err(E_ARGS));
        assert_eq!(unsafe { read_event(std::ptr::null()) }, Err(E_ARGS));
    }
}
