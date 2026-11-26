//! The Tizen platform: what the application is handed on a Samsung television.
//!
//! **What this is, and is not.** A hosted `.so`: the .NET host (a Tizen `.tpk`, built by the `tizen`
//! packaging recipe, not here) `dlopen`s it, calls [`plx_host_start`](exports), and the application runs on
//! a loop thread this crate starts while the host's main thread stays the Ecore main loop. It has run on the
//! Tizen TV emulator and on no retail set. The x86_64 build appends `(emulator)` to the Plex platform
//! version and the retail ARMv7 build logs its super-alpha status ([`identity`]). **Playback is the `uri` feature's** (W10 part B): built with it, the
//! media boot is [`player::MEDIA`], the URI driver over the host's native player, which the planner offers a narrow set of titles to (host-tested only: nothing has been run on a device, and no title is claimed to play);
//! built without it, the media boot is `host::NO_MEDIA` and nothing plays (`compose: backends=none`).
//! Both configurations have no native crash capture hook.
//!
//! **The contract** is `include/plx_host.h` (the .NET side is written against that text), mirrored by
//! [`host_abi`] and pinned against it by a test. [`hosted`] holds the machinery: start and stop, the loop
//! thread, the input queue, the deadline and the exactly-once `on_stopped`. [`exports`] is the three C
//! symbols and the order `plx_host_start` brings things up in.
//!
//! **The pieces**, one module each:
//!
//! - [`launch`], the launch string the host passes (directories, the mock server's address, the locale, the
//!   UDP log target, the dev values) and [`layout`], the two layout hooks that answer from it;
//! - [`ecore`] and [`egl`], the window (Ecore_Wl2, on the host thread) and the ES2 context (EGL, on the loop
//!   thread), both bound at run time through `plx_base::dynlib!`; [`surface`], the [`Shell`] and [`Surface`]
//!   the application drives, with the hand-off between the two threads;
//! - [`keys`], the remote's key names to the application's `Action`s, and the dump that shows an unbound
//!   name; [`ime`], the system on-screen keyboard (Ecore IMF, on the host thread); [`log_udp`], the event log's tap onto a UDP socket (the emulator has no file access);
//! - [`devboot`], the dev-token boot armed from the launch string instead of trigger files;
//! - [`identity`], the platform name and the locale;
//! - `player` (the `uri` feature), the Sans-IO URI driver over the host's media player, what the platform certifies
//!   for the planner, and the composition that installs both.
//!
//! Like the other platform crates it reaches the application through `plxnative_modules::host` alone,
//! names no other platform, and is named by nothing. The library is compiled only under `tizen` (its
//! own feature, in no default set), so a default-feature pass that lists this package builds an empty
//! library and no build of the television's or the desktop's ever contains it.
#![cfg(feature = "tizen")]

mod devboot;
mod ecore;
mod egl;
mod exports;
mod host_abi;
mod hosted;
mod identity;
mod ime;
mod keys;
mod launch;
mod layout;
mod log_udp;
#[cfg(feature = "uri")]
mod player;
mod surface;

use plx_os::tv::authority::Authority;
use plx_os::tv::secure::StoreMode;
use plx_os::tv::{IdentityHooks, Libraries, LegacyWebosMedia, SecureHooks, Services, StorageHooks, SystemHooks};
use plxnative_modules::host::{self, Platform};

/// The process's one hosted run: the host's callbacks, the input queue, the stop gate. The four exports and
/// the Ecore handlers reach it; nothing else does.
pub(crate) static HOSTED: hosted::Hosted = hosted::Hosted::new();

/// What the lower layers read from the Tizen platform. Every hook is stated, as `tv` requires; where Tizen
/// has nothing to say the shared `NONE` group's own value is named.
///
/// - `identity`: the device probe publishes `Tizen` and the build's platform version; the locale is
///   the host's (`DirectoryInfo`'s culture, passed in the launch string); there is no capability probe, no
///   native video to repair, no device files and no sound-output service.
/// - `secure`: no key manager, and `MemoryOnly`: nothing persists a credential on this platform, so a session
///   is held for the run and the sign-in is asked again at the next (Decision 7 B).
/// - `system`: no Home key and no system bus.
/// - `storage`: no storage helper, the records are files under the state root.
/// - `layout`: the host's two directories ([`layout`]).
/// - `libraries`: the names the shared tables open, measured on the emulator (E12/E13): `libcurl.so.4` and
///   `libEGL.so.1`. This crate's own Ecore and EGL tables are literals in their own modules instead: they
///   load on the host thread BEFORE the install publishes this group, so they cannot be read from it.
/// - `legacy_webos_media`: none: Starfish and ACB are webOS's.
static SERVICES: Services = Services {
    identity: IdentityHooks {
        probe_device: identity::probe_device,
        start_capability_probe: IdentityHooks::NONE.start_capability_probe,
        repair_native_video: IdentityHooks::NONE.repair_native_video,
        device_files: IdentityHooks::NONE.device_files,
        system_locale: identity::system_locale,
        sound_output: IdentityHooks::NONE.sound_output,
    },
    secure: SecureHooks {
        seal: SecureHooks::NONE.seal,
        open: SecureHooks::NONE.open,
        remove: SecureHooks::NONE.remove,
        mode: StoreMode::MemoryOnly,
    },
    system: SystemHooks::NONE,
    storage: StorageHooks { authority: Authority::Files },
    layout: layout::LAYOUT,
    libraries: Libraries { curl: &["libcurl.so.4"], egl: &["libEGL.so.1"] },
    crash_capture: plx_os::tv::crash_capture::Hooks::NONE,
    legacy_webos_media: LegacyWebosMedia::NONE,
};

