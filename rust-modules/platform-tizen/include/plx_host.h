/*
 * plx_host.h -- the hosted C ABI of PlxNative's Tizen library (docs/platform-ports.md, Target shape 7).
 *
 * The single source of the contract: the Rust side (rust-modules/platform-tizen/src/host_abi.rs) and the
 * .NET host's DllImports and its export check are both written from this file, and a host test pins that the
 * names, values and field order below equal the Rust ones. That test (and the crate's others) is compiled
 * only with the crate's `tizen` feature: it runs under `cargo test --lib -p plx_platform_tizen --features
 * tizen`, and a plain `cargo test -p plx_platform_tizen` builds an empty crate.
 *
 * Three exports, no others:
 *     int32_t plx_host_start(const PlxHostApi *api, const char *launch_json);
 *     int32_t plx_host_post(const PlxHostEvent *event);
 *     void    plx_host_stop(void);
 *
 * THREADS
 *   host thread  The .NET host's main thread, which is the Ecore main loop (OnCreate, OnPause, OnResume,
 *                OnTerminate). plx_host_start and plx_host_stop are called there, and ONLY there: the
 *                Ecore_Wl2 window, its key and focus handlers and its teardown belong to this thread.
 *   loop thread  A thread the library starts inside plx_host_start; it runs the application. The library
 *                never calls an Ecore window function from it.
 *   any thread   plx_host_post may be called from any thread.
 *
 * OWNERSHIP
 *   A pointer passed INTO the library (the table, the launch string, an event) is read and copied before the
 *   call returns; the host may free it at once. A pointer passed to the host (the arg of a task) is the
 *   library's, and the host passes it back untouched. The host keeps the function pointers it put in the
 *   table callable until on_stopped has been delivered with clean = 1; after clean = 0 it releases nothing
 *   and treats process exit as the teardown. The library is never dlclosed, and the host never calls exit():
 *   the process end is the host's own runtime's.
 *
 * START
 *   plx_host_start is called once. It refuses a table that is NULL, smaller than version 1
 *   (size < PLX_HOST_API_V1_SIZE), of an abi_version of 0, or with a flags bit the library does not know
 *   (version 1 knows none: flags must be 0): PLX_HOST_E_ARGS, and no callback of any kind is made, because
 *   the table was not accepted. A later field of a bigger table is read only when size covers it AND
 *   abi_version names it: the player group below (version 2) is read only when size >= PLX_HOST_API_V2_SIZE and
 *   abi_version >= 2, and then only when BOTH pointers are non-NULL. A version-1 host, a version-2 host that
 *   left either NULL, or a table whose size stops at run_on_host is NOT refused: the library starts, reports
 *   "no media player" and plays nothing (the platform composes no URI driver). Skew is "no playback", never a
 *   refused start.
 *
 *   From the moment the table is ACCEPTED, on_stopped is delivered exactly once, whatever happens next:
 *     PLX_HOST_OK        the loop thread was started; on_stopped comes from it (or from the deadline thread
 *                        after plx_host_stop, see STOP).
 *     PLX_HOST_E_LAUNCH  launch_json was refused (not JSON, a required key missing or of the wrong type, a
 *                        value out of range). Nothing was started.
 *     PLX_HOST_E_WINDOW  the Ecore window or the EGL libraries could not be brought up (a library that does
 *                        not open, a window that cannot be made), or bringing the platform up panicked; what
 *                        was made is released first. Nothing was started.
 *     PLX_HOST_E_THREAD  the loop thread could not be started. Nothing was started.
 *   For the three refusals on_stopped(1, 1) is called on the HOST thread, before plx_host_start returns, and
 *   the return value is the code above (clean = 1: nothing is held).
 *   A second plx_host_start returns PLX_HOST_E_STATE and makes no callback: the first start's exactly-once
 *   report stands.
 *
 * STOP
 *   plx_host_stop (host thread, from OnTerminate) refuses every later callback into the host, asks the
 *   application to end and returns at once. It never blocks. on_stopped then arrives once:
 *     - from the loop thread with clean = 1, after the application returned, its GL context is released
 *       and every callback that was running has returned; or
 *     - from the loop thread with clean = 0, when the application returned without releasing everything: a
 *       worker still inside firmware, a refused boot, or a panic caught on the loop thread (the loop's own
 *       report: what it did not release stays held); or
 *     - from the library's deadline thread with clean = 0, when the loop has not finished within the
 *       deadline (a stalled swap); or
 *     - from the HOST thread with clean = 0, inside plx_host_stop itself, when the library could not start
 *       its deadline thread (code 1).
 *   With clean = 1 the host may release its callbacks; with clean = 0 it releases nothing. The Ecore window
 *   is not part of that promise: after a host-initiated stop the library does not release it (it lives until
 *   the process ends, see run_on_host); it is released only when the application ended by itself and the
 *   host ran the teardown the library queued.
 *   **A player that is open at a stop belongs to the host.** plx_host_stop closes the library's gate to the host AT ONCE, so the
 *   library can no longer send the player CLOSE verb (it answers the request itself, the host is not asked), and no PLAYER_CLOSED
 *   can come back for a CLOSE that was not sent. The library therefore treats a CLOSE refused that way as a release the host makes: its
 *   stop is not held for a confirmation that cannot come, and on_stopped can be clean = 1 with a player open. A CLOSE the host had
 *   ALREADY accepted before the stop is another case: its PLAYER_CLOSED is still heard while the loop runs (it is the only player
 *   event heard after a stop, and only for a live epoch), because the library's stop waits for it. The host MUST close its native player itself
 *   in OnTerminate, before it waits for on_stopped (PlxPlayer.Shutdown, bounded), so that the native player is released on every
 *   terminate path, including a wait that ends in the platform's own kill.
 *   After on_stopped no callback is made again. on_stopped may be called on the loop thread, the deadline
 *   thread, the host thread (a refused start, or inside plx_host_stop as above); it must return promptly
 *   and make no call into the library. A host must therefore not hold, across plx_host_stop, a lock that
 *   its on_stopped takes.
 *
 * run_on_host
 *   The loop never waits on the host thread. Work that must run there (freeing the Ecore window when the
 *   application ends by itself) leaves as run_on_host(task, arg): the host queues task(arg) for its main
 *   thread, calls it there exactly once, and returns 0; any other return means the request was refused and
 *   the library drops it. It is never called after a stop was requested: plx_host_stop closes the library's
 *   gate at once, whether or not the host thread is busy, so the window of a host-initiated stop is NOT torn
 *   down by the library and lives until the process ends. (A teardown that waited for on_stopped could not be
 *   requested either: the host thread is the one returning from OnTerminate.) A task queued before a stop
 *   (the window teardown of an application that ended by itself) calls Ecore, so the host must run its queue
 *   before it shuts EFL down; the code and library memory it uses stay mapped for ever.
 *
 * LAUNCH JSON (UTF-8, copied; unknown keys are ignored)
 *   required   "payload_dir"       string, absolute: the package's read-only payload (res/: fonts, icons)
 *              "state_dir"         string, absolute: the app's writable data directory
 *   optional   "pms_host"          string: a fixed Plex server for dev boot
 *              "pms_port"          integer 1..65535, supplied together with pms_host.
 *                                  Omit both PMS keys to boot without a fixed server.
 *              "locale"            string, a BCP-47 tag, default "en-US"
 *              "platform_version"  string, the Tizen version ("10.0"), default "unknown"
 *              "log_udp"           string "a.b.c.d:port", an IPv4 literal: one datagram per log line goes
 *                                  there. Absent or empty: the log channel is off.
 *              "dev"               object, honoured only by a build with dev triggers (never a release):
 *                                    "token"       string, a placeholder identity for the mock server
 *                                    "pms_origin"  string, "http://host:port" of the mock server
 *   The string carries the dev token: the library never logs it.
 *
 * POST
 *   plx_host_post copies the event and queues it: a bounded push that never blocks. PLX_HOST_E_FULL means
 *   the queue was full and the event was dropped; PLX_HOST_E_STATE, that the library is not running (not
 *   started, or a stop was requested, except for a PLAYER_CLOSED of a live epoch, which is still heard while the loop runs: see STOP); PLX_HOST_E_ARGS, a NULL event, a size below PLX_HOST_EVENT_V1_SIZE
 *   (below PLX_HOST_EVENT_V2_SIZE for a player kind) or an unknown kind; PLX_HOST_E_STALE, a player event whose
 *   epoch is not live (see PLAYER).
 *   A pause or a resume is one transition: the application is told once per change of state. A pause while
 *   paused, and a resume while running (the host's resume at launch), are accepted and make no event.
 *
 * PLAYER (version 2)
 *   Two function pointers, player_open and player_verb, are the library's requests to the host's media
 *   player. The host implements them however it likes (the shipped one P/Invokes libcapi-media-player.so.0;
 *   tizen/host/PlxHost/PlxPlayer.cs); the table does not name an implementation.
 *   - A request RETURNS AT ONCE: PLX_HOST_PLAYER_OK means it was queued for the host's own video thread, a
 *     negative PLX_HOST_PLAYER_E_* means it was refused and nothing was queued. The library calls them from its
 *     loop thread and never waits on the host; a result is an EVENT, posted with plx_host_post.
 *   - epoch is a non-zero u32 the library mints per open. Every event the host posts for that player carries
 *     it. The library keeps an epoch LIVE from player_open until it accepts the PLAYER_CLOSED of that epoch;
 *     an event for an epoch that is not live (never opened, or already closed) is dropped and answers
 *     PLX_HOST_E_STALE, so a completion that lands after a stop never reaches the application.
 *   - player_open(epoch, uri, x, y, w, h): create a player, give it uri (a NUL-terminated string, copied
 *     before the call returns; it carries the Plex token in its query, so the host never logs it), bind it as
 *     an OVERLAY to the library's own Ecore_Wl2 window over the rectangle x, y, w, h, and prepare it. The
 *     window is not passed: the host finds it from the Ecore side (ecore_wl2_connected_display_get(NULL), then
 *     ecore_wl2_display_window_find(display, 1): the library makes exactly one window and it is the first, which
 *     the emulator probe measured to be the very pointer the library logs as "ecore: window_new(...)"). The
 *     display is bound BEFORE prepare: a player without one does not advance. An open while another epoch is open closes that one first (its
 *     PLAYER_CLOSED is posted). A failure AFTER the request was queued arrives as PLAYER_ERROR, never as a
 *     return code. PLAYER_PREPARED follows a successful prepare.
 *   - player_verb(epoch, verb, arg0, arg1): PLX_HOST_PLAYER_VERB_PLAY (a play requested before PREPARED
 *     starts at PREPARED), _PAUSE, _SEEK (arg0 = target milliseconds, arg1 = the library's command id, echoed
 *     in PLAYER_SEEK_DONE; a seek before PREPARED, or while another is in flight, is held and the newest sent when it can be: every
 *     command id gets its SEEK_DONE, folded ones included) and _CLOSE. CLOSE ends in exactly ONE PLAYER_CLOSED for that epoch, whether or not a
 *     player existed, and releases it (the host never leaves a player behind). A verb for an epoch that is not
 *     the host's current one is refused with PLX_HOST_PLAYER_E_STALE, and so is every PLAY, PAUSE or SEEK for an epoch whose CLOSE the
 *     host has accepted: the close retires the epoch at the door, before its PLAYER_CLOSED is posted (a second CLOSE of it is still
 *     accepted and still answers one PLAYER_CLOSED, which the library drops as stale).
 *   - Events: PlxHostEvent grows (size >= PLX_HOST_EVENT_V2_SIZE) by epoch and two i64 payload words a, b. The
 *     kinds, with their payloads:
 *       PLAYER_PREPARED(a = duration ms)  PLAYER_STARTED  PLAYER_PAUSED  PLAYER_POSITION(a = ms)
 *       PLAYER_SEEK_DONE(a = command id, b = ms, or -1 refused, or -2 expired)  PLAYER_BUFFERING(a = percent)  PLAYER_VIDEO_SIZE(a = w, b = h)
 *       PLAYER_COMPLETED (the player's completed callback, as it fired)  PLAYER_ERROR(a = the player's native error code, or PLX_HOST_PLAYER_FAIL_BASE + n)  PLAYER_INTERRUPTED(a = native code)
 *       PLAYER_CLOSED  PLAYER_STALL (a position read has not returned for several seconds)
 *     The library queues them on a queue of its own (not the input queue), lossless for every kind but
 *     PLAYER_POSITION and PLAYER_BUFFERING, which keep only the latest per epoch. Position is polled on the HOST
 *     side (about every 250 ms while playing, off the request queue) and posted: there is no synchronous
 *     position call. A read that has not returned after 3 s posts ONE PLAYER_STALL; the host then stops waiting for it (it starts no
 *     second read on that player) and keeps polling the epoch that is current.
 *   - SEEK_DONE always arrives, once per command id, before PLAYER_CLOSED, and b says how it went:
 *       b >= 0   the seek landed; b is the target in ms.
 *       b = -1   refused: it did not land. The native player refused it (no PLAYER_ERROR: the player is still there and the next held
 *                seek is tried), or the epoch ended or never prepared first (a close, or an open that failed, answers every command id
 *                still held or in flight with b = -1, and a seek that reached the queue for a player that has since failed to open is answered the same way; one sent after
 *                the epoch's CLOSE was accepted is refused with PLX_HOST_PLAYER_E_STALE and queues nothing).
 *       b = -2   expired: no completion within 10 s. Expiry is not refusal: the seek MAY STILL LAND, so PLAYER_POSITION is the
 *                authority on where the player is. The next held seek is then sent while the expired one may still be pending, and a late
 *                completion is credited to the next seek's command ids (the player's callback carries no token).
 *   - HOME: ONE OWNER, the library. The host forwards OnPause and OnResume as PLX_HOST_EVENT_PAUSE and _RESUME, and pauses a PLAYING
 *     player at OnPause (PLAYER_PAUSED is posted) so the picture and the sound stop at once, because the native player raises nothing
 *     of its own when the app goes to the background (emulator, 2026-11-22). The host NEVER starts a player at OnResume. A start that
 *     was asked before PREPARED is held back at OnPause (PauseForHome) and OnResume never restarts it; that is safe because the
 *     library's park (its CLOSE at the background) ends that epoch and its restore is a NEW one. The library
 *     parks its attempt at the background (it sends CLOSE, and the host answers CLOSED) and, at the foreground, re-opens the same source
 *     at the saved position as a NEW epoch (a PLAYER_OPEN, then the usual PREPARED, seek and PLAY). A host start of the old epoch would
 *     race that restore, so OnResume only logs why it resumed nothing.
 *   - THREADS: the requests come from the library's loop thread; the host runs them on its own video thread,
 *     never on the loop thread, and never on the Ecore main thread EXCEPT one step: finding the Ecore window and
 *     binding the player to it run there (Ecore belongs to that thread) while the video thread waits, for at most 3 s. plx_host_post may be called from any thread,
 *     including a native callback of the player. After plx_host_stop no request is made (the library's gate
 *     refuses it); a request already running holds the clean on_stopped until it returns.
 */
