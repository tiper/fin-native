//! The UDS control socket — the typed API's transport. A UNIX domain socket
//! (`SOCK_STREAM`, JSON-lines) at `plxnative-api.sock` in the runtime directory, beside
//! the dev FIFO (`plxnative-remote`) and its state dump (`state.json`).
//!
//! The FIFO is a one-way byte pipe: a token in, a log line as the only answer, and the
//! state file as the only read-out. A typed control API needs a CONVERSATION: every
//! request carries a client-chosen request id, and exactly one response carries it back —
//! a structured outcome (what actually happened) or a structured error (a
//! machine-readable code and the human-readable reason).
//!
//! The response is produced when the OUTCOME is known, not when the request is queued.
//! The synchronous verbs answer their commit on the spot — the same commit path the
//! app's own actions use (`no_engine` with the state instead of a bare "no", the
//! committed track's `sid`, a refused commit as a structured error). The asynchronous
//! ones (play, seek, pause, resume, profile, shot) park in `api`'s AWAITING table and answer on the
//! frame their outcome lands: `play_started` with the title, `seek_landed` with the
//! position, `profile_seated` with the seated profile's title, `shot_captured` with the
//! numbered file — or the structured failure that explains why.
//!
//! The protocol is one JSON object per line (UTF-8, `\n`-terminated):
//!
//! ```text
//! client → {"id": 1, "req": {"kind": "state"}}
//! app    → {"id": 1, "ok": true, "outcome": {"kind": "state", "snapshot": {"state": "playing", "pos_ms": 354000, "title": "…", …}}}
//! client → {"id": 3, "req": {"kind": "play", "rk": "1$abc"}}
//! app    → {"id": 3, "ok": true, "outcome": {"kind": "play_started", "rk": "1$abc", "title": "…"}}   (on the first playing frame)
//! app    → {"id": 4, "ok": false, "error": {"code": "no_engine", "state": "idle"}}
//! app    → {"id": null, "ok": false, "error": {"code": "malformed", "reason": "..."}}
//! ```
//!
//! The `req` vocabulary is the typed form of the FIFO's `api:*` verbs — the same actions
//! and the same argument shapes (play, seek, audio, sub, quality, profile, pause,
//! resume, stop), plus the two reads (`state`, `shot`). Execution happens on the frame
//! thread through the same single input path as the FIFO
//! (the app control queue); the response is produced there — where the outcome is
//! known — and written back by this module's server thread over a bounded reply queue.
//!
//! One client is served at a time. Each request replies only to its own connection.
//! Keep IDs unique while requests are outstanding. Disconnecting discards replies;
//! it does not roll back commands already sent to the app.
//!
//! The frame thread builds the state snapshot on demand, using the same data as the
//! FIFO state dump. The socket is owner-only and requires the plxnative-api boot
//! trigger in a devtriggers build.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc;
use std::time::Duration;

use plx_base::eventlog::log;

/// A request on the typed protocol: the client's id + the verb.
pub struct Request {
    /// The client's own correlation id; echoed unchanged on the response.
    pub(super) reply: ReplyTo,
    pub verb: Verb,
}

/// The typed request vocabulary — the FIFO's `api:*` verbs with their argument shapes.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Verb {
    /// `api:play:<rk>` — the headless play ritual (resolve from PMS, seat the engine).
    Play { rk: String },
    /// `api:seek:<ms>` — an absolute engine seek (milliseconds).
    Seek { ms: u64 },
    /// `api:audio:<sid>` — the track menu's audio commit for the stream with this id.
    Audio { sid: i64 },
    /// `api:sub:<sid>` — the track menu's subtitle commit for the stream with this id.
    Sub { sid: i64 },
    /// `api:quality:<rung>` — the `…` menu's quality commit for this rung.
    Quality { rung: String },
    /// `api:profile:<index>` — the session machine's profile commit.
    Profile { index: usize },
    /// `api:pause` — the PAUSE key.
    Pause,
    /// `api:resume` — the PLAY key.
    Resume,
    /// `api:stop` — the STOP key's leave-playback ritual.
    Stop,
    /// Read: the frame thread's snapshot (the value `state.json` holds).
    State,
    /// Read: a capture of the next frame (a numbered PNG beside the state file).
    Shot,
}

