//! The dev-token boot, armed from the launch string instead of files.
//!
//! The application's dev boot reads two triggers as files in the runtime root (`plxnative-token` and
//! `plxnative-pms-origin`, through `plx_base::devtrig`), which a harness writes over ssh. Nothing can write
//! a file on the Tizen emulator (no `sdb shell`, no file pull: `docs/platform-ports.md`, E5), so the host
//! packages the same two values in the launch string's `dev` object and this module hands them to
//! `devtrig::set_preset`, the platform's table that `devtrig::read` consults after the file. It runs on the
//! host thread inside `plx_host_start`, before the loop starts, and writes no file and needs no runtime
//! root.
//!
//! The token is any non-empty placeholder: the mock server accepts and never checks it, and no real Plex
//! credential belongs here. **The preset exists only in a build with `devtriggers`** (a release build has
//! neither the table nor this call); without that feature the `dev` object is ignored and [`arm`] says so by
//! returning `false`. The log names the armed triggers and never their values.
use crate::launch::Dev;

/// The preset table for `dev`: bare trigger name, then its content. The names are the triggers the
/// application's dev boot reads (`dev/scenarios.rs`). Only a build with dev triggers has a table to fill.
#[cfg(any(feature = "devtriggers", test))]
pub fn table(dev: &Dev) -> Vec<(&'static str, String)> {
    let mut table = Vec::new();
    if let Some(token) = &dev.token {
        table.push(("token", token.clone()));
    }
    if let Some(origin) = &dev.pms_origin {
        table.push(("pms-origin", origin.clone()));
    }
    table
}

/// Hand `dev` to the trigger table. `true` when a preset was set; `false` when there is nothing to set, the
/// build has no dev triggers, or the table was already set.
#[cfg(feature = "devtriggers")]
pub fn arm(dev: &Dev) -> bool {
    let table = table(dev);
    if table.is_empty() {
        return false;
    }
    let names: Vec<&str> = table.iter().map(|(name, _)| *name).collect();
    match plx_base::devtrig::set_preset(&table) {
        Ok(()) => {
            plx_base::eventlog::log(&format!("devboot: armed {names:?} from the launch string"));
            true
        }
        Err(_) => {
            plx_base::eventlog::log("devboot: the trigger table was already set; the launch string's dev values are not used");
            false
        }
    }
}

/// A build without dev triggers has no table to fill: the launch string's `dev` object is ignored.
#[cfg(not(feature = "devtriggers"))]
pub fn arm(_dev: &Dev) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_carries_the_two_triggers_the_boot_reads() {
        let dev = Dev { token: Some("placeholder".into()), pms_origin: Some("http://10.0.2.2:32499".into()) };
        assert_eq!(
            table(&dev),
            [("token", "placeholder".to_string()), ("pms-origin", "http://10.0.2.2:32499".to_string())]
        );
        assert_eq!(table(&Dev { token: Some("t".into()), pms_origin: None }), [("token", "t".to_string())]);
        assert_eq!(table(&Dev::default()), []);
    }

    /// **The preset reaches the readers, and only the two names do.** One test (the table is once per
    /// process): before it `devtrig::read` finds nothing, after it the token and the origin read back as a
    /// file's content would, the log shows the names and never a value, and a second arm is refused.
    #[cfg(feature = "devtriggers")]
    #[test]
    fn arming_makes_the_dev_boot_triggers_readable_and_logs_no_value() {
        // A private trigger root, so a file a harness left in the shared one cannot answer for the preset.
        plx_base::devtrig::with_private_triggers(|| {
            assert_eq!(plx_base::devtrig::read("token"), None);
            let dev = Dev { token: Some("placeholder-token".into()), pms_origin: Some("http://10.0.2.2:32499".into()) };
            let log = plx_base::eventlog::with_private_log(|| {
                assert!(arm(&dev));
                std::fs::read_to_string(plx_base::eventlog::events_log()).unwrap_or_default()
            });
            assert_eq!(plx_base::devtrig::read("token").as_deref(), Some("placeholder-token"));
            assert_eq!(plx_base::devtrig::read("pms-origin").as_deref(), Some("http://10.0.2.2:32499"));
            assert!(log.contains("devboot: armed [\"token\", \"pms-origin\"]"), "{log}");
            assert!(!log.contains("placeholder-token") && !log.contains("10.0.2.2"), "no value in the log: {log}");
            plx_base::eventlog::with_private_log(|| assert!(!arm(&dev), "a second arm is refused"));
            assert!(!arm(&Dev::default()));
        });
    }

    #[cfg(not(feature = "devtriggers"))]
    #[test]
    fn a_build_without_dev_triggers_ignores_the_dev_object() {
        assert!(!arm(&Dev { token: Some("t".into()), pms_origin: None }));
    }
}
