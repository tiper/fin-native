//! The probe's runner: the one worker thread that performs the machine's commands against `libesplusplayer.so`, moves
//! the player's callbacks into the machine, and writes the lines.
//!
//! **Threads.** Every player call is made on this worker, in order, except the display bind, which runs on the host's
//! main thread (the Ecore main loop owns the window; the URI player's bind is made there too) while the worker waits
//! for it. The player's callbacks arrive on threads of its own (the runner counts which: the log says so): each one only
//! pushes a [`Callback`] into the scenario's [`Mailbox`] and wakes the worker.
//!
//! **The callbacks' memory.** The mailbox is the callbacks' `user_data`, handed to the library as a counted reference
//! ([`UserData`]) that is given back only once `esplusplayer_destroy` has returned 0. A player that is not destroyed (its
//! display bind never answered, its destroy was refused, or the worker unwound with it open) keeps its mailbox for the
//! rest of the process, so a callback that comes late always finds the memory it was given; the cost is one small
//! mailbox per such player. A player whose bind is still pending is not stopped or closed either: the host's job may be
//! calling into it on the main thread.
//!
//! **Ending early.** Between calls the worker checks the probe's deadline ([`PROBE_DEADLINE`]) and whether the app went
//! to the background or is stopping ([`Hosted::backgrounded`]); either cancels the probe ([`Probe::cancel`]), which tears
//! the current player down through the ordinary teardown and starts nothing more, so no tone plays behind Home and no
//! decoder stays held. A native call that never returns is not interrupted; [`super::command`] then reports the run done.

use super::api::{self, AudioStreamInfo, EsPacket, VideoStreamInfo};
use super::clip::Clip;
use super::machine::{Callback, Cmd, Outcome, Probe, Stream};
use crate::hosted::Hosted;
use plx_base::dynlib::Loaded;
use plx_base::eventlog::log;
use std::collections::VecDeque;
use std::ffi::{c_char, c_int, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

/// The whole probe's budget, checked between calls: past it the current player is torn down and the rest is not run. It
/// cannot interrupt a call that never returns; [`super::command`] reports such a run done a grace period later.
pub(super) const PROBE_DEADLINE: Duration = Duration::from_secs(180);
/// How long the worker waits for the host's main thread to bind the display.
const DISPLAY_WAIT: Duration = Duration::from_secs(5);
/// The worker's longest sleep between commands.
const IDLE_WAIT: Duration = Duration::from_millis(10);
/// The screen the window covers; the library's window is the screen's size (`ecore::Window::create`).
const AREA: [c_int; 4] = [0, 0, 1920, 1080];

pub struct Mailbox {
    queue: Mutex<VecDeque<Callback>>,
    wake: Condvar,
    worker: ThreadId,
    on_worker: AtomicU32,
    elsewhere: AtomicU32,
}

impl Mailbox {
    fn new(worker: ThreadId) -> Mailbox {
        Mailbox { queue: Mutex::new(VecDeque::new()), wake: Condvar::new(), worker, on_worker: AtomicU32::new(0), elsewhere: AtomicU32::new(0) }
    }

    fn push(&self, cb: Callback) {
        if std::thread::current().id() == self.worker {
            self.on_worker.fetch_add(1, Ordering::Relaxed);
        } else {
            self.elsewhere.fetch_add(1, Ordering::Relaxed);
        }
        self.queue.lock().unwrap_or_else(|e| e.into_inner()).push_back(cb);
        self.wake.notify_one();
    }

    fn drain(&self, wait: Duration) -> Vec<Callback> {
        let mut q = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        if q.is_empty() && !wait.is_zero() {
            q = self.wake.wait_timeout(q, wait).map(|(g, _)| g).unwrap_or_else(|e| e.into_inner().0);
        }
        q.drain(..).collect()
    }
}

/// One player's `user_data`: a counted reference to its mailbox, made with `Arc::into_raw`. Dropping it gives nothing
/// back (a raw pointer has no destructor), so every end but [`UserData::release`] leaks the reference on purpose,
/// unwinding included; see the module doc.
struct UserData(*const Mailbox);

impl UserData {
    fn new(mailbox: &Arc<Mailbox>) -> UserData {
        UserData(Arc::into_raw(Arc::clone(mailbox)))
    }

    fn ptr(&self) -> *mut c_void {
        self.0 as *mut c_void
    }

    /// Give the reference back.
    ///
    /// # Safety
    /// The player it was handed to is destroyed (`esplusplayer_destroy` returned 0), so no callback can come later.
    unsafe fn release(self) {
        drop(Arc::from_raw(self.0));
    }
}

/// The mailbox a callback's `user_data` names. SAFETY (callers): `user` is a [`UserData`] pointer, whose reference is
/// given back only after its player was destroyed.
unsafe fn mailbox<'a>(user: *mut c_void) -> Option<&'a Mailbox> {
    (user as *const Mailbox).as_ref()
}

fn stream_of(t: c_int) -> Stream {
    if t == api::STREAM_VIDEO {
        Stream::Video
    } else {
        Stream::Audio
    }
}

fn post(user: *mut c_void, cb: Callback) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: see `mailbox`.
        if let Some(m) = unsafe { mailbox(user) } {
            m.push(cb);
        }
    }));
}