/// A structured outcome: what actually happened, in the action's own vocabulary — never
/// an acknowledgement. The synchronous verbs (audio, sub, quality, stop)
/// answer their COMMIT: the media layer accepts or refuses on the spot. The asynchronous
/// ones (play, seek, pause, resume, profile, shot) are parked in `api`'s AWAITING table when the
/// request's frame commits them, and answered on the frame their OUTCOME lands:
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    /// play: the play actually started — the first frame reached `playing`; `title` is
    /// what is playing now.
    PlayStarted {
        rk: String,
        title: String,
    },
    /// play: the play never started, with the reason from the layer that refused it —
    /// metadata loading (no PMS selected, no such item, no playable part), the routing
    /// question, the engine's verdict, a supersession (a stop or a newer play), or the
    /// 120 s deadline.
    PlayFailed {
        rk: String,
        reason: String,
    },
    /// The backend acknowledged this seek; pos_ms is its observed playback position.
    SeekLanded {
        ms: u64,
        pos_ms: i64,
    },
    /// seek: the seek never landed — the session left playback, or no landing in 10 s.
    SeekFailed {
        ms: u64,
        reason: String,
    },
    /// audio/sub: the track menu's commit was performed for the stream with this id
    /// (`track` is `audio` or `sub`; it is not named `kind` — that is the tag).
    TrackCommitted {
        track: &'static str,
        sid: i64,
    },
    /// quality: the `…` menu's commit was performed for this rung.
    QualityCommitted {
        rung: String,
    },
    /// profile: the switch LANDED — the session's auth flow reached `ready` after the
    /// pick; `title` is the seated profile's.
    ProfileSeated {
        index: usize,
        title: String,
    },
    /// profile: the switch failed — the flow's own error line, or the flow left the
    /// switch without this pick (a sign-out, the 60 s deadline), naming the phase.
    ProfileFailed {
        index: usize,
        reason: String,
    },
    Paused,
    Resumed,
    Stopped,
    /// state: the frame thread's snapshot — the same value `state.json` holds.
    State {
        snapshot: serde_json::Value,
    },
    /// shot: the capture landed; `path` is the numbered `shot-<n>.png` in the runtime
    /// directory, written on the frame swap that followed the request.
    ShotCaptured {
        path: String,
    },
    /// shot: the capture failed (empty viewport, encode/write error) — or no capture in
    /// 10 s.
    ShotFailed {
        reason: String,
    },
}

/// A structured error: a machine-readable code (the stable vocabulary a client may
/// branch on) plus the reason (human-readable; its wording may change).
#[derive(Debug, Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum ApiError {
    /// The request line was not a request (bad JSON, an unknown `kind`, a missing or
    /// mistyped argument). Echoes a valid envelope ID when present.
    Malformed { reason: String },
    /// The verb needs a seated engine; the session is in the named state instead.
    NoEngine { state: &'static str },
    /// No audio/subtitle track with that id on the playing item.
    NoTrack { sid: i64 },
    /// The quality rung is not one the routing vocabulary knows.
    UnknownRung { rung: String },
    /// The profile pick needs the roster screen; the session's auth flow is in the named
    /// phase instead (a pick outside the roster phase is one the session machine would
    /// silently drop — the typed surface says so).
    NotOnRoster { phase: String },
    /// The roster has no profile at that index.
    NoProfile { index: usize, count: usize },
    /// The frame loop's command queue was full when the request was posted.
    QueueFull,
    /// Internal (serialization or an invariant).
    Internal { reason: String },
}

/// A response frame the frame thread has produced but the server thread has not yet
/// written.
pub struct Reply {
    pub ok: bool,
    pub outcome: Option<Outcome>,
    pub error: Option<ApiError>,
    /// The playback state of the frame this reply was judged on — which state the result
    /// belongs to. The frame thread stamps it on every reply it produces ([`Reply::with_state`]);
    /// protocol answers that need no frame (a malformed line, a full queue, a drain) leave
    /// it unset, and the field then does not appear on the wire.
    pub state: Option<&'static str>,
}

impl Reply {
    pub fn ok(outcome: Outcome) -> Self {
        Reply {
            ok: true,
            outcome: Some(outcome),
            error: None,
            state: None,
        }
    }
    pub fn err(error: ApiError) -> Self {
        Reply {
            ok: false,
            outcome: None,
            error: Some(error),
            state: None,
        }
    }
    /// Stamp the frame's playback state onto the reply (the frame-thread producers).
    pub fn with_state(mut self, st: &'static str) -> Self {
        self.state = Some(st);
        self
    }
}

