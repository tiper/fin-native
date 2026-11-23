# The Tizen build for Samsung TVs and the TV emulator

`PLATFORM=tizen` builds PlxNative for a Samsung Tizen television as a **hosted library**: a shared object, `libplx.so`, that a small
.NET application (`tizen/host/PlxHost/`) loads and drives through three C functions. The design is `docs/platform-ports.md`
("Target shape", W9); this page is how to build, check, install and read what comes out. Nothing here is verified on a retail set.

## Scope of the MVP

- **Two builds.** `TIZEN_ARCH=x86_64` targets the Tizen TV 10.0 emulator. `TIZEN_ARCH=armv7` builds the
  retail SUPER ALPHA package for 2021+ Samsung TVs (Tizen 6.0+). Both use the `tizen80` managed target.
  A retail TV still needs to run it (H1, H2 and H6 in `docs/platform-ports.md`).
- **Playback is the `uri` feature's. The driver has run on the Windows emulator against the mock server, and has NOT been run on a retail set (H6 stays OPEN).** The first emulator run (at `126d1f04c`) found faults A-E ("Findings of the first emulator run", below); runs at `fc5d28594` and `b62e1276e` passed those fixes and round 4's and found what review round 5 fixes. Every fix since a run is "fixed in code, emulator re-run pending" until a run passes it. A build with the platform crate's `uri` feature (the makefile's default; `TIZEN_URI=0` builds without it) composes the Tizen URI driver over the host's native player (below): for a title the planner can offer it, the library asks the host to play a progressive URI source. **Which titles those are is a narrow set, and nothing here says one plays**: the planner refuses most real titles before the Tizen qualifier is asked ("The refusal set", below), and a source with more than one audio stream is never certified. A build without `uri`, or a host with no player group in its table, composes no backend (`compose: backends=none`): on a detail page Resume is then dimmed and cannot be focused, and the metadata line reads "Playback unavailable" (the log: `preview: blocked rk=... reason=Cached(RefusedDirect)`); on Home the hero's Play looks enabled, and pressing it opens the player route, which shows "Playback failed / The server cannot play or convert this file" with the footer `decision_refused` and logs no decision line: a known gap (below). What a `uri` build has NOT shown: a picture through this driver (the window's alpha and the straighten pass are part B2's, below; the library emits no `Presented` for it), and Home with a player open. H6 (below) is open.
- **No frame-rate or latency claim.** The emulator's GL runs on llvmpipe through the emulator's GL forwarding; a number taken there
  says nothing about a television, and none is quoted.
- **Sign-in.** Published packages start at the Plex sign-in screen. Sign-in is held in memory
  (`MemoryOnly`) and starts again on each launch. Emulator development can opt into a mock Plex server
  (`tests/mock_pms.py`) and placeholder token through `TIZEN_MOCK_HOST`, with the host loopback at `10.0.2.2`.
- **CI checks.** `.github/workflows/tizen.yml` links the emulator library and builds and verifies the ARMv7 package.
  The emulator run is manual.

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

Every target is `make PLATFORM=tizen <target>`. A bare `make PLATFORM=tizen` is `tizen-check`; choose `tizen-tpk`
or `tizen-tpk-nightly` to create a package. The webOS package goals (`all`, `ipk`, `deploy`, `install`, the storage helper and the C
units, `WEBOS_TV_GOALS` in the `Makefile`) are refused under `PLATFORM=tizen` with a message, so `make PLATFORM=tizen ipk` cannot package
the television build by accident; the queries (`print-*`) answer under every `PLATFORM`.

