// PlxNative's Tizen host: the .NET side of the hosted library (docs/tizen.md).
//
// A plain CoreUIApplication. OnCreate loads lib/libplx.so and calls plx_host_start ONCE on the main thread (the Ecore main loop, where the
// library's window belongs); OnPause and OnResume are forwarded with plx_host_post; OnTerminate calls plx_host_stop and waits for the library's
// on_stopped. The library makes its own window, draws, reads the remote and runs the application; this host draws nothing.
//
// THE CONTRACT is rust-modules/platform-tizen/include/plx_host.h, and the three declarations in `Native` below, the two structs, the delegates for
// its function pointers and the `PLX_HOST_*` constants are written from it. Version 2 adds the media player: the table's player_open and player_verb
// are implemented by PlxPlayer.cs (over the device's libcapi-media-player, PlayerNative.cs), and its events come back through plx_host_post. `make PLATFORM=tizen tizen-abi` (ci/check-tizen-so.py) reads this file and the header as text
// and fails on a name, a signature, a constant's value, a struct field's name, order or width, or a delegate's widths that differs, so the two cannot
// drift apart. (The media player's own DllImports are in PlayerNative.cs and are the device library's, not this contract's.)
//
// WHAT THIS HOST NEVER DOES. It never calls libc exit(): the process's end is the .NET runtime's. It never frees a callback while the library
// can still call it: the delegates the table points at are held in static fields until on_stopped has arrived with clean = 1, and after clean = 0
// (the library's deadline thread reported a loop that did not finish) it releases nothing at all and leaves the teardown to the process exit.
//
// THE LOG. The only instrument on a television is its log, and a failed library load can be reported from here alone, so every line goes to
// dlog (`sdb dlog PLXHOST`), to <data>/plx-host.log and, when the package carries res/log-host.txt (`TIZEN_LOG_HOST=IPV4:PORT` at build time),
// to that address as one UDP datagram per line, `h<seq> <line>`; the same address is handed to the library as `log_udp` for its own lines.

using System;
using System.Globalization;
using System.IO;
using System.Net;
using System.Net.Sockets;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using Tizen.Applications;

namespace PlxHost
{
    // plx_host.h: PlxHostApi. Field for field, in the header's order; IntPtr for a function pointer.
    [StructLayout(LayoutKind.Sequential)]
    internal struct PlxHostApi
    {
        public uint Size;
        public uint AbiVersion;
        public uint Flags;
        public uint Reserved;
        public IntPtr OnStopped;
        public IntPtr RunOnHost;
        public IntPtr PlayerOpen;
        public IntPtr PlayerVerb;
    }

    // plx_host.h: PlxHostEvent.
    [StructLayout(LayoutKind.Sequential)]
    internal struct PlxHostEvent
    {
        public uint Size;
        public uint Kind;
        public uint Epoch;
        public uint Reserved;
        public long A;
        public long B;
    }

    // plx_host.h: PlxOnStopped, PlxRunOnHost, PlxHostTask.
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    internal delegate void PlxOnStopped(int code, int clean);

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    internal delegate int PlxRunOnHost(IntPtr task, IntPtr arg);

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    internal delegate void PlxHostTask(IntPtr arg);

    // plx_host.h: PlxPlayerOpen and PlxPlayerVerb (version 2). `uri` is a NUL-terminated UTF-8 string, read and copied before the call returns.
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    internal delegate int PlxPlayerOpen(uint epoch, IntPtr uri, int x, int y, int w, int h);

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    internal delegate int PlxPlayerVerb(uint epoch, uint verb, long arg0, long arg1);

    internal static class Native
    {
        // The soname of the library (ci/check-tizen-so.py checks that it is libplx.so). The exact file name, not a bare "plx": a name the
        // runtime does not have to complete is the spelling a shipped Tizen .NET application is known to resolve.
        private const string Lib = "libplx.so";