#ifndef PLX_HOST_H
#define PLX_HOST_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define PLX_HOST_ABI_VERSION 2

/* Return codes of plx_host_start and plx_host_post. */
#define PLX_HOST_OK 0
#define PLX_HOST_E_ARGS (-1)
#define PLX_HOST_E_STATE (-2)
#define PLX_HOST_E_LAUNCH (-3)
#define PLX_HOST_E_WINDOW (-4)
#define PLX_HOST_E_THREAD (-5)
#define PLX_HOST_E_FULL (-6)
#define PLX_HOST_E_STALE (-7) /* a player event of an epoch that is not live: dropped */

/* Return codes of player_open and player_verb: 0 queued, negative refused. */
#define PLX_HOST_PLAYER_OK 0
#define PLX_HOST_PLAYER_E_UNAVAILABLE (-1) /* the host has no media player, or its video thread is not running */
#define PLX_HOST_PLAYER_E_BUSY (-2)        /* the host's request queue is full */
#define PLX_HOST_PLAYER_E_ARGS (-3)        /* epoch 0, a NULL or empty uri, an unknown verb */
#define PLX_HOST_PLAYER_E_STALE (-4)       /* a verb for an epoch that is not the host's current one */
/* PLAYER_ERROR.a >= this is a failure of the HOST (+1 the library's window was not found, +2 the display bind did not finish, +3 a request threw);
   anything below it is the native player's own error code. */
