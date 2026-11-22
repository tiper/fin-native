//! Bind a TV library at RUNTIME instead of at link time, by SONAME candidate list.
//!
//! # Why this exists
//!
//! Everything the app links names its library in `DT_NEEDED`, and the dynamic loader treats a
//! missing entry as fatal: the process dies at `exec()`, before `main`, before the event log is
//! open. That is exactly the failure a webOS 5 television gives today — `libAcbAPI.so.1` was
//! deleted at 5.0 and every FFmpeg SONAME moved a major — and it presents to the user as "the
//! app does nothing", with no log line to read and nothing to report.
//!
//! `DT_NEEDED` also cannot express "either of these". One binary cannot depend on
//! `libavformat.so.57` *and* `libavformat.so.58`; naming both means both must be present, which
//! is true on no firmware ever shipped. So a single binary that runs on both eras has to resolve
//! those libraries itself. That is all this module does.
//!
//! # What it is not
//!
//! It is not a general dynamic-loading framework and it is not for libraries that exist
//! everywhere. `libSDL2`, `libGLESv2`, `libglib-2.0`, `libluna-service2`, `libwayland-client`,
//! `libplayerAPIs` and `libpf-1.0` are present with the same SONAME on every release from 4.4.2
//! through 11.2.0 (`tools/fwcompat.py --inventory` will show you), so they stay linked normally
//! and keep real link-time symbol checking. Moving a library here TRADES that checking away for
//! version tolerance, and is only worth it where the version actually varies.
//!
//! # The contract
//!
//! [`dynlib!`] takes a block shaped exactly like the `extern "C"` block it replaces and emits an
//! `unsafe fn` of the same name and signature for each entry, so call sites do not change. The
//! generated `load()` is all-or-nothing: it resolves every symbol or it
//! reports which are missing and leaves the table empty. Callers must gate on that result —
//! `ff::boot()` does, and `ff::demux()` refuses when it failed.
//!
//! Calling a wrapper whose symbol never resolved is a bug in that gating, not a recoverable
//! condition, so it logs the symbol name and panics rather than jumping through a null pointer.
//! The alternative — returning a per-signature sentinel — would let a missing symbol travel as a
//! plausible-looking value into the media pipeline, which is the harder failure to diagnose of
//! the two, on a device where diagnosis means reading a log over ssh.
#![allow(dead_code)]

use crate::choice::Choice;
use libc::{dlerror, dlopen, dlsym, RTLD_GLOBAL, RTLD_NOW};
use std::ffi::CString;
use std::os::raw::c_void;
use std::path::Path;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicPtr, Ordering};

/// `RTLD_NOW | RTLD_GLOBAL`. NOW because a lazily-bound miss would surface as a jump into the
/// resolver at an arbitrary later moment — the whole point here is to learn at load time. GLOBAL
/// because these libraries satisfy each other's imports (libavformat needs libavcodec needs
/// libavutil) and because the ACB path wants its symbols visible to anything loaded after it.
///
/// From `libc`, not hand-written: the flags are not the same numbers everywhere. `0x100` is
/// RTLD_GLOBAL on glibc but RTLD_FIRST on Darwin — and this module's unit tests run on Darwin, so
/// a hardcoded constant would have meant the tests exercised a different mode than the device.
const RTLD_NOW_GLOBAL: libc::c_int = RTLD_NOW | RTLD_GLOBAL;

/// An opened library. Not `Drop` — nothing here is ever closed, deliberately: the media libraries
/// stay mapped for the life of the process, and `dlclose` on a library with live pipeline threads
/// inside it is a way to unmap code that is executing.
#[derive(Clone, Copy)]
pub struct Handle(*mut c_void);
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

