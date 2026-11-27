//! The hosted C ABI's types and constants: the Rust spelling of `include/plx_host.h`.
//!
//! The header is the contract's single source (the .NET host's `DllImport`s and its export check are
//! written from it); this file is its Rust twin, and `the_header_and_the_rust_types_agree` reads the
//! header's text and pins every name, value and field order against the types below. The four exports
//! (`plx_host_start`, `plx_host_post`, `plx_host_post_cue`, `plx_host_stop`) are in [`crate::exports`]; the logic behind them,
//! which a host test drives with a fake host table and no Ecore, is [`crate::hosted`].
use std::os::raw::c_void;

/// `PLX_HOST_ABI_VERSION`: the version of [`PlxHostApi`] this library implements.
pub const ABI_VERSION: u32 = 2;

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
/// `PLX_HOST_E_STALE`: a player event of an epoch that is not live (never opened, or already closed).
pub const E_STALE: i32 = -7;

/// `PLX_HOST_PLAYER_OK`: a player request that was queued for the host's video thread.
pub const PLAYER_OK: i32 = 0;
/// `PLX_HOST_PLAYER_E_UNAVAILABLE`: the host has no media player, or its video thread is not running.
pub const PLAYER_E_UNAVAILABLE: i32 = -1;
/// `PLX_HOST_PLAYER_E_BUSY`: the host's request queue is full.
pub const PLAYER_E_BUSY: i32 = -2;
/// `PLX_HOST_PLAYER_E_ARGS`: epoch 0, a NULL or empty uri, an unknown verb.
pub const PLAYER_E_ARGS: i32 = -3;
/// `PLX_HOST_PLAYER_E_STALE`: a verb for an epoch that is not the host's current one.
pub const PLAYER_E_STALE: i32 = -4;
/// Not an ABI code (the host never returns it and `plx_host.h` has no name for it): the LIBRARY's own answer when its
/// gate is closed because `plx_host_stop` was called. The host was not asked. `plx_host.h`, STOP: from then on the host
/// closes its own player (`OnTerminate`), so a CLOSE refused this way is a release the host owns, not one nobody can make.
pub const PLAYER_E_STOPPING: i32 = -100;

/// `PLX_HOST_EVENT_PAUSE`: `OnPause`, the app went to the background.
pub const EVENT_PAUSE: u32 = 1;
/// `PLX_HOST_EVENT_RESUME`: `OnResume`, the app came back.
pub const EVENT_RESUME: u32 = 2;
/// `PLX_HOST_EVENT_PLAYER_*`: the player events (version 2), each with its epoch and payload words.
pub const EVENT_PLAYER_PREPARED: u32 = 16;
pub const EVENT_PLAYER_STARTED: u32 = 17;
pub const EVENT_PLAYER_PAUSED: u32 = 18;
pub const EVENT_PLAYER_POSITION: u32 = 19;
pub const EVENT_PLAYER_SEEK_DONE: u32 = 20;
pub const EVENT_PLAYER_BUFFERING: u32 = 21;
pub const EVENT_PLAYER_VIDEO_SIZE: u32 = 22;
pub const EVENT_PLAYER_COMPLETED: u32 = 23;
pub const EVENT_PLAYER_ERROR: u32 = 24;
pub const EVENT_PLAYER_INTERRUPTED: u32 = 25;
pub const EVENT_PLAYER_CLOSED: u32 = 26;
pub const EVENT_PLAYER_STALL: u32 = 27;
/// `PLX_HOST_EVENT_PLAYER_TRACK`: the answer to a `VERB_AUDIO_TRACK` (a = command id, b = the native index selected, or -1).
pub const EVENT_PLAYER_TRACK: u32 = 28;
/// `PLX_HOST_EVENT_PLAYER_AUDIO_LANGUAGE`: one native audio track's language, posted before PREPARED (a = the native index,
/// b = the code's ASCII bytes, first character in the lowest byte, 0 when the player has none; [`language_of_word`]).
pub const EVENT_PLAYER_AUDIO_LANGUAGE: u32 = 29;

