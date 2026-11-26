# mk/platform-tizen.mk -- the Tizen (Samsung television) build. `include`d by the Makefile ONLY when PLATFORM=tizen, so the default
# `make` and `make -n` for webOS never read this file (`make -n` is byte-identical with and without it: docs/tizen.md, "Verifying that").
#
# WHAT IT BUILDS. A shared object, `libplx.so`, that the .NET host (tizen/host/PlxHost/) loads and drives through the C exports
# its header declares (the `plx_host_*` functions of rust-modules/platform-tizen/include/plx_host.h), and the signed `.tpk` that
# carries both. The library is the platform crate `plx_platform_tizen` as a cargo STATICLIB (`cargo rustc --crate-type staticlib`,
# the crate itself stays an rlib: ci/test_no_host_staticlib.py), linked by the host C compiler with `--whole-archive` together with
# `svg.o` (the nanosvg rasterizer the UI's icons use) and a STATIC SDL2 + SDL2_ttf built from the pins in ci/dependencies.json
# (tizen/build-sdl.sh). No main.c, access.c, exit.c, starfish.c, video_plane.c or crash recorder: none is referenced on this
# platform (docs/tizen.md). The ASS renderer is NOT linked in either: it is the same separate `libass-plx` the webOS package
# carries (the plx_ass_* facade over a static FreeType/FriBidi/HarfBuzz/libunibreak/libass), opened by name from the
# application directory (`res/`) by the subtitle renderer and the right-to-left text path, and built by `tizen-libass` with
# ci/build-libass.py, the webOS recipe, unchanged in its pins and flags. The Makefile's `CC` (ci/arm-cc.py), `RUST_ENV` (cortex-a9 flags) and `src/svg.o` are the ARM build's and are
# kept separate for the emulator. The ARM build uses the same NDK but its own objects and Cargo tree.
#
# x86_64 targets the Tizen TV 10.0 emulator. armv7 cross-compiles with the pinned NDK for retail
# Tizen 6.0 and newer. The retail library still needs a device run.
#
# THE PLAYBACK FEATURE is this crate's own `uri` (W10 part B: the Tizen URI driver over the host's native media player), NOT the
# application's `play-uri` (the webOS ACB/LS2 driver, never on a Tizen build) or `play-feed`, and `PLAYBACK_BACKENDS` is NOT touched
# (`PLAYBACK_BACKENDS=none` is a parse-time error for any goal outside the Makefile's side-effect-free list). The shipped Tizen
# build plays, so `TIZEN_URI` is 1 and the feature list below names `uri`; `TIZEN_URI=0` builds the library WITHOUT it, which composes no
# driver (`compose: backends=none resolver=no`, the Play control's no-driver path), for the checks that want that boot.
#
# Emulator linking needs Linux x86_64. ARM linking needs an installed NDK.
# Packaging needs .NET 8 with Tizen.NET.Sdk. The dev target uses a Samsung certificate profile in the environment.
# The nightly target creates its own temporary signing keys for Apps2Samsung to replace at install time.
#
#   make PLATFORM=tizen tizen-check     the library's source compiles for the emulator target, and the C# host and plx_host.h agree
#   make PLATFORM=tizen tizen-sdl       the static SDL2 + SDL2_ttf (cached under tizen/sdl-build/)
#   make PLATFORM=tizen tizen-libass    tizen/out/x86_64/libass-plx-host.so.0 (armv7: libass-plx.so.0), the subtitle renderer
#   make PLATFORM=tizen tizen-so        tizen/out/x86_64/libplx.so (and tizen-libass)
#   make PLATFORM=tizen tizen-verify    DT_NEEDED allowlist, undefined symbols, exports, glibc ceiling of both (ci/check-tizen-so.py)
#   make PLATFORM=tizen tizen-stage     the package's res/ and lib/ staged into the host project (what tizen-tpk packs)
#   make PLATFORM=tizen tizen-tpk       the signed package; refuses at once, naming the missing variables, without the signing four
#   make PLATFORM=tizen print-tizen-config   the configuration, one NAME=value per line
#
# A bare `make PLATFORM=tizen` is `tizen-check`: the Makefile's own default goal builds the television package.

