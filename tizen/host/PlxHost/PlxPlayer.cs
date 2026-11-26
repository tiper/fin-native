// The host's media player: what the table's `player_open` and `player_verb` do (rust-modules/platform-tizen/include/plx_host.h, PLAYER).
//
// THE SHAPE, from the header's rules.
//  * A request RETURNS AT ONCE. OpenNative and VerbNative run on the library's loop thread; they validate, copy, enqueue and return. Everything
//    that touches the native player runs on ONE video thread (`plx-player`) that consumes the queue in order, so no player call ever runs on the
//    loop thread or on the Ecore main thread (the one exception, binding the display, is dispatched to the main thread and waited for with a
//    bound: see BindDisplay).
//  * Results are EVENTS, posted with plx_host_post from whatever thread has them: the video thread, the position poller or a native callback.
//    Every event carries the epoch the library minted for the open, and every native callback is registered with that epoch as its `user_data`,
//    so a late callback names its own player; the library drops an event whose epoch is no longer live (the nuvio session-counter rule, Video.cs).
//  * Position is POLLED here, about every 250 ms while playing, on a thread of its own and off the request queue: the native read can take
//    seconds on some firmware, and a read that has not returned after StallMs posts PLAYER_STALL once. There is no synchronous position call.
//
// MEASURED on the emulator 2026-11-22 (TV 10.0 x86_64; a probe on a scratch branch, a local clip): prepared, buffering and completed callbacks fire
// on ONE native thread per player instance that is not the main thread (a new thread for each player), so they only post events and never touch
// managed UI state; a player with no display does not advance; the completed callback fires with the native state still PLAYING. The thread the
// callbacks use on a Samsung set is UNVERIFIED, and the design does not depend on it.
//
// WHAT THE HOST NEVER DOES HERE: log the uri (it carries the Plex token), destroy a player a position read is inside (the handle is guarded;
// a stuck read leaks the handle and the process exit is the teardown), or leave a player behind: CLOSE always ends in exactly one PLAYER_CLOSED.

using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

namespace PlxHost
{
    internal sealed class PlxPlayer
    {
        internal static readonly PlxPlayer Instance = new PlxPlayer();

        private const int QueueLimit = 64;
        private const int PollMs = 250;
        private const int StallMs = 3000;
        private const int BindWaitMs = 3000;
        private const int CloseWaitMs = 3000;
        // A native seek whose completion callback has not fired within this long is given up on: its command ids get SEEK_DONE with the expiry
        // marker (b = -2: it MAY still land) and the next held seek is sent (plx_host.h, PLAYER_SEEK_DONE).
        private const int SeekWaitMs = 10000;
        // PLAYER_SEEK_DONE's b when the seek did not land, and when it did not complete in time (plx_host.h).
        private const long SeekRefused = -1;
        private const long SeekExpired = -2;

        // The window the library made is the first Ecore_Wl2 window (id 1) on the connected display: the library makes exactly one. Measured on
        // the emulator to be the very pointer the library logs ("ecore: window_new(...) -> 0x..."); BindDisplay logs the pointer it found, so the
        // two lines can be compared in the UDP log. If a later change makes a second window first, this id is the line to revisit.
        private const uint LibraryWindowId = 1;

        // One player's host-side state. Video thread owns everything except what is marked.
        private sealed class Session
        {
            internal readonly uint Epoch;
            internal IntPtr Handle;
            internal bool Prepared;
            internal bool PlayPending;
            internal bool OpenFailed; // an open step failed: the player will never prepare, so a seek is answered at once
            internal volatile bool Playing; // read by the poller
            internal volatile bool Closing; // read by the poller and the main-thread bind
            // Set when the HOST paused (or held back the start of) this player because the app went to the background (PauseForHome), cleared by
            // ResumeAfterHome and by any request of the library's own that decides play or pause. Video thread only.
            internal bool HostPaused;
            internal bool StallSent; // poller only
            // At most ONE native seek is waited on (nothing measured says two overlap safely): `SeekNow` is it, `SeekLater` holds the newest request
            // that arrived meanwhile (or before PREPARED). A seek is { target ms, the command ids it answers }; a request that replaces a held one folds
            // the older command ids into it, and every one of them gets its PLAYER_SEEK_DONE (with the final target) when that seek lands. After
            // SeekWaitMs without a completion the seek is expired (its ids get b = -2) and the next held seek is sent while the expired one may
            // still be pending in the native player: a late completion is credited to the NEXT seek's ids (the callback carries no token).
            // Every command id is answered before its epoch ends: by landing, by refusal or expiry, or by Release / a failed open (b = -1).
            internal Seek SeekNow;
            internal Seek SeekLater;
            // An audio track asked for before PREPARED (the native call needs a prepared player): applied in OnPrepared before PREPARED is
            // posted. The newest index wins and the older command ids are folded into it; every one gets its PLAYER_TRACK.
            internal Track TrackLater;
            // The audio tracks as read at PREPARED (ReadTracks); null before it. Video thread only.
            internal AudioTracks Tracks;
            // Held by anything that calls into the native player on this handle off the video thread (position reads, the display bind). The
            // video thread takes it, with a bound, before it destroys the handle.
            internal readonly object HandleGate = new object();

            internal Session(uint epoch) { Epoch = epoch; }
        }

        private sealed class Track
        {
            internal int Index;
            internal readonly List<long> Commands = new List<long>();
        }

        // PLAYER_TRACK's b when the track was not selected (plx_host.h, PLAYER_TRACK).
        private const long TrackRefused = -1;

        private sealed class Seek
        {
            internal int TargetMs;
            internal readonly List<long> Commands = new List<long>();
            internal Timer Deadline; // armed while this seek is in flight
        }

