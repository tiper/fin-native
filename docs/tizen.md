# The Tizen build (Samsung television, emulator MVP)

`PLATFORM=tizen` builds PlxNative for a Samsung Tizen television as a **hosted library**: a shared object, `libplx.so`, that a small
.NET application (`tizen/host/PlxHost/`) loads and drives through three C functions. The design is `docs/platform-ports.md`
("Target shape", W9); this page is how to build, check, install and read what comes out. Nothing here is verified on a retail set.

## Scope of the MVP

- **Emulator only.** The one configuration that is built and checked is x86_64: the Tizen TV 10.0 emulator image, `tizen80`.
  An armv7 (retail) configuration is recorded in `docs/platform-ports.md` and refused by the makefile (`TIZEN_ARCH`) until a retail
  set has answered what it must (H1, H2 there).
- **No playback.** No backend is composed (`compose: backends=none`). On a detail page Resume is dimmed and cannot be focused, and the metadata line reads "Playback unavailable" (the log: `preview: blocked rk=... reason=Cached(RefusedDirect)`). On Home the hero's Play looks enabled, and pressing it opens the player route, which shows "Playback failed / The server cannot play or convert this file" with the footer `decision_refused` and logs no decision line: a known gap (below).
- **No frame-rate or latency claim.** The emulator's GL runs on llvmpipe through the emulator's GL forwarding; a number taken there
  says nothing about a television, and none is quoted.
- **A mock server, never a real account.** The package signs in against the mock Plex server (`tests/mock_pms.py`) with a placeholder
  token; the emulator reaches the build machine's loopback as `10.0.2.2`. There is no plex.tv sign-in in the MVP, so the session lives in
  memory (`MemoryOnly`) and nothing is written to anyone's Plex account.
- **Not a required check.** `.github/workflows/tizen.yml` proves the library links and passes its checks; it is compile-only, and the
  emulator run is by hand.

## What is where

| Path | What |
| --- | --- |
| `mk/platform-tizen.mk` | the build; the `Makefile` includes it only when `PLATFORM=tizen`, so every webOS and desktop `make` is untouched |
| `rust-modules/platform-tizen/` | the platform crate (`plx_platform_tizen`) and `include/plx_host.h`, the C contract, written once and read by both sides |
| `tizen/build-sdl.sh` | static SDL2 + SDL2_ttf from the pins `sdl2` and `sdl2-ttf` in `ci/dependencies.json` (no HarfBuzz) |
| `tizen/host/PlxHost/` | the .NET host: `PlxHost.csproj`, `tizen-manifest.xml`, `Program.cs` |
| `tizen/package.py` | stages the package's `lib/` and `res/`, refuses without signing material, packs and checks the `.tpk` |
| `ci/check-tizen-so.py` | the contract check: header against C# as text (`abi`), the linker's version script (`map`), the linked library (`verify`) |
| `tools/tizen-log-receiver.py` | the UDP log receiver, with a lost-datagram report |

Generated and never committed (`.gitignore`): `rust-modules/target-tizen/`, `tizen/out/`, `tizen/sdl-build/`, and under
`tizen/host/PlxHost/` the staged `lib/`, `bin/`, `obj/`, `res/appfont*.ttf`, `res/OFL.txt`, `res/dev-boot.json`,
`res/log-host.txt`, `res/THIRD-PARTY-NOTICES.md`, `res/licenses/` and the icon.

## The targets

Every target is `make PLATFORM=tizen <target>`. A bare `make PLATFORM=tizen` is `tizen-check` (the Makefile's own default goal
builds the webOS package; this platform has none).

