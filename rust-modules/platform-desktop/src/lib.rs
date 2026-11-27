//! The desktop platform: what `plxnative-sim` and the macOS app hand the application.
//!
//! A desktop has no television's services, so this is a platform in its own right and not the webOS
//! port with its answers faked: [`PLATFORM`] states every hook the application reads, from the
//! shared `NONE` groups of `plx_os::tv` wherever a host has nothing to say (the key manager, the
//! settings service, the device files, the URI media pipe) and with its own body where it has
//! something to do:
//!
//! - the capability probe publishes the host's answer, and honours the same `dvcaps0` / `dvcaps1`
//!   dev triggers the television's does (`devcaps::dv`, which both probes end in);
//! - the root press is a log line, the one thing a Mac window has to say about a launcher that does
//!   not exist;
//! - the system toast says it was not sent;
//! - the video sink is the Feed backend's host sink (`HOST_SINK`) when the build has one, else the
//!   refusing `NoSink`.
//!
//! The shell's hooks and the frame probe are plain no-ops, bar the two that bring SDL up and open its
//! window (`init` and `open`, the shell crate's, with nothing around `SDL_Init`), the frame seam and the
//! heartbeat's ` sim=1` tag: SDL owns the window and its event loop, there is no compositor to hint, no
//! bus to pump and no remote pointer to hide. The frame seam is the desktop's own (`Shell::frame`): the
//! WSLg-aware software frame pacer in `pacer.rs` (a wall-clock read, a sleep and two environment reads,
//! all of it IO and so the platform's), and the simulator's present hooks the application publishes
//! (`host::SIM_PRESENT`).
//! The secure store is
//! `SecureHooks::NONE`: nothing seals, so a session is the 0600 plaintext file the session layer
//! writes itself, which the simulator's harnesses plant and read.
//! The storage group is `StorageHooks::NONE`: there is no storage helper, so the account's records
//! live in files under the persistent state root.
//!
//! This crate names no other platform and reaches the application through `plxnative_modules::host`
//! alone, so a desktop executable contains none of the television's code: the
//! webOS port (the Wayland handles, glib, luna-service2) is `plx_platform_webos`, which no desktop crate
//! depends on and the application never names. The simulator links without a television library to drop.
//!
//! The crate is compiled only under `sim` (its own feature, in no default set), so a default-feature
//! pass that lists this package builds an empty library, and a television build never builds it. It is a
//! mounted module of the `platform desktop` layer (`ci/module-layers.ini`): nothing names it, and it names
//! no other platform (`docs/platform-ports.md`, W7). The `plxnative-sim` binary beside it
//! (`src/bin/sim.rs`) is its executable.
#![cfg(feature = "sim")]
mod pacer;

use plxnative_modules::host::{
    self, sdl_init, sdl_open, Exit, FrameProbe, FrameSeam, GlProfile, InitHooks, InitSteps, MediaBoot, OpenAsk, Placement,
    Platform, SdlFlavor, Shell, Surface, WindowPlan,
};
use plx_os::devcaps::dv::{self, DvCapability, DvProbe, ProbeSource};
use plx_os::tv::toast::{Identity, Outcome, Sent};
use plx_os::tv::{IdentityHooks, Layout, Libraries, LegacyWebosMedia, SecureHooks, Services, StorageHooks, SystemHooks};
use std::ffi::CStr;
use std::os::raw::{c_int, c_void};

/// The Feed backend's host sink, built with the simulator's decoded picture.
#[cfg(feature = "play-feed")]
const SINK: &dyn plx_os::tv::sink::VideoSink = &host::HOST_SINK;
/// A build without the Feed backend has no picture to show: every verb is refused.
#[cfg(not(feature = "play-feed"))]
const SINK: &dyn plx_os::tv::sink::VideoSink = &plx_os::tv::sink::NoSink;

/// The display binding the table must state. The desktop runs no plane boot, so nothing reads it in
/// production; it is the host sink only so that the hook is never `NoSink`.
const DISPLAY: &dyn plx_os::tv::sink::DisplaySink = SINK;