unsafe extern "C" fn on_error(code: c_int, user: *mut c_void) {
    post(user, Callback::Error(code));
}
unsafe extern "C" fn on_buffer(t: c_int, status: c_int, user: *mut c_void) {
    post(user, Callback::Buffer(stream_of(t), status));
}
unsafe extern "C" fn on_conflict(user: *mut c_void) {
    post(user, Callback::ResourceConflicted);
}
unsafe extern "C" fn on_eos(user: *mut c_void) {
    post(user, Callback::Eos);
}
unsafe extern "C" fn on_ready_to_prepare(t: c_int, user: *mut c_void) {
    post(user, Callback::ReadyToPrepare(stream_of(t)));
}
unsafe extern "C" fn on_prepare_done(ok: bool, user: *mut c_void) {
    post(user, Callback::PrepareDone(ok));
}
unsafe extern "C" fn on_first_frame(user: *mut c_void) {
    post(user, Callback::FirstFrame);
}
unsafe extern "C" fn on_seek_done(user: *mut c_void) {
    post(user, Callback::SeekDone);
}
unsafe extern "C" fn on_ready_to_seek(t: c_int, offset: u64, user: *mut c_void) {
    post(user, Callback::ReadyToSeek(stream_of(t), offset));
}

struct Player {
    handle: *mut c_void,
    mailbox: Arc<Mailbox>,
    /// The library's reference to `mailbox`: given back only after a destroy that returned 0.
    user: UserData,
    /// The display bind was asked of the host and never answered: the host may still touch the handle, so it is never
    /// stopped, closed or destroyed.
    display_pending: bool,
}

struct Runner {
    hosted: &'static Hosted,
    player: Option<Player>,
    worker: ThreadId,
    /// The optional first-decoded-picture export is bound.
    first_frame: bool,
}