| Target | Needs | What it does |
| --- | --- | --- |
| `tizen-abi` | any host, python3 | reads `plx_host.h` and `Program.cs` as text: the three `plx_host_*` functions, each one's return type and parameter count, both structs field for field (name, order, width), every function-pointer typedef against the host's `delegate` of the same name (return and parameter widths), and every `PLX_HOST_*` constant the host spells equal the header's (the host must spell the version and every event kind and verb) |
| `tizen-check` | any host with the target's std | `tizen-abi`, then `cargo check --lib` of the platform crate for the selected target |
| `tizen-sdl` | host tools and the selected compiler | the static SDL2 + SDL2_ttf under `tizen/sdl-build/<arch>` (a stamp of the pins, the script and the flags skips a rebuild that would change nothing) |
| `tizen-so` | Linux x86_64 with `libgles-dev`, or the ARM NDK | `cargo rustc --crate-type staticlib` of the platform crate (features `tizen,uri,devtriggers`, profile `tvdev`; under `RELEASE=1` profile `release` and features `tizen,uri` only, so a release library carries no dev-trigger seam), then the link into `tizen/out/<arch>/libplx.so` |
| `tizen-verify` | GNU binutils (from the NDK on ARM) | `ci/check-tizen-so.py verify` on the linked library (below) |
| `tizen-stage` | python3 | stages `lib/libplx.so`, `res/` and the icon into the host project; `TIZEN_MOCK_HOST` and `TIZEN_LOG_HOST` become `res/dev-boot.json` and `res/log-host.txt` |
| `tizen-tpk` | the above, .NET 8 SDK, a Samsung certificate profile | the signed `tizen/out/PlxNative-x86_64.tpk`; refuses at once, naming the missing variables, without the profile |
| `tizen-tpk-nightly` | the above, .NET 8 SDK, OpenSSL, `RELEASE=1` | builds and verifies `plxnative-tizen-v<label>-<arch>.tpk` with throwaway signing |
| `print-tizen-config` | any host | the configuration, one `NAME=value` per line (the CI job prints it) |

The emulator link runs on x86_64 Linux. The ARM build uses the pinned webOS NDK on macOS or arm64
Linux and the `armv7-unknown-linux-gnueabi` Rust target. Run `make setup-env` to install both.

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
  `libdl.so.2`, `libpthread.so.0`, `librt.so.1`, `libutil.so.1`, `libgcc_s.so.1`, and the matching loader (`ld-linux-x86-64.so.2` for x86_64 or `ld-linux.so.3` for ARM).
  `--as-needed` leaves out any a newer glibc has folded into libc. The ARM link must be ELF32 ARM EABI5
  soft-float and stays below glibc 2.30.
- anything else (`libSDL2`, `libwayland-*`, `libudev`, `libX11`) fails: the static SDL link did not happen, or a configure picked up a
  library from the build machine that the television may not have.

Beyond that: the soname is `libplx.so` and the library has neither a text relocation nor a static-TLS flag; every undefined dynamic symbol is the
loader's (a versioned `GLIBC_*`/`GCC_*` symbol, never `GLIBC_PRIVATE`), an unversioned OpenGL ES 2.0 core function (the fixed `GLES2_CORE` list
in the script; libGLESv2 has no symbol versions, and an ES 3.0 name such as `glBindVertexArray` or `glBlitFramebuffer` is refused) or one of
the weak toolchain hooks; the **exported**
symbols are exactly `plx_host_start`, `plx_host_post` and `plx_host_stop`, equal to the header's functions and to the C# `DllImport` names;
`TTF_Init`, `TTF_OpenFont`, `TTF_RenderUTF8_Blended`, `FT_Init_FreeType` and `SDL_FreeSurface` are code symbols of the library and none of
`SDL_*`/`TTF_*`/`FT_*` is exported. On x86_64 the highest `GLIBC_x.y` version needed is at most `PLX_MAX_GLIBC` (default 2.40, the emulator image's
glibc; the CI runner, ubuntu-24.04, links against 2.39). A newer build host is a failure that names the ceiling: link on an older host, raise `PLX_MAX_GLIBC`
if you know the target's glibc, or `PLX_ALLOW_NEW_GLIBC=1` to report and continue. The parsers and every rule are unit-tested on canned
`readelf`/`nm` text (`ci/test_tizen_so_check.py`). The ARM check also compares every required import and
library name against `ci/testdata/tizen60-armv7-abi.json`. Its exports come from the Tizen 6.0 ARM
rootstrap and the matching Tizen SDK compiler's `libgcc_s.so.1`. Both archives and their SHA-256
digests are in `ci/dependencies.json` under `tizen60-abi-reference`. The inventory covers link-time
imports. Runtime libraries loaded by name still need device verification.

### The contract with the host

`rust-modules/platform-tizen/include/plx_host.h` is the single source. `PlxHost`'s `Native` class, its two structs and its
`PLX_HOST_*` constants are written from it, and `tizen-abi` (and `ci/test_tizen_so_check.py`, so `make check`) fails on any difference.
The thread rules, ownership, start/stop and the `launch_json` keys are the header's comment. What the host adds:

- `OnCreate` builds `launch_json` from `DirectoryInfo.Resource` (`payload_dir`), `DirectoryInfo.Data` (`state_dir`), the UI culture
  (`locale`), the Tizen platform version, `res/log-host.txt` (`log_udp`) and the members of `res/dev-boot.json` (`pms_host`, `pms_port`
  and the dev-only `dev` object), then calls `plx_host_start` once on the main thread. Without
  `res/dev-boot.json` the app starts the ordinary sign-in flow.
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
  branches, so it does not say which of the two it was). Whether the straightened picture is right over a bound plane is not measured: the driver has run on the emulator since (the Windows session's later runs), but no reading of the picture itself was taken there, and a retail set is H6's.
- **Home round trip.** The native player raises nothing when the app goes to the background. Pausing at `OnPause` (`player_pause`) and
  `player_start` at `OnResume` resumes in about 130 ms at the same position; the picture is back when the window is (about 2 s). `PlxPlayer`
  did that on its video thread in part B2: at `OnPause` a PLAYING player is paused (`PLAYER_PAUSED` is posted) and a start requested before `PREPARED` is
  held back. **Part B1's review round changed the resume half**: the library owns it (it parks the attempt with a CLOSE at the background and re-opens
  the same source at the saved position at the foreground, see the first emulator run's findings below), so `OnResume` starts nothing and only logs
  why it resumed nothing.
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

At the time of this run the player was not yet bound, so `PauseForHome` and `ResumeAfterHome` had not been exercised with a player open: the first run of part B1's driver (at `126d1f04c`, below) did, and found that Home with a player open never resumed (fixed since in code, re-run pending). Still UNVERIFIED: a picture through the
real URI driver (part B1, which calls `Hosted::player_open`; the later emulator runs drove it against the mock, and no reading of the picture was taken), and anything on a retail
set (H6 stays OPEN: the library's thread, the overlay beneath an alpha window, privileges, firmware differences). The C# is built by no CI job
(`.github/workflows/tizen.yml` runs `tizen-abi` and `tizen-so`, which links the Rust library and never builds `PlxHost`); it is compiled by a
`make PLATFORM=tizen tizen-tpk` on a machine with the .NET SDK (the run above did that for this tree).

### The Rust driver over it (the `uri` feature, W10 part B)

`rust-modules/platform-tizen/src/player/` holds `TizenUriDriver`, a Sans-IO `DriverSet` + `UriDriver` over a `HostPlayer` port that `hosted` implements: it
starts no worker and reads no global, its requests go out as `Hosted::player_open` / `player_verb` and every answer returns as one addressed event
through `drain_player`. Every request and event carries the epoch, and an event of any other epoch (a completion after a stop) is dropped and counted.
The driver states `progressive_uri`, `pause`, `seek` and `audio_track`, and no HLS, packet feed, subtitle track, audio-off or subtitle-off. `audio_track` has a narrow
meaning: the host ABI has no track verb, and which audio stream the retail player starts on when a source has several is unmeasured, so the qualifier certifies only a
source with ONE audio stream (`!has_multiple_audio_tracks`, the webOS precedent `ManyAudioTracksExported`); the planner needs the flag because it issues a `SelectTrack`
for any title whose server-selected audio stream is stated, and the driver settles that request for the one stream **by construction** (there was nothing to choose between), not
because the native player confirmed anything. A request for any other stream is refused `Unsupported` and tears nothing down. Stop sends the CLOSE verb and waits for the host's `CLOSED`; past a deadline it reports a
quarantined release and the process-exit disposition stays Preserve. The platform's own `TizenQualifier` certifies a source only where Standard would
direct-play it (so DTS and TrueHD, and every container Standard hands to the server, are refused with the planner's typed reason) and never a converted PMS
output (`qualify_pms` is Unknown); the codec table behind Standard is still `Caps::assumed()`, the 49SM9000PLA row, not a Tizen measurement. The composition
(`player::MEDIA`) replaces `host::NO_MEDIA` in a `uri` build; it installs the driver only when the host's table offers a player and the run is not a controlled
one, and returns the resolver only when the adapter accepted the driver. It names no LG binding and publishes no default backend.

**The refusal set.** Before the Tizen qualifier is asked, the planner's own preflight (`ResolvePort::refusal`, final for the play) refuses an original for: a selected rate other than 1x; a quality ceiling or Direct play rule; a Plex relay; Auto on a link not known to be local; Boost Dialog or Normalize Loudness (`audio_contract.any()`); a burned subtitle; a client-drawn (sidecar) subtitle; and, most often, **`SubtitleOff`**: the planner sets `subtitle_choice = Off` whenever the item has ANY subtitle stream and none is selected (`plan.rs`), and no driver here can switch the native player's default subtitle off. A selected embedded subtitle fails the driver's `subtitle_track: false`. With no Feed and `qualify_pms` Unknown, each of these ends as the planner's typed `NoQualifiedBackend` refusal, not as a play. So a title plays through this driver only if it has no subtitle streams (or none that the planner treats as unselected), one audio stream, a container and codecs Standard direct-plays, local Auto or a fixed quality, and no audio enhancement. A resolver test pins `subtitle_choice: Off` to `SubtitleOff` and the others to their reasons. **Open measurement:** whether Tizen's native player renders an embedded subtitle when none is set (which would decide whether `SubtitleOff` could be allowed); until it is measured it is NOT allowed.

**A pick the driver cannot carry does not replace the playing title.** Picking another audio or subtitle track mid-play on a URI attempt that cannot switch natively used to re-plan the play, which retires the playing item first and, here, can only end in `NoQualifiedBackend`. The shared door (`route::request_viewer_reconfigure`) now asks `PlaybackSession::source_track_change_carriable()` first: true with a Feed (webOS) or when the URI resolver's provider says it can certify such an output (`OriginalQualifier::carries_source_track_change`, default true; `TizenQualifier` says false), and a pick nothing can carry is refused with `ViewerReconfigureRefusal::NoBackendCarriesTrack` before anything is retired or spent. **The Quality ladder is the same kind of request** (a rung asks the server to convert, and `request_transcode_refresh` only queues a `Retranscode` that the Feed engine's pump consumes): `PlaybackSession::quality_pick_carriable()` (the same provider flag) makes the More menu not offer the Quality rows and `set_quality` refuse a rung before it persists it or moves the route contract; unrefused, the pick left the title playing unchanged and the next re-resolve of it (a Home restore, a stall retry) planned under the new ceiling and ended in `NoQualifiedBackend`.

**Findings of the first emulator run of the driver (the Windows session, at `126d1f04c`; emulator and mock data only).**

- **Home with a player open never resumed (fixed in code; the emulator re-run is pending).** The host's `PauseForHome` paused the player, but `ResumeAfterHome` found nothing to resume, and the library's own restore opened nothing. The library parks a URI attempt at the background (`park_playback`: `Stop(Suspend)`, the CLOSE the host confirms), so by `OnResume` the host's epoch is `Closing`. The foreground restore then reached `begin_resume`, whose prelude answered `NoRoute` because a URI original plan installs no `PlaybackSession::url` (`plan.uri_source_ready`: the source travels in the offer the driver consumed). Reproduced on the host before the fix (a hostsim test over the webOS `play-uri` driver's rig, `a_home_round_trip_reopens_a_uri_original_attempt_at_the_saved_position`: the restore never opened a second native session). **One owner, the library**: `begin_resume`'s prelude treats a parked URI original (`route::uri_restore_owed`: no URL, a source selection, the request) as a re-resolve, not a rebuild. It answers `Pending`, the frame performs it with the metadata store the lifecycle machine lacks (`player::pump_uri_restore`, `route::restore_uri_original`: the failed start transaction is ended, the old logical resource retired, and the SAME item is resolved again at the saved position as a new attempt of the same request, origin `Resume`), and the offer that lands re-opens the driver (`land_resume` hands the lifecycle `Prepared`, whose `Load` then starts it). A re-plan that falls to a Feed route hands over to the ordinary resume. A refused re-resolve answers `RebuildRejected` and leaves the session parked for the next foreground with the read-out up. The host no longer starts a player at `OnResume` (it would race the restore): it only pauses at `OnPause` and logs why an `OnResume` resumed nothing; `plx_host.h` states the ownership. **webOS:** the same code path serves the webOS URI original (`play-uri`), whose plan installs no URL either, so the fix covers it and the test above runs on that driver; webOS Feed sessions always have a URL, never read as owed, and their restore is unchanged. A paused save is re-paused by the app's existing `repause_at` after the Load, not by `InitialPlayback`. **The parked attempt's stop (round 4, fixed in code; the emulator re-run is pending):** the park reports nothing (the webOS Feed's suspend reports nothing either: its engine goes on under the same session), but the restore retires the parked session, and that session's `stopped` timeline used to go out only as a side effect of the REPLACEMENT's first report (the retired-session mechanism of #1236 posts it at the new report's position): the mock saw `timeline A playing 9377`, `playQueue B`, `transcode stop A`, `timeline A stopped time=15588`, `timeline B playing 15588`. `route::restore_uri_original` now takes the parked attempt's final report (`parked_final_report`: its own last position and duration, `begin_teardown`'s rule) and sends it through the ordered stop worker (`scrobble_stop`) before the second resolve starts, which also stops its transcode behind it; hostsim `a_restore_reports_the_parked_attempt_stopped_at_its_own_position_before_the_next_play` fails without it. Not run: the Tizen emulator (the re-run is pending), a retail set.
- **A frozen picture was reported as playing (fixed).** A player that posts POSITION with a position that does not advance while the driver believes it is Playing is now a stall: after `frozen_grace_ms` (a pause suspends it, a seek or a Started restarts it) the driver reports `Rebuffering(true)` and the ordinary stall deadline applies. A host test feeds a frozen position and a paused one.
- **Terminating the app with a player open was not a clean stop (fixed in the library and the host; the C# is uncompiled here).** `OnTerminate` waited for `on_stopped` while the player was still open; a close needs the host. The library now answers a CLOSE it cannot send after `plx_host_stop` as `Refusal::Stopping`, which releases the attempt (the host owns the close at a stop) instead of holding the exit; and `Program.cs` `OnTerminate` requests the stop, closes the player (`PlxPlayer.Shutdown`), then waits for `on_stopped`. The header states that a player open at a stop belongs to the host. The `.cs` change has not been compiled or run: the Windows session must build it and check for `on_stopped clean=1` with a player open. **Round 4 (fixed in code; the emulator re-run is pending):** the terminate reaches the library as a Suspend, which PARKS the attempt (the close is `Released` through the `Refusal::Stopping` answer, the exit disposition Clean); nothing is live and no route attempt is bound, so the exit's `playback_needs_teardown` read false and the server was never told the title stopped (no `stopped` timeline, no transcode stop). `PlayerAdapter::parked_uri_owes_stop` (an unfinalized parked URI attempt whose restore has not begun) now also sends `run::end_playback_for_exit` through `stop_playback`, which reports it stopped at its own position and stops its transcode through `scrobble_stop`; hostsim `an_exit_while_a_uri_attempt_is_parked_reports_it_stopped_at_its_own_position`.
- **"EOS: stopping an off-route engine nothing owns" logged five times (guarded; the cause is by reading).** The EOS orphan branch (`app/run.rs`) ran on every frame while a URI attempt was Releasing (it had no engine to stop); it now skips an attempt that is already releasing.
- **A stall ended in "The server cannot play or convert this file" (fixed in code; the emulator re-run is pending).** The stall (the machine's `Failed(TimedOut)`, then a close) reached the shared recovery, `PlayerAdapter::note_uri_failure`, whose prospect `can_retry_uri_after_release` counted the still-unknown PMS offers (and the Feed's, which the policy lists whether or not a Feed is composed) as "a backend remains". The retry closed the player, re-resolved with the original spent (`DIRECT PLAY`, playing item), found nothing a no-Feed composition with a provider that certifies no converted output can play, and ended in `PlayVerdict::NoQualifiedBackend` (`decision_refused`, the generic open sentence) about 30 s later, with no log line. The prospect now needs a Feed that is composed, or a URI provider that certifies a converted output (`uri_retry_can_convert`, the provider flag `carries_source_track_change`), so on Tizen no retry starts and the terminal failure is the stall itself: a `TimedOut` after `Playing` was observed reads as `PlaybackInterrupted` ("stopped"), not as `LoadTimeout`, and a refusal that does end a resolve logs `play: no backend can carry this play (uri original: <reason>; pms: <reason>)` for every viewer (the `uri: original declined` line stays an opted-in viewer's). webOS URI-only builds keep their retry (the default provider flag is true). Tests: `a_failed_uri_original_is_not_retried_when_no_composed_backend_could_carry_the_retry`, `a_uri_stall_after_playing_reads_as_stopped_and_a_start_that_never_completed_as_a_timeout`, `a_refused_play_on_a_composition_without_a_feed_logs_the_typed_reasons_whatever_the_engine`.
- **Review round 4's smaller items (fixed in code).** The track-pick refusal test now seats a playing URI attempt, a bound source selection and a held request (it re-planned and returned true with the gate removed, observed red); leaving the host's buffering state restarts the frozen-picture clock (`last_position` is reset by a change of the host's buffering state, test `leaving_host_buffering_starts_the_frozen_picture_clock_afresh`); the EOS orphan branch's `Phase::Releasing` guard has its own hostsim test (`an_ended_uri_attempt_with_no_player_page_is_stopped_once_while_its_release_is_pending`); the host closes its player at a stop and the library owns the resume (the module doc of `machine.rs`, `PlxPlayer.cs` and `Program.cs` said otherwise).
- **Player events are logged (done).** The library logs one line per player ABI event (`tizen-uri: ...`) through the existing event log; POSITION and BUFFERING are rate limited to one per 10 s (`EVENT_LOG_EVERY_MS`).

- **Review round 5 (and the emulator runs at `fc5d28594` and `b62e1276e`; every fix below is "fixed in code, emulator re-run pending", each with a test observed failing first).**
  - **A restore the request door refused reported the parked attempt stopped twice.** `route::restore_uri_original` sends A's `stopped` (`scrobble_stop`) BEFORE the door is asked, because the replacement's resolve waits behind that stop; nothing marked A finalized, so when the door refused (a route transition owned the player, or the resolve worker did not spawn) A stayed parked and owing the stop it had already sent, and the exit (`parked_uri_owes_stop`) or the next foreground's restore reported it a second time. The call now answers `UriRestoreStart { started, reported }` and `pump_uri_restore` finalizes the attempt when `reported`; `parked_final_report` is read only while the report is owed. The order the emulator verified (A's `stopped`, then `POST /playQueues` B) is unchanged. Test: hostsim `a_refused_restore_leaves_the_parked_attempt_reported_once_and_its_exit_does_not_report_it_again`.
  - **A CLOSE the host had accepted just before `plx_host_stop` held the stop for `STOP_DEADLINE` and reported it unclean.** `Hosted::post_player` refused every event after a stop, `PLAYER_CLOSED` included, so the attempt stayed `Closing` for the 5 s deadline. The post boundary now still hears a `PLAYER_CLOSED` of a LIVE epoch while the loop runs (it is the confirmation the stop waits for; every other kind is still `E_STATE`, and an epoch that is not live is still stale). The same event answers the in-flight CLOSE that `PlxPlayer.Shutdown`'s `CompleteAdding` refused (`E_UNAVAILABLE`): Shutdown's own close posts it. A CLOSE the library could not send (the gate shut) stays the machine's `Stopping` release. Test: `a_close_accepted_before_the_stop_is_confirmed_by_the_closed_that_arrives_after_it`.
  - **`PlxPlayer.Shutdown` could leave an open that nothing closes (C#, NOT compiled here).** `Program.cs` now calls Shutdown before waiting for `on_stopped`, so a `player_open` already past the library's gate could (a) start the video thread after a Shutdown that found none (`EnsureThreads` never read `shuttingDown`), or (b) be queued between Shutdown's close job and `CompleteAdding` and run after it. Shutdown now sets `shuttingDown` under `startGate`, `EnsureThreads` returns false once it is set (so `Open` and `Verb` answer `E_UNAVAILABLE`), and `DoOpen` declines an open when it is set (its epoch ends with one `PLAYER_CLOSED`). `check-tizen-so abi` passes; nothing compiled or run.
  - **A stall left the PMS session open while the failure read-out showed: no change, because the webOS Feed holds it the same way.** `release_failed_engine` (`player/route_start.rs`) says it plainly: under a terminal playback error the failed Engine's pipeline is released but "the session, URL and transcode session stay, nothing is reported as stopped", so Try again, Play automatically and a quality pick can start from the session, and Back ends it with the real stop. `scrobble_stop` is reached only from `begin_teardown` (a full stop) and from the restore and retry doors. The URI path holds it for the same reasons, through the same mechanism. **What was wrong there is the button:** the read-out offered *Change quality* on a URI-only composition, where the ladder is withheld (round 4's R5) and a pick is refused. `FailureContext` now carries `quality_carriable` (`PlaybackSession::quality_pick_carriable()`), and `failure_actions` offers `ChangeQuality` only when it is true. Test: `a_composition_that_can_carry_no_quality_rung_is_not_offered_change_quality_on_a_failure` (the live wiring is asserted in `a_quality_pick_no_composed_backend_can_carry_is_refused_and_leaves_the_route_alone`).
  - **OPEN MEASUREMENT, not a cause: two main-thread hangs of about 300 to 400 ms during the URI restore** (around the host's `Buffering` and `SeekDone epoch=2` lines; one earlier run saw 301 ms to 419 ms right after the host's `ecore window id` line, before `Prepared`). Read from the code, nothing on the library's frame thread in this PR's restore and open path joins or holds a lock across IO: `pump_uri_restore` and `restore_uri_original` hand the stop and the resolve to workers (`scrobble_stop`, `spawn_off_frame("resolve")`), `HostedPlayer::open` and `verb` enter the gate (an atomic count) and call the host, which only enqueues (`TryEnqueue`; the host's own work runs on its video thread), and `drain_player` takes one short mutex. What the frame thread DOES do and the code cannot price: the reverse P/Invoke into managed code (`Open` takes `startGate` and starts two threads on the first open; the first call into each delegate may JIT), the synchronous event-log append for every logged event, `scrobble_stop`'s `drain_scrobble` when a previous stop worker is still in flight (no stop is in flight at the restore's first call), and the library's own swap onto the Ecore window while the host binds the player overlay to the same window (that bind runs on the Ecore main thread, with the video thread waiting for up to 3 s). None of these is shown to be the stall. **To diagnose** (the `profile-tv` skill's method, adapted to the emulator): wrap the restore frame's legs (`pump_uri_restore`, the driver's `dispatch` and `poll`, the swap) in temporary `plx_base::diag::spans::span("label", || ...)` on a scratch branch so the `spans=` field of the late frame names the call. Until then the cause is not determined.

What the host-side tests cover: the driver over a scripted fake port, the same conformance suite (`plx_playback_contract::uri_conformance`) over the contract's
`FakeUriDriver` and over this driver, `hosted::testing` wiring (a round trip, STOP as CLOSE, a late completion refused as stale, no host call after `plx_host_stop`),
and the qualifier through the planner's resolver. **Not covered:** a picture on screen through this driver (the window's alpha and the straighten pass are part B2's above; this driver emits no `Presented`), Home with a player open after the round-4 and round-5 fixes (an emulator re-run is pending), the retail player's
default audio stream, a real PMS's answer to `X-Plex-Platform: Tizen`, and everything H6 asks of a retail set (decode, sound, armv7, the TV-menu round trip).

An emulator playback run (the Windows session's, against the mock) has no audio device, so it must serve VIDEO-ONLY media: `python3 tests/mock_pms.py --extra-media clip.mp4 --extra-media-video-only` (the
mock already serves byte ranges; a file with an audio stream is refused without the flag). By the source and its test (`identity::probe_device`, read per request by `plex::identity`; not observed on the wire, the mock's request log keeps no headers) the app sends
`X-Plex-Platform: Tizen` and `X-Plex-Platform-Version: <version> (emulator)`.

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

### Developer package signing

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

### The nightly package

```sh
make PLATFORM=tizen TIZEN_ARCH=armv7 RELEASE=1 tizen-tpk-nightly \
  TIZEN_NIGHTLY_LABEL="$(python3 ci/nightly.py plan --date "$(date -u +%Y%m%d)" --head HEAD --dry | sed -n 's/^label=//p')"
```

The nightly target verifies the library, stages the project in a temporary directory and signs it
with throwaway author and distributor certificates. Apps2Samsung replaces both during install.
The manifest version advances daily and the library reports the same nightly label as the release.
The shared source bundle includes the Tizen host and build recipes. The Samsung job attaches
`tizen-sdl-source.tar` with the two pinned SDL archives alongside the package.
The package carries no mock server or log address. The build clears the four endpoint variables read
by `telemetry/sender.rs`, so it has no Sentry or PostHog destination.

Before enabling `SAMSUNG_NIGHTLY_PUBLISH`, build the same target for `x86_64` on Linux or dispatch
**Tizen library CI** with **Produce the x86_64 nightly package** enabled. Stamp its UDP
destination as below, re-sign the stamped copy using the emulator's Samsung profile, and install it
on the TV 10.0 emulator. Confirm the sign-in screen, matching nightly version in the log, and
successful receipt of the UDP log. Set the repository variable to `true` after that smoke test.
A real **Nightly** dispatch with **Build even when main has not changed** can then publish on
an unused UTC date. The [Samsung nightly guide](samsung-nightly.md) covers the retail
Apps2Samsung path.

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
data directory is refused, so the log leaves the device over UDP. Use the UDP receiver on retail TVs as well.

### The log channel

With `TIZEN_LOG_HOST=IPV4:PORT` the host writes every line as one UDP datagram, `h<seq> <line>`, and hands the address to the library as
`log_udp` for its own lines. `seq` counts per sender, so a gap is a lost datagram; `tools/tizen-log-receiver.py` prints the report
(`-- N datagram(s) lost before ...`) and ends with a count: a run whose log is claimed as evidence ends with `lost 0`. The address is an
IPv4 literal (no name lookup in a logger). The host also writes `<data>/plx-host.log` and dlog tag `PLXHOST`.
For a downloaded package, use `--stamp IN.tpk --save FILE` and install the stamped copy after
re-signing it. On the emulator use `--address 10.0.2.2` to target the host's loopback.

## What success looks like

In the receiver, in order (the host's lines are fixed by `Program.cs`; the library's wording is its PR's and is not repeated here):

1. `host started: OnCreate on the main thread; ...`, the `network log: UDP -> ...` line, the runtime and the platform lines, and
   `native library .../lib/libplx.so exists, N bytes`;
2. `dev boot: the members of .../dev-boot.json ...` (no `dev boot: none`);
3. `plx_host_start OK; the library's loop is running`, and no `plx_host_start FAILED`, `DllNotFoundException` or `EntryPointNotFoundException`;
4. the library's own lines: the platform identity (`platform: Tizen <version> (emulator only; direct play through the host's native player, not run on a retail set)`; a build without `uri` says `(emulator only; no playback)`), the mock server's identity, the GL line without a failure,
   Home reached with the mock's libraries, Resume dimmed on a detail page with `preview: blocked ... reason=Cached(RefusedDirect)` in a build without `uri` (the Home hero's Play is a known gap, below); in a `uri` build the log says `uri: display path=ecore-overlay ready` and `compose: backends=uri resolver=yes`, and each remote key logged as an action;
5. no sequence gap in the receiver, and no crash lines;
6. a Home-button round trip logs `OnPause` then `OnResume` and the screen redraws; closing the app logs `OnTerminate` and ends with
   `on_stopped code=0 clean=1`.

State plainly what was not seen. The log is the evidence. A host-side window capture of the emulator does show the GL UI (a Windows
`PrintWindow` capture showed Home, the detail page and the focus ring), so a capture is a record of the screen to attach beside the log.

## Known gaps seen on the emulator (2026-11-21)

- The Home hero's Play looks enabled; pressing it opens the player route ("Playback failed / The server cannot play or convert this file", footer `decision_refused`) and logs no decision line. That route clears to `(0, 0, 0, 0)` over a translucent window: the emulator shows black behind the read-out (above); a retail set is unmeasured (H6).
- (Fixed in part B2.) `shader compile FAILED: 0:37(1): error: syntax error, unexpected NEW_IDENTIFIER` on YaGL, followed by `straighten: no program`, was seen on 2026-11-21. Part B2 gave the copy variant a source of its own ("The video plane", above) and the 2026-11-22 run at `3d56d35b2` logs `straighten: no framebuffer fetch - the pass copies the frame` instead.
- webOS-only lines are still printed: `devcaps: ... assuming the 49SM9000PLA profile`, the player footer `webOS unknown — os_info.json unreadable · unknown set`, and three `ff: ... MISSING ... reinstall the package` lines, whose advice does not apply to this package.
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
