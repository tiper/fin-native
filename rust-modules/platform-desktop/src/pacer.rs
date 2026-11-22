//! The desktop's software frame pacer, for WSLg.
//!
//! WSLg's X11/GLX swap can accept interval 1 without waiting for the Windows compositor, so a presented
//! frame returns at once and the loop would spin. On Linux, under WSL with SDL's X11 driver, the pacer
//! sleeps out what is left of a 60 Hz frame period after the swap. Everywhere else (native Linux, macOS,
//! Windows' own SDL) the driver's swap paces the frame and the pacer is not armed.
//!
//! This is IO (a wall-clock read, a sleep, two environment reads), so it is the platform's and not the
//! application's: the loop (`app::run`) asks `armed` once at boot, and hands the stamp `begin` returned to
//! `end`. Recorded UI replays continue to see only their injected ticks, because the application never
//! holds a clock for this. The Linux-only bodies are `cfg(target_os = "linux")`; elsewhere the pacer is
//! `Pacer::NONE`.

use plxnative_modules::host::Pacer;

/// The desktop's pacer: the WSLg one on Linux.
#[cfg(target_os = "linux")]
pub(crate) const PACER: Pacer = Pacer { armed, begin, end };

/// The desktop's pacer off Linux: the driver's swap paces the frame, so nothing is armed.
#[cfg(not(target_os = "linux"))]
pub(crate) const PACER: Pacer = Pacer::NONE;

/// Whether the pacer is armed for this boot: vsync is on (a frame dump and `plxnative-novsync` run at swap
/// interval 0 and never wait on the display), and the process runs under WSLg's X11 SDL driver.
#[cfg(target_os = "linux")]
fn armed(vsync: bool) -> bool {
    vsync && under_wslg_x11(std::env::var_os("WSL_DISTRO_NAME").is_some(), std::env::var("SDL_VIDEODRIVER").ok().as_deref())
}

/// The WSLg condition, from the two facts the environment gives: a WSL distro name is set, and SDL's video
/// driver is X11.
#[cfg(target_os = "linux")]
fn under_wslg_x11(wsl_distro_set: bool, sdl_video_driver: Option<&str>) -> bool {
    wsl_distro_set && sdl_video_driver == Some("x11")
}

/// One frame period at 60 Hz, in nanoseconds.
#[cfg(target_os = "linux")]
const PERIOD_NS: u64 = 16_666_667;

/// The platform's monotonic epoch: stamps are nanoseconds since it, so the stamp is a plain `u64` the
/// application can carry without naming a clock.
#[cfg(target_os = "linux")]
fn epoch() -> std::time::Instant {
    static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    *EPOCH.get_or_init(std::time::Instant::now)
}

#[cfg(target_os = "linux")]
fn now_ns() -> u64 {
    u64::try_from(epoch().elapsed().as_nanos()).unwrap_or(u64::MAX)
}

/// The top of a paced frame.
#[cfg(target_os = "linux")]
fn begin() -> u64 {
    now_ns()
}

/// After the swap: sleep what is left of the frame period.
#[cfg(target_os = "linux")]
fn end(started: u64) {
    if let Some(remaining) = remaining_ns(started, now_ns()) {
        std::thread::sleep(std::time::Duration::from_nanos(remaining));
    }
}

/// What is left of the frame period when `now` follows `started`; `None` once the frame took its period or more.
#[cfg(target_os = "linux")]
fn remaining_ns(started: u64, now: u64) -> Option<u64> {
    PERIOD_NS.checked_sub(now.saturating_sub(started)).filter(|left| *left > 0)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    /// WSLg's two facts, both needed: a distro name alone (WSL with Wayland) or the X11 driver alone (a
    /// native X11 desktop) leaves the driver's own swap pacing the frame.
    #[test]
    fn the_pacer_arms_only_under_wsl_with_the_x11_driver() {
        assert!(under_wslg_x11(true, Some("x11")));
        assert!(!under_wslg_x11(true, Some("wayland")));
        assert!(!under_wslg_x11(true, None));
        assert!(!under_wslg_x11(false, Some("x11")));
    }

    /// A frame that took less than the period waits out the rest; one that took the period or more does not wait.
    #[test]
    fn a_frame_waits_out_what_is_left_of_its_period_and_never_more() {
        assert_eq!(remaining_ns(1_000, 1_000), Some(PERIOD_NS));
        assert_eq!(remaining_ns(0, 6_666_667), Some(10_000_000));
        assert_eq!(remaining_ns(0, PERIOD_NS), None);
        assert_eq!(remaining_ns(0, PERIOD_NS + 1), None);
        // A stamp in the future (the clock never goes back, but the arithmetic must not wrap) waits one period at most.
        assert_eq!(remaining_ns(10, 5), Some(PERIOD_NS));
    }

    /// The stamp is monotonic: a later `begin` is never smaller than an earlier one.
    #[test]
    fn the_stamp_does_not_go_backwards() {
        let first = begin();
        assert!(begin() >= first);
    }
}
