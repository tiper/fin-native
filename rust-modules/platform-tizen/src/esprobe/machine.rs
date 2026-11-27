//! The probe's logic: which native call comes next, what each answer means, and the PASS / FAIL line it writes.
//!
//! Sans-IO: the machine calls nothing. [`Probe::next`] names the next [`Cmd`] (or `None`: wait for a callback or for
//! time to pass), the runner performs it and hands the answer back through [`Probe::done`], and every native callback
//! arrives through [`Probe::callback`]. Time is the caller's `now_ms`. The lines it writes wait in [`Probe::take_lines`]
//! for the runner to log.
//!
//! Each [`Scenario`] is one player instance from create to destroy. A `Clock` scenario walks the steps the Feed engine
//! needs of a sink, in order: open, display, stream info, prepare (packets are submitted only after the player asks for
//! them, `ready_to_prepare`), start, position advancing, pause holding the position, resume, a seek (the player's own
//! flush: it asks for packets again from an offset, `ready_to_seek`, and reports `seek_done`), the end of stream, and
//! the teardown. An `Accept` scenario stops once the position has advanced: is this codec accepted, and does it play.

use super::clip::Clip;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Audio,
    Video,
}

impl Stream {
    fn name(self) -> &'static str {
        match self {
            Stream::Audio => "audio",
            Stream::Video => "video",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Clock,
    Accept,
}

pub struct Scenario {
    pub name: &'static str,
    pub kind: Kind,
    pub video: Option<Arc<Clip>>,
    pub audio: Option<Arc<Clip>>,
}

/// What the runner is asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    /// Create a player, register its callbacks, open it.
    Open,
    /// Bind it to the library's window as an overlay (on the host's main thread).
    Display,
    StreamInfo(Stream),
    Prepare,
    /// Submit packet `index` of the stream's clip.
    Submit(Stream, usize),
    Eos(Stream),
    Start,
    Pause,
    Resume,
    Seek(u64),
    /// Read the playing time.
    Time,
    /// Stop, close and destroy.
    Teardown,
}

/// The answer to a [`Cmd`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A call's return code (0 is success) or, for [`Cmd::Submit`] and [`Cmd::Eos`], the submit status.
    Rc(i32),
    /// [`Cmd::Open`]: which step failed and its code.
    Open(Result<(), (&'static str, i32)>),
    /// [`Cmd::Display`] could not reach the native call: why.
    NoDisplay(&'static str),
    Time(Result<u64, i32>),
    /// [`Cmd::Teardown`]'s three codes: stop, close, destroy.
    Teardown(i32, i32, i32),
}

/// A native callback, as the runner's mailbox carries it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Callback {
    ReadyToPrepare(Stream),
    PrepareDone(bool),
    ReadyToSeek(Stream, u64),
    SeekDone,
    Eos,
    Error(i32),
    Buffer(Stream, i32),
    ResourceConflicted,
    /// `first_video_decoding_done`: the decoder produced its first picture.
    FirstFrame,
}

/// The privilege the firmware's managed binding checks (Partner level); this package does not declare it.
pub const PRIVILEGE: &str = "http://developer.samsung.com/privilege/esplay";
const PERMISSION_DENIED: i32 = -13;
/// How far ahead of the playing time packets are submitted.
const FEED_AHEAD_MS: u64 = 2000;
/// What is submitted before `prepare_async` may finish.
const PRIME_MS: u64 = 1000;
const FULL_BACKOFF_MS: u64 = 50;
const TIME_EVERY_MS: u64 = 100;
const PREPARE_DEADLINE_MS: u64 = 8000;
const ADVANCE_DEADLINE_MS: u64 = 5000;
const SEEK_DEADLINE_MS: u64 = 5000;
const EOS_GRACE_MS: u64 = 6000;
/// The pause check reads the position twice, this far apart, after letting a pause settle.
const PAUSE_SETTLE_MS: u64 = 400;
const PAUSE_HOLD_MS: u64 = 700;
const PAUSE_TOLERANCE_MS: u64 = 60;
/// Where the clock scenario pauses, seeks from and seeks to.
const PAUSE_AT_MS: u64 = 1500;
const SEEK_FROM_MS: u64 = 3000;
pub const SEEK_TO_MS: u64 = 5500;
const SEEK_LANDING_SLACK_MS: u64 = 700;
/// The accept scenario's "it plays".
const ACCEPT_ADVANCE_MS: u64 = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Open,
    Display,
    InfoVideo,
    InfoAudio,
    Prepare,
    Preparing { since: u64 },
    Start,
    AwaitAdvance { since: u64 },
    Playing,
    PauseCall,
    PauseSettle { until: u64 },
    PauseHold { first: u64, until: u64 },
    ResumeCall { held: u64 },
    AwaitResume { since: u64, held: u64 },
    SeekCall,
    Seeking { since: u64 },
    SeekLand { until: u64 },
    AwaitEos { deadline: u64 },
    Teardown,
    Done,
}