TIZEN_ARCH        ?= x86_64
$(if $(and $(filter 1,$(words $(TIZEN_ARCH))),$(filter x86_64 armv7,$(TIZEN_ARCH))),,$(error TIZEN_ARCH "$(TIZEN_ARCH)" must be x86_64 or armv7))
# The compatibility axis (the Rust target, the profile, the managed TFM), recorded once and separate from PLATFORM, which does not choose it.
ifeq ($(TIZEN_ARCH),armv7)
TIZEN_RUST_TARGET  = armv7-unknown-linux-gnueabi
TIZEN_CC          ?= $(CURDIR)/ci/arm-cc.py
TIZEN_CFLAGS       = --sysroot=$(SYSROOT)
TIZEN_RUST_CODEGEN = $(RUST_CODEGEN)
TIZEN_SDL_TOOLS    = PLX_SDL_HOST=arm-linux-gnueabi AR='$(TOOLPREFIX)ar' NM='$(TOOLPREFIX)nm' RANLIB='$(TOOLPREFIX)ranlib'
TIZEN_ELF_TOOLS    = --readelf '$(TOOLPREFIX)readelf' --nm '$(TOOLPREFIX)nm'
TIZEN_ELF_REFERENCE = --against ci/testdata/tizen60-armv7-abi.json
TIZEN_COMPAT_OBJ   = $(TIZEN_BUILD)/getauxval.o
# The renderer is the webOS package's own cross build (ci/build-libass.py without HOST): the same NDK, sysroot and flags.
TIZEN_ASS_NAME     = libass-plx.so.0
TIZEN_ASS_ENV      = WEBOS_SDK='$(WEBOS_SDK)'
TIZEN_GUARD        = test -x '$(TOOLPREFIX)gcc' || { echo 'tizen: install the pinned NDK with make setup-env' >&2; exit 1; }
else
TIZEN_RUST_TARGET  = x86_64-unknown-linux-gnu
TIZEN_CC          ?= cc
# The renderer is ci/build-libass.py's host build (HOST=1), linked by the build host's `cc` like the library below.
TIZEN_ASS_NAME     = libass-plx-host.so.0
TIZEN_ASS_ENV      = HOST=1
TIZEN_GUARD        = test "$$(uname -s)-$$(uname -m)" = Linux-x86_64 || { echo "tizen: the emulator library needs an x86_64 Linux build host" >&2; exit 1; }
endif
TIZEN_TFM          = tizen80
# `tvdev` (no LTO) unless RELEASE=1: the library is not a shipped artifact, no size or frame-rate claim is made of it (docs/tizen.md), and the
# edit-rebuild loop and the CI job should not pay for fat LTO. The profiles are the manifest's own (rust-modules/Cargo.toml).
TIZEN_PROFILE     ?= $(if $(RELEASE),release,tvdev)
$(if $(filter release tvdev,$(TIZEN_PROFILE)),,$(error unknown TIZEN_PROFILE "$(TIZEN_PROFILE)" -- one of: release tvdev))
TIZEN_PROFILE_FLAG = $(if $(filter release,$(TIZEN_PROFILE)),--release,--profile $(TIZEN_PROFILE))
# The crate's feature set. `tizen` is the crate's own gate (its library is empty without it); `uri` is its playback (the driver and its
# composition: `TIZEN_URI=0` leaves it out); `devtriggers` is the compile-time seam the dev boot's preset reads, and a `RELEASE=1` library does
# not carry it (the webOS rule: a shipped build cannot arm a dev boot). Neither application backend (`play-uri`, `play-feed`) is named.
TIZEN_COMMA        = ,
TIZEN_URI         ?= 1
$(if $(filter 0 1,$(TIZEN_URI)),,$(error TIZEN_URI "$(TIZEN_URI)" is not 0 or 1))
TIZEN_FEATURES    ?= tizen$(if $(filter 1,$(TIZEN_URI)),$(TIZEN_COMMA)uri)$(if $(RELEASE),,$(TIZEN_COMMA)devtriggers)

