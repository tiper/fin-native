//! Debug control commands for the remote FIFO and Unix socket.
//! Arm the socket at launch with the instance's plxnative-api dev trigger.

mod socket;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use serde_json::json;

use plx_base::eventlog::log;
use plx_media::player::backend::LogicalRequestId;
use plx_media::player::PlaybackState;

static PENDING: Mutex<VecDeque<socket::Request>> = Mutex::new(VecDeque::new());

/// What a parked request is waiting for.
enum What {
    /// `play:<rk>` — answers `play_started` (with the title) on the first frame that
    /// reaches `playing`; `play_failed` on the engine's verdict, a supersession, a
    /// metadata refusal, or the deadline.
    Play {
        rk: String,
        request: Option<LogicalRequestId>,
    },
    /// Wait for the exact addressed seek to land or be retired.
    Seek {
        ms: u64,
        generation: u32,
        operation: SeekOperation,
    },
    Transport {
        play: bool,
        generation: u32,
    },
    /// `profile:<index>` — the switch is `profiles → switching → ready|error`, so the
    /// entry waits OUT the `switching` leg: `ready` is the seated profile (with its
    /// title), `error` is the flow's error line, a return to the roster after the switch
    /// began is a dead pick, and any other phase is the flow leaving without it.
    /// `seen_switching` tells the first still-on-the-roster frames (the command is
    /// processed a frame or two after the pick) from a return to the roster.
    Profile {
        index: usize,
        uuid: String,
        correlation: u32,
        epoch: Option<u64>,
        seen_switching: bool,
    },
    /// `shot` — answers `shot_captured` when the numbered PNG is written (shot.rs) or
    /// `shot_failed` when the capture fails or on the deadline.
    Shot,
}
enum SeekOperation {
    Feed(u32),
    Uri {
        attempt: plx_playback_contract::AttemptId,
        command: plx_playback_contract::CommandId,
    },
}

impl SeekOperation {
    fn submitted(
        ps: &plx_media::route::PlaybackSession,
        player: &plx_media::player::adapter::PlayerAdapter,
    ) -> Option<Self> {
        if ps.attempt_view().backend() == Some(plx_media::player::attempt_view::Backend::Uri) {
            let snapshot = player.backend_snapshot();
            Some(Self::Uri {
                attempt: snapshot.attempt?,
                command: snapshot.last_queued?,
            })
        } else {
            Some(Self::Feed(ps.readout().seek_epoch()))
        }
    }

    /// None means this operation still owes a landing; false means it was retired.
    fn settled(
        &self,
        ps: &plx_media::route::PlaybackSession,
        uri: &plx_playback_contract::Snapshot,
    ) -> Option<bool> {
        match *self {
            Self::Feed(epoch) if ps.readout().seek_epoch() != epoch => Some(false),
            Self::Feed(epoch) if ps.readout().raw().seek_landed_epoch == epoch => Some(true),
            Self::Feed(_) => (!ps.readout().seeking()).then_some(false),
            Self::Uri { attempt, .. } if uri.attempt != Some(attempt) => Some(false),
            Self::Uri { command, .. } if uri.seek_landed == Some(command) => Some(true),
            Self::Uri { command, .. } => uri
                .seek_landed
                .filter(|landed| *landed > command)
                .map(|_| false),
        }
    }
}

/// One parked typed request: the id its answer must echo, and the instant it parked.
pub(super) struct InFlight {
    reply: socket::ReplyTo,
    since_ms: u32,
    what: What,
}
/// In-flight horizons: a play that never seats, a seek that never lands, a switch that
/// never settles, a capture that never fires — each answers its `*failed` outcome at the
/// deadline instead of leaving the client hanging.
const AWAIT_PLAY_SECS: u32 = 120;
const AWAIT_SEEK_SECS: u32 = 10;
const AWAIT_TRANSPORT_SECS: u32 = 10;
const AWAIT_PROFILE_SECS: u32 = 60;
const AWAIT_SHOT_SECS: u32 = 10;
/// Frame-owned requests are admitted before their effects are committed.
const AWAITING_MAX: usize = 16;

fn can_park(waiting: &VecDeque<InFlight>, verb: &socket::Verb) -> bool {
    waiting.len() < AWAITING_MAX
        && !waiting.iter().any(|entry| {
            matches!(
                (verb, &entry.what),
                (socket::Verb::Profile { .. }, What::Profile { .. })
                    | (socket::Verb::Seek { .. }, What::Seek { .. })
                    | (socket::Verb::Shot, What::Shot)
            )
        })
}

fn arm_awaiting(waiting: &mut VecDeque<InFlight>, reply: socket::ReplyTo, now_ms: u32, what: What) {
    waiting.push_back(InFlight {
        reply,
        since_ms: now_ms,
        what,
    });
}