#[derive(Default, Clone, Copy)]
struct Lane {
    present: bool,
    /// The player asked for packets (ready_to_prepare / ready_to_seek) and has not been flushed since.
    open: bool,
    next: usize,
    eos_sent: bool,
}

struct Run {
    scenario: Scenario,
    phase: Phase,
    lanes: [Lane; 2],
    /// The player handle exists, so a failure must still tear it down.
    opened: bool,
    failed: bool,
    position: Option<u64>,
    last_time_at: Option<u64>,
    backoff_until: u64,
    in_flight: Option<Cmd>,
    seek_target: u64,
    /// The clock scenario pauses once.
    paused_once: bool,
    /// When `prepare_async` was accepted, for the first-frame timing.
    prepare_at: u64,
    /// The first decoded picture was reported (or its absence already judged).
    first_frame: bool,
}

fn idx(s: Stream) -> usize {
    match s {
        Stream::Audio => 0,
        Stream::Video => 1,
    }
}

impl Run {
    fn new(scenario: Scenario) -> Run {
        let mut lanes = [Lane::default(); 2];
        lanes[0].present = scenario.audio.is_some();
        lanes[1].present = scenario.video.is_some();
        Run {
            scenario,
            phase: Phase::Open,
            lanes,
            opened: false,
            failed: false,
            position: None,
            last_time_at: None,
            backoff_until: 0,
            in_flight: None,
            seek_target: 0,
            paused_once: false,
            prepare_at: 0,
            first_frame: false,
        }
    }

    fn clip(&self, s: Stream) -> Option<&Arc<Clip>> {
        match s {
            Stream::Audio => self.scenario.audio.as_ref(),
            Stream::Video => self.scenario.video.as_ref(),
        }
    }

    /// The stream whose next packet should go now, if any: the one with the earliest next presentation time among the
    /// open lanes whose next packet is within `horizon`.
    fn feed_candidate(&self, horizon: u64) -> Option<(Stream, usize)> {
        let mut best: Option<(Stream, usize, u64)> = None;
        for s in [Stream::Video, Stream::Audio] {
            let lane = self.lanes[idx(s)];
            if !lane.present || !lane.open {
                continue;
            }
            let Some(clip) = self.clip(s) else { continue };
            let Some(p) = clip.packets.get(lane.next) else { continue };
            if p.pts_ms > horizon {
                continue;
            }
            if best.map_or(true, |(_, _, t)| p.pts_ms < t) {
                best = Some((s, lane.next, p.pts_ms));
            }
        }
        best.map(|(s, i, _)| (s, i))
    }

    fn all_fed(&self) -> bool {
        [Stream::Audio, Stream::Video].iter().all(|&s| {
            let lane = self.lanes[idx(s)];
            !lane.present || self.clip(s).map_or(true, |c| lane.next >= c.packets.len())
        })
    }

    fn end_ms(&self) -> u64 {
        [Stream::Audio, Stream::Video].iter().filter_map(|&s| self.clip(s)).map(|c| c.end_ms()).max().unwrap_or(0)
    }
}

/// The whole probe: the scenarios in order, and the tally.
pub struct Probe {
    queue: std::collections::VecDeque<Scenario>,
    run: Option<Run>,
    lines: Vec<String>,
    pub pass: u32,
    pub fail: u32,
    /// The first open has said what it shows about the privilege.
    privilege_told: bool,
    /// The library offers the first-decoded-picture callback, so a video scenario notes whether it came.
    watch_first_frame: bool,
    /// [`Probe::cancel`] was called: nothing more is started.
    cancelled: bool,
}