impl Handle {
    /// Try each SONAME in order, returning the first that opens **and which one it was**.
    ///
    /// Order is by preference, not by age: put the SONAME you expect on the firmware you most
    /// care about first. It matters only when a device carries both, which does happen —
    /// releases 5.3.1 and 6.4.0 keep a `libcurl.so.5` compat alias beside the real
    /// `libcurl.so.4`, so a list that asked for `.so.4` first would still be answered on a set
    /// where `.so.5` also resolves.
    ///
    /// The name comes back with the handle rather than being recovered by a second pass, because
    /// a second pass means calling `dlopen` again — which bumps the library's reference count and
    /// costs a real open on every candidate that was skipped.
    pub fn open<'a>(candidates: &[&'a str]) -> Option<(Handle, &'a str)> {
        Self::open_in(None, candidates)
    }

    /// The `dlopen` itself, once, so `open_in`'s two branches cannot drift.
    fn dlopen_str(path: &str) -> *mut c_void {
        match CString::new(path) {
            Ok(c) => unsafe { dlopen(c.as_ptr(), RTLD_NOW_GLOBAL) },
            Err(_) => null_mut(),
        }
    }

    /// As [`open`], but resolving names inside `dir` when one is given.
    ///
    /// `dir` is how the BUNDLED FFmpeg is found: those libraries live beside the binary, which is
    /// on no library search path, so a bare SONAME would either fail or — worse, on webOS 11,
    /// which ships FFmpeg 6 itself — silently open the television's copy instead of ours. An
    /// absolute path makes "which library did we get" structural rather than a matter of search
    /// order. (The `-plx` build suffix is the second half of that guarantee.)
    /// `dir` is a `Path`, not a `&str`, because the caller's app directory comes from
    /// `paths::app_dir()` and a `.to_str()` on the way in has a `None` branch — one that would
    /// silently turn "our bundled FFmpeg" into "whatever the loader finds".
    pub fn open_in<'a>(dir: Option<&Path>, candidates: &[&'a str]) -> Option<(Handle, &'a str)> {
        for name in candidates {
            let h = match dir {
                Some(d) => match d.join(name).to_str() {
                    Some(p) => Self::dlopen_str(p),
                    // A non-UTF-8 install path. Refuse rather than fall back to a bare SONAME,
                    // which on webOS 11 could open the television's FFmpeg instead of ours.
                    None => continue,
                },
                None => Self::dlopen_str(name),
            };
            if !h.is_null() {
                return Some((Handle(h), name));
            }
        }
        None
    }

    /// As [`open`], but only a library that is **already mapped** into the process
    /// (`RTLD_NOLOAD`): the first candidate somebody else already loaded, never a fresh load.
    ///
    /// For asking questions of the library another component is using — SDL `dlopen`s EGL
    /// `RTLD_LOCAL`, so `RTLD_DEFAULT` cannot see it, and loading a second copy (or a different
    /// implementation under a candidate SONAME) would answer about state that library does not
    /// own. `RTLD_NOW` because glibc requires one of LAZY/NOW alongside NOLOAD; the mode of an
    /// already-loaded object is not downgraded by it.
    pub fn open_loaded<'a>(candidates: &[&'a str]) -> Option<(Handle, &'a str)> {
        candidates.iter().find_map(|name| {
            let c = CString::new(*name).ok()?;
            let h = unsafe { dlopen(c.as_ptr(), RTLD_NOW | libc::RTLD_NOLOAD) };
            (!h.is_null()).then_some((Handle(h), *name))
        })
    }

    /// A handle to the process's own global symbol scope (`RTLD_DEFAULT`), for asking "did the
    /// library that actually loaded bring this entry point" about something already linked.
    ///
    /// This is how the webOS-version splits are detected without linking either side: SDL's
    /// exported-window family exists only from webOS 5.0, `SDL_webOSGetPanelResolution` only from
    /// 4.4.2, and naming either at link time would make the binary demand a symbol that older
    /// televisions do not have.
    pub fn self_handle() -> Handle {
        Handle(std::ptr::null_mut()) // RTLD_DEFAULT
    }

    pub fn sym(&self, name: &str) -> Option<*mut c_void> {
        let c = CString::new(name).ok()?;
        let p = unsafe {
            dlerror(); // clear any stale error so a NULL-valued symbol is distinguishable
            dlsym(self.0, c.as_ptr())
        };
        if p.is_null() && !unsafe { dlerror() }.is_null() {
            None
        } else {
            Some(p)
        }
    }
}

