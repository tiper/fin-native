//! The hosted contract's logic (`docs/platform-ports.md`, Target shape 7), with no Ecore and no EGL in it.
//!
//! [`Hosted`] is everything behind the three C exports: the gate every host-bound callback enters through,
//! the exactly-once completion, the bounded input queue, the copy of the host's callback table, and the
//! start, post, stop and deliver steps. It is an instance (built by a `const fn`, held in one `static` by
//! [`crate::exports`]) so a host test builds its own and drives it with a fake host table and injected
//! closures for the two things that need the platform: bringing the window up on the host thread
//! (`bring_up`) and running the application on the loop thread (`run`). The contract's list is tested
//! here (they run under `cargo test --lib -p plx_platform_tizen --features tizen`: the crate is empty
//! without its `tizen` feature, which no `make check` target passes): a refused table or launch string, a quit while idle, a stop with a callback in flight
//! (`clean = 1` only after it returns), a loop that returns with a retained worker (`clean = 0`), a
//! stalled loop and the deadline, a pause then a stop.
//!
//! Threads. `start` and `stop` run on the host's main thread; the loop thread is spawned here and runs
//! the application; `post` may be called from any thread and `push` from the Ecore handlers (the host
//! thread): both are a bounded queue push that never blocks. [`Hosted::deliver`] drains the queue on the
//! loop thread, which polls it every iteration, so nothing needs waking. The loop never waits on the host:
//! work for the host thread leaves as [`Hosted::on_host`], through the host's `run_on_host`.
use crate::host_abi::{self, HostCallbacks, HostEvent, PlxHostApi, PlxHostEvent};
use crate::launch::Launch;
use plxnative_modules::host::{Action, Completion, Edge, EventSink, Exit, HostGate, Lifecycle, Stopped};
use std::collections::VecDeque;
use std::ffi::{c_char, c_void, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// How many inputs the queue holds before a push is refused. A remote makes a handful of events a second;
/// the bound is only ever reached when the loop thread is not draining.
const QUEUE_LIMIT: usize = 256;

/// The loop thread's stack: the application was written for a television's 8 MiB main thread.
const LOOP_STACK_BYTES: usize = 8 << 20;

/// What `plx_host_stop` waits for the loop to finish before it reports the stop unclean.
pub const STOP_DEADLINE: Duration = Duration::from_secs(5);

/// One input the host side queued for the loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    /// A key with an action, and its edge.
    Key(Action, Edge),
    /// `OnPause`.
    Pause,
    /// `OnResume`.
    Resume,
}

/// A piece of work for the host's main thread.
type Job = Box<dyn FnOnce() + Send>;

pub struct Hosted {
    callbacks: OnceLock<HostCallbacks>,
    /// The loop thread was started and has not reported yet.
    running: AtomicBool,
    gate: HostGate,
    done: Completion,
    queue: Mutex<VecDeque<Input>>,
    /// Pushes refused because the queue was full.
    dropped: AtomicUsize,
    /// How many of those [`Hosted::deliver`] has said so in the log.
    said: AtomicUsize,
    /// `deliver` has handed the application the stop as a quit.
    quit_sent: AtomicBool,
    /// The application was told it went to the background and has not been told it came back. Loop thread
    /// only (`deliver`); atomic so that `new` stays `const`.
    paused: AtomicBool,
}

impl Default for Hosted {
    fn default() -> Self {
        Self::new()
    }
}

impl Hosted {
    pub const fn new() -> Self {
        Self {
            callbacks: OnceLock::new(),
            running: AtomicBool::new(false),
            gate: HostGate::new(),
            done: Completion::new(),
            queue: Mutex::new(VecDeque::new()),
            dropped: AtomicUsize::new(0),
            said: AtomicUsize::new(0),
            quit_sent: AtomicBool::new(false),
            paused: AtomicBool::new(false),
        }
    }

    /// `on_stopped`, through the copied table. Only [`Completion::deliver`] reaches it, so it runs once.
    fn report(&self, stopped: Stopped) {
        if let Some(cb) = self.callbacks.get() {
            // SAFETY: the host's callback, kept callable until `on_stopped` has been delivered clean.
            unsafe { (cb.on_stopped)(stopped.code, i32::from(stopped.clean)) };
        }
    }

    /// A refusal after the table was accepted: nothing was started and nothing is held, so the report is
    /// `on_stopped(1, 1)` on this (the host) thread, and later callbacks are refused.
    pub(crate) fn refuse(&self, code: i32) -> i32 {
        self.gate.stop();
        self.done.deliver(Stopped { code: 1, clean: true }, |s| self.report(s));
        code
    }