impl Runner {
    fn open(&mut self) -> Outcome {
        // SAFETY: a bound table (the runner loads it before any scenario); no argument.
        let handle = unsafe { api::esplusplayer_create() };
        if handle.is_null() {
            return Outcome::Open(Err(("create", 0)));
        }
        let mailbox = Arc::new(Mailbox::new(self.worker));
        let data = UserData::new(&mailbox);
        let user = data.ptr();
        self.player = Some(Player { handle, mailbox, user: data, display_pending: false });
        // SAFETY: a live handle and callbacks of the declared types; `user` outlives the player (see `UserData`). Every
        // callback is set before `open`, as the binding sets them.
        let steps: [(&'static str, c_int); 8] = unsafe {
            [
                ("set_error_cb", api::esplusplayer_set_error_cb(handle, on_error, user)),
                ("set_buffer_status_cb", api::esplusplayer_set_buffer_status_cb(handle, on_buffer, user)),
                ("set_resource_conflicted_cb", api::esplusplayer_set_resource_conflicted_cb(handle, on_conflict, user)),
                ("set_eos_cb", api::esplusplayer_set_eos_cb(handle, on_eos, user)),
                ("set_ready_to_prepare_cb", api::esplusplayer_set_ready_to_prepare_cb(handle, on_ready_to_prepare, user)),
                ("set_prepare_async_done_cb", api::esplusplayer_set_prepare_async_done_cb(handle, on_prepare_done, user)),
                ("set_seek_done_cb", api::esplusplayer_set_seek_done_cb(handle, on_seek_done, user)),
                ("set_ready_to_seek_cb", api::esplusplayer_set_ready_to_seek_cb(handle, on_ready_to_seek, user)),
            ]
        };
        if let Some(&(step, rc)) = steps.iter().find(|(_, rc)| *rc != 0) {
            return Outcome::Open(Err((step, rc)));
        }
        if self.first_frame {
            // SAFETY: as above; the optional table was bound before the first scenario.
            let rc = unsafe { api::esplusplayer_set_first_video_decoding_done_cb(handle, on_first_frame, user) };
            if rc != 0 {
                return Outcome::Open(Err(("set_first_video_decoding_done_cb", rc)));
            }
        }
        // SAFETY: a live handle with its callbacks set.
        let rc = unsafe { api::esplusplayer_open(handle) };
        if rc != 0 {
            return Outcome::Open(Err(("open", rc)));
        }
        Outcome::Open(Ok(()))
    }

    fn display(&mut self) -> Outcome {
        let Some(player) = self.player.as_mut() else { return Outcome::NoDisplay("no player") };
        let handle = player.handle as usize;
        let (tx, rx) = mpsc::channel::<Result<(c_int, c_int), &'static str>>();
        let job = Box::new(move || {
            let answer = catch_unwind(AssertUnwindSafe(|| bind_display(handle as *mut c_void))).unwrap_or(Err("the bind panicked"));
            let _ = tx.send(answer);
        });
        if !self.hosted.on_host(job) {
            return Outcome::NoDisplay("the host refused the main-thread job");
        }
        match rx.recv_timeout(DISPLAY_WAIT) {
            Ok(Ok((rc, mode))) => {
                log(&format!("esprobe: set_ecore_display -> {rc}, set_display_mode(letter box) -> {mode}"));
                Outcome::Rc(rc)
            }
            Ok(Err(why)) => Outcome::NoDisplay(why),
            Err(_) => {
                player.display_pending = true;
                Outcome::NoDisplay("no answer from the host's main thread")
            }
        }
    }

    fn handle(&self) -> *mut c_void {
        self.player.as_ref().map_or(std::ptr::null_mut(), |p| p.handle)
    }

    fn stream_info(&self, probe: &Probe, s: Stream) -> Outcome {
        let Some(clip) = probe.clip(s) else { return Outcome::Rc(-22) };
        let (cd, cd_len) = codec_data(clip);
        let h = self.handle();
        // SAFETY: a live handle; the struct and the codec data it points at live across the call (the library copies
        // what it keeps, as the binding's own call frees both right after).
        let rc = unsafe {
            match s {
                Stream::Video => {
                    let mut info = VideoStreamInfo {
                        codec_data: cd,
                        codec_data_length: cd_len,
                        mime_type: api::mime(clip.codec),
                        width: clip.a,
                        height: clip.b,
                        max_width: clip.a,
                        max_height: clip.b,
                        framerate_num: clip.c,
                        framerate_den: clip.d.max(1),
                    };
                    api::esplusplayer_set_video_stream_info(h, &mut info)
                }
                Stream::Audio => {
                    let mut info = AudioStreamInfo {
                        codec_data: cd,
                        codec_data_length: cd_len,
                        mime_type: api::mime(clip.codec),
                        bitrate: clip.c,
                        channels: clip.a,
                        sample_rate: clip.b,
                    };
                    api::esplusplayer_set_audio_stream_info(h, &mut info)
                }
            }
        };
        Outcome::Rc(rc)
    }

    fn submit(&self, probe: &Probe, s: Stream, index: usize) -> Outcome {
        let Some((packet, clip)) = probe.packet(s, index) else { return Outcome::Rc(api::SUBMIT_INVALID_PACKET) };
        let mut colour = api::hdr10_colour();
        let with_colour = clip.hdr10 && index == 0;
        let mut es = EsPacket {
            stream_type: if s == Stream::Video { api::STREAM_VIDEO } else { api::STREAM_AUDIO },
            buffer: packet.data.as_ptr() as *mut c_char,
            buffer_size: packet.data.len() as u32,
            pts: packet.pts_ms,
            duration: packet.duration_ms,
            matroska_color_info: if with_colour { (&mut colour as *mut api::MatroskaColor).cast() } else { std::ptr::null_mut() },
            hdr10p_metadata_size: 0,
            hdr10p_metadata: std::ptr::null_mut(),
        };
        // SAFETY: a live handle; the packet, its data and the colour live across the call, and the library copies the
        // payload before it returns (the binding frees its own copy right after the call).
        Outcome::Rc(unsafe { api::esplusplayer_submit_packet(self.handle(), &mut es) })
    }

    fn time(&self) -> Outcome {
        let mut ms: u64 = 0;
        // SAFETY: a live handle and a valid out pointer.
        let rc = unsafe { api::esplusplayer_get_playing_time(self.handle(), &mut ms) };
        Outcome::Time(if rc == 0 { Ok(ms) } else { Err(rc) })
    }

    fn teardown(&mut self) -> (Outcome, Option<Arc<Mailbox>>) {
        let Some(player) = self.player.take() else { return (Outcome::Teardown(0, 0, 0), None) };
        if player.display_pending {
            // The host's job may be inside `set_ecore_display` on this handle right now: no call is made on it, and its
            // mailbox is kept (`player.user` is dropped without being released).
            log("esprobe: the display bind never answered: the player is not stopped, closed or destroyed, and its callbacks' mailbox is kept");
            return (Outcome::Teardown(-1, -1, -1), Some(player.mailbox));
        }
        let h = player.handle;
        // SAFETY: a live handle, released here exactly once.
        let (stop, close, destroy) = unsafe { (api::esplusplayer_stop(h), api::esplusplayer_close(h), api::esplusplayer_destroy(h)) };
        if destroy == 0 {
            // SAFETY: the player is destroyed, so no callback of it can come later.
            unsafe { player.user.release() };
        } else {
            log("esprobe: destroy refused: the callbacks' mailbox is kept");
        }
        (Outcome::Teardown(stop, close, destroy), Some(player.mailbox))
    }

    fn perform(&mut self, probe: &Probe, cmd: Cmd) -> (Outcome, Option<Arc<Mailbox>>) {
        let h = self.handle();
        // SAFETY (every arm): `h` is the live handle `open` made; the machine asks for no call before `Open` succeeded
        // and none after `Teardown`.
        let outcome = match cmd {
            Cmd::Open => self.open(),
            Cmd::Display => self.display(),
            Cmd::StreamInfo(s) => self.stream_info(probe, s),
            Cmd::Prepare => Outcome::Rc(unsafe { api::esplusplayer_prepare_async(h) }),
            Cmd::Submit(s, i) => self.submit(probe, s, i),
            Cmd::Eos(s) => {
                let t = if s == Stream::Video { api::STREAM_VIDEO } else { api::STREAM_AUDIO };
                Outcome::Rc(unsafe { api::esplusplayer_submit_eos_packet(h, t) })
            }
            Cmd::Start => Outcome::Rc(unsafe { api::esplusplayer_start(h) }),
            Cmd::Pause => Outcome::Rc(unsafe { api::esplusplayer_pause(h) }),
            Cmd::Resume => Outcome::Rc(unsafe { api::esplusplayer_resume(h) }),
            Cmd::Seek(ms) => Outcome::Rc(unsafe { api::esplusplayer_seek(h, ms) }),
            Cmd::Time => self.time(),
            Cmd::Teardown => return self.teardown(),
        };
        (outcome, None)
    }
}

fn codec_data(clip: &Clip) -> (*mut c_char, u32) {
    if clip.codec_data.is_empty() {
        (std::ptr::null_mut(), 0)
    } else {
        (clip.codec_data.as_ptr() as *mut c_char, clip.codec_data.len() as u32)
    }
}

/// HOST THREAD. Find the library's window and bind the player to it as an overlay; the two return codes.
fn bind_display(handle: *mut c_void) -> Result<(c_int, c_int), &'static str> {
    if !api::ecore_find::load(None).ok() {
        return Err("libecore_wl2.so.1 lacks the window lookup");
    }
    // SAFETY: a bound table, called on the Ecore main thread; NULL asks for the connected display.
    let display = unsafe { api::ecore_wl2_connected_display_get(std::ptr::null()) };
    if display.is_null() {
        return Err("no connected Ecore display");
    }
    // SAFETY: as above, with the display just found.
    let window = unsafe { api::ecore_wl2_display_window_find(display, 1) };
    if window.is_null() {
        return Err("Ecore window 1 not found");
    }
    let [x, y, w, h] = AREA;
    // SAFETY: a live player handle (the worker waits for this job) and the library's own window.
    let rc = unsafe { api::esplusplayer_set_ecore_display(handle, api::DISPLAY_OVERLAY, window, x, y, w, h) };
    // SAFETY: as above.
    let mode = if rc == 0 { unsafe { api::esplusplayer_set_display_mode(handle, api::DISPLAY_MODE_LETTER_BOX) } } else { 0 };
    Ok((rc, mode))
}