#define PLX_HOST_PLAYER_FAIL_BASE 2147418112

/* PlxHostEvent.kind */
#define PLX_HOST_EVENT_PAUSE 1  /* OnPause: the app went to the background */
#define PLX_HOST_EVENT_RESUME 2 /* OnResume: the app came back */
/* Player events (version 2); each carries PlxHostEvent.epoch. */
#define PLX_HOST_EVENT_PLAYER_PREPARED 16    /* a = duration ms */
#define PLX_HOST_EVENT_PLAYER_STARTED 17     /* the host's Start() ran */
#define PLX_HOST_EVENT_PLAYER_PAUSED 18      /* the host's Pause() ran */
#define PLX_HOST_EVENT_PLAYER_POSITION 19    /* a = ms; latest wins */
#define PLX_HOST_EVENT_PLAYER_SEEK_DONE 20   /* a = command id, b = ms; -1 refused, -2 expired (see player_verb) */
#define PLX_HOST_EVENT_PLAYER_BUFFERING 21   /* a = percent; latest wins */
#define PLX_HOST_EVENT_PLAYER_VIDEO_SIZE 22  /* a = width, b = height */
#define PLX_HOST_EVENT_PLAYER_COMPLETED 23   /* playback reached the end (the player's completed callback) */
#define PLX_HOST_EVENT_PLAYER_ERROR 24       /* a = the player's native error code */
#define PLX_HOST_EVENT_PLAYER_INTERRUPTED 25 /* a = native code: the system took the player */
#define PLX_HOST_EVENT_PLAYER_CLOSED 26      /* the player of this epoch is released */
#define PLX_HOST_EVENT_PLAYER_STALL 27       /* a position read has not returned for several seconds */