    /// `plx_host_start`. `bring_up` runs on the calling (host) thread and returns what the loop thread needs
    /// from it (the window); `run` runs the application on the new loop thread and returns its [`Exit`].
    pub fn start<W: Made + 'static>(
        &'static self,
        api: *const PlxHostApi,
        launch_json: *const c_char,
        bring_up: impl FnOnce(&Launch) -> Result<W, i32>,
        run: impl FnOnce(Launch, W) -> Exit + Send + 'static,
    ) -> i32 {
        // SAFETY: `api` is the host's table, readable as far as its own `size` says.
        let callbacks = match unsafe { host_abi::read_table(api) } {
            Ok(callbacks) => callbacks,
            Err(code) => return code,
        };
        if self.gate.stopping() || self.callbacks.set(callbacks).is_err() {
            return host_abi::E_STATE;
        }
        let launch = match launch_of(launch_json) {
            Some(launch) => launch,
            None => return self.refuse(host_abi::E_LAUNCH),
        };
        // A panic while bringing up is a refusal like any other (`E_WINDOW`, with the callback): the step that
        // made the window releases it itself ([`guarded`]), so nothing is left shown.
        let window = match catch_unwind(AssertUnwindSafe(|| bring_up(&launch))) {
            Ok(Ok(window)) => window,
            Ok(Err(code)) => return self.refuse(code),
            Err(_) => {
                plx_base::eventlog::log("host: bringing the platform up panicked");
                return self.refuse(host_abi::E_WINDOW);
            }
        };
        self.running.store(true, Ordering::SeqCst);
        let started = give_to_loop(
            window,
            |body| plx_base::task::spawn_sized("plx-loop", LOOP_STACK_BYTES, body).is_some(),
            move |window| {
                // A panic in the application must still end in a report: it is a loop that died, so nothing it
                // held is released (`Exit::refused` is `Retained`).
                let exit = catch_unwind(AssertUnwindSafe(|| run(launch, window))).unwrap_or(Exit::refused(1));
                self.finish(&exit);
            },
        );
        if !started {
            self.running.store(false, Ordering::SeqCst);
            return self.refuse(host_abi::E_THREAD);
        }
        host_abi::OK
    }

    /// The loop thread's tail once the application returned `exit`: ask the gate until no callback is
    /// running, then report once (the deadline thread may have won the race, and then this is suppressed).
    fn finish(&self, exit: &Exit) {
        loop {
            if let Some(stopped) = self.gate.report(exit) {
                self.done.deliver(stopped, |s| self.report(s));
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        self.running.store(false, Ordering::SeqCst);
    }

    /// `plx_host_stop`: refuse every later callback, ask the application to end (the loop's next
    /// [`Hosted::deliver`] hands it a quit) and arm the deadline. Returns at once.
    pub fn stop(&'static self, deadline: Duration) {
        if !self.gate.stop() || !self.running.load(Ordering::SeqCst) {
            return;
        }
        let armed = plx_base::task::spawn("plx-deadline", move || {
            std::thread::sleep(deadline);
            // The loop did not finish in time (a stalled swap): never clean, because it released nothing.
            self.done.deliver(self.gate.expire(1), |s| self.report(s));
        });
        if armed.is_none() {
            self.done.deliver(self.gate.expire(1), |s| self.report(s));
        }
    }

    /// Queue `input`. Refused (`Err`) after a stop or when the loop is not running, and when the queue is
    /// full; never blocks.
    pub fn push(&self, input: Input) -> Result<(), i32> {
        if self.gate.stopping() || !self.running.load(Ordering::SeqCst) {
            return Err(host_abi::E_STATE);
        }
        let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        if queue.len() >= QUEUE_LIMIT {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return Err(host_abi::E_FULL);
        }
        queue.push_back(input);
        Ok(())
    }

    /// How many inputs were refused for a full queue.
    #[cfg(test)]
    pub fn dropped(&self) -> usize {
        self.dropped.load(Ordering::Relaxed)
    }

    /// `plx_host_post`: decode the host's event and queue it.
    pub fn post(&self, event: *const PlxHostEvent) -> i32 {
        // SAFETY: `event` is the host's, readable as far as its own `size` says.
        let input = match unsafe { host_abi::read_event(event) } {
            Ok(HostEvent::Pause) => Input::Pause,
            Ok(HostEvent::Resume) => Input::Resume,
            Err(code) => return code,
        };
        self.push(input).map_or_else(|code| code, |()| host_abi::OK)
    }

    /// `Surface::deliver`: hand the application everything queued, in order, then the stop as a quit.
    /// Loop thread only. A pause disarms the process watchdog after the application has been told, and a
    /// resume arms it again BEFORE it is told, on this thread (`arm` captures the calling thread).
    pub fn deliver(&self, sink: &mut dyn EventSink) {
        let batch: Vec<Input> = self.queue.lock().unwrap_or_else(|e| e.into_inner()).drain(..).collect();
        for input in batch {
            match input {
                Input::Key(action, edge) => {
                    sink.key(Some(action), None, edge);
                }
                Input::Pause => {
                    // One transition is one DID edge: the application works and logs once per edge, and DID
                    // alone is complete. A pause while paused is no transition.
                    if !self.paused.swap(true, Ordering::SeqCst) {
                        sink.lifecycle(Lifecycle::DidBackground);
                        plx_base::task::watchdog::disarm();
                    }
                }
                Input::Resume => {
                    if self.paused.swap(false, Ordering::SeqCst) {
                        plx_base::task::watchdog::arm();
                        sink.lifecycle(Lifecycle::DidForeground);
                    }
                }
            }
        }
        let lost = self.dropped.load(Ordering::Relaxed);
        if lost > self.said.swap(lost, Ordering::Relaxed) {
            plx_base::eventlog::log(&format!("host: {lost} input(s) dropped so far (the queue was full)"));
        }
        if self.gate.stopping() && !self.quit_sent.swap(true, Ordering::SeqCst) {
            sink.quit();
        }
    }

    /// A stop was requested (`plx_host_stop` closed the gate).
    pub(crate) fn stopping(&self) -> bool {
        self.gate.stopping()
    }

    /// Ask the host to run `job` on its main thread, through `run_on_host`, without waiting for it. `false`
    /// when a stop was requested (the gate refuses it: a host that stopped the library owns the window's end
    /// with the process's), or the host refused the request; the job is then dropped unrun.
    pub fn on_host(&self, job: Job) -> bool {
        let Some(_in_flight) = self.gate.enter() else { return false };
        let Some(callbacks) = self.callbacks.get() else { return false };
        let arg = Box::into_raw(Box::new(job)).cast::<c_void>();
        // SAFETY: the host's callback; `arg` is passed back to `run_job` exactly once or reclaimed below.
        if unsafe { (callbacks.run_on_host)(run_job, arg) } == host_abi::OK {
            return true;
        }
        // SAFETY: the host refused the request, so `run_job` will never see `arg`.
        drop(unsafe { Box::from_raw(arg.cast::<Job>()) });
        false
    }
}

/// What `bring_up` made on the host thread and the host thread must release if the start is then refused
/// (the Ecore window: shown, with its handlers registered, until [`Made::unmake`]).
pub trait Made: Send {
    /// Release it. HOST THREAD.
    fn unmake(self);
}

impl Made for () {
    fn unmake(self) {}
}

/// Hand `made` to the loop thread: `spawn` starts a thread running the closure it is given (`false`: it could
/// not), and that closure passes `made` to `body`. The window waits in a slot the loop thread empties first, so
/// when the thread cannot be started it is released HERE, on the host thread, before the refusal reports
/// "nothing is held" (a window dropped inside the never-run closure would stay shown for the process's life).
fn give_to_loop<W: Made + 'static>(
    made: W,
    spawn: impl FnOnce(Box<dyn FnOnce() + Send>) -> bool,
    body: impl FnOnce(W) + Send + 'static,
) -> bool {
    let slot = std::sync::Arc::new(Mutex::new(Some(made)));
    let theirs = std::sync::Arc::clone(&slot);
    let started = spawn(Box::new(move || {
        let taken = theirs.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(made) = taken {
            body(made);
        }
    }));
    if !started {
        if let Some(made) = slot.lock().unwrap_or_else(|e| e.into_inner()).take() {
            made.unmake();
        }
    }
    started
}