/// `PLX_HOST_AUDIO_LANGUAGE_REPORTS`: the most native audio tracks whose `PLAYER_AUDIO_LANGUAGE` the host posts at one
/// PREPARED; a report past it is dropped by the driver, so that track's language reads as unknown.
pub const AUDIO_LANGUAGE_REPORTS: usize = 32;
/// `PLX_HOST_EVENT_PLAYER_TEXT_LANGUAGE`: one native TEXT track's language, posted before PREPARED (a = the native text index,
/// b = as [`EVENT_PLAYER_AUDIO_LANGUAGE`]).
pub const EVENT_PLAYER_TEXT_LANGUAGE: u32 = 30;
/// `PLX_HOST_TEXT_LANGUAGE_REPORTS`: the most native text tracks whose `PLAYER_TEXT_LANGUAGE` the host posts at one PREPARED.
pub const TEXT_LANGUAGE_REPORTS: usize = 32;
/// `PLX_HOST_CUE_TEXT_MAX`: the longest cue text, in bytes, `plx_host_post_cue` accepts.
pub const CUE_TEXT_MAX: u32 = 4096;

/// The language code a `PLAYER_AUDIO_LANGUAGE` word carries: two or three ASCII letters, first character in the lowest
/// byte and zero-padded. Anything else (no code, a tag with a region, a byte that is not a letter) is `None`, which the
/// driver treats as a track whose language is unknown.
pub fn language_of_word(word: i64) -> Option<String> {
    let bytes = word.to_le_bytes();
    let len = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    let (code, pad) = bytes.split_at(len);
    (code.len() >= 2 && code.len() <= 3 && code.iter().all(u8::is_ascii_alphabetic) && pad.iter().all(|b| *b == 0))
        .then(|| code.iter().map(|b| char::from(*b)).collect())
}

/// `PLX_HOST_PLAYER_FAIL_BASE`: a `PLAYER_ERROR` payload at or above this is a failure of the host, not a native code.
#[cfg_attr(not(any(test, feature = "uri")), allow(dead_code))] // the Tizen URI driver (the `uri` feature) reads it
pub const PLAYER_FAIL_BASE: i64 = 2_147_418_112;

/// `PLX_HOST_PLAYER_VERB_*`: `player_verb`'s verbs. The Tizen URI driver (the `uri` feature) is their caller.
#[cfg_attr(not(any(test, feature = "uri")), allow(dead_code))]
pub const VERB_PLAY: u32 = 1;
#[cfg_attr(not(any(test, feature = "uri")), allow(dead_code))]
pub const VERB_PAUSE: u32 = 2;
#[cfg_attr(not(any(test, feature = "uri")), allow(dead_code))]
pub const VERB_SEEK: u32 = 3;
#[cfg_attr(not(any(test, feature = "uri")), allow(dead_code))]
pub const VERB_CLOSE: u32 = 4;
/// `PLX_HOST_PLAYER_VERB_AUDIO_TRACK`: select the audio track at native index arg0; arg1 is the command id, echoed in
/// `PLAYER_TRACK`.
#[cfg_attr(not(any(test, feature = "uri")), allow(dead_code))]
pub const VERB_AUDIO_TRACK: u32 = 5;
/// `PLX_HOST_PLAYER_VERB_TEXT_TRACK`: select the TEXT track at native index arg0 and turn the epoch's cues on; arg1 is the
/// command id, echoed in `PLAYER_TRACK`.
#[cfg_attr(not(any(test, feature = "uri")), allow(dead_code))]
pub const VERB_TEXT_TRACK: u32 = 6;

/// What `on_stopped` is: the host's report sink, called exactly once with the process status and whether
/// the host may release its callbacks and surfaces (`clean` 1) or releases nothing (`clean` 0).
pub type OnStopped = unsafe extern "C" fn(code: i32, clean: i32);
/// A task the host runs once on its main thread.
pub type HostTask = unsafe extern "C" fn(arg: *mut c_void);
/// `run_on_host`: queue `task(arg)` for the host's main thread and return 0, or refuse it (non-zero).
pub type RunOnHost = unsafe extern "C" fn(task: HostTask, arg: *mut c_void) -> i32;
/// `player_open`: queue "create a player for `epoch`, give it `uri`, bind it to the library's Ecore window (which the
/// host finds itself) and prepare it"
/// for the host's video thread, and return at once ([`PLAYER_OK`] queued, negative refused). `uri` is a
/// NUL-terminated string the host copies before returning.
pub type PlayerOpen =
    unsafe extern "C" fn(epoch: u32, uri: *const std::os::raw::c_char, x: i32, y: i32, w: i32, h: i32) -> i32;