/// The OS-owned library names that differ from one platform to the next: what the platform states in
/// `plx_os::tv::Services::libraries` and [`publish`] hands to this module, once, before the first table is
/// read.
///
/// Two lists, each in the order worth trying (see [`Handle::open`]): `curl` for the three libcurl tables
/// (`plx_net`'s `curl`, `curlmulti` and `raw`), `egl` for the EGL probe (`plx_gfx::egl`). Nothing else
/// is here on purpose: the bundled `-plx` FFmpeg and libass, zlib and the gst libraries are the same
/// everywhere, and a platform crate's OWN tables (Tizen's Ecore and EGL window chain load before the
/// install, on the host thread) stay literals inside that crate. A table reads its list through
/// [`dynlib!`]'s `platform(..)` form, which calls [`libraries`] when it loads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Libraries {
    pub curl: &'static [&'static str],
    pub egl: &'static [&'static str],
}

impl Libraries {
    /// A platform with no libcurl and no EGL to name: a table loaded from it reports `NoLibrary` with its
    /// own log line (see [`load_into`]), never a panic. No shipped platform states it.
    pub const NONE: Self = Self { curl: &[], egl: &[] };

    /// What a read sees when NOTHING was published (a host unit test, a tool that never runs a platform's
    /// entry): the union of every name any platform of this project opens, so such a process still finds
    /// libcurl and EGL wherever it runs. `libcurl.4.dylib` is the Mac's, in the dyld shared cache
    /// (`dlopen("libcurl.4.dylib")` resolves it with no install), and is what lets the desktop simulator
    /// sign in at all; a television never reaches it. `libEGLfk.so.2` is first: webOS 2.2.3-5.3.1, which
    /// is every release this app ran on before 6.
    pub const HOST: Self = Self {
        curl: &["libcurl.so.4", "libcurl.so.5", "libcurl.4.dylib"],
        egl: &["libEGLfk.so.2", "libEGL.so.1", "libEGL.so"],
    };
}

static LIBRARIES: Choice<Libraries> = Choice::new();

/// The platform's [`Libraries`], once: published by the application's entry before the first read
/// (`Platform::install_services`), and refused like [`paths::publish`](crate::paths::publish) is when a
/// second one arrives or a table was read first. **Never logs**; the caller reports a refusal.
pub fn publish(libraries: Libraries) -> Result<(), crate::choice::PublishError> {
    LIBRARIES.publish(libraries)
}

/// The names to try. The first read with nothing published freezes the answer to [`Libraries::HOST`].
pub fn libraries() -> Libraries {
    libraries_of(&LIBRARIES)
}

fn libraries_of(cell: &Choice<Libraries>) -> Libraries {
    cell.get().copied().unwrap_or(Libraries::HOST)
}