        // A job and the epoch it works for, so a job that throws can be answered with that epoch's PLAYER_ERROR.
        private struct Job
        {
            internal uint Epoch;
            internal Action Run;
        }

        // Unbounded for what the player's own callbacks hand over (prepared, completed, seek-done: dropping one would lose the player's story); the
        // library's requests are bounded by QueueLimit at the door (TryEnqueue).
        private readonly BlockingCollection<Job> queue = new BlockingCollection<Job>();
        private readonly object startGate = new object();
        private Thread videoThread;
        private Thread pollThread;
        private volatile bool shuttingDown;
        private volatile bool threadsFailed;

        private Session current; // video thread
        private volatile Session published; // the poller's view of `current`
        private long latestEpoch; // the epoch of the last open; written on the loop thread, read there (Interlocked)

        // The native callbacks are static, so nothing but the end of the process lets the runtime collect their thunks while a player holds them.
        private static readonly MediaPlayerNative.VoidCallback PreparedCb = user => Instance.Enqueue(EpochOf(user), s => Instance.OnPrepared(s));
        private static readonly MediaPlayerNative.VoidCallback CompletedCb = user => Instance.Enqueue(EpochOf(user), s => Instance.OnCompleted(s));
        private static readonly MediaPlayerNative.VoidCallback SeekDoneCb = user => Instance.Enqueue(EpochOf(user), s => Instance.OnSeekDone(s));
        private static readonly MediaPlayerNative.IntCallback ErrorCb =
            (code, user) => PlxApp.PostPlayer((uint)Native.PLX_HOST_EVENT_PLAYER_ERROR, EpochOf(user), code, 0);
        private static readonly MediaPlayerNative.IntCallback BufferingCb =
            (percent, user) => PlxApp.PostPlayer((uint)Native.PLX_HOST_EVENT_PLAYER_BUFFERING, EpochOf(user), percent, 0);
        private static readonly MediaPlayerNative.IntCallback InterruptedCb =
            (code, user) => PlxApp.PostPlayer((uint)Native.PLX_HOST_EVENT_PLAYER_INTERRUPTED, EpochOf(user), code, 0);

        private static IntPtr UserOf(uint epoch) { return new IntPtr(unchecked((int)epoch)); }
        private static uint EpochOf(IntPtr user) { return unchecked((uint)user.ToInt64()); }

        // Whether the device has the media player library at all. Called once at start; false leaves the table's player pointers NULL (the
        // library then plays nothing and says so), and `note` is the line for the log.
        internal static bool Probe(out string note)
        {
            IntPtr h = Dl.dlopen(MediaPlayerNative.MediaLib, Dl.RTLD_NOW);
            if (h != IntPtr.Zero)
            {
                note = "media player: " + MediaPlayerNative.MediaLib + " opened; the table offers player_open and player_verb";
                return true;
            }
            IntPtr err = Dl.dlerror();
            note = "media player: " + MediaPlayerNative.MediaLib + " did not open (" + (err == IntPtr.Zero ? "no dlerror" : Marshal.PtrToStringAnsi(err))
                + "); the table leaves the player pointers NULL, so the library plays nothing";
            return false;
        }

        // Logs the library's window pointer once, after plx_host_start returned (the library makes its window during start), so a smoke run can
        // compare it with libplx's `ecore: window_new(...) -> 0x...` line without opening a player. Main thread (Ecore). Never throws.
        internal static void LogWindow()
        {
            try
            {
                IntPtr display = MediaPlayerNative.ecore_wl2_connected_display_get(IntPtr.Zero);
                IntPtr window = display == IntPtr.Zero ? IntPtr.Zero : MediaPlayerNative.ecore_wl2_display_window_find(display, LibraryWindowId);
                PlxApp.Log("player: at start, ecore window id " + LibraryWindowId + " -> 0x" + window.ToInt64().ToString("x")
                    + " (compare with the library's `ecore: window_new(...) -> ` line; 0x0 = not made yet)");
            }
            catch (Exception e)
            {
                PlxApp.Log("player: the window lookup at start threw " + e.GetType().Name + ": " + e.Message);
            }
        }

        // ---- the table's two requests (loop thread): validate, enqueue, return --------------------------------------------------------------

        // PlxPlayerOpen.
        internal static int OpenNative(uint epoch, IntPtr uri, int x, int y, int w, int h)
        {
            try
            {
                if (epoch == 0 || uri == IntPtr.Zero) return Native.PLX_HOST_PLAYER_E_ARGS;
                string text = ReadUtf8(uri);
                if (text.Length == 0) return Native.PLX_HOST_PLAYER_E_ARGS;
                return Instance.Open(epoch, text, x, y, w, h);
            }
            catch (Exception e)
            {
                PlxApp.Log("player_open: " + e.GetType().Name + ": " + e.Message);
                return Native.PLX_HOST_PLAYER_E_UNAVAILABLE;
            }
        }

        // PlxPlayerVerb.
        internal static int VerbNative(uint epoch, uint verb, long arg0, long arg1)
        {
            try
            {
                return Instance.Verb(epoch, verb, arg0, arg1);
            }
            catch (Exception e)
            {
                PlxApp.Log("player_verb: " + e.GetType().Name + ": " + e.Message);
                return Native.PLX_HOST_PLAYER_E_UNAVAILABLE;
            }
        }

        // Marshal.PtrToStringUTF8 is not in the netstandard2.0 surface the tizen80 TFM compiles against: read the bytes.
        private static string ReadUtf8(IntPtr p)
        {
            int n = 0;
            while (Marshal.ReadByte(p, n) != 0) n++;
            var bytes = new byte[n];
            Marshal.Copy(p, bytes, 0, n);
            return Encoding.UTF8.GetString(bytes);
        }