| Target | Needs | What it does |
| --- | --- | --- |
| `tizen-abi` | any host, python3 | reads `plx_host.h` and `Program.cs` as text: the three `plx_host_*` functions, each one's return type and parameter count, both structs field for field (name, order, width), every function-pointer typedef against the host's `delegate` of the same name (return and parameter widths), and every `PLX_HOST_*` constant the host spells equal the header's (the host must spell the version and every event kind and verb) |
| `tizen-check` | any host with the target's std | `tizen-abi`, then `cargo check --lib` of the platform crate for `x86_64-unknown-linux-gnu`: compile-only, no link, no SDL |
| `tizen-sdl` | Linux x86_64, `cc`, `make`, `curl` | the static SDL2 + SDL2_ttf under `tizen/sdl-build/x86_64` (a stamp of the pins, the script and the flags skips a rebuild that would change nothing) |
| `tizen-so` | Linux x86_64, `libgles-dev` | `cargo rustc --crate-type staticlib` of the platform crate (features `tizen,devtriggers`, profile `tvdev`; under `RELEASE=1` profile `release` and features `tizen` only, so a release library carries no dev-trigger seam), then the link into `tizen/out/x86_64/libplx.so` |
| `tizen-verify` | Linux, binutils | `ci/check-tizen-so.py verify` on the linked library (below) |
| `tizen-stage` | python3 | stages `lib/libplx.so`, `res/` and the icon into the host project; `TIZEN_MOCK_HOST` and `TIZEN_LOG_HOST` become `res/dev-boot.json` and `res/log-host.txt` |
| `tizen-tpk` | the above, .NET 8 SDK, a Samsung certificate profile | the signed `tizen/out/PlxNative-x86_64.tpk`; refuses at once, naming the missing variables, without the profile |
| `print-tizen-config` | any host | the configuration, one `NAME=value` per line (the CI job prints it) |

`tizen-so` and `tizen-sdl` check in a recipe line that the host is Linux x86_64, so `make -n` resolves anywhere and a real run elsewhere
stops with the reason.

### What the link contains