/// `player_verb`: queue one of the `VERB_*` verbs for the player of `epoch`, and return at once.
pub type PlayerVerb = unsafe extern "C" fn(epoch: u32, verb: u32, arg0: i64, arg1: i64) -> i32;

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
    /// Version 2: the media player's two requests. Read only when `size` covers them and `abi_version` is 2
    /// or more; both or neither.
    pub player_open: Option<PlayerOpen>,
    pub player_verb: Option<PlayerVerb>,
}

/// One event the host posts. Version 1 carries only a kind; version 2 appends what a player event needs
/// (`reserved` keeps the two `i64` words 8-aligned on every architecture, so no side relies on padding).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PlxHostEvent {
    pub size: u32,
    pub kind: u32,
    pub epoch: u32,
    pub reserved: u32,
    pub a: i64,
    pub b: i64,
}

/// `PlxHostCue`: one subtitle cue the host hands over with `plx_host_post_cue` (`plx_host.h`, PLAYER, CUE).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PlxHostCue {
    pub size: u32,
    pub epoch: u32,
    pub position_ms: i64,
    pub duration_ms: i64,
    pub text: *const std::os::raw::c_char,
    pub len: u32,
    pub reserved: u32,
}

/// One cue as the library queues it: copied out of the host's [`PlxHostCue`]. An empty `text` is the player's report that
/// the cue on screen ended (it raises one after a seek as well).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostCue {
    pub epoch: u32,
    pub position_ms: i64,
    pub duration_ms: i64,
    pub text: String,
}

/// Read the cue at `cue`, or [`E_ARGS`]: NULL, a `size` below the struct, text NULL with a length, or a length above
/// [`CUE_TEXT_MAX`]. Bytes that are not UTF-8 are replaced, never refused: a cue is shown or lost, it fails nothing.
///
/// # Safety
/// `cue` is null or points at `size` readable bytes, `size` being its first word, and `text` (when `len` > 0) at `len`
/// readable bytes.
pub unsafe fn read_cue(cue: *const PlxHostCue) -> Result<HostCue, i32> {
    if cue.is_null() {
        return Err(E_ARGS);
    }
    // SAFETY: the first word is `size`, readable by the contract.
    let size = unsafe { cue.cast::<u32>().read_unaligned() };
    if (size as usize) < std::mem::size_of::<PlxHostCue>() {
        return Err(E_ARGS);
    }
    // SAFETY: `size` covers the whole struct.
    let raw = unsafe { cue.read_unaligned() };
    if raw.len > CUE_TEXT_MAX || (raw.len > 0 && raw.text.is_null()) {
        return Err(E_ARGS);
    }
    let text = if raw.len == 0 {
        String::new()
    } else {
        // SAFETY: `text` points at `len` readable bytes by the contract, checked non-NULL above.
        let bytes = unsafe { std::slice::from_raw_parts(raw.text.cast::<u8>(), raw.len as usize) };
        String::from_utf8_lossy(bytes).into_owned()
    };
    Ok(HostCue { epoch: raw.epoch, position_ms: raw.position_ms, duration_ms: raw.duration_ms, text })
}

/// What `size` must cover for a version-1 [`PlxHostApi`]: up to and including its last version-1 field.
pub const API_V1_SIZE: u32 = (std::mem::offset_of!(PlxHostApi, run_on_host) + std::mem::size_of::<Option<RunOnHost>>()) as u32;
/// What `size` must cover for a version-1 [`PlxHostEvent`].
pub const EVENT_V1_SIZE: u32 = (std::mem::offset_of!(PlxHostEvent, kind) + std::mem::size_of::<u32>()) as u32;
/// What `size` must cover for a version-2 [`PlxHostApi`]: through `player_verb`.
pub const API_V2_SIZE: u32 = (std::mem::offset_of!(PlxHostApi, player_verb) + std::mem::size_of::<Option<PlayerVerb>>()) as u32;
/// What `size` must cover for a player event: through `b`.
pub const EVENT_V2_SIZE: u32 = (std::mem::offset_of!(PlxHostEvent, b) + std::mem::size_of::<i64>()) as u32;

/// The host's media player requests (the version-2 group).
#[derive(Clone, Copy)]
pub struct PlayerCalls {
    pub open: PlayerOpen,
    pub verb: PlayerVerb,
}

