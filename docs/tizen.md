# The Tizen build for Samsung TVs and the TV emulator

`PLATFORM=tizen` builds PlxNative for a Samsung Tizen television as a **hosted library**: a shared object, `libplx.so`, that a small
.NET application (`tizen/host/PlxHost/`) loads and drives through the C functions its header exports (`plx_host_*`). The design is `docs/platform-ports.md`
("Target shape", W9); this page is how to build, check, install and read what comes out. Except where a section names the retail tester's report (discussion #1460), nothing here is verified on a retail set.

## Scope of the MVP

- **Two builds.** `TIZEN_ARCH=x86_64` targets the Tizen TV 10.0 emulator. `TIZEN_ARCH=armv7` builds the
  retail SUPER ALPHA package for 2021+ Samsung TVs (Tizen 6.0+). Both use the `tizen80` managed target.
  A retail TV still needs to run it (H1, H2 and H6 in `docs/platform-ports.md`).
- **Playback is the `uri` feature's. The driver has run on the Windows emulator against the mock server, and has NOT been run on a retail set (H6 stays OPEN).** The first emulator run (at `126d1f04c`) found faults A-E ("Findings of the first emulator run", below); runs at `fc5d28594` and `b62e1276e` passed those fixes and round 4's and found what review round 5 fixes. Every fix since a run is "fixed in code, emulator re-run pending" until a run passes it. A build with the platform crate's `uri` feature (the makefile's default; `TIZEN_URI=0` builds without it) composes the Tizen URI driver over the host's native player (below): for a title the planner can offer it, the library asks the host to play a progressive URI source. **Which titles those are is a narrow set, and nothing here says one plays**: the planner refuses most real titles before the Tizen qualifier is asked ("The refusal set", below), and a source with several audio streams is certified for its selected stream only since the audio track verb ("The audio track is selected", below), which chooses the native track by its language because the player's track order is unmeasured on a retail set. A build without `uri`, or a host with no player group in its table, composes no backend (`compose: backends=none`): on a detail page Resume is then dimmed and cannot be focused, and the metadata line reads "Playback unavailable" (the log: `preview: blocked rk=... reason=Cached(RefusedDirect)`); on Home the hero's Play looks enabled, and pressing it opens the player route, which showed "Playback failed / The server cannot play or convert this file" with the footer `decision_refused` at the emulator run, and logs no decision line: a known gap (below). The refusal is the app's own, so a build since then names it: "Playback failed — this TV can’t play this file in PlxNative", footer `no_qualified_backend` (not re-run on the emulator). What a `uri` build has NOT shown: a picture through this driver (the window's alpha and the straighten pass are part B2's, below; the library emits no `Presented` for it), and Home with a player open. H6 (below) is open.
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
| `ci/build-libass.py` | the subtitle renderer `libass-plx` (the webOS recipe, shared: `tizen-libass` runs it with `PLX_ASS_STAGE`; "The subtitle renderer", below) |
| `tizen/host/PlxHost/` | the .NET host: `PlxHost.csproj`, `tizen-manifest.xml`, `Program.cs` |
| `tizen/package.py` | stages the package's `lib/` and `res/`, refuses without signing material, packs and checks the `.tpk` |
| `ci/check-tizen-so.py` | the contract check: header against C# as text (`abi`), the linker's version script (`map`), the linked library (`verify`) |
| `tools/tizen-log-receiver.py` | the UDP log receiver, with a lost-datagram report |
| `tizen/esprobe/`, `tools/make-esprobe-clips.py` | the packet-player probe's clips and the script that makes them ("The packet player", below) |

Generated and never committed (`.gitignore`): `rust-modules/target-tizen/`, `tizen/out/`, `tizen/sdl-build/`, and under
`tizen/host/PlxHost/` the staged `lib/`, `bin/`, `obj/`, `res/appfont*.ttf`, `res/fonts/`, `res/libass-plx*.so.0`, `res/OFL.txt`, `res/dev-boot.json`,
`res/log-host.txt`, `res/THIRD-PARTY-NOTICES.md`, `res/licenses/`, `res/esprobe/` and the icon.

## The targets

Every target is `make PLATFORM=tizen <target>`. A bare `make PLATFORM=tizen` is `tizen-check`; choose `tizen-tpk`
or `tizen-tpk-nightly` to create a package. The webOS package goals (`all`, `ipk`, `deploy`, `install`, the storage helper and the C
units, `WEBOS_TV_GOALS` in the `Makefile`) are refused under `PLATFORM=tizen` with a message, so `make PLATFORM=tizen ipk` cannot package
the television build by accident; the queries (`print-*`) answer under every `PLATFORM`.

| Target | Needs | What it does |
| --- | --- | --- |
| `tizen-abi` | any host, python3 | reads `plx_host.h` and `Program.cs` as text: the header's `plx_host_*` functions, each one's return type and parameter count, every struct (`PlxHostApi`, `PlxHostEvent`, `PlxHostCue`) field for field (name, order, width), every function-pointer typedef against the host's `delegate` of the same name (return and parameter widths), and every `PLX_HOST_*` constant the host spells equal the header's (the host must spell the version and every event kind and verb) |
| `tizen-check` | any host with the target's std | `tizen-abi`, then `cargo check --lib` of the platform crate for the selected target |
| `tizen-sdl` | host tools and the selected compiler | the static SDL2 + SDL2_ttf under `tizen/sdl-build/<arch>` (a stamp of the pins, the script and the flags skips a rebuild that would change nothing) |
| `tizen-libass` | the above, cmake and pkg-config | the subtitle renderer `tizen/out/<arch>/libass-plx-host.so.0` (x86_64) or `libass-plx.so.0` (armv7), stripped; `ci/build-libass.py` keeps the static stack under `vendor/libass-prefix-host` or `vendor/libass-prefix` (shared with the simulators and the webOS package) and relinks only the facade when it is current |
| `tizen-so` | Linux x86_64 with `libgles-dev`, or the ARM NDK | `tizen-libass`, then `cargo rustc --crate-type staticlib` of the platform crate (features `tizen,uri,devtriggers`, profile `tvdev`; under `RELEASE=1` profile `release` and features `tizen,uri` only, so a release library carries no dev-trigger seam), then the link into `tizen/out/<arch>/libplx.so` |
| `tizen-verify` | GNU binutils (from the NDK on ARM) | `ci/check-tizen-so.py verify` on the linked library and, with `--ass`, the renderer (below) |
| `tizen-stage` | python3 | stages `lib/libplx.so`, `res/` (the renderer, the fonts, the notices) and the icon into the host project; `TIZEN_MOCK_HOST` and `TIZEN_LOG_HOST` become `res/dev-boot.json` and `res/log-host.txt` |
| `tizen-tpk` | the above, .NET 8 SDK, a Samsung certificate profile | the signed `tizen/out/PlxNative-x86_64.tpk`; refuses at once, naming the missing variables, without the profile |
| `tizen-tpk-nightly` | the above, .NET 8 SDK, OpenSSL, `RELEASE=1` | builds and verifies `plxnative-tizen-v<label>-<arch>.tpk` with throwaway signing |
| `print-tizen-config` | any host | the configuration, one `NAME=value` per line (the CI job prints it) |

The emulator link runs on x86_64 Linux. The ARM build uses the pinned webOS NDK on macOS or arm64
Linux and the `armv7-unknown-linux-gnueabi` Rust target. Run `make setup-env` to install both. The NDK has no
x86_64 Linux build, so an x86_64 Linux machine (WSL included) builds only the emulator package; the armv7 one is CI's
(`tizen-package.yml`) or a Mac's.

### What the link contains

