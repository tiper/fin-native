//! The packet-player probe, as the application sees it: a diagnostic a platform may offer that feeds the set's
//! elementary-stream player a few bundled clips and logs a PASS or FAIL line per step (docs/tizen.md, "The packet
//! player"). Settings > Diagnostics shows its row only where the platform states a hook; the runner asks through
//! [`command`] and hands the answer back to the page as an addressed event.

/// What the page asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Start a probe, unless one is running or the set is playing.
    Start,
    /// Where the probe stands.
    Status,
}

/// Where the probe stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    /// This platform has no probe: the page shows no row.
    #[default]
    Unavailable,
    /// Not run since the app started.
    Idle,
    Running,
    /// The last run's tally; its lines are in the event log.
    Done { pass: u32, fail: u32 },
    /// A Start that could not begin: a video is playing, or the worker did not start.
    Refused,
}

/// The platform's probe, or [`Status::Unavailable`] where it has none.
pub fn command(command: Command) -> Status {
    match super::services().identity.packet_probe {
        Some(hook) => hook(command),
        None => Status::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No platform installed (a host test) is a platform without a probe: both commands answer `Unavailable`, so a
    /// Start does nothing.
    #[test]
    fn with_no_platform_the_probe_is_unavailable() {
        assert_eq!(command(Command::Status), Status::Unavailable);
        assert_eq!(command(Command::Start), Status::Unavailable);
        assert_eq!(Status::default(), Status::Unavailable);
    }
}
