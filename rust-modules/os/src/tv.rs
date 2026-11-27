//! The television, as everything outside the port sees it (step L15, docs/module-layers.md).
pub mod authority;
pub mod crash_capture;
pub mod device;
pub mod home;
pub mod launch;
pub mod native_video;
pub mod packet_probe;
pub mod plane;
pub mod secure;
pub mod sink;
pub mod sound;
pub mod toast;
pub mod uri;

pub use authority::StorageHooks;
/// The layout group's type is `plx_base`'s (`paths` cannot read `tv`); platforms name it from here, like the other groups.
pub use plx_base::paths::Layout;
/// The libraries group's type is `plx_base`'s too (`dynlib` is below `tv`, and `plx_net` and `plx_gfx` read it there).
pub use plx_base::dynlib::Libraries;

use std::sync::OnceLock;

/// What the platform's settings service said about the locale. i18n owns the parse and the log lines.
pub enum LocaleReply { NoPlatform, Unavailable, Reply(String) }

/// What the lower layers read from the platform, in eight groups. Installed once ([`install`]) before
/// anything reads it, and read only by `tv`'s own modules, so no other module can issue a verb the
/// interfaces below do not offer. The exceptions are [`layout`](Services::layout) and
/// [`libraries`](Services::libraries): they are no verbs, and the application's `Platform::install_services`
/// reads them to publish them into `plx_base::paths` and `plx_base::dynlib`. The window, the bus pump and the
/// frame probe are NOT here: only the loop reads them, so they belong to the application's own `Platform`.
///
/// Every hook is required. A platform states each field of each group, and a named `NONE` is that
/// statement; a gap filled in by `Default` is not allowed. The one exemption is
/// [`LegacyWebosMedia`], which a platform without the Starfish and ACB verbs leaves at its `NONE`.
pub struct Services {
    pub identity: IdentityHooks,
    pub secure: SecureHooks,
    pub system: SystemHooks,
    pub storage: StorageHooks,
    /// Where the platform's payload and state live. `Platform::install_services` publishes it into
    /// `plx_base::paths`, right after the install: the one group `tv` itself never reads, because `paths`
    /// is below `tv` and is what reads it.
    pub layout: Layout,
    /// The names of the OS-owned libraries that differ per platform: libcurl and the EGL probe's. Like
    /// [`layout`](Services::layout) it is no verb and `tv` never reads it: `Platform::install_services`
    /// publishes it into `plx_base::dynlib`, which the tables in `plx_net` and `plx_gfx` read. A platform
    /// with neither states [`Libraries::NONE`].
    pub libraries: Libraries,
    pub crash_capture: crash_capture::Hooks,
    pub legacy_webos_media: LegacyWebosMedia,
}

/// What the device is, what it can do and what is wrong with it.
pub struct IdentityHooks {
    pub probe_device: fn(),
    pub start_capability_probe: fn(),
    /// The user-confirmed repair of a blocked native video path ([`native_video::repair`]), for a
    /// platform that has one. `None` is a platform with nothing to repair: the repair then answers
    /// [`native_video::Failure::Unsupported`].
    pub repair_native_video: Option<fn() -> Result<(), native_video::Failure>>,
    /// The packet-player probe ([`packet_probe::command`]), for a platform that offers one. `None` is a platform
    /// without it: the probe answers [`packet_probe::Status::Unavailable`] and Settings shows no row for it.
    pub packet_probe: Option<fn(packet_probe::Command) -> packet_probe::Status>,
    /// The files the capability probe parses, read the platform's way ([`device::files`]).
    pub device_files: device::Files,
    pub system_locale: fn() -> LocaleReply,
    pub sound_output: fn() -> sound::Reply,
}

/// The credential store, and whether this platform keeps one the app may use at all.
pub struct SecureHooks {
    pub seal: fn(&[u8]) -> Option<secure::Sealed>,
    pub open: fn(&secure::Sealed) -> Option<Vec<u8>>,
    pub remove: fn(&secure::Backend, &str),
    /// What [`secure::mode`] answers: a value the platform states, not a hook it runs.
    pub mode: secure::StoreMode,
}

