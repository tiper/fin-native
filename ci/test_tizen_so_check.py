#!/usr/bin/env python3
"""Unit tests of ci/check-tizen-so.py: the parsers and every rule of `verify`, over canned header/C#/binutils text.

The real `verify` reads a linked libplx.so with readelf/nm and needs Linux; these tests need neither: `verify_object` takes the tools' output
as text, so each rule is proven RED here (a bad input names the rule) and the clean input GREEN, on any host that runs `make check`.
"""
import importlib.util
import re
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("check_tizen_so", HERE / "check-tizen-so.py")
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)

HEADER = """
#ifndef PLX_HOST_H
#define PLX_HOST_H
/* plx_host_unrelated(...) in a comment is not a declaration */
#define PLX_HOST_ABI_VERSION 1
#define PLX_HOST_OK 0
#define PLX_HOST_E_ARGS (-1)
#define PLX_HOST_EVENT_PAUSE 1  /* OnPause */
#define PLX_HOST_EVENT_RESUME 2
#define PLX_HOST_EVENT_PLAYER_ERROR 24
#define PLX_HOST_PLAYER_VERB_CLOSE 4
typedef void (*PlxHostTask)(void *arg);
typedef void (*PlxOnStopped)(int32_t code, int32_t clean);
typedef int32_t (*PlxRunOnHost)(PlxHostTask task, void *arg);
typedef int32_t (*PlxPlayerOpen)(uint32_t epoch, const char *uri, int32_t x, int32_t y, int32_t w, int32_t h);
typedef int32_t (*PlxPlayerVerb)(uint32_t epoch, uint32_t verb, int64_t arg0, int64_t arg1);
typedef struct PlxHostApi {
    uint32_t size;
    uint32_t abi_version;
    uint32_t flags;
    uint32_t reserved;
    PlxOnStopped on_stopped;
    PlxRunOnHost run_on_host;
    PlxPlayerOpen player_open;
    PlxPlayerVerb player_verb;
} PlxHostApi;
typedef struct PlxHostEvent {
    uint32_t size;
    uint32_t kind;
    uint32_t epoch;
    uint32_t reserved;
    int64_t a;
    int64_t b;
} PlxHostEvent;
#define PLX_HOST_API_V1_SIZE ((uint32_t)(offsetof(PlxHostApi, run_on_host) + sizeof(PlxRunOnHost)))
int32_t plx_host_start(const PlxHostApi *api, const char *launch_json);
int32_t plx_host_post(const PlxHostEvent *event);
void plx_host_stop(void);
#endif
"""

CSHARP = """
static class Native
{
    const string Lib = "libplx.so";
    const string Dl = "libdl.so.2";
    // [DllImport(Lib)] static extern void plx_host_commented(int a);
    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int plx_host_start(ref PlxHostApi api, [MarshalAs(UnmanagedType.LPUTF8Str)] string launchJson);
    [DllImport(Lib)]
    public static extern int plx_host_post(ref PlxHostEvent ev);
    [DllImport(Lib)]
    [SuppressGCTransition]
    public static extern void plx_host_stop();
    [DllImport(Dl)]
    public static extern IntPtr dlopen(string path, int flags);
    public const int PLX_HOST_ABI_VERSION = 1;
    public const int PLX_HOST_E_ARGS = -1;
    public const int PLX_HOST_EVENT_PAUSE = 1;
    public const uint PLX_HOST_EVENT_RESUME = 2;
    public const int PLX_HOST_EVENT_PLAYER_ERROR = 24;
    public const int PLX_HOST_PLAYER_VERB_CLOSE = 4;
}
[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate void PlxHostTask(IntPtr arg);
[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate void PlxOnStopped(int code, int clean);
[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate int PlxRunOnHost(IntPtr task, IntPtr arg);
[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate int PlxPlayerOpen(uint epoch, IntPtr uri, int x, int y, int w, int h);
[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
internal delegate int PlxPlayerVerb(uint epoch, uint verb, long arg0, long arg1);
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
[StructLayout(LayoutKind.Sequential)]
internal struct PlxHostEvent { public uint Size; public uint Kind; public uint Epoch; public uint Reserved; public long A; public long B; }
"""