/* player_verb's verb */
#define PLX_HOST_PLAYER_VERB_PLAY 1
#define PLX_HOST_PLAYER_VERB_PAUSE 2
#define PLX_HOST_PLAYER_VERB_SEEK 3  /* arg0 = target ms, arg1 = command id */
#define PLX_HOST_PLAYER_VERB_CLOSE 4

typedef void (*PlxHostTask)(void *arg);
typedef void (*PlxOnStopped)(int32_t code, int32_t clean);
typedef int32_t (*PlxRunOnHost)(PlxHostTask task, void *arg);
typedef int32_t (*PlxPlayerOpen)(uint32_t epoch, const char *uri, int32_t x, int32_t y, int32_t w, int32_t h);
typedef int32_t (*PlxPlayerVerb)(uint32_t epoch, uint32_t verb, int64_t arg0, int64_t arg1);

typedef struct PlxHostApi {
    uint32_t size;        /* sizeof(PlxHostApi) as the host was built */
    uint32_t abi_version; /* PLX_HOST_ABI_VERSION */
    uint32_t flags;       /* reserved: 0 */
    uint32_t reserved;    /* reserved: 0 */
    PlxOnStopped on_stopped;
    PlxRunOnHost run_on_host;
    /* version 2: both or neither (NULL = no media player); read only when size >= PLX_HOST_API_V2_SIZE */
    PlxPlayerOpen player_open;
    PlxPlayerVerb player_verb;
} PlxHostApi;