/// Why the table carries no media player. Not an error: the library starts and plays nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoPlayer {
    /// A version-1 table, or a table too small to hold the player group.
    TableV1,
    /// A version-2 table with a NULL `player_open` or `player_verb`.
    NullPointer,
    /// The library has not been started, so there is no table to ask.
    NotStarted,
}

impl NoPlayer {
    /// The reason, for the one boot log line.
    pub fn reason(self) -> &'static str {
        match self {
            NoPlayer::NotStarted => "the library has not been started",
            NoPlayer::TableV1 => "the host's table has no player group (version 1)",
            NoPlayer::NullPointer => "the host's table leaves a player pointer NULL",
        }
    }
}

/// The host's callbacks, copied out of the table at the start: plain function pointers, valid until
/// `on_stopped` has been delivered clean.
#[derive(Clone, Copy)]
pub struct HostCallbacks {
    pub on_stopped: OnStopped,
    pub run_on_host: RunOnHost,
    /// The media player, or why there is none.
    pub player: Result<PlayerCalls, NoPlayer>,
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
    // SAFETY: `size` covers every version-1 field, and only those are read (the table is read as a v1 prefix).
    let table = unsafe { read_v1_prefix(api) };
    if table.abi_version < 1 || table.flags != 0 {
        return Err(E_ARGS);
    }
    let (Some(on_stopped), Some(run_on_host)) = (table.on_stopped, table.run_on_host) else {
        return Err(E_ARGS);
    };
    Ok(HostCallbacks { on_stopped, run_on_host, player: read_player(api, size, table.abi_version) })
}

/// The version-1 fields of the table at `api` (the later ones zero: they are not read from a table that
/// does not cover them).
///
/// # Safety
/// `api` points at `API_V1_SIZE` readable bytes.
unsafe fn read_v1_prefix(api: *const PlxHostApi) -> PlxHostApi {
    let mut table = PlxHostApi {
        size: 0,
        abi_version: 0,
        flags: 0,
        reserved: 0,
        on_stopped: None,
        run_on_host: None,
        player_open: None,
        player_verb: None,
    };
    // SAFETY: the caller guarantees `API_V1_SIZE` readable bytes, and `table` is at least that large.
    unsafe { std::ptr::copy_nonoverlapping(api.cast::<u8>(), (&mut table as *mut PlxHostApi).cast::<u8>(), API_V1_SIZE as usize) };
    table
}

/// The player group of the table at `api`: present only when `size` covers it, `abi_version` names it and
/// both pointers are set. A host that does not offer one is not refused (skew is "no playback").
///
/// # Safety
/// `api` points at `size` readable bytes.
unsafe fn read_player(api: *const PlxHostApi, size: u32, abi_version: u32) -> Result<PlayerCalls, NoPlayer> {
    if size < API_V2_SIZE || abi_version < 2 {
        return Err(NoPlayer::TableV1);
    }
    // SAFETY: `size` covers every version-2 field.
    let table = unsafe { api.read_unaligned() };
    match (table.player_open, table.player_verb) {
        (Some(open), Some(verb)) => Ok(PlayerCalls { open, verb }),
        _ => Err(NoPlayer::NullPointer),
    }
}

/// What a player event reports. The payload words of the kinds that carry none are 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerKind {
    Prepared,
    Started,
    Paused,
    Position,
    SeekDone,
    Buffering,
    VideoSize,
    Completed,
    Error,
    Interrupted,
    Closed,
    Stall,
    Track,
    AudioLanguage,
    TextLanguage,
}

impl PlayerKind {
    fn of(kind: u32) -> Option<PlayerKind> {
        Some(match kind {
            EVENT_PLAYER_PREPARED => PlayerKind::Prepared,
            EVENT_PLAYER_STARTED => PlayerKind::Started,
            EVENT_PLAYER_PAUSED => PlayerKind::Paused,
            EVENT_PLAYER_POSITION => PlayerKind::Position,
            EVENT_PLAYER_SEEK_DONE => PlayerKind::SeekDone,
            EVENT_PLAYER_BUFFERING => PlayerKind::Buffering,
            EVENT_PLAYER_VIDEO_SIZE => PlayerKind::VideoSize,
            EVENT_PLAYER_COMPLETED => PlayerKind::Completed,
            EVENT_PLAYER_ERROR => PlayerKind::Error,
            EVENT_PLAYER_INTERRUPTED => PlayerKind::Interrupted,
            EVENT_PLAYER_CLOSED => PlayerKind::Closed,
            EVENT_PLAYER_STALL => PlayerKind::Stall,
            EVENT_PLAYER_TRACK => PlayerKind::Track,
            EVENT_PLAYER_AUDIO_LANGUAGE => PlayerKind::AudioLanguage,
            EVENT_PLAYER_TEXT_LANGUAGE => PlayerKind::TextLanguage,
            _ => return None,
        })
    }

