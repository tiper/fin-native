//! Where the package's payload and the app's records live: the launch values `plx_host_start` stored, and
//! the two [`Layout`] hooks that answer from them.
//!
//! The host learns both directories at run time (`DirectoryInfo.Resource` and `.Data`) and passes them in
//! the launch string; [`store`] keeps the validated [`Launch`] in one once-set cell on the host thread,
//! FIRST in `plx_host_start`, before the loop thread exists. The application publishes the table's layout
//! group into `plx_base::paths` as the first thing it does, and the hooks run on its first read of either
//! directory, so they never run before the cell is set. Per the hook rules (`plx_base::paths`, `Layout`):
//! neither hook logs (the event log resolves its own path through the same module), `payload_dir` calls no
//! `app_dir` and `state_root` no `persistent_state_root`. A hook that finds the cell empty (a process that
//! never started) answers the derivation, exactly as a platform that stated nothing would.
use crate::launch::Launch;
use plx_base::paths::Layout;
use std::path::PathBuf;
use std::sync::OnceLock;

static STORED: OnceLock<Launch> = OnceLock::new();

/// Keep `launch` for the hooks. Once per process: a second call is refused and the first values stand.
pub fn store(launch: &Launch) -> bool {
    STORED.set(launch.clone()).is_ok()
}

/// The stored launch values, if a start stored them.
pub fn stored() -> Option<&'static Launch> {
    STORED.get()
}

/// The package's read-only payload directory (`res/`: fonts, icons).
pub fn payload_dir() -> PathBuf {
    stored().map_or_else(|| (Layout::DERIVED.payload_dir)(), |l| l.payload_dir.clone())
}

/// The directory the app's own record files live under (the host's data directory).
pub fn state_root() -> PathBuf {
    stored().map_or_else(|| (Layout::DERIVED.state_root)(), |l| l.state_dir.clone())
}

/// The layout group of the Tizen `Services` table.
pub const LAYOUT: Layout = Layout { payload_dir, state_root };

#[cfg(test)]
mod tests {
    use super::*;

    fn launch() -> Launch {
        Launch::parse(
            r#"{"payload_dir":"/opt/usr/globalapps/pkg/res","state_dir":"/opt/usr/home/owner/apps_rw/pkg/data","pms_host":"h","pms_port":1}"#,
        )
        .unwrap()
    }

    /// **The hooks answer what the host sent.** One test, because the cell is once per process: before a
    /// start both hooks answer the derivation (the same answer a platform that stated nothing gives), after
    /// the store they answer the launch string's directories, and a second store changes nothing.
    #[test]
    fn the_hooks_answer_the_stored_directories_and_a_second_store_is_refused() {
        assert_eq!(stored(), None);
        assert_eq!((LAYOUT.payload_dir)(), (Layout::DERIVED.payload_dir)(), "nothing stored: the derivation");
        assert_eq!((LAYOUT.state_root)(), (Layout::DERIVED.state_root)());
        assert!(store(&launch()));
        assert_eq!((LAYOUT.payload_dir)(), PathBuf::from("/opt/usr/globalapps/pkg/res"));
        assert_eq!((LAYOUT.state_root)(), PathBuf::from("/opt/usr/home/owner/apps_rw/pkg/data"));
        let other = Launch { payload_dir: "/elsewhere".into(), ..launch() };
        assert!(!store(&other), "the first values stand");
        assert_eq!((LAYOUT.payload_dir)(), PathBuf::from("/opt/usr/globalapps/pkg/res"));
    }
}