# Derived output. The cargo tree has its own directory (the `target*` glob of tools/build-gc.sh sweeps it); everything else this file
# writes is under tizen/out and tizen/sdl-build (.gitignore).
TIZEN_NIGHTLY_SUBDIR = $(if $(filter nightly,$(FLAVOR)),nightly/,)
TIZEN_TDIR         = target-tizen$(if $(TIZEN_NIGHTLY_SUBDIR),/nightly)
TIZEN_CARGO_DIR    = rust-modules/$(TIZEN_TDIR)/$(TIZEN_RUST_TARGET)/$(TIZEN_PROFILE)
TIZEN_A            = $(TIZEN_CARGO_DIR)/libplx_platform_tizen.a
TIZEN_BUILD        = tizen/out/$(TIZEN_NIGHTLY_SUBDIR)$(TIZEN_ARCH)
TIZEN_SDL          = tizen/sdl-build/$(TIZEN_NIGHTLY_SUBDIR)$(TIZEN_ARCH)
TIZEN_SO           = $(TIZEN_BUILD)/libplx.so
TIZEN_ASS          = $(TIZEN_BUILD)/$(TIZEN_ASS_NAME)
TIZEN_ASS_HEADER   = include/ass.h
TIZEN_MAP          = $(TIZEN_BUILD)/plx_host.map
TIZEN_HEADER       = rust-modules/platform-tizen/include/plx_host.h
TIZEN_HOST_DIR     = tizen/host/PlxHost
TIZEN_HOST_CS      = $(TIZEN_HOST_DIR)/Program.cs
# What Rust's std needs from the system on x86_64-unknown-linux-gnu (`rustc --print native-static-libs`), after the objects, with
# `--as-needed` so a glibc that has folded libpthread/libdl/librt into libc leaves them out of DT_NEEDED. SDL's own are appended from its prefix.
TIZEN_SYSLIBS      = -lgcc_s -lutil -lrt -lpthread -lm -ldl -lc

TIZEN_CARGO_ENV    = env -u PLX_SENTRY_DSN -u PLX_SENTRY_DSN_DEV -u PLX_POSTHOG_KEY -u PLX_POSTHOG_KEY_DEV \
                     PATH="$$HOME/.cargo/bin:$$PATH" CARGO_INCREMENTAL=0 \
                     RUSTFLAGS="$(TIZEN_RUST_CODEGEN) $(RUST_REMAP) $(RUST_CAPLINTS)"

.PHONY: tizen-abi tizen-check tizen-sdl tizen-libass tizen-so tizen-verify tizen-stage tizen-signing tizen-tpk tizen-tpk-nightly print-tizen-config
.DEFAULT_GOAL := tizen-check

# The header's `plx_host_*` exports the .NET host calls, and the C# DllImport list and callback table checked against it. Text only.
tizen-abi:
	python3 ci/check-tizen-so.py abi --header $(TIZEN_HEADER) --host-cs $(TIZEN_HOST_CS)

# Compile-only: no link, no SDL, no certificate, any host with the target's std installed. The local loop's fast check: CI
# (.github/workflows/tizen.yml) runs `tizen-abi` and the full link (`tizen-so`, `tizen-verify`) instead of this.
tizen-check: tizen-abi
	cd rust-modules && $(TIZEN_CARGO_ENV) \
	  cargo +$(RUST_TOOLCHAIN) check --lib $(TIZEN_PROFILE_FLAG) --target $(TIZEN_RUST_TARGET) \
	    -p plx_platform_tizen --no-default-features --features $(TIZEN_FEATURES) --target-dir $(TIZEN_TDIR)

# The static SDL2 + SDL2_ttf, from the sha256-pinned archives `ci/deps.py` fetches (ci/dependencies.json: `sdl2`, `sdl2-ttf`). Idempotent.
tizen-sdl:
	@$(TIZEN_GUARD)
	CC='$(TIZEN_CC)' PLX_SDL_CPPFLAGS='$(TIZEN_CFLAGS)' $(TIZEN_SDL_TOOLS) tizen/build-sdl.sh $(TIZEN_SDL)

# The subtitle renderer and right-to-left shaper, `libass-plx` (docs/tizen.md, "The subtitle renderer"). The recipe is the webOS one,
# ci/build-libass.py, which keeps its own content-keyed stamp of the static stack under vendor/libass-prefix(-host) (shared with the webOS
# package and the simulators, so an unchanged stack is not rebuilt) and relinks the small facade each time. PLX_ASS_STAGE publishes the
# stripped library into this build's directory instead of pkg/, which no Tizen goal writes. It needs cmake and pkg-config.
tizen-libass:
	@$(TIZEN_GUARD)
	$(TIZEN_ASS_ENV) PLX_ASS_STAGE='$(CURDIR)/$(TIZEN_BUILD)' ./ci/build-libass.sh

# The nanosvg rasterizer the UI's icons use, for this host (the ARM `src/svg.o` is a different object). nanosvg's headers are
# staged from the pins by the Makefile's own rule.
$(TIZEN_BUILD)/svg.o: src/svg.c $(NANOSVG_STAMP)
	mkdir -p $(TIZEN_BUILD)
	$(TIZEN_CC) $(TIZEN_CFLAGS) -c src/svg.c -o $@ -O2 -fPIC -Isrc -Ivendor/nanosvg

$(TIZEN_BUILD)/getauxval.o: src/compat/getauxval.c
	mkdir -p $(TIZEN_BUILD)
	$(TIZEN_CC) $(TIZEN_CFLAGS) -c $< -o $@ -O2 -fPIC

