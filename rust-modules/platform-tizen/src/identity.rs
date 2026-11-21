//! What the Tizen build says it is: the platform the Plex server sees, and the locale the host stored.
//!
//! **Emulator only, and it says so.** The Tizen build has no playback and has run on no retail set, so the
//! name and version the server reads (`X-Plex-Platform` / `X-Plex-Platform-Version`, built by
//! `plex::identity` from the published `Platform`) are `Tizen` and `<version> (emulator)`, and the boot logs
//! one line, `platform: Tizen <version> (emulator only; no playback)`. The only seam is
//! `plx_os::tv::device::publish_platform`; nothing here builds a header.
//!
//! No `Info` or `Hardware` is published: their `major` is the webOS firmware major (the capability tables, the
//! session store's migration and the Feed's era all read it), and `0` is the answer they handle for "not a
//! webOS set", which is what a host test and the desktop leave it at.
use crate::layout;
use plx_os::tv::device::{publish_platform, Platform};
use plx_os::tv::LocaleReply;

/// The version string the PMS sees for the Tizen version `version`.
pub fn pms_version(version: &str) -> String {
    format!("{version} (emulator)")
}

/// The boot-time device probe: publish the platform and say, once, what this build is.
pub fn probe_device() {
    let version = layout::stored().map_or("unknown", |l| l.platform_version.as_str());
    publish_platform(Platform { name: "Tizen", version: pms_version(version) });
    plx_base::eventlog::log(&format!("platform: Tizen {version} (emulator only; no playback)"));
}

/// The settings reply the i18n layer parses, built from the host's locale tag: the shape the television's
/// settings service answers (`localeInfo.locales.{UI,FMT}` and `clock`), so the one parser serves both.
pub fn locale_reply(tag: &str) -> String {
    serde_json::json!({
        "returnValue": true,
        "settings": { "localeInfo": { "locales": { "UI": tag, "FMT": tag }, "clock": "locale" } }
    })
    .to_string()
}

/// `Services.identity.system_locale`: the locale the host sent in the launch string.
pub fn system_locale() -> LocaleReply {
    LocaleReply::Reply(locale_reply(layout::stored().map_or("en-US", |l| l.locale.as_str())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_reads_the_version_as_an_emulator() {
        assert_eq!(pms_version("10.0"), "10.0 (emulator)");
    }

    /// The reply carries the tag in both locale slots and parses as the settings service's reply does.
    #[test]
    fn the_locale_reply_has_the_settings_shape() {
        let v: serde_json::Value = serde_json::from_str(&locale_reply("be-BY")).unwrap();
        assert_eq!(v["returnValue"].as_bool(), Some(true));
        assert_eq!(v["settings"]["localeInfo"]["locales"]["UI"].as_str(), Some("be-BY"));
        assert_eq!(v["settings"]["localeInfo"]["locales"]["FMT"].as_str(), Some("be-BY"));
        assert_eq!(v["settings"]["localeInfo"]["clock"].as_str(), Some("locale"));
    }

    /// **The probe publishes `Tizen` and one honest log line.** The platform cell is once per process, so
    /// this is the one test that publishes: before it nothing is published, after it the name and the
    /// `(emulator)` version are what `plex::identity` will put on the wire.
    #[test]
    fn the_probe_publishes_the_platform_and_logs_one_line() {
        assert!(plx_os::tv::device::platform().is_none());
        let log = plx_base::eventlog::with_private_log(|| {
            probe_device();
            std::fs::read_to_string(plx_base::eventlog::events_log()).unwrap_or_default()
        });
        let published = plx_os::tv::device::platform().expect("published");
        assert_eq!(published.name, "Tizen");
        assert_eq!(published.version, "unknown (emulator)", "no launch values stored in this test: the default");
        assert_eq!(
            log.lines().filter(|l| l.contains("platform: Tizen")).collect::<Vec<_>>(),
            ["platform: Tizen unknown (emulator only; no playback)"]
        );
    }
}
