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
 *   the table was not accepted. A later field of a bigger table is read only when size covers it.
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
 *              "pms_host"          string: the Plex server the application boots against
 *              "pms_port"          integer 1..65535
 *   optional   "locale"            string, a BCP-47 tag, default "en-US"
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
 *   started, or a stop was requested); PLX_HOST_E_ARGS, a NULL event, a size below PLX_HOST_EVENT_V1_SIZE or
 *   an unknown kind.
 *   A pause or a resume is one transition: the application is told once per change of state. A pause while
 *   paused, and a resume while running (the host's resume at launch), are accepted and make no event.
 */
#ifndef PLX_HOST_H
#define PLX_HOST_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define PLX_HOST_ABI_VERSION 1

/* Return codes of plx_host_start and plx_host_post. */
#define PLX_HOST_OK 0
#define PLX_HOST_E_ARGS (-1)
#define PLX_HOST_E_STATE (-2)
#define PLX_HOST_E_LAUNCH (-3)
#define PLX_HOST_E_WINDOW (-4)
#define PLX_HOST_E_THREAD (-5)
#define PLX_HOST_E_FULL (-6)

/* PlxHostEvent.kind */
#define PLX_HOST_EVENT_PAUSE 1  /* OnPause: the app went to the background */
#define PLX_HOST_EVENT_RESUME 2 /* OnResume: the app came back */

typedef void (*PlxHostTask)(void *arg);
typedef void (*PlxOnStopped)(int32_t code, int32_t clean);
typedef int32_t (*PlxRunOnHost)(PlxHostTask task, void *arg);

typedef struct PlxHostApi {
    uint32_t size;        /* sizeof(PlxHostApi) as the host was built */
    uint32_t abi_version; /* PLX_HOST_ABI_VERSION */
    uint32_t flags;       /* reserved: 0 */
    uint32_t reserved;    /* reserved: 0 */
    PlxOnStopped on_stopped;
    PlxRunOnHost run_on_host;
} PlxHostApi;

typedef struct PlxHostEvent {
    uint32_t size; /* sizeof(PlxHostEvent) as the host was built */
    uint32_t kind; /* PLX_HOST_EVENT_* */
} PlxHostEvent;

/* What size must cover for version 1: up to and including the last version-1 field. */
#define PLX_HOST_API_V1_SIZE ((uint32_t)(offsetof(PlxHostApi, run_on_host) + sizeof(PlxRunOnHost)))
#define PLX_HOST_EVENT_V1_SIZE ((uint32_t)(offsetof(PlxHostEvent, kind) + sizeof(uint32_t)))

int32_t plx_host_start(const PlxHostApi *api, const char *launch_json);
int32_t plx_host_post(const PlxHostEvent *event);
void plx_host_stop(void);

#ifdef __cplusplus
}
#endif

#endif /* PLX_HOST_H */