typedef struct PlxHostEvent {
    uint32_t size; /* sizeof(PlxHostEvent) as the host was built */
    uint32_t kind; /* PLX_HOST_EVENT_* */
    /* version 2: player events only (a pause or a resume may stop at kind); read only when size >= PLX_HOST_EVENT_V2_SIZE */
    uint32_t epoch;    /* the player's epoch */
    uint32_t reserved; /* 0; keeps the two i64 words 8-aligned on every architecture */
    int64_t a;
    int64_t b;
} PlxHostEvent;

/* What size must cover for version 1: up to and including the last version-1 field. */
#define PLX_HOST_API_V1_SIZE ((uint32_t)(offsetof(PlxHostApi, run_on_host) + sizeof(PlxRunOnHost)))
#define PLX_HOST_EVENT_V1_SIZE ((uint32_t)(offsetof(PlxHostEvent, kind) + sizeof(uint32_t)))
/* ...and for version 2: up to and including the last version-2 field. */
#define PLX_HOST_API_V2_SIZE ((uint32_t)(offsetof(PlxHostApi, player_verb) + sizeof(PlxPlayerVerb)))
#define PLX_HOST_EVENT_V2_SIZE ((uint32_t)(offsetof(PlxHostEvent, b) + sizeof(int64_t)))

int32_t plx_host_start(const PlxHostApi *api, const char *launch_json);
int32_t plx_host_post(const PlxHostEvent *event);
void plx_host_stop(void);

#ifdef __cplusplus
}
#endif

#endif /* PLX_HOST_H */
