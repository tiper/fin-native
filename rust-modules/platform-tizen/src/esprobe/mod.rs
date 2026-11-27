//! The packet-player probe: does this television's elementary-stream player take the packets the Feed engine would
//! give it? (docs/tizen.md, "The packet player".)
//!
//! It feeds the set's `libesplusplayer.so` the clips this package ships (`res/esprobe/*.plxes`, made by
//! `tools/make-esprobe-clips.py`, plus a PCM tone it makes), never the viewer's media, and writes one `esprobe: PASS` or
//! `esprobe: FAIL` line per step to the event log, which reaches the UDP log channel: the API and its struct layouts,
//! whether create and open work without the Partner privilege the managed binding checks, then one player per scenario.
//! It talks to no server and writes nothing anywhere but the log.
//!
//! - [`clip`]: the bundled clip format and the PCM tone;
//! - [`api`]: the player's functions, structs and constants, from the firmware's own binding;
//! - [`machine`]: the Sans-IO logic (which call next, what an answer means);
//! - [`run`]: the worker that makes the calls.
//!
//! One probe at a time: [`start`] refuses while one runs. Settings > Diagnostics starts it through [`command`], the
//! platform's `packet_probe` hook, which also refuses while the host's player is open. The run ends early when the app
//! goes to the background or stops, and at its deadline ([`run`]); a run stuck in a native call past the deadline and a
//! grace is reported done by [`command`], with the tally so far and one more FAIL, while its worker is left to it.

mod api;
mod clip;
mod machine;
mod run;
#[cfg(test)]
mod tests;

use crate::hosted::Hosted;
use machine::{Kind, Scenario};
use plx_base::eventlog::log;
use plx_os::tv::packet_probe::{Command, Status};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

static RUNNING: AtomicBool = AtomicBool::new(false);
/// The last run's tally, `pass << 32 | fail`, or [`NEVER`].
static LAST: AtomicU64 = AtomicU64::new(NEVER);
const NEVER: u64 = u64::MAX;
/// The running probe's tally so far, packed as [`LAST`].
static LIVE: AtomicU64 = AtomicU64::new(0);
/// When the running probe started.
static STARTED: Mutex<Option<Instant>> = Mutex::new(None);
/// The stall has been logged for this run.
static STALL_SAID: AtomicBool = AtomicBool::new(false);
/// Past [`run::PROBE_DEADLINE`], how long a run may still take to end (its teardown, a display wait) before it is
/// reported done.
const STALL_GRACE: Duration = Duration::from_secs(30);

fn pack(pass: u32, fail: u32) -> u64 {
    (u64::from(pass) << 32) | u64::from(fail)
}

fn unpack(packed: u64) -> (u32, u32) {
    ((packed >> 32) as u32, packed as u32)
}

/// A run that started longer ago than its deadline and the grace: its worker is stuck in a native call.
fn overdue() -> bool {
    running()
        && STARTED.lock().unwrap_or_else(|e| e.into_inner()).is_some_and(|t| t.elapsed() > run::PROBE_DEADLINE + STALL_GRACE)
}

/// The bundled clips, by file stem.
const CLIPS: [&str; 8] = ["h264", "aac", "hevc", "hevc10", "av1", "vp9", "ac3", "eac3"];

/// Whether a probe is running now.
pub fn running() -> bool {
    RUNNING.load(Ordering::Acquire)
}

/// The platform's `packet_probe` hook (`plx_os::tv::packet_probe`): start a probe, or say where it stands.
pub fn command(command: Command) -> Status {
    let stalled = overdue();
    if command == Command::Start {
        if stalled {
            log("esprobe: refused: the stalled probe still holds its worker");
            return Status::Refused;
        }
        if !running() {
            if crate::HOSTED.player_live() {
                log("esprobe: refused: the host's player is open");
                return Status::Refused;
            }
            if !start(&crate::HOSTED, &crate::layout::payload_dir(), move |pass, fail| LAST.store(pack(pass, fail), Ordering::Release)) {
                return Status::Refused;
            }
        }
    }
    if stalled && !STALL_SAID.swap(true, Ordering::AcqRel) {
        let (pass, fail) = unpack(LIVE.load(Ordering::Acquire));
        log(&format!(
            "esprobe: timeout: no end {} s after the start (a native call has not returned); reported done as pass={pass} fail={}",
            (run::PROBE_DEADLINE + STALL_GRACE).as_secs(),
            fail.saturating_add(1)
        ));
    }
    status_of(running(), LAST.load(Ordering::Acquire), LIVE.load(Ordering::Acquire), stalled)
}