/// The platform's Home key and its system toast.
pub struct SystemHooks {
    pub go_home: fn(),
    pub poll_home: fn(),
    pub deliver_toast: fn(&str, toast::Identity) -> toast::Sent,
}

/// The Starfish, ACB and `com.webos.media` verbs: webOS's own playback seam, named for what it is.
pub struct LegacyWebosMedia {
    /// The Starfish-shaped verb sink (see `sink::VideoSink`); the OS-neutral sink is step L15b.
    /// Read only by [`sink::installed`], the one way out of `tv` for it.
    pub sink: &'static dyn sink::VideoSink,
    /// Shared display binding, available independently of a packet-feed executor.
    pub display_sink: &'static dyn sink::DisplaySink,
    /// The URI media pipe; a platform without one provides `NoUriMedia`.
    /// Read only by [`uri::installed`].
    pub uri_media: &'static dyn uri::UriMedia,
}

fn nothing() {}

impl IdentityHooks {
    /// A platform with no identity of its own to publish: nothing is probed, native video cannot be
    /// repaired and there is no settings service to ask.
    pub const NONE: Self = Self {
        probe_device: nothing,
        start_capability_probe: nothing,
        repair_native_video: None,
        packet_probe: None,
        device_files: device::Files::NONE,
        system_locale: || LocaleReply::NoPlatform,
        sound_output: || sound::Reply::NoPlatform,
    };
}

impl SecureHooks {
    /// No key manager: nothing seals and nothing opens. This alone is NOT [`secure::StoreMode::MemoryOnly`]:
    /// the session layer then writes a plaintext file of its own, so the mode stays `Durable`.
    pub const NONE: Self = Self {
        seal: |_| None,
        open: |_| None,
        remove: |_, _| {},
        mode: secure::StoreMode::Durable,
    };
}

impl SystemHooks {
    /// No Home key and no system bus: the toast says so in the log and reports it was not sent.
    pub const NONE: Self = Self { go_home: nothing, poll_home: nothing, deliver_toast: toast::deliver_without_port };
}

impl LegacyWebosMedia {
    /// No Starfish, no ACB and no URI pipe: every verb is refused.
    pub const NONE: Self = Self { sink: &sink::NoSink, display_sink: &sink::NoSink, uri_media: &uri::NoUriMedia };
}

impl Services {
    /// No platform installed: host unit tests (nothing installs one). Every entry is what today's
    /// `cfg(any(test, feature = "test-support"))` arm answered, minus the off-device log lines the other entries would
    /// write (`deliver_toast` keeps its one).
    pub const HOST_TEST: Self = Self {
        identity: IdentityHooks::NONE,
        secure: SecureHooks::NONE,
        system: SystemHooks::NONE,
        storage: StorageHooks::NONE,
        layout: Layout::DERIVED,
        libraries: Libraries::HOST,
        crash_capture: crash_capture::Hooks::NONE,
        legacy_webos_media: LegacyWebosMedia::NONE,
    };
}

static INSTALLED: OnceLock<&'static Services> = OnceLock::new();

/// Once, by the application's entry, before anything reads them. A second call is refused and changes
/// nothing.
pub fn install(services: &'static Services) -> Result<(), &'static Services> { INSTALLED.set(services) }

/// The installed services. Only `tv`'s own modules read them, so no other module can issue a
/// verb the interfaces below do not offer; the sink leaves through [`sink::installed`] and the
/// URI media pipe through `uri::installed`.
pub(in crate::tv) fn services() -> &'static Services { INSTALLED.get().copied().unwrap_or_else(absent) }

/// Nothing installed. A host test expects that; the shipping build never should, and a silent
/// fall back to the host-test defaults would read as a television that does nothing, so say it once.
fn absent() -> &'static Services {
    #[cfg(not(any(test, feature = "test-support")))]
    {
        static SAID: std::sync::Once = std::sync::Once::new();
        SAID.call_once(|| plx_base::eventlog::log("tv: port not installed - using the no-port defaults"));
    }
    &Services::HOST_TEST
}

pub fn probe_device() { (services().identity.probe_device)() }
pub fn start_capability_probe() { (services().identity.start_capability_probe)() }
pub fn system_locale() -> LocaleReply { (services().identity.system_locale)() }

#[cfg(test)]
mod tests {
    use super::*;