        internal const int PLX_HOST_ABI_VERSION = 2;
        internal const int PLX_HOST_OK = 0;
        internal const int PLX_HOST_E_STATE = -2;
        internal const int PLX_HOST_E_STALE = -7;
        internal const int PLX_HOST_PLAYER_OK = 0;
        internal const int PLX_HOST_PLAYER_E_UNAVAILABLE = -1;
        internal const int PLX_HOST_PLAYER_E_BUSY = -2;
        internal const int PLX_HOST_PLAYER_E_ARGS = -3;
        internal const int PLX_HOST_PLAYER_E_STALE = -4;
        internal const int PLX_HOST_PLAYER_FAIL_BASE = 2147418112;
        internal const int PLX_HOST_EVENT_PAUSE = 1;
        internal const int PLX_HOST_EVENT_RESUME = 2;
        internal const int PLX_HOST_EVENT_PLAYER_PREPARED = 16;
        internal const int PLX_HOST_EVENT_PLAYER_STARTED = 17;
        internal const int PLX_HOST_EVENT_PLAYER_PAUSED = 18;
        internal const int PLX_HOST_EVENT_PLAYER_POSITION = 19;
        internal const int PLX_HOST_EVENT_PLAYER_SEEK_DONE = 20;
        internal const int PLX_HOST_EVENT_PLAYER_BUFFERING = 21;
        internal const int PLX_HOST_EVENT_PLAYER_VIDEO_SIZE = 22;
        internal const int PLX_HOST_EVENT_PLAYER_COMPLETED = 23;
        internal const int PLX_HOST_EVENT_PLAYER_ERROR = 24;
        internal const int PLX_HOST_EVENT_PLAYER_INTERRUPTED = 25;
        internal const int PLX_HOST_EVENT_PLAYER_CLOSED = 26;
        internal const int PLX_HOST_EVENT_PLAYER_STALL = 27;
        internal const int PLX_HOST_PLAYER_VERB_PLAY = 1;
        internal const int PLX_HOST_PLAYER_VERB_PAUSE = 2;
        internal const int PLX_HOST_PLAYER_VERB_SEEK = 3;
        internal const int PLX_HOST_PLAYER_VERB_CLOSE = 4;