The library is the `plx_platform_tizen` staticlib (the whole application behind one platform crate), `svg.o` (the nanosvg rasterizer the
UI's icons use) and a **static SDL2 + SDL2_ttf with its own FreeType**: a Tizen television ships no SDL, and the text path is SDL_ttf over SDL
surfaces. SDL never opens a window and its video subsystem is never initialised; the window, the EGL context and the keys are Tizen's own
(Ecore_Wl2). The link line (`mk/platform-tizen.mk`) is `cc -shared` with `-Wl,-soname,libplx.so`, `-z defs` (an undefined symbol fails this
link, not `dlopen` on the emulator: the library is loaded `BIND_NOW`), `-z relro -z now`, `--as-needed`, `--gc-sections`, and a **version
script generated from the header** that makes the three `plx_host_*` names global and everything else local, so no SDL, FreeType or Rust
symbol is exported. The link is written to `libplx.so.part` and renamed, so a failed link never leaves a half-written library for
`tizen-verify` to read.

**Notices.** The package statically links SDL2 and SDL2_ttf (zlib) and FreeType (the FreeType License, which requires the credit to be shown
in the documentation shipped with the product), so the `.tpk` carries `res/THIRD-PARTY-NOTICES.md` and `res/licenses/` (the project's own
`LICENSE`, `LICENSING.md`, `TRADEMARKS.md` and every `licenses/*.txt`, the set the `.ipk` ships); `tizen/package.py` stages them and
`verify_tpk` fails a package without the notices, `Zlib.txt` or `FreeType.txt`. `THIRD-PARTY-NOTICES.md` section 2.8 is this package's additional entry and states, heading by heading, which other sections of the file cover the `.tpk`; section 3 there describes the webOS package, so read 2.8 first.

### What `tizen-verify` checks

`DT_NEEDED` is an allowlist, not a deny-list:

- `libGLESv2.so.2` (required: the renderer's GL imports bind at link time, decision D3), `libc.so.6` (required), `libm.so.6`,
  `libdl.so.2`, `libpthread.so.0`, `librt.so.1`, `libutil.so.1`, `libgcc_s.so.1`, `ld-linux-x86-64.so.2`. The last four are what a Rust std
  link on `x86_64-unknown-linux-gnu` truly needs beyond the first five; `--as-needed` leaves out any a newer glibc has folded into libc.
- anything else (`libSDL2`, `libwayland-*`, `libudev`, `libX11`) fails: the static SDL link did not happen, or a configure picked up a
  library from the build machine that the television may not have.

Beyond that: the soname is `libplx.so` and the library has neither a text relocation nor a static-TLS flag; every undefined dynamic symbol is the
loader's (a versioned `GLIBC_*`/`GCC_*` symbol, never `GLIBC_PRIVATE`), an unversioned OpenGL ES 2.0 core function (the fixed `GLES2_CORE` list
in the script; libGLESv2 has no symbol versions, and an ES 3.0 name such as `glBindVertexArray` or `glBlitFramebuffer` is refused) or one of
the weak toolchain hooks; the **exported**
symbols are exactly `plx_host_start`, `plx_host_post` and `plx_host_stop`, equal to the header's functions and to the C# `DllImport` names;
`TTF_Init`, `TTF_OpenFont`, `TTF_RenderUTF8_Blended`, `FT_Init_FreeType` and `SDL_FreeSurface` are code symbols of the library and none of
`SDL_*`/`TTF_*`/`FT_*` is exported; and the highest `GLIBC_x.y` version needed is at most `PLX_MAX_GLIBC` (default 2.40, the emulator image's
glibc; the CI runner, ubuntu-24.04, links against 2.39). A newer build host is a failure that names the ceiling: link on an older host, raise `PLX_MAX_GLIBC`
if you know the target's glibc, or `PLX_ALLOW_NEW_GLIBC=1` to report and continue. The parsers and every rule are unit-tested on canned
`readelf`/`nm` text (`ci/test_tizen_so_check.py`), so the Linux-only check is exercised on any host that runs `make check`.

### The contract with the host

`rust-modules/platform-tizen/include/plx_host.h` is the single source. `PlxHost`'s `Native` class, its two structs and its
`PLX_HOST_*` constants are written from it, and `tizen-abi` (and `ci/test_tizen_so_check.py`, so `make check`) fails on any difference.
The thread rules, ownership, start/stop and the `launch_json` keys are the header's comment. What the host adds:

- `OnCreate` builds `launch_json` from `DirectoryInfo.Resource` (`payload_dir`), `DirectoryInfo.Data` (`state_dir`), the UI culture
  (`locale`), the Tizen platform version, `res/log-host.txt` (`log_udp`) and the members of `res/dev-boot.json` (`pms_host`, `pms_port`
  and the dev-only `dev` object), then calls `plx_host_start` once on the main thread. Without `res/dev-boot.json` the launch names no
  Plex server and the library refuses it (`PLX_HOST_E_LAUNCH`), which the host logs.
- `OnPause` / `OnResume` call `plx_host_post`, and then `PlxPlayer.PauseForHome` / `ResumeAfterHome` (below). A locale change is logged and **not forwarded**: the header has no locale event.
- `run_on_host` posts the library's task to the main thread's `SynchronizationContext` (captured in `OnCreate`) and runs it there.
  UNVERIFIED on a device: that Tizen's `CoreUIApplication` installs one on the main thread. If it does not, the host logs a warning at
  start and refuses every task, and the library drops the request (the window is freed by the process exit).
- `OnTerminate` calls `plx_host_stop` and waits up to 15 s for `on_stopped`. With `clean = 1` it releases the callback delegates; with
  `clean = 0` (or no report) it releases nothing. The host never calls libc `exit`; a failed start ends through `Exit()`.
- A failed library load is logged from C# (`DllNotFoundException`, a `dlopen` by absolute path with the loader's own `dlerror`,
  `EntryPointNotFoundException`, `BadImageFormatException`), because nothing else can say why the library never started.

### The media player (ABI version 2; the host side of W10)

Version 2 of the table adds two requests, `player_open(epoch, uri, x, y, w, h)` and `player_verb(epoch, verb, arg0, arg1)`, and the events
that answer them come back through `plx_host_post` with an `epoch` and two payload words. The header's PLAYER section is the contract.
There are still exactly three exports. A version-1 host, a table that stops at `run_on_host`, or a version-2 table with a NULL player pointer is
**accepted**: the library starts, logs `host: no media player: <why>` and plays nothing (the platform composes no URI driver for it). A host built
from this tree leaves both pointers NULL when `dlopen("libcapi-media-player.so.0")` fails.

| Piece | What it does |
|---|---|
| `tizen/host/PlxHost/PlxPlayer.cs` | the requests (validate, enqueue, return at once), one video thread that runs every native player call in order, a position poller (about every 250 ms while playing, off the request queue; `PLAYER_STALL` after 3 s without a reply), and the event posts. Every native callback is registered with the epoch as its `user_data`. |
| `tizen/host/PlxHost/PlayerNative.cs` | the P/Invokes of `libcapi-media-player.so.0` (`player_create`, `player_set_uri`, `player_set_ecore_wl_display`, `player_prepare_async`, `player_start`, ...), with the prior art cited: TizenFX's Interop and the field project nuvio's `Video.cs`. |
| `rust-modules/platform-tizen/src/hosted.rs`, `hosted/player.rs` | `Hosted::player_open` / `player_verb` (through the gate: refused after a stop, a request in flight holds the clean `on_stopped`), the live-epoch set and the player's own event queue (lossless but for position and buffering, which keep the latest; `Hosted::drain_player`). |