READELF_D = """
Dynamic section at offset 0x1000 contains 20 entries:
  Tag        Type                         Name/Value
 0x0000000000000001 (NEEDED)             Shared library: [libGLESv2.so.2]
 0x0000000000000001 (NEEDED)             Shared library: [libgcc_s.so.1]
 0x0000000000000001 (NEEDED)             Shared library: [libc.so.6]
 0x0000000000000001 (NEEDED)             Shared library: [ld-linux-x86-64.so.2]
 0x000000000000000e (SONAME)             Library soname: [libplx.so]
"""

READELF_SYMS = """
Symbol table '.dynsym' contains 8 entries:
   Num:    Value          Size Type    Bind   Vis      Ndx Name
     0: 0000000000000000     0 NOTYPE  LOCAL  DEFAULT  UND
     1: 0000000000000000     0 FUNC    GLOBAL DEFAULT  UND memcpy@GLIBC_2.14 (2)
     2: 0000000000000000     0 FUNC    GLOBAL DEFAULT  UND glClear
     3: 0000000000000000     0 NOTYPE  WEAK   DEFAULT  UND __gmon_start__
     4: 0000000000000000     0 FUNC    GLOBAL DEFAULT  UND _Unwind_Resume@GCC_3.0 (3)
     5: 0000000000001000    40 FUNC    GLOBAL DEFAULT   12 plx_host_start
     6: 0000000000001100    40 FUNC    GLOBAL DEFAULT   12 plx_host_post
     7: 0000000000001200    40 FUNC    GLOBAL DEFAULT   12 plx_host_stop
"""

NM = """
0000000000002000 t TTF_Init
0000000000002100 t TTF_OpenFont
0000000000002200 t TTF_RenderUTF8_Blended
0000000000002300 t FT_Init_FreeType
0000000000002400 t SDL_FreeSurface
0000000000001000 T plx_host_start
"""


def verify(**overrides):
    args = dict(readelf_d=READELF_D, readelf_syms=READELF_SYMS, nm_local=NM, header_text=HEADER, cs_text=CSHARP, max_version=checker.DEFAULT_MAX_GLIBC)
    args.update(overrides)
    return checker.verify_object(**args)


class ArmLink(unittest.TestCase):
    header = "  Class: ELF32\n  Machine: ARM\n  Flags: 0x5000200, Version5 EABI, soft-float ABI\n"

    def check(self, **overrides):
        args = dict(arch="armv7", readelf_h=self.header, max_version="2.30",
                    readelf_d=READELF_D.replace("ld-linux-x86-64.so.2", "ld-linux.so.3"))
        args.update(overrides)
        return verify(**args)

    def test_an_eabi5_soft_float_elf_with_arm_loader_passes(self):
        self.assertEqual(self.check(), [])

    def test_wrong_class_float_abi_and_loader_fail(self):
        self.assertTrue(any("ELF32" in p for p in self.check(readelf_h=self.header.replace("ELF32", "ELF64"))))
        self.assertTrue(any("soft-float" in p for p in self.check(readelf_h=self.header.replace("soft-float", "hard-float"))))
        for loader in ("ld-linux-x86-64.so.2", "ld-linux-armhf.so.3"):
            self.assertTrue(any(loader in p for p in self.check(readelf_d=READELF_D.replace("ld-linux-x86-64.so.2", loader))))

    def test_arm_ceiling_and_weak_reference_fallback(self):
        self.assertTrue(any("GLIBC_2.31" in p for p in self.check(readelf_syms=READELF_SYMS.replace("2.14", "2.31"))))
        weak = READELF_SYMS.replace("__gmon_start__", "__clock_gettime64")
        self.assertEqual(self.check(readelf_syms=weak), [])
        self.assertTrue(any("__clock_gettime64" in p for p in self.check(readelf_syms=weak.replace("WEAK", "GLOBAL"))))

    def test_sdk_requires_exact_versions_from_needed_sonames(self):
        symbols = checker.dynsyms(READELF_SYMS)
        reference = {"libraries": {"libc.so.6": {"symbols": ["memcpy@GLIBC_2.14"]},
                                  "libGLESv2.so.2": {"symbols": ["glClear"]},
                                  "libgcc_s.so.1": {"symbols": ["_Unwind_Resume@GCC_3.0"]}}}
        needed = list(reference["libraries"])
        self.assertEqual(checker.verify_inventory(needed, symbols, reference), [])
        problems = checker.verify_inventory(["libc.so.6", "ld-linux.so.3"], symbols, reference)
        self.assertIn("Tizen 6.0 SDK has no ld-linux.so.3", problems)
        self.assertIn("Tizen 6.0 SDK has no export for glClear", problems)
        symbols[0]["version"] = "GLIBC_2.12"
        self.assertIn("Tizen 6.0 SDK has no export for memcpy@GLIBC_2.12",
                      checker.verify_inventory(needed, symbols, reference))
        symbols[0]["bind"] = "WEAK"
        self.assertIn("Tizen 6.0 SDK has no export for memcpy@GLIBC_2.12",
                      checker.verify_inventory(needed, symbols, reference))