        private int Open(uint epoch, string uri, int x, int y, int w, int h)
        {
            if (!EnsureThreads()) return Native.PLX_HOST_PLAYER_E_UNAVAILABLE;
            long previous = Interlocked.Exchange(ref latestEpoch, epoch);
            int rc = TryEnqueue(epoch, () => DoOpen(epoch, uri, x, y, w, h));
            if (rc != Native.PLX_HOST_PLAYER_OK) Interlocked.Exchange(ref latestEpoch, previous); // the open never happened: the old player keeps its verbs
            return rc;
        }

        private int Verb(uint epoch, uint verb, long arg0, long arg1)
        {
            if (!EnsureThreads()) return Native.PLX_HOST_PLAYER_E_UNAVAILABLE;
            switch ((int)verb)
            {
                case Native.PLX_HOST_PLAYER_VERB_CLOSE:
                {
                    // Always accepted: it ends in one PLAYER_CLOSED whether or not a player exists.
                    int rc = TryEnqueue(epoch, () => DoClose(epoch));
                    // An accepted close retires the epoch AT THE DOOR (plx_host.h, PLAYER): every later PLAY, PAUSE or SEEK for it is E_STALE here
                    // and never reaches the queue. Only if it is still the latest: a newer open's epoch is not touched.
                    if (rc == Native.PLX_HOST_PLAYER_OK) Interlocked.CompareExchange(ref latestEpoch, 0, epoch);
                    return rc;
                }
                case Native.PLX_HOST_PLAYER_VERB_PLAY:
                case Native.PLX_HOST_PLAYER_VERB_PAUSE:
                case Native.PLX_HOST_PLAYER_VERB_SEEK:
                case Native.PLX_HOST_PLAYER_VERB_AUDIO_TRACK:
                    if (Interlocked.Read(ref latestEpoch) != epoch) return Native.PLX_HOST_PLAYER_E_STALE;
                    return TryEnqueue(epoch, () => DoVerb(epoch, (int)verb, arg0, arg1));
                default:
                    return Native.PLX_HOST_PLAYER_E_ARGS;
            }
        }

        // The library's requests: refused as BUSY when QueueLimit jobs wait.
        private int TryEnqueue(uint epoch, Action job)
        {
            try
            {
                if (queue.Count >= QueueLimit) return Native.PLX_HOST_PLAYER_E_BUSY;
                queue.Add(new Job { Epoch = epoch, Run = job });
                return Native.PLX_HOST_PLAYER_OK;
            }
            catch (InvalidOperationException)
            {
                return Native.PLX_HOST_PLAYER_E_UNAVAILABLE; // shut down
            }
        }

        // A native callback's hand-off to the video thread: never blocks the callback thread and never drops (the queue is unbounded for these).
        private void Enqueue(uint epoch, Action<Session> job)
        {
            try
            {
                queue.Add(new Job
                {
                    Epoch = epoch,
                    Run = () =>
                    {
                        Session s = current;
                        if (s != null && s.Epoch == epoch) job(s);
                    },
                });
            }
            catch (InvalidOperationException)
            {
                PlxApp.Log("player: a callback for epoch " + epoch + " arrived after the shutdown");
            }
        }

        private bool EnsureThreads()
        {
            if (threadsFailed) return false;
            lock (startGate)
            {
                // Shutdown set this under the same lock: after it no thread starts and no request is taken, so a request that was already past the
                // library's gate can neither create a video thread that nothing closes nor reach a native player that Shutdown has already swept.
                if (shuttingDown) return false;
                if (videoThread != null) return true;
                try
                {
                    videoThread = new Thread(VideoLoop) { IsBackground = true, Name = "plx-player" };
                    pollThread = new Thread(PollLoop) { IsBackground = true, Name = "plx-player-poll" };
                    videoThread.Start();
                    pollThread.Start();
                    return true;
                }
                catch (Exception e)
                {
                    threadsFailed = true;
                    PlxApp.Log("player: the video thread could not be started: " + e.GetType().Name + ": " + e.Message);
                    return false;
                }
            }
        }

        // ---- the video thread ---------------------------------------------------------------------------------------------------------------

        private void VideoLoop()
        {
            foreach (Job job in queue.GetConsumingEnumerable())
            {
                try { job.Run(); }
                catch (Exception e)
                {
                    // plx_host.h: a failure after the request was queued arrives as PLAYER_ERROR (an entry point missing on some firmware, say).
                    PlxApp.Log("player: epoch " + job.Epoch + ": a request threw " + e.GetType().Name + ": " + e.Message);
                    if (job.Epoch != 0) Post((uint)Native.PLX_HOST_EVENT_PLAYER_ERROR, job.Epoch, Native.PLX_HOST_PLAYER_FAIL_BASE + 3, 0);
                }
            }
        }

        private static void Post(uint kind, uint epoch, long a, long b) { PlxApp.PostPlayer(kind, epoch, a, b); }

        // PLAYER_ERROR with the native player's own code in `a`.
        private static void Fail(Session s, string step, int rc)
        {
            PlxApp.Log("player: epoch " + s.Epoch + ": " + step + " returned " + rc);
            Post((uint)Native.PLX_HOST_EVENT_PLAYER_ERROR, s.Epoch, rc, 0);
        }

        // PLAYER_ERROR for a failure that is the HOST's, not the native player's: `a` is PLX_HOST_PLAYER_FAIL_BASE + n, a value no native code takes.
        private static void FailHost(Session s, string step, int n)
        {
            PlxApp.Log("player: epoch " + s.Epoch + ": " + step + " (host failure " + n + ")");
            Post((uint)Native.PLX_HOST_EVENT_PLAYER_ERROR, s.Epoch, Native.PLX_HOST_PLAYER_FAIL_BASE + n, 0);
        }