    /// **Every answer of the host-test services, pinned.** The destructuring names each field of each
    /// group and has no `..`, so a hook added to a group breaks this test until it is pinned here.
    #[test]
    fn the_host_test_services_answer_nothing_and_say_so() {
        let Services {
            identity: IdentityHooks { probe_device, start_capability_probe, repair_native_video, packet_probe, device_files, system_locale, sound_output },
            secure: SecureHooks { seal, open, remove, mode },
            system: SystemHooks { go_home, poll_home, deliver_toast },
            storage: StorageHooks { authority },
            layout: Layout { payload_dir, state_root, fallback_dir },
            libraries: Libraries { curl, egl },
            crash_capture: crash_capture::Hooks { start, stop, set_user, record_window },
            legacy_webos_media: LegacyWebosMedia { sink, display_sink, uri_media },
        } = Services::HOST_TEST;

        assert!(start.is_none());
        stop();
        set_user(None);
        record_window(crash_capture::Breadcrumb {
            stage: "test", playing: None, version: None, display: None, surface: None,
        });
        probe_device();
        start_capability_probe();
        assert!(repair_native_video.is_none(), "a platform with nothing to repair states no hook");
        assert!(packet_probe.is_none(), "no packet-player probe");
        assert_eq!(packet_probe::command(packet_probe::Command::Status), packet_probe::Status::Unavailable);
        assert_eq!(native_video::repair(), Err(native_video::Failure::Unsupported));
        assert_eq!(
            (device_files.read)("/etc/umediaserver/device_codec_capability_config.json").unwrap_err().kind(),
            std::io::ErrorKind::NotFound,
            "no files: a read fails as a missing file does"
        );
        assert!(!(device_files.exists)("/"), "and nothing exists");
        assert!(matches!(system_locale(), LocaleReply::NoPlatform));
        assert!(matches!(sound_output(), sound::Reply::NoPlatform));

        let sealed = secure::Sealed {
            backend: secure::Backend::KEYMANAGER3,
            key: "k".into(),
            iv: "i".into(),
            data: "d".into(),
        };
        assert!(seal(b"secret").is_none());
        assert!(open(&sealed).is_none());
        remove(&secure::Backend::PALM_KEYMANAGER, "k");
        assert_eq!(mode, secure::StoreMode::Durable, "no key manager is not a memory-only platform");

        go_home();
        poll_home();
        assert_eq!(
            deliver_toast("hello", toast::Identity::AsApp),
            toast::Sent { reply: None, outcome: toast::Outcome::NoBus }
        );

        assert_eq!(authority, authority::Authority::Files, "no storage helper: the app keeps its records in files");

        assert_eq!(payload_dir(), plx_base::paths::app_dir(), "no layout stated: the payload is where the executable is");
        assert_eq!(state_root(), plx_base::paths::persistent_state_root(), "and the state root is the derived one");
        assert_eq!(fallback_dir, plx_base::paths::NO_FALLBACK_DIR, "and no install prefix to fall back to");

        assert_eq!((curl, egl), (Libraries::HOST.curl, Libraries::HOST.egl), "no platform: the union, the process's own default");

        assert!(!sink.feed_abi_is_cow(), "the refusing sink, not a native one");
        assert_eq!(display_sink.window_mode(), sink::VP_NONE);
        assert_eq!(display_sink.window_id(), c"");
        assert_eq!(uri_media.open(uri::Shape::Anon), Err("no bus".into()));
    }

    /// Nothing is installed in a test process, so every reader falls back to the host-test services.
    #[test]
    fn an_uninstalled_process_reads_the_host_test_services() {
        assert!(INSTALLED.get().is_none(), "no test installs services; the platform's own entry does");
        probe_device();
        start_capability_probe();
        assert!(matches!(system_locale(), LocaleReply::NoPlatform));
        // The installed value itself, not `secure::mode()`: that one honours the process-global test
        // override another test may hold while this one runs.
        assert_eq!(services().secure.mode, secure::StoreMode::Durable);
        assert!(secure::seal(b"secret").is_none());
        assert_eq!(native_video::repair(), Err(native_video::Failure::Unsupported));
        assert_eq!(uri::installed().open(uri::Shape::Anon), Err("no bus".into()));
    }
}