How the picture is meant to get on screen: the host binds the player to **the library's own Ecore_Wl2 window** as an OVERLAY
(`player_set_ecore_wl_display`), before prepare. It finds the window itself (`ecore_wl2_connected_display_get(NULL)`, then
`ecore_wl2_display_window_find(display, 1)`: the library makes exactly one window, the first), so the table carries no window handle; the host logs
`player: ecore window id 1 -> 0x...` and the library logs `ecore: window_new(...) -> 0x...`, and the two pointers must be equal. The id 1
assumption is the line to revisit if a second window is ever made first. The Ecore calls and the bind run on the host's main thread, and the video
thread waits for them for at most 3 s.

Measured on the emulator 2026-11-22 (TV 10.0 x86_64, a probe on a scratch branch, a local clip): the chain `player_create`, the three callbacks,
`player_set_uri`, `player_set_ecore_wl_display(OVERLAY, our window, 0,0,1920,1080)`, `player_set_display_mode(LETTER_BOX)`, `player_prepare_async`
(the prepared callback in 250-520 ms), `player_start`, `player_get_play_position` (advancing in real time), the completed callback at the end,
`player_stop`, `player_unprepare` and `player_destroy` returned `PLAYER_ERROR_NONE` throughout and the error callback never fired. The native
state stays PLAYING after the completed callback (until `player_stop`), so the host does not use the state to validate a completion. A player with no
display does not advance (position frozen), hence display-before-prepare. The prepared, buffering and completed callbacks fire on one non-main native
thread per player instance, so they only post events. `EnableExportingVideoFrame` is unsupported (no raw video). The existing `internet` privilege
was enough. With the window's alpha on the UI draws unchanged, and a picture shows only through a (0, 0, 0, 0) clear (next section).

### The video plane: the window's alpha, the hole and Home (W10 part B2)

Measured on the TV 10.0 x86_64 emulator, 2026-11-22 (a scratch probe; emulator only, no retail set):

- With the window's alpha **on** (`ecore_wl2_window_alpha_set(win, 1)`; this crate set 0 until part B2) the native player's OVERLAY video shows
  **only through the pixels the frame clears to (0, 0, 0, 0)**; the UI around it is intact. The window is therefore created translucent and stays so
  (`plx_platform_tizen::ecore::WINDOW_ALPHA`): it is a flag of the window, so nothing is routed per frame and the Tizen shell's `opaque_route` has no
  work. The pages with a ground clear to an opaque colour, so only the player route and the detail trailer preview (`frame_clear_through`) open the
  hole, the same shared chain webOS uses. **The player route's clear is keyed on the route, not on the plane**: the loop clears to `(0, 0, 0, 0)`
  whenever the player route is up (`fr.player`), bound or not, and `PlayerScreen` has no ground, so its spinner and its "Playback failed" read-out sit
  on a transparent clear even with no plane bound. The straighten pass and the capture skip are the ones keyed on `Player::video_plane_bound`.
- The **video plane mixes STRAIGHT alpha** (pixel-exact: a straight (255, 255, 255, 128) strip is a 50 % veil over the video) while window-over-window
  composition is premultiplied: the split webOS has, which `plx_gfx` already handles. Over the bound plane the loop's last draw is the straighten pass
  (`gfx/straighten.rs`), which divides the frame's premultiplied colour by its alpha; off the plane the frame goes out premultiplied, which is what the
  window compositor wants.