/// What a `load()` did. `Ok` means every symbol resolved and the table is live.
pub enum Loaded {
    Ok(&'static str),
    /// None of the candidate SONAMEs opened. Carries the list that was tried.
    NoLibrary,
    /// The library opened but is missing symbols. Carries the SONAME and how many are absent;
    /// the names themselves go to the event log, because a caller only ever needs the count.
    Incomplete(&'static str, usize),
}

impl Loaded {
    pub fn ok(&self) -> bool {
        matches!(self, Loaded::Ok(_))
    }
}

/// The panic path for a wrapper whose symbol never resolved. Cold, and it names the symbol,
/// because the whole value of failing here is that the log says which one.
#[cold]
#[inline(never)]
pub fn missing_symbol(candidates: &[&str], sym: &str) -> ! {
    let lib = candidates.first().copied().unwrap_or("(no library named)");
    crate::eventlog::log(&format!(
        "dynlib: FATAL — {sym} was called but never resolved from {lib}; the load gate was skipped"
    ));
    panic!("dynlib: {lib}:{sym} unresolved");
}

/// Resolve `names` from the first candidate SONAME that opens, into `cells`.
///
/// Split out of the macro so the logic exists once rather than once per library: a macro that
/// expands a loop body five times is five copies to keep in step.
pub fn load_into(
    dir: Option<&Path>,
    candidates: &'static [&'static str],
    required: &[(&'static str, &AtomicPtr<c_void>)],
) -> Loaded {
    if candidates.is_empty() {
        // A platform that states no library for this table (`Libraries::NONE`): missing, like any other.
        crate::eventlog::log(&format!(
            "dynlib: the platform names no library for the table that binds {}; treated as missing",
            required.first().map_or("(nothing)", |(name, _)| name)
        ));
        return Loaded::NoLibrary;
    }
    let Some((h, soname)) = Handle::open_in(dir, candidates) else {
        return Loaded::NoLibrary;
    };
    let mut missing = 0usize;
    let mut resolved = Vec::with_capacity(required.len());
    for (name, _) in required {
        match h.sym(name) {
            Some(p) if !p.is_null() => resolved.push(p),
            _ => {
                missing += 1;
                crate::eventlog::log(&format!("dynlib: {soname} has no symbol {name}"));
                resolved.push(null_mut());
            }
        }
    }
    if missing > 0 {
        return Loaded::Incomplete(soname, missing);
    }
    // Publish only once EVERY symbol is known good, so a partial table is never observable.
    for ((_, cell), p) in required.iter().zip(resolved) {
        cell.store(p, Ordering::Release);
    }
    Loaded::Ok(soname)
}

/// The C symbol a wrapper resolves: its own name, or an explicit override. Two wrappers may name
/// the SAME symbol — which is how a variadic C function is bound here: one wrapper per call shape,
/// each naming the concrete types of its own trailing argument.
///
/// **Concrete types, but still declared `...`** — see the macro's `$dots` note. Spelling the
/// trailing argument out is not the same as dropping the ellipsis, and on Apple ARM64 the
/// difference is a SIGSEGV; this doc used to say the opposite in its last line and that is the bug
/// it cost.
#[macro_export]
macro_rules! dynlib_sym {
    ($fname:ident) => {
        stringify!($fname)
    };
    ($fname:ident, $sym:literal) => {
        $sym
    };
}

/// Declare a runtime-bound library. The body is shaped like the `extern "C"` block it replaces.
///
/// ```ignore
/// dynlib! {
///     /// doc comment lands on the generated module
///     avformat: ["libavformat.so.58", "libavformat.so.57"] {
///         fn av_read_frame(s: *mut AVFormatContext, pkt: *mut AVPacket) -> c_int;
///     }
/// }
/// ```
/// emits `mod avformat { pub fn load() -> Loaded; pub fn candidates() -> &'static [&'static str]; }` plus a
/// module-level `unsafe fn av_read_frame(...) -> c_int` that dispatches through the resolved pointer.
///
/// # A list the PLATFORM states
///
/// `curl: platform(curl) { .. }` in place of the literal list names a [`Libraries`] field instead: the
/// candidates are read from [`libraries`] when `load()` runs (and when `candidates()` is asked), so the
/// names a table opens are the platform's own statement and not a literal in a layer. Use it only for an
/// OS-owned library whose name differs per platform; a bundled or same-everywhere library keeps its
/// literal list.
///
/// # A VARIADIC C function: put `...` where C puts it, and spell the rest
///
/// Everything **after** the ellipsis is an argument this wrapper passes through the variadic part
/// of the call — the trailing argument's type is fixed by the option id, which is how one C symbol
/// is bound as more than one wrapper:
///
/// ```ignore
/// fn curl_easy_setopt_ptr = "curl_easy_setopt"(h: *mut CURL, opt: c_int, ..., v: *const c_void)
///     -> c_int;
/// ```
///
/// That reads oddly and it is deliberate: it mirrors `curl.h`, where `h` and `opt` are the only
/// NAMED parameters and everything else arrives through `va_arg`. Getting this wrong is not a
/// style matter — the ellipsis selects a calling convention. **Apple's ARM64 ABI passes variadic
/// arguments on the STACK** while named ones go in registers, so declaring all three as named
/// leaves libcurl reading whatever was on the stack and dereferencing it: `EXC_BAD_ACCESS` inside
/// `_platform_strlen`, from a `dlopen`'d library, with nothing in the app's own log.
///
/// It was latent for exactly as long as the desktop build could not open a libcurl at all, and it
/// surfaced the moment one could — on the FIRST plex.tv call, i.e. sign-in, i.e. the first thing a
/// new user does. ARM32 (the television) and x86-64 pass these two ways identically, which is
/// precisely why no amount of device testing could have found it.
#[macro_export]
macro_rules! dynlib {
    // Crate-private, the form every table uses unless it must be named from another crate.
    (
        $(#[$meta:meta])*
        $modname:ident : [ $($cand:literal),+ $(,)? ] {
            $(
                $(#[$fmeta:meta])*
                fn $fname:ident $(= $sym:literal)? ( $($params:tt)* ) $(-> $ret:ty)? ;
            )*
        }
    ) => {
        $crate::dynlib! {
            @emit [pub(crate)]
            $(#[$meta])*
            $modname : [ &[$($cand),+] ] {
                $( $(#[$fmeta])* fn $fname $(= $sym)? ( $($params)* ) $(-> $ret)? ; )*
            }
        }
    };
    // `dynlib! { pub curl: [...] { ... } }`: the table's module, its cells and its wrappers are
    // `pub`, for a layer crate whose callers in another crate bind the same library (`plx_net`'s
    // `net::curl_easy_*`, which `curlio` calls). Nothing else about the expansion differs.
    (
        $(#[$meta:meta])*
        pub $modname:ident : [ $($cand:literal),+ $(,)? ] {
            $(
                $(#[$fmeta:meta])*
                fn $fname:ident $(= $sym:literal)? ( $($params:tt)* ) $(-> $ret:ty)? ;
            )*
        }
    ) => {
        $crate::dynlib! {
            @emit [pub]
            $(#[$meta])*
            $modname : [ &[$($cand),+] ] {
                $( $(#[$fmeta])* fn $fname $(= $sym)? ( $($params)* ) $(-> $ret)? ; )*
            }
        }
    };
    // The platform's list: `[`...`]` above is a literal, `platform(curl)` names a `Libraries` field.
    (
        $(#[$meta:meta])*
        $modname:ident : platform( $field:ident ) {
            $(
                $(#[$fmeta:meta])*
                fn $fname:ident $(= $sym:literal)? ( $($params:tt)* ) $(-> $ret:ty)? ;
            )*
        }
    ) => {
        $crate::dynlib! {
            @emit [pub(crate)]
            $(#[$meta])*
            $modname : [ $crate::dynlib::libraries().$field ] {
                $( $(#[$fmeta])* fn $fname $(= $sym)? ( $($params)* ) $(-> $ret)? ; )*
            }
        }
    };
    (
        $(#[$meta:meta])*
        pub $modname:ident : platform( $field:ident ) {
            $(
                $(#[$fmeta:meta])*
                fn $fname:ident $(= $sym:literal)? ( $($params:tt)* ) $(-> $ret:ty)? ;
            )*
        }
    ) => {
        $crate::dynlib! {
            @emit [pub]
            $(#[$meta])*
            $modname : [ $crate::dynlib::libraries().$field ] {
                $( $(#[$fmeta])* fn $fname $(= $sym)? ( $($params)* ) $(-> $ret)? ; )*
            }
        }
    };
    (
        @emit [$vis:vis]
        $(#[$meta:meta])*
        // The candidate list as one EXPRESSION of type `&'static [&'static str]`: a literal slice, or
        // the platform's field.
        $modname:ident : [ $cands:expr ] {
            $(
                $(#[$fmeta:meta])*
                // The parameter list is captured as raw tokens and re-parsed by `dynlib_wrapper!`,
                // which has one rule per shape (variadic / not). A `$(...)?` group cannot be used
                // for this: a repetition in the OUTPUT needs a metavariable inside it to know how
                // often to repeat, and an ellipsis is not one.
                fn $fname:ident $(= $sym:literal)? ( $($params:tt)* ) $(-> $ret:ty)? ;
            )*
        }
    ) => {
        $(#[$meta])*
        #[allow(non_upper_case_globals)]
        $vis mod $modname {
            use std::os::raw::c_void;
            use std::sync::atomic::AtomicPtr;

            /// The SONAMEs this table tries, in order: its literal list, or the platform's.
            $vis fn candidates() -> &'static [&'static str] {
                $cands
            }
            $( $vis static $fname: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut()); )*

            /// Open the first candidate SONAME that exists and resolve every required symbol.
            /// `dir` scopes the search to one directory — see `Handle::open_in`.
            $vis fn load(dir: Option<&std::path::Path>) -> $crate::dynlib::Loaded {
                $crate::dynlib::load_into(
                    dir,
                    candidates(),
                    &[$(($crate::dynlib_sym!($fname $(, $sym)?), &$fname)),*],
                )
            }
        }

        $(
            $crate::dynlib_wrapper! {
                $(#[$fmeta])*
                [$vis, $modname, $fname, $crate::dynlib_sym!($fname $(, $sym)?)]
                ( $($params)* ) $(-> $ret)?
            }
        )*

    };
}

/// One dispatching wrapper, in the two shapes a C function comes in. Split out of [`dynlib!`]
/// because the choice is per FUNCTION, and a `macro_rules` arm chooses per INVOCATION — so the
/// parameter list is handed over as tokens and matched again here.
///
/// Both arms are the same three steps: load the resolved pointer, refuse loudly if the load gate
/// was skipped, transmute to the C signature and call. They differ only in that signature.
#[macro_export]
macro_rules! dynlib_wrapper {
    // C-VARIADIC — the `...` sits where `curl.h` puts it, and every argument after it is passed
    // through the variadic part of the call. See `dynlib!`'s doc for why this is not cosmetic.
    (
        $(#[$fmeta:meta])*
        [$vis:vis, $modname:ident, $fname:ident, $sym:expr]
        ( $($arg:ident : $argty:ty),* , ... , $($varg:ident : $vargty:ty),+ $(,)? ) $(-> $ret:ty)?
    ) => {
        $(#[$fmeta])*
        #[inline]
        #[allow(non_snake_case)]
        $vis unsafe fn $fname ( $($arg : $argty,)* $($varg : $vargty),+ ) $(-> $ret)? {
            let p = $modname::$fname.load(std::sync::atomic::Ordering::Relaxed);
            if p.is_null() {
                $crate::dynlib::missing_symbol($modname::candidates(), $sym);
            }
            let f: unsafe extern "C" fn( $($argty,)* ... ) $(-> $ret)? = std::mem::transmute(p);
            f($($arg,)* $($varg),+)
        }
    };
    // The ordinary shape.
    (
        $(#[$fmeta:meta])*
        [$vis:vis, $modname:ident, $fname:ident, $sym:expr]
        ( $($arg:ident : $argty:ty),* $(,)? ) $(-> $ret:ty)?
    ) => {
        $(#[$fmeta])*
        #[inline]
        // The C symbol name IS the contract — `sws_getContext` cannot be spelled any other way.
        // As an `extern` block these were exempt from the style lint; as generated Rust functions
        // they are not.
        #[allow(non_snake_case)]
        $vis unsafe fn $fname ( $($arg : $argty),* ) $(-> $ret)? {
            // Relaxed, not Acquire: an Acquire load emits a full `dmb ish` on this ARM core, on
            // EVERY FFmpeg call. It buys nothing here — the table is published by `ff::boot()` on
            // the main thread before any pipeline thread exists, and `thread::spawn` is itself the
            // synchronising edge for the workers.
            let p = $modname::$fname.load(std::sync::atomic::Ordering::Relaxed);
            if p.is_null() {
                $crate::dynlib::missing_symbol($modname::candidates(), $sym);
            }
            let f: unsafe extern "C" fn($($argty),*) $(-> $ret)? = std::mem::transmute(p);
            f($($arg),*)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The host's own C library, so these tests need no fixture and run wherever `cargo test`
    /// does. They grade the LOADER, not FFmpeg — the dev machine has neither libavformat nor a
    /// television, which is exactly why the loader is the part worth testing here.
    #[cfg(target_os = "macos")]
    static HOST_LIBC: &[&str] = &["libSystem.B.dylib"];
    #[cfg(not(target_os = "macos"))]
    static HOST_LIBC: &[&str] = &["libc.so.6"];

    /// A SONAME that exists on no platform must report `NoLibrary` rather than opening something.
    /// Cheap, but it is the branch every non-TV host takes, including this test runner.
    #[test]
    fn absent_library_is_reported_not_opened() {
        assert!(Handle::open(&["libplxnative-does-not-exist.so.99"]).is_none());
    }

    /// `load_into` on an absent library must report `NoLibrary` and leave every cell null, so a
    /// caller that ignores the verdict crashes at the wrapper rather than calling a stale pointer.
    #[test]
    fn a_failed_load_publishes_nothing() {
        static A: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
        static B: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
        static CAND: &[&str] = &["libplxnative-nope.so.99"];
        let v = load_into(None, CAND, &[("x", &A), ("y", &B)]);
        assert!(matches!(v, Loaded::NoLibrary));
        assert!(A.load(Ordering::Acquire).is_null() && B.load(Ordering::Acquire).is_null());
    }

    /// The all-or-nothing rule: one missing symbol out of two must publish NEITHER. A partial
    /// table is the state where `ff::boot` says "not usable" while a wrapper happily dispatches.
    #[test]
    fn a_partial_load_publishes_nothing() {
        static A: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
        static B: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
        let v = load_into(
            None,
            HOST_LIBC,
            &[("malloc", &A), ("plxnative_no_such_symbol", &B)],
        );
        assert!(
            matches!(v, Loaded::Incomplete(_, 1)),
            "expected one missing symbol"
        );
        assert!(
            A.load(Ordering::Acquire).is_null(),
            "malloc must not be published alone"
        );
    }

    /// The happy path, graded against the host libc so it runs everywhere `cargo test` does.
    #[test]
    fn a_complete_load_publishes_every_cell() {
        static A: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
        static B: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
        let v = load_into(None, HOST_LIBC, &[("malloc", &A), ("free", &B)]);
        assert!(v.ok(), "libc should open and carry malloc/free");
        assert!(!A.load(Ordering::Acquire).is_null() && !B.load(Ordering::Acquire).is_null());
    }

    /// The candidate list is ordered, and the FIRST that opens wins. Graded against the host's
    /// own C library so the test needs no fixture: a bogus name ahead of a real one must fall
    /// through to the real one, which is precisely the 57-vs-58 behaviour the TV path depends on.
    #[test]
    fn first_openable_candidate_wins() {
        assert!(Handle::open(&["libplxnative-nope.so.1", HOST_LIBC[0]]).is_some());
    }

    /// `open_loaded` answers only for a library that is already mapped, and never loads one:
    /// the C library every process has is found, a name nothing loaded is not, and the first
    /// mapped candidate is the one reported.
    #[test]
    fn open_loaded_finds_only_what_is_already_mapped() {
        #[cfg(target_os = "macos")]
        let mapped = "/usr/lib/libSystem.B.dylib";
        #[cfg(not(target_os = "macos"))]
        let mapped = "libc.so.6";
        assert!(Handle::open_loaded(&["libplxnative-nope.so.1"]).is_none());
        let (h, name) = Handle::open_loaded(&["libplxnative-nope.so.1", mapped])
            .expect("the C library is mapped in every process");
        assert_eq!(name, mapped);
        assert!(h.sym("malloc").is_some_and(|p| !p.is_null()));
    }

    /// Libraries that ship with the base system but that a test process has no reason to map.
    /// A LIST, because no single one is on every host: the GitHub Ubuntu runner has no
    /// `libthread_db.so.1`. glibc ships the rest in `libc6` itself (`libanl`/`libutil` survive
    /// 2.34's merge as compat stubs; `libBrokenLocale`, the NSS modules and `libresolv` never
    /// moved).
    #[cfg(target_os = "macos")]
    const UNMAPPED_CANDIDATES: &[&str] = &[
        "/usr/lib/libpanel.5.4.dylib",
        "/usr/lib/libform.5.4.dylib",
        "/usr/lib/libmenu.5.4.dylib",
    ];
    #[cfg(not(target_os = "macos"))]
    const UNMAPPED_CANDIDATES: &[&str] = &[
        "libthread_db.so.1",
        "libBrokenLocale.so.1",
        "libanl.so.1",
        "libnss_dns.so.2",
        "libnss_files.so.2",
        "libutil.so.1",
        "libresolv.so.2",
    ];

    /// The half that needs `RTLD_NOLOAD`: a library that EXISTS but nothing loaded is not
    /// answered — and is still not mapped afterwards, so `open_loaded` did not load it either.
    /// Only then does a real `open` load it, after which `open_loaded` finds it.
    ///
    /// Grades the first candidate that is not mapped AND exists. Never skips: a host where none
    /// qualifies FAILS with the list tried, because a skip would make the NOLOAD mutation guard
    /// vacuous on exactly the machine (CI) that is meant to enforce it.
    #[test]
    fn open_loaded_does_not_load_an_existing_library() {
        let _g = crate::testlock::serial();
        // Mapped-ness is read with a raw NOLOAD `dlopen`, never through the function under
        // test: an `open_loaded` that lost its NOLOAD would otherwise load the library here and
        // pass itself off as "already mapped".
        let is_mapped = |lib: &str| {
            let c = CString::new(lib).unwrap();
            !unsafe { dlopen(c.as_ptr(), RTLD_NOW | libc::RTLD_NOLOAD) }.is_null()
        };
        let mut tried = Vec::new();
        for &lib in UNMAPPED_CANDIDATES {
            if is_mapped(lib) {
                tried.push(format!("{lib}: already mapped"));
                continue;
            }
            assert!(
                Handle::open_loaded(&[lib]).is_none(),
                "open_loaded answered for {lib}, which is not mapped"
            );
            assert!(!is_mapped(lib), "open_loaded loaded {lib}");
            // Existence is learned only now, by loading it for real; a missing one is the next
            // candidate's turn.
            if Handle::open(&[lib]).is_none() {
                tried.push(format!("{lib}: not present"));
                continue;
            }
            assert!(
                Handle::open_loaded(&[lib]).is_some(),
                "open_loaded missed {lib} once it was mapped"
            );
            eprintln!("open_loaded NOLOAD graded against {lib}");
            return;
        }
        panic!("no candidate library is present and unmapped on this host: {tried:?}");
    }
    crate::dynlib! {
        /// A table whose list is the platform's, as `plx_net`'s curl tables are.
        platform_probe: platform(curl) {
            fn plxnative_no_such_symbol() -> i32;
        }
    }

    /// **The lists, pinned.** `NONE` is empty (a platform with no curl and no EGL); `HOST` is the union of
    /// today's names, in today's order, so a process that published nothing opens what it always did.
    #[test]
    fn the_none_group_is_empty_and_the_unset_group_is_the_union_of_todays_names() {
        assert!(Libraries::NONE.curl.is_empty() && Libraries::NONE.egl.is_empty());
        assert_eq!(Libraries::HOST.curl, ["libcurl.so.4", "libcurl.so.5", "libcurl.4.dylib"]);
        assert_eq!(Libraries::HOST.egl, ["libEGLfk.so.2", "libEGL.so.1", "libEGL.so"]);
    }

    /// Nothing is published in a test process, so a read sees the union, a table built with
    /// `platform(..)` answers it, and a publish after that read is refused like `paths::publish`'s is. On
    /// private cells for the successes, because a publish into the process-wide cell would steer every
    /// other test's libcurl.
    #[test]
    fn an_unpublished_read_is_the_union_and_a_late_or_second_publish_is_refused() {
        assert_eq!(libraries(), Libraries::HOST, "no test publishes");
        assert_eq!(platform_probe::candidates(), Libraries::HOST.curl);
        assert_eq!(publish(Libraries::NONE), Err(crate::choice::PublishError::AlreadyRead));
        assert_eq!(libraries(), Libraries::HOST, "the refused group changed nothing");

        let cell = Choice::new();
        let stated = Libraries { curl: &["libcurl.so.4"], egl: &["libEGL.so.1"] };
        assert_eq!(libraries_of(&cell), Libraries::HOST, "an empty cell reads the union");
        assert_eq!(cell.publish(stated), Err(crate::choice::PublishError::AlreadyRead));

        let cell = Choice::new();
        assert_eq!(cell.publish(stated), Ok(()));
        assert_eq!(libraries_of(&cell), stated);
        assert_eq!(cell.publish(Libraries::NONE), Err(crate::choice::PublishError::AlreadyPublished));
        assert_eq!(libraries_of(&cell), stated, "the first group stands");
    }

    /// A table loaded with an empty list fails like a missing library, with its own log line, and
    /// publishes nothing; the wrapper path that names the library never indexes an empty list.
    #[test]
    fn an_empty_list_is_a_missing_library_not_a_panic() {
        static A: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
        let v = load_into(None, &[], &[("x", &A)]);
        assert!(matches!(v, Loaded::NoLibrary));
        assert!(A.load(Ordering::Acquire).is_null());
        let lost = std::panic::catch_unwind(|| missing_symbol(&[], "x")).expect_err("a refused call panics");
        let text = lost.downcast_ref::<String>().cloned().unwrap_or_default();
        assert!(text.contains("(no library named):x"), "{text}");
    }
}