/// What the lower layers read from the desktop platform. Every hook is stated, as `tv` requires: a
/// hook a host has nothing to say for is the shared `NONE` group's own, named, so a hook added to a
/// group is a compile error here until this platform states it.
static SERVICES: Services = Services {
    identity: IdentityHooks {
        probe_device: IdentityHooks::NONE.probe_device,
        start_capability_probe,
        repair_native_video: IdentityHooks::NONE.repair_native_video,
        packet_probe: IdentityHooks::NONE.packet_probe,
        device_files: IdentityHooks::NONE.device_files,
        system_locale: IdentityHooks::NONE.system_locale,
        sound_output: IdentityHooks::NONE.sound_output,
    },
    secure: SecureHooks::NONE,
    system: SystemHooks { go_home, poll_home: SystemHooks::NONE.poll_home, deliver_toast },
    storage: StorageHooks::NONE,
    layout: Layout::DERIVED,
    // One crate serves the Mac (libcurl in the dyld shared cache: `libcurl.4.dylib`) and Linux/WSLg (`.so.4`),
    // and a developer's Linux box may carry either EGL: the union of every name, stated here.
    libraries: Libraries {
        curl: &["libcurl.so.4", "libcurl.so.5", "libcurl.4.dylib"],
        egl: &["libEGLfk.so.2", "libEGL.so.1", "libEGL.so"],
    },
    crash_capture: plx_os::tv::crash_capture::Hooks::NONE,
    legacy_webos_media: LegacyWebosMedia {
        sink: SINK,
        display_sink: DISPLAY,
        uri_media: LegacyWebosMedia::NONE.uri_media,
    },
};

/// The boot capability probe: a host has no configd to ask, so it answers `unknown`, from the host,
/// unless a dev trigger forces the answer. Synchronous, with no worker: the answer needs no bus,
/// and the app reads it only after boot.
fn start_capability_probe() {
    let started = std::time::Instant::now();
    if dv::publish_forced(started) {
        return;
    }
    dv::publish_logged(
        DvProbe { capability: DvCapability::Unknown, source: ProbeSource::Host, reason: "host" },
        started,
        None,
        None,
    );
}

/// The root press: there is no launcher to hand a desktop window to, so the screen does not change
/// and the press is a log line, the one the simulator's skill (`ui-sim`) tells a reader to look for.
fn go_home() {
    plx_base::eventlog::log("gohome: request mode=auto");
    plx_base::eventlog::log(
        "gohome: no LS2 bus off-device — the root press would launch com.webos.app.home on a television",
    );
}

/// The system toast: a host has no notification service this app may call, so nothing is sent.
fn deliver_toast(_message: &str, identity: Identity) -> Sent {
    plx_base::eventlog::log(&format!("toast: no system notifications on this host — {identity:?} toast not sent"));
    Sent { reply: None, outcome: Outcome::NoBus }
}

/// What the desktop platform states about its SDL: the stock SDL2 a desktop links ([`SdlFlavor::Stock`]'s event
/// layout), a core 4.1 context (macOS has no GLES at all), no step around `SDL_Init`, and the one step behind the
/// GL context a core profile needs: the vertex array object it has no default for.
const SDL: InitHooks = InitHooks {
    steps: InitSteps { before: InitSteps::NONE.before, after: InitSteps::NONE.after, after_context: bind_core_profile_vao },
    flavor: SdlFlavor::Stock,
    gl: GlProfile::Core41,
};

/// The window a desktop asks for: centred at whatever fits the display, sized for its pixel density once a
/// window exists (the shell crate's `Placement::Desktop`), titled as a desktop shows it. The request is the
/// desktop's own size, not a setting, so the stored Interface resolution is not read and there is no retry.
const WINDOW: WindowPlan = WindowPlan { title: c"PlxNative", placement: Placement::Desktop };