/// The public fallback's presence, as a note (no PASS or FAIL: nothing of it is called): whether the native media
/// player library opens and exports its elementary-stream push family (`player_set_media_stream_info`,
/// `player_push_media_stream`, `player_foreach_media_stream_supported_format`; Tizen 3.0+ public API, present in the
/// TV 10.0 emulator's `libcapi-media-player.so.0.4.0`). If the set refuses `libesplusplayer.so`, this line says whether
/// the second route is there to try, without a second round with the tester.
fn fallback_presence() -> String {
    const LIB: &str = "libcapi-media-player.so.0";
    const SYMBOLS: [&str; 3] = ["player_set_media_stream_info", "player_push_media_stream", "player_foreach_media_stream_supported_format"];
    let Some((handle, _)) = plx_base::dynlib::Handle::open(&[LIB]) else {
        return format!("fallback {LIB} did not open");
    };
    let missing: Vec<&str> = SYMBOLS.iter().copied().filter(|s| handle.sym(s).is_none()).collect();
    if missing.is_empty() {
        format!("fallback {LIB}: the media-stream push family is present ({})", SYMBOLS.join(", "))
    } else {
        format!("fallback {LIB}: lacks {}", missing.join(", "))
    }
}

fn describe(what: &str, loaded: Loaded) -> Result<String, String> {
    match loaded {
        Loaded::Ok(soname) => Ok(format!("{what} bound from {soname}")),
        Loaded::NoLibrary => Err(format!("{what}: libesplusplayer.so did not open")),
        Loaded::Incomplete(soname, n) => Err(format!("{what}: {soname} lacks {n} symbol(s) (named above)")),
    }
}