/// What [`command`] answers from the running flag, the packed last and live tallies, and whether the run is overdue: a
/// stalled run reads done, its tally so far with one more FAIL for the step that never returned.
fn status_of(running: bool, last: u64, live: u64, overdue: bool) -> Status {
    match (running, overdue, last) {
        (true, false, _) => Status::Running,
        (true, true, _) => {
            let (pass, fail) = unpack(live);
            Status::Done { pass, fail: fail.saturating_add(1) }
        }
        (false, _, NEVER) => Status::Idle,
        (false, _, packed) => {
            let (pass, fail) = unpack(packed);
            Status::Done { pass, fail }
        }
    }
}

/// Read the clips from `dir` (the package's `res/esprobe/`): each one parsed, or why not.
fn load_clips(dir: &Path) -> Vec<(&'static str, Result<clip::Clip, String>)> {
    CLIPS
        .iter()
        .map(|&name| {
            let path = dir.join(format!("{name}.plxes"));
            let parsed = std::fs::read(&path)
                .map_err(|e| format!("{name}.plxes unreadable ({:?})", e.kind()))
                .and_then(|bytes| clip::parse(&bytes).map_err(|e| format!("{name}.plxes refused: {e:?}")));
            (name, parsed)
        })
        .collect()
}

/// The scenarios, in order, from the clips that loaded: the clock run on H.264, H.264 with AAC together, then one
/// acceptance run per codec. A scenario whose clip did not load is left out; its clip's FAIL line says why.
pub(crate) fn scenarios(clips: &[(&'static str, Result<clip::Clip, String>)]) -> Vec<Scenario> {
    let get = |name: &str| clips.iter().find(|(n, _)| *n == name).and_then(|(_, c)| c.as_ref().ok()).cloned().map(Arc::new);
    let mut out = Vec::new();
    let h264 = get("h264");
    let aac = get("aac");
    if let Some(v) = &h264 {
        out.push(Scenario { name: "h264-clock", kind: Kind::Clock, video: Some(Arc::clone(v)), audio: None });
    }
    if let (Some(v), Some(a)) = (&h264, &aac) {
        out.push(Scenario { name: "h264+aac", kind: Kind::Accept, video: Some(Arc::clone(v)), audio: Some(Arc::clone(a)) });
    }
    for (name, label) in [("hevc", "hevc"), ("hevc10", "hevc10-hdr10"), ("av1", "av1"), ("vp9", "vp9")] {
        if let Some(v) = get(name) {
            out.push(Scenario { name: label, kind: Kind::Accept, video: Some(v), audio: None });
        }
    }
    for name in ["aac", "ac3", "eac3"] {
        if let Some(a) = get(name) {
            out.push(Scenario { name, kind: Kind::Accept, video: None, audio: Some(a) });
        }
    }
    out.push(Scenario { name: "pcm_s16le", kind: Kind::Accept, video: None, audio: Some(Arc::new(clip::pcm_tone(2))) });
    out
}

/// Start the probe on a worker of its own. `false` when one is already running or the worker could not be started;
/// `on_done(pass, fail)` is called on the worker when it ends.
pub fn start(hosted: &'static Hosted, payload_dir: &Path, on_done: impl FnOnce(u32, u32) + Send + 'static) -> bool {
    if RUNNING.swap(true, Ordering::AcqRel) {
        log("esprobe: refused: a probe is already running");
        return false;
    }
    let dir = payload_dir.join("esprobe");
    LIVE.store(0, Ordering::Release);
    STALL_SAID.store(false, Ordering::Release);
    *STARTED.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    let started = plx_base::task::spawn("esprobe", move || {
        // Cleared however the worker ends: a panic in the run must not leave the row reading Running and every later
        // start refused for the rest of the session.
        struct Clear;
        impl Drop for Clear {
            fn drop(&mut self) {
                RUNNING.store(false, Ordering::Release);
            }
        }
        let _clear = Clear;
        log(&format!("esprobe: start ({} build; clips from {})", std::env::consts::ARCH, dir.display()));
        let clips = load_clips(&dir);
        let preface: Vec<(bool, String, String)> = clips
            .iter()
            .filter_map(|(name, c)| c.as_ref().err().map(|why| (false, format!("clip {name}"), why.clone())))
            .collect();
        let (pass, fail) = run::run(hosted, scenarios(&clips), preface, &|pass, fail| LIVE.store(pack(pass, fail), Ordering::Release));
        log(&format!("esprobe: done pass={pass} fail={fail}"));
        on_done(pass, fail);
    })
    .is_some();
    if !started {
        RUNNING.store(false, Ordering::Release);
    }
    started
}