/// Run `step` on what was just made; a panic in it releases it and is a refusal (`E_WINDOW`), so a panic
/// between making the window and the loop owning it leaves nothing shown.
pub fn guarded<W: Made>(made: W, step: impl FnOnce(&W)) -> Result<W, i32> {
    if catch_unwind(AssertUnwindSafe(|| step(&made))).is_err() {
        plx_base::eventlog::log("host: a step on the new window panicked; the window is released");
        made.unmake();
        return Err(host_abi::E_WINDOW);
    }
    Ok(made)
}

/// The task the host calls on its main thread for [`Hosted::on_host`].
unsafe extern "C" fn run_job(arg: *mut c_void) {
    // SAFETY: `arg` is the `Box<Job>` `on_host` leaked, handed back once.
    let job = *unsafe { Box::from_raw(arg.cast::<Job>()) };
    // A panic must not unwind into the host's native frames.
    let _ = catch_unwind(AssertUnwindSafe(job));
}

/// The launch string at `launch_json`, parsed; `None` for a null pointer, text that is not UTF-8 and every
/// refusal [`Launch::parse`] makes. Never logs: the text carries the dev token.
fn launch_of(launch_json: *const c_char) -> Option<Launch> {
    if launch_json.is_null() {
        return None;
    }
    // SAFETY: the host's NUL-terminated string, valid for this call; it is copied by the parse.
    let text = unsafe { CStr::from_ptr(launch_json) }.to_str().ok()?;
    Launch::parse(text).ok()
}

