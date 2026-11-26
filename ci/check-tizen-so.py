#!/usr/bin/env python3
"""The Tizen library's contract with the .NET host, checked in text and on the linked object (mk/platform-tizen.mk; docs/tizen.md).

    python3 ci/check-tizen-so.py abi    --header H --host-cs CS
    python3 ci/check-tizen-so.py map    --header H --out FILE
    python3 ci/check-tizen-so.py verify SO --header H --host-cs CS [--ass ASS --ass-header include/ass.h]

THE CONTRACT has three sides that cannot see one another, in three languages: the C header the Rust crate implements
(rust-modules/platform-tizen/include/plx_host.h), the C# the host loads it with (tizen/host/PlxHost/Program.cs), and the shared
object the linker made. `abi` compares the first two as text and runs ANYWHERE (it is a step of `make check`'s Python gates and of
the CI job): the `plx_host_*` functions the header declares are exactly the ones the host P/Invokes, and the host's mirror of the callback
table (`PlxHostApi`) lists the same fields in the same order, which is the only thing that keeps the struct's layout equal on both sides.
`map` writes the linker's version script from the header: the three names global, everything else local, so SDL, FreeType and every
Rust symbol stay out of the dynamic table (`--exclude-libs ALL` would hide the three exports too). `verify` reads the linked library
with the build host's binutils (`readelf`, `nm`) and needs Linux:

  * DT_NEEDED is an allowlist, not a deny-list: the C runtime, libgcc_s, and libGLESv2.so.2 (which must be there: the renderer's GL imports
    are bound at link time, decision D3 of the Tizen plan). A libSDL2, libwayland-*, libudev ... would mean the static SDL link did not
    happen or the configure picked a library up from the build machine that the television may not have;
  * SONAME is libplx.so, and neither a text relocation (TEXTREL) nor a static TLS flag (STATIC_TLS): a non-PIC object was linked in;
  * every undefined dynamic symbol is the loader's (a versioned glibc/libgcc symbol, never GLIBC_PRIVATE), an unversioned OpenGL ES 2.0
    core function (GLES2_CORE; libGLESv2 has no symbol versions; an ES 3.0 name such as glBindVertexArray is refused), or one of the weak
    toolchain hooks; anything else (`SDL_*`, `TTF_*`, `ecore_*`, `egl*`, `wl_*`) fails: the library is loaded
    BIND_NOW and one missing symbol stops `dlopen` of the whole library;
  * the exported (defined, global) symbols equal the header's `plx_host_*` names and the host's DllImport names, no more and no fewer;
  * the SDL, SDL_ttf and FreeType code the text path calls is INSIDE the object (a local symbol) and none of it is exported;
  * the highest `GLIBC_x.y` symbol version needed is at most PLX_MAX_GLIBC (default 2.40, the emulator image's glibc, the newest
    target we know; the CI runner's is 2.39, so a library linked there is below the ceiling with room to spare and the ceiling only
    catches a host newer than the image). PLX_ALLOW_NEW_GLIBC=1 reports instead of failing.

`--ass` verifies the package's SECOND native library the same way: `libass-plx`, the subtitle renderer and right-to-left shaper that
the application opens by name from `res/` (ci/build-libass.py, the webOS recipe). Its rules are its own: DT_NEEDED is the C runtime
only (ASS_NEEDED_ALLOWED; no libGLESv2, no firmware FreeType/FriBidi/HarfBuzz), its SONAME is the one the build gives this
architecture (ASS_SONAMES, a name `dynlib!` tries), no text relocation or static TLS, every undefined symbol a versioned glibc/libgcc
one, its exports exactly the `plx_ass_*` functions `--ass-header` declares (so no FT_/hb_/fribidi_/ass_ name of the static stack
leaks), the same glibc ceiling and, with `--against`, the same Tizen 6.0 inventory.

Standard library only; Python 3.10 or newer.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path

import deps

PREFIX = "plx_host_"
SONAME = "libplx.so"
DEFAULT_MAX_GLIBC = "2.40"  # the emulator image's glibc; the CI runner (ubuntu-24.04) links against 2.39, below it

ARCHITECTURES = {
    "x86_64": ("ELF64", "Advanced Micro Devices X86-64", "ld-linux-x86-64.so.2", DEFAULT_MAX_GLIBC),
    "armv7": ("ELF32", "ARM", "ld-linux.so.3", "2.30"),
}
NEEDED_ALLOWED = (
    "libc.so.6", "libm.so.6", "libdl.so.2", "libpthread.so.0", "librt.so.1", "libutil.so.1", "libgcc_s.so.1",
    "libGLESv2.so.2",
)
NEEDED_REQUIRED = ("libGLESv2.so.2", "libc.so.6")
# The OpenGL ES 2.0 core entry points (Khronos gl2.h): the whole set libGLESv2 answers unversioned on every Tizen image. A `gl*` outside it
# (glBindVertexArray, glGenVertexArrays, glBlitFramebuffer, glTexImage3D ... are ES 3.0) may be absent from the television's driver, and
# the loader binds every import at dlopen (BIND_NOW), so one missing name stops the whole library.
GLES2_CORE = frozenset("""
glActiveTexture glAttachShader glBindAttribLocation glBindBuffer glBindFramebuffer glBindRenderbuffer glBindTexture glBlendColor
glBlendEquation glBlendEquationSeparate glBlendFunc glBlendFuncSeparate glBufferData glBufferSubData glCheckFramebufferStatus glClear
glClearColor glClearDepthf glClearStencil glColorMask glCompileShader glCompressedTexImage2D glCompressedTexSubImage2D glCopyTexImage2D
glCopyTexSubImage2D glCreateProgram glCreateShader glCullFace glDeleteBuffers glDeleteFramebuffers glDeleteProgram glDeleteRenderbuffers
glDeleteShader glDeleteTextures glDepthFunc glDepthMask glDepthRangef glDetachShader glDisable glDisableVertexAttribArray glDrawArrays
glDrawElements glEnable glEnableVertexAttribArray glFinish glFlush glFramebufferRenderbuffer glFramebufferTexture2D glFrontFace
glGenBuffers glGenerateMipmap glGenFramebuffers glGenRenderbuffers glGenTextures glGetActiveAttrib glGetActiveUniform glGetAttachedShaders
glGetAttribLocation glGetBooleanv glGetBufferParameteriv glGetError glGetFloatv glGetFramebufferAttachmentParameteriv glGetIntegerv
glGetProgramInfoLog glGetProgramiv glGetRenderbufferParameteriv glGetShaderInfoLog glGetShaderPrecisionFormat glGetShaderSource
glGetShaderiv glGetString glGetTexParameterfv glGetTexParameteriv glGetUniformLocation glGetUniformfv glGetUniformiv glGetVertexAttribPointerv
glGetVertexAttribfv glGetVertexAttribiv glHint glIsBuffer glIsEnabled glIsFramebuffer glIsProgram glIsRenderbuffer glIsShader glIsTexture
glLineWidth glLinkProgram glPixelStorei glPolygonOffset glReadPixels glReleaseShaderCompiler glRenderbufferStorage glSampleCoverage
glScissor glShaderBinary glShaderSource glStencilFunc glStencilFuncSeparate glStencilMask glStencilMaskSeparate glStencilOp
glStencilOpSeparate glTexImage2D glTexParameterf glTexParameterfv glTexParameteri glTexParameteriv glTexSubImage2D
glUniform1f glUniform1fv glUniform1i glUniform1iv glUniform2f glUniform2fv glUniform2i glUniform2iv glUniform3f glUniform3fv glUniform3i
glUniform3iv glUniform4f glUniform4fv glUniform4i glUniform4iv glUniformMatrix2fv glUniformMatrix3fv glUniformMatrix4fv glUseProgram
glValidateProgram glVertexAttrib1f glVertexAttrib1fv glVertexAttrib2f glVertexAttrib2fv glVertexAttrib3f glVertexAttrib3fv
glVertexAttrib4f glVertexAttrib4fv glVertexAttribPointer glViewport
""".split())
# Optional toolchain hooks. On ARM the pinned Rust std's sys/{fs,random,thread,thread_local,time}
# tests these weak glibc references before use and falls back when the loader leaves one unresolved.
UNDEFINED_WEAK_OK = {"__gmon_start__", "_ITM_deregisterTMCloneTable", "_ITM_registerTMCloneTable", "__cxa_finalize"}
ARMV7_WEAK_OK = {
    "__clock_gettime64", "__cxa_thread_atexit_impl", "__futimens64", "copy_file_range",
    "getrandom", "gettid", "statx",
}
# The entry points of the text path that must be inside the object: SDL_ttf, its FreeType, SDL's surface.
INSIDE_OBJECT = ("TTF_Init", "TTF_OpenFont", "TTF_RenderUTF8_Blended", "FT_Init_FreeType", "SDL_FreeSurface")
STATIC_PREFIXES = ("SDL_", "TTF_", "FT_")
# The renderer (`--ass`): what its build names it on each architecture (ci/build-libass.py: the ARM cross build and the Linux host build,
# both in the candidate list of rust-modules/media/src/player/ass.rs and gfx/src/rtl.rs), and the C runtime it may link.
ASS_PREFIX = "plx_ass_"
ASS_SONAMES = {"x86_64": "libass-plx-host.so.0", "armv7": "libass-plx.so.0"}
ASS_NEEDED_ALLOWED = ("libc.so.6", "libm.so.6", "libpthread.so.0", "librt.so.1", "libgcc_s.so.1")
TIZEN60_ARCHIVE_ID = "tizen60-abi-reference"
ROOTSTRAP_PREFIX = "data/platforms/tizen-6.0/mobile/rootstraps/mobile-6.0-device.core/"
ROOTSTRAP_ELFS = (
    "lib/ld-2.30.so", "lib/libc-2.30.so", "lib/libdl-2.30.so", "lib/libm-2.30.so",
    "lib/libpthread-2.30.so", "lib/librt-2.30.so", "lib/libutil-2.30.so", "usr/lib/libGLESv2.so.2.0",
)


class CheckError(Exception):
    """A violated rule, with the rule."""


# --- text: the header and the C# -------------------------------------------------------------------------------------------


def strip_c_comments(text: str) -> str:
    text = re.sub(r"/\*.*?\*/", " ", text, flags=re.S)
    return re.sub(r"//[^\n]*", "", text)


def header_functions(text: str) -> list[str]:
    """The `plx_host_*` functions the header declares, in order of appearance. A function-pointer typedef or a struct member
    (`(*plx_host_xxx)(`) is not a declaration, and neither is a macro or a comment."""
    text = re.sub(r"(?m)^[ \t]*#[^\n]*(?:\\\n[^\n]*)*", "", strip_c_comments(text))
    found: list[str] = []
    for match in re.finditer(r"(?<!\w)(" + PREFIX + r"[a-z0-9_]+)\s*\(", text):
        if re.search(r"\(\s*\*\s*$", text[:match.start(1)]):  # `(*plx_host_cb)(...)`: a pointer member or typedef
            continue
        if match.group(1) not in found:
            found.append(match.group(1))
    return found


def csharp_constants(text: str) -> dict[str, str]:
    """`const string NAME = "value";` of the host source, to resolve the library name a DllImport uses."""
    return dict(re.findall(r'\bconst\s+string\s+(\w+)\s*=\s*"([^"]*)"', text))


def csharp_dllimports(text: str, library: str | None = None) -> list[tuple[str, str]]:
    """(library, name) of each `[DllImport(...)] ... extern TYPE NAME(` in the source, in order."""
    text = strip_c_comments(text)
    constants = csharp_constants(text)
    found = []
    pattern = re.compile(r"\[\s*DllImport\s*\(\s*([^,)\]]+)[^\]]*\]\s*(?:\[[^\]]*\]\s*)*[^;{}]*?\bextern\b[^;(]*?\b(\w+)\s*\(", re.S)
    for match in pattern.finditer(text):
        arg = match.group(1).strip()
        lib = arg.strip('"') if arg.startswith('"') else constants.get(arg, arg)
        found.append((lib, match.group(2)))
    if library is not None:
        found = [item for item in found if item[0] == library]
    return found


C_TO_CS = {"int32_t": "int", "uint32_t": "uint", "int64_t": "long", "uint64_t": "ulong", "void": "void"}
# Width classes, for the function-pointer typedefs against the host's delegates (a delegate spells a pointer as IntPtr or string).
WIDTH = {"int32_t": "i4", "uint32_t": "i4", "int64_t": "i8", "uint64_t": "i8", "void": "void",
         "int": "i4", "uint": "i4", "long": "i8", "ulong": "i8"}


def _c_params(text: str) -> int:
    text = re.sub(r"\s+", " ", text.strip())
    return 0 if text in ("", "void") else len(text.split(","))


def header_signatures(text: str) -> dict[str, tuple[str, int]]:
    """name -> (C# spelling of the return type, parameter count) of each `plx_host_*` declaration."""
    text = re.sub(r"(?m)^[ \t]*#[^\n]*(?:\\\n[^\n]*)*", "", strip_c_comments(text))
    found = {}
    for match in re.finditer(r"(?<![\w(])(int32_t|void)\s+(" + PREFIX + r"[a-z0-9_]+)\s*\(([^)]*)\)\s*;", text):
        found[match.group(2)] = (C_TO_CS[match.group(1)], _c_params(match.group(3)))
    return found


def csharp_signatures(text: str, library: str) -> dict[str, tuple[str, int]]:
    """name -> (return type, parameter count) of each DllImport of `library`."""
    text = strip_c_comments(text)
    constants = csharp_constants(text)
    found = {}
    pattern = re.compile(r"\[\s*DllImport\s*\(\s*([^,)\]]+)[^\]]*\]\s*(?:\[[^\]]*\]\s*)*[^;{}]*?\bextern\s+([\w.]+)\s+(\w+)\s*\(([^)]*)\)", re.S)
    for match in pattern.finditer(text):
        arg = match.group(1).strip()
        lib = arg.strip('"') if arg.startswith('"') else constants.get(arg, arg)
        if lib != library:
            continue
        params = re.sub(r"\[[^\]]*\]", "", match.group(4))
        found[match.group(3)] = (match.group(2), _c_params(params))
    return found


def header_defines(text: str) -> dict[str, int]:
    """`#define PLX_HOST_NAME 1` / `(-1)` of the header (integer values only, the size macros are expressions and are not here)."""
    text = strip_c_comments(text)
    return {m.group(1): int(m.group(2)) for m in re.finditer(r"(?m)^[ \t]*#define[ \t]+(PLX_HOST_[A-Z0-9_]+)[ \t]+\(?\s*(-?\d+)\s*\)?[ \t]*$", text)}


def header_struct(text: str, name: str) -> list[tuple[str, str]]:
    """(C type, field name) of `typedef struct NAME { ... } NAME;`, in order."""
    text = strip_c_comments(text)
    match = re.search(r"typedef\s+struct\s+" + name + r"\s*\{(.*?)\}\s*" + name + r"\s*;", text, re.S)
    if not match:
        raise CheckError(f"the header declares no `typedef struct {name}`")
    fields = []
    for decl in match.group(1).split(";"):
        decl = decl.strip()
        if decl:
            parts = decl.split()
            fields.append((" ".join(parts[:-1]), parts[-1].lstrip("*")))
    return fields


def csharp_struct(text: str, name: str) -> list[tuple[str, str]]:
    """(type, name) of the public fields of `struct NAME { ... }`, in order."""
    text = strip_c_comments(text)
    match = re.search(r"\bstruct\s+" + name + r"\s*\{(.*?)\}", text, re.S)
    if not match:
        raise CheckError(f"the host source declares no `struct {name}`: the table must be mirrored field for field")
    return [(m.group(1), m.group(2)) for m in re.finditer(r"\bpublic\s+([\w.]+)\s+(\w+)\s*;", match.group(1))]


def _c_width(param: str) -> str:
    """The width class of one C parameter: any pointer is `ptr`, else its type's."""
    if "*" in param:
        return "ptr"
    return WIDTH.get(re.sub(r"\s+", " ", param.strip()).rsplit(" ", 1)[0].strip(), "ptr")


def header_callbacks(text: str) -> dict[str, tuple[str, list[str]]]:
    """name -> (return width class, parameter width classes) of each `typedef RET (*Name)(PARAMS);` of the header."""
    text = strip_c_comments(text)
    found = {}
    for m in re.finditer(r"typedef\s+([\w\s\*]+?)\s*\(\s*\*\s*(\w+)\s*\)\s*\(([^)]*)\)\s*;", text):
        params = [] if m.group(3).strip() in ("", "void") else [_c_width(p) for p in m.group(3).split(",")]
        found[m.group(2)] = (WIDTH.get(m.group(1).strip(), "ptr"), params)
    return found


def csharp_delegates(text: str) -> dict[str, tuple[str, list[str]]]:
    """name -> (return width class, parameter width classes) of each `delegate RET Name(PARAMS);` of the host."""
    found = {}
    for m in re.finditer(r"\bdelegate\s+([\w.]+)\s+(\w+)\s*\(([^)]*)\)\s*;", strip_c_comments(text)):
        params = [] if not m.group(3).strip() else [WIDTH.get(p.strip().rsplit(" ", 1)[0].strip(), "ptr") for p in m.group(3).split(",")]
        found[m.group(2)] = (WIDTH.get(m.group(1), "ptr"), params)
    return found


def _norm(name: str) -> str:
    return name.replace("_", "").lower()


def _cs_type_of(c_type: str) -> str:
    return C_TO_CS.get(c_type, "IntPtr")  # every other field is a function pointer or a data pointer


def csharp_consts(text: str) -> dict[str, int]:
    """`const int PLX_HOST_NAME = -1;` of the host source: the header's names, spelled as the header spells them."""
    return {m.group(1): int(m.group(2)) for m in re.finditer(r"\bconst\s+(?:int|uint)\s+(PLX_HOST_[A-Z0-9_]+)\s*=\s*(-?\d+)\s*;", strip_c_comments(text))}



def host_library(text: str) -> str:
    """The library name the host's own imports use: the one `Lib` constant of its `Native` class."""
    constants = csharp_constants(strip_c_comments(text))
    if "Lib" not in constants:
        raise CheckError("the host source has no `const string Lib = \"...\"`: its DllImports name the library by that constant")
    return constants["Lib"]


def check_abi(header_text: str, cs_text: str) -> list[str]:
    """Every disagreement between the header and the host, as lines (empty when they agree)."""
    problems: list[str] = []
    library = host_library(cs_text)
    if library != SONAME:
        problems.append(f"the host loads {library!r} but the library's soname is {SONAME!r}")
    functions = header_functions(header_text)
    if not functions:
        problems.append(f"the header declares no {PREFIX}* function")
    imports = [name for _, name in csharp_dllimports(cs_text, library)]
    if sorted(functions) != sorted(imports):
        only_header = sorted(set(functions) - set(imports))
        only_host = sorted(set(imports) - set(functions))
        problems.append(f"the header's functions and the host's DllImports differ: only in the header {only_header}, only in the host {only_host}")
    if len(imports) != len(set(imports)):
        problems.append(f"the host imports a function twice: {imports}")
    theirs, ours = header_signatures(header_text), csharp_signatures(cs_text, library)
    for name in sorted(set(theirs) & set(ours)):
        if theirs[name] != ours[name]:
            problems.append(f"{name}: the header says (returns {theirs[name][0]}, {theirs[name][1]} parameter(s)), the host's DllImport says (returns {ours[name][0]}, {ours[name][1]} parameter(s))")
    for struct in ("PlxHostApi", "PlxHostEvent"):
        c_fields, cs_fields = header_struct(header_text, struct), csharp_struct(cs_text, struct)
        want = [(_cs_type_of(t), _norm(n)) for t, n in c_fields]
        have = [(t, _norm(n)) for t, n in cs_fields]
        if want != have:
            problems.append(f"struct {struct}: the header's fields {[(t, n) for t, n in c_fields]} and the host's {cs_fields} differ in names, order or width "
                            f"(the layouts must be equal: the host marshals this struct by value)")
    consts = csharp_consts(cs_text)
    defines = header_defines(header_text)
    for name, value in sorted(consts.items()):
        if name not in defines:
            problems.append(f"the host declares {name}, which the header does not define")
        elif defines[name] != value:
            problems.append(f"{name} is {value} in the host and {defines[name]} in the header")
    # The host sends the version and every event kind and verb, so it spells each of them (and the check pins each to the header).
    required = ["PLX_HOST_ABI_VERSION"] + sorted(n for n in defines if n.startswith(("PLX_HOST_EVENT_", "PLX_HOST_PLAYER_VERB_")))
    for name in required:
        if name not in consts:
            problems.append(f"the host does not declare {name} (`const int {name} = N;`): it sends this value, and the check pins it to the header")
    # Every function pointer the header types has a delegate of the same name in the host, with the same widths (a pointer is any of IntPtr, string).
    delegates = csharp_delegates(cs_text)
    for name, signature in sorted(header_callbacks(header_text).items()):
        if name not in delegates:
            problems.append(f"the header types the function pointer {name}, and the host declares no `delegate ... {name}(...)`")
        elif delegates[name] != signature:
            problems.append(f"delegate {name}: the header says {signature}, the host's says {delegates[name]} (return and parameter widths must be equal)")
    return problems


def write_map(functions: list[str], out: Path) -> None:
    body = "".join(f"    {name};\n" for name in sorted(functions))
    out.write_text("{\n  global:\n" + body + "  local:\n    *;\n};\n")


# --- the linked object: binutils output, parsed ----------------------------------------------------------------------------


def needed_and_soname(readelf_d: str) -> tuple[list[str], str | None]:
    needed = re.findall(r"Shared library: \[([^\]]+)\]", readelf_d)
    soname = re.search(r"Library soname: \[([^\]]+)\]", readelf_d)
    return needed, soname.group(1) if soname else None


def dynsyms(readelf_syms: str) -> list[dict[str, str]]:
    """The rows of `readelf --dyn-syms -W`: bind, ndx and the name with its version (`memcpy@GLIBC_2.14 (3)` -> `memcpy`, `GLIBC_2.14`)."""
    rows = []
    for line in readelf_syms.splitlines():
        parts = line.split()
        if len(parts) < 8 or not parts[0].rstrip(":").isdigit():
            continue
        raw = " ".join(parts[7:])
        name, _, version = raw.partition("@")
        version = version.lstrip("@").split(" ")[0] if version else ""
        rows.append({"bind": parts[4], "ndx": parts[6], "name": name.strip(), "version": version})
    return rows


def max_glibc(symbols: list[dict[str, str]]) -> str | None:
    versions = [tuple(int(n) for n in m.group(1).split(".")) for row in symbols for m in [re.fullmatch(r"GLIBC_([0-9][0-9.]*)", row["version"])] if m]
    return ".".join(str(n) for n in max(versions)) if versions else None


def version_key(text: str) -> tuple[int, ...]:
    return tuple(int(n) for n in text.split("."))


def verify_object(*, readelf_d: str, readelf_syms: str, nm_local: str, header_text: str, cs_text: str, max_version: str,
                  allow_new_glibc: bool = False, text_relocations: bool = False, static_tls: bool = False,
                  arch: str = "x86_64", readelf_h: str | None = None) -> list[str]:
    """Every rule `verify` states, over already-captured tool output (so a test can feed it canned text)."""
    problems: list[str] = []
    needed, soname = needed_and_soname(readelf_d)
    elf_class, machine, loader, _ = ARCHITECTURES[arch]
    if readelf_h is not None:
        fields = dict(re.findall(r"^\s*(Class|Machine|Flags):\s*([^\n]+)", readelf_h, re.MULTILINE))
        if (fields.get("Class"), fields.get("Machine")) != (elf_class, machine):
            problems.append(f"expected {elf_class} {machine} for {arch}")
        flags = fields.get("Flags", "")
        if arch == "armv7" and ("Version5 EABI" not in flags or "soft-float ABI" not in flags or "hard-float ABI" in flags):
            problems.append(f"expected ARM EABI5 soft-float flags, found {flags!r}")
    allowed = (*NEEDED_ALLOWED, loader)
    bad = [name for name in needed if name not in allowed]
    if bad:
        problems.append(f"DT_NEEDED outside the allowlist: {' '.join(bad)} (allowed: {' '.join(allowed)})")
    for name in NEEDED_REQUIRED:
        if name not in needed:
            problems.append(f"DT_NEEDED lacks {name}")
    if soname != SONAME:
        problems.append(f"SONAME is {soname!r}, not {SONAME!r}")
    if text_relocations:
        problems.append("the library has text relocations (a non-PIC object was linked in)")
    if static_tls:
        problems.append("the library asks for static TLS (DF_STATIC_TLS: a dlopen'ed library cannot be given a static TLS block once the "
                        "process has started; with a text relocation it is the signature of a non-PIC object)")

    symbols = dynsyms(readelf_syms)
    undefined = [row for row in symbols if row["ndx"] == "UND" and row["name"]]
    weak_allowed = UNDEFINED_WEAK_OK | (ARMV7_WEAK_OK if arch == "armv7" else set())
    outside = []
    for row in undefined:
        name = row["name"]
        if row["version"] == "GLIBC_PRIVATE":
            # glibc's internal interface: no promise it exists in the television's glibc, whatever the version number says
            outside.append(name + "@GLIBC_PRIVATE")
            continue
        if row["version"].startswith(("GLIBC_", "GCC_")):
            continue
        if row["bind"] == "WEAK" and not row["version"] and name in weak_allowed:
            continue
        if not row["version"] and name in GLES2_CORE:
            continue  # libGLESv2 has no symbol versions
        outside.append(name + (f"@{row['version']}" if row["version"] else ""))
    if outside:
        problems.append(f"undefined symbols outside the allowlist (the loader binds these at dlopen, BIND_NOW): {' '.join(sorted(set(outside)))}")

    exported = sorted({row["name"] for row in symbols if row["ndx"] not in ("UND", "ABS") and row["bind"] in ("GLOBAL", "WEAK") and row["name"]})
    functions = sorted(header_functions(header_text))
    imports = sorted(name for _, name in csharp_dllimports(cs_text, host_library(cs_text)))
    if exported != functions:
        problems.append(f"exported symbols {exported} are not the header's functions {functions}")
    if exported != imports:
        problems.append(f"exported symbols {exported} are not the host's DllImport names {imports}")
    leaked = [name for name in exported if name.startswith(STATIC_PREFIXES)]
    if leaked:
        problems.append(f"SDL/TTF/FreeType names in the dynamic symbol table: {' '.join(leaked[:5])}")

    local = {m.group(1) for line in nm_local.splitlines() for m in [re.match(r"^[0-9a-fA-F]+\s+[tT]\s+(\S+)$", line.strip())] if m}
    for name in INSIDE_OBJECT:
        if name not in local:
            problems.append(f"{name} is not a code symbol of the library: the static SDL/SDL_ttf/FreeType is missing from the link")

    top = max_glibc(symbols)
    if top is not None and version_key(top) > version_key(max_version):
        message = (f"GLIBC_{top} exceeds the allowed GLIBC_{max_version}: a target with an older glibc refuses to load this. Link on an older "
                   f"host, raise PLX_MAX_GLIBC if you know the target's glibc, or set PLX_ALLOW_NEW_GLIBC=1 to proceed anyway")
        if allow_new_glibc:
            print("WARNING: " + message, file=sys.stderr)
        else:
            problems.append(message)
    return problems


def ass_header_functions(text: str) -> list[str]:
    """The `plx_ass_*` functions include/ass.h declares, sorted (comments and preprocessor lines are not declarations)."""
    text = re.sub(r"(?m)^[ \t]*#[^\n]*(?:\\\n[^\n]*)*", "", strip_c_comments(text))
    return sorted(set(re.findall(r"(?<!\w)(" + ASS_PREFIX + r"[a-z0-9_]+)\s*\(", text)))


def verify_ass_object(*, readelf_d: str, readelf_syms: str, header_text: str, max_version: str, allow_new_glibc: bool = False,
                      text_relocations: bool = False, static_tls: bool = False, arch: str = "x86_64",
                      readelf_h: str | None = None) -> list[str]:
    """Every rule `verify --ass` states for the renderer, over already-captured tool output."""
    problems: list[str] = []
    needed, soname = needed_and_soname(readelf_d)
    elf_class, machine, loader, _ = ARCHITECTURES[arch]
    if readelf_h is not None:
        fields = dict(re.findall(r"^\s*(Class|Machine|Flags):\s*([^\n]+)", readelf_h, re.MULTILINE))
        if (fields.get("Class"), fields.get("Machine")) != (elf_class, machine):
            problems.append(f"renderer: expected {elf_class} {machine} for {arch}")
        flags = fields.get("Flags", "")
        if arch == "armv7" and ("Version5 EABI" not in flags or "soft-float ABI" not in flags or "hard-float ABI" in flags):
            problems.append(f"renderer: expected ARM EABI5 soft-float flags, found {flags!r}")
    allowed = (*ASS_NEEDED_ALLOWED, loader)
    bad = [name for name in needed if name not in allowed]
    if bad:
        problems.append(f"renderer: DT_NEEDED outside the allowlist: {' '.join(bad)} (allowed: {' '.join(allowed)}; the font stack is static)")
    if "libc.so.6" not in needed:
        problems.append("renderer: DT_NEEDED lacks libc.so.6")
    if soname != ASS_SONAMES[arch]:
        problems.append(f"renderer: SONAME is {soname!r}, not {ASS_SONAMES[arch]!r} (the name the application opens on {arch})")
    if text_relocations:
        problems.append("renderer: the library has text relocations (a non-PIC object was linked in)")
    if static_tls:
        problems.append("renderer: the library asks for static TLS (DF_STATIC_TLS), which a dlopen'ed library cannot be given")
    symbols = dynsyms(readelf_syms)
    weak_allowed = UNDEFINED_WEAK_OK | (ARMV7_WEAK_OK if arch == "armv7" else set())
    outside = []
    for row in symbols:
        if row["ndx"] != "UND" or not row["name"]:
            continue
        if row["version"] == "GLIBC_PRIVATE":
            outside.append(row["name"] + "@GLIBC_PRIVATE")
        elif row["version"].startswith(("GLIBC_", "GCC_")):
            continue
        elif not (row["bind"] == "WEAK" and not row["version"] and row["name"] in weak_allowed):
            outside.append(row["name"] + (f"@{row['version']}" if row["version"] else ""))
    if outside:
        problems.append(f"renderer: undefined symbols outside the C runtime: {' '.join(sorted(set(outside)))}")
    exported = sorted({row["name"] for row in symbols if row["ndx"] not in ("UND", "ABS") and row["bind"] in ("GLOBAL", "WEAK") and row["name"]})
    functions = ass_header_functions(header_text)
    if not functions:
        problems.append(f"renderer: the header declares no {ASS_PREFIX}* function")
    elif exported != functions:
        leaked = sorted(set(exported) - set(functions))
        absent = sorted(set(functions) - set(exported))
        problems.append(f"renderer: exports are not the header's {ASS_PREFIX}* functions: exported but not declared {leaked[:8]}, "
                        f"declared but not exported {absent}")
    top = max_glibc(symbols)
    if top is not None and version_key(top) > version_key(max_version):
        message = (f"renderer: GLIBC_{top} exceeds the allowed GLIBC_{max_version} (PLX_MAX_GLIBC / PLX_ALLOW_NEW_GLIBC, as for the library)")
        if allow_new_glibc:
            print("WARNING: " + message, file=sys.stderr)
        else:
            problems.append(message)
    return problems


def run(command: list[str]) -> str:
    try:
        result = subprocess.run(command, capture_output=True, text=True)
    except FileNotFoundError:
        raise CheckError(f"{command[0]} not found: `verify` reads the library with binutils (apt install binutils); it needs a Linux build host")
    if result.returncode != 0:
        raise CheckError(f"{' '.join(command)} failed: {result.stderr.strip()}")
    return result.stdout


def inventory(rootstrap: Path, toolchain: Path, readelf: str) -> dict:
    assets = deps.assets_of(deps.entry(TIZEN60_ARCHIVE_ID))
    paths = {"rootstrap": rootstrap, "gcc": toolchain}
    for key, path in paths.items():
        if deps.digest(path) != assets[key]["sha256"]:
            raise CheckError(f"{key} archive did not match the Tizen 6.0 SDK pin")
    libraries = {}
    with tempfile.TemporaryDirectory(prefix="plx-tizen-abi-") as scratch:
        binary = Path(scratch) / "lib.so"
        for key, path in paths.items():
            with zipfile.ZipFile(path) as archive:
                names = [ROOTSTRAP_PREFIX + name for name in ROOTSTRAP_ELFS] if key == "rootstrap" else [
                    name for name in archive.namelist() if name.endswith("/libgcc_s.so.1")
                ]
                for name in names:
                    binary.write_bytes(archive.read(name))
                    if binary.read_bytes()[:4] != b"\x7fELF":
                        if key == "gcc":
                            continue  # the SDK also records some symlinks as text
                        raise CheckError(f"{name} is not ELF")
                    header = run([readelf, "-h", str(binary)])
                    if not re.search(r"^\s*Machine:\s*ARM\s*$", header, re.MULTILINE):
                        continue
                    _, soname = needed_and_soname(run([readelf, "-d", str(binary)]))
                    if soname is None:
                        raise CheckError(f"{name} has no SONAME")
                    symbols = dynsyms(run([readelf, "--dyn-syms", "-W", str(binary)]))
                    exported = {
                        row["name"] + ("@" + row["version"] if row["version"] else "")
                        for row in symbols if row["ndx"] not in ("UND", "ABS") and row["bind"] in ("GLOBAL", "WEAK", "UNIQUE")
                    }
                    record = {"archive": key, "path": name, "symbols": sorted(exported)}
                    if soname in libraries and libraries[soname]["symbols"] != record["symbols"]:
                        raise CheckError(f"the SDK has different ARM implementations of {soname}")
                    libraries[soname] = record
    required = set(NEEDED_ALLOWED) | {ARCHITECTURES["armv7"][2]}
    if libraries.keys() != required:
        raise CheckError(f"SDK SONAME inventory differs from the ARM allowlist: {sorted(required ^ libraries.keys())}")
    return {"sources": {key: {"url": asset["url"], "sha256": asset["sha256"]} for key, asset in assets.items()},
            "libraries": libraries}


def verify_inventory(needed: list[str], symbols: list[dict[str, str]], reference: dict) -> list[str]:
    libraries = reference["libraries"]
    missing = sorted(set(needed) - libraries.keys())
    available = {symbol for name in needed if name in libraries for symbol in libraries[name]["symbols"]}
    imports = {
        row["name"] + ("@" + row["version"] if row["version"] else "")
        for row in symbols
        if row["ndx"] == "UND" and row["name"] and (row["bind"] != "WEAK" or row["version"])
    }
    problems = [f"Tizen 6.0 SDK has no {name}" for name in missing]
    problems.extend(f"Tizen 6.0 SDK has no export for {symbol}" for symbol in sorted(imports - available))
    return problems


def verify_ass_file(ass: Path, header_text: str, arch: str, readelf: str, max_version: str, reference: dict | None) -> list[str]:
    readelf_d = run([readelf, "-d", str(ass)])
    readelf_syms = run([readelf, "--dyn-syms", "-W", str(ass)])
    problems = verify_ass_object(readelf_d=readelf_d, readelf_syms=readelf_syms, header_text=header_text, max_version=max_version,
                                 allow_new_glibc=arch == "x86_64" and os.environ.get("PLX_ALLOW_NEW_GLIBC") == "1",
                                 text_relocations="TEXTREL" in readelf_d, static_tls="STATIC_TLS" in readelf_d,
                                 arch=arch, readelf_h=run([readelf, "-h", str(ass)]))
    needed, _ = needed_and_soname(readelf_d)
    symbols = dynsyms(readelf_syms)
    if reference is not None:
        sdk_problems = ["renderer: " + problem for problem in verify_inventory(needed, symbols, reference)]
        problems.extend(sdk_problems)
        if not sdk_problems:
            print("Tizen 6.0 ARM SDK: all of the renderer's imports and SONAMEs found")
    print(f"{ass}: {ass.stat().st_size} bytes")
    print("renderer DT_NEEDED: " + " ".join(needed))
    print(f"renderer exports: {len({row['name'] for row in symbols if row['ndx'] not in ('UND', 'ABS') and row['bind'] in ('GLOBAL', 'WEAK')})} "
          f"{ASS_PREFIX}* functions")
    top = max_glibc(symbols)
    print(f"renderer highest glibc symbol version: {'GLIBC_' + top if top else 'none'} (ceiling GLIBC_{max_version})")
    return problems


def verify_file(so: Path, header_text: str, cs_text: str, arch: str, readelf: str, nm: str, against: Path | None,
                ass: Path | None = None, ass_header_text: str | None = None) -> list[str]:
    readelf_h = run([readelf, "-h", str(so)])
    readelf_d = run([readelf, "-d", str(so)])
    readelf_syms = run([readelf, "--dyn-syms", "-W", str(so)])
    nm_local = run([nm, str(so)])
    max_version = ARCHITECTURES[arch][3]
    if arch == "x86_64":
        max_version = os.environ.get("PLX_MAX_GLIBC") or max_version
    problems = verify_object(readelf_d=readelf_d, readelf_syms=readelf_syms, nm_local=nm_local, header_text=header_text, cs_text=cs_text,
                             max_version=max_version,
                             allow_new_glibc=arch == "x86_64" and os.environ.get("PLX_ALLOW_NEW_GLIBC") == "1",
                             text_relocations="TEXTREL" in readelf_d, static_tls="STATIC_TLS" in readelf_d,
                             arch=arch, readelf_h=readelf_h)
    needed, _ = needed_and_soname(readelf_d)
    symbols = dynsyms(readelf_syms)
    if against is not None:
        reference = json.loads(against.read_text())
        sdk_problems = verify_inventory(needed, symbols, reference)
        problems.extend(sdk_problems)
        if not sdk_problems:
            print("Tizen 6.0 ARM SDK: all required imports and SONAMEs found")
    print(f"{so}: {so.stat().st_size} bytes")
    print("DT_NEEDED: " + " ".join(needed))
    print("exports: " + " ".join(sorted({row["name"] for row in dynsyms(readelf_syms) if row["ndx"] not in ("UND", "ABS") and row["bind"] in ("GLOBAL", "WEAK")})))
    top = max_glibc(dynsyms(readelf_syms))
    print(f"highest glibc symbol version: {'GLIBC_' + top if top else 'none'} (ceiling GLIBC_{max_version})")
    if ass is not None:
        problems.extend(verify_ass_file(ass, ass_header_text or "", arch, readelf, max_version,
                                        json.loads(against.read_text()) if against is not None else None))
    return problems


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("abi")
    p.add_argument("--header", required=True, type=Path)
    p.add_argument("--host-cs", required=True, type=Path)
    p = sub.add_parser("map")
    p.add_argument("--header", required=True, type=Path)
    p.add_argument("--out", required=True, type=Path)
    p = sub.add_parser("verify")
    p.add_argument("so", type=Path)
    p.add_argument("--arch", choices=ARCHITECTURES, default="x86_64")
    p.add_argument("--readelf", default="readelf")
    p.add_argument("--nm", default="nm")
    p.add_argument("--against", type=Path, help="check the ARM link against the Tizen 6.0 SDK inventory")
    p.add_argument("--header", required=True, type=Path)
    p.add_argument("--host-cs", required=True, type=Path)
    p.add_argument("--ass", type=Path, help="also verify the subtitle renderer the package carries (libass-plx)")
    p.add_argument("--ass-header", type=Path, help="include/ass.h: the renderer's exports must be exactly its plx_ass_* functions")
    p = sub.add_parser("inventory")
    p.add_argument("--rootstrap", required=True, type=Path)
    p.add_argument("--gcc", required=True, type=Path)
    p.add_argument("--readelf", default="readelf")
    p.add_argument("--out", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        if args.command == "inventory":
            reference = inventory(args.rootstrap, args.gcc, args.readelf)
            args.out.write_text(json.dumps(reference, indent=2, sort_keys=True) + "\n")
            return 0
        header_text = args.header.read_text()
        if args.command == "map":
            functions = header_functions(header_text)
            if not functions:
                raise CheckError(f"{args.header} declares no {PREFIX}* function")
            write_map(functions, args.out)
            return 0
        cs_text = args.host_cs.read_text()
        if args.command == "verify" and (args.ass is None) != (args.ass_header is None):
            raise CheckError("--ass and --ass-header go together: the renderer's exports are checked against the header")
        problems = (check_abi(header_text, cs_text) if args.command == "abi"
                    else verify_file(args.so, header_text, cs_text, args.arch, args.readelf, args.nm, args.against,
                                     args.ass, args.ass_header.read_text() if args.ass_header else None))
    except (CheckError, OSError, ValueError, KeyError, zipfile.BadZipFile) as error:
        print(f"check-tizen-so: {error}", file=sys.stderr)
        return 1
    for problem in problems:
        print(f"check-tizen-so: {problem}", file=sys.stderr)
    if not problems:
        print("check-tizen-so: " + ("the header and the host agree" if args.command == "abi" else
                                    "the library is as the host expects" + (", and the renderer is self-contained" if args.ass else "")))
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