# The library's export list: the header's `plx_host_*` functions and nothing else (`local: *`). WITHOUT it every symbol of the whole-archived
# staticlib would be exported, and `--exclude-libs ALL` would hide the ones the host needs.
$(TIZEN_MAP): $(TIZEN_HEADER) ci/check-tizen-so.py
	mkdir -p $(TIZEN_BUILD)
	python3 ci/check-tizen-so.py map --header $(TIZEN_HEADER) --out $@

# `-z defs`: an undefined symbol fails THIS link, not `dlopen` on the emulator (the library is loaded BIND_NOW). `--gc-sections` from the
# exported `plx_host_*` roots drops the code the platform never reaches before the undefined-symbol check, as rustc does for a cdylib.
# `--as-needed` keeps DT_NEEDED to what the link truly uses. SDL's archives precede their dependencies; the system libraries come last.
# The result is written beside its name and renamed, so a failed link never leaves a half-written libplx.so for `tizen-verify` to read.
tizen-so: tizen-sdl tizen-libass $(TIZEN_BUILD)/svg.o $(TIZEN_COMPAT_OBJ) $(TIZEN_MAP)
	@$(TIZEN_GUARD)
	mkdir -p rust-modules/$(TIZEN_TDIR) $(TIZEN_BUILD)
	cd rust-modules && $(TIZEN_CARGO_ENV) \
	  cargo +$(RUST_TOOLCHAIN) rustc $(TIZEN_PROFILE_FLAG) --target $(TIZEN_RUST_TARGET) \
	    -p plx_platform_tizen --lib --crate-type staticlib --target-dir $(TIZEN_TDIR) \
	    --no-default-features --features $(TIZEN_FEATURES) \
	    --message-format=json-render-diagnostics > ../$(TIZEN_BUILD)/cargo-artifacts.json
	python3 ci/check-staticlib-artifact.py $(TIZEN_BUILD)/cargo-artifacts.json $(TIZEN_A) plx_platform_tizen
	$(TIZEN_CC) $(TIZEN_CFLAGS) -shared -o $(TIZEN_SO).part \
	  -Wl,-soname,libplx.so -Wl,-z,defs -Wl,-z,relro -Wl,-z,now -Wl,--as-needed -Wl,--gc-sections \
	  -Wl,--version-script=$(TIZEN_MAP) \
	  -Wl,--whole-archive $(TIZEN_A) -Wl,--no-whole-archive \
	  $(TIZEN_BUILD)/svg.o $(TIZEN_COMPAT_OBJ) $(TIZEN_SDL)/lib/libSDL2_ttf.a $(TIZEN_SDL)/lib/libSDL2.a \
	  -lGLESv2 $$(cat $(TIZEN_SDL)/link-libs.txt) $(TIZEN_SYSLIBS)
	mv -f $(TIZEN_SO).part $(TIZEN_SO)
	ls -l $(TIZEN_SO)

# What the linked library may need and export (ci/check-tizen-so.py verify): the DT_NEEDED allowlist, no undefined symbol outside the
# loader's own, the exports equal to the header's and to the C# DllImport list, no text relocation, SDL/FreeType present and not exported, and
# the glibc ceiling PLX_MAX_GLIBC. `--ass` holds the renderer to its own rules: the C runtime only, exactly the plx_ass_* functions of
# include/ass.h as its exports, the same glibc ceiling and (armv7) the same Tizen 6.0 inventory. Needs the binutils of the build host.
TIZEN_VERIFY = for f in $(TIZEN_SO) $(TIZEN_ASS); do test -f $$f || { echo "tizen-verify: $$f is missing; run: make PLATFORM=tizen tizen-so" >&2; exit 1; }; done; \
	  python3 ci/check-tizen-so.py verify $(TIZEN_SO) --arch $(TIZEN_ARCH) $(TIZEN_ELF_TOOLS) $(TIZEN_ELF_REFERENCE) --header $(TIZEN_HEADER) --host-cs $(TIZEN_HOST_CS) \
	    --ass $(TIZEN_ASS) --ass-header $(TIZEN_ASS_HEADER)
tizen-verify:
	@$(TIZEN_VERIFY)
# `make -j tizen-so tizen-verify` must not read the old library while the new one links: when both are goals, verify waits for the link.
ifneq ($(filter tizen-so,$(MAKECMDGOALS)),)
tizen-verify: | tizen-so
endif