- **The straighten program did not build on YaGL** (the emulator's GLES2): `shader compile FAILED: 0:37(1): ... unexpected NEW_IDENTIFIER` at the
  `precision` statement of the copy variant, then `straighten: no program`. The emulator has neither framebuffer-fetch extension, so the copy variant
  is its program. The one source held all three variants behind `#if defined(...)` and two `#extension ... : require` lines (the suspect recorded in
  `docs/platform-ports.md`); the copy variant is now a file of its own, `fs_straighten_copy.frag`, with no preprocessor directive, and
  `fs_straighten.frag` keeps only the two fetch variants. The arithmetic is the same text in both and a test holds it equal. **After the split the
  emulator run at `3d56d35b2` (below) logs no compile failure and `straighten: no framebuffer fetch - the pass copies the frame`.** What was never
  established is WHY YaGL rejected the shared source (the `#extension` lines were the suspect; the split removes them together with the `#if`
  branches, so it does not say which of the two it was). Whether the straightened picture is right over a bound plane is not measured: no plane is
  bound until part B1.
- **Home round trip.** The native player raises nothing when the app goes to the background. Pausing at `OnPause` (`player_pause`) and
  `player_start` at `OnResume` resumes in about 130 ms at the same position; the picture is back when the window is (about 2 s). `PlxPlayer`
  does exactly that on its video thread: at `OnPause` a PLAYING player is paused (`PLAYER_PAUSED` is posted) and a start requested before `PREPARED` is
  held back; at `OnResume` exactly that is started again through the library's PLAY path (`PLAYER_STARTED`). A player the library paused, closed or
  replaced in between is left alone. `PLAYER_PAUSED` / `PLAYER_STARTED` are the events the library already understands, so a host pause shows as an
  ordinary pause then play.
- A `CLOSE` the host accepts now retires its epoch at the door (`Interlocked.CompareExchange(ref latestEpoch, 0, epoch)`): a later PLAY, PAUSE or SEEK
  for it is `E_STALE` and never queued, as `plx_host.h` promises.

**Emulator run of this part, 2026-11-22, at `3d56d35b2`** (the Windows session; emulator only):

- The `.tpk` built with MSBuild 0 warnings and 0 errors, and `check-tizen-so` passed.
- **No shader compile failure.** The log shows `straighten: no framebuffer fetch - the pass copies the frame`: the copy program now builds on YaGL.
- The UI is opaque and correct at alpha 1 (Home, detail, back), a clean stop (`on_stopped code=0 clean=1`), 0 log gaps.
- A Home round trip resumes the same pid and comes back drawn.
- **The unbound player route:** hero Play opens "Playback failed" (no URI driver yet, so the plane is never bound), and behind it the emulator shows exactly
  (0, 0, 0) black (5778 of 5784 samples, the rest the footer text), unchanged from +0.49 s to +6.86 s; the text and pills render cleanly, and Back returns to an
  opaque Home. The emulator's compositor had nothing under the window: what a retail set composites under a `(0, 0, 0, 0)` clear with no player is
  unmeasured and belongs to H6.

Still UNVERIFIED: **Home with a player open** (`PauseForHome` and `ResumeAfterHome` were never exercised, because no player existed), a picture through the
real URI driver (part B1, which calls `Hosted::player_open`: the library still composes no backend, so this part plays nothing), and anything on a retail
set (H6 stays OPEN: the library's thread, the overlay beneath an alpha window, privileges, firmware differences). The C# is built by no CI job
(`.github/workflows/tizen.yml` runs `tizen-abi` and `tizen-so`, which links the Rust library and never builds `PlxHost`); it is compiled by a
`make PLATFORM=tizen tizen-tpk` on a machine with the .NET SDK (the run above did that for this tree).

The application and package id is `plxntizen0` (ten alphanumerics): a neutral name for the emulator package. A retail package id is
assigned by Samsung's seller process, not chosen here.

## Toolchain (a Linux x86_64 build machine)

WSL2 Ubuntu 22.04 or newer is enough. Versions come from the repository's pins; none is restated here.

- the checkout on the branch named for the run, and `rustup` with the toolchain pinned in `rust-modules/rust-toolchain.toml`
  (the plain host target, no cross compiler; `make` installs nothing);
- `gcc`, `make`, `curl`, `tar`, `python3` (3.10 is enough) and binutils (`apt install build-essential curl python3`);
  `pkg-config` is not required (a "could not discover SDL2" warning from the crates' build scripts is expected and harmless);
- `apt install libgles-dev`: the link stub for `libGLESv2.so`. Keep `libsdl2-dev` and `libsdl2-ttf-dev` off a build machine that
  only builds the library (the host unit tests link them, CI installs them); the library links its own static SDL by path and
  `tizen-verify` fails on a `libSDL2` in `DT_NEEDED` either way;
- a checkout path without spaces (SDL's autoconf cannot build there; under WSL use a path in the Linux filesystem, not `/mnt/c/...`);
- network to `raw.githubusercontent.com` (the nanosvg headers) and the SDL release hosts: every one of those downloads is pinned and
  sha256-verified by `ci/deps.py` and cached machine-wide;
- for the `.tpk`: a user-local .NET 8 SDK (`dotnet-install.sh --channel 8.0 --install-dir ~/.dotnet`, with `DOTNET_ROOT` if elsewhere),
  `sdb`, network to nuget.org and the Samsung certificate profile below. The NuGet packages are exact-version pins in `PlxHost.csproj`
  (`Tizen.NET` 8.0.0.15631, `Tizen.NET.Sdk` 1.1.9; `Tizen.NET.API8` comes in transitively), resolved from nuget.org: they are NOT
  sha256-verified by this repository (NuGet checks its own package signatures and hashes, nothing here pins them);
- the emulator: Tizen SDK 10.0 CLI with TV Extension 10.0 (Tizen Studio, the IDE, is deprecated), the TV 10.0 x86_64 image under KVM,
  started with `LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe`.

### Signing

A Tizen television or emulator installs only a package signed with a **Samsung certificate profile** (an author certificate and a
distributor one bound to the device's DUID). The SDK's bundled default certificates are expired and are never used. Four
environment variables name the profile; `tizen/package.py` is the only reader, `tizen-tpk` runs its check first (before any build), and
the passwords are never on a command line, in a file of this repository or in make's echo (they reach `dotnet build` as MSBuild
environment properties of that one process):

```
PLX_TIZEN_AUTHOR_P12=/path/author.p12    PLX_TIZEN_AUTHOR_PASS=...
PLX_TIZEN_DIST_P12=/path/distributor.p12 PLX_TIZEN_DIST_PASS=...
```

`tizen-so`, `tizen-verify` and every CI step need none of this. No certificate, profile, password or DUID is committed or printed.

## Build, install, launch

```sh
python3 tests/mock_pms.py --host 127.0.0.1 --port 32499         # the mock Plex server
python3 tools/tizen-log-receiver.py --port 9999                   # the log receiver (a second terminal)

make PLATFORM=tizen tizen-so
make PLATFORM=tizen tizen-verify                                  # report its output
make PLATFORM=tizen tizen-tpk TIZEN_MOCK_HOST=10.0.2.2:32499 TIZEN_LOG_HOST=10.0.2.2:9999

sdb install tizen/out/PlxNative-x86_64.tpk
sdb shell 0 execute plxntizen0                                    # or launch "PlxNative" from the app list
```

`TIZEN_MOCK_HOST` and `TIZEN_LOG_HOST` are inputs of the **package**, not of the library: they go to `tizen-tpk` (or `tizen-stage`),
and a value that is absent writes no file and removes a stale one, so the package is exactly what the command asked for. They are
addresses, never credentials: `res/dev-boot.json` carries a placeholder token that only the mock accepts.

On the TV 10.0 emulator `sdb shell <cmd>` prints nothing (only `sdb shell 0 <cmd>` works), `sdb dlog` is empty and `sdb pull` of the app's
data directory is refused, so the log leaves the device over UDP. A retail television has no `sdb shell` and no dlog at all.

### The log channel

With `TIZEN_LOG_HOST=IPV4:PORT` the host writes every line as one UDP datagram, `h<seq> <line>`, and hands the address to the library as
`log_udp` for its own lines. `seq` counts per sender, so a gap is a lost datagram; `tools/tizen-log-receiver.py` prints the report
(`-- N datagram(s) lost before ...`) and ends with a count: a run whose log is claimed as evidence ends with `lost 0`. The address is an
IPv4 literal (no name lookup in a logger). The host also writes `<data>/plx-host.log` and dlog tag `PLXHOST`.

## What success looks like

In the receiver, in order (the host's lines are fixed by `Program.cs`; the library's wording is its PR's and is not repeated here):

1. `host started: OnCreate on the main thread; ...`, the `network log: UDP -> ...` line, the runtime and the platform lines, and
   `native library .../lib/libplx.so exists, N bytes`;
2. `dev boot: the members of .../dev-boot.json ...` (no `dev boot: none`);
3. `plx_host_start OK; the library's loop is running`, and no `plx_host_start FAILED`, `DllNotFoundException` or `EntryPointNotFoundException`;
4. the library's own lines: the platform identity (Tizen, emulator, no playback), the mock server's identity, the GL line without a failure,
   Home reached with the mock's libraries, Resume dimmed on a detail page with `preview: blocked ... reason=Cached(RefusedDirect)` (the Home hero's Play is a known gap, below), and each remote key logged as an action;
5. no sequence gap in the receiver, and no crash lines;
6. a Home-button round trip logs `OnPause` then `OnResume` and the screen redraws; closing the app logs `OnTerminate` and ends with
   `on_stopped code=0 clean=1`.

State plainly what was not seen. The log is the evidence. A host-side window capture of the emulator does show the GL UI (a Windows
`PrintWindow` capture showed Home, the detail page and the focus ring), so a capture is a record of the screen to attach beside the log.

## Known gaps seen on the emulator (2026-11-21)

- The Home hero's Play looks enabled; pressing it opens the player route ("Playback failed / The server cannot play or convert this file", footer `decision_refused`) and logs no decision line. That route clears to `(0, 0, 0, 0)` over a translucent window: the emulator shows black behind the read-out (above); a retail set is unmeasured (H6).
- (Fixed in part B2.) `shader compile FAILED: 0:37(1): error: syntax error, unexpected NEW_IDENTIFIER` on YaGL, followed by `straighten: no program`, was seen on 2026-11-21. Part B2 gave the copy variant a source of its own ("The video plane", above) and the 2026-11-22 run at `3d56d35b2` logs `straighten: no framebuffer fetch - the pass copies the frame` instead.
- webOS-only lines are still printed: `devcaps: ... assuming the 49SM9000PLA profile`, `surface: webOS major 0`, the player footer `webOS unknown — os_info.json unreadable · unknown set`, and three `ff: ... MISSING ... reinstall the package` lines, whose advice does not apply to this package.
- Closing the app logs `surface: the host asked the library to stop, so the window teardown is not requested; it lives until the process ends`.
- **The host's self-exit path is unexercised on the emulator.** When the library ends without a host stop (an `Exit` the platform does not intercept, or a
  `Shell::open` refusal after an OK start), `Program.cs` queues the application's own `Exit()` on the main thread; that decision (`ShouldExitHost`) is
  not unit-tested either. The remote's Exit key reaches the library as `Exit Down`, but the platform treats Exit as a system key: it sends `OnPause` at
  once and `OnTerminate` about 365 ms later, so the host's stop is already requested when `on_stopped` arrives; Back on Home does not quit.
  No remote route makes the library end by itself there.
- The image cache's candidate directories (`plx_base::paths::image_cache_candidates`) are webOS paths, so the log reads `imgcache: no writable directory; disk cache unavailable` and the disk cache is off: a shared-path follow-up.

## CI

`.github/workflows/tizen.yml`, job `tizen-so` ("Tizen library (link and verify, no emulator)"), on `ubuntu-24.04`, only for the
paths it names (`rust-modules/**` is one: the library is the whole application, so a new `extern` symbol anywhere is this link's
undefined symbol). It prints the configuration, runs `tizen-abi`, `tizen-so` and `tizen-verify`, then
`cargo test --lib -p plx_platform_tizen --features tizen` from `rust-modules`: the pattern of `ci.yml`'s host unit job, which already
runs the crate with its default features (an empty library), so this pass adds the crate's own `tizen` tests. It uses no emulator, no
.NET SDK and no certificate, is **not** a required check, and branch protection is not edited.

## Verifying that webOS is unchanged

`Makefile` is a prerequisite of the ARM library, so a plain dry run would differ in what it rebuilds; compare with `-B` on both sides, from
two checkouts (a scratch `git worktree` of the base, never a stash):

```sh
git worktree add ../plx-base <base>
make -C ../plx-base -B -n > /tmp/make-n.base
make -B -n > /tmp/make-n.head
diff /tmp/make-n.base /tmp/make-n.head && echo identical
make PLATFORM=tizen -n tizen-so                                   # resolves on any host
```

The Tizen makefile is read only under `PLATFORM=tizen`.