/// Bind the one vertex array object a desktop core profile requires.
///
/// **GLES2 has no VAOs and a core profile has no DEFAULT one**, and the difference is silent: with
/// VAO 0 bound, `glVertexAttribPointer` and `glDrawArrays` raise `GL_INVALID_OPERATION` and draw
/// nothing at all. `glClear` is unaffected, so the symptom is a window painted the clear colour and
/// a completely empty interface — which is exactly what the first simulator screenshots were, at
/// every frame sampled.
///
/// One VAO for the process is enough and is not a simplification: the renderer has a single
/// attribute layout — attribute 0, the shared unit quad in one static VBO, set up by `init_gl` and
/// never rebound. There is nothing for a second VAO to describe.
///
/// **This is the desktop platform's, and only its**, because these two entry points do not EXIST in GLES2:
/// the television's libGLESv2 exports neither, so naming them in a crate the television links would be a
/// link-time undefined symbol on the device. The shell crate runs it right behind the context's creation
/// (`InitSteps::after_context`), before the renderer's first call.
///
/// # Safety
/// On the thread whose GL context was just made current, which is where the shell crate calls it.
unsafe fn bind_core_profile_vao() {
    extern "C" {
        fn glGetString(name: u32) -> *const u8;
        fn glGenVertexArrays(n: c_int, arrays: *mut u32);
        fn glBindVertexArray(array: u32);
    }
    /// `GL_VERSION`.
    const GL_VERSION: u32 = 0x1F02;
    // This platform is not necessarily a core profile — Mesa GLES2 exists on desktops too, and there VAO 0
    // is legal and these entry points are absent. Ask the driver, as the renderer's shader preamble does.
    let version = glGetString(GL_VERSION);
    let es = version.is_null() || std::ffi::CStr::from_ptr(version.cast()).to_bytes().starts_with(b"OpenGL ES");
    if es {
        return;
    }
    let mut vao: u32 = 0;
    glGenVertexArrays(1, &mut vao);
    glBindVertexArray(vao);
}

/// The desktop shell: SDL owns the window and its event loop (`init` and `open` are the shell crate's,
/// with no hook around `SDL_Init`), so nothing is bound, grabbed, hinted or pumped, and no surface is
/// ever held. The pointer is a mouse the application does not hide, so hiding and showing it do
/// nothing, and a click is always a real one (the hide is not a cursor swap).
struct DesktopShell;

impl Shell for DesktopShell {
    fn sdl(&self) -> InitHooks {
        SDL
    }
    fn window(&self, _stored: Option<(c_int, c_int)>) -> WindowPlan {
        WINDOW
    }
    fn init(&self, mt: &host::MainThread) -> Result<(), Exit> {
        // SAFETY: the desktop states no step around `SDL_Init`, and its one other step (the vertex array)
        // is run by the shell crate behind the context, not here.
        unsafe { sdl_init(mt, &SDL) }
    }
    fn open(&self, mt: &host::MainThread, ask: &OpenAsk) -> Result<Box<dyn Surface>, Exit> {
        // SAFETY: `SDL` is this platform's own table (its `after_context` is `bind_core_profile_vao`, sound with the new
        // context current), called on the main thread `mt` proves.
        unsafe { sdl_open(mt, &SDL, ask) }
    }
    fn bind_window(&self, _win: *mut c_void) {}
    fn grab_surface(&self, _win: *mut c_void) {}
    fn release_surface(&self) -> bool {
        false
    }
    fn arm_opaque_region(&self) {}
    fn opaque_route(&self, _player: bool) {}
    fn clear_opaque_region(&self) {}
    fn pump_bus(&self) {}
    fn hide_pointer(&self) {}
    fn show_pointer_after_motion(&self) {}
    fn hide_is_cursor_swap(&self) -> bool {
        false
    }
    fn frame(&self) -> &'static FrameSeam {
        &FRAME
    }
}

/// The desktop's frame seam: the WSLg-aware software pacer (`pacer`: armed on Linux under WSL's X11 driver,
/// where the swap does not wait for the display) and the simulator's present hooks (`host::SIM_PRESENT`: the
/// compositor stand-in under a bound frame, and the supersampled copy to the window), which the application
/// keeps because they draw.
static FRAME: FrameSeam = FrameSeam { pacer: pacer::PACER, present: host::SIM_PRESENT };