impl Probe {
    pub fn new(scenarios: Vec<Scenario>) -> Probe {
        Probe {
            queue: scenarios.into(),
            run: None,
            lines: Vec::new(),
            pass: 0,
            fail: 0,
            privilege_told: false,
            watch_first_frame: false,
            cancelled: false,
        }
    }

    /// End the probe early (the app went to the background or is stopping, or the probe's deadline passed): one FAIL
    /// that says why, the current player torn down through the ordinary [`Cmd::Teardown`], and no further scenario.
    /// Once only; the runner calls it between commands, never with one in flight.
    pub fn cancel(&mut self, why: &str) {
        if self.cancelled {
            return;
        }
        self.cancelled = true;
        self.queue.clear();
        let name = self.scenario_name();
        self.record(false, &format!("{name} cancelled"), why);
        if let Some(run) = self.run.as_mut() {
            run.failed = true;
            if !matches!(run.phase, Phase::Teardown | Phase::Done) {
                run.phase = if run.opened { Phase::Teardown } else { Phase::Done };
            }
        }
    }

    /// The same probe (its tally and unwritten lines) with `scenarios` to run next.
    pub fn with_scenarios(mut self, scenarios: Vec<Scenario>) -> Probe {
        self.queue.extend(scenarios);
        self
    }

    /// Whether the runner registered the first-decoded-picture callback (an optional export).
    pub fn watch_first_frame(&mut self, on: bool) {
        self.watch_first_frame = on;
    }

    /// A line of the runner's own (the API and layout checks), counted like the machine's.
    pub fn record(&mut self, pass: bool, step: &str, detail: &str) {
        if pass {
            self.pass += 1;
        } else {
            self.fail += 1;
        }
        self.lines.push(format!("esprobe: {} {step} {detail}", if pass { "PASS" } else { "FAIL" }));
    }

    pub fn note(&mut self, text: String) {
        self.lines.push(format!("esprobe: {text}"));
    }

    pub fn take_lines(&mut self) -> Vec<String> {
        std::mem::take(&mut self.lines)
    }

    pub fn finished(&self) -> bool {
        self.run.is_none() && self.queue.is_empty()
    }

    /// The packet a [`Cmd::Submit`] names, for the runner to hand over.
    pub fn packet(&self, s: Stream, index: usize) -> Option<(&super::clip::Packet, &Clip)> {
        let run = self.run.as_ref()?;
        let clip = run.clip(s)?;
        Some((clip.packets.get(index)?, clip))
    }

    /// The clip of the current scenario's stream, for [`Cmd::StreamInfo`].
    pub fn clip(&self, s: Stream) -> Option<&Clip> {
        self.run.as_ref()?.clip(s).map(|c| &**c)
    }