fn drop_plays(waiting: &mut VecDeque<InFlight>, reason: &str, state: PlaybackState) {
    waiting.retain(|entry| {
        let What::Play { rk, .. } = &entry.what else {
            return true;
        };
        entry.reply.send(
            socket::Reply::ok(socket::Outcome::PlayFailed {
                rk: rk.clone(),
                reason: reason.to_owned(),
            })
            .with_state(state_str(state)),
        );
        false
    });
}

/// Correlate the shared play continuation with the route generation it started.
pub(super) fn play_wait_finished(
    waiting: &mut VecDeque<InFlight>,
    before: Option<LogicalRequestId>,
    after: Option<LogicalRequestId>,
    state: PlaybackState,
) {
    let Some(after) = after.filter(|after| Some(*after) != before) else {
        drop_plays(waiting, "the item could not be played", state);
        return;
    };
    for entry in waiting.iter_mut() {
        if let What::Play { request, .. } = &mut entry.what {
            *request = Some(after);
        }
    }
}

/// An API shot waits in the same queue that owns its deadline.
pub(super) fn shot_pending(waiting: &VecDeque<InFlight>) -> bool {
    waiting.iter().any(|entry| matches!(entry.what, What::Shot))
}

pub(super) fn shot_completed(
    waiting: &mut VecDeque<InFlight>,
    result: Result<String, String>,
    state: PlaybackState,
) {
    let outcome = match result {
        Ok(path) => socket::Outcome::ShotCaptured { path },
        Err(reason) => socket::Outcome::ShotFailed { reason },
    };
    if let Some(index) = waiting
        .iter()
        .position(|entry| matches!(entry.what, What::Shot))
    {
        let request = waiting
            .remove(index)
            .expect("position came from the same queue");
        request
            .reply
            .send(socket::Reply::ok(outcome).with_state(state_str(state)));
    }
}