    /// Only the latest of these per epoch matters: a newer one replaces an older one that is still queued.
    pub fn coalesces(self) -> bool {
        matches!(self, PlayerKind::Position | PlayerKind::Buffering)
    }
}

/// One player event as the library queues it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerEvent {
    pub epoch: u32,
    pub kind: PlayerKind,
    pub a: i64,
    pub b: i64,
}

/// An event the host posted, as the queue holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostEvent {
    Pause,
    Resume,
    Player(PlayerEvent),
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
    let kind = unsafe { event.cast::<u32>().add(1).read_unaligned() };
    match kind {
        EVENT_PAUSE => Ok(HostEvent::Pause),
        EVENT_RESUME => Ok(HostEvent::Resume),
        _ => {
            let kind = PlayerKind::of(kind).ok_or(E_ARGS)?;
            if size < EVENT_V2_SIZE {
                return Err(E_ARGS);
            }
            // SAFETY: `size` covers every version-2 field, which is the whole struct.
            let ev = unsafe { event.read_unaligned() };
            Ok(HostEvent::Player(PlayerEvent { epoch: ev.epoch, kind, a: ev.a, b: ev.b }))
        }
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
            ("PLX_HOST_E_STALE", i64::from(E_STALE)),
            ("PLX_HOST_PLAYER_OK", i64::from(PLAYER_OK)),
            ("PLX_HOST_PLAYER_E_UNAVAILABLE", i64::from(PLAYER_E_UNAVAILABLE)),
            ("PLX_HOST_PLAYER_E_BUSY", i64::from(PLAYER_E_BUSY)),
            ("PLX_HOST_PLAYER_E_ARGS", i64::from(PLAYER_E_ARGS)),
            ("PLX_HOST_PLAYER_E_STALE", i64::from(PLAYER_E_STALE)),
            ("PLX_HOST_EVENT_PAUSE", i64::from(EVENT_PAUSE)),
            ("PLX_HOST_EVENT_RESUME", i64::from(EVENT_RESUME)),
            ("PLX_HOST_EVENT_PLAYER_PREPARED", i64::from(EVENT_PLAYER_PREPARED)),
            ("PLX_HOST_EVENT_PLAYER_STARTED", i64::from(EVENT_PLAYER_STARTED)),
            ("PLX_HOST_EVENT_PLAYER_PAUSED", i64::from(EVENT_PLAYER_PAUSED)),
            ("PLX_HOST_EVENT_PLAYER_POSITION", i64::from(EVENT_PLAYER_POSITION)),
            ("PLX_HOST_EVENT_PLAYER_SEEK_DONE", i64::from(EVENT_PLAYER_SEEK_DONE)),
            ("PLX_HOST_EVENT_PLAYER_BUFFERING", i64::from(EVENT_PLAYER_BUFFERING)),
            ("PLX_HOST_EVENT_PLAYER_VIDEO_SIZE", i64::from(EVENT_PLAYER_VIDEO_SIZE)),
            ("PLX_HOST_EVENT_PLAYER_COMPLETED", i64::from(EVENT_PLAYER_COMPLETED)),
            ("PLX_HOST_EVENT_PLAYER_ERROR", i64::from(EVENT_PLAYER_ERROR)),
            ("PLX_HOST_EVENT_PLAYER_INTERRUPTED", i64::from(EVENT_PLAYER_INTERRUPTED)),
            ("PLX_HOST_EVENT_PLAYER_CLOSED", i64::from(EVENT_PLAYER_CLOSED)),
            ("PLX_HOST_EVENT_PLAYER_STALL", i64::from(EVENT_PLAYER_STALL)),
            ("PLX_HOST_EVENT_PLAYER_TRACK", i64::from(EVENT_PLAYER_TRACK)),
            ("PLX_HOST_EVENT_PLAYER_AUDIO_LANGUAGE", i64::from(EVENT_PLAYER_AUDIO_LANGUAGE)),
            ("PLX_HOST_AUDIO_LANGUAGE_REPORTS", AUDIO_LANGUAGE_REPORTS as i64),
            ("PLX_HOST_EVENT_PLAYER_TEXT_LANGUAGE", i64::from(EVENT_PLAYER_TEXT_LANGUAGE)),
            ("PLX_HOST_TEXT_LANGUAGE_REPORTS", TEXT_LANGUAGE_REPORTS as i64),
            ("PLX_HOST_CUE_TEXT_MAX", i64::from(CUE_TEXT_MAX)),
            ("PLX_HOST_PLAYER_FAIL_BASE", PLAYER_FAIL_BASE),
            ("PLX_HOST_PLAYER_VERB_PLAY", i64::from(VERB_PLAY)),
            ("PLX_HOST_PLAYER_VERB_PAUSE", i64::from(VERB_PAUSE)),
            ("PLX_HOST_PLAYER_VERB_SEEK", i64::from(VERB_SEEK)),
            ("PLX_HOST_PLAYER_VERB_CLOSE", i64::from(VERB_CLOSE)),
            ("PLX_HOST_PLAYER_VERB_AUDIO_TRACK", i64::from(VERB_AUDIO_TRACK)),
            ("PLX_HOST_PLAYER_VERB_TEXT_TRACK", i64::from(VERB_TEXT_TRACK)),
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
            ("player_open", offset_of!(PlxHostApi, player_open)),
            ("player_verb", offset_of!(PlxHostApi, player_verb)),
        ];
        assert_eq!(fields("PlxHostApi"), api.map(|(n, _)| n), "PlxHostApi's fields, in order");
        assert!(api.windows(2).all(|w| w[0].1 < w[1].1), "the Rust offsets ascend in the header's order: {api:?}");
        let event = [
            ("size", offset_of!(PlxHostEvent, size)),
            ("kind", offset_of!(PlxHostEvent, kind)),
            ("epoch", offset_of!(PlxHostEvent, epoch)),
            ("reserved", offset_of!(PlxHostEvent, reserved)),
            ("a", offset_of!(PlxHostEvent, a)),
            ("b", offset_of!(PlxHostEvent, b)),
        ];
        assert_eq!(fields("PlxHostEvent"), event.map(|(n, _)| n), "PlxHostEvent's fields, in order");
        let cue = [
            ("size", offset_of!(PlxHostCue, size)),
            ("epoch", offset_of!(PlxHostCue, epoch)),
            ("position_ms", offset_of!(PlxHostCue, position_ms)),
            ("duration_ms", offset_of!(PlxHostCue, duration_ms)),
            ("text", offset_of!(PlxHostCue, text)),
            ("len", offset_of!(PlxHostCue, len)),
            ("reserved", offset_of!(PlxHostCue, reserved)),
        ];
        assert_eq!(fields("PlxHostCue"), cue.map(|(n, _)| n), "PlxHostCue's fields, in order");
        assert!(cue.windows(2).all(|w| w[0].1 < w[1].1));
        // The layout the host marshals: the pointer sits at 24 on both architectures and the struct is 40 bytes on both.
        assert_eq!((offset_of!(PlxHostCue, position_ms), offset_of!(PlxHostCue, text)), (8, 24));
        assert_eq!(std::mem::size_of::<PlxHostCue>(), 40, "36 bytes of fields on ARM, padded to the i64 alignment");
        assert!(event.windows(2).all(|w| w[0].1 < w[1].1));
        assert_eq!(API_V2_SIZE as usize, std::mem::size_of::<PlxHostApi>(), "version 2 ends at the last field");
        assert_eq!(EVENT_V2_SIZE as usize, std::mem::size_of::<PlxHostEvent>());
        assert_eq!(API_V1_SIZE as usize, offset_of!(PlxHostApi, player_open), "the player group starts where version 1 ends");
        assert_eq!(EVENT_V1_SIZE as usize, offset_of!(PlxHostEvent, epoch), "the player payload starts where version 1 ends");
        // Fixed offsets, so a reordering that keeps the names is still caught: the layout the host marshals.
        assert_eq!((offset_of!(PlxHostEvent, a), offset_of!(PlxHostEvent, b), EVENT_V2_SIZE), (16, 24, 32));
        for name in ["PLX_HOST_API_V2_SIZE", "PLX_HOST_EVENT_V2_SIZE"] {
            assert!(code().contains(&format!("#define {name} ")), "the header defines {name}");
        }
        let code = code();
        for decl in [
            "int32_t plx_host_start(const PlxHostApi *api, const char *launch_json);",
            "int32_t plx_host_post(const PlxHostEvent *event);",
            "int32_t plx_host_post_cue(const PlxHostCue *cue);",
            "void plx_host_stop(void);",
            "typedef void (*PlxHostTask)(void *arg);",
            "typedef void (*PlxOnStopped)(int32_t code, int32_t clean);",
            "typedef int32_t (*PlxRunOnHost)(PlxHostTask task, void *arg);",
            "typedef int32_t (*PlxPlayerOpen)(uint32_t epoch, const char *uri, int32_t x, int32_t y, int32_t w, int32_t h);",
            "typedef int32_t (*PlxPlayerVerb)(uint32_t epoch, uint32_t verb, int64_t arg0, int64_t arg1);",
        ] {
            assert!(code.contains(decl), "the header declares `{decl}`");
        }
    }

    unsafe extern "C" fn stopped(_: i32, _: i32) {}
    unsafe extern "C" fn run(_: HostTask, _: *mut c_void) -> i32 {
        0
    }

    unsafe extern "C" fn open(_: u32, _: *const std::os::raw::c_char, _: i32, _: i32, _: i32, _: i32) -> i32 {
        0
    }
    unsafe extern "C" fn verb(_: u32, _: u32, _: i64, _: i64) -> i32 {
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
            player_open: Some(open),
            player_verb: Some(verb),
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
        let later = PlxHostApi { size: API_V2_SIZE + 64, abi_version: 3, ..ok };
        assert!(unsafe { read_table(&later) }.is_ok(), "a later version's table is read as far as version 2 covers");
    }

    /// **The player group is read only when the table offers it**: a version-2 table with both pointers has
    /// it; a table that stops at `run_on_host` (a version-1 host), a version-1 table of a bigger size, and a
    /// version-2 table with either pointer NULL are all ACCEPTED (skew is "no playback", never a refused
    /// start) and carry the reason.
    #[test]
    fn the_player_group_is_read_only_when_the_table_offers_it() {
        let ok = table();
        assert!(unsafe { read_table(&ok) }.unwrap().player.is_ok());
        let v1 = PlxHostApi { size: API_V1_SIZE, abi_version: 1, player_open: None, player_verb: None, ..ok };
        assert_eq!(unsafe { read_table(&v1) }.unwrap().player.err(), Some(NoPlayer::TableV1));
        let short = PlxHostApi { size: API_V1_SIZE, ..ok };
        assert_eq!(unsafe { read_table(&short) }.unwrap().player.err(), Some(NoPlayer::TableV1), "size stops at run_on_host");
        let versioned = PlxHostApi { abi_version: 1, ..ok };
        assert_eq!(unsafe { read_table(&versioned) }.unwrap().player.err(), Some(NoPlayer::TableV1), "version 1 names no player group");
        for broken in [PlxHostApi { player_open: None, ..ok }, PlxHostApi { player_verb: None, ..ok }] {
            assert_eq!(unsafe { read_table(&broken) }.unwrap().player.err(), Some(NoPlayer::NullPointer));
        }
    }

    /// A version-1 host's table is a prefix in memory: only `API_V1_SIZE` bytes are readable, and only those
    /// are read (the library must not run past a v1 host's struct).
    #[test]
    fn a_version_one_table_is_read_without_touching_the_bytes_after_it() {
        let ok = table();
        let mut raw = vec![0u8; API_V1_SIZE as usize];
        // SAFETY: copies the v1 prefix of a live table into a buffer of exactly that size.
        unsafe { std::ptr::copy_nonoverlapping((&ok as *const PlxHostApi).cast::<u8>(), raw.as_mut_ptr(), raw.len()) };
        raw[..4].copy_from_slice(&API_V1_SIZE.to_ne_bytes());
        raw[4..8].copy_from_slice(&1u32.to_ne_bytes());
        // SAFETY: the buffer is API_V1_SIZE bytes, which is what its `size` word says.
        let read = unsafe { read_table(raw.as_ptr().cast()) }.expect("a v1 table is accepted");
        assert_eq!(read.player.err(), Some(NoPlayer::TableV1));
    }

    #[test]
    fn an_event_is_decoded_by_its_kind_and_size() {
        let ev = |size, kind| PlxHostEvent { size, kind, epoch: 0, reserved: 0, a: 0, b: 0 };
        assert_eq!(unsafe { read_event(&ev(EVENT_V1_SIZE, EVENT_PAUSE)) }, Ok(HostEvent::Pause));
        assert_eq!(unsafe { read_event(&ev(EVENT_V1_SIZE, EVENT_RESUME)) }, Ok(HostEvent::Resume));
        assert_eq!(unsafe { read_event(&ev(EVENT_V1_SIZE, 0)) }, Err(E_ARGS));
        assert_eq!(unsafe { read_event(&ev(EVENT_V1_SIZE, 99)) }, Err(E_ARGS));
        assert_eq!(unsafe { read_event(&ev(EVENT_V1_SIZE - 1, EVENT_PAUSE)) }, Err(E_ARGS));
        assert_eq!(unsafe { read_event(std::ptr::null()) }, Err(E_ARGS));
    }

    /// A player event is decoded with its epoch and payload words; a short `size` (a version-1 sized event
    /// carrying a player kind) is refused, and a version-1 pause is still accepted at its own size.
    #[test]
    fn the_language_word_is_the_codes_ascii_bytes_and_anything_else_is_none() {
        let word = |s: &str| { let mut b = [0u8; 8]; b[..s.len()].copy_from_slice(s.as_bytes()); i64::from_le_bytes(b) };
        assert_eq!(language_of_word(word("deu")), Some("deu".to_owned()));
        assert_eq!(language_of_word(word("en")), Some("en".to_owned()));
        assert_eq!(language_of_word(0x0075_6564), Some("deu".to_owned()), "first character in the lowest byte");
        assert_eq!(language_of_word(0), None, "no code");
        assert_eq!(language_of_word(word("e1")), None, "not letters");
        assert_eq!(language_of_word(word("de-DE")), None, "not a bare code");
        assert_eq!(language_of_word(word("abcdefgh")), None, "longer than a language code");
    }

    #[test]
    fn a_player_event_is_decoded_and_a_short_one_is_refused() {
        let ev = |size, kind, epoch, a, b| PlxHostEvent { size, kind, epoch, reserved: 0, a, b };
        assert_eq!(
            unsafe { read_event(&ev(EVENT_V2_SIZE, EVENT_PLAYER_SEEK_DONE, 7, 42, 91_000)) },
            Ok(HostEvent::Player(PlayerEvent { epoch: 7, kind: PlayerKind::SeekDone, a: 42, b: 91_000 }))
        );
        assert_eq!(
            unsafe { read_event(&ev(EVENT_V2_SIZE + 16, EVENT_PLAYER_COMPLETED, 3, 0, 0)) },
            Ok(HostEvent::Player(PlayerEvent { epoch: 3, kind: PlayerKind::Completed, a: 0, b: 0 })),
            "a later version's bigger event is read as version 2"
        );
        assert_eq!(unsafe { read_event(&ev(EVENT_V1_SIZE, EVENT_PLAYER_COMPLETED, 3, 0, 0)) }, Err(E_ARGS), "short");
        assert_eq!(unsafe { read_event(&ev(EVENT_V2_SIZE - 1, EVENT_PLAYER_POSITION, 3, 9, 0)) }, Err(E_ARGS), "short by one byte");
        assert_eq!(unsafe { read_event(&ev(EVENT_V2_SIZE, EVENT_PLAYER_TEXT_LANGUAGE + 1, 3, 0, 0)) }, Err(E_ARGS), "an unknown kind");
        assert_eq!(unsafe { read_event(&ev(EVENT_V2_SIZE, EVENT_PLAYER_TEXT_LANGUAGE, 3, 1, 0x6E65)) },
            Ok(HostEvent::Player(PlayerEvent { epoch: 3, kind: PlayerKind::TextLanguage, a: 1, b: 0x6E65 })), "a text track's language");
        // A v1 host's pause is 8 bytes in memory: only 8 are readable and only 8 are read.
        let pause = [EVENT_V1_SIZE, EVENT_PAUSE];
        assert_eq!(unsafe { read_event(pause.as_ptr().cast()) }, Ok(HostEvent::Pause));
    }
}