class TextParsers(unittest.TestCase):
    def test_header_functions_are_the_declarations_only(self):
        self.assertEqual(checker.header_functions(HEADER), ["plx_host_start", "plx_host_post", "plx_host_stop"])

    def test_the_real_header_has_three_functions_and_two_tables(self):
        real = (HERE.parent / "rust-modules/platform-tizen/include/plx_host.h").read_text()
        self.assertEqual(checker.header_functions(real), ["plx_host_start", "plx_host_post", "plx_host_stop"])
        self.assertEqual([n for _, n in checker.header_struct(real, "PlxHostApi")],
                         ["size", "abi_version", "flags", "reserved", "on_stopped", "run_on_host", "player_open", "player_verb"])
        self.assertEqual(checker.header_defines(real)["PLX_HOST_E_FULL"], -6)

    def test_the_shipped_host_and_the_shipped_header_agree(self):
        """THE gate of the contract: tizen/host/PlxHost/Program.cs against rust-modules/platform-tizen/include/plx_host.h, as they are."""
        header = (HERE.parent / "rust-modules/platform-tizen/include/plx_host.h").read_text()
        host = (HERE.parent / "tizen/host/PlxHost/Program.cs").read_text()
        self.assertEqual(checker.check_abi(header, host), [])

    def test_a_pointer_returning_function_is_a_declaration(self):
        self.assertEqual(checker.header_functions("const char *plx_host_version(void);\nchar* plx_host_other(void);"), ["plx_host_version", "plx_host_other"])

    def test_csharp_dllimports_resolve_the_library_constant(self):
        imports = checker.csharp_dllimports(CSHARP)
        self.assertEqual(imports, [("libplx.so", "plx_host_start"), ("libplx.so", "plx_host_post"), ("libplx.so", "plx_host_stop"),
                                   ("libdl.so.2", "dlopen")])
        self.assertEqual(checker.csharp_signatures(CSHARP, "libplx.so"),
                         {"plx_host_start": ("int", 2), "plx_host_post": ("int", 1), "plx_host_stop": ("void", 0)})
        self.assertEqual(checker.header_signatures(HEADER), {"plx_host_start": ("int", 2), "plx_host_post": ("int", 1), "plx_host_stop": ("void", 0)})
        self.assertEqual([n for _, n in checker.csharp_dllimports(CSHARP, "libplx.so")], ["plx_host_start", "plx_host_post", "plx_host_stop"])

    def test_abi_agrees(self):
        self.assertEqual(checker.check_abi(HEADER, CSHARP), [])

    def test_abi_names_a_changed_signature(self):
        problems = checker.check_abi(HEADER.replace("void plx_host_stop(void);", "void plx_host_stop(int code);"), CSHARP)
        self.assertTrue(any("plx_host_stop" in p and "parameter" in p for p in problems), problems)
        problems = checker.check_abi(HEADER.replace("int32_t plx_host_post", "void plx_host_post"), CSHARP)
        self.assertTrue(any("plx_host_post" in p and "returns" in p for p in problems), problems)

    def test_abi_names_a_table_whose_fields_differ(self):
        for bad in (CSHARP.replace("public uint Flags;\n    public uint Reserved;", "public uint Reserved;\n    public uint Flags;"),
                    CSHARP.replace("public IntPtr RunOnHost;", "public int RunOnHost;"),
                    CSHARP.replace("public IntPtr OnStopped;\n", ""),
                    CSHARP.replace("public uint Kind;", "public uint Type;")):
            problems = checker.check_abi(HEADER, bad)
            self.assertTrue(any(p.startswith("struct Plx") for p in problems), (bad, problems))

    def test_abi_names_a_constant_that_differs_or_is_missing(self):
        problems = checker.check_abi(HEADER, CSHARP.replace("PLX_HOST_E_ARGS = -1", "PLX_HOST_E_ARGS = -2"))
        self.assertTrue(any("PLX_HOST_E_ARGS is -2" in p for p in problems), problems)
        problems = checker.check_abi(HEADER, CSHARP.replace("PLX_HOST_EVENT_PAUSE = 1;", ""))
        self.assertTrue(any("PLX_HOST_EVENT_PAUSE" in p and "does not declare" in p for p in problems), problems)
        problems = checker.check_abi(HEADER, CSHARP.replace("PLX_HOST_E_ARGS = -1", "PLX_HOST_E_NEW = 7"))
        self.assertTrue(any("PLX_HOST_E_NEW" in p for p in problems), problems)

    def test_abi_names_a_64_bit_field_declared_as_a_pointer_or_32_bit(self):
        for bad in (CSHARP.replace("public long A;", "public IntPtr A;"), CSHARP.replace("public long B;", "public int B;")):
            problems = checker.check_abi(HEADER, bad)
            self.assertTrue(any(p.startswith("struct PlxHostEvent") for p in problems), (bad, problems))

    def test_abi_names_a_delegate_whose_widths_differ_or_is_missing(self):
        problems = checker.check_abi(HEADER, CSHARP.replace("long arg0, long arg1", "int arg0, long arg1"))
        self.assertTrue(any("delegate PlxPlayerVerb" in p for p in problems), problems)
        problems = checker.check_abi(HEADER, CSHARP.replace("uint epoch, IntPtr uri, int x", "uint epoch, int x"))
        self.assertTrue(any("delegate PlxPlayerOpen" in p for p in problems), problems)
        problems = checker.check_abi(HEADER, CSHARP.replace("internal delegate int PlxPlayerOpen(uint epoch, IntPtr uri, int x, int y, int w, int h);", ""))
        self.assertTrue(any("PlxPlayerOpen" in p and "declares no" in p for p in problems), problems)

    def test_abi_requires_every_event_kind_and_verb_the_header_defines(self):
        for name in ("PLX_HOST_EVENT_PLAYER_ERROR = 24;", "PLX_HOST_PLAYER_VERB_CLOSE = 4;"):
            problems = checker.check_abi(HEADER, CSHARP.replace(name, ""))
            self.assertTrue(any(name.split(" ")[0] in p and "does not declare" in p for p in problems), problems)

    def test_abi_names_a_library_that_is_not_the_soname(self):
        problems = checker.check_abi(HEADER, CSHARP.replace('Lib = "libplx.so"', 'Lib = "plx"'))
        self.assertTrue(any("soname" in p for p in problems), problems)

    def test_abi_names_a_function_the_host_does_not_import(self):
        problems = checker.check_abi(HEADER + "int32_t plx_host_extra(void);\n", CSHARP)
        self.assertEqual(len(problems), 1)
        self.assertIn("plx_host_extra", problems[0])

    def test_abi_names_a_function_the_header_lacks(self):
        problems = checker.check_abi(HEADER.replace("void plx_host_stop(void);", ""), CSHARP)
        self.assertTrue(any("plx_host_stop" in p for p in problems), problems)

    def test_map_has_exactly_the_header_names_global(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "plx.map"
            checker.write_map(checker.header_functions(HEADER), out)
            self.assertEqual(out.read_text(), "{\n  global:\n    plx_host_post;\n    plx_host_start;\n    plx_host_stop;\n  local:\n    *;\n};\n")


class BinutilsParsers(unittest.TestCase):
    def test_needed_and_soname(self):
        needed, soname = checker.needed_and_soname(READELF_D)
        self.assertEqual(needed, ["libGLESv2.so.2", "libgcc_s.so.1", "libc.so.6", "ld-linux-x86-64.so.2"])
        self.assertEqual(soname, "libplx.so")

    def test_dynsyms_split_name_and_version(self):
        rows = checker.dynsyms(READELF_SYMS)
        memcpy = next(r for r in rows if r["name"] == "memcpy")
        self.assertEqual((memcpy["version"], memcpy["ndx"]), ("GLIBC_2.14", "UND"))
        self.assertEqual(next(r for r in rows if r["name"] == "plx_host_stop")["ndx"], "12")

    def test_max_glibc_is_numeric_not_lexical(self):
        rows = [{"version": "GLIBC_2.9"}, {"version": "GLIBC_2.14"}, {"version": "GCC_3.0"}, {"version": ""}]
        self.assertEqual(checker.max_glibc(rows), "2.14")
        self.assertIsNone(checker.max_glibc([{"version": ""}]))


class VerifyRules(unittest.TestCase):
    def test_clean_library_passes(self):
        self.assertEqual(verify(), [])

    def test_a_dynamic_sdl_is_refused(self):
        problems = verify(readelf_d=READELF_D + " 0x0000000000000001 (NEEDED)             Shared library: [libSDL2-2.0.so.0]\n")
        self.assertTrue(any("allowlist" in p and "libSDL2-2.0.so.0" in p for p in problems), problems)

    def test_missing_gles_is_refused(self):
        problems = verify(readelf_d=READELF_D.replace("libGLESv2.so.2", "libm.so.6"))
        self.assertTrue(any("lacks libGLESv2.so.2" in p for p in problems), problems)

    def test_wrong_soname_is_refused(self):
        self.assertTrue(any("SONAME" in p for p in verify(readelf_d=READELF_D.replace("[libplx.so]", "[libother.so]"))))

    def test_text_relocation_is_refused(self):
        self.assertTrue(any("text relocations" in p for p in verify(text_relocations=True)))

    def test_an_unresolved_sdl_symbol_is_refused(self):
        syms = READELF_SYMS + "     8: 0000000000000000     0 FUNC    GLOBAL DEFAULT  UND SDL_Init\n"
        problems = verify(readelf_syms=syms)
        self.assertTrue(any("SDL_Init" in p and "undefined" in p for p in problems), problems)

    def test_a_versioned_gl_name_is_not_the_loaders_to_answer(self):
        # `glClear` unversioned is libGLESv2's; a `gl*` WITH a version would be something else's
        syms = READELF_SYMS + "     8: 0000000000000000     0 FUNC    GLOBAL DEFAULT  UND glFoo@VER_1 (4)\n"
        self.assertTrue(any("glFoo@VER_1" in p for p in verify(readelf_syms=syms)))

    def test_an_extra_export_is_refused(self):
        syms = READELF_SYMS + "     8: 0000000000003000    10 FUNC    GLOBAL DEFAULT   12 rust_eh_personality\n"
        problems = verify(readelf_syms=syms)
        self.assertTrue(any("rust_eh_personality" in p for p in problems), problems)

    def test_a_leaked_sdl_export_is_named(self):
        syms = READELF_SYMS + "     8: 0000000000003000    10 FUNC    GLOBAL DEFAULT   12 SDL_Quit\n"
        self.assertTrue(any("dynamic symbol table" in p and "SDL_Quit" in p for p in verify(readelf_syms=syms)))

    def test_a_missing_export_is_refused(self):
        syms = READELF_SYMS.replace("plx_host_stop", "plx_host_zzz")
        self.assertTrue(any("not the header's functions" in p for p in verify(readelf_syms=syms)))

    def test_static_sdl_must_be_inside(self):
        problems = verify(nm_local=NM.replace("TTF_OpenFont", "other"))
        self.assertTrue(any("TTF_OpenFont" in p and "missing from the link" in p for p in problems), problems)

    def test_glibc_ceiling(self):
        syms = READELF_SYMS + "     8: 0000000000000000     0 FUNC    GLOBAL DEFAULT  UND pidfd_open@GLIBC_2.39 (5)\n"
        self.assertEqual(verify(readelf_syms=syms, max_version="2.39"), [])
        problems = verify(readelf_syms=syms, max_version="2.38")
        self.assertTrue(any("GLIBC_2.39 exceeds" in p for p in problems), problems)
        self.assertEqual(verify(readelf_syms=syms, max_version="2.38", allow_new_glibc=True), [])

    def test_an_es3_gl_import_is_refused(self):
        # glBindVertexArray is OpenGL ES 3.0 (the desktop simulator's path): the television's libGLESv2 may not answer it, and BIND_NOW makes
        # one missing name the whole library's dlopen failure
        for name in ("glBindVertexArray", "glBlitFramebuffer", "glGenVertexArrays", "glTexImage3D"):
            syms = READELF_SYMS + f"     8: 0000000000000000     0 FUNC    GLOBAL DEFAULT  UND {name}\n"
            problems = verify(readelf_syms=syms)
            self.assertTrue(any(name in p and "allowlist" in p for p in problems), (name, problems))

    def test_every_gl_function_the_renderer_calls_on_the_tizen_build_is_es2_core(self):
        # the application declares its GL externs in gfx.rs and surface.rs; the ones behind `sim` are the simulator's (the blit; the VAO externs are the desktop platform's own and are not in these files)
        root = Path(__file__).resolve().parent.parent / "rust-modules"
        declared = set()
        for path in list(root.glob("gfx/src/*.rs")) + [root / "base/src/surface.rs"]:
            text = path.read_text(encoding="utf-8")
            # drop every item that follows a `#[cfg(feature = "sim")]` attribute: the next `extern "C" {...}` block or `fn ...}`
            text = re.sub(r'#\[cfg\(feature = "sim"\)\]\s*(?:pub\s+)?(?:unsafe\s+)?(?:extern "C"\s*)?(?:fn[^{]*)?\{.*?\n\}\n', "", text, flags=re.S)
            declared |= set(re.findall(r"\bfn (gl[A-Z][A-Za-z0-9]*)\s*\(", text))
        self.assertIn("glClear", declared)
        self.assertEqual(sorted(declared - checker.GLES2_CORE), [])

    def test_gles2_core_names_are_plain_gl_functions(self):
        self.assertTrue(all(re.fullmatch(r"gl[A-Z][A-Za-z0-9]*", n) for n in checker.GLES2_CORE))
        self.assertNotIn("glBindVertexArray", checker.GLES2_CORE)

    def test_glibc_private_is_refused_whatever_its_name(self):
        syms = READELF_SYMS + "     8: 0000000000000000     0 FUNC    GLOBAL DEFAULT  UND __libc_enable_secure@GLIBC_PRIVATE (6)\n"
        problems = verify(readelf_syms=syms)
        self.assertTrue(any("__libc_enable_secure@GLIBC_PRIVATE" in p for p in problems), problems)

    def test_static_tls_is_refused(self):
        problems = verify(static_tls=True)
        self.assertTrue(any("static TLS" in p for p in problems), problems)
        both = verify(static_tls=True, text_relocations=True)
        self.assertTrue(any("static TLS" in p for p in both) and any("text relocations" in p for p in both), both)

    def test_the_default_ceiling_is_the_emulator_images_glibc(self):
        self.assertEqual(checker.DEFAULT_MAX_GLIBC, "2.40")
        at = READELF_SYMS + "     8: 0000000000000000     0 FUNC    GLOBAL DEFAULT  UND memfd_create@GLIBC_2.40 (5)\n"
        self.assertEqual(verify(readelf_syms=at), [])
        over = READELF_SYMS + "     8: 0000000000000000     0 FUNC    GLOBAL DEFAULT  UND memfd_create@GLIBC_2.41 (5)\n"
        self.assertTrue(any("GLIBC_2.41 exceeds" in p for p in verify(readelf_syms=over)))


if __name__ == "__main__":
    unittest.main()
