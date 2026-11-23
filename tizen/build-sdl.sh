#!/usr/bin/env bash
# Build the STATIC SDL2 + SDL2_ttf that the Tizen library links in (`make PLATFORM=tizen tizen-sdl`).
#
#   tizen/build-sdl.sh PREFIX
#
# Product (PREFIX is gitignored and a cache; a stamp of the pins, this script and the flags skips a rebuild
# that would change nothing):
#   PREFIX/include/SDL2/*.h
#   PREFIX/lib/libSDL2.a  libSDL2_ttf.a      (PIC, linkable into a shared object)
#   PREFIX/link-libs.txt                     the system libraries the archives need (-lm, -ldl, ...), one per line
#   PREFIX/pins.txt                          the versions, for the log
#
# WHY IT BUILDS ITS OWN SDL. A Tizen television ships no SDL at all, and the application's text path (SDL2_ttf over
# SDL surfaces, `plx_gfx`) is SDL's. The library therefore carries both, linked in, and exports neither (the version script
# the makefile generates keeps only the three `plx_host_*` names). The window, the EGL context and the keys stay Tizen's own
# (Ecore_Wl2): SDL never opens a window and its video subsystem is never initialised.
#
# THE PINS ARE NOT HERE. The two upstream archives are entries `sdl2` and `sdl2-ttf` of ci/dependencies.json, read and
# fetched (sha256-verified, machine-wide cache) by ci/deps.py like every other third-party input of this repository.
# SDL2_ttf is built with its own bundled FreeType and WITHOUT HarfBuzz: the Latin and Cyrillic the emulator build shows
# need no shaping, and the application's own FreeType/HarfBuzz pins belong to libass, which this build does not carry.
#
# SDL2 is built with only what the application's SDL calls need: the surface half of the video subsystem with the dummy driver,
# timers, events, threads, file, loadso and cpuinfo. No audio, joystick, haptic, sensor, render, GL/GLES/Vulkan, Wayland/X11/KMS,
# udev, dbus or IME. Everything off is named, not left to autodetection: a build machine that happens to have X11 or ALSA
# development packages must not grow libraries or dlopen paths the television lacks.
#
# Needs: bash, python3, curl, tar, make and a C compiler (`CC`, default `cc`); sha256sum or shasum. No cmake, no pkg-config.
# PLX_SDL_HOST selects a cross build and AR, NM and RANLIB name the matching tools.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
PREFIX="${1:?usage: tizen/build-sdl.sh PREFIX}"
case "$PREFIX" in /*) ;; *) PREFIX="$ROOT/$PREFIX" ;; esac

say() { printf '\n== sdl %s\n' "$*"; }
die() { echo "build-sdl.sh: $*" >&2; exit 1; }

# autoconf and libtool break on a path with a space (WSL checkouts often sit under /mnt/c/Users/...).
case "$ROOT" in *[[:space:]]*) die "the checkout path contains a space ($ROOT); autoconf cannot build there. Clone to a path without spaces (e.g. ~/plx)." ;; esac
for tool in python3 curl tar make; do command -v "$tool" >/dev/null 2>&1 || die "$tool not found (apt install build-essential curl python3)"; done
CC_CMD="${CC:-cc}"
command -v "$CC_CMD" >/dev/null 2>&1 || die "no C compiler ($CC_CMD): apt install build-essential"
AR_CMD="${AR:-ar}"
NM_CMD="${NM:-nm}"
RANLIB_CMD="${RANLIB:-ranlib}"
for tool in "$AR_CMD" "$NM_CMD" "$RANLIB_CMD"; do
  command -v "$tool" >/dev/null 2>&1 || die "$tool not found (binutils)"
done

deps() { python3 "$ROOT/ci/deps.py" "$@"; }
sdl_ver="$(deps get sdl2 version)"
ttf_ver="$(deps get sdl2-ttf version)"
# `fetch` prints the verified path of each asset on stdout, progress on stderr.
SDL_TAR="$(deps fetch sdl2)"
TTF_TAR="$(deps fetch sdl2-ttf)"

jobs() { nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 2; }
sha256_stream() { if command -v sha256sum >/dev/null 2>&1; then sha256sum; else shasum -a 256; fi; }

CFLAGS_ALL="-O2 -g0 -fPIC -ffunction-sections -fdata-sections"
EXTRA_SDL=()
HOST_FLAGS=()
if [ -n "${PLX_SDL_HOST:-}" ]; then
  HOST_FLAGS=(--host="$PLX_SDL_HOST")
  EXTRA_SDL=(--disable-arm-neon --disable-arm-simd)
elif [ "$(uname -s)" = Darwin ]; then
  # Only so the recipe can be exercised on a Mac: no Cocoa/Metal windowing, SDL_Surface and the dummy driver are all it needs.
  EXTRA_SDL=(--disable-video-cocoa --disable-video-metal --disable-render-metal --disable-joystick-mfi)
fi
SDL_CONFIG_FLAGS=(
  --disable-shared --enable-static --with-pic
  --disable-audio --disable-joystick --disable-haptic --disable-hidapi --disable-sensor
  --disable-power --disable-render
  --disable-video-opengl --disable-video-opengles --disable-video-opengles1 --disable-video-opengles2
  --disable-video-vulkan --disable-video-wayland --disable-video-x11 --disable-video-kmsdrm
  --disable-video-rpi --disable-video-vivante --disable-video-offscreen --disable-video-directfb
  --disable-libudev --disable-dbus --disable-ime --disable-ibus --disable-fcitx
  --disable-pipewire --disable-pulseaudio --disable-alsa --disable-jack --disable-sndio
  --disable-esd --disable-arts --disable-nas --disable-libsamplerate
  ${EXTRA_SDL[@]+"${EXTRA_SDL[@]}"}
)
TTF_CONFIG_FLAGS=(--disable-shared --enable-static --with-pic --disable-sdltest --disable-harfbuzz --enable-freetype-builtin)

STAMP_WANT="$( { echo "$sdl_ver $ttf_ver"; deps get sdl2 sha256; deps get sdl2-ttf sha256
                 echo "$CC_CMD|$AR_CMD|$NM_CMD|$RANLIB_CMD|$CFLAGS_ALL|${PLX_SDL_CPPFLAGS:-}|${HOST_FLAGS[*]}|$(uname -sm)"
                 echo "${SDL_CONFIG_FLAGS[*]}"; echo "${TTF_CONFIG_FLAGS[*]}"
                 cat "$0"; } | sha256_stream | cut -d' ' -f1)"
if [ -f "$PREFIX/STAMP" ] && [ "$(cat "$PREFIX/STAMP")" = "$STAMP_WANT" ] \
   && [ -f "$PREFIX/lib/libSDL2.a" ] && [ -f "$PREFIX/lib/libSDL2_ttf.a" ] && [ -f "$PREFIX/link-libs.txt" ]; then
  echo "sdl: up to date ($PREFIX; SDL2 $sdl_ver, SDL2_ttf $ttf_ver)."
  exit 0
fi

WORK="$(mktemp -d "${TMPDIR:-/tmp}/plx-tizen-sdl.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
rm -rf "$PREFIX"
mkdir -p "$PREFIX"
tar -xzf "$SDL_TAR" -C "$WORK"
tar -xzf "$TTF_TAR" -C "$WORK"
SDL_SRC="$(find "$WORK" -maxdepth 1 -type d -name 'SDL2-*' | head -n 1)"
TTF_SRC="$(find "$WORK" -maxdepth 1 -type d -name 'SDL2_ttf-*' | head -n 1)"
[ -x "$SDL_SRC/configure" ] && [ -x "$TTF_SRC/configure" ] || die "an archive has no ./configure (unexpected layout)"

# ./configure treats the source dir as read-only and builds out of tree.
run_configure() { # srcdir builddir flags...
  local src="$1" bld="$2"; shift 2
  mkdir -p "$bld"
  ( cd "$bld" && "$src/configure" --prefix="$PREFIX" ${HOST_FLAGS[@]+"${HOST_FLAGS[@]}"} "$@" \
      CC="$CC_CMD" AR="$AR_CMD" NM="$NM_CMD" RANLIB="$RANLIB_CMD" \
      CPPFLAGS="${PLX_SDL_CPPFLAGS:-}" CFLAGS="$CFLAGS_ALL" ) > "$bld/configure.log" 2>&1 \
    || { tail -40 "$bld/configure.log" >&2; die "configure failed in $bld"; }
}

say "SDL2 $sdl_ver: configure, make"
run_configure "$SDL_SRC" "$WORK/sdl" "${SDL_CONFIG_FLAGS[@]}"
( cd "$WORK/sdl" && make -j"$(jobs)" >make.log 2>&1 && make install >>make.log 2>&1 ) \
  || { tail -40 "$WORK/sdl/make.log" >&2; die "SDL2 build failed"; }

say "SDL2_ttf $ttf_ver (bundled FreeType, no HarfBuzz): configure, make"
run_configure "$TTF_SRC" "$WORK/ttf" --with-sdl-prefix="$PREFIX" "${TTF_CONFIG_FLAGS[@]}"
( cd "$WORK/ttf" && make -j"$(jobs)" libSDL2_ttf.la >make.log 2>&1 ) \
  || { tail -40 "$WORK/ttf/make.log" >&2; die "SDL2_ttf build failed"; }
# libtool's own libSDL2_ttf.a has libSDL2.a nested inside it as a member, which no linker unpacks. The library is SDL_ttf.c plus
# the FreeType modules: archive exactly those objects.
objs="$(cd "$WORK/ttf" && find . -name 'libSDL2_ttf_la-*.o' | sort)"
[ "$(printf '%s\n' "$objs" | wc -l)" -ge 40 ] || die "expected SDL_ttf.o plus ~42 FreeType objects, found: $(printf '%s\n' "$objs" | wc -l)"
( cd "$WORK/ttf" && rm -f "$PREFIX/lib/libSDL2_ttf.a" && "$AR_CMD" rcs "$PREFIX/lib/libSDL2_ttf.a" $objs )
cp "$TTF_SRC/SDL_ttf.h" "$PREFIX/include/SDL2/SDL_ttf.h"

# What the archives need from the system: sdl2-config's static line minus the -L, the archive path and -lSDL2 itself (the makefile
# names the archives by path, so a system libSDL2 on the library path can never be picked up instead).
"$PREFIX/bin/sdl2-config" --static-libs | tr ' ' '\n' | awk 'NF && $0 !~ /^-L/ && $0 !~ /\.a$/ && $0 != "-lSDL2"' > "$PREFIX/link-libs.txt"
echo "SDL2 $sdl_ver, SDL2_ttf $ttf_ver (bundled FreeType, no HarfBuzz)" > "$PREFIX/pins.txt"

say "checks"
syms="$("$NM_CMD" -A "$PREFIX/lib/libSDL2.a" "$PREFIX/lib/libSDL2_ttf.a" 2>/dev/null || true)"
for s in SDL_Init SDL_CreateRGBSurfaceWithFormat SDL_UpperBlit SDL_ConvertSurfaceFormat SDL_GetTicks TTF_Init TTF_OpenFont TTF_RenderUTF8_Blended FT_Init_FreeType; do
  grep -Eq " T _?$s\$" <<<"$syms" || die "the archives define no $s"
done
echo "the archives define the SDL/TTF/FreeType entry points the text path calls"
echo "link-libs: $(tr '\n' ' ' < "$PREFIX/link-libs.txt")"
printf '%s\n' "$STAMP_WANT" > "$PREFIX/STAMP"
ls -l "$PREFIX/lib/libSDL2.a" "$PREFIX/lib/libSDL2_ttf.a"
echo "sdl: done -> $PREFIX"