# Stages the package's payload into the host project (tizen/package.py): lib/libplx.so, the renderer and the fonts under res/, OFL.txt, the
# notices, the icon, and the two
# dev-boot inputs. TIZEN_MOCK_HOST=HOST:PORT writes res/dev-boot.json (the mock Plex server the emulator reaches as 10.0.2.2; tests/mock_pms.py)
# and TIZEN_LOG_HOST=IPV4:PORT writes res/log-host.txt (the UDP log channel; tools/tizen-log-receiver.py); a value that is absent writes NO file and
# removes a stale one, so the package is exactly what the command line asked for. The values are addresses, never a credential.
tizen-stage:
	python3 tizen/package.py stage --so $(TIZEN_SO) --ass $(TIZEN_ASS) $(if $(TIZEN_MOCK_HOST),--mock-host '$(TIZEN_MOCK_HOST)') $(if $(TIZEN_LOG_HOST),--log-host '$(TIZEN_LOG_HOST)')

# The signing environment is read by tizen/package.py from the environment itself: PLX_TIZEN_AUTHOR_P12, PLX_TIZEN_AUTHOR_PASS,
# PLX_TIZEN_DIST_P12, PLX_TIZEN_DIST_PASS. A password is NEVER written in this file (make would echo it and put it in argv). This check
# comes before the build, so a missing profile costs seconds, not a link.
tizen-signing:
	python3 tizen/package.py check-signing

# The verification is a RECIPE line, after the link: as a prerequisite of its own it could run beside `tizen-so` under -j and read the old library.
tizen-tpk: tizen-signing tizen-so
	@$(TIZEN_VERIFY)
	python3 tizen/package.py tpk --so $(TIZEN_SO) --ass $(TIZEN_ASS) $(if $(TIZEN_MOCK_HOST),--mock-host '$(TIZEN_MOCK_HOST)') $(if $(TIZEN_LOG_HOST),--log-host '$(TIZEN_LOG_HOST)')

# The configuration, for the CI job's log and a PR body (docs/platform-ports.md: "their variables are printed by the CI job").
print-tizen-config:
	@printf '%s\n' 'PLATFORM=$(PLATFORM)' 'TIZEN_ARCH=$(TIZEN_ARCH)' 'TIZEN_RUST_TARGET=$(TIZEN_RUST_TARGET)' 'TIZEN_TFM=$(TIZEN_TFM)' \
	  'TIZEN_PROFILE=$(TIZEN_PROFILE)' 'TIZEN_FEATURES=$(TIZEN_FEATURES)' 'RUST_TOOLCHAIN=$(RUST_TOOLCHAIN)' 'TIZEN_CC=$(TIZEN_CC)' \
	  'TIZEN_STATICLIB=$(TIZEN_A)' 'TIZEN_SO=$(TIZEN_SO)' 'TIZEN_ASS=$(TIZEN_ASS)' 'TIZEN_SYSLIBS=$(TIZEN_SYSLIBS)'

tizen-tpk-nightly:
	@test '$(RELEASE)' = 1 || { echo 'tizen-tpk-nightly needs RELEASE=1' >&2; exit 1; }
	python3 tizen/package.py check-nightly --label '$(TIZEN_NIGHTLY_LABEL)' $(if $(TIZEN_MOCK_HOST),--mock-host '$(TIZEN_MOCK_HOST)') $(if $(TIZEN_LOG_HOST),--log-host '$(TIZEN_LOG_HOST)')
	$(MAKE) PLATFORM=tizen TIZEN_ARCH=$(TIZEN_ARCH) RELEASE=1 FLAVOR=nightly TIZEN_PROFILE=release \
	  TIZEN_FEATURES=tizen,uri PLX_NIGHTLY_DATE='$(lastword $(subst -, ,$(TIZEN_NIGHTLY_LABEL)))' tizen-so tizen-verify
	python3 tizen/package.py tpk-nightly --so tizen/out/nightly/$(TIZEN_ARCH)/libplx.so --ass tizen/out/nightly/$(TIZEN_ARCH)/$(TIZEN_ASS_NAME) --label '$(TIZEN_NIGHTLY_LABEL)'

# The disk budget at the two build entries of this file (docs/disk-budget.md): both start cargo. ORDER-ONLY, as every entry is. The estimate
# is not a measurement yet: the checked-in table (tools/disk-budget.py NEED_GIB) has no `target-tizen` row.
tizen-so tizen-check: | disk-preflight
tizen-so tizen-check: DISK_TREE = $(CURDIR)/rust-modules/$(TIZEN_TDIR)
tizen-so tizen-check: DISK_NEED = 2.0
DISK_ENTRIES += tizen-so tizen-check