        private void DoOpen(uint epoch, string uri, int x, int y, int w, int h)
        {
            // An open queued before Shutdown's CompleteAdding but after its close job would create a native player nothing closes: decline it. The
            // epoch ends with its one PLAYER_CLOSED (no player was made), which the library still hears while its loop runs.
            if (shuttingDown)
            {
                PlxApp.Log("player: epoch " + epoch + ": the open was declined, the host is shutting down");
                Post((uint)Native.PLX_HOST_EVENT_PLAYER_CLOSED, epoch, 0, 0);
                return;
            }
            OpenSteps(epoch, uri, x, y, w, h);
        }

        private void OpenSteps(uint epoch, string uri, int x, int y, int w, int h)
        {
            if (current != null) DoClose(current.Epoch); // an open while another is open closes that one first
            var s = new Session(epoch);
            current = s;
            published = s;
            bool opened = false;
            try
            {
                opened = OpenPlayer(s, uri, x, y, w, h);
            }
            finally
            {
                if (!opened) // a failed step or a throw (VideoLoop reports it): verbs that follow are answered at once, none is held
                {
                    s.OpenFailed = true;
                    AnswerSeeks(s);
                    AnswerTracks(s);
                }
            }
        }

        // The open's steps; false after the first failure (already posted as PLAYER_ERROR).
        private bool OpenPlayer(Session s, string uri, int x, int y, int w, int h)
        {
            IntPtr user = UserOf(s.Epoch);
            int rc = MediaPlayerNative.player_create(out s.Handle);
            if (rc != 0 || s.Handle == IntPtr.Zero) { s.Handle = IntPtr.Zero; Fail(s, "player_create", rc); return false; }
            rc = MediaPlayerNative.player_set_uri(s.Handle, uri);
            if (rc != 0) { Fail(s, "player_set_uri", rc); return false; }
            // The display is bound BEFORE prepare: measured on the emulator, a player with no display does not advance.
            rc = BindDisplay(s, x, y, w, h);
            if (rc >= Native.PLX_HOST_PLAYER_FAIL_BASE) { FailHost(s, "bind the display", rc - Native.PLX_HOST_PLAYER_FAIL_BASE); return false; }
            if (rc != 0) { Fail(s, "player_set_ecore_wl_display", rc); return false; }
            rc = MediaPlayerNative.player_set_display_mode(s.Handle, MediaPlayerNative.DISPLAY_MODE_LETTER_BOX);
            if (rc != 0) { Fail(s, "player_set_display_mode", rc); return false; }
            MediaPlayerNative.player_set_completed_cb(s.Handle, CompletedCb, user);
            MediaPlayerNative.player_set_error_cb(s.Handle, ErrorCb, user);
            MediaPlayerNative.player_set_buffering_cb(s.Handle, BufferingCb, user);
            MediaPlayerNative.player_set_interrupted_cb(s.Handle, InterruptedCb, user);
            rc = MediaPlayerNative.player_prepare_async(s.Handle, PreparedCb, user);
            if (rc != 0) { Fail(s, "player_prepare_async", rc); return false; }
            return true;
        }

        // Find the library's window and bind the player to it as an OVERLAY. Ecore belongs to the main thread (plx_host.h, THREADS), so the two
        // Ecore calls and the bind run there, and this thread waits for them with a bound. UNVERIFIED which thread the native library needs.
        private int BindDisplay(Session s, int x, int y, int w, int h)
        {
            int rc = -1;
            bool ran = PlxApp.RunOnMain(() =>
            {
                try
                {
                    lock (s.HandleGate)
                    {
                        if (s.Closing || s.Handle == IntPtr.Zero) return;
                        IntPtr display = MediaPlayerNative.ecore_wl2_connected_display_get(IntPtr.Zero);
                        IntPtr window = display == IntPtr.Zero ? IntPtr.Zero : MediaPlayerNative.ecore_wl2_display_window_find(display, LibraryWindowId);
                        PlxApp.Log("player: ecore window id " + LibraryWindowId + " -> 0x" + window.ToInt64().ToString("x")
                            + " (compare with the library's `ecore: window_new(...) -> ` line)");
                        if (window == IntPtr.Zero) { rc = Native.PLX_HOST_PLAYER_FAIL_BASE + 1; return; }
                        rc = MediaPlayerNative.player_set_ecore_wl_display(s.Handle, MediaPlayerNative.DISPLAY_TYPE_OVERLAY, window, x, y, w, h);
                    }
                }
                catch (Exception e)
                {
                    // An entry point missing on some firmware, say: a throw is the host's failure (+3), never mistaken for a native code.
                    PlxApp.Log("player: the display bind threw " + e.GetType().Name + ": " + e.Message);
                    rc = Native.PLX_HOST_PLAYER_FAIL_BASE + 3;
                }
            }, BindWaitMs);
            if (!ran)
            {
                PlxApp.Log("player: binding the display did not finish within " + BindWaitMs + " ms on the main thread");
                return Native.PLX_HOST_PLAYER_FAIL_BASE + 2;
            }
            return rc;
        }

        private void DoVerb(uint epoch, int verb, long arg0, long arg1)
        {
            Session s = current;
            if (s == null || s.Epoch != epoch) return;
            if (s.Handle == IntPtr.Zero || s.Closing || s.OpenFailed)
            {
                // A dead player still owes its seek or track an answer (the library waits for every command id).
                if (verb == Native.PLX_HOST_PLAYER_VERB_SEEK) Post((uint)Native.PLX_HOST_EVENT_PLAYER_SEEK_DONE, s.Epoch, arg1, SeekRefused);
                if (verb == Native.PLX_HOST_PLAYER_VERB_AUDIO_TRACK) Post((uint)Native.PLX_HOST_EVENT_PLAYER_TRACK, s.Epoch, arg1, TrackRefused);
                return;
            }
            switch (verb)
            {
                case Native.PLX_HOST_PLAYER_VERB_PLAY: DoPlay(s); break;
                case Native.PLX_HOST_PLAYER_VERB_PAUSE: DoPause(s); break;
                case Native.PLX_HOST_PLAYER_VERB_SEEK: DoSeek(s, arg0, arg1); break;
                case Native.PLX_HOST_PLAYER_VERB_AUDIO_TRACK: DoTrack(s, arg0, arg1); break;
            }
        }