/// The per-frame half of the request→outcome correlation: [`publish`] calls this once
/// per frame — the one place that reads the state, the position, the auth flow and the
/// title together, which is exactly the data every parked request's answer needs. An
/// entry resolves when the frame it waited for has arrived; the response goes over the
/// socket under the entry's id.
unsafe fn resolve_awaiting(
    app: &mut crate::app::App,
    st: plx_media::player::PlaybackState,
    now_ms: u32,
) {
    let admission = app.bridge.take_api_profile_admission();
    let guard = &mut app.api_waiting;
    if guard.is_empty() {
        return;
    }
    let api_wait = app
        .menu_play_await
        .as_ref()
        .is_some_and(super::input::MenuPlayAwait::is_api);
    let bridge = app.runtime.bridge(&mut app.bridge);
    let views = bridge.views();
    let ps = &app.player.session;
    let auth = views.auth.0;
    let state_name = state_str(st);
    let pos_ms = plx_media::player::playpos_ns(ps).unwrap_or(0) / 1_000_000;
    let mut keep = VecDeque::new();
    while let Some(mut f) = guard.pop_front() {
        let r = match &mut f.what {
            What::Play { rk, request } => {
                use plx_media::player::PlaybackState;
                let current = ps.metric_request();
                let failed = request.is_some() && current.is_none() && st == PlaybackState::Error;
                if !failed && request.map_or(!api_wait, |request| Some(request) != current) {
                    Some(socket::Reply::ok(socket::Outcome::PlayFailed {
                        rk: rk.clone(),
                        reason: "superseded by another playback route".to_string(),
                    }))
                } else if request.is_some() && st == PlaybackState::Playing {
                    let title = views
                        .metadata
                        .now_playing()
                        .map(|n| n.title.clone())
                        .unwrap_or_default();
                    Some(socket::Reply::ok(socket::Outcome::PlayStarted {
                        rk: rk.clone(),
                        title,
                    }))
                } else if request.is_some()
                    && matches!(st, PlaybackState::Error | PlaybackState::Idle)
                {
                    let verdict = plx_media::route::play_verdict(ps).map(|s| s.to_string());
                    Some(socket::Reply::ok(socket::Outcome::PlayFailed {
                        rk: rk.clone(),
                        reason: format!(
                            "the engine errored (state {state_name}, verdict {})",
                            verdict.unwrap_or_else(|| "none".to_string())
                        ),
                    }))
                } else if now_ms.wrapping_sub(f.since_ms) >= AWAIT_PLAY_SECS * 1_000 {
                    Some(socket::Reply::ok(socket::Outcome::PlayFailed {
                        rk: rk.clone(),
                        reason: format!(
                            "no first frame within {AWAIT_PLAY_SECS} s (state {state_name})"
                        ),
                    }))
                } else {
                    None
                }
            }
            What::Seek {
                ms,
                generation,
                operation,
            } => {
                let outcome = if !is_seated(st)
                    || *generation != plx_media::route::playback_trace_generation()
                {
                    Some(socket::Outcome::SeekFailed {
                        ms: *ms,
                        reason: format!("the session left playback (state {state_name})"),
                    })
                } else if let Some(landed) =
                    operation.settled(ps, app.adapters.player.backend_snapshot())
                {
                    if landed {
                        Some(socket::Outcome::SeekLanded { ms: *ms, pos_ms })
                    } else {
                        Some(socket::Outcome::SeekFailed {
                            ms: *ms,
                            reason: "the seek was retired before landing".into(),
                        })
                    }
                } else if now_ms.wrapping_sub(f.since_ms) >= AWAIT_SEEK_SECS * 1_000 {
                    Some(socket::Outcome::SeekFailed { ms: *ms, reason: format!("no landing within {AWAIT_SEEK_SECS} s (state {state_name}, pos {pos_ms} ms)") })
                } else {
                    None
                };
                outcome.map(socket::Reply::ok)
            }
            What::Transport { play, generation } => {
                use plx_media::player::PlaybackState;
                if *generation != plx_media::route::playback_trace_generation() || !is_seated(st) {
                    Some(socket::Reply::err(socket::ApiError::NoEngine {
                        state: state_name,
                    }))
                } else if *play && st == PlaybackState::Playing {
                    Some(socket::Reply::ok(socket::Outcome::Resumed))
                } else if !*play && st == PlaybackState::Paused {
                    Some(socket::Reply::ok(socket::Outcome::Paused))
                } else if now_ms.wrapping_sub(f.since_ms) >= AWAIT_TRANSPORT_SECS * 1_000 {
                    Some(socket::Reply::err(socket::ApiError::Internal {
                        reason: format!("transport did not settle (state {state_name})"),
                    }))
                } else {
                    None
                }
            }
            What::Profile {
                index,
                uuid,
                correlation,
                epoch,
                seen_switching,
            } => {
                if let Some(accepted_epoch) = profile_epoch(*correlation, admission) {
                    if accepted_epoch.is_none() {
                        f.reply.send(
                            socket::Reply::ok(socket::Outcome::ProfileFailed {
                                index: *index,
                                reason: "the session refused the selection".into(),
                            })
                            .with_state(state_name),
                        );
                        continue;
                    }
                    *epoch = accepted_epoch;
                }
                if now_ms.wrapping_sub(f.since_ms) >= AWAIT_PROFILE_SECS * 1_000 {
                    Some(socket::Reply::ok(socket::Outcome::ProfileFailed {
                        index: *index,
                        reason: format!("the switch did not land within {AWAIT_PROFILE_SECS} s"),
                    }))
                } else if let Some(epoch) = *epoch {
                    *seen_switching |= auth.flow_epoch == epoch
                        && auth.phase == plx_session::auth::Phase::Switching;
                    if auth.flow_epoch > epoch {
                        Some(socket::Reply::ok(socket::Outcome::ProfileFailed {
                            index: *index,
                            reason: "superseded by another session command".into(),
                        }))
                    } else if auth.flow_epoch == epoch
                        && auth.phase == plx_session::auth::Phase::Ready
                    {
                        let outcome = match auth.profile.as_ref().filter(|p| p.uuid == *uuid) {
                            Some(profile) => socket::Outcome::ProfileSeated {
                                index: *index,
                                title: profile.title.clone(),
                            },
                            None => socket::Outcome::ProfileFailed {
                                index: *index,
                                reason: "the selected profile did not seat".into(),
                            },
                        };
                        Some(socket::Reply::ok(outcome))
                    } else if auth.flow_epoch == epoch
                        && auth.phase == plx_session::auth::Phase::Error
                    {
                        Some(socket::Reply::ok(socket::Outcome::ProfileFailed {
                            index: *index,
                            reason: if auth.error.is_empty() {
                                "the flow reported an error".to_string()
                            } else {
                                auth.error.to_string()
                            },
                        }))
                    } else if *seen_switching && auth.phase == plx_session::auth::Phase::Profiles {
                        // The switch began, then went back to the roster without seating (a
                        // denied PIN, a cancel): this pick is dead.
                        Some(socket::Reply::ok(socket::Outcome::ProfileFailed {
                            index: *index,
                            reason: "the flow returned to the roster without seating".to_string(),
                        }))
                    } else if auth.flow_epoch == epoch
                        && !matches!(
                            auth.phase,
                            plx_session::auth::Phase::Profiles
                                | plx_session::auth::Phase::Switching
                        )
                    {
                        // Any other phase (idle, a re-sign-in in flight, erased local state)
                        // is the flow leaving the switch without this pick.
                        Some(socket::Reply::ok(socket::Outcome::ProfileFailed {
                            index: *index,
                            reason: format!("the flow left the switch (phase {:?})", auth.phase)
                                .to_lowercase(),
                        }))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            What::Shot => {
                if now_ms.wrapping_sub(f.since_ms) >= AWAIT_SHOT_SECS * 1_000 {
                    Some(socket::Reply::ok(socket::Outcome::ShotFailed {
                        reason: format!("no capture within {AWAIT_SHOT_SECS} s"),
                    }))
                } else {
                    None
                }
            }
        };
        if let Some(r) = r {
            // Every outcome this frame judges is stamped with THIS frame's playback state —
            // the state the result belongs to.
            f.reply.send(r.with_state(state_name));
        } else {
            keep.push_back(f);
        }
    }
    *guard = keep;
}
fn profile_epoch(correlation: u32, admission: Option<(u32, Option<u64>)>) -> Option<Option<u64>> {
    admission
        .filter(|(reply_to, _)| *reply_to == correlation)
        .map(|(_, epoch)| epoch)
}

static UP_SINCE: OnceLock<u32> = OnceLock::new();

pub(super) fn init() {
    let _ = UP_SINCE.get_or_init(super::clock::now);
    socket::start();
}

pub(super) fn take_pending() -> Vec<socket::Request> {
    PENDING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .drain(..)
        .collect()
}

fn push_req(req: socket::Request) -> bool {
    let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    if pending.len() >= 64 {
        return false;
    }
    pending.push_back(req);
    true
}

pub(super) fn reject_during_replay(req: socket::Request) {
    req.reply
        .send(socket::Reply::err(socket::ApiError::Internal {
            reason: "the app is replaying recorded input".to_string(),
        }));
}

static STATE_FILE_EPOCH: AtomicU32 = AtomicU32::new(0);

pub(super) fn retire_state_file() {
    STATE_FILE_EPOCH.fetch_add(1, Ordering::SeqCst);
}

fn state_file(app: &mut super::App, fr: &super::run::Frame) {
    let json = snapshot(app, fr).to_string();
    let epoch = STATE_FILE_EPOCH.load(Ordering::SeqCst);
    let path = plx_base::paths::in_runtime_dir(plx_base::paths::runtime_file::CONTROL_STATE);
    if plx_base::storage_worker::submit(move || {
        if STATE_FILE_EPOCH.load(Ordering::SeqCst) != epoch {
            return;
        }
        if let Err(error) = plx_plex::plex::session::write_atomic(&path, json.as_bytes()) {
            log(&format!(
                "api: state: could not write {}: {error:?}",
                path.display()
            ));
        } else if STATE_FILE_EPOCH.load(Ordering::SeqCst) != epoch {
            // Erasure raced the rename. The shared worker serializes dump writers.
            if let Err(error) = super::input::remove_or_prove_absent(&path) {
                log(&format!(
                    "api: state: could not finish erasing {}: {error}",
                    path.display()
                ));
            }
        }
    })
    .is_err()
    {
        log("api: state: storage worker refused the write");
    }
}

pub(crate) unsafe fn handle(
    app: &mut crate::app::App,
    fr: &mut crate::app::run::Frame,
    spec: &str,
) -> bool {
    let (kind, arg) = match spec.split_once(':') {
        Some(pair) => pair,
        None => (spec, ""),
    };
    match kind {
        "play" => handle_play(app, fr, arg),
        "state" => {
            state_file(app, fr);
            true
        }
        "pause" => handle_transport(app, fr, false),
        "resume" => handle_transport(app, fr, true),
        "stop" => handle_stop(app),
        "seek" => match arg.parse::<u64>().ok().and_then(seek_ns) {
            Some(ns) => app
                .adapters
                .player
                .request_seek(&mut app.player.session, ns),
            _ => false,
        },
        "audio" | "sub" => arg
            .parse()
            .is_ok_and(|sid| handle_track(app, sid, kind == "audio").is_ok()),
        "quality" => handle_quality(app, arg).is_ok(),
        "profile" => handle_profile(app, fr, arg),
        _ => false,
    }
}

/// The frame thread's half of a TYPED UDS request: the same handlers the FIFO's tokens
/// reach (the typed surface is a CONVERSATION over the command set, not a second command
/// set), and the response over the socket instead of a log line — the outcome in the
/// action's own vocabulary, or a structured error naming the reason.
pub(crate) unsafe fn execute_request(
    app: &mut crate::app::App,
    fr: &mut crate::app::run::Frame,
    req: socket::Request,
) {
    use socket::{ApiError, Outcome, Reply, Verb};
    let reply = req.reply;
    let st = plx_media::player::state(&app.player.session);
    let st_name = state_str(st);
    let no_engine = Reply::err(ApiError::NoEngine { state: st_name });
    // Every reply this frame produces is stamped with THIS frame's playback state — the
    // state the result belongs to (a reply is a judgment, not an acknowledgement).
    let answer = |app: &super::App, r: Reply| {
        reply.send(r.with_state(state_str(plx_media::player::state(&app.player.session))));
    };
    if matches!(
        req.verb,
        Verb::Play { .. }
            | Verb::Seek { .. }
            | Verb::Profile { .. }
            | Verb::Pause
            | Verb::Resume
            | Verb::Shot
    ) && !can_park(&app.api_waiting, &req.verb)
    {
        answer(app, Reply::err(ApiError::QueueFull));
        return;
    }
    match req.verb {
        Verb::Play { rk } => {
            // Commit, then park: the response is the OUTCOME — the first playing frame,
            // or the refusal that lands in between (a refusal the frame can already see
            // answers now, the ones only time reveals ride the per-frame scan) — never
            // this acceptance.
            let accepted = handle_play(app, fr, &rk);
            if accepted {
                arm_awaiting(
                    &mut app.api_waiting,
                    reply.clone(),
                    fr.now,
                    What::Play { rk, request: None },
                );
            } else {
                answer(
                    app,
                    Reply::ok(Outcome::PlayFailed {
                        rk,
                        reason: "no server or item selected".to_string(),
                    }),
                );
            }
        }
        Verb::Seek { ms } => {
            // The seek target is a `i64` of NANOSECONDS: a ms beyond ~292 years is not a
            // seek (and a blind `as i64` would wrap negative — the FIFO's own guard
            // rejects negatives for exactly this reason).
            let ns = seek_ns(ms);
            if is_seated(st) {
                if let Some(ns) = ns {
                    if app
                        .adapters
                        .player
                        .request_seek(&mut app.player.session, ns)
                    {
                        if let Some(operation) =
                            SeekOperation::submitted(&app.player.session, &app.adapters.player)
                        {
                            arm_awaiting(
                                &mut app.api_waiting,
                                reply.clone(),
                                fr.now,
                                What::Seek {
                                    ms,
                                    generation: plx_media::route::playback_trace_generation(),
                                    operation,
                                },
                            );
                            return;
                        }
                    }
                    answer(
                        app,
                        Reply::ok(Outcome::SeekFailed {
                            ms,
                            reason: "seek was refused or could not be tracked".into(),
                        }),
                    );
                } else {
                    answer(
                        app,
                        Reply::err(ApiError::Internal {
                            reason: "the seek target overflows nanoseconds".to_string(),
                        }),
                    );
                }
            } else {
                answer(app, no_engine);
            }
        }
        Verb::Audio { sid } | Verb::Sub { sid } => {
            let audio = matches!(req.verb, Verb::Audio { .. });
            let result = handle_track(app, sid, audio);
            answer(
                app,
                match result {
                    Ok(()) => Reply::ok(Outcome::TrackCommitted {
                        track: if audio { "audio" } else { "sub" },
                        sid,
                    }),
                    Err(error) => Reply::err(error),
                },
            );
        }
        Verb::Quality { rung } => {
            let result = handle_quality(app, &rung);
            answer(
                app,
                match result {
                    Ok(()) => Reply::ok(Outcome::QualityCommitted { rung }),
                    Err(error) => Reply::err(error),
                },
            );
        }
        Verb::Profile { index } => {
            // Structured pre-checks the FIFO's token cannot express: the session machine
            // silently drops a pick made outside the roster phase — say so — and the
            // roster must have that index.
            let (auth_phase, user_count, uuid) = {
                let bridge = app.runtime.bridge(&mut app.bridge);
                let auth = bridge.views().auth.0;
                (
                    auth.phase,
                    auth.users.len(),
                    auth.users.get(index).map(|user| user.uuid.clone()),
                )
            };
            if !matches!(
                auth_phase,
                plx_session::auth::Phase::Profiles | plx_session::auth::Phase::Switching
            ) {
                answer(
                    app,
                    Reply::err(ApiError::NotOnRoster {
                        phase: format!("{auth_phase:?}").to_lowercase(),
                    }),
                );
            } else if let Some(uuid) = uuid {
                let target = app.bridge.api_profile_reply_to();
                super::bridge::execute_session_command(
                    &mut app.pages,
                    plx_session::auth::SessionCmd::SelectProfileWithReply {
                        index,
                        pin: None,
                        reply: target,
                    },
                );
                arm_awaiting(
                    &mut app.api_waiting,
                    reply.clone(),
                    fr.now,
                    What::Profile {
                        index,
                        uuid,
                        correlation: target.correlation,
                        epoch: None,
                        seen_switching: false,
                    },
                );
            } else {
                answer(
                    app,
                    Reply::err(ApiError::NoProfile {
                        index,
                        count: user_count,
                    }),
                );
            }
        }

        Verb::Pause | Verb::Resume => {
            let play = matches!(req.verb, Verb::Resume);
            if handle_transport(app, fr, play) {
                arm_awaiting(
                    &mut app.api_waiting,
                    reply.clone(),
                    fr.now,
                    What::Transport {
                        play,
                        generation: plx_media::route::playback_trace_generation(),
                    },
                );
            } else {
                answer(app, no_engine);
            }
        }
        Verb::Stop => {
            // Idempotent: an idle session has nothing to leave, and "stopped" is still
            // the true answer to "is it playing".
            handle_stop(app);
            answer(app, Reply::ok(Outcome::Stopped));
        }
        Verb::State => {
            let snapshot = snapshot(app, fr);
            answer(app, Reply::ok(Outcome::State { snapshot }));
        }
        Verb::Shot => {
            // Parked: the response is the CAPTURE (the numbered PNG, written on the
            // frame swap after this one) or the capture's refusal — not this request.
            app.pages
                .present
                .note(plx_machine::present::PresentEvent::Wake);
            arm_awaiting(&mut app.api_waiting, reply.clone(), fr.now, What::Shot);
        }
    }
}

/// The playback state's snapshot vocabulary: the same strings the state file's `state`
/// field and the structured `no_engine` error answer share.
fn state_str(st: plx_media::player::PlaybackState) -> &'static str {
    match st {
        plx_media::player::PlaybackState::Idle => "idle",
        plx_media::player::PlaybackState::Resolving => "resolving",
        plx_media::player::PlaybackState::Connecting => "connecting",
        plx_media::player::PlaybackState::Buffering => "buffering",
        plx_media::player::PlaybackState::Seeking => "seeking",
        plx_media::player::PlaybackState::Playing => "playing",
        plx_media::player::PlaybackState::Paused => "paused",
        plx_media::player::PlaybackState::Error => "error",
    }
}

/// The seated-engine question the transport verbs (and the typed seek) ask before they
/// touch the engine: `sink().pause` and `request_seek` are Starfish calls, and a session
/// without an engine (idle, resolving, connecting, errored) has nothing for them to
/// reach.
fn is_seated(st: plx_media::player::PlaybackState) -> bool {
    matches!(
        st,
        plx_media::player::PlaybackState::Buffering
            | plx_media::player::PlaybackState::Paused
            | plx_media::player::PlaybackState::Seeking
            | plx_media::player::PlaybackState::Playing
    )
}

fn seek_ns(ms: u64) -> Option<i64> {
    i64::try_from(ms).ok()?.checked_mul(1_000_000)
}

unsafe fn handle_play(app: &mut super::App, fr: &super::run::Frame, rk: &str) -> bool {
    let sid = plx_media::route::surface_sid();
    if rk.is_empty() || plx_plex::plex::client_opt().is_none() {
        return false;
    }
    handle_stop(app);
    let mut bridge = app.runtime.bridge(&mut app.bridge);
    bridge.metadata_run(plx_data::stores::metadata::MetadataCmd::RequestDetail {
        sid,
        rk: rk.to_owned(),
    });
    app.menu_play_await = Some(super::input::MenuPlayAwait::for_api(
        sid,
        rk.to_owned(),
        fr.now,
    ));
    true
}

fn handle_transport(app: &mut super::App, fr: &super::run::Frame, play: bool) -> bool {
    if !is_seated(plx_media::player::state(&app.player.session)) {
        return false;
    }
    super::playback::apply_transport(
        &mut app.player.session,
        &mut app.adapters.player,
        Some(play),
        fr.now,
        &mut app.pages,
    )
}

fn handle_stop(app: &mut super::App) -> bool {
    app.menu_play_await = None;
    if plx_media::player::state(&app.player.session) != plx_media::player::PlaybackState::Idle
        || super::bridge::player(&app.pages).is_some()
    {
        super::playback::exit_player(
            &mut app.player,
            &mut app.adapters.player,
            &mut app.repause_at,
            &mut app.refresh_hubs,
            &mut app.pages,
        );
    }
    drop_plays(
        &mut app.api_waiting,
        "superseded by a stop or a newer play",
        plx_media::player::state(&app.player.session),
    );
    true
}

/// `api:audio:<sid>` / `api:sub:<sid>` — find the `playing()` item's track row by PMS
/// stream id and commit it through the media layer's own selection functions (the bare
/// `_selection` commits the ladder uses: no viewer memo is remembered, which is right
/// for an agent).
fn handle_track(app: &mut super::App, sid: i64, audio: bool) -> Result<(), socket::ApiError> {
    let bridge = app.runtime.bridge(&mut app.bridge);
    let views = bridge.views();
    let Some(item) = views.metadata.playing() else {
        return Err(socket::ApiError::NoTrack { sid });
    };
    use plx_appkit::track_menu::TrackCommit;
    let commit = if audio {
        let Some(i) = item.audio.iter().position(|stream| stream.id == sid) else {
            return Err(socket::ApiError::NoTrack { sid });
        };
        let ordinal = plx_data::metadata::audio_ordinal(&item.audio, i);
        TrackCommit::Audio {
            carried: plx_media::route::CarriedAudio::from_stream(&item.audio[i], ordinal),
            memo: None,
        }
    } else if sid == 0 {
        TrackCommit::Subtitle {
            render_ordinal: -1,
            stream_id: 0,
            sidecar_key: None,
            sidecar_codec: String::new(),
            memo: None,
        }
    } else {
        let Some(i) = item.subs.iter().position(|stream| stream.id == sid) else {
            return Err(socket::ApiError::NoTrack { sid });
        };
        let stream = &item.subs[i];
        TrackCommit::Subtitle {
            render_ordinal: plx_data::metadata::sub_render_ordinal(&item.subs, i),
            stream_id: sid,
            sidecar_key: stream.sidecar_renderable().then(|| stream.key.clone()),
            sidecar_codec: stream.codec.clone(),
            memo: None,
        }
    };
    let mut bridge = bridge;
    super::playback::commit_track(
        &mut app.player.session,
        &mut app.adapters.player,
        bridge.metadata_mut(),
        commit,
    )
    .then_some(())
    .ok_or_else(|| socket::ApiError::Internal {
        reason: "the player refused the track change".into(),
    })
}

fn handle_quality(app: &mut super::App, arg: &str) -> Result<(), socket::ApiError> {
    let quality =
        serde_json::from_value(serde_json::Value::String(arg.to_owned())).map_err(|_| {
            socket::ApiError::UnknownRung {
                rung: arg.to_owned(),
            }
        })?;
    let mut bridge = app.runtime.bridge(&mut app.bridge);
    super::playback::apply_quality(
        &mut app.player.session,
        &mut app.adapters.player,
        &mut bridge,
        quality,
        &mut app.pages.present,
    )
    .then_some(())
    .ok_or_else(|| socket::ApiError::Internal {
        reason: "the player refused the quality change".into(),
    })
}

/// `api:profile:<index>[:<pin>|-]` — commit a roster pick through the session machine\u2019s own
/// command (the same commit the roster\u2019s rows land in). The pick moves the flow to Switching;
/// its outcome is visible in the state file's `auth` section (phase, error).
unsafe fn handle_profile(
    app: &mut crate::app::App,
    _fr: &mut crate::app::run::Frame,
    arg: &str,
) -> bool {
    let Some((idx, pin)) = arg.split_once(':') else {
        return false;
    };
    let Ok(index) = idx.parse::<usize>() else {
        return false;
    };
    let pin = (pin != "-").then(|| pin.to_string());
    commit_profile(app, index, pin);
    true
}

/// The profile commit the FIFO's `api:profile` token and the typed `profile` request both
/// reach: the session machine's own command (the same commit the roster's rows land in).
/// The pick moves the flow to Switching; its outcome is visible in the state file's
/// `auth` section (phase, error).
unsafe fn commit_profile(app: &mut crate::app::App, index: usize, pin: Option<String>) {
    log(&format!(
        "api: profile: select index {index} (pin {})",
        if pin.is_some() { "given" } else { "none" }
    ));
    crate::app::bridge::execute_session_command(
        &mut app.pages,
        plx_session::auth::SessionCmd::SelectProfile { index, pin },
    );
}

pub(super) unsafe fn publish(app: &mut super::App, now_ms: u32) {
    let st = plx_media::player::state(&app.player.session);
    resolve_awaiting(app, st, now_ms);
}

fn track_ordinal(tracks: &[plx_data::metadata::Stream], index: usize, audio: bool) -> i32 {
    if audio {
        plx_data::metadata::audio_ordinal(tracks, index)
    } else {
        plx_data::metadata::sub_render_ordinal(tracks, index)
    }
}

fn snapshot(app: &mut super::App, fr: &super::run::Frame) -> serde_json::Value {
    let st = plx_media::player::state(&app.player.session);
    let state_name = state_str(st);
    let bridge = app.runtime.bridge(&mut app.bridge);
    let views = bridge.views();
    let ps = &app.player.session;
    // This frame's auth section: the sign-in flow's phase, the offered "who's watching"
    // roster (title, PIN flag, admin flag) and the flow's error line — the state file's
    // `auth` section.
    let auth = {
        let snap = views.auth.0;
        json!({
            "phase": format!("{:?}", snap.phase).to_lowercase(),
            "error": if snap.error.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(snap.error.to_string())
            },
            "profiles": snap.users.iter().enumerate().map(|(i, u)| {
                json!({
                    "index": i,
                    "title": u.title,
                    "pin": u.protected,
                    "admin": u.admin,
                })
            }).collect::<Vec<_>>(),
        })
    };
    let now = views.metadata.now_playing();
    let playing = views.metadata.playing();
    let tracks = |item: Option<&plx_data::metadata::PlayingItem>,
                  audio: bool|
     -> Vec<serde_json::Value> {
        let Some(item) = item else {
            return Vec::new();
        };
        let list: &Vec<plx_data::metadata::Stream> = if audio { &item.audio } else { &item.subs };
        list.iter()
            .enumerate()
            .map(|(i, s)| {
                let mut o = json!({
                    "sid": s.id,
                    "ord": track_ordinal(list, i, audio),
                    "codec": s.codec,
                    "lang": s.lang,
                    "title": s.title,
                    "selected": s.selected,
                });
                if audio {
                    o["ch"] = json!(s.channels);
                    o["layout"] = json!(s.layout);
                    o["kbits"] = json!(s.bitrate);
                    o["default"] = json!(s.default);
                } else {
                    o["external"] = json!(s.external);
                    o["forced"] = json!(s.forced);
                }
                o
            })
            .collect()
    };
    let verdict = plx_media::route::play_verdict(ps).map(|s| s.to_string());
    let mut url = plx_media::route::url(ps);
    if let Some(query) = url.find('?') {
        url.truncate(query);
    }
    let mut obj = json!({
        "t": fr.now,
        "up_ms": UP_SINCE.get().map_or(0, |start| fr.now.wrapping_sub(*start)),
        "state": state_name,
        "state_id": st as u8,
        "playing": plx_media::player::is_playing(ps),
        // The viewer's pause — `true` while the transport is paused for the viewer (a claim
        // hold's own pause is NOT the viewer's and stays out of this lens).
        "paused": plx_media::player::transport::viewer_paused_for(ps, &app.adapters.player),
        "pos_ms": plx_media::player::playpos_ns(ps).unwrap_or(0) / 1_000_000,
        "dur_ms": plx_media::player::duration_ns(ps).unwrap_or(0) / 1_000_000,
        "title": now.map(|n| n.title.clone()).unwrap_or_default(),
        "ep_title": now.map(|n| n.ep_title.clone()).unwrap_or_default(),
        "rk": playing.map(|p| p.rk.clone()).unwrap_or_default(),
        "kind": playing.map(|p| p.kind.clone()).unwrap_or_default(),
        "url": url,
        "transcode": plx_media::route::transcode_session(ps),
        "kbps": plx_media::route::transport_kbps(ps),
        "audio_sid": plx_media::route::cur_audio_sid(ps),
        // The session's live subtitle pick (0 = Off), the read-out an agent verifies a `sub`
        // commit against: `subs[].selected` mirrors the metadata the plan set, which a live
        // commit does not rewrite.
        "sub_sid": plx_media::route::cur_sub_sid(ps),
        "verdict": verdict,
        "audio": tracks(playing, true),
        "subs": tracks(playing, false),
    });
    obj["auth"] = auth;
    obj
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_later_nearby_feed_seek_cannot_complete_the_first() {
        use plx_media::player::readout::ReadoutEvent;
        let mut player = plx_media::route::PlaybackSession::default();
        let uri = plx_playback_contract::Snapshot::default();
        player.step_readout(ReadoutEvent::SeekRequested {
            target_ns: 60_000_000_000,
            display_ns: 60_000_000_000,
        });
        let first = SeekOperation::Feed(player.readout().seek_epoch());
        assert_eq!(first.settled(&player, &uri), None);
        player.step_readout(ReadoutEvent::SeekRequested {
            target_ns: 61_000_000_000,
            display_ns: 61_000_000_000,
        });
        assert_eq!(first.settled(&player, &uri), Some(false));
        let second = SeekOperation::Feed(player.readout().seek_epoch());
        assert_eq!(second.settled(&player, &uri), None);
        player.step_readout(ReadoutEvent::SeekAbandoned);
        assert_eq!(second.settled(&player, &uri), Some(false));
    }

    #[test]
    fn profile_admission_uses_the_owners_epoch_and_matches_correlation() {
        assert_eq!(profile_epoch(4, Some((3, Some(10)))), None);
        assert_eq!(profile_epoch(4, Some((4, Some(12)))), Some(Some(12)));
        assert_eq!(profile_epoch(4, Some((4, None))), Some(None));
    }

    #[test]
    fn capture_and_supersession_reply_with_the_state_at_completion() {
        let mut waiting = VecDeque::new();
        let (play, play_response) = socket::ReplyTo::test_channel(11);
        let (shot, shot_response) = socket::ReplyTo::test_channel(12);
        arm_awaiting(
            &mut waiting,
            play,
            0,
            What::Play {
                rk: "movie".into(),
                request: None,
            },
        );
        arm_awaiting(&mut waiting, shot, 0, What::Shot);

        assert!(!can_park(&waiting, &socket::Verb::Shot));
        assert!(shot_pending(&waiting));
        drop_plays(&mut waiting, "stopped", PlaybackState::Idle);
        let (id, reply) = play_response.try_recv().unwrap();
        assert_eq!(id, Some(11));
        assert_eq!(reply.state, Some("idle"));
        assert!(matches!(
            reply.outcome,
            Some(socket::Outcome::PlayFailed { .. })
        ));

        shot_completed(&mut waiting, Ok("shot.png".into()), PlaybackState::Playing);
        let (id, reply) = shot_response.try_recv().unwrap();
        assert_eq!(id, Some(12));
        assert_eq!(reply.state, Some("playing"));
        assert!(matches!(
            reply.outcome,
            Some(socket::Outcome::ShotCaptured { .. })
        ));
        assert!(!shot_pending(&waiting));
    }

    #[test]
    fn snapshot_subtitle_ordinals_exclude_external_streams() {
        let tracks = [
            plx_data::metadata::Stream {
                index: 0,
                external: true,
                ..Default::default()
            },
            plx_data::metadata::Stream {
                index: 1,
                ..Default::default()
            },
        ];
        assert_eq!(track_ordinal(&tracks, 0, false), -1);
        assert_eq!(track_ordinal(&tracks, 1, false), 0);
        assert_eq!(track_ordinal(&tracks, 1, true), 1);
    }

    #[test]
    fn seek_target_must_fit_signed_nanoseconds() {
        assert_eq!(seek_ns(123), Some(123_000_000));
        assert_eq!(seek_ns(u64::MAX), None);
        assert_eq!(seek_ns(i64::MAX as u64), None);
    }
}