/// The fake host the contract tests and the surface tests share.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use crate::host_abi::ABI_VERSION;
    use plxnative_modules::host::{PointerEdge, RawKey, Release};
    use std::sync::mpsc;
    use std::time::Instant;

    pub(crate) const LAUNCH: &CStr = c"{\"payload_dir\":\"/res\",\"state_dir\":\"/data\",\"pms_host\":\"10.0.2.2\",\"pms_port\":32499}";

    /// What the fake host saw. Its callbacks are plain `extern "C"` functions, which capture nothing, so the
    /// record is a process global and every test that uses it holds the process-global test lock.
    pub(crate) struct Seen {
        /// Each `on_stopped`: the code, `clean`, whether the watchdog was armed, and the thread.
        pub(crate) stopped: Vec<(i32, i32, bool, std::thread::ThreadId)>,
        /// What `run_on_host` was asked to run, kept for the test to run on its own "host thread".
        pub(crate) tasks: Vec<(crate::host_abi::HostTask, usize)>,
        /// `run_on_host`'s answer.
        pub(crate) run_answer: i32,
        /// Armed by a test: `run_on_host` blocks until this is released, holding a callback in flight.
        pub(crate) block_run: Option<mpsc::Receiver<()>>,
        pub(crate) entered: Option<mpsc::Sender<()>>,
    }

    pub(crate) static SEEN: Mutex<Seen> =
        Mutex::new(Seen { stopped: Vec::new(), tasks: Vec::new(), run_answer: 0, block_run: None, entered: None });

    pub(crate) fn seen<R>(f: impl FnOnce(&mut Seen) -> R) -> R {
        f(&mut SEEN.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub(crate) unsafe extern "C" fn fake_on_stopped(code: i32, clean: i32) {
        let armed = plx_base::task::watchdog::is_armed();
        seen(|s| s.stopped.push((code, clean, armed, std::thread::current().id())));
    }

    pub(crate) unsafe extern "C" fn fake_run_on_host(task: crate::host_abi::HostTask, arg: *mut c_void) -> i32 {
        let (answer, blocker, entered) = seen(|s| (s.run_answer, s.block_run.take(), s.entered.take()));
        if let Some(entered) = entered {
            let _ = entered.send(());
        }
        if let Some(blocker) = blocker {
            let _ = blocker.recv_timeout(Duration::from_secs(5));
        }
        if answer == 0 {
            seen(|s| s.tasks.push((task, arg as usize)));
        }
        answer
    }

    pub(crate) fn table() -> PlxHostApi {
        PlxHostApi {
            size: std::mem::size_of::<PlxHostApi>() as u32,
            abi_version: ABI_VERSION,
            flags: 0,
            reserved: 0,
            on_stopped: Some(fake_on_stopped),
            run_on_host: Some(fake_run_on_host),
        }
    }

    /// A host test's world: the serial lock (`Completion::deliver` disarms the process watchdog), a clean
    /// record, and a fresh `Hosted` that lives for the process (its threads need `'static`).
    pub(crate) struct World {
        pub(crate) _serial: plx_base::testlock::Serial,
        pub(crate) hosted: &'static Hosted,
    }

    impl World {
        pub(crate) fn new() -> World {
            let mut world = World { _serial: plx_base::testlock::serial(), hosted: Box::leak(Box::new(Hosted::new())) };
            world.reset();
            world
        }

        /// A fresh `Hosted` and an empty record, for a second scenario in one test.
        pub(crate) fn reset(&mut self) {
            seen(|s| {
                s.stopped.clear();
                s.tasks.clear();
                s.run_answer = 0;
                s.block_run = None;
                s.entered = None;
            });
            self.hosted = Box::leak(Box::new(Hosted::new()));
        }

        pub(crate) fn stopped(&self) -> Vec<(i32, i32)> {
            seen(|s| s.stopped.iter().map(|r| (r.0, r.1)).collect())
        }

        /// Wait until `n` reports have arrived (or fail after 5 s).
        pub(crate) fn wait_stopped(&self, n: usize) -> Vec<(i32, i32)> {
            let end = Instant::now() + Duration::from_secs(5);
            while seen(|s| s.stopped.len()) < n {
                assert!(Instant::now() < end, "{n} on_stopped report(s) within 5 s, saw {:?}", self.stopped());
                std::thread::sleep(Duration::from_millis(2));
            }
            self.stopped()
        }

        pub(crate) fn start<W: super::Made + 'static>(
            &self,
            bring_up: impl FnOnce(&Launch) -> Result<W, i32>,
            run: impl FnOnce(Launch, W) -> Exit + Send + 'static,
        ) -> i32 {
            let api = table();
            self.hosted.start(&api, LAUNCH.as_ptr(), bring_up, run)
        }
    }

    pub(crate) fn complete() -> Exit {
        Exit { code: 0, release: Release::Complete }
    }

    /// An `EventSink` that records what the loop was handed.
    #[derive(Default)]
    pub(crate) struct Rec {
        pub(crate) seen: Vec<String>,
    }

    impl EventSink for Rec {
        fn key(&mut self, action: Option<Action>, raw: Option<RawKey>, edge: Edge) -> bool {
            self.seen.push(format!("key {action:?} {raw:?} {edge:?}"));
            true
        }
        fn text(&mut self, text: &str) {
            self.seen.push(format!("text {text}"));
        }
        fn pointer(&mut self, _x: f32, _y: f32, _edge: PointerEdge) {
            self.seen.push("pointer".into());
        }
        fn wheel(&mut self, dy: i32) {
            self.seen.push(format!("wheel {dy}"));
        }
        fn lifecycle(&mut self, edge: Lifecycle) {
            self.seen.push(format!("lifecycle {edge:?}"));
        }
        fn quit(&mut self) {
            self.seen.push("quit".into());
        }
    }

    /// The loop an application runs: poll `deliver` until it is told to quit, then return `exit`.
    pub(crate) fn loop_until_quit(hosted: &'static Hosted, exit: Exit) -> impl FnOnce(Launch, ()) -> Exit + Send + 'static {
        move |_, ()| {
            let mut rec = Rec::default();
            while !rec.seen.iter().any(|e| e == "quit") {
                hosted.deliver(&mut rec);
                std::thread::sleep(Duration::from_millis(1));
            }
            exit
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use crate::host_abi::{EVENT_PAUSE, EVENT_RESUME};
    use plxnative_modules::host::Release;
    use std::sync::mpsc;
    use std::time::Instant;

    /// A table smaller than version 1 (or NULL) is refused at the door and NO callback is made: it was never
    /// accepted, so there is no `on_stopped` to call.
    #[test]
    fn a_refused_table_makes_no_callback() {
        let w = World::new();
        let small = PlxHostApi { size: 8, ..table() };
        for api in [std::ptr::null(), &small as *const PlxHostApi] {
            assert_eq!(w.hosted.start(api, LAUNCH.as_ptr(), |_| Ok(()), |_, ()| unreachable!()), host_abi::E_ARGS);
        }
        assert_eq!(w.stopped(), []);
    }

    /// A launch string the host got wrong is a boot refusal reported through `on_stopped`, on the host
    /// thread, before `start` returns; the loop never starts, and the report stands for the whole process
    /// (a second start is refused with no second callback).
    #[test]
    fn a_malformed_launch_string_is_reported_through_on_stopped_on_the_host_thread() {
        let _w = World::new();
        for bad in [std::ptr::null(), c"{not json".as_ptr(), c"{}".as_ptr()] {
            let hosted: &'static Hosted = Box::leak(Box::new(Hosted::new()));
            seen(|s| s.stopped.clear());
            let api = table();
            let code = hosted.start(&api, bad, |_| -> Result<(), i32> { unreachable!("no window for a refused launch") }, |_, ()| unreachable!());
            assert_eq!(code, host_abi::E_LAUNCH);
            let got = seen(|s| s.stopped.clone());
            assert_eq!(got.len(), 1, "exactly one report");
            assert_eq!((got[0].0, got[0].1), (1, 1), "code 1, clean: nothing was started");
            assert_eq!(got[0].3, std::thread::current().id(), "on the host thread, inside start");
            assert_eq!(hosted.start(&api, LAUNCH.as_ptr(), |_| Ok(()), |_, ()| unreachable!()), host_abi::E_STATE);
            assert_eq!(seen(|s| s.stopped.len()), 1, "a second start adds no report");
        }
    }

    #[test]
    fn a_window_that_will_not_open_is_a_refusal_reported_once() {
        let w = World::new();
        let code = w.start(|_| -> Result<(), i32> { Err(host_abi::E_WINDOW) }, |_, ()| unreachable!("no loop"));
        assert_eq!(code, host_abi::E_WINDOW);
        assert_eq!(w.stopped(), [(1, 1)]);
        assert_eq!(w.hosted.push(Input::Pause), Err(host_abi::E_STATE), "nothing is running");
    }

    /// A plain value standing in for the window in the tests that check what the loop is handed.
    impl Made for u32 {
        fn unmake(self) {}
    }

    /// A `Made` that counts its release and remembers the thread.
    struct Tracked(std::sync::Arc<Mutex<Vec<std::thread::ThreadId>>>);

    impl Made for Tracked {
        fn unmake(self) {
            self.0.lock().unwrap().push(std::thread::current().id());
        }
    }

    /// **A loop thread that cannot be started releases the window on the host thread**, once, before the
    /// refusal; one that starts owns it and nothing is released by the host.
    #[test]
    fn a_loop_thread_that_cannot_start_returns_the_window_to_the_host_thread() {
        let released = std::sync::Arc::new(Mutex::new(Vec::new()));
        let ran = std::sync::Arc::new(AtomicBool::new(false));
        let flag = ran.clone();
        let started = give_to_loop(Tracked(released.clone()), |_dropped| false, move |_| flag.store(true, Ordering::SeqCst));
        assert!(!started);
        assert!(!ran.load(Ordering::SeqCst), "the body never ran");
        assert_eq!(*released.lock().unwrap(), [std::thread::current().id()], "released once, here");

        released.lock().unwrap().clear();
        let (tx, rx) = mpsc::channel();
        let started = give_to_loop(
            Tracked(released.clone()),
            |body| {
                std::thread::spawn(body);
                true
            },
            move |window| {
                tx.send(()).unwrap();
                drop(window);
            },
        );
        assert!(started);
        rx.recv_timeout(Duration::from_secs(5)).expect("the body got the window");
        assert!(released.lock().unwrap().is_empty(), "a started loop owns it; the host releases nothing");
    }

    /// A panic in a step on the new window releases it (on this thread) and is `E_WINDOW`; a step that
    /// returns keeps it.
    #[test]
    fn a_panic_on_the_new_window_releases_it_and_refuses() {
        let released = std::sync::Arc::new(Mutex::new(Vec::new()));
        let out = plx_base::eventlog::with_private_log(|| guarded(Tracked(released.clone()), |_| panic!("show")));
        assert!(matches!(out, Err(code) if code == host_abi::E_WINDOW));
        assert_eq!(*released.lock().unwrap(), [std::thread::current().id()]);
        released.lock().unwrap().clear();
        assert!(guarded(Tracked(released.clone()), |_| {}).is_ok());
        assert!(released.lock().unwrap().is_empty());
    }

    /// **A panic while bringing up is the header's `E_WINDOW` refusal**, with the callback it promises:
    /// `on_stopped(1, 1)` once on the host thread, and nothing runs.
    #[test]
    fn a_panic_in_bring_up_is_a_documented_refusal_with_its_callback() {
        let w = World::new();
        let code = plx_base::eventlog::with_private_log(|| {
            w.start(|_| -> Result<(), i32> { panic!("bring up") }, |_, ()| unreachable!("no loop"))
        });
        assert_eq!(code, host_abi::E_WINDOW);
        assert_eq!(w.stopped(), [(1, 1)]);
        assert_eq!(w.hosted.push(Input::Pause), Err(host_abi::E_STATE), "nothing is running");
    }

    /// The loop runs on a thread of its own, with the launch values and the window `bring_up` made, and a
    /// clean quit is reported once as `(code, clean = 1)`.
    #[test]
    fn the_loop_runs_on_its_own_thread_and_a_clean_end_is_reported_once() {
        let w = World::new();
        let host_thread = std::thread::current().id();
        let (tx, rx) = mpsc::channel();
        let code = w.start(
            |launch| {
                assert_eq!((launch.pms_host.as_str(), launch.pms_port), ("10.0.2.2", 32499));
                Ok(41u32)
            },
            move |launch, window| {
                tx.send((std::thread::current().id(), window, launch.state_dir.clone())).unwrap();
                complete()
            },
        );
        assert_eq!(code, host_abi::OK);
        let (loop_thread, window, state) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_ne!(loop_thread, host_thread);
        assert_eq!(window, 41);
        assert_eq!(state, std::path::PathBuf::from("/data"));
        assert_eq!(w.wait_stopped(1), [(0, 1)]);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(w.stopped(), [(0, 1)], "exactly once");
    }

    /// A loop that returns with a worker still inside firmware released nothing: `clean = 0`, whatever the
    /// callbacks did. A refused boot (also `Retained`) is the same.
    #[test]
    fn a_returning_loop_with_a_retained_worker_is_never_clean() {
        let mut w = World::new();
        assert_eq!(w.start(|_| Ok(()), |_, ()| Exit { code: 0, release: Release::Retained }), host_abi::OK);
        assert_eq!(w.wait_stopped(1), [(0, 0)]);
        w.reset();
        assert_eq!(w.start(|_| Ok(()), |_, ()| Exit::refused(7)), host_abi::OK);
        assert_eq!(w.wait_stopped(1), [(7, 0)]);
    }

    /// A loop that panics still ends in a report, never clean.
    #[test]
    fn a_panicking_loop_is_reported_unclean() {
        let w = World::new();
        assert_eq!(w.start(|_| Ok(()), |_, ()| -> Exit { panic!("the loop died") }), host_abi::OK);
        assert_eq!(w.wait_stopped(1), [(1, 0)]);
    }

    /// **Quit during idle**: the host stops an application that is sitting in its loop doing nothing. The
    /// loop's next `deliver` hands it a quit (once), it returns having released everything, and the report
    /// is `clean = 1`.
    #[test]
    fn a_stop_while_idle_quits_the_loop_and_reports_clean() {
        let w = World::new();
        assert_eq!(w.start(|_| Ok(()), loop_until_quit(w.hosted, complete())), host_abi::OK);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(w.stopped(), [], "nothing is reported while the application runs");
        w.hosted.stop(Duration::from_secs(30));
        assert_eq!(w.wait_stopped(1), [(0, 1)]);
        assert_eq!(w.hosted.push(Input::Pause), Err(host_abi::E_STATE), "no input after the stop");
    }

    /// A stop is delivered to the application as ONE quit, after the input queued before it.
    #[test]
    fn the_stop_is_one_quit_after_the_queued_input() {
        let w = World::new();
        w.hosted.running.store(true, Ordering::SeqCst); // a loop is "running": this test drives deliver itself
        w.hosted.push(Input::Key(Action::Ok, Edge::Down)).unwrap();
        w.hosted.gate.stop();
        let mut rec = Rec::default();
        w.hosted.deliver(&mut rec);
        w.hosted.deliver(&mut rec);
        assert_eq!(rec.seen, ["key Some(Ok) None Down", "quit"], "the key, then one quit, never two");
    }

    /// **A callback in flight at stop holds the clean report**: another thread (a worker's completion) is
    /// inside the host's `run_on_host`, which blocks here; the host asks for a stop and the loop returns
    /// having released everything. The report comes only after the callback returns, and then it is clean.
    #[test]
    fn a_callback_in_flight_at_stop_holds_the_clean_report_until_it_returns() {
        let w = World::new();
        assert_eq!(w.start(|_| Ok(()), loop_until_quit(w.hosted, complete())), host_abi::OK);
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        seen(|s| {
            s.entered = Some(entered_tx);
            s.block_run = Some(release_rx);
        });
        let hosted = w.hosted;
        let worker = std::thread::spawn(move || hosted.on_host(Box::new(|| {})));
        entered_rx.recv_timeout(Duration::from_secs(5)).expect("the callback is running");
        hosted.stop(Duration::from_secs(30));
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(w.stopped(), [], "the loop returned, but a callback is still running: nothing is reported");
        assert_eq!(hosted.gate.in_flight(), 1);
        release_tx.send(()).unwrap();
        assert_eq!(w.wait_stopped(1), [(0, 1)], "clean, and only after the callback returned");
        assert!(worker.join().unwrap(), "the admitted callback completed");
    }

    /// **A swap that stalls**: the loop does not return after the stop. The deadline reports `clean = 0`;
    /// when the loop wakes and returns, its own report is suppressed. One report, ever.
    #[test]
    fn a_stalled_loop_is_reported_unclean_by_the_deadline_and_the_late_return_is_suppressed() {
        let w = World::new();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        assert_eq!(
            w.start(|_| Ok(()), move |_, ()| {
                let _ = release_rx.recv_timeout(Duration::from_secs(10));
                complete()
            }),
            host_abi::OK
        );
        w.hosted.stop(Duration::from_millis(60));
        assert_eq!(w.wait_stopped(1), [(1, 0)], "the deadline: code 1, never clean");
        release_tx.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(w.stopped(), [(1, 0)], "the loop's late report was suppressed");
    }

    /// A loop that returns in time suppresses the deadline: the report is the loop's, once.
    #[test]
    fn a_loop_that_returns_in_time_suppresses_the_deadline() {
        let w = World::new();
        assert_eq!(w.start(|_| Ok(()), loop_until_quit(w.hosted, complete())), host_abi::OK);
        w.hosted.stop(Duration::from_millis(150));
        assert_eq!(w.wait_stopped(1), [(0, 1)]);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(w.stopped(), [(0, 1)], "the deadline woke to a delivered report and said nothing");
    }

    /// **Pause then stop**: the pause disarms the watchdog (after the application has been told), the
    /// resume arms it on the loop thread, and a stop after a pause reports with the watchdog disarmed (the
    /// loop thread is about to end, and its id with it).
    #[test]
    fn pause_then_stop_reports_with_the_watchdog_disarmed() {
        let w = World::new();
        let (tx, rx) = mpsc::channel();
        let hosted = w.hosted;
        assert_eq!(
            w.start(|_| Ok(()), move |_, ()| {
                let mut rec = Rec::default();
                let mut states = Vec::new();
                // pause, then resume, then pause again: armed state after each is what is asserted.
                for (phase, expected_len) in [(Input::Pause, 1usize), (Input::Resume, 2), (Input::Pause, 3)] {
                    hosted.push(phase).unwrap();
                    hosted.deliver(&mut rec);
                    assert_eq!(rec.seen.len(), expected_len, "{:?}", rec.seen);
                    states.push(plx_base::task::watchdog::is_armed());
                }
                tx.send((states, rec.seen.clone())).unwrap();
                while !rec.seen.iter().any(|e| e == "quit") {
                    hosted.deliver(&mut rec);
                    std::thread::sleep(Duration::from_millis(1));
                }
                complete()
            }),
            host_abi::OK
        );
        let (states, told) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(states, [false, true, false], "pause disarms, resume re-arms");
        assert_eq!(
            told,
            ["lifecycle DidBackground", "lifecycle DidForeground", "lifecycle DidBackground"]
        );
        hosted.stop(Duration::from_secs(30));
        w.wait_stopped(1);
        let armed_at_report = seen(|s| s.stopped[0].2);
        assert!(!armed_at_report, "the watchdog was disarmed when on_stopped ran");
    }

    /// **One transition is one event.** The application logs and works once per lifecycle edge it is sent
    /// (`run::host_background`, `host_foreground`), and the DID edge alone is a complete transition (`Lifecycle`'s
    /// doc: a platform that sends only DID background is enough on its own), so a pause is ONE `DidBackground`
    /// and a resume ONE `DidForeground`. A host that reports the same state twice (the .NET `OnPause` and a
    /// window event, a resume at launch before any pause) makes no second transition: a pause while paused and a
    /// resume while running are dropped, and the watchdog is only touched by a real one.
    #[test]
    fn one_transition_is_one_event_and_a_repeat_is_not_a_transition() {
        let w = World::new();
        w.hosted.running.store(true, Ordering::SeqCst);
        plx_base::task::watchdog::arm();
        let mut rec = Rec::default();
        // A resume before any pause (the host's resume at launch): nothing.
        w.hosted.push(Input::Resume).unwrap();
        w.hosted.deliver(&mut rec);
        assert_eq!(rec.seen, Vec::<String>::new());
        assert!(plx_base::task::watchdog::is_armed());
        // Two pauses: one background edge, watchdog disarmed.
        w.hosted.push(Input::Pause).unwrap();
        w.hosted.push(Input::Pause).unwrap();
        w.hosted.deliver(&mut rec);
        assert_eq!(rec.seen, ["lifecycle DidBackground"]);
        assert!(!plx_base::task::watchdog::is_armed());
        // Two resumes: one foreground edge, watchdog armed again.
        w.hosted.push(Input::Resume).unwrap();
        w.hosted.push(Input::Resume).unwrap();
        w.hosted.deliver(&mut rec);
        assert_eq!(rec.seen, ["lifecycle DidBackground", "lifecycle DidForeground"]);
        assert!(plx_base::task::watchdog::is_armed());
    }

    /// `post` is a bounded push that never blocks: before a start and after a stop it answers E_STATE, a bad
    /// event E_ARGS, a full queue E_FULL (the event is dropped and counted), and the queued events reach
    /// the application in order as lifecycle edges.
    #[test]
    fn post_is_bounded_ordered_and_refused_outside_a_run() {
        let w = World::new();
        let pause = PlxHostEvent { size: host_abi::EVENT_V1_SIZE, kind: EVENT_PAUSE };
        assert_eq!(w.hosted.post(&pause), host_abi::E_STATE, "not started");
        let hosted = w.hosted;
        let (go_tx, go_rx) = mpsc::channel::<()>();
        assert_eq!(
            w.start(|_| Ok(()), move |_, ()| {
                let _ = go_rx.recv_timeout(Duration::from_secs(10));
                complete()
            }),
            host_abi::OK
        );
        assert_eq!(hosted.post(std::ptr::null()), host_abi::E_ARGS);
        assert_eq!(hosted.post(&PlxHostEvent { size: host_abi::EVENT_V1_SIZE, kind: 99 }), host_abi::E_ARGS);
        assert_eq!(hosted.post(&pause), host_abi::OK);
        assert_eq!(hosted.post(&PlxHostEvent { size: host_abi::EVENT_V1_SIZE, kind: EVENT_RESUME }), host_abi::OK);
        let started = Instant::now();
        let mut refused = 0;
        for _ in 0..(QUEUE_LIMIT * 2) {
            if hosted.post(&pause) == host_abi::E_FULL {
                refused += 1;
            }
        }
        assert!(started.elapsed() < Duration::from_secs(2), "a full queue refuses at once");
        assert_eq!(refused, QUEUE_LIMIT * 2 - (QUEUE_LIMIT - 2), "everything past the bound was refused");
        assert_eq!(hosted.dropped(), refused);
        let mut rec = Rec::default();
        hosted.deliver(&mut rec);
        assert_eq!(&rec.seen[..2], ["lifecycle DidBackground", "lifecycle DidForeground"]);
        hosted.stop(Duration::from_secs(30));
        assert_eq!(hosted.post(&pause), host_abi::E_STATE, "stopping");
        go_tx.send(()).unwrap();
        w.wait_stopped(1);
    }

    /// Work for the host thread leaves through `run_on_host` and runs there when the host calls it; once a
    /// stop was requested the gate refuses it and the job never runs; a host that refuses the request
    /// gets the job dropped unrun.
    #[test]
    fn host_thread_work_is_queued_through_the_callback_and_refused_after_a_stop() {
        let w = World::new();
        let hosted = w.hosted;
        let (go_tx, go_rx) = mpsc::channel::<()>();
        assert_eq!(
            w.start(|_| Ok(()), move |_, ()| {
                let _ = go_rx.recv_timeout(Duration::from_secs(10));
                complete()
            }),
            host_abi::OK
        );
        let ran = std::sync::Arc::new(AtomicUsize::new(0));
        let counted = {
            let ran = ran.clone();
            Box::new(move || {
                ran.fetch_add(1, Ordering::SeqCst);
            })
        };
        assert!(hosted.on_host(counted), "accepted");
        assert_eq!(ran.load(Ordering::SeqCst), 0, "the loop's call only queued it");
        let (task, arg) = seen(|s| s.tasks.pop()).expect("the host was handed a task");
        // The host's main thread runs it.
        let host = std::thread::spawn(move || unsafe { task(arg as *mut c_void) });
        host.join().unwrap();
        assert_eq!(ran.load(Ordering::SeqCst), 1, "run once, by the host");

        seen(|s| s.run_answer = -1);
        let refused = {
            let ran = ran.clone();
            Box::new(move || {
                ran.fetch_add(10, Ordering::SeqCst);
            })
        };
        assert!(!hosted.on_host(refused), "the host refused it");
        assert_eq!(ran.load(Ordering::SeqCst), 1, "and the job was dropped unrun");

        seen(|s| s.run_answer = 0);
        hosted.stop(Duration::from_secs(30));
        let late = {
            let ran = ran.clone();
            Box::new(move || {
                ran.fetch_add(100, Ordering::SeqCst);
            })
        };
        assert!(!hosted.on_host(late), "a stop was requested: the gate refuses it");
        assert!(seen(|s| s.tasks.is_empty()), "the host was never asked");
        go_tx.send(()).unwrap();
        w.wait_stopped(1);
    }

    /// A key's action reaches the application alone (no raw pair), with its edge, in the order queued
    /// around the lifecycle events.
    #[test]
    fn keys_reach_the_application_as_actions_in_order() {
        let w = World::new();
        w.hosted.running.store(true, Ordering::SeqCst);
        for input in [Input::Key(Action::Right, Edge::Down), Input::Key(Action::Right, Edge::Up), Input::Pause, Input::Key(Action::Back, Edge::Down)] {
            w.hosted.push(input).unwrap();
        }
        let mut rec = Rec::default();
        w.hosted.deliver(&mut rec);
        assert_eq!(
            rec.seen,
            ["key Some(Right) None Down", "key Some(Right) None Up", "lifecycle DidBackground", "key Some(Back) None Down"]
        );
        plx_base::task::watchdog::arm();
    }
}