        // An audio track: held until PREPARED (the native call needs a prepared player), then applied at once.
        private void DoTrack(Session s, long index, long command)
        {
            if (s.TrackLater == null) s.TrackLater = new Track();
            s.TrackLater.Index = (int)Math.Max(-1, Math.Min(index, int.MaxValue));
            s.TrackLater.Commands.Add(command);
            ApplyHeldTrack(s);
        }

        // Select the held track and answer its command ids with the index now selected, or TrackRefused. A refusal is not a PLAYER_ERROR: the
        // player keeps the track it had. It reads nothing of its own: the count and the current index are the PREPARED-time snapshot (ReadTracks),
        // kept current by the selections made here.
        private static void ApplyHeldTrack(Session s)
        {
            if (!s.Prepared || s.TrackLater == null) return;
            Track track = s.TrackLater;
            s.TrackLater = null;
            long answer = TrackRefused;
            AudioTracks tracks = s.Tracks ?? new AudioTracks();
            if (track.Index < 0)
            {
                PlxApp.Log("player: epoch " + s.Epoch + ": audio track " + track.Index + " is no index");
            }
            else if (tracks.RcCount == 0 && tracks.Count == 0)
            {
                PlxApp.Log("player: epoch " + s.Epoch + ": audio track " + track.Index + " asked of a source with no audio track");
            }
            else if (tracks.RcCurrent == 0 && tracks.Current == track.Index)
            {
                answer = track.Index; // already the current track: nothing to call
            }
            else if (track.Index == 0 && tracks.RcCount == 0 && tracks.Count == 1)
            {
                answer = 0; // the only track is the one the player plays, whatever it says of the current index
            }
            else if (tracks.RcCount == 0 && track.Index >= tracks.Count)
            {
                PlxApp.Log("player: epoch " + s.Epoch + ": audio track " + track.Index + " is not one of the " + tracks.Count + " tracks");
            }
            else
            {
                try
                {
                    int rc = MediaPlayerNative.player_select_track(s.Handle, MediaPlayerNative.STREAM_TYPE_AUDIO, track.Index);
                    if (rc == 0) { answer = track.Index; tracks.Current = track.Index; tracks.RcCurrent = 0; }
                    else PlxApp.Log("player: epoch " + s.Epoch + ": player_select_track(audio, " + track.Index + ") returned " + rc);
                }
                catch (Exception e)
                {
                    // An entry point missing on some firmware is a refused track, not a failed player (plx_host.h, PLAYER_TRACK).
                    PlxApp.Log("player: epoch " + s.Epoch + ": player_select_track threw " + e.GetType().Name + ": " + e.Message);
                }
            }
            PlxApp.Log("player: epoch " + s.Epoch + ": audio track " + track.Index + " -> " + answer);
            foreach (long command in track.Commands) Post((uint)Native.PLX_HOST_EVENT_PLAYER_TRACK, s.Epoch, command, answer);
        }

        // Answers a held track's command ids with TrackRefused and forgets it: the epoch is ending or never prepared.
        private static void AnswerTracks(Session s)
        {
            Track held = s.TrackLater;
            s.TrackLater = null;
            if (held == null) return;
            foreach (long command in held.Commands) Post((uint)Native.PLX_HOST_EVENT_PLAYER_TRACK, s.Epoch, command, TrackRefused);
        }

        // A native call that threw (an entry point missing on some firmware) reads as a failed call, so a play is never failed by a read.
        private const int NativeThrew = int.MinValue;

        // The most native audio tracks whose languages are read and reported (plx_host.h, PLAYER_AUDIO_LANGUAGE).
        private const int LanguageReports = Native.PLX_HOST_AUDIO_LANGUAGE_REPORTS;

        // The player's audio tracks as read ONCE at PREPARED: the count, the current index and each track's language code in the player's own
        // order (null = none given). The language reads are what the library's track choice needs; the log line is built from the same read.
        private sealed class AudioTracks
        {
            internal int RcCount = NativeThrew, Count;
            internal int RcCurrent = NativeThrew, Current = -1;
            internal readonly List<string> Languages = new List<string>();
            internal readonly List<string> Notes = new List<string>(); // for the log line: why a language is missing
        }

        private static AudioTracks ReadTracks(Session s)
        {
            var tracks = new AudioTracks();
            try { tracks.RcCount = MediaPlayerNative.player_get_track_count(s.Handle, MediaPlayerNative.STREAM_TYPE_AUDIO, out tracks.Count); }
            catch (Exception e) { PlxApp.Log("player: epoch " + s.Epoch + ": player_get_track_count threw " + e.GetType().Name); }
            if (tracks.RcCount != 0) tracks.Count = 0;
            try { tracks.RcCurrent = MediaPlayerNative.player_get_current_track(s.Handle, MediaPlayerNative.STREAM_TYPE_AUDIO, out tracks.Current); }
            catch (Exception e) { PlxApp.Log("player: epoch " + s.Epoch + ": player_get_current_track threw " + e.GetType().Name); }
            for (int i = 0; i < tracks.Count && i < LanguageReports; i++)
            {
                // The string is the library's (malloc'd) only when the call returned 0: only then is it read and freed.
                IntPtr code = IntPtr.Zero;
                string lang = null, note = null;
                int rcLang;
                try { rcLang = MediaPlayerNative.player_get_track_language_code(s.Handle, MediaPlayerNative.STREAM_TYPE_AUDIO, i, out code); }
                catch (Exception e) { rcLang = NativeThrew; note = "threw " + e.GetType().Name; }
                if (rcLang == 0 && code != IntPtr.Zero)
                {
                    try { lang = ReadUtf8(code); } finally { Marshal.FreeHGlobal(code); }
                }
                else if (rcLang != NativeThrew) note = "rc " + rcLang;
                tracks.Languages.Add(lang);
                tracks.Notes.Add(note);
            }
            return tracks;
        }