The library is the `plx_platform_tizen` staticlib (the whole application behind one platform crate), `svg.o` (the nanosvg rasterizer the
UI's icons use) and a **static SDL2 + SDL2_ttf with its own FreeType**: a Tizen television ships no SDL, and the text path is SDL_ttf over SDL
surfaces. SDL never opens a window and its video subsystem is never initialised; the window, the EGL context and the keys are Tizen's own
(Ecore_Wl2). The link line (`mk/platform-tizen.mk`) is `cc -shared` with `-Wl,-soname,libplx.so`, `-z defs` (an undefined symbol fails this
link, not `dlopen` on the emulator: the library is loaded `BIND_NOW`), `-z relro -z now`, `--as-needed`, `--gc-sections`, and a **version
script generated from the header** that makes the header's `plx_host_*` names global and everything else local, so no SDL, FreeType or Rust
symbol is exported. The link is written to `libplx.so.part` and renamed, so a failed link never leaves a half-written library for
`tizen-verify` to read.

### The subtitle renderer

The package carries a second native library, `libass-plx`, the same one the webOS package ships: the `plx_ass_*` facade (`src/ass.c`,
`src/ass_fonts.c`, `src/ass_text.c`, `include/ass.h`) over a statically linked libass, FreeType, FriBidi, HarfBuzz and libunibreak. It is
what draws every text subtitle (`player::ass`, `ass_source`) and what shapes Arabic and Hebrew UI text (`plx_gfx::rtl`). Without it a
selected subtitle shows "Styled subtitle support is unavailable in this installation" and right-to-left text falls back to boxes.
It is NOT linked into `libplx.so`: the application opens it at run time by absolute path from its application directory (`dynlib!`,
candidates `libass-plx-host.so.0`, `libass-plx.so.0`), which on Tizen is `res/` (`DirectoryInfo.Resource`), so it is staged as
`res/<name>` beside the fonts it reads.

- **Built by the webOS recipe, not a parallel one.** `tizen-libass` runs `ci/build-libass.py` with `PLX_ASS_STAGE=tizen/out/<arch>`, which
  only changes where the stripped library is published (never `pkg/`, which no Tizen goal writes). On armv7 it is the recipe's ARM
  cross build, from the same inputs and under the same stamp key as the webOS one: the Tizen armv7 link already uses the same pinned webOS NDK, sysroot (glibc 2.12 symbols) and
  `ci/arm-cc.py`, and the file is `libass-plx.so.0`. On x86_64 it is the recipe's Linux host build, `libass-plx-host.so.0`, compiled by the build host's
  `cc` exactly as `libplx.so` is (and under the same `PLX_MAX_GLIBC` ceiling). The static stack is cached by the recipe's own content key
  under `vendor/libass-prefix(-host)`, shared with the webOS package and the simulators.
- **The fonts.** `tizen/package.py` stages every face of the UI text chain (`appfont.ttf`, `appfont-bold.ttf`, `appfont-cjk.ttf`,
  `appfont-emoji.ttf`, the Arabic and Hebrew faces with their bold cuts; `ci/test_shipped_faces.py` holds the list to `text.rs`) and the
  per-script subtitle faces with their manifest in `res/fonts/subs/`. The renderer's default face is `appfont-cjk.ttf`; `verify_tpk`
  refuses a package that lacks a face the packaged manifest names.
- **Checked.** `tizen-verify` holds the renderer to its own rules (below); `verify_tpk` requires exactly one renderer, under one of the two
  names, byte-equal to the verified file, and no other ELF object outside `lib/`.
- **Size.** The x86_64 emulator package grew from 15,043,205 to 25,621,891 bytes (a `tvdev` build with the dev-boot files, measured
  2026-11-26). The renderer is 3.2 MB stripped and `appfont-cjk.ttf` 13.4 MB, both before compression.

**Notices.** The package statically links SDL2 and SDL2_ttf (zlib) and FreeType (the FreeType License, which requires the credit to be shown
in the documentation shipped with the product), and carries the renderer (libass ISC, FreeType, FriBidi LGPL-2.1-or-later, HarfBuzz Old MIT,
libunibreak zlib: `THIRD-PARTY-NOTICES.md` 1.2), so the `.tpk` carries `res/THIRD-PARTY-NOTICES.md` and `res/licenses/` (the project's own
`LICENSE`, `LICENSING.md`, `TRADEMARKS.md` and every `licenses/*.txt`, the set the `.ipk` ships); `tizen/package.py` stages them and
`verify_tpk` fails a package without the notices, `Zlib.txt`, `FreeType.txt` or the renderer's texts (`libass-ISC.txt`, `HarfBuzz-Old-MIT.txt`,
`libunibreak-Zlib.txt`, `LGPL-2.1.txt`). `THIRD-PARTY-NOTICES.md` section 2.8 is this package's additional entry and states, heading by heading, which other sections of the file cover the `.tpk`; section 3 there describes the webOS package, so read 2.8 first.

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
symbols are exactly the header's `plx_host_*` functions (`plx_host_start`, `plx_host_post`, `plx_host_stop` and `plx_host_post_cue`), equal to
the C# `DllImport` names;
`TTF_Init`, `TTF_OpenFont`, `TTF_RenderUTF8_Blended`, `FT_Init_FreeType` and `SDL_FreeSurface` are code symbols of the library and none of
`SDL_*`/`TTF_*`/`FT_*` is exported. On x86_64 the highest `GLIBC_x.y` version needed is at most `PLX_MAX_GLIBC` (default 2.40, the emulator image's
glibc; the CI runner, ubuntu-24.04, links against 2.39). A newer build host is a failure that names the ceiling: link on an older host, raise `PLX_MAX_GLIBC`
if you know the target's glibc, or `PLX_ALLOW_NEW_GLIBC=1` to report and continue. The parsers and every rule are unit-tested on canned
`readelf`/`nm` text (`ci/test_tizen_so_check.py`). The ARM check also compares every required import and
library name against `ci/testdata/tizen60-armv7-abi.json` (the renderer's too). Its exports come from the Tizen 6.0 ARM
rootstrap and the matching Tizen SDK compiler's `libgcc_s.so.1`. Both archives and their SHA-256
digests are in `ci/dependencies.json` under `tizen60-abi-reference`. The inventory covers link-time
imports. Runtime libraries loaded by name still need device verification.

`--ass` verifies the renderer the same way, under its own rules (`verify_ass_object`): `DT_NEEDED` only `libc.so.6` (required), `libm.so.6`,
`libpthread.so.0`, `librt.so.1`, `libgcc_s.so.1` and the loader (no `libGLESv2`, and no firmware FreeType, FriBidi, HarfBuzz or libass: the
stack is static); the SONAME the build gives the architecture (`libass-plx-host.so.0` on x86_64, `libass-plx.so.0` on armv7); no text relocation
or static TLS; every undefined symbol a versioned glibc/libgcc one; the exports exactly the `plx_ass_*` functions `include/ass.h` declares
(so no `FT_`/`hb_`/`fribidi_`/`ass_` name leaks); the same glibc ceiling and, on armv7, the same Tizen 6.0 inventory.

### The contract with the host

`rust-modules/platform-tizen/include/plx_host.h` is the single source. `PlxHost`'s `Native` class, its structs (`PlxHostApi`, `PlxHostEvent`, `PlxHostCue`) and its
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
Version 2 has four exports: the fourth, `plx_host_post_cue`, hands the library a selected subtitle's cues ("Selected text subtitles", below) and
changes neither the table nor the event layout, so the version stays 2 (the host and the library ship in one package). A version-1 host, a table that stops at `run_on_host`, or a version-2 table with a NULL player pointer is
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
  **only through the pixels the frame clears to (0, 0, 0, 0)**; the UI around it is intact. With it off and nothing else, no video shows.
  **Superseded on 2026-11-26 (next section):** a translucent window shows the TV's home screen through those pixels on a retail set, so the window
  is now opaque and states `has_video` (`plx_platform_tizen::ecore::WindowMode`), and is translucent only on an image without that call. Either
  way the mode is a flag of the window set at creation, so nothing is routed per frame and the Tizen shell's `opaque_route` has no
  work. The pages with a ground clear to an opaque colour, so only the player route and the detail trailer preview (`frame_clear_through`) open the
  hole, the same shared chain webOS uses. **The player route's clear is keyed on the route, not on the plane**: the loop clears to `(0, 0, 0, 0)`
  whenever the player route is up (`fr.player`), bound or not, and `PlayerScreen` has no ground, so its spinner and its "Playback failed" read-out sit
  on a transparent clear even with no plane bound. The straighten pass and the capture skip are the ones keyed on `Player::video_plane_bound`.
- The **video plane mixes STRAIGHT alpha** (measured with the window TRANSLUCENT; not re-taken since the window became opaque with
  `has_video`, whose compositor path is a different one, so it is unverified there; pixel-exact: a straight (255, 255, 255, 128) strip is a 50 % veil over the video) while window-over-window
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
set (H6 stays OPEN: the library's thread, the overlay beneath an opaque `has_video` window, or an alpha window where the symbol is missing, privileges, firmware differences). The C# is built by no CI job
(`.github/workflows/tizen.yml` runs `tizen-abi` and `tizen-so`, which links the Rust library and never builds `PlxHost`); it is compiled by a
`make PLATFORM=tizen tizen-tpk` on a machine with the .NET SDK (the run above did that for this tree).

### The Rust driver over it (the `uri` feature, W10 part B)

`rust-modules/platform-tizen/src/player/` holds `TizenUriDriver`, a Sans-IO `DriverSet` + `UriDriver` over a `HostPlayer` port that `hosted` implements: it
starts no worker and reads no global, its requests go out as `Hosted::player_open` / `player_verb` and every answer returns as one addressed event
through `drain_player`. Every request and event carries the epoch, and an event of any other epoch (a completion after a stop) is dropped and counted.
The driver states `progressive_uri`, `pause`, `seek`, `audio_track`, `subtitle_track` and `subtitle_off`, and no HLS, packet feed or audio-off. `subtitle_off` holds by construction: the native player draws no subtitle of its own ("Subtitles off" and "Selected text subtitles", below), the host never calls `player_set_subtitle_path`, and a `SelectTrack(Subtitle, Off)` is settled without a host call (the app stops drawing and drops the cues that still come). `subtitle_track` is the app's own drawing of an embedded TEXT track from the player's cues ("Selected text subtitles", below). `audio_track` is settled
by construction for a Part with one audio stream (no host call, as before the verb existed) and is a host call otherwise: the planner issues a `SelectTrack` for any
title whose server-selected audio stream is stated, and for a stream with siblings the driver asks the host for the stream at the native
index it chooses by the stream's language (the host reports each native track's before PREPARED), never by position alone (`PLX_HOST_PLAYER_VERB_AUDIO_TRACK`,
`player_select_track`), and settles it on the host's `PLAYER_TRACK` answer ("The audio track is selected", below). A stream the map does not name, or whose native
track cannot be told by its language, is refused `Unsupported`, which fails a start (the start plan's track selection is mandatory); a refused or unanswered request is the command's refusal. Stop sends the CLOSE verb and waits for the host's `CLOSED`; past a deadline it reports a
quarantined release and the process-exit disposition stays Preserve. The platform's own `TizenQualifier` certifies a source only where Standard would
direct-play it (so DTS and TrueHD, and every container Standard hands to the server, are refused with the planner's typed reason) and never a converted PMS
output (`qualify_pms` is Unknown); the codec table behind Standard is still `Caps::assumed()`, the 49SM9000PLA row, not a Tizen measurement. The composition
(`player::MEDIA`) replaces `host::NO_MEDIA` in a `uri` build; it installs the driver only when the host's table offers a player and the run is not a controlled
one, and returns the resolver only when the adapter accepted the driver. It names no LG binding and publishes no default backend.

**The refusal set.** Before the Tizen qualifier is asked, the planner's own preflight (`ResolvePort::refusal`, final for the play) refuses an original for: a selected rate other than 1x; a quality ceiling or Direct play rule; a Plex relay; Auto on a link not known to be local; Boost Dialog or Normalize Loudness (`audio_contract.any()`); a burned subtitle; and a client-drawn (sidecar) subtitle. `SubtitleOff` (the item has subtitle streams and none is selected, `subtitle_choice = Off` in `plan.rs`) is refused only where the driver's evidence cannot carry Off, so it no longer stops a Tizen play: that refusal ended every title with subtitles on the first retail report (a 2023 UA65CU7700, discussion #1460), and the one title there without subtitle streams played. A selected embedded subtitle is carried when every embedded subtitle stream of the Part is SubRip or ASS (`MEASURED_TEXT_CODECS` in `player/qualifier.rs`): the app draws it from the host player's cues ("Selected text subtitles", below). A selected picture subtitle (PGS, VobSub), a `mov_text` or other unmeasured text codec, or a Part where such a stream sits beside the selected one, still fails (`SubtitleNotCarried`). With no Feed and `qualify_pms` Unknown, each refusal ends as the planner's typed `NoQualifiedBackend`, not as a play. So a title plays through this driver if it has no selected subtitle (or a selected embedded SubRip or ASS one, in a Part whose embedded subtitle streams are all SubRip or ASS), an audio stream whose position among the Part's audio streams is known and, when the Part has several, whose PMS language exactly one native track (or the ordinal's own) reports ("The audio track is selected", below; a multi-audio title without one fails its start rather than play the player's default), a container and codecs Standard direct-plays, local Auto or a fixed quality, and no audio enhancement. The resolver tests pin Off to a certified start and the others to their reasons.

**A pick the driver cannot carry does not replace the playing title.** Picking another audio or subtitle track mid-play on a URI attempt that cannot switch natively used to re-plan the play, which retires the playing item first and, here, can only end in `NoQualifiedBackend`. The shared door (`route::request_viewer_reconfigure`) now asks `PlaybackSession::source_track_change_carriable()` first: true with a Feed (webOS) or when the URI resolver's provider says it can certify such an output (`OriginalQualifier::carries_source_track_change`, default true; `TizenQualifier` says false), and a pick nothing can carry is refused with `ViewerReconfigureRefusal::NoBackendCarriesTrack` before anything is retired or spent. **The Quality ladder is the same kind of request** (a rung asks the server to convert, and `request_transcode_refresh` only queues a `Retranscode` that the Feed engine's pump consumes): `PlaybackSession::quality_pick_carriable()` (the same provider flag) makes the More menu not offer the Quality rows and `set_quality` refuse a rung before it persists it or moves the route contract; unrefused, the pick left the title playing unchanged and the next re-resolve of it (a Home restore, a stall retry) planned under the new ceiling and ended in `NoQualifiedBackend`.

**Findings of the first emulator run of the driver (the Windows session, at `126d1f04c`; emulator and mock data only).**

- **Home with a player open never resumed (fixed in code; the emulator re-run is pending).** The host's `PauseForHome` paused the player, but `ResumeAfterHome` found nothing to resume, and the library's own restore opened nothing. The library parks a URI attempt at the background (`park_playback`: `Stop(Suspend)`, the CLOSE the host confirms), so by `OnResume` the host's epoch is `Closing`. The foreground restore then reached `begin_resume`, whose prelude answered `NoRoute` because a URI original plan installs no `PlaybackSession::url` (`plan.uri_source_ready`: the source travels in the offer the driver consumed). Reproduced on the host before the fix (a hostsim test over the webOS `play-uri` driver's rig, `a_home_round_trip_reopens_a_uri_original_attempt_at_the_saved_position`: the restore never opened a second native session). **One owner, the library**: `begin_resume`'s prelude treats a parked URI original (`route::uri_restore_owed`: no URL, a source selection, the request) as a re-resolve, not a rebuild. It answers `Pending`, the frame performs it with the metadata store the lifecycle machine lacks (`player::pump_uri_restore`, `route::restore_uri_original`: the failed start transaction is ended, the old logical resource retired, and the SAME item is resolved again at the saved position as a new attempt of the same request, origin `Resume`), and the offer that lands re-opens the driver (`land_resume` hands the lifecycle `Prepared`, whose `Load` then starts it). A re-plan that falls to a Feed route hands over to the ordinary resume. A refused re-resolve answers `RebuildRejected` and leaves the session parked for the next foreground with the read-out up. The host no longer starts a player at `OnResume` (it would race the restore): it only pauses at `OnPause` and logs why an `OnResume` resumed nothing; `plx_host.h` states the ownership. **webOS:** the same code path serves the webOS URI original (`play-uri`), whose plan installs no URL either, so the fix covers it and the test above runs on that driver; webOS Feed sessions always have a URL, never read as owed, and their restore is unchanged. A paused save is re-paused by the app's existing `repause_at` after the Load, not by `InitialPlayback`. **The parked attempt's stop (round 4, fixed in code; the emulator re-run is pending):** the park reports nothing (the webOS Feed's suspend reports nothing either: its engine goes on under the same session), but the restore retires the parked session, and that session's `stopped` timeline used to go out only as a side effect of the REPLACEMENT's first report (the retired-session mechanism of #1236 posts it at the new report's position): the mock saw `timeline A playing 9377`, `playQueue B`, `transcode stop A`, `timeline A stopped time=15588`, `timeline B playing 15588`. `route::restore_uri_original` now takes the parked attempt's final report (`parked_final_report`: its own last position and duration, `begin_teardown`'s rule) and sends it through the ordered stop worker (`scrobble_stop`) before the second resolve starts, which also stops its transcode behind it; hostsim `a_restore_reports_the_parked_attempt_stopped_at_its_own_position_before_the_next_play` fails without it. Not run: the Tizen emulator (the re-run is pending), a retail set.
- **A frozen picture was reported as playing (fixed).** A player that posts POSITION with a position that does not advance while the driver believes it is Playing is now a stall: after `frozen_grace_ms` (a pause suspends it, a seek or a Started restarts it) the driver reports `Rebuffering(true)` and the ordinary stall deadline applies. A host test feeds a frozen position and a paused one.
- **Terminating the app with a player open was not a clean stop (fixed in the library and the host; the C# is uncompiled here).** `OnTerminate` waited for `on_stopped` while the player was still open; a close needs the host. The library now answers a CLOSE it cannot send after `plx_host_stop` as `Refusal::Stopping`, which releases the attempt (the host owns the close at a stop) instead of holding the exit; and `Program.cs` `OnTerminate` requests the stop, closes the player (`PlxPlayer.Shutdown`), then waits for `on_stopped`. The header states that a player open at a stop belongs to the host. The `.cs` change has not been compiled or run: the Windows session must build it and check for `on_stopped clean=1` with a player open. **Round 4 (fixed in code; the emulator re-run is pending):** the terminate reaches the library as a Suspend, which PARKS the attempt (the close is `Released` through the `Refusal::Stopping` answer, the exit disposition Clean); nothing is live and no route attempt is bound, so the exit's `playback_needs_teardown` read false and the server was never told the title stopped (no `stopped` timeline, no transcode stop). `PlayerAdapter::parked_uri_owes_stop` (an unfinalized parked URI attempt whose restore has not begun) now also sends `run::end_playback_for_exit` through `stop_playback`, which reports it stopped at its own position and stops its transcode through `scrobble_stop`; hostsim `an_exit_while_a_uri_attempt_is_parked_reports_it_stopped_at_its_own_position`.
- **"EOS: stopping an off-route engine nothing owns" logged five times (guarded; the cause is by reading).** The EOS orphan branch (`app/run.rs`) ran on every frame while a URI attempt was Releasing (it had no engine to stop); it now skips an attempt that is already releasing.
- **A stall ended in "The server cannot play or convert this file" (fixed in code; the emulator re-run is pending).** The stall (the machine's `Failed(TimedOut)`, then a close) reached the shared recovery, `PlayerAdapter::note_uri_failure`, whose prospect `can_retry_uri_after_release` counted the still-unknown PMS offers (and the Feed's, which the policy lists whether or not a Feed is composed) as "a backend remains". The retry closed the player, re-resolved with the original spent (`DIRECT PLAY`, playing item), found nothing a no-Feed composition with a provider that certifies no converted output can play, and ended in `PlayVerdict::NoQualifiedBackend` (at that run `decision_refused`, the generic open sentence; since then its own kind, `no_qualified_backend`) about 30 s later, with no log line. The prospect now needs a Feed that is composed, or a URI provider that certifies a converted output (`uri_retry_can_convert`, the provider flag `carries_source_track_change`), so on Tizen no retry starts and the terminal failure is the stall itself: a `TimedOut` after `Playing` was observed reads as `PlaybackInterrupted` ("stopped"), not as `LoadTimeout`, and a refusal that does end a resolve logs `play: no backend can carry this play (uri original: <reason>; pms: <reason>)` for every viewer (the `uri: original declined` line stays an opted-in viewer's). webOS URI-only builds keep their retry (the default provider flag is true). Tests: `a_failed_uri_original_is_not_retried_when_no_composed_backend_could_carry_the_retry`, `a_uri_stall_after_playing_reads_as_stopped_and_a_start_that_never_completed_as_a_timeout`, `a_refused_play_on_a_composition_without_a_feed_logs_the_typed_reasons_whatever_the_engine`.
- **Review round 4's smaller items (fixed in code).** The track-pick refusal test now seats a playing URI attempt, a bound source selection and a held request (it re-planned and returned true with the gate removed, observed red); leaving the host's buffering state restarts the frozen-picture clock (`last_position` is reset by a change of the host's buffering state, test `leaving_host_buffering_starts_the_frozen_picture_clock_afresh`); the EOS orphan branch's `Phase::Releasing` guard has its own hostsim test (`an_ended_uri_attempt_with_no_player_page_is_stopped_once_while_its_release_is_pending`); the host closes its player at a stop and the library owns the resume (the module doc of `machine.rs`, `PlxPlayer.cs` and `Program.cs` said otherwise).
- **Player events are logged (done).** The library logs one line per player ABI event (`tizen-uri: ...`) through the existing event log; POSITION and BUFFERING are rate limited to one per 10 s (`EVENT_LOG_EVERY_MS`).

- **Review round 5 (and the emulator runs at `fc5d28594` and `b62e1276e`; every fix below is "fixed in code, emulator re-run pending", each with a test observed failing first).**
  - **A restore the request door refused reported the parked attempt stopped twice.** `route::restore_uri_original` sends A's `stopped` (`scrobble_stop`) BEFORE the door is asked, because the replacement's resolve waits behind that stop; nothing marked A finalized, so when the door refused (a route transition owned the player, or the resolve worker did not spawn) A stayed parked and owing the stop it had already sent, and the exit (`parked_uri_owes_stop`) or the next foreground's restore reported it a second time. The call now answers `UriRestoreStart { started, reported }` and `pump_uri_restore` finalizes the attempt when `reported`; `parked_final_report` is read only while the report is owed. The order the emulator verified (A's `stopped`, then `POST /playQueues` B) is unchanged. Test: hostsim `a_refused_restore_leaves_the_parked_attempt_reported_once_and_its_exit_does_not_report_it_again`.
  - **A CLOSE the host had accepted just before `plx_host_stop` held the stop for `STOP_DEADLINE` and reported it unclean.** `Hosted::post_player` refused every event after a stop, `PLAYER_CLOSED` included, so the attempt stayed `Closing` for the 5 s deadline. The post boundary now still hears a `PLAYER_CLOSED` of a LIVE epoch while the loop runs (it is the confirmation the stop waits for; every other kind is still `E_STATE`, and an epoch that is not live is still stale). The same event answers the in-flight CLOSE that `PlxPlayer.Shutdown`'s `CompleteAdding` refused (`E_UNAVAILABLE`): Shutdown's own close posts it. A CLOSE the library could not send (the gate shut) stays the machine's `Stopping` release. Test: `a_close_accepted_before_the_stop_is_confirmed_by_the_closed_that_arrives_after_it`.
  - **`PlxPlayer.Shutdown` could leave an open that nothing closes (C#, NOT compiled here).** `Program.cs` now calls Shutdown before waiting for `on_stopped`, so a `player_open` already past the library's gate could (a) start the video thread after a Shutdown that found none (`EnsureThreads` never read `shuttingDown`), or (b) be queued between Shutdown's close job and `CompleteAdding` and run after it. Shutdown now sets `shuttingDown` under `startGate`, `EnsureThreads` returns false once it is set (so `Open` and `Verb` answer `E_UNAVAILABLE`), and `DoOpen` declines an open when it is set (its epoch ends with one `PLAYER_CLOSED`). `check-tizen-so abi` passes; nothing compiled or run.
  - **A stall left the PMS session open while the failure read-out showed: no change, because the webOS Feed holds it the same way.** `release_failed_engine` (`player/route_start.rs`) says it plainly: under a terminal playback error the failed Engine's pipeline is released but "the session, URL and transcode session stay, nothing is reported as stopped", so Try again, Play automatically and a quality pick can start from the session, and Back ends it with the real stop. `scrobble_stop` is reached only from `begin_teardown` (a full stop) and from the restore and retry doors. The URI path holds it for the same reasons, through the same mechanism. **What was wrong there is the button:** the read-out offered *Change quality* on a URI-only composition, where the ladder is withheld (round 4's R5) and a pick is refused. The failure now carries `quality_carriable` (`FrozenOffers`, from `PlaybackSession::quality_pick_carriable()`), and `failure_actions` offers `ChangeQuality` only when it is true. Test: `a_composition_that_can_carry_no_quality_rung_is_not_offered_change_quality_on_a_failure` (the live wiring is asserted in `a_quality_pick_no_composed_backend_can_carry_is_refused_and_leaves_the_route_alone`).
  - **OPEN MEASUREMENT, not a cause: two main-thread hangs of about 300 to 400 ms during the URI restore** (around the host's `Buffering` and `SeekDone epoch=2` lines; one earlier run saw 301 ms to 419 ms right after the host's `ecore window id` line, before `Prepared`). Read from the code, nothing on the library's frame thread in this PR's restore and open path joins or holds a lock across IO: `pump_uri_restore` and `restore_uri_original` hand the stop and the resolve to workers (`scrobble_stop`, `spawn_off_frame("resolve")`), `HostedPlayer::open` and `verb` enter the gate (an atomic count) and call the host, which only enqueues (`TryEnqueue`; the host's own work runs on its video thread), and `drain_player` takes one short mutex. What the frame thread DOES do and the code cannot price: the reverse P/Invoke into managed code (`Open` takes `startGate` and starts two threads on the first open; the first call into each delegate may JIT), the synchronous event-log append for every logged event, `scrobble_stop`'s `drain_scrobble` when a previous stop worker is still in flight (no stop is in flight at the restore's first call), and the library's own swap onto the Ecore window while the host binds the player overlay to the same window (that bind runs on the Ecore main thread, with the video thread waiting for up to 3 s). None of these is shown to be the stall. **To diagnose** (the `profile-tv` skill's method, adapted to the emulator): wrap the restore frame's legs (`pump_uri_restore`, the driver's `dispatch` and `poll`, the swap) in temporary `plx_base::diag::spans::span("label", || ...)` on a scratch branch so the `spans=` field of the late frame names the call. Until then the cause is not determined.

What the host-side tests cover: the driver over a scripted fake port, the same conformance suite (`plx_playback_contract::uri_conformance`) over the contract's
`FakeUriDriver` and over this driver, `hosted::testing` wiring (a round trip, STOP as CLOSE, a late completion refused as stale, no host call after `plx_host_stop`),
and the qualifier through the planner's resolver. **Not covered:** a picture on screen through this driver (the window's alpha and the straighten pass are part B2's above; this driver emits no `Presented`), Home with a player open after the round-4 and round-5 fixes (an emulator re-run is pending), the retail player's
native audio order, language codes and `player_select_track` in READY ("The audio track is selected", "Not measured"), a real PMS's answer to `X-Plex-Platform: Tizen`, and everything H6 asks of a retail set (decode, sound, armv7, the TV-menu round trip).

An emulator playback run (the Windows session's, against the mock) has no audio device, so it must serve VIDEO-ONLY media: `python3 tests/mock_pms.py --extra-media clip.mp4 --extra-media-video-only` (the
mock already serves byte ranges; a file with an audio stream is refused without the flag). By the source and its test (`identity::probe_device`, read per request by `plex::identity`; not observed on the wire, the mock's request log keeps no headers) the app sends
`X-Plex-Platform: Tizen` and `X-Plex-Platform-Version: <version> (emulator)`.

The application and package id is `plxntizen0` (ten alphanumerics): a neutral name for the emulator package. A retail package id is
assigned by Samsung's seller process, not chosen here.

## Toolchain (a Linux x86_64 build machine)

WSL2 Ubuntu 22.04 or newer is enough. Versions come from the repository's pins; none is restated here.

- the checkout on the branch named for the run, and `rustup` with the toolchain pinned in `rust-modules/rust-toolchain.toml`
  (the plain host target, no cross compiler; `make` installs nothing);
- `gcc`, `g++`, `make`, `curl`, `tar`, `python3` (3.10 is enough) and binutils (`apt install build-essential curl python3`);
- `cmake` and `pkg-config` (`apt install cmake pkg-config`) for the subtitle renderer's static font stack (`tizen-libass`); a "could not
  discover SDL2" warning from the crates' build scripts is expected and harmless;
- `apt install libgles-dev`: the link stub for `libGLESv2.so`. Keep `libsdl2-dev` and `libsdl2-ttf-dev` off a build machine that
  only builds the library (the host unit tests link them, CI installs them); the library links its own static SDL by path and
  `tizen-verify` fails on a `libSDL2` in `DT_NEEDED` either way;
- a checkout path without spaces (SDL's autoconf cannot build there; under WSL use a path in the Linux filesystem, not `/mnt/c/...`);
- network to `raw.githubusercontent.com` (the nanosvg headers), the SDL release hosts and the libass stack's (FreeType, FriBidi, HarfBuzz,
  libunibreak, libass): every one of those downloads is pinned and
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

## Subtitles off (emulator run, 2026-11-25)

The first retail report (discussion #1460: a 2023 UA65CU7700, Tizen 9.0 platform) logged `uri original: the file has subtitle
streams and none is selected` for every title that had a subtitle stream; the one title without subtitle streams played (Prepared,
`VideoSize 1920x1026`, Started, an advancing position, a clean Stop and Closed, twice). The question that rule left open was whether
the native player draws an embedded subtitle when the app sets none. On the Windows machine's Tizen TV 10.0 emulator, against
`tests/mock_pms.py --movies 0 --extra-media-video-only` with two video-only 180 s clips, each carrying ONE embedded English subtitle
flagged `default` (a `mov_text` track in an MP4 and a SubRip track in an MKV) whose cue covers 0.5 s to 179 s, and with the mock's
selection cleared (`PUT /library/parts/<id>?subtitleStreamID=0`, the state the retail log's `select streams: ... sub=0` shows):

- Before the change, the MP4 play ended in `uri original: the file has subtitle streams and none is selected`, and with the shared
  refusal lifted but `UriSource::can_supply` still answering `false` for Off, in `the certified source did not match this play`.
- After it, both clips played through this driver (Buffering, Prepared, `VideoSize 1280x720`, Started, the timeline advancing; the
  MP4 to 45 s, then a clean Stop and Closed), and captures of the panel at 1.3 s and 25 s (MP4) and at 7.4 s and 15.4 s (MKV) show
  the test pattern with no subtitle text on it.

So on the emulator the native player draws no embedded subtitle the app did not set, including a default-flagged one, and the
driver states `subtitle_off`. **The retail set is not measured**: the next nightly asks the tester to confirm that a title with
subtitles shows none. A selected embedded subtitle was refused then (`SubtitleNotCarried`); "Selected text subtitles" (below) is how
a SubRip or ASS one is now drawn. The emulator has no audio path, so sound is not determinable here.

## Selected text subtitles (emulator probe and run, 2026-11-26)

A selected EMBEDDED text subtitle (SubRip or ASS) is drawn by the app from the host player's cues. The design:

- **Which tracks are carried.** `TizenQualifier` certifies a selected embedded subtitle only when EVERY embedded subtitle stream of
  the Part is SubRip or ASS (`MEASURED_TEXT_CODECS` in `rust-modules/platform-tizen/src/player/qualifier.rs`). A selected PGS, VobSub or
  `mov_text` stream, or a SubRip/ASS one with such a sibling, stays declined (`SubtitleNotCarried`); a burned subtitle and an external
  (PMS sidecar) one stay refused by the planner. External SRT is a later change.
- **How the native track is chosen.** Every embedded subtitle stream is mapped at its position among them, with the same
  `NativeTrackCheck` as audio: `Sole` for a lone stream, else the stream's language, matched by the shared language table against the
  language the host reports for each native text track before `PREPARED` (`PLX_HOST_EVENT_PLAYER_TEXT_LANGUAGE`, 30). The driver asks for
  the index with `PLX_HOST_PLAYER_VERB_TEXT_TRACK` (6); the host calls `player_select_track(PLAYER_STREAM_TYPE_TEXT, index)` and answers
  `PLAYER_TRACK`, as for audio. A pick of another mapped embedded stream mid-play is asked of the host the same way
  (`player_select_track(TEXT)`, measured mid-play above). While that request is unanswered no cue is handed over, and a poll judges its
  cues before the events it settles: the host selects before it answers, so a cue then may already be the new track's, and it is dropped
  rather than drawn as the old one's (the first cue or so of the new track can be lost).
  Picking Off, or another track, from the track menu reaches the driver (`app::playback::commit_track`); only a refused pick re-resolves.
  An accepted Off or pick supersedes a pick the host has not answered yet, as the contract allows: the late answer to the older one
  settles nothing and makes no track the captioned one, so Off picked during a switch stays off. A pick that is refused, by the driver or
  by the host's queue refusing its verb, supersedes nothing: the earlier pick is pending again, and settles or times out as before.
- **How cues flow.** The host registers `player_set_subtitle_updated_cb` ONCE, at `PREPARED`, for a source with a text track (registration
  in READY is what was measured; registering in PLAYING was not, so a mid-play pick after a start with subtitles Off finds the callback
  already there), posts cues only once a text track was answered, and hands every cue to the library through the fourth export,
  `plx_host_post_cue(const PlxHostCue *)` (a 40-byte struct: `size`, `epoch`, `position_ms`, `duration_ms`, `text`, `len`, `reserved`;
  text at most `PLX_HOST_CUE_TEXT_MAX`, 4096 bytes), which has its own bounded lossy queue of 32. The driver turns each cue of the live
  epoch into a `UriCaption`, the shared caption store's `ass_source` takes it, and the app's libass subtitle renderer draws it. The
  cue's `position_ms` is the host's estimate: the last polled position plus the time since, while playing.
- **Off.** There is no text-off verb: Off is the app no longer drawing, and the cues that still come are dropped.
- `PLX_HOST_ABI_VERSION` stays 2: the table and the event layout are unchanged, and the host and the library ship in one package.

**The probe** (the Tizen TV 10.0 x86_64 emulator; the scratch branch `claude/tizen-subtitle-probe`; MKV clips carrying SubRip and ASS
streams, served by `tests/mock_pms.py`):

- The callback's signature is `void (*)(unsigned long duration, char *text, void *user_data)`. `player_set_subtitle_updated_cb`
  returned 0 both before prepare and in READY/PREPARED. The callbacks run on a thread of the player's own, not the main thread.
- The embedded text track's cues arrive WITHOUT `player_set_subtitle_path`; text track 0 is current by default.
- Track count and language: count=1 `[0=en]` for one clip, count=3 `[0=en,1=fr,2=de]` for another (ISO 639-1 codes).
- `player_select_track(PLAYER_STREAM_TYPE_TEXT, n)` returned 0 at PREPARED and mid-play, in 4-5 ms; the next cue came from the new track.
- Timing: a cue arrives when it is due (position 2003 ms for a cue at 2000 ms, duration 1500), and an EMPTY text with duration 0
  arrives at the cue's end (position 3501). After a seek an empty cue arrives at once, and a cue spanning the landing point is not
  re-sent. While paused, no callback fires.
- Markup: the player removes SubRip `<b>`/`<i>` and ASS override blocks (`{\an8\i1}`). It keeps a literal `{brace}`, which the app's
  shared text converter then reads as an override block, as on Feed. A line break arrives as `\n`, ASS `\N` as ` \n`, and `\h` stays
  literal.
- `player_set_subtitle_path` returned 0 for an http URL and for a local file; such a sidecar REPLACES the embedded track (its language
  reads `un`). The app does not use it: the URL would carry the Plex token through the host.
- The TV draws no subtitle of its own, with or without the callback.
- `mov_text` (MP4) cue delivery is not measured, so it stays declined.

**The feature run** (the same emulator; an MKV with an English SubRip and a German ASS track, English selected server-side): the host
logged `text tracks count=2 current=0 [0=en,1=de]` and `text track 0 -> 0`, and cues at [2098..3598], [4000..5500] and
[5995..7495] ms for cues every 2 s lasting 1.5 s. With the package as `make PLATFORM=tizen tizen-tpk` builds it (which stages
`libass-plx` and the subtitle fonts, "The subtitle renderer" above), the line "TWO 07 top bold & / TWO 07 bottom" was drawn at 15.1 s
(the cue runs 14.0-15.5 s) and nothing logged `ASS refused` (2026-11-27). The player had stripped SubRip's `<b>`.

**Not measured on a retail Samsung set:** whether its player raises the callback for an embedded text track and on which thread, its
text track order and language codes, which tags it strips, how `player_select_track(TEXT)` behaves, and `mov_text`.

## The window is opaque and has video (retail report and emulator run, 2026-11-26)

The first retail set (discussion #1460) played video with the translucent window above and sent two photos. The TV's own
home screen (Smart Hub) showed through every pixel the frame cleared to (0, 0, 0, 0): in the letterbox bars above and below
a wide picture, and over the whole screen, under the player's controls, when no picture was up. The emulator never showed
this because nothing was below its window.

The compositor's source explains it (Tizen's E20, read from a 2019 mirror; the TV's current compositor was not read):

- `ecore_wl2_window_alpha_set(win, 1)` sends a null opaque region, which marks the window ARGB, and E20's visibility walk
  (`e_client.c`) never lets an ARGB window hide the windows below it.
- `ecore_wl2_window_video_has(win, 1)` (Tizen-only, public since 6.0) sends `tizen_policy.has_video`. For a window that is
  NOT ARGB the compositor then puts a (0, 0, 0, 0) COPY rectangle under it and blends its buffer's alpha
  (`e_comp_wl_subsurface.c`, `_e_comp_wl_subsurface_check_below_bg_rectangle`). The window's alpha-0 pixels reach the video
  plane, and the windows below stay hidden.

So `ecore::WindowMode` makes the window opaque (alpha 0) and calls `video_has` after the surface exists. It falls back to the
translucent window only where the symbol is missing, because an opaque window without `has_video` shows no video at all.

Measured on the TV 10.0 emulator against `tests/mock_pms.py`:

- The log reads `ecore: ecore_wl2_video bound` and `ecore: window mode OpaqueWithVideo (alpha 0)`.
- A 1280x720 clip plays (Prepared, `VideoSize 1280x720`, Started) and its picture fills the panel.
- The controls draw over the picture.
- Home and the detail page are opaque, before playback and after Back.

**Not measured:** a retail set. Whether Smart Hub stays hidden, and what the letterbox bars show (the plane's own background,
expected black), is the tester's next nightly.

## The audio track is selected (retail report, 2026-11-26; NOT run on a set)

The first retail set (discussion #1460) refused every title with more than one audio track: `TizenQualifier` certified a source with one audio
stream only, because the host had no track call and nobody had measured which track the retail player starts on. It now certifies the selected
stream wherever it sits, and the driver selects it:

- The offer's track map names the selected stream at its position among the Part's audio streams in PMS's order (`audio_ordinal`). A selected
  stream with no known position is still declined (`NotCertified`). The offer also states the stream's identity check (`NativeTrackCheck`):
  `Sole` for a Part with one audio stream, otherwise the stream's language as PMS states it (`languageCode`, e.g. `ger`).
- At PREPARED the host reads the native audio tracks ONCE (`player_get_track_count`, `player_get_current_track` and, for each of at most 32
  tracks, `player_get_track_language_code`) and posts one `PLX_HOST_EVENT_PLAYER_AUDIO_LANGUAGE` per track before `PREPARED` (a = the native
  index, b = the code's two or three ASCII letters, first in the lowest byte; 0 when the player gives none or a code that is not one). That one
  read also answers a held request and makes the log line; nothing on the start path reads the player again.
- A `Sole` stream is settled BY CONSTRUCTION, as before the verb existed: `TrackSelected` and `TrackSettled` at once, no
  `PLX_HOST_PLAYER_VERB_AUDIO_TRACK`, and a `tizen-uri: audio stream S is the only one: selected by construction, no host call` line. The
  player plays the only track it has, so asking it proves nothing, and a firmware that reports no track for an HTTP source in READY (a count of
  0, a failed read, a missing `player_select_track`) would answer -1 and halt the start of every single-audio title, the kind that already
  plays on the tester's set (review round 2 of #1566; pinned in `a_sole_audio_stream_settles_by_construction_without_asking_the_host`).
- For a stream with siblings the driver chooses the native index by that evidence, never by position alone: the ordinal when its
  native track reports the stream's language, else the ONE native track that does. Both sides are folded by the shared language table
  (`plx_data::metadata::lang_matches`, re-exported by `plx_media::route::uri_preflight`), so `ger`, `deu` and `de` are one language, as are
  `fre`/`fra`/`fr`, `cze`/`ces`/`cs` and `dut`/`nld`/`nl` (pinned in `every_spelling_of_one_language_is_one_language`). Several matching tracks
  with the ordinal not among them, no matching track, a native track with no code, or no language on PMS's side (empty, `und`, `mul`, `mis`,
  `zxx`) is a refusal: `Unsupported`, with a `tizen-uri: audio stream S at ordinal O refused: ...` line naming why, and nothing asked of the host.
  An index other than the ordinal is logged too (`... is native track I, not its ordinal O, by its language`).
- Once the player is prepared, a multi-audio start plan's `SelectTrack` becomes `PLX_HOST_PLAYER_VERB_AUDIO_TRACK` (arg0 = the chosen index, arg1 = the
  command id). The host calls `player_select_track(player, PLAYER_STREAM_TYPE_AUDIO, index)` (TizenFX API8 `Interop.Player.cs`; valid in READY,
  PLAYING or PAUSED, so a request before PREPARED is held and applied before PREPARED is posted) and answers `PLAYER_TRACK` (a = the command id,
  b = the index now selected, or -1). An index that is already the current track is answered without a call, and so is index 0 of a player
  that counts exactly one audio track. A player that counts none refuses every index; one that cannot count or say its current track is asked
  (`player_select_track`), and a failed or missing call is a refusal.
- The driver settles the request only on that answer, with the index it asked for; -1, another index, no answer by the control deadline, or a
  refused verb is the command's refusal. A refusal is not a player error: the player keeps the track it had.
- **What a refusal does to the play.** The start plan's `SelectTrack` is mandatory (`is_mandatory_refusal` in
  `rust-modules/media/src/player/backend_startup.rs`, stage `WaitAudio`): a refused one halts the start plan, so the play FAILS with the
  start's typed failure rather than playing a track nobody picked. A multi-audio title whose native track cannot be told by language therefore
  does not play through this driver, and the refusal line says which case it was. A single-audio title never reaches this: it asks the host nothing.
- After `PREPARED` is posted the host logs the native audio tracks in the player's own order, from the same read:
  `player: epoch N: audio tracks count=C current=I [0=eng,1=ger,...]`, then `player: epoch N: audio track I -> R` for each request.
- A pick of another track mid-play is still refused by the session (`carries_source_track_change` stays false): that one flag also gates the
  Quality ladder, and no converted output is certified here.
- No change to the table or event layout, so `PLX_HOST_ABI_VERSION` stays 2. The host and the library ship in one package, and a host without
  the verb answers it with `PLX_HOST_PLAYER_E_ARGS`, which the library takes as a refusal.

**Not measured, and only a retail tester's log can measure them:** (1) whether the player's native audio order is PMS's order on a retail set
(on LG the exported window's service listed a four-track file as AAC, Vorbis, AAC, Vorbis while Plex listed AAC, AAC, Vorbis, Vorbis, so index 1
selected the wrong track: `docs/uri-mode.md`, webOS 5.6.2). The language match catches a swap between tracks of different languages but
NOT one between siblings of one language (an English main track and an English commentary): when several native tracks share the stream's
language and the ordinal is among them, the ordinal is trusted and the player may play the sibling. No second check closes that: language
is the only per-track identity the host reads with a verified call, and the player API's codec and channel reads
(`player_get_audio_stream_info`, `player_get_codec_info`) are of the current stream, not of an index. (2) which language codes the retail player reports, and whether it reports any for an HTTP progressive source; (3) whether
`player_select_track` succeeds in READY for an HTTP progressive source on retail. The PREPARED line and the `audio track I -> R` and
`tizen-uri: audio stream` lines answer all three against the title's audio list in Plex. The emulator has no audio device, so its runs serve
video-only files and no audio track is selected there.

## The media keys are grabbed (retail report and emulator runs, 2026-11-26)

On the first retail set (discussion #1460) the remote's dedicated Play/Pause key paused and resumed the video, and the TV also
raised its own "Not Available" popup. The OK button did not raise it. A key the window has not grabbed is delivered to the app
AND acted on by the TV's shell. flutter-tizen hit the same popup (flutter-tizen issue #319) and removed it by grabbing its media
keys TOPMOST after the window is shown (`BindKeys`, `tizen_window_ecore_wl2.cc`).

`ecore::Window::show` now grabs every transport row of `keys::TABLE` (`keys::grabbed`: Play, Pause, the two Play/Pause toggle
names `XF86PlayBack` and `XF86AudioPlayPause`, Stop, Rewind, Next, Forward). It uses `ecore_wl2_window_keygrab_set(win, name, 0,
0, 0, TOPMOST)`, a `dynlib!` table of its own. As in flutter-tizen there is no unset: the grab goes with the window's surface. TOPMOST gives the window the key while it
is on top of the stack. The Tizen guide lists no privilege for it; the other modes are "not for third-party applications".
Navigation keys are not grabbed.

Measured on the TV 10.0 emulator:

- The log reads `ecore: media keys grabbed TOPMOST; refused []`.
- During playback, Pause (`XF86AudioPause`, code 209) still reaches the app and pauses (`player event Paused`), and Play
  (`XF86AudioPlay`, code 208) resumes (`Started`).
- The skin remote's Play/Pause button is `KEY_PLAY_PAUSE`, code 244, in the skin's `controller/Smart(2016)/layout.xml`.
  The skin numbers its buttons in evdev (Up is 103), and `ecp-cli keycode` takes the same numbers, so `ecp-cli keycode 244`
  presses it from a script. The app receives it as X keycode 252, `XF86PlayBack` -> `PlayPause`.
- The popup appears about 1 s after the key and stays about 2.5 s, so a capture taken right after the key misses it. Frames
  captured every 300 ms for 4 s after code 244:
  - on Smart Hub, where no app grabs the key: "Not Available" shows;
  - on the app's Home, built before the grab (2026-11-25): "Not Available" shows;
  - on the app's Home, built with the grab: the log reads the key and no popup shows.

**Not measured:** whether the popup is gone on a retail set, and which name the retail remote's toggle key sends. It is the
tester's next nightly.

## The system keyboard (emulator run, 2026-11-25)

Search's field raises the television's own on-screen keyboard through Ecore IMF (`platform-tizen/src/ime.rs`). The
platform's `Shell` states `keyboard_available` / `show_keyboard` / `hide_keyboard`, which `textinput` calls where an SDL
shell calls SDL. Every IMF call runs on the host's main thread (`run_on_host`). The sequence is flutter-tizen's
`tizen_input_method_context.cc`: a context from `ecore_imf_context_default_id_get`, the window as its client by
`ecore_wl2_window_id_get`, then `focus_in` + `input_panel_show` to raise it and `input_panel_hide` + `focus_out` to dismiss it.

Measured on the TV 10.0 emulator against `tests/mock_pms.py`:

- **Raise.** OK on the field logs `keyboard: start support=1`, `ime: context_add(wayland)`, `ime: panel show` and
  `ime: panel state Show`. The Samsung keyboard is drawn over the lower part of the window.
- **Typing.** While the panel is up the remote's arrows and OK move and press inside the panel. They do not reach the window.
  Each letter arrives as one COMMIT (`ime: commit (1 bytes)`; the text is never logged) and the query follows it.
- **Keys the panel sends to the window.** Three of the panel's keys arrive as ordinary key events, now in `keys.rs` as
  measured rows:
  - Its delete key is `BackSpace` (code 22) and deletes one character.
  - Done is `Select` (code 0). It is bound to OK, so the field commits the query and closes the panel. Done also
    hides the panel, and its HIDE and its key can arrive in either order; `ime::Panel::done` makes them one decision. A
    `Select` while the panel is up is the OK and the HIDE behind it is the library's own; a `Select` after the HIDE
    already dismissed the field is dropped, so it cannot raise the panel again.
  - Back is `Cancel` (code 231). It has no action, because the panel closes itself.
- **Closed on the panel.** After Back, the panel's STATE callback reports HIDE and the library pushes
  `Input::KeyboardDismissed` (`ime: panel state Hide (closed on the panel)`). The loop turns it into the same
  `SystemKeyboard(false)` the background edge sends, so the field stops owning a keyboard that has gone, and OK on the
  field raises the panel again. A HIDE after the library's own `hide` is not counted as the viewer's (`ime::Panel`).

Not measured: a retail set, other layouts and languages (the `globe` key), the panel's arrows `<` / `>`, and the
`filter_event` forwarding flutter-tizen does while the panel is up, which this emulator did not need.

## The packet player (emulator probe and one retail set, 2026-11-27)

Phase 1 of the Feed port: which native API takes the elementary-stream packets the Feed engine demuxes, what it does on the TV 10.0
emulator, a probe a retail tester can run, and the port's design. The tester's retail Tizen 9.0 set ran the probe from the nightly and
passed every step ("On a retail set", below).

**The candidates, from the firmware itself** (the TV 10.0 x86_64 emulator image; the managed assemblies were read with a
System.Reflection.Metadata dump, the native libraries with `nm -D`, `readelf` and `objdump`):

| API | What it is | Verdict |
| --- | --- | --- |
| `libesplusplayer.so` (`esplusplayer_*`, the image's build is esplusplayer 1.11.3) | the native elementary-stream player: one call per packet (`esplusplayer_submit_packet` with a 64-byte `esplusplayer_es_packet`), per-stream info structs, callbacks for each state change | **chosen**: `dlopen`ed by `libplx.so` itself, by bare SONAME (the managed binding's `/usr/lib64` path is the x86_64 image's; armv7 differs) |
| `Tizen.TV.Multimedia.ESPlayer` (C#) | Samsung's managed binding over the same library: its `DllImport`s name `/usr/lib64/libesplusplayer.so` and the same `esplusplayer_*` entry points | not used: a packet a call through the host's C# boundary buys nothing the direct call lacks, and its `PrivilegeChecker` refuses without the privilege below |
| `libcapi-media-player.so.0` media-stream push (`player_set_media_stream_info`, `player_push_media_stream`, `player_foreach_media_stream_supported_format`) | Tizen's public native player API (3.0 and later): packets as `media_packet_h` over a `media_format_h` | the fallback if a retail set refuses esplusplayer; present on the emulator (all three exported by `libcapi-media-player.so.0.4.0`), not probed beyond that, and the probe logs whether a set has it |

- **The privilege.** The managed binding checks `http://developer.samsung.com/privilege/esplay`, which Samsung's TizenFX TV API
  reference for `ESPlayer` lists at the **Partner** level (with `SDK Support: N`). The native library carries no privilege string (nor do
  `libespplayer-core.so` and `libtrackrenderer.so`). This package does not declare it. On the emulator, create and open succeed without
  it, and so do they on the tester's retail set ("On a retail set", below).
- **The layouts.** The probe compares the library's own `get_size_of_esplusplayer_es_packet`, `..._video_stream_info` and
  `..._audio_stream_info` with this build's structs before any packet is submitted (64, 40 and 32 bytes on x86_64) and refuses on any
  difference: armv7's sizes are the set's, read at run time, not assumed.
- **The protocol** (as the binding drives it, and as the emulator answered): `create`, the callbacks, `open`; `set_ecore_display` on the
  Ecore main thread (the probe hands it to the host thread, `Hosted::on_host`, like the URI player's display) over the library's window
  (`ecore_wl2_display_window_find(display, 1)`); `set_video_stream_info` / `set_audio_stream_info` (the mime numbers are the binding's
  `VideoMimeType`/`AudioMimeType`: H.264 2, HEVC 3, VP9 8, AV1 10, AAC 1, AC3 4, EAC3 5, PCM S16LE 8); `prepare_async`; packets once
  `ready_to_prepare` names their stream; `prepare_async_done(true)`; `start`. Times are milliseconds.
- **The payload is the Feed engine's own**: Annex-B H.264 and HEVC with the parameter sets in-band on the keyframe and no codec data,
  B-frames in decode order, ADTS AAC, AV1 with its `av1C` as codec data, VP9 as demuxed. HDR10 rides the first packet after a prepare
  (and, by the binding's design, after a seek: not verified, since the probe never seeks the `hevc10` clip) as the packet's
  `matroska_color_info` (the mastering display and content light levels); HDR10+ has a metadata pointer of its
  own; the packet has no Dolby Vision field.
- **The clock** is `esplusplayer_get_playing_time` (ms). **Seek** is `esplusplayer_seek(ms)`, then `ready_to_seek(stream, offset)` per
  stream, the feed resumed from the keyframe at or before the offset, then `seek_done`; the probe never calls `esplusplayer_flush`.
  **The end** is `submit_eos_packet` per stream, then the `eos` callback.
- **Threads.** Every callback the emulator raised came on the player's own threads (the probe counts them: 7 in the clock run, none on
  the calling thread), so a callback only posts into a mailbox the worker drains; nothing is done in one.
- **The first picture.** The library exports `esplusplayer_set_first_video_decoding_done_cb`, which the binding does not use. Its setter
  keeps `rdi`, `rsi` and `rdx` exactly as `esplusplayer_set_eos_cb`'s does (handle, callback, user data), and the library's trampoline
  is `listener_bridge::OnFirstDecodingDone(void*)`, the shape of the `OnEos(void*)` behind the binding's `void (*)(void*)` end-of-stream
  callback. That is evidence for the setter's arguments, not a primary statement of the callback's. The probe sets it, before `open`
  like every other callback, from a table of its own, and **the emulator never raised it** (two runs, every video scenario), so it is a
  note in the log (`first-frame: ...`), not a counted step. The tester's retail set never raised it either.
- **Which sets have it.** The emulator image (TV 10.0) has esplusplayer 1.11.3; Samsung's reference gives the managed class version
  5.5.0. The tester's set (discussion #1460, a 2023 UA65CU7700) logged a **Tizen 9.0** platform, not 7.0. Its library opened and its
  sizes matched the build's armv7 structs.

### The probe

**Settings > Diagnostics > Packet player test** runs it, on Tizen only: the row exists where the platform states
`IdentityHooks::packet_probe` (`plx_os::tv::packet_probe`), and the page learns that from the runner's answer to its status query
(`AppFx::PacketProbe`), never from a platform check of its own; every other platform answers `Unavailable` and keeps its three rows. It
survives `RELEASE=1` (it is not a dev trigger). It plays only the clips this package ships (`res/esprobe/*.plxes`, 0.74 MB, made by
`tools/make-esprobe-clips.py` from FFmpeg's test sources, and a PCM tone it makes), never the viewer's media; it asks no server and writes
nothing but its log lines. One run at a time, refused while the host's player is open (`Busy`; only at the start: a title opened during a run contends with
the probe for the decoder, so the tester must not play anything until the row shows the tally); the row then reads `Running` and, at the
end, the tally (`57 passed, 5 failed` on the emulator), so a run whose log was lost still says how it went. The run ends early, with a
`cancelled` FAIL and the current player torn down, when the app goes to the background or stops, and at 180 s; a run stuck in a native
call reads done 30 s after that, with one more FAIL and an `esprobe: timeout:` line. A failure counts once per scenario: what the same
cause raises after it (a later callback, a refused teardown) is an `after the failure:` note.

The code is `rust-modules/platform-tizen/src/esprobe/`: `clip.rs` (the clip format), `api.rs` (the library's functions, structs and
constants, each from the binding), `machine.rs` (the Sans-IO logic: which call next and what an answer means, tested over a simulated
player in `tests.rs`) and `run.rs` (the worker that makes the calls). Every line starts `esprobe:`; a step is `esprobe: PASS <step>
<detail>` or `esprobe: FAIL <step> <detail>`, with return codes by number and name. In order:

1. `fallback libcapi-media-player.so.0: ...` (a note: whether the public push family is there), `api`, `layout`;
2. `h264-clock`, the Feed engine's steps on H.264 alone: `open` (and, once, `privilege`), `display`, `video-info`, `prepare`,
   `position` (advancing after `start`), `pause` (held for 700 ms), `resume`, `seek` (from 3 s to 5.5 s: landed near the target), `eos`,
   `teardown`, and a `first-frame:` note (whether `first_video_decoding_done` came, and when; not counted);
3. `h264+aac` together, then one acceptance run per codec: `hevc`, `hevc10-hdr10` (Main 10, PQ, BT.2020, the HDR10 colour on its first
   packet), `av1`, `vp9`, `aac`, `ac3`, `eac3`, `pcm_s16le`: info accepted, prepared, playing;
4. `esprobe: done pass=N fail=M`.

### On the emulator

The row was pressed on a debug build at `5c3a00875` signed in to the mock (`TIZEN_MOCK_HOST`): the row dimmed and read `Running`, and
the log ended `done pass=57 fail=5`, which the row then showed as `57 passed, 5 failed`; `av1`'s `prepare_async_done(false)` after its
error callback is the note `av1 after the failure: ...`, not a second FAIL. The timings below are from the previous press (`done pass=57
fail=6`, the same steps, from a build that still counted every failure line). A `RELEASE=1` build of the same tree (features `tizen,uri`, no dev triggers: 22,596,196 bytes, the
eight clips staged under `res/esprobe/`) linked and verified, but a release build has no dev boot and opens on the plex.tv sign-in, so
its row was not pressed; the row, its effect and the hook have no feature gate. Earlier runs from a scratch trigger and with the
first-frame callback counted gave the same steps.

| Step | Emulator |
| --- | --- |
| `fallback` | the push family present |
| `api`, `layout` | PASS; sizes 64 / 40 / 32, equal to this build's |
| `privilege` | PASS: create and open without declaring it |
| `h264-clock` | open 6102 ms (the first of a process; later opens 1-3 ms), `set_ecore_display -> 0`, prepared 182 ms after `prepare_async` with 26 packets, advancing 615 ms after `start`, paused at 1547 ms and moved 0 ms in 700 ms, resumed in 206 ms, `seek_done` 156 ms after the seek, landed at 5863 ms for 5500, `eos` at 7737 ms, `stop`/`close`/`destroy` 0 |
| `h264+aac`, `hevc`, `hevc10-hdr10`, `vp9` | PASS (prepared and advancing) |
| `first-frame` | never raised, in any scenario |
| `av1` | FAIL: error callback `-26476497` (`not_supported_video_codec`), then `prepare_async_done(false)` |
| `aac`, `ac3`, `eac3` alone | FAIL: prepared, then the clock stayed at 128 ms. The emulator has no audio path (its process has no PulseAudio connection), so these are **not determined**, not codec verdicts |
| `pcm_s16le` | FAIL: `prepare_async_done(false)`. Not determined: the same emulator, and it decides the TrueHD and DTS plan below |

No picture was seen: the app's window is opaque and the overlay shows only through a `(0, 0, 0, 0)` hole, which neither the player
route's failure read-out nor the Settings page draws.

### On a retail set

The tester (discussion #1460) pressed the row on the armv7 nightly, on the 2023 UA65CU7700 (platform Tizen 9.0, .NET 6.0.9, kernel 5.4),
and posted the log and a photo of the row reading `64 passed, 0 failed`. The log ended `done pass=64 fail=0`:

| Step | Retail set |
| --- | --- |
| `fallback` | the push family present |
| `api`, `layout` | PASS: the library bound by bare SONAME; sizes 48 / 36 / 24, equal to this build's armv7 structs |
| `privilege` | PASS: create and open without declaring it |
| `h264-clock` | open 97 ms (the first of the process; later opens 1 ms), `set_ecore_display -> 0`, prepared 333 ms after `prepare_async` with 26 packets, advancing 207 ms after `start`, paused at 1530 ms and moved 0 ms in 700 ms, resumed in 207 ms, `seek_done` 125 ms after the seek, landed at 5877 ms for 5500, `eos` at 7999 ms, `stop`/`close`/`destroy` 0; 12 callbacks, all on the player's own threads |
| `h264+aac`, `hevc`, `hevc10-hdr10`, `av1`, `vp9` | PASS: prepared in 171-259 ms, advancing about 510 ms after `start` |
| `aac`, `ac3`, `eac3`, `pcm_s16le` alone | PASS: prepared in 64-101 ms, advancing about 508 ms after `start` |
| `first-frame` | never raised, in any scenario (as on the emulator) |

The video runs logged `buffer video underrun` notes, 16 in all: 7 in `h264-clock`, three each in `h264+aac` and `hevc10-hdr10`, one each in
`hevc`, `av1` and `vp9`. Their cause is not determined. The probe feeds a 2 s horizon ahead of the clock (`FEED_AHEAD_MS`), and every
run passed regardless. So most of the Feed port's open questions are answered for this set. The packet player opens without the
Partner privilege, and its armv7 layouts are the build's. Every codec the probe sends is accepted: H.264, HEVC, HEVC Main 10 with
HDR10, AV1, VP9, AAC, AC3, EAC3 and PCM. Opus, which the Feed engine also sends, was not probed, and neither was undecoded DTS. Pause,
seek and EOS behave as on the emulator. PCM prepares and plays, so the TrueHD and DTS plan below holds. No first-picture callback
comes, so the sink cannot wait for one. Still not measured: whether the panel showed a picture (the row runs under the opaque Settings
page), whether anything was heard, and Dolby Vision.

### The port (design; not built)

- **The seam.** The Feed engine reaches the set through `plx_os::tv::sink::VideoSink`, which is Starfish-shaped (`feed_abi_is_cow`, the
  ACB window) and installed through `LegacyWebosMedia.sink`, which Tizen leaves at `NONE`. The OS-neutral sink is step L15b; the Tizen
  sink is that interface's second implementation, so L15b lands first, cut from what both need: open with stream infos, submit (a full
  queue answered, not dropped), EOS, seek with a per-stream resume offset, pause, resume, the clock, close, and events.
- **Direct `dlopen`, not host verbs.** The packet path is a call per frame; `libplx.so` calls `libesplusplayer.so` itself, as the probe
  does. The only host-thread call is `set_ecore_display`, through `Hosted::on_host`.
- **Threads and Sans-IO.** The sink is a runner, not a machine: it owns the player handle on one worker (`plx_base::task::spawn`, inside
  the existing ledgers), submits from the engine's feed, and turns every callback into one event posted to a mailbox (the probe's
  `Mailbox`, keyed by `user_data`, never a static), which the engine drains as it drains Starfish's. The library's reference to a
  mailbox is given back only after a destroy that returned 0; a player left undestroyed (a display bind that never answered, a refused
  destroy, an unwinding worker) keeps its mailbox, and one whose bind is pending gets no call at all (the probe's `UserData`). The
  engine's machines are unchanged.
- **FFmpeg in the TPK.** The same bundled FFmpeg 9.0 (`ci/build-ffmpeg.sh`: LGPL, `-plx` suffix, no network), built for armv7 with the
  pinned NDK the Tizen armv7 link already uses and for x86_64 for the emulator, staged under `res/` and checked by `verify_tpk` the way
  #1576 stages `libass-plx`, and opened by absolute path from the payload directory, as `media-codecs/src/ffi.rs` opens it on webOS. Its
  size in the package is measured at that step, not here.
- **TrueHD and DTS** are decoded in the app to PCM, as on webOS (`ff_pcm::PcmDecoder`), and submitted as PCM, if a retail set prepares
  PCM. The emulator did not, but the tester's retail set did (`pcm_s16le` PASS, above). A set that refuses PCM sends those titles to
  the server's audio transcode.
- **PGS and VobSub** are decoded in the app with FFmpeg and drawn by the GL overlay above the plane, as on webOS; the window's hole and
  straight-alpha mixing ("The video plane", above) apply unchanged.
- **A mid-play track switch** is a reload, the webOS rule: close the player and open a new one at the current position, never a stream
  swap on a live player.
- **Composition.** `player/compose.rs` installs only `TizenUriDriver`. With the Feed sink it installs both, and the Feed's own checks (the
  library opens and its sizes match) run at its first use, never at boot: a viewer who never needs the Feed pays no `dlopen` of the
  player stack (review-checklist item 3), and the planner chooses between them per title as it does on webOS: the URI driver
  for the host player's direct play, Feed for what needs the app's demux (remux, app-decoded audio, picture subtitles).

**What the tester's log answered** ("On a retail set", above):
- a retail Tizen 9.0 set opens `libesplusplayer.so`, with sizes equal to this build's armv7 structs;
- create and open succeed without the Partner privilege;
- it accepts HEVC Main 10 with HDR10 and AV1, and AAC, AC3, EAC3 and PCM on their own (Opus and undecoded DTS were not probed);
- `first_video_decoding_done` does not fire;
- the clock, pause, seek and EOS work as on the emulator;
- the public push family is present.

Not measured anywhere yet: Dolby Vision, Opus, a picture on the plane, sound out of the speakers.

## Known gaps seen on the emulator (2026-11-21)

- The Home hero's Play looks enabled; pressing it opens the player route ("Playback failed / The server cannot play or convert this file", footer `decision_refused`, at the emulator run; the app's own refusal now reads "this TV can’t play this file in PlxNative", footer `no_qualified_backend`) and logs no decision line. That route clears to `(0, 0, 0, 0)`: the emulator shows black behind the read-out (above). The retail set showed its home screen there while the window was translucent; since the window is opaque with video ("The window is opaque and has video", above), what it shows is unmeasured.
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
undefined symbol; the renderer's recipe, facade sources and the staged fonts are others). It restores the Linux simulator job's cache of the
host libass stack (same path and key), prints the configuration, runs `tizen-abi`, `tizen-so` (which builds the renderer first) and
`tizen-verify`, then
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