fn flush_lines(probe: &mut Probe) {
    for line in probe.take_lines() {
        log(&line);
    }
}

/// The API and layout checks, then every scenario. Returns the tally; `progress` is told the tally so far after every
/// step, on this thread.
pub fn run(
    hosted: &'static Hosted,
    scenarios: Vec<super::machine::Scenario>,
    preface: Vec<(bool, String, String)>,
    progress: &dyn Fn(u32, u32),
) -> (u32, u32) {
    let mut probe = Probe::new(Vec::new());
    for (pass, step, detail) in preface {
        probe.record(pass, &step, &detail);
    }
    probe.note(fallback_presence());
    match describe("the player API", api::esplus::load(None)) {
        Ok(detail) => probe.record(true, "api", &detail),
        Err(detail) => {
            probe.record(false, "api", &detail);
            flush_lines(&mut probe);
            return (probe.pass, probe.fail);
        }
    }
    let sizes = match describe("the size exports", api::esplus_sizes::load(None)) {
        // SAFETY: a bound table; the functions take nothing and return a size.
        Ok(_) => unsafe {
            [
                i64::from(api::get_size_of_esplusplayer_es_packet()),
                i64::from(api::get_size_of_esplusplayer_video_stream_info()),
                i64::from(api::get_size_of_esplusplayer_audio_stream_info()),
            ]
        },
        Err(detail) => {
            probe.record(false, "layout", &detail);
            flush_lines(&mut probe);
            return (probe.pass, probe.fail);
        }
    };
    let ours = api::compiled_sizes();
    let detail = format!("es_packet {} / {}, video_stream_info {} / {}, audio_stream_info {} / {} (library / this build)",
        sizes[0], ours[0], sizes[1], ours[1], sizes[2], ours[2]);
    if !api::layout_matches(sizes) {
        probe.record(false, "layout", &detail);
        flush_lines(&mut probe);
        return (probe.pass, probe.fail);
    }
    probe.record(true, "layout", &detail);

    let first_frame = api::esplus_first_frame::load(None).ok();
    if !first_frame {
        probe.note("the first-decoded-picture export is missing: no first-frame step".to_owned());
    }
    let mut probe = probe.with_scenarios(scenarios);
    probe.watch_first_frame(first_frame);
    let mut runner = Runner { hosted, player: None, worker: std::thread::current().id(), first_frame };
    let started = Instant::now();
    let now = || started.elapsed().as_millis() as u64;
    let mut current: Option<Arc<Mailbox>> = None;
    loop {
        flush_lines(&mut probe);
        progress(probe.pass, probe.fail);
        if probe.finished() {
            break;
        }
        // Both end the probe through the machine (`cancel` is once only), so the open player gets its ordinary teardown.
        if started.elapsed() > PROBE_DEADLINE {
            probe.cancel(&format!("the probe ran past {} s; the current player is torn down and the rest is not run", PROBE_DEADLINE.as_secs()));
        } else if hosted.backgrounded() {
            probe.cancel("the app went to the background or is stopping; the current player is torn down and the rest is not run");
        }
        let mailbox = runner.player.as_ref().map(|p| Arc::clone(&p.mailbox)).or_else(|| current.clone());
        if let Some(m) = &mailbox {
            for cb in m.drain(Duration::ZERO) {
                probe.callback(cb, now());
            }
        }
        match probe.next(now()) {
            Some(cmd) => {
                let t0 = Instant::now();
                let (outcome, ended) = runner.perform(&probe, cmd);
                let took = t0.elapsed().as_millis() as u64;
                if let Some(m) = ended {
                    let (here, there) = (m.on_worker.load(Ordering::Relaxed), m.elsewhere.load(Ordering::Relaxed));
                    probe.note(format!("{} callbacks: {there} on the player's own threads, {here} on the calling thread", probe.scenario_name()));
                    current = None;
                } else if runner.player.is_some() {
                    current = runner.player.as_ref().map(|p| Arc::clone(&p.mailbox));
                }
                probe.done(cmd, outcome, now(), took);
            }
            None => {
                if let Some(m) = &mailbox {
                    for cb in m.drain(IDLE_WAIT) {
                        probe.callback(cb, now());
                    }
                } else {
                    std::thread::sleep(IDLE_WAIT);
                }
            }
        }
    }
    flush_lines(&mut probe);
    progress(probe.pass, probe.fail);
    drop(runner);
    (probe.pass, probe.fail)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A player's mailbox outlives every end but a destroy.** The library holds the mailbox as `user_data`; a worker
    /// that unwinds with the player open, or a player left undestroyed, must leave that memory alive for a late
    /// callback, and only a destroyed player's reference is given back.
    #[test]
    fn a_players_mailbox_outlives_every_end_but_a_destroy() {
        let mailbox = Arc::new(Mailbox::new(std::thread::current().id()));
        let unwound = UserData::new(&mailbox);
        let user = unwound.ptr();
        assert_eq!(Arc::strong_count(&mailbox), 2);
        let _ = catch_unwind(AssertUnwindSafe(move || {
            let _held = unwound;
            panic!("the worker unwinds with a player open");
        }));
        assert_eq!(Arc::strong_count(&mailbox), 2, "the unwind kept the library's reference");
        post(user, Callback::Eos);
        assert!(matches!(mailbox.drain(Duration::ZERO)[..], [Callback::Eos]), "a late callback still lands");

        let destroyed = UserData::new(&mailbox);
        assert_eq!(Arc::strong_count(&mailbox), 3);
        // SAFETY: no player was ever given this reference.
        unsafe { destroyed.release() };
        assert_eq!(Arc::strong_count(&mailbox), 2, "a destroyed player's reference is given back");
    }
}