type Response = (Option<u64>, Reply);

#[derive(Clone)]
pub(super) struct ReplyTo {
    id: u64,
    sender: mpsc::SyncSender<Response>,
}

impl ReplyTo {
    #[cfg(test)]
    pub(super) fn test_channel(id: u64) -> (Self, mpsc::Receiver<Response>) {
        let (sender, receiver) = mpsc::sync_channel(1);
        (Self { id, sender }, receiver)
    }

    pub(super) fn send(&self, reply: Reply) {
        if matches!(
            self.sender.try_send((Some(self.id), reply)),
            Err(mpsc::TrySendError::Full(_))
        ) {
            log("api sock: client reply queue full");
        }
    }
}

/// Bind the socket and start the server when the developer has armed it at boot.
pub fn start() {
    if plx_base::devtrig::read("api").is_none() {
        return;
    }
    let path = plx_base::paths::in_runtime_dir("plxnative-api.sock");
    let listener = match bind_private(&path) {
        Ok(listener) => listener,
        Err(error) => {
            log(&format!(
                "api sock: bind {} failed: {error}",
                path.display()
            ));
            return;
        }
    };
    let name = path.display().to_string();
    let worker = plx_base::task::spawn("api-sock", move || {
        log(&format!("api sock: serving on {name}"));
        loop {
            match listener.accept() {
                Ok((conn, _)) => serve(conn, super::push_req),
                Err(e) => {
                    log(&format!("api sock: accept failed: {e}"));
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    });
    if worker.is_none() {
        let _ = std::fs::remove_file(path);
        log("api sock: the server thread was refused");
    }
}

/// Bind out of reach until the socket is owner-only, then publish its name.
fn bind_private(path: &std::path::Path) -> std::io::Result<UnixListener> {
    use std::io::ErrorKind;
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_socket() => match UnixStream::connect(path) {
            Err(error) if error.kind() == ErrorKind::ConnectionRefused => {}
            Ok(_) => return Err(ErrorKind::AddrInUse.into()),
            Err(error) => return Err(error),
        },
        Ok(_) => return Err(ErrorKind::AlreadyExists.into()),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let dir = path.with_file_name(format!(".api-{}", std::process::id()));
    std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
    let staging = dir.join("s");
    let listener = (|| {
        let listener = UnixListener::bind(&staging)?;
        std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o600))?;
        std::fs::rename(&staging, path)?;
        Ok(listener)
    })();
    let _ = std::fs::remove_file(&staging);
    let _ = std::fs::remove_dir(&dir);
    listener
}

// A peer can close its writing half and still be able to receive replies.
fn peer_disconnected(conn: &UnixStream) -> bool {
    let mut fd = libc::pollfd {
        fd: conn.as_raw_fd(),
        events: 0,
        revents: 0,
    };
    // SAFETY: fd is live through conn, and poll reads/writes exactly one pollfd.
    (unsafe { libc::poll(&mut fd, 1, 0) }) > 0
        && fd.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0
}

/// Serve one connection: read request lines, write responses. Runs until the client
/// closes, or a write to it times out (a client that cannot drain its buffer in a second
/// is gone; the next connect gets a fresh channel).
fn serve(mut conn: UnixStream, mut enqueue: impl FnMut(Request) -> bool) {
    let _ = conn.set_read_timeout(Some(Duration::from_millis(100)));
    let _ = conn.set_write_timeout(Some(Duration::from_secs(1)));
    let (sender, responses) = mpsc::sync_channel(256);
    let mut sender = Some(sender);
    let mut line = Vec::new();
    let mut pending = VecDeque::new();
    loop {
        pending.extend(responses.try_iter().take(256));
        if !pending.is_empty() {
            let mut out = String::new();
            for (id, r) in pending.drain(..) {
                out.push_str(&wire(id, &r));
                out.push('\n');
            }
            if conn.write_all(out.as_bytes()).is_err() {
                return;
            }
        }
        let Some(tx) = sender.as_ref() else {
            if peer_disconnected(&conn) {
                return;
            }
            match responses.recv_timeout(Duration::from_millis(100)) {
                Ok(reply) => pending.push_back(reply),
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            continue;
        };
        let mut buf = [0u8; 4096];
        match conn.read(&mut buf) {
            Ok(0) => sender = None,
            Ok(n) => {
                for chunk in buf[..n].split_inclusive(|byte| *byte == b'\n') {
                    if line.len() + chunk.len() > 64 * 1024 {
                        return;
                    }
                    line.extend_from_slice(chunk);
                    if chunk.ends_with(b"\n") {
                        if let Some(text) = std::str::from_utf8(&line).ok().map(str::trim) {
                            if !text.is_empty() {
                                handle_line(text, tx, &mut pending, &mut enqueue);
                            }
                        } else {
                            handle_line("", tx, &mut pending, &mut enqueue);
                        }
                        line.clear();
                    }
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(_) => return,
        }
    }
}

/// Parse the envelope first so malformed verbs retain a valid request ID.
/// One line of the wire: a request object (`{"id": 42, "req": ...}`) or
/// nothing. Returns the client's id and its verb when the line is a request
/// object; a malformed line's reply is the protocol's only on-the-spot answer,
/// and echoes its ID when the envelope contains one.
fn parse(line: &str) -> Result<(u64, Verb), Option<u64>> {
    let mut request: serde_json::Value = serde_json::from_str(line).map_err(|_| None)?;
    let id = request["id"].as_u64().ok_or(None)?;
    let verb = serde_json::from_value(request["req"].take()).map_err(|_| Some(id))?;
    Ok((id, verb))
}

/// Post a well-formed request to the frame thread's input path (the response
/// comes via [`reply`], later) or answer a protocol-level error immediately —
/// a malformed envelope may have no id to correlate with, and the frame thread should
/// never parse it.
fn handle_line(
    line: &str,
    sender: &mpsc::SyncSender<Response>,
    pending: &mut VecDeque<Response>,
    enqueue: &mut impl FnMut(Request) -> bool,
) {
    let (id, verb) = match parse(line) {
        Ok(v) => v,
        Err(id) => {
            pending.push_back((
                id,
                Reply::err(ApiError::Malformed {
                    reason: "the line is not a request object".to_string(),
                }),
            ));
            return;
        }
    };
    if !enqueue(Request {
        reply: ReplyTo {
            id,
            sender: sender.clone(),
        },
        verb,
    }) {
        pending.push_back((Some(id), Reply::err(ApiError::QueueFull)));
    }
    // No on-the-spot answer to a well-formed request: it sits on the frame
    // thread's queue until the action's outcome lands.
}

/// One response line.
fn wire(id: Option<u64>, r: &Reply) -> String {
    #[derive(Serialize)]
    struct F<'a> {
        id: Option<u64>,
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        outcome: Option<&'a Outcome>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<&'a ApiError>,
        #[serde(skip_serializing_if = "Option::is_none")]
        state: Option<&'a str>,
    }
    let f = F {
        id,
        ok: r.ok,
        outcome: r.outcome.as_ref(),
        error: r.error.as_ref(),
        state: r.state,
    };
    serde_json::to_string(&f).unwrap_or_else(|_| {
        r#"{"id":null,"ok":false,"error":{"code":"internal","reason":"response serialization failed"}}"#
            .to_string()
    })
}

#[cfg(test)]
mod tests {
    //! The wire, pinned: every verb, every outcome, every error serializes to
    //! the exact JSON a client may match on, and a full conversation round-trips
    //! over a real socket — the reply channel included.
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    fn verb(v: &Verb) -> &'static str {
        match v {
            Verb::Play { .. } => "play",
            Verb::Seek { .. } => "seek",
            Verb::Audio { .. } => "audio",
            Verb::Sub { .. } => "sub",
            Verb::Quality { .. } => "quality",
            Verb::Profile { .. } => "profile",
            Verb::Pause => "pause",
            Verb::Resume => "resume",
            Verb::Stop => "stop",
            Verb::State => "state",
            Verb::Shot => "shot",
        }
    }

    #[test]
    fn parse_the_whole_verb_vocabulary() {
        let cases: [(&str, u64, Verb); 11] = [
            (
                r#"{"id": 1, "req": {"kind": "play", "rk": "1$abc"}}"#,
                1,
                Verb::Play { rk: "1$abc".into() },
            ),
            (
                r#"{"id": 2, "req": {"kind": "seek", "ms": 120000}}"#,
                2,
                Verb::Seek { ms: 120_000 },
            ),
            (
                r#"{"id": 3, "req": {"kind": "audio", "sid": 2}}"#,
                3,
                Verb::Audio { sid: 2 },
            ),
            (
                r#"{"id": 4, "req": {"kind": "sub", "sid": 1}}"#,
                4,
                Verb::Sub { sid: 1 },
            ),
            (
                r#"{"id": 5, "req": {"kind": "quality", "rung": "720p_4_mbps"}}"#,
                5,
                Verb::Quality {
                    rung: "720p_4_mbps".into(),
                },
            ),
            (
                r#"{"id": 6, "req": {"kind": "profile", "index": 1}}"#,
                6,
                Verb::Profile { index: 1 },
            ),
            (r#"{"id": 7, "req": {"kind": "pause"}}"#, 7, Verb::Pause),
            (r#"{"id": 8, "req": {"kind": "resume"}}"#, 8, Verb::Resume),
            (r#"{"id": 9, "req": {"kind": "stop"}}"#, 9, Verb::Stop),
            (r#"{"id": 10, "req": {"kind": "state"}}"#, 10, Verb::State),
            (r#"{"id": 11, "req": {"kind": "shot"}}"#, 11, Verb::Shot),
        ];
        for (line, want_id, want) in cases {
            let (id, got) = parse(line).unwrap_or_else(|_| panic!("parse: {line}"));
            assert_eq!(id, want_id, "the id for {line}");
            assert_eq!(verb(&got), verb(&want), "the verb for {line}");
        }
        // The door: not a request object, and an unknown kind are not requests.
        for bad in [
            r#"not json at all"#,
            r#"{"id": 1}"#,
            r#"{"id": 1, "req": {"kind": "explode"}}"#,
            r#"{"id": 1, "req": {"kind": "seek"}}"#,
            r#"{"id": 1, "req": {"kind": "play"}}"#,
            "{}",
        ] {
            assert!(parse(bad).is_err(), "the door: {bad}");
        }
    }

    #[test]
    fn the_outcome_vocabulary_is_wire_pinned() {
        let o = |o: Outcome| {
            let id: Option<u64> = Some(7);
            let r = Reply::ok(o);
            wire(id, &r)
        };
        // The state snapshot is the app's own snapshot JSON, passed through
        // (a Value map serializes with sorted keys).
        assert_eq!(
            o(Outcome::State {
                snapshot: serde_json::json!({"state": "idle", "pos_ms": 0, "up_ms": 0})
            }),
            r#"{"id":7,"ok":true,"outcome":{"kind":"state","snapshot":{"pos_ms":0,"state":"idle","up_ms":0}}}"#
        );
        assert_eq!(
            o(Outcome::PlayStarted {
                rk: "1$abc".into(),
                title: "Akira".into()
            }),
            r#"{"id":7,"ok":true,"outcome":{"kind":"play_started","rk":"1$abc","title":"Akira"}}"#
        );
        assert_eq!(
            o(Outcome::PlayFailed {
                rk: "1$abc".into(),
                reason: "no PMS server selected yet (sign-in pending)".into()
            }),
            r#"{"id":7,"ok":true,"outcome":{"kind":"play_failed","rk":"1$abc","reason":"no PMS server selected yet (sign-in pending)"}}"#
        );
        assert_eq!(
            o(Outcome::SeekLanded {
                ms: 120_000,
                pos_ms: 120_345
            }),
            r#"{"id":7,"ok":true,"outcome":{"kind":"seek_landed","ms":120000,"pos_ms":120345}}"#
        );
        assert_eq!(
            o(Outcome::SeekFailed {
                ms: 120_000,
                reason: "the session left playback".into()
            }),
            r#"{"id":7,"ok":true,"outcome":{"kind":"seek_failed","ms":120000,"reason":"the session left playback"}}"#
        );
        assert_eq!(
            o(Outcome::TrackCommitted {
                track: "audio",
                sid: 2
            }),
            r#"{"id":7,"ok":true,"outcome":{"kind":"track_committed","track":"audio","sid":2}}"#
        );
        assert_eq!(
            o(Outcome::QualityCommitted {
                rung: "720p_4_mbps".into()
            }),
            r#"{"id":7,"ok":true,"outcome":{"kind":"quality_committed","rung":"720p_4_mbps"}}"#
        );
        assert_eq!(
            o(Outcome::ProfileSeated {
                index: 1,
                title: "Kids".into()
            }),
            r#"{"id":7,"ok":true,"outcome":{"kind":"profile_seated","index":1,"title":"Kids"}}"#
        );
        assert_eq!(
            o(Outcome::ProfileFailed {
                index: 3,
                reason: "the flow left".into()
            }),
            r#"{"id":7,"ok":true,"outcome":{"kind":"profile_failed","index":3,"reason":"the flow left"}}"#
        );
        assert_eq!(
            o(Outcome::Paused),
            r#"{"id":7,"ok":true,"outcome":{"kind":"paused"}}"#
        );
        assert_eq!(
            o(Outcome::Resumed),
            r#"{"id":7,"ok":true,"outcome":{"kind":"resumed"}}"#
        );
        assert_eq!(
            o(Outcome::Stopped),
            r#"{"id":7,"ok":true,"outcome":{"kind":"stopped"}}"#
        );
        assert_eq!(
            o(Outcome::ShotCaptured {
                path: "shot-7.png".into()
            }),
            r#"{"id":7,"ok":true,"outcome":{"kind":"shot_captured","path":"shot-7.png"}}"#
        );
        assert_eq!(
            o(Outcome::ShotFailed {
                reason: "the viewport was empty".into()
            }),
            r#"{"id":7,"ok":true,"outcome":{"kind":"shot_failed","reason":"the viewport was empty"}}"#
        );
    }

    #[test]
    fn the_error_vocabulary_is_wire_pinned() {
        let e = |e: ApiError| {
            let id: Option<u64> = Some(3);
            let r = Reply::err(e);
            wire(id, &r)
        };
        assert_eq!(
            e(ApiError::Malformed {
                reason: "the line is not a request object".into()
            }),
            r#"{"id":3,"ok":false,"error":{"code":"malformed","reason":"the line is not a request object"}}"#
        );
        assert_eq!(
            e(ApiError::NoEngine { state: "idle" }),
            r#"{"id":3,"ok":false,"error":{"code":"no_engine","state":"idle"}}"#
        );
        assert_eq!(
            e(ApiError::NoTrack { sid: 2 }),
            r#"{"id":3,"ok":false,"error":{"code":"no_track","sid":2}}"#
        );
        assert_eq!(
            e(ApiError::UnknownRung { rung: "4k".into() }),
            r#"{"id":3,"ok":false,"error":{"code":"unknown_rung","rung":"4k"}}"#
        );
        assert_eq!(
            e(ApiError::NotOnRoster {
                phase: "auth".into()
            }),
            r#"{"id":3,"ok":false,"error":{"code":"not_on_roster","phase":"auth"}}"#
        );
        assert_eq!(
            e(ApiError::NoProfile { index: 9, count: 2 }),
            r#"{"id":3,"ok":false,"error":{"code":"no_profile","index":9,"count":2}}"#
        );
        assert_eq!(
            e(ApiError::QueueFull),
            r#"{"id":3,"ok":false,"error":{"code":"queue_full"}}"#
        );
        assert_eq!(
            e(ApiError::Internal {
                reason: "the frame thread dropped it".into()
            }),
            r#"{"id":3,"ok":false,"error":{"code":"internal","reason":"the frame thread dropped it"}}"#
        );
        // The id-less door: a malformed line's reply carries no id.
        assert_eq!(
            {
                let r = Reply::err(ApiError::Malformed { reason: "x".into() });
                wire(None, &r)
            },
            r#"{"id":null,"ok":false,"error":{"code":"malformed","reason":"x"}}"#
        );
    }

    #[test]
    fn malformed_verb_keeps_a_valid_request_id() {
        let (sender, _) = mpsc::sync_channel(1);
        let mut pending = VecDeque::new();
        handle_line(
            r#"{"id":7,"req":{"kind":"explode"}}"#,
            &sender,
            &mut pending,
            &mut |_| panic!("invalid verb reached the app"),
        );
        let (id, reply) = pending.pop_front().unwrap();
        assert_eq!(id, Some(7));
        assert!(matches!(reply.error, Some(ApiError::Malformed { .. })));
    }

    fn start_serve() -> (String, mpsc::Receiver<Request>) {
        // The suite's TMPDIR hygiene check fails a binary that leaves anything behind,
        // so the scratch dir comes from the base's per-process helper (removed at exit).
        let dir = plx_base::testscratch::process_dir("api-sock");
        let sock = dir.join("t.sock");
        let _ = std::fs::remove_file(&sock);
        let listener = bind_private(&sock).unwrap();
        let path = sock.to_string_lossy().into_owned();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (conn, _) = listener.accept().unwrap();
            serve(conn, |request| tx.send(request).is_ok());
        });
        (path, rx)
    }

    fn read_line(conn: &UnixStream) -> String {
        let mut reader = BufReader::new(conn);
        let mut s = String::new();
        reader.read_line(&mut s).unwrap();
        s
    }

    #[test]
    fn a_conversation_round_trips_over_the_socket() {
        let (path, requests) = start_serve();
        let mut conn = UnixStream::connect(&path).unwrap();
        conn.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        conn.set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        use std::io::Read;

        // 1. The door: a malformed line answers on the spot, with no id.
        conn.write_all(b"garbage\n").unwrap();
        let line = read_line(&conn);
        assert!(line.contains("\"code\":\"malformed\""), "the door: {line}");
        assert!(line.contains("\"id\":null"), "the door names no id: {line}");

        // 2. A well-formed request gets NO on-the-spot answer: it sits on the
        //    frame thread's queue until the outcome lands.
        conn.write_all(b"{\"id\":7,\"req\":{\"kind\":\"state\"}}\n")
            .unwrap();
        let request = requests.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(request.verb, Verb::State));
        let mut probe = [0u8; 1];
        let blocked = conn.read(&mut probe).is_err();
        assert!(blocked, "no on-the-spot answer to a queued request");

        // 3. The reply channel: the frame thread's outcome rides back with the
        //    request's id, and the conversation continues on the same
        //    connection.
        request.reply.send(Reply::ok(Outcome::State {
            snapshot: serde_json::json!({
                "state": "playing",
                "pos_ms": 354_000,
                "title": "Akira",
                "up_ms": 90_000
            }),
        }));
        let line = read_line(&conn);
        let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["id"], 7, "the reply carries the request's id");
        assert_eq!(v["ok"], true);
        assert_eq!(v["outcome"]["kind"], "state");
        assert_eq!(v["outcome"]["snapshot"]["pos_ms"], 354_000);
    }

    #[test]
    fn a_client_can_close_its_writing_half_and_still_read_an_outcome() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let (send, requests) = mpsc::channel();
        let handler =
            std::thread::spawn(move || serve(server, |request| send.send(request).is_ok()));
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        client
            .write_all(b"{\"id\":17,\"req\":{\"kind\":\"stop\"}}\n")
            .unwrap();
        client.shutdown(std::net::Shutdown::Write).unwrap();
        let request = requests.recv_timeout(Duration::from_secs(2)).unwrap();
        request.reply.send(Reply::ok(Outcome::Stopped));
        drop(request);
        assert!(read_line(&client).contains("\"kind\":\"stopped\""));
        assert!(read_line(&client).is_empty());
        handler.join().unwrap();
    }

    #[test]
    fn a_late_reply_cannot_reach_a_new_connection_reusing_the_id() {
        let (mut previous, server) = UnixStream::pair().unwrap();
        let (send, old_requests) = mpsc::channel();
        let old_handler =
            std::thread::spawn(move || serve(server, |request| send.send(request).is_ok()));
        let request = b"{\"id\":7,\"req\":{\"kind\":\"state\"}}\n";
        previous.write_all(request).unwrap();
        let old = old_requests.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(previous);
        old_handler.join().unwrap();

        let (client, server) = UnixStream::pair().unwrap();
        let (send, receive) = mpsc::channel();
        let handler =
            std::thread::spawn(move || serve(server, |request| send.send(request).is_ok()));
        let mut client = client;
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        client.write_all(request).unwrap();
        let new = receive.recv_timeout(Duration::from_secs(2)).unwrap();
        old.reply.send(Reply::ok(Outcome::Stopped));
        new.reply.send(Reply::ok(Outcome::Resumed));
        let line = read_line(&client);
        assert!(line.contains("\"kind\":\"resumed\""), "{line}");
        drop(client);
        handler.join().unwrap();
    }
}