/// The desktop frame probe: always unarmed, there is no compositor callback to clock.
struct DesktopProbe;

impl FrameProbe for DesktopProbe {
    fn request(&self) {}
    fn prepare_begin(&self) {}
    fn prepare_end(&self) {}
    fn waiting(&self) {}
    fn acquired(&self) {}
    fn fields(&self) -> String {
        String::new()
    }
    /// Every simulator heartbeat ends ` sim=1`. The heartbeat is the app's perf surface: `tests/run.py --fps`
    /// grades `loop=` and `fps=` from it, and the floors are calibrated to the SM9000's Mali. A Mac renders the
    /// same interface through a completely different GPU, driver and compositor, so those numbers are not merely
    /// optimistic, they are about a different machine. A log line is the unit that gets pasted into an issue or
    /// handed between agents, so the disclaimer has to travel ON the line rather than sit in a doc. Tools grep
    /// the text.
    fn heartbeat_tag(&self) -> &'static str {
        " sim=1"
    }
}

/// The desktop's media boot, the ready value the application publishes (`host::FEED_MEDIA`): nothing to
/// initialize before the first adapter (the plane owner's default facts are the absent ones the host
/// sink's `VP_EXPORTED`/`VP_NONE` boot would state), and the Feed over the host sink as its only driver.
/// It composes no URI driver (that is the television's ACB driver), so it returns no resolver and the
/// session is published the Feed alone.
const MEDIA: MediaBoot = host::FEED_MEDIA;

/// The desktop platform: what [`run`] hands `host::run`.
static PLATFORM: Platform = Platform::new(
    &SERVICES,
    &DesktopShell,
    &DesktopProbe,
    MEDIA,
);