    pub fn scenario_name(&self) -> &'static str {
        self.run.as_ref().map_or("-", |r| r.scenario.name)
    }

    fn pass_line(&mut self, step: &str, detail: String) {
        let name = self.scenario_name();
        self.record(true, &format!("{name} {step}"), &detail);
    }

    /// A failed step: the line, then the teardown (or the end of the scenario when nothing was opened). Only a run's
    /// first failure counts: what follows it (an AV1 refusal raises the error callback, then `prepare_async_done(false)`)
    /// is the same cause seen again, so it is a note, and the tally counts failed scenarios' causes, not their echoes.
    fn fail_step(&mut self, step: &str, detail: String) {
        let name = self.scenario_name();
        if self.run.as_ref().is_some_and(|r| r.failed) {
            self.lines.push(format!("esprobe: {name} after the failure: {step} {detail}"));
            return;
        }
        self.record(false, &format!("{name} {step}"), &detail);
        if let Some(run) = self.run.as_mut() {
            run.failed = true;
            run.phase = if run.opened { Phase::Teardown } else { Phase::Done };
        }
    }

    /// The next command, or `None` to wait. Starts the next scenario when the current one is done.
    pub fn next(&mut self, now: u64) -> Option<Cmd> {
        loop {
            if self.run.as_ref().map_or(true, |r| r.phase == Phase::Done) {
                if let Some(done) = self.run.take() {
                    let verdict = if done.failed { "failed" } else { "passed" };
                    self.lines.push(format!("esprobe: scenario {} {verdict}", done.scenario.name));
                }
                let next = self.queue.pop_front()?;
                self.lines.push(format!("esprobe: scenario {} begins", next.name));
                self.run = Some(Run::new(next));
            }
            if self.run.as_ref().is_some_and(|r| r.in_flight.is_some()) {
                return None;
            }
            self.check_deadlines(now);
            let Some(run) = self.run.as_mut() else { continue };
            if run.phase == Phase::Done {
                continue;
            }
            let cmd = Self::choose(run, now);
            // The first-picture callback is a note, never a step: the TV 10.0 emulator decodes and advances the clock
            // without ever raising it, so its absence is evidence to read, not a failure to count.
            if cmd == Some(Cmd::Teardown) && !run.failed && !run.first_frame && run.lanes[idx(Stream::Video)].present
                && self.watch_first_frame
            {
                run.first_frame = true;
                self.lines.push(format!("esprobe: {} first-frame: no first_video_decoding_done callback", run.scenario.name));
            }
            if let Some(cmd) = cmd {
                run.in_flight = Some(cmd);
            }
            return cmd;
        }
    }

    fn choose(run: &mut Run, now: u64) -> Option<Cmd> {
        match run.phase {
            Phase::Open => Some(Cmd::Open),
            Phase::Display => Some(Cmd::Display),
            Phase::InfoVideo => Some(Cmd::StreamInfo(Stream::Video)),
            Phase::InfoAudio => Some(Cmd::StreamInfo(Stream::Audio)),
            Phase::Prepare => Some(Cmd::Prepare),
            Phase::Start => Some(Cmd::Start),
            Phase::PauseCall => Some(Cmd::Pause),
            Phase::ResumeCall { .. } => Some(Cmd::Resume),
            Phase::SeekCall => Some(Cmd::Seek(SEEK_TO_MS)),
            Phase::Teardown => Some(Cmd::Teardown),
            Phase::Done => None,
            Phase::Preparing { .. } => Self::feed(run, now, PRIME_MS),
            Phase::PauseSettle { .. } | Phase::PauseHold { .. } => Self::time_if_due(run, now),
            _ => {
                let horizon = run.position.unwrap_or(0) + FEED_AHEAD_MS;
                if let Some(cmd) = Self::feed(run, now, horizon) {
                    return Some(cmd);
                }
                if run.scenario.kind == Kind::Clock && run.all_fed() && !matches!(run.phase, Phase::Seeking { .. }) {
                    for s in [Stream::Video, Stream::Audio] {
                        let lane = run.lanes[idx(s)];
                        if lane.present && lane.open && !lane.eos_sent {
                            return Some(Cmd::Eos(s));
                        }
                    }
                    let all_sent = run.lanes.iter().all(|l| !l.present || l.eos_sent);
                    if all_sent && run.phase == Phase::Playing {
                        let deadline = now + run.end_ms().saturating_sub(run.position.unwrap_or(0)) + EOS_GRACE_MS;
                        run.phase = Phase::AwaitEos { deadline };
                    }
                }
                Self::time_if_due(run, now)
            }
        }
    }

    fn feed(run: &Run, now: u64, horizon: u64) -> Option<Cmd> {
        if now < run.backoff_until {
            return None;
        }
        run.feed_candidate(horizon).map(|(s, i)| Cmd::Submit(s, i))
    }

    fn time_if_due(run: &Run, now: u64) -> Option<Cmd> {
        if run.last_time_at.map_or(true, |t| now >= t + TIME_EVERY_MS) {
            Some(Cmd::Time)
        } else {
            None
        }
    }

    fn check_deadlines(&mut self, now: u64) {
        let Some(run) = self.run.as_ref() else { return };
        let missed = match run.phase {
            Phase::Preparing { since } if now > since + PREPARE_DEADLINE_MS => Some(("prepare", "no prepare_async_done")),
            Phase::AwaitAdvance { since } if now > since + ADVANCE_DEADLINE_MS => Some(("position", "the position did not advance")),
            Phase::AwaitResume { since, .. } if now > since + ADVANCE_DEADLINE_MS => Some(("resume", "the position did not advance after resume")),
            Phase::Seeking { since } if now > since + SEEK_DEADLINE_MS => Some(("seek", "no seek_done")),
            Phase::AwaitEos { deadline } if now > deadline => Some(("eos", "no eos callback")),
            _ => None,
        };
        if let Some((step, why)) = missed {
            let at = run.position.map_or_else(|| "none".to_owned(), |p| p.to_string());
            self.fail_step(step, format!("{why} in time (position {at} ms)"));
        }
    }

    /// The answer to the command [`next`](Self::next) returned. `took_ms` is how long the call took.
    pub fn done(&mut self, cmd: Cmd, outcome: Outcome, now: u64, took_ms: u64) {
        let Some(run) = self.run.as_mut() else { return };
        if run.in_flight != Some(cmd) {
            return;
        }
        run.in_flight = None;
        let has_video = run.lanes[idx(Stream::Video)].present;
        let has_audio = run.lanes[idx(Stream::Audio)].present;
        match (cmd, outcome) {
            (Cmd::Open, Outcome::Open(Ok(()))) => {
                run.opened = true;
                run.phase = if has_video { Phase::Display } else { Phase::InfoAudio };
                self.pass_line("open", format!("(create, callbacks, open: {took_ms} ms)"));
                if !self.privilege_told {
                    self.privilege_told = true;
                    self.record(true, "privilege", &format!("create and open succeeded without declaring {PRIVILEGE}"));
                }
            }
            (Cmd::Open, Outcome::Open(Err((step, rc)))) => {
                run.opened = step != "create";
                self.fail_step("open", format!("{step} -> {rc} ({})", super::api::error_name(rc)));
                if !self.privilege_told && (rc == PERMISSION_DENIED || step == "create") {
                    self.privilege_told = true;
                    self.record(false, "privilege", &format!("{step} -> {rc} without {PRIVILEGE} declared"));
                }
            }
            (Cmd::Display, Outcome::Rc(0)) => {
                run.phase = Phase::InfoVideo;
                self.pass_line("display", format!("(overlay on the library's window: {took_ms} ms)"));
            }
            (Cmd::Display, Outcome::Rc(rc)) => self.fail_step("display", format!("-> {rc} ({})", super::api::error_name(rc))),
            (Cmd::Display, Outcome::NoDisplay(why)) => self.fail_step("display", why.to_owned()),
            (Cmd::StreamInfo(s), Outcome::Rc(0)) => {
                let codec = run.clip(s).map_or("?", |c| c.codec.name());
                run.phase = if s == Stream::Video && has_audio { Phase::InfoAudio } else { Phase::Prepare };
                self.pass_line(&format!("{}-info", s.name()), format!("{codec} accepted by set_{}_stream_info", s.name()));
            }
            (Cmd::StreamInfo(s), Outcome::Rc(rc)) => {
                let codec = run.clip(s).map_or("?", |c| c.codec.name());
                self.fail_step(&format!("{}-info", s.name()), format!("{codec} refused: -> {rc} ({})", super::api::error_name(rc)));
            }
            (Cmd::Prepare, Outcome::Rc(0)) => {
                run.phase = Phase::Preparing { since: now };
                run.prepare_at = now;
            }
            (Cmd::Prepare, Outcome::Rc(rc)) => self.fail_step("prepare", format!("prepare_async -> {rc} ({})", super::api::error_name(rc))),
            (Cmd::Submit(s, i), Outcome::Rc(status)) => match status {
                super::api::SUBMIT_SUCCESS => run.lanes[idx(s)].next = i + 1,
                super::api::SUBMIT_FULL => run.backoff_until = now + FULL_BACKOFF_MS,
                super::api::SUBMIT_NOT_PREPARED => {
                    // The player is between a flush and its next ready_to_*: hold the lane until it asks again.
                    run.lanes[idx(s)].open = false;
                }
                other => {
                    let name = super::api::submit_name(other);
                    self.fail_step("submit", format!("{} packet {i} -> {other} ({name})", s.name()));
                }
            },
            (Cmd::Eos(s), Outcome::Rc(status)) => {
                run.lanes[idx(s)].eos_sent = true;
                if status != super::api::SUBMIT_SUCCESS {
                    self.fail_step("eos", format!("submit_eos_packet({}) -> {status} ({})", s.name(), super::api::submit_name(status)));
                    return;
                }
            }
            (Cmd::Start, Outcome::Rc(0)) => {
                run.phase = Phase::AwaitAdvance { since: now };
            }
            (Cmd::Start, Outcome::Rc(rc)) => self.fail_step("start", format!("-> {rc} ({})", super::api::error_name(rc))),
            (Cmd::Pause, Outcome::Rc(0)) => {
                run.paused_once = true;
                run.phase = Phase::PauseSettle { until: now + PAUSE_SETTLE_MS };
            }
            (Cmd::Pause, Outcome::Rc(rc)) => self.fail_step("pause", format!("-> {rc} ({})", super::api::error_name(rc))),
            (Cmd::Resume, Outcome::Rc(0)) => {
                let held = match run.phase {
                    Phase::ResumeCall { held } => held,
                    _ => 0,
                };
                run.phase = Phase::AwaitResume { since: now, held };
            }
            (Cmd::Resume, Outcome::Rc(rc)) => self.fail_step("resume", format!("-> {rc} ({})", super::api::error_name(rc))),
            (Cmd::Seek(target), Outcome::Rc(0)) => {
                run.seek_target = target;
                for lane in &mut run.lanes {
                    lane.open = false;
                }
                run.phase = Phase::Seeking { since: now };
            }
            (Cmd::Seek(_), Outcome::Rc(rc)) => self.fail_step("seek", format!("seek -> {rc} ({})", super::api::error_name(rc))),
            (Cmd::Time, Outcome::Time(Ok(ms))) => {
                run.last_time_at = Some(now);
                run.position = Some(ms);
                self.on_position(ms, now);
            }
            (Cmd::Time, Outcome::Time(Err(rc))) => {
                run.last_time_at = Some(now);
                // Before the first frame the time may not be readable yet; only a playing player must answer.
                if matches!(run.phase, Phase::Playing | Phase::PauseHold { .. }) {
                    self.fail_step("position", format!("get_playing_time -> {rc} ({})", super::api::error_name(rc)));
                }
            }
            (Cmd::Teardown, Outcome::Teardown(stop, close, destroy)) => {
                run.phase = Phase::Done;
                let ok = stop == 0 && close == 0 && destroy == 0;
                let detail = format!("stop {stop}, close {close}, destroy {destroy} ({took_ms} ms)");
                let (failed, name) = (run.failed, run.scenario.name);
                if ok {
                    self.pass_line("teardown", detail);
                } else if failed {
                    // The same cause as the failure before it (a player whose open was refused may refuse its stop too).
                    self.lines.push(format!("esprobe: {name} after the failure: teardown {detail}"));
                } else {
                    let name = self.scenario_name();
                    self.record(false, &format!("{name} teardown"), &detail);
                    if let Some(run) = self.run.as_mut() {
                        run.failed = true;
                    }
                }
            }
            (cmd, outcome) => self.fail_step("internal", format!("unexpected answer {outcome:?} to {cmd:?}")),
        }
    }

    fn on_position(&mut self, ms: u64, now: u64) {
        let Some(run) = self.run.as_mut() else { return };
        match run.phase {
            Phase::AwaitAdvance { since } => {
                let need = if run.scenario.kind == Kind::Accept { ACCEPT_ADVANCE_MS } else { 200 };
                if ms >= need {
                    let accept = run.scenario.kind == Kind::Accept;
                    run.phase = if accept { Phase::Teardown } else { Phase::Playing };
                    let step = if accept { "plays" } else { "position" };
                    self.pass_line(step, format!("advancing: {ms} ms, {} ms after start", now - since));
                }
            }
            Phase::Playing if run.scenario.kind == Kind::Clock => {
                if ms >= PAUSE_AT_MS && ms < SEEK_FROM_MS && run.seek_target == 0 && !run.paused_once {
                    run.phase = Phase::PauseCall;
                } else if ms >= SEEK_FROM_MS && run.seek_target == 0 {
                    run.phase = Phase::SeekCall;
                }
            }
            Phase::PauseSettle { until } if now >= until => {
                run.phase = Phase::PauseHold { first: ms, until: now + PAUSE_HOLD_MS };
            }
            Phase::PauseHold { first, until } if now >= until => {
                let moved = ms.abs_diff(first);
                if moved <= PAUSE_TOLERANCE_MS {
                    run.phase = Phase::ResumeCall { held: ms };
                    self.pass_line("pause", format!("position held at {ms} ms (moved {moved} ms in {PAUSE_HOLD_MS} ms)"));
                } else {
                    self.fail_step("pause", format!("position moved {moved} ms while paused ({first} -> {ms})"));
                }
            }
            Phase::AwaitResume { since, held } if ms >= held + 200 => {
                run.phase = Phase::Playing;
                self.pass_line("resume", format!("advancing again: {ms} ms, {} ms after resume", now - since));
            }
            Phase::SeekLand { until } if now >= until => {
                let target = run.seek_target;
                if ms + SEEK_LANDING_SLACK_MS >= target && ms <= target + SEEK_LANDING_SLACK_MS * 2 {
                    run.phase = Phase::Playing;
                    self.pass_line("seek", format!("landed: position {ms} ms for a seek to {target} ms"));
                } else {
                    self.fail_step("seek", format!("position {ms} ms after a seek to {target} ms"));
                }
            }
            _ => {}
        }
    }

    /// A native callback, already moved off the player's thread by the runner.
    pub fn callback(&mut self, cb: Callback, now: u64) {
        let Some(run) = self.run.as_mut() else { return };
        match cb {
            Callback::ReadyToPrepare(s) => {
                if run.lanes[idx(s)].present {
                    run.lanes[idx(s)].open = true;
                }
            }
            Callback::PrepareDone(true) => {
                if let Phase::Preparing { since } = run.phase {
                    run.phase = Phase::Start;
                    let fed: Vec<String> = [Stream::Video, Stream::Audio]
                        .iter()
                        .filter(|&&s| run.lanes[idx(s)].present)
                        .map(|&s| format!("{} {}", s.name(), run.lanes[idx(s)].next))
                        .collect();
                    let detail = format!("prepare_async_done(true) {} ms after prepare; packets before it: {}", now - since, fed.join(", "));
                    let step = if run.scenario.kind == Kind::Accept { "accepted" } else { "prepare" };
                    self.pass_line(step, detail);
                }
            }
            Callback::PrepareDone(false) => self.fail_step("prepare", "prepare_async_done(false)".to_owned()),
            Callback::ReadyToSeek(s, offset) => {
                let lane = &mut run.lanes[idx(s)];
                if lane.present {
                    lane.open = true;
                    lane.eos_sent = false;
                    let clip = run.clip(s).cloned();
                    let resume = clip.as_ref().and_then(|c| c.resume_index(offset));
                    let lane = &mut run.lanes[idx(s)];
                    match resume {
                        Some(i) => lane.next = i,
                        None => lane.next = clip.map_or(0, |c| c.packets.len()),
                    }
                    let i = run.lanes[idx(s)].next;
                    self.lines.push(format!("esprobe: {} ready_to_seek {} offset {offset} ms: resuming at packet {i}", run.scenario.name, s.name()));
                }
            }
            Callback::SeekDone => {
                if let Phase::Seeking { since } = run.phase {
                    run.phase = Phase::SeekLand { until: now + 300 };
                    self.lines.push(format!("esprobe: {} seek_done {} ms after seek", run.scenario.name, now - since));
                }
            }
            Callback::Eos => {
                if let Phase::AwaitEos { .. } = run.phase {
                    run.phase = Phase::Teardown;
                    let at = run.position.map_or_else(|| "?".to_owned(), |p| p.to_string());
                    self.pass_line("eos", format!("eos callback (last position {at} ms)"));
                } else {
                    self.lines.push(format!("esprobe: {} eos callback outside the end step", run.scenario.name));
                }
            }
            Callback::Error(code) => self.fail_step("error", format!("error callback {code} ({})", super::api::error_name(code))),
            Callback::ResourceConflicted => self.fail_step("resource", "resource_conflicted".to_owned()),
            Callback::FirstFrame => {
                if !run.first_frame && run.lanes[idx(Stream::Video)].present {
                    run.first_frame = true;
                    let after = now.saturating_sub(run.prepare_at);
                    self.lines.push(format!("esprobe: {} first-frame: first_video_decoding_done {after} ms after prepare", run.scenario.name));
                }
            }
            Callback::Buffer(s, status) => {
                let what = if status == super::api::BUFFER_UNDERRUN { "underrun" } else { "overrun" };
                self.lines.push(format!("esprobe: {} buffer {} {what}", run.scenario.name, s.name()));
            }
        }
    }
}