        [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int plx_host_start(ref PlxHostApi api, [MarshalAs(UnmanagedType.LPUTF8Str)] string launchJson);

        [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
        internal static extern int plx_host_post(ref PlxHostEvent ev);

        [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
        internal static extern void plx_host_stop();
    }

    internal static class Dl
    {
        internal const int RTLD_NOW = 2;
        internal const int RTLD_GLOBAL = 0x100;

        [DllImport("libdl.so.2")]
        internal static extern IntPtr dlopen([MarshalAs(UnmanagedType.LPStr)] string file, int mode);

        [DllImport("libdl.so.2")]
        internal static extern IntPtr dlerror();
    }

    // The host's UDP mirror of its own log lines. Non-blocking; every failure is ignored.
    internal static class NetLog
    {
        private static readonly object Gate = new object();
        private static Socket sock;
        private static IPEndPoint to;
        private static long seq;

        // "a.b.c.d:port" (IPv4). Returns null when the sink is on, else the reason it is not.
        internal static string Start(string hostPort)
        {
            try
            {
                int colon = hostPort.LastIndexOf(':');
                IPAddress ip;
                int port;
                if (colon <= 0
                    || !IPAddress.TryParse(hostPort.Substring(0, colon), out ip)
                    || ip.AddressFamily != AddressFamily.InterNetwork
                    || !int.TryParse(hostPort.Substring(colon + 1), out port)
                    || port < 1 || port > 65535)
                {
                    return "expected IPV4:PORT, got \"" + hostPort + "\"";
                }
                var s = new Socket(AddressFamily.InterNetwork, SocketType.Dgram, ProtocolType.Udp);
                s.Blocking = false;
                lock (Gate)
                {
                    to = new IPEndPoint(ip, port);
                    sock = s;
                }
                return null;
            }
            catch (Exception e)
            {
                return e.GetType().Name + ": " + e.Message;
            }
        }

        internal static void Send(string line)
        {
            try
            {
                lock (Gate)
                {
                    if (sock == null) return;
                    // Counted per attempt, so a failed send still leaves a visible gap.
                    string text = "h" + (seq++).ToString("D6", CultureInfo.InvariantCulture) + " " + line + "\n";
                    byte[] bytes = Encoding.UTF8.GetBytes(text);
                    if (bytes.Length > 1400) Array.Resize(ref bytes, 1400);
                    sock.SendTo(bytes, SocketFlags.None, to);
                }
            }
            catch (Exception) { }
        }
    }

    internal class PlxApp : CoreUIApplication
    {
        // The table's delegates. Static so nothing but the end of the process (or a clean on_stopped, in OnTerminate) lets the runtime collect
        // them while the library holds their native thunks.
        private static PlxOnStopped onStoppedDelegate;
        private static PlxRunOnHost runOnHostDelegate;
        private static PlxPlayerOpen playerOpenDelegate;
        private static PlxPlayerVerb playerVerbDelegate;

        // The synchronization context of the main thread, captured in OnCreate: run_on_host posts the library's task to it.
        private static SynchronizationContext hostContext;

        // Set by on_stopped, from whichever thread delivers it.
        private static readonly ManualResetEventSlim Stopped = new ManualResetEventSlim(false);
        private static volatile int stoppedClean = -1;

        // True from a plx_host_start that returned OK until OnTerminate.
        private static volatile bool running;

        // True once OnTerminate asked the library to stop: an on_stopped after that answers OUR request, any earlier one is the library ending
        // by itself (the remote's exit key, a start it refused after plx_host_start returned OK) and the application must end with it.
        private static volatile bool stopRequested;

        // The one application instance, for the exit request on_stopped makes from a library thread.
        private static PlxApp current;

        private static readonly object LogGate = new object();
        private static string logPath;

        // dlog, the shared log file and, when the package names a log host, UDP.
        internal static void Log(string msg)
        {
            string line = "[host " + DateTime.Now.ToString("HH:mm:ss.fff", CultureInfo.InvariantCulture) + "] " + msg;
            NetLog.Send(line);
            try { Tizen.Log.Info("PLXHOST", line); } catch (Exception) { }
            try
            {
                lock (LogGate)
                {
                    if (logPath != null) File.AppendAllText(logPath, line + "\n");
                }
            }
            catch (Exception) { }
        }

        protected override void OnCreate()
        {
            base.OnCreate();

            string data = this.DirectoryInfo.Data;
            string res = this.DirectoryInfo.Resource;
            hostContext = SynchronizationContext.Current;
            current = this;
            lock (LogGate) { logPath = Path.Combine(data, "plx-host.log"); }
            string logUdp = StartNetLog(res, out string netNote);
            Log("host started: OnCreate on the main thread; data dir " + data + "; res dir " + res);
            Log(netNote);
            if (hostContext == null)
            {
                Log("WARNING: the main thread has no SynchronizationContext, so run_on_host will refuse every task and the window is freed only by the process exit");
            }
            LogEnvironment(res);
            string platformVersion = SystemString("http://tizen.org/feature/platform.version");

            string launch = BuildLaunchJson(res, data, platformVersion, logUdp);
            int rc = StartLibrary(launch);
            if (rc != Native.PLX_HOST_OK)
            {
                // A refused start has already delivered on_stopped(code, 1) on this thread (plx_host.h, START); a failed load never reached the
                // library. Either way there is nothing to wait for: leave through the application's own exit, not libc's.
                Log("plx_host_start FAILED rc=" + rc + " (codes: plx_host.h; host: -101 library not found, -102 entry point missing, -103 other). Exiting.");
                Exit();
                return;
            }
            running = true;
            Log("plx_host_start OK; the library's loop is running");
            if (playerOpenDelegate != null) PlxPlayer.LogWindow(); // main thread: Ecore
            if (Stopped.IsSet && ShouldExitHost(stopRequested, running))
            {
                // The loop ended before this line ran (on_stopped found `running` still false): nothing else will end the process.
                Log("the library stopped by itself during start: exiting");
                Exit();
            }
        }

        protected override void OnPause()
        {
            base.OnPause();
            Log("OnPause");
            Post(Native.PLX_HOST_EVENT_PAUSE, "pause");
            // The native player does not pause itself when the app goes to the background (emulator, 2026-11-22): pause what is playing.
            PlxPlayer.Instance.PauseForHome();
        }

        protected override void OnResume()
        {
            base.OnResume();
            Log("OnResume");
            Post(Native.PLX_HOST_EVENT_RESUME, "resume");
            PlxPlayer.Instance.ResumeAfterHome(); // starts nothing: the library's foreground restore re-opens the source (plx_host.h, HOME); this only drops the host's pause note and logs why
        }

        protected override void OnTerminate()
        {
            base.OnTerminate();
            Log("OnTerminate");
            if (running)
            {
                stopRequested = true;
                running = false;
                bool asked = RequestLibraryStop();
                // After the library has been asked to stop (it refuses requests from then on, so it can NOT close a player it has open: its
                // CLOSE is answered locally and the library treats the release as ours, plx_host.h STOP): close that player and end the video
                // thread NOW, before waiting for on_stopped, so the native player is released on every terminate path, also when the wait
                // below runs into the platform's own kill. Of the player events it still posts only PLAYER_CLOSED is heard by the library (the confirmation of a close it had accepted); the rest are refused.
                PlxPlayer.Instance.Shutdown();
                if (asked) WaitLibraryStopped();
            }
        }

        protected override void OnLocaleChanged(LocaleChangedEventArgs e)
        {
            base.OnLocaleChanged(e);
            // The ABI has no locale event (plx_host.h: the host's events are PAUSE and RESUME, and the player's): the locale the library knows is the one at start.
            Log("OnLocaleChanged " + e.Locale + " (not forwarded: the library reads the locale at start only)");
        }

        // plx_host_start, with every way the call can fail on a device recorded. Returns the library's code, or a host code below -100.
        private int StartLibrary(string launch)
        {
            onStoppedDelegate = OnStoppedNative;
            runOnHostDelegate = RunOnHostNative;
            // The media player: both pointers, or neither. A device without libcapi-media-player gets NULLs, and the library then plays nothing
            // (plx_host.h, START: a table without the player group is not refused).
            string playerNote;
            bool player;
            try
            {
                player = PlxPlayer.Probe(out playerNote);
            }
            catch (Exception e)
            {
                // Skew means "no playback", never a refused start: anything the probe throws (the dlopen binding, the player's static state) leaves the pointers NULL.
                player = false;
                playerNote = "media player: the probe threw " + e.GetType().Name + ": " + e.Message + "; the table leaves the player pointers NULL";
            }
            Log(playerNote);
            if (player)
            {
                playerOpenDelegate = PlxPlayer.OpenNative;
                playerVerbDelegate = PlxPlayer.VerbNative;
            }
            var api = new PlxHostApi
            {
                Size = (uint)Marshal.SizeOf<PlxHostApi>(),
                AbiVersion = (uint)Native.PLX_HOST_ABI_VERSION,
                Flags = 0,
                Reserved = 0,
                OnStopped = Marshal.GetFunctionPointerForDelegate(onStoppedDelegate),
                RunOnHost = Marshal.GetFunctionPointerForDelegate(runOnHostDelegate),
                PlayerOpen = player ? Marshal.GetFunctionPointerForDelegate(playerOpenDelegate) : IntPtr.Zero,
                PlayerVerb = player ? Marshal.GetFunctionPointerForDelegate(playerVerbDelegate) : IntPtr.Zero,
            };
            try
            {
                return Native.plx_host_start(ref api, launch);
            }
            catch (DllNotFoundException e)
            {
                Log("DllNotFoundException for libplx.so: " + e.Message);
                if (!PreloadByPath()) return -101;
                try
                {
                    Log("retrying plx_host_start after preload");
                    return Native.plx_host_start(ref api, launch);
                }
                catch (Exception e2)
                {
                    Log("still failing after preload: " + e2.GetType().Name + ": " + e2.Message);
                    return -101;
                }
            }
            catch (EntryPointNotFoundException e)
            {
                Log("EntryPointNotFoundException (the library loaded but lacks plx_host_start): " + e.Message);
                return -102;
            }
            catch (BadImageFormatException e)
            {
                Log("BadImageFormatException (wrong architecture?): " + e.Message);
                return -101;
            }
            catch (Exception e)
            {
                Log("exception from plx_host_start: " + e);
                return -103;
            }
        }

        // plx_host_stop returns at once; on_stopped then arrives from the library's loop (clean = 1) or its deadline thread (clean = 0).
        // False when the call threw (the callbacks stay referenced: the library may still call them).
        private bool RequestLibraryStop()
        {
            try
            {
                Native.plx_host_stop();
                return true;
            }
            catch (Exception e)
            {
                Log("exception from plx_host_stop: " + e.GetType().Name + ": " + e.Message);
                return false;
            }
        }

        private void WaitLibraryStopped()
        {
            // The wait is bounded: the library's own deadline thread reports a loop that does not finish, so this is the backstop for a library
            // that never reports at all.
            if (!Stopped.Wait(TimeSpan.FromSeconds(15)))
            {
                Log("on_stopped did not arrive within 15 s: releasing nothing");
                return;
            }
            if (stoppedClean == 1)
            {
                Log("on_stopped clean: the callbacks are released");
                onStoppedDelegate = null;
                runOnHostDelegate = null;
                playerOpenDelegate = null;
                playerVerbDelegate = null;
            }
            else
            {
                Log("on_stopped NOT clean: releasing nothing, the process exit is the teardown");
            }
        }

        // One player event for the library, from any thread (the video thread, the position poller, a native callback of the player): a non-blocking
        // plx_host_post. The library answers PLX_HOST_E_STALE for an epoch it no longer holds and PLX_HOST_E_STATE after a stop: both expected
        // and silent; anything else is logged. The uri and the token are never in an event.
        internal static void PostPlayer(uint kind, uint epoch, long a, long b)
        {
            if (!running) return;
            try
            {
                var ev = new PlxHostEvent { Size = (uint)Marshal.SizeOf<PlxHostEvent>(), Kind = kind, Epoch = epoch, Reserved = 0, A = a, B = b };
                int rc = Native.plx_host_post(ref ev);
                if (rc != Native.PLX_HOST_OK && rc != Native.PLX_HOST_E_STALE && rc != Native.PLX_HOST_E_STATE) Log("plx_host_post(player kind " + kind + ") returned " + rc);
            }
            catch (Exception e)
            {
                Log("exception from plx_host_post(player kind " + kind + "): " + e.GetType().Name + ": " + e.Message);
            }
        }

        // Run `action` on the main thread (the Ecore main loop) and wait for it, at most `timeoutMs`. False when it did not finish in time (it may
        // still run later: the action must be safe to run late). Without a main-thread context it runs inline on the calling thread.
        internal static bool RunOnMain(Action action, int timeoutMs)
        {
            SynchronizationContext context = hostContext;
            if (context == null)
            {
                action();
                return true;
            }
            using (var done = new ManualResetEventSlim(false))
            {
                context.Post(state =>
                {
                    try { action(); }
                    catch (Exception e) { Log("exception from a main-thread player step: " + e.GetType().Name + ": " + e.Message); }
                    finally { try { done.Set(); } catch (ObjectDisposedException) { } }
                }, null);
                return done.Wait(timeoutMs);
            }
        }

        private static void Post(uint kind, string what)
        {
            if (!running) return;
            try
            {
                var ev = new PlxHostEvent { Size = (uint)Marshal.SizeOf<PlxHostEvent>(), Kind = kind, Epoch = 0, Reserved = 0, A = 0, B = 0 };
                int rc = Native.plx_host_post(ref ev);
                if (rc != Native.PLX_HOST_OK) Log("plx_host_post(" + what + ") returned " + rc);
            }
            catch (Exception e)
            {
                Log("exception from plx_host_post(" + what + "): " + e.GetType().Name + ": " + e.Message);
            }
        }

        // Whether an on_stopped means the application must end: the library was running and the host did not ask it to stop. A library that
        // ends by itself has torn its window down, and a .NET process left behind with no window answers the next launch (launch_mode
        // "single") with that dead instance. NOT unit-tested (no C# runner in this repository's gates), and NOT exercised on the emulator
        // either: the remote's Exit key reaches the library as `Exit Down`, but the platform treats Exit as a system key, sends OnPause at once and
        // OnTerminate 365 ms later, so `stopRequested` is already set when on_stopped arrives; Back on Home does not quit. No remote route makes
        // the library end by itself there.
        internal static bool ShouldExitHost(bool stopWasRequested, bool isRunning)
        {
            return isRunning && !stopWasRequested;
        }

        // PlxOnStopped. Called by the library on the loop thread, its deadline thread or (for a refused start) this one: it returns promptly
        // and calls nothing in the library. An exception must never cross back into native code.
        private static void OnStoppedNative(int code, int clean)
        {
            try
            {
                stoppedClean = clean;
                Stopped.Set();
                Log("on_stopped code=" + code + " clean=" + clean);
                if (ShouldExitHost(stopRequested, running)) RequestHostExit();
            }
            catch (Exception) { }
        }

        // End the application from the main thread. The request is queued behind the window teardown the library posted with run_on_host
        // (same context, in order), so the library's task runs before the runtime shuts EFL down.
        private static void RequestHostExit()
        {
            SynchronizationContext context = hostContext;
            PlxApp app = current;
            if (context == null || app == null)
            {
                Log("the library stopped by itself and there is no main-thread context to end the application with");
                return;
            }
            context.Post(state =>
            {
                try
                {
                    Log("the library stopped by itself: exiting the application");
                    app.Exit();
                }
                catch (Exception e)
                {
                    Log("exception ending the application: " + e.GetType().Name + ": " + e.Message);
                }
            }, null);
        }

        // PlxRunOnHost: queue task(arg) for the main thread and return 0, or refuse (non-zero) and the library drops it.
        private static int RunOnHostNative(IntPtr task, IntPtr arg)
        {
            try
            {
                SynchronizationContext context = hostContext;
                if (context == null || task == IntPtr.Zero) return -1;
                context.Post(state =>
                {
                    try
                    {
                        Marshal.GetDelegateForFunctionPointer<PlxHostTask>(task)(arg);
                    }
                    catch (Exception e)
                    {
                        Log("exception from a run_on_host task: " + e.GetType().Name + ": " + e.Message);
                    }
                }, null);
                return 0;
            }
            catch (Exception)
            {
                return -1;
            }
        }

        // The launch string (plx_host.h, LAUNCH JSON). The host's own members first, then, when the package carries one, the dev-boot file's
        // members (pms_host, pms_port and the dev-only `dev` object: the placeholder identity of the mock server; tizen/package.py writes it
        // from TIZEN_MOCK_HOST). Without that file the launch names no Plex server and the library refuses it, which is logged.
        private string BuildLaunchJson(string res, string data, string platformVersion, string logUdp)
        {
            var sb = new StringBuilder("{");
            AppendMember(sb, "payload_dir", res);
            sb.Append(',');
            AppendMember(sb, "state_dir", data);
            sb.Append(',');
            string locale = CultureInfo.CurrentUICulture.Name;
            AppendMember(sb, "locale", string.IsNullOrEmpty(locale) ? "en-US" : locale);
            if (!string.IsNullOrEmpty(platformVersion))
            {
                sb.Append(',');
                AppendMember(sb, "platform_version", platformVersion);
            }
            if (logUdp != null)
            {
                sb.Append(',');
                AppendMember(sb, "log_udp", logUdp);
            }
            string dev = ReadDevBoot(res);
            if (dev != null) sb.Append(',').Append(dev);
            return sb.Append('}').ToString();
        }

        // <res>/dev-boot.json is ONE JSON object written by tizen/package.py; its members are spliced into the launch string as they are.
        private static string ReadDevBoot(string res)
        {
            string path = Path.Combine(res, "dev-boot.json");
            try
            {
                if (!File.Exists(path))
                {
                    Log("dev boot: none, starting at Plex sign-in");
                    return null;
                }
                string text = File.ReadAllText(path).Trim();
                if (text.Length < 2 || text[0] != '{' || text[text.Length - 1] != '}')
                {
                    Log("dev boot: " + path + " is not one JSON object; ignored");
                    return null;
                }
                string inner = text.Substring(1, text.Length - 2).Trim();
                if (inner.Length == 0) return null;
                Log("dev boot: the members of " + path + " (a mock server; never logged in full: it carries a placeholder token)");
                return inner;
            }
            catch (Exception e)
            {
                Log("dev boot: " + e.GetType().Name + ": " + e.Message);
                return null;
            }
        }

        private static void AppendMember(StringBuilder sb, string key, string value)
        {
            sb.Append('"').Append(key).Append("\":");
            AppendString(sb, value);
        }

        private static void AppendString(StringBuilder sb, string value)
        {
            sb.Append('"');
            foreach (char c in value)
            {
                switch (c)
                {
                    case '"': sb.Append("\\\""); break;
                    case '\\': sb.Append("\\\\"); break;
                    case '\n': sb.Append("\\n"); break;
                    case '\r': sb.Append("\\r"); break;
                    case '\t': sb.Append("\\t"); break;
                    default:
                        if (c < 0x20) sb.Append("\\u").Append(((int)c).ToString("x4", CultureInfo.InvariantCulture));
                        else sb.Append(c);
                        break;
                }
            }
            sb.Append('"');
        }

        // <res>/log-host.txt, written by tizen/package.py (TIZEN_LOG_HOST), holds "a.b.c.d:port". Returns the address when the host's UDP log
        // started (null when there is no file or it is unusable), and a one-line note for the log either way.
        private static string StartNetLog(string res, out string note)
        {
            string path = Path.Combine(res, "log-host.txt");
            try
            {
                if (!File.Exists(path))
                {
                    note = "network log: off (no " + path + ")";
                    return null;
                }
                string hostPort = File.ReadAllText(path).Trim();
                string err = NetLog.Start(hostPort);
                if (err != null)
                {
                    note = "network log: OFF, unusable log host in " + path + ": " + err;
                    return null;
                }
                note = "network log: UDP -> " + hostPort + " (host lines h<seq>)";
                return hostPort;
            }
            catch (Exception e)
            {
                note = "network log: OFF, " + e.GetType().Name + ": " + e.Message;
                return null;
            }
        }

        // dlopen lib/libplx.so by absolute path so the loader's own error text (dlerror) lands in the log: that text is what tells "wrong
        // glibc" from "library refused" from "wrong architecture".
        private bool PreloadByPath()
        {
            string path = LibraryPath(this.DirectoryInfo.Resource);
            IntPtr h = Dl.dlopen(path, Dl.RTLD_NOW | Dl.RTLD_GLOBAL);
            if (h != IntPtr.Zero)
            {
                Log("preload dlopen(" + path + ") ok");
                return true;
            }
            IntPtr err = Dl.dlerror();
            Log("preload dlopen(" + path + ") FAILED: " + (err == IntPtr.Zero ? "(no dlerror)" : Marshal.PtrToStringAnsi(err)));
            return false;
        }

        // <app>/res/ is DirectoryInfo.Resource; the package's lib/ is its sibling.
        private static string LibraryPath(string res)
        {
            return Path.GetFullPath(Path.Combine(res, "..", "lib", "libplx.so"));
        }

        private static void LogEnvironment(string res)
        {
            Log("runtime " + RuntimeInformation.FrameworkDescription
                + " / os " + RuntimeInformation.OSDescription
                + " / OSArchitecture " + RuntimeInformation.OSArchitecture
                + " / ProcessArchitecture " + RuntimeInformation.ProcessArchitecture
                + " / IntPtr.Size " + IntPtr.Size);
            foreach (string key in new[]
            {
                "http://tizen.org/feature/platform.native.api.version",
                "http://tizen.org/system/platform.name",
                "http://tizen.org/system/model_name",
                "http://tizen.org/system/build.string",
            })
            {
                SystemString(key);
            }
            string lib = LibraryPath(res);
            try
            {
                var fi = new FileInfo(lib);
                Log("native library " + lib + (fi.Exists ? " exists, " + fi.Length + " bytes" : " DOES NOT EXIST"));
            }
            catch (Exception e)
            {
                Log("native library check failed: " + e.Message);
            }
        }

        // Log one Tizen system string and return it (null when the key is missing or the call threw).
        private static string SystemString(string key)
        {
            try
            {
                string v;
                bool ok = Tizen.System.Information.TryGetValue<string>(key, out v);
                Log(key + " = " + (ok ? v : "(unavailable)"));
                return ok ? v : null;
            }
            catch (Exception e)
            {
                Log(key + " -> " + e.GetType().Name + ": " + e.Message);
                return null;
            }
        }

        private static void Main(string[] args)
        {
            // The runtime puts the process in the television's locale, and a decimal comma breaks printf("%f")-built text. .NET's
            // SetEnvironmentVariable need not reach native getenv, so the library also calls setlocale(LC_NUMERIC, "C") itself.
            Environment.SetEnvironmentVariable("LC_NUMERIC", "C");
            AppDomain.CurrentDomain.UnhandledException += (s, e) => Log("Unhandled exception: " + e.ExceptionObject);
            new PlxApp().Run(args);
        }
    }
}