/// The desktop entry: boot the application on [`PLATFORM`] and return the status it ended with.
/// Ending the process is the executable's (`src/bin/sim.rs`), which owns it, like the television's
/// entry does (`port::plex_main` in `plx_platform_webos`): a process that hosts the application never does.
///
/// Nothing is logged before the application's entry: a boot it refuses early must leave the log as the
/// harnesses expect (`tests/replay_fixtures.py`).
pub fn run(pms_host: &CStr, pms_port: u16) -> c_int {
    host::run(&PLATFORM, pms_host, pms_port).code
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The desktop hides no pointer and says so**: its mouse is not the application's to hide, so a
    /// hide and a show do nothing, and the hide is not a cursor swap, which is what makes a click
    /// always a real one (`input::click_is_ok`). The simulator's pointer behaviour is exactly this.
    #[test]
    fn the_desktop_shell_hides_no_pointer_and_its_clicks_are_real() {
        let shell: &dyn Shell = &DesktopShell;
        shell.hide_pointer();
        shell.show_pointer_after_motion();
        assert!(!shell.hide_is_cursor_swap());
    }

    /// **The desktop's frame seam is the pacer and the simulator's present hooks, and the heartbeat says
    /// it is a simulator.** Off Linux the pacer is never armed; on Linux it is armed only under WSLg
    /// (`pacer.rs` grades the two facts), so a test machine's own environment never has to be WSL.
    #[test]
    fn the_desktop_frame_seam_states_its_pacer_and_the_simulators_present_hooks() {
        let shell: &dyn Shell = &DesktopShell;
        let frame = shell.frame();
        assert!(std::ptr::fn_addr_eq(frame.present.compose_under, host::SIM_PRESENT.compose_under));
        assert!(std::ptr::fn_addr_eq(frame.present.before_swap, host::SIM_PRESENT.before_swap));
        #[cfg(not(target_os = "linux"))]
        assert!(!(frame.pacer.armed)(true), "off Linux the driver's swap paces the frame");
        assert!(!(frame.pacer.armed)(false), "an uncapped run (frame dump, novsync) never waits on the display");
        assert_eq!(DesktopProbe.heartbeat_tag(), " sim=1");
    }

    /// A desktop has no storage helper: the session, the install's preferences and the consent record
    /// are files. The same value a host test reads from `Services::HOST_TEST`.
    #[test]
    fn the_desktop_table_keeps_the_account_in_files() {
        assert_eq!(SERVICES.storage.authority, plx_os::tv::authority::Authority::Files);
    }

    /// **The desktop states the union of library names and no install prefix.** The lists are the ones a
    /// process that published nothing reads (`Libraries::HOST`), so the simulator and the host tests open the
    /// same libcurl and EGL; the Mac's `libcurl.4.dylib` is what lets it sign in at all. The layout is the
    /// derivation, whose last resort is the current directory.
    #[test]
    fn the_desktop_table_states_the_union_of_library_names_and_no_install_prefix() {
        assert_eq!(SERVICES.libraries, Libraries::HOST);
        assert_eq!(SERVICES.libraries.curl, ["libcurl.so.4", "libcurl.so.5", "libcurl.4.dylib"]);
        assert_eq!(SERVICES.libraries.egl, ["libEGLfk.so.2", "libEGL.so.1", "libEGL.so"]);
        assert_eq!(SERVICES.layout.fallback_dir, plx_base::paths::NO_FALLBACK_DIR);
    }

    /// **The desktop declares no SDL function.** The window, the GL context, the queue and the text-input
    /// panel are `plx_shell_sdl`'s (`init` and `open` above reach them through `host::sdl_init` and
    /// `host::sdl_open`), and the television's LG half is the webOS crate's; a declaration here, in the
    /// library or the `plxnative-sim` binary, would be a second SDL seam beside the shell crate's. Reads the
    /// sources, because no host test can see a declaration. (The application crate holds itself to
    /// `clock.rs`'s `SDL_GetTicks` the same way, and the webOS crate to the four it keeps.)
    #[test]
    fn the_desktop_declares_no_sdl_function() {
        fn walk(dir: &std::path::Path, src: &std::path::Path, hits: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).expect("read a source directory") {
                let path = entry.expect("a directory entry").path();
                if path.is_dir() {
                    walk(&path, src, hits);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    let rel = path.strip_prefix(src).expect("under src").to_string_lossy().replace('\\', "/");
                    let text = std::fs::read_to_string(&path).expect("read a source file");
                    // Split with `concat!`, so this file's own text is never what it matches.
                    if text.contains(concat!("fn ", "SDL_")) {
                        hits.push(rel);
                    }
                }
            }
        }
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut hits = Vec::new();
        walk(&src, &src, &mut hits);
        assert!(hits.is_empty(), "an SDL function is declared in the desktop platform: {hits:?}");
    }

    /// The video sink the Feed runs over and the display binding the table states are the one host
    /// sink in the simulator, never `NoSink`: a Feed that ran over `NoSink` would show no picture
    /// and nothing in a simulator run would say so.
    #[cfg(feature = "play-feed")]
    #[test]
    fn the_simulator_port_display_is_the_host_sink() {
        let sink = std::ptr::from_ref(&host::HOST_SINK).cast::<()>();
        let media = &SERVICES.legacy_webos_media;
        assert_eq!(std::ptr::from_ref(media.display_sink).cast::<()>(), sink,
            "display_installed() is the host sink");
        assert_eq!(std::ptr::from_ref(media.sink).cast::<()>(), sink, "so is the video sink");
    }

    /// The desktop's own media boot, the one its `PLATFORM` states, composes the Feed and no URI
    /// driver: the adapter's installed drivers form a set with no `Uri` in it, and the composition
    /// carries no resolver. A resolver here would describe a driver this platform never installed. (What
    /// the application then publishes to the session from a composition is `publish_composition`'s, and
    /// is the application's own to test: a platform cannot reach it.)
    #[test]
    fn the_desktop_platform_composes_the_feed_and_no_resolver() {
        use plx_media::player::adapter::PlayerAdapter;
        let mut adapter = PlayerAdapter::new(unsafe { host::MainThread::assume() });
        let composed = (MEDIA.compose)(&mut adapter, false);
        assert!(composed.resolver.is_none());
        assert!(!adapter.composed_backends().uri);
        #[cfg(feature = "play-feed")]
        assert!(adapter.composed_backends().feed, "the Feed is the desktop's driver");
    }
}