        // A language code as PLAYER_AUDIO_LANGUAGE carries it: the primary subtag's two or three ASCII letters, the first in the lowest byte; 0
        // for anything else, which the library reads as unknown.
        private static long LanguageWord(string code)
        {
            if (code == null) return 0;
            string primary = code.Trim().Split('-', '_')[0];
            if (primary.Length < 2 || primary.Length > 3) return 0;
            long word = 0;
            for (int i = 0; i < primary.Length; i++)
            {
                char c = primary[i];
                if (!((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z'))) return 0;
                word |= (long)c << (8 * i);
            }
            return word;
        }

        // One line with the native audio tracks in the player's own order (count, current, each one's language), so a retail log can be compared
        // with the title's audio list in Plex. Built from the PREPARED-time read; it reads nothing.
        private static void LogTracks(Session s, AudioTracks tracks)
        {
            if (tracks.RcCount != 0) { PlxApp.Log("player: epoch " + s.Epoch + ": player_get_track_count(audio) returned " + tracks.RcCount); return; }
            var languages = new StringBuilder();
            for (int i = 0; i < tracks.Languages.Count; i++)
            {
                if (i > 0) languages.Append(',');
                languages.Append(i).Append('=').Append(tracks.Languages[i] ?? tracks.Notes[i] ?? "?");
            }
            PlxApp.Log("player: epoch " + s.Epoch + ": audio tracks count=" + tracks.Count + " current="
                + (tracks.RcCurrent == 0 ? tracks.Current.ToString() : "rc " + tracks.RcCurrent) + " [" + languages + "]");
        }

        private void DoPlay(Session s)
        {
            s.HostPaused = false;
            if (!s.Prepared) { s.PlayPending = true; return; } // starts at PREPARED
            s.PlayPending = false;
            int rc = MediaPlayerNative.player_start(s.Handle);
            if (rc != 0) { Fail(s, "player_start", rc); return; }
            s.Playing = true;
            Post((uint)Native.PLX_HOST_EVENT_PLAYER_STARTED, s.Epoch, 0, 0);
        }

        private void DoPause(Session s)
        {
            s.HostPaused = false;
            s.PlayPending = false;
            if (!s.Prepared) return;
            int rc = MediaPlayerNative.player_pause(s.Handle);
            if (rc != 0) { Fail(s, "player_pause", rc); return; }
            s.Playing = false;
            Post((uint)Native.PLX_HOST_EVENT_PLAYER_PAUSED, s.Epoch, 0, 0);
        }

        // A seek before PREPARED, or while another is in flight, is held (the newest wins, the older command ids are folded into it); it is sent
        // when the player is prepared / when the one in flight has landed.
        private void DoSeek(Session s, long targetMs, long command)
        {
            int ms = (int)Math.Max(0, Math.Min(targetMs, int.MaxValue));
            if (s.SeekLater == null) s.SeekLater = new Seek();
            s.SeekLater.TargetMs = ms;
            s.SeekLater.Commands.Add(command);
            SendHeldSeek(s);
        }

        private void SendHeldSeek(Session s)
        {
            if (!s.Prepared || s.SeekNow != null || s.SeekLater == null) return;
            Seek seek = s.SeekLater;
            s.SeekLater = null;
            s.SeekNow = seek;
            int rc = MediaPlayerNative.player_set_play_position(s.Handle, seek.TargetMs, 1, SeekDoneCb, UserOf(s.Epoch));
            if (rc != 0)
            {
                // A refused seek is not the end of the player: every command id it answers gets its SEEK_DONE with the failure marker, and the
                // next held seek (if any) is tried. Each pass consumes SeekLater, so this ends.
                PlxApp.Log("player: epoch " + s.Epoch + ": player_set_play_position returned " + rc);
                s.SeekNow = null;
                FailSeek(s, seek, SeekRefused);
                SendHeldSeek(s);
                return;
            }
            // The completion callback is bounded: a player that never answers must not hold every later seek (and its command ids) for ever.
            uint epoch = s.Epoch;
            seek.Deadline = new Timer(_ => Enqueue(epoch, ss => OnSeekDeadline(ss, seek)), null, SeekWaitMs, Timeout.Infinite);
        }

        // PLAYER_SEEK_DONE with the marker (SeekRefused or SeekExpired) in b for every command id the seek answered.
        private static void FailSeek(Session s, Seek seek, long marker)
        {
            DisarmSeek(seek);
            foreach (long command in seek.Commands)
            {
                Post((uint)Native.PLX_HOST_EVENT_PLAYER_SEEK_DONE, s.Epoch, command, marker);
            }
        }

        // Answers the command ids of the seek in flight and of the held one with b = -1 and forgets both: the epoch is ending (Release) or
        // never prepared (a failed open), so nothing will land them.
        private static void AnswerSeeks(Session s)
        {
            Seek now = s.SeekNow;
            Seek later = s.SeekLater;
            s.SeekNow = null;
            s.SeekLater = null;
            if (now != null) FailSeek(s, now, SeekRefused);
            if (later != null) FailSeek(s, later, SeekRefused);
        }

        private static void DisarmSeek(Seek seek)
        {
            Timer t = seek.Deadline;
            seek.Deadline = null;
            if (t != null) t.Dispose();
        }

        // The video thread, SeekWaitMs after a seek was sent: no completion yet. Expiry is not refusal: the seek may still land, so its ids get
        // b = -2 (the position is then authoritative) and the next held seek is sent. A completion that arrives after this is credited to the
        // NEXT seek's ids (the callback carries no token), which is the cost of not waiting for ever.
        private void OnSeekDeadline(Session s, Seek seek)
        {
            if (s.SeekNow != seek) return;
            PlxApp.Log("player: epoch " + s.Epoch + ": a seek did not complete within " + SeekWaitMs + " ms");
            s.SeekNow = null;
            FailSeek(s, seek, SeekExpired);
            SendHeldSeek(s);
        }

        private void OnPrepared(Session s)
        {
            s.Prepared = true;
            // ONE read of the audio tracks: each one's language goes to the library before PREPARED (its track choice needs them), and the same
            // read answers a held track and makes the log line.
            s.Tracks = ReadTracks(s);
            for (int i = 0; i < s.Tracks.Languages.Count; i++)
            {
                Post((uint)Native.PLX_HOST_EVENT_PLAYER_AUDIO_LANGUAGE, s.Epoch, i, LanguageWord(s.Tracks.Languages[i]));
            }
            ApplyHeldTrack(s); // a track asked for before PREPARED is selected before the library hears PREPARED
            int duration;
            int rc = MediaPlayerNative.player_get_duration(s.Handle, out duration);
            Post((uint)Native.PLX_HOST_EVENT_PLAYER_PREPARED, s.Epoch, rc == 0 ? duration : 0, 0);
            LogTracks(s, s.Tracks); // after PREPARED: a diagnostic, off the start path
            int width, height;
            if (MediaPlayerNative.player_get_video_size(s.Handle, out width, out height) == 0 && width > 0 && height > 0)
            {
                Post((uint)Native.PLX_HOST_EVENT_PLAYER_VIDEO_SIZE, s.Epoch, width, height);
            }
            SendHeldSeek(s); // a seek asked for before PREPARED goes first, so play starts from it
            if (s.PlayPending) DoPlay(s);
        }

        private void OnSeekDone(Session s)
        {
            Seek landed = s.SeekNow;
            if (landed == null) return;
            s.SeekNow = null;
            DisarmSeek(landed);
            foreach (long command in landed.Commands)
            {
                Post((uint)Native.PLX_HOST_EVENT_PLAYER_SEEK_DONE, s.Epoch, command, landed.TargetMs);
            }
            SendHeldSeek(s);
        }

        // MEASURED: the completed callback fires with the native state still PLAYING (it changes at player_stop), so the state cannot validate
        // it. The callback is reported as it fired; the poller stops reading the position.
        private void OnCompleted(Session s)
        {
            s.Playing = false;
            Post((uint)Native.PLX_HOST_EVENT_PLAYER_COMPLETED, s.Epoch, 0, 0);
        }

        // CLOSE: stop, unprepare and destroy (once), then exactly one PLAYER_CLOSED for the epoch.
        private void DoClose(uint epoch)
        {
            Session s = current;
            if (s != null && s.Epoch == epoch)
            {
                Release(s);
                current = null;
                published = null;
            }
            Post((uint)Native.PLX_HOST_EVENT_PLAYER_CLOSED, epoch, 0, 0);
        }

        private static void Release(Session s)
        {
            s.Closing = true;
            s.Playing = false;
            AnswerSeeks(s); // every command id is answered before PLAYER_CLOSED (the caller posts it after this returns)
            AnswerTracks(s);
            if (s.Handle == IntPtr.Zero) return;
            // A position read or the display bind may be inside the handle: wait for it, with a bound. A read that never returns leaks the handle
            // (the process exit is its teardown) rather than destroying it under the read.
            if (!Monitor.TryEnter(s.HandleGate, CloseWaitMs))
            {
                PlxApp.Log("player: epoch " + s.Epoch + ": a native call is stuck on the handle; it is left undestroyed");
                return;
            }
            try
            {
                // stop and unprepare may answer an invalid-state code for a player that was never started or prepared: that is expected here.
                MediaPlayerNative.player_stop(s.Handle);
                MediaPlayerNative.player_unprepare(s.Handle);
                int rc = MediaPlayerNative.player_destroy(s.Handle);
                if (rc != 0) PlxApp.Log("player: epoch " + s.Epoch + ": player_destroy returned " + rc);
                s.Handle = IntPtr.Zero;
            }
            finally
            {
                Monitor.Exit(s.HandleGate);
            }
        }

        // ---- the Home round trip (OnPause / OnResume, main thread) ---------------------------------------------------------------------------
        //
        // MEASURED on the emulator 2026-11-22 (TV 10.0 x86_64, a scratch probe): the native player raises nothing on its own when the app goes to
        // the background, and a player paused at OnPause and started at OnResume resumes in about 130 ms at the same position. The host does
        // only the first half of that: it pauses what is PLAYING at OnPause, so nothing plays behind the Home screen in the moment before the
        // library acts. The library owns the rest (plx_host.h, HOME): at the background it parks the attempt (a CLOSE the host performs, the
        // host closes its player at a stop), and at the foreground it re-opens the same source at the saved position as a new epoch. OnResume
        // therefore starts nothing. Both only enqueue: the native calls run on the video thread, in order with the library's own requests,
        // and never on the Ecore main thread that called us.

        // OnPause. A player that is playing is paused (PLAYER_PAUSED is posted, as for the library's own pause); one that was asked to play before
        // PREPARED is held back so it does not start behind the Home screen. Nothing is paused for a player that is not playing (idle, paused by
        // the user, completed): there is nothing to give back.
        internal void PauseForHome()
        {
            if (!HasThreads()) return;
            TryEnqueueInternal(() =>
            {
                Session s = current;
                if (s == null || s.Handle == IntPtr.Zero || s.Closing || s.OpenFailed) return;
                if (s.Prepared && s.Playing)
                {
                    int rc = MediaPlayerNative.player_pause(s.Handle);
                    if (rc != 0) { Fail(s, "player_pause (home)", rc); return; }
                    s.Playing = false;
                    s.HostPaused = true;
                    PlxApp.Log("player: epoch " + s.Epoch + ": paused for the Home screen");
                    Post((uint)Native.PLX_HOST_EVENT_PLAYER_PAUSED, s.Epoch, 0, 0);
                }
                else if (!s.Prepared && s.PlayPending)
                {
                    s.PlayPending = false;
                    s.HostPaused = true;
                    PlxApp.Log("player: epoch " + s.Epoch + ": its start is held back for the Home screen");
                }
            });
        }

        // OnResume. Starts NOTHING: the resume of a player is the library's (see plx_host.h, HOME). It only drops what PauseForHome remembered and
        // logs why an OnResume resumed nothing, so a run's log names the path.
        internal void ResumeAfterHome()
        {
            if (!HasThreads()) return;
            TryEnqueueInternal(() =>
            {
                Session s = current;
                if (s == null || !s.HostPaused)
                {
                    PlxApp.Log("player: OnResume: nothing to resume (" + (s == null ? "no player" : "epoch " + s.Epoch + ": the host did not pause it, or the library's own play, pause or close decided since") + ")");
                    return;
                }
                if (s.Handle == IntPtr.Zero || s.Closing || s.OpenFailed)
                {
                    s.HostPaused = false;
                    PlxApp.Log("player: OnResume: epoch " + s.Epoch + " is closed or closing (the library parked it at the background); not resumed");
                    return;
                }
                // The library owns the resume: it parks the attempt at the background (CLOSE) and re-opens the same source at the saved position at the
                // foreground, as a new epoch. A host start here would race that restore, so the pause stays and the flag is dropped.
                s.HostPaused = false;
                PlxApp.Log("player: OnResume: epoch " + s.Epoch + " left as it is: the library's foreground restore owns the resume");
            });
        }

        // Whether the video thread exists: before the first open there is no player to pause, and a Home round trip must not start the threads.
        private bool HasThreads()
        {
            lock (startGate) { return videoThread != null; }
        }

        // The host's own job (not the library's request, so not bounded by QueueLimit); after the shutdown it is dropped.
        private void TryEnqueueInternal(Action job)
        {
            try { queue.Add(new Job { Epoch = 0, Run = job }); }
            catch (InvalidOperationException) { }
        }

        // ---- the position poller ------------------------------------------------------------------------------------------------------------

        private void PollLoop()
        {
            while (!shuttingDown)
            {
                Thread.Sleep(PollMs);
                Session s = published;
                if (s == null || !s.Playing || s.Closing) continue;
                try { PollOnce(s); }
                catch (Exception e) { PlxApp.Log("player: position poll: " + e.GetType().Name + ": " + e.Message); }
            }
        }

        // A read that outlives StallMs is given up on (STALL is posted once) but its task is kept: no second read starts on that handle while it
        // is stuck, and the poller moves on, so a later epoch's player is polled and can stall on its own.
        private Task<int> stuckRead; // poller only
        private Session stuckSession; // poller only

        private void PollOnce(Session s)
        {
            Task<int> read = stuckSession == s ? stuckRead : null;
            if (read == null) read = Task.Run(() => ReadPosition(s));
            else if (!read.IsCompleted) return; // still stuck on this handle: STALL was already posted
            stuckRead = null;
            stuckSession = null;
            if (!read.IsCompleted && !read.Wait(StallMs))
            {
                if (!s.StallSent)
                {
                    s.StallSent = true;
                    Post((uint)Native.PLX_HOST_EVENT_PLAYER_STALL, s.Epoch, 0, 0);
                }
                stuckRead = read;
                stuckSession = s;
                return;
            }
            int ms = read.Result;
            if (ms >= 0)
            {
                s.StallSent = false;
                if (!s.Closing) Post((uint)Native.PLX_HOST_EVENT_PLAYER_POSITION, s.Epoch, ms, 0);
            }
        }

        private static int ReadPosition(Session s)
        {
            if (!Monitor.TryEnter(s.HandleGate)) return -1;
            try
            {
                if (s.Closing || s.Handle == IntPtr.Zero) return -1;
                int ms;
                return MediaPlayerNative.player_get_play_position(s.Handle, out ms) == 0 ? ms : -1;
            }
            finally
            {
                Monitor.Exit(s.HandleGate);
            }
        }

        // ---- the end ------------------------------------------------------------------------------------------------------------------------

        // OnTerminate, after the library was asked to stop: close any player that is still open, with a bound, and end the threads. Requests
        // after this are refused as unavailable. The library's own stop already refuses requests, so this only catches a player the library
        // could not close itself (a host-initiated stop).
        internal void Shutdown()
        {
            Thread video;
            lock (startGate)
            {
                shuttingDown = true; // under the gate EnsureThreads reads it under: a start that is not past it yet is refused, one that is has its thread in `video`
                video = videoThread;
            }
            if (video == null) return;
            try
            {
                queue.Add(new Job
                {
                    Epoch = 0,
                    Run = () =>
                    {
                        Session s = current;
                        if (s != null) DoClose(s.Epoch);
                    },
                });
                queue.CompleteAdding();
            }
            catch (InvalidOperationException) { }
            if (!video.Join(CloseWaitMs + 1000)) PlxApp.Log("player: the video thread did not end within the bound; the process exit is the teardown");
        }
    }
}