static SHELL: surface::TizenShell = surface::TizenShell { hosted: &HOSTED };

static PROBE: surface::NoProbe = surface::NoProbe;

/// What the platform composes for playback. With the `uri` feature: the Tizen URI driver over the host's native
/// player ([`player::MEDIA`]). Without it: `host::NO_MEDIA`, no driver and no resolver.
#[cfg(feature = "uri")]
const MEDIA: host::MediaBoot = player::MEDIA;
#[cfg(not(feature = "uri"))]
const MEDIA: host::MediaBoot = host::NO_MEDIA;

/// The Tizen platform: what [`exports`]' loop thread hands `host::run`.
pub(crate) static PLATFORM: Platform = Platform::new(&SERVICES, &SHELL, &PROBE, MEDIA);

#[cfg(test)]
mod tests {
    use super::*;

    /// **The table says what the module note says.** Nothing is persisted (`MemoryOnly`), the records are
    /// files, and the legacy webOS playback pieces are the refusing ones.
    #[test]
    fn the_table_states_a_memory_only_files_platform_with_no_playback() {
        assert_eq!(SERVICES.secure.mode, StoreMode::MemoryOnly);
        assert_eq!(SERVICES.storage.authority, Authority::Files);
        assert!(SERVICES.identity.repair_native_video.is_none());
        assert!(matches!((SERVICES.identity.sound_output)(), plx_os::tv::sound::Reply::NoPlatform));
        assert_eq!(SERVICES.legacy_webos_media.display_sink.window_mode(), plx_os::tv::sink::VP_NONE);
        assert!(!SERVICES.legacy_webos_media.sink.feed_abi_is_cow(), "the refusing sink, not a native one");
    }

    /// **The shared-table names and the install fallback.** The emulator's `libcurl.so.4` and `libEGL.so.1`
    /// (E12/E13), pinned; the EGL name is the one this crate's own EGL table opens, so the probe in
    /// `plx_gfx` and the window's context cannot name two libraries. The layout states no install prefix:
    /// the host names the payload in its launch string.
    #[test]
    fn the_table_states_the_emulators_library_names_and_no_install_prefix() {
        assert_eq!(SERVICES.libraries.curl, ["libcurl.so.4"]);
        assert_eq!(SERVICES.libraries.egl, ["libEGL.so.1"]);
        assert_eq!(SERVICES.libraries.egl, egl::libegl::candidates());
        assert_eq!(SERVICES.layout.fallback_dir, plx_base::paths::NO_FALLBACK_DIR);
    }

    /// Read every source of this crate, with its path relative to `src`.
    fn sources() -> Vec<(String, String)> {
        fn walk(dir: &std::path::Path, src: &std::path::Path, out: &mut Vec<(String, String)>) {
            for entry in std::fs::read_dir(dir).expect("read a source directory") {
                let path = entry.expect("a directory entry").path();
                if path.is_dir() {
                    walk(&path, src, out);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    let rel = path.strip_prefix(src).expect("under src").to_string_lossy().replace('\\', "/");
                    out.push((rel, std::fs::read_to_string(&path).expect("read a source file")));
                }
            }
        }
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut out = Vec::new();
        walk(&src, &src, &mut out);
        out
    }

    /// **The Tizen platform declares no SDL function.** SDL is the television's and the desktop's window
    /// library; this platform's window is Ecore's, its D1 consequence (a null `Surface::window`) rests on SDL
    /// never being initialised here, and a declaration here would be a second SDL seam beside the shell
    /// crate's. Reads the sources, because no host test can see a declaration.
    #[test]
    fn the_tizen_platform_declares_no_sdl_function() {
        let hits: Vec<String> = sources()
            .into_iter()
            // Split with `concat!`, so this file's own text is never what it matches.
            .filter(|(_, text)| text.contains(concat!("fn ", "SDL_")) || text.contains(concat!("SDL_", "Init")))
            .map(|(rel, _)| rel)
            .collect();
        assert!(hits.is_empty(), "SDL is named in the Tizen platform: {hits:?}");
    }

    /// **The application is reached through `host` and nothing else** (Decision 3's amendment): every path
    /// this crate writes into `plxnative_modules` goes through `host`.
    #[test]
    fn the_application_is_named_only_through_host() {
        let mut bad = Vec::new();
        for (rel, text) in sources() {
            for (n, line) in text.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                let (mut rest, needle) = (code, concat!("plxnative_", "modules::"));
                while let Some(at) = rest.find(needle) {
                    let after = &rest[at + needle.len()..];
                    if !after.starts_with("host") {
                        bad.push(format!("{rel}:{}: {}", n + 1, line.trim()));
                    }
                    rest = after;
                }
            }
        }
        assert!(bad.is_empty(), "the application is reached around `host`: {bad:?}");
    }
}
