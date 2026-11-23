#!/usr/bin/env python3
"""Unit tests of tizen/package.py: input validation, the staging contract, the signing refusal and the package check.

Nothing here runs dotnet or needs a certificate: `stage` and `verify_tpk` work on a throwaway tree of fake files, and the signing refusal is
the part a CI machine can prove. The refusal must NAME the missing variables and must never print a value.
"""
import importlib.util
import base64
import json
import re
import tempfile
import unittest
import unittest.mock
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("tizen_package", HERE.parent / "tizen" / "package.py")
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)

rspec = importlib.util.spec_from_file_location("tizen_log_receiver", HERE.parent / "tools" / "tizen-log-receiver.py")
receiver = importlib.util.module_from_spec(rspec)
rspec.loader.exec_module(receiver)

SECRET = "hunter2-do-not-print"


def fake_root(tmp: Path) -> Path:
    root = tmp / "root"
    for source, _ in package.PAYLOAD:
        (root / source).parent.mkdir(parents=True, exist_ok=True)
        (root / source).write_bytes(b"payload " + source.encode())
    (root / package.HOST_DIR).mkdir(parents=True)
    for name in (package.NOTICES_FILE, *package.LICENSE_DOCS, "licenses/Zlib.txt", "licenses/FreeType.txt"):
        (root / name).parent.mkdir(parents=True, exist_ok=True)
        (root / name).write_bytes(b"notice " + name.encode())
    return root


class Inputs(unittest.TestCase):
    def test_log_host_is_an_ipv4_literal_and_port(self):
        self.assertEqual(package.log_host("10.0.2.2:9999"), "10.0.2.2:9999")
        for bad in ("localhost:9999", "10.0.2.2", "10.0.2.256:1", "10.0.2.2:0", "10.0.2.2:70000", "http://10.0.2.2:9", "10.0.2.2:9 "):
            with self.assertRaises(package.PackageError, msg=bad):
                package.log_host(bad)

    def test_mock_host_is_host_and_port_never_a_url(self):
        self.assertEqual(package.mock_host("10.0.2.2:32499"), "10.0.2.2:32499")
        self.assertEqual(package.mock_host("mock.lan:8080"), "mock.lan:8080")
        for bad in ("http://10.0.2.2:1", "10.0.2.2", "10.0.2.2:99999", "300.0.0.1:5", "-bad.host:5", "a..b:5", "10.0.2.2:5/x"):
            with self.assertRaises(package.PackageError, msg=bad):
                package.mock_host(bad)

    def test_dev_boot_carries_a_placeholder_only(self):
        text = package.dev_boot_json("10.0.2.2:32499")
        parsed = json.loads(text)
        self.assertEqual((parsed["pms_host"], parsed["pms_port"]), ("10.0.2.2", 32499))
        self.assertEqual(parsed["dev"], {"token": package.DEV_BOOT_TOKEN, "pms_origin": "http://10.0.2.2:32499"})
        self.assertIn("placeholder", package.DEV_BOOT_TOKEN)

    def test_msbuild_escape_percent_first(self):
        self.assertEqual(package.msbuild_escape("a%41;b@(x)"), "a%2541%3Bb%40%28x%29")


class Signing(unittest.TestCase):
    def test_refusal_names_the_missing_variables_and_no_value(self):
        env = {"PLX_TIZEN_AUTHOR_PASS": SECRET, "PLX_TIZEN_DIST_P12": "/nonexistent/dist.p12"}
        with self.assertRaises(package.PackageError) as caught:
            package.check_signing(env)
        text = str(caught.exception)
        self.assertIn("PLX_TIZEN_AUTHOR_P12", text)
        self.assertIn("PLX_TIZEN_DIST_PASS", text)
        self.assertNotIn(SECRET, text)
        self.assertNotIn("/nonexistent", text)

    def test_nothing_set_refuses_all_four(self):
        self.assertEqual(package.missing_signing({}), list(package.SIGNING_VARS))

    def test_a_path_that_is_no_file_is_named_without_its_value(self):
        env = {name: "/nonexistent/x" for name in package.SIGNING_VARS}
        with self.assertRaises(package.PackageError) as caught:
            package.check_signing(env)
        self.assertIn("PLX_TIZEN_AUTHOR_P12 names no file", str(caught.exception))
        self.assertNotIn("/nonexistent", str(caught.exception))

    def test_complete_inputs_resolve_paths(self):
        with tempfile.TemporaryDirectory() as tmp:
            a, d = Path(tmp) / "a.p12", Path(tmp) / "d.p12"
            a.write_bytes(b"x")
            d.write_bytes(b"x")
            env = {"PLX_TIZEN_AUTHOR_P12": str(a), "PLX_TIZEN_AUTHOR_PASS": "p", "PLX_TIZEN_DIST_P12": str(d), "PLX_TIZEN_DIST_PASS": "q"}
            self.assertEqual(package.check_signing(env), (a.resolve(), d.resolve()))

    def test_the_tpk_build_refuses_before_staging_anything(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = fake_root(Path(tmp))
            so = Path(tmp) / "libplx.so"
            so.write_bytes(b"elf")
            with self.assertRaises(package.PackageError):
                package.build_tpk(root, so, None, None, {})
            self.assertFalse((root / package.HOST_DIR / "lib").exists())


class DotnetBuild(unittest.TestCase):
    """`make PLATFORM=tizen` puts PLATFORM=tizen in every recipe's environment, and MSBuild reads the environment as properties,
    case-insensitively: left in, the build writes bin/tizen/Release/... and the glob for bin/Release/... finds no package."""

    def build(self, env_extra):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            root = fake_root(tmp)
            so = tmp / "x86_64" / "libplx.so"
            so.parent.mkdir()
            so.write_bytes(b"elf")
            author, dist = tmp / "a.p12", tmp / "d.p12"
            author.write_bytes(b"x")
            dist.write_bytes(b"x")
            env = {"PATH": "/usr/bin", "PLX_TIZEN_AUTHOR_P12": str(author), "PLX_TIZEN_AUTHOR_PASS": "p",
                   "PLX_TIZEN_DIST_P12": str(dist), "PLX_TIZEN_DIST_PASS": "q"}
            env.update(env_extra)
            seen = {}

            def fake_run(command, cwd=None, env=None, **kwargs):
                seen["command"], seen["env"] = list(command), dict(env)
                out = Path(cwd) / "bin/Release" / package.TFM
                out.mkdir(parents=True)
                with zipfile.ZipFile(out / "plxntizen0-0.1.0.tpk", "w") as z:
                    for name in package.REQUIRED_IN_TPK:
                        z.writestr(name, b"elf" if name == "lib/libplx.so" else "x")
                return unittest.mock.Mock(returncode=0)

            with unittest.mock.patch.object(package.subprocess, "run", fake_run), \
                    unittest.mock.patch.object(package.shutil, "which", lambda *a, **k: "/usr/bin/dotnet"):
                package.build_tpk(root, so, None, None, env)
            return seen

    def test_the_child_env_has_no_platform_and_the_command_pins_anycpu(self):
        seen = self.build({"PLATFORM": "tizen", "Platform": "tizen"})
        self.assertEqual([k for k in seen["env"] if k.upper() == "PLATFORM"], [], "PLATFORM reached MSBuild as a property")
        self.assertIn("-p:Platform=AnyCPU", seen["command"])

    def test_the_signing_values_still_reach_the_child_only_as_properties(self):
        seen = self.build({"PLATFORM": "tizen"})
        self.assertEqual(seen["env"]["AuthorPass"], "p")
        self.assertFalse(any("p" == a or "q" == a for a in seen["command"]), "a password is on the command line")


class Nightly(unittest.TestCase):
    label = "0.11.0-nightly-20261122"

    def test_versions_advance_and_inputs_are_validated(self):
        self.assertEqual(package.nightly_version(self.label), "0.11.9822")
        self.assertEqual(package.nightly_version("0.11.0-nightly-20261123"), "0.11.9823")
        self.assertEqual(package.nightly_version("255.255.0-nightly-20261122"), "255.255.9822")
        for label in ("0.11.0-nightly-20260230", "0.11.0-nightly-19991231", "1.2.3",
                      "256.2.3-nightly-20261122", "1.256.3-nightly-20261122"):
            with self.assertRaises(package.PackageError):
                package.nightly_version(label)
        for mock, log, env in (("mock", None, {}), (None, "log", {}), (None, None, {"PLX_TIZEN_AUTHOR_PASS": ""})):
            with self.assertRaises(package.PackageError):
                package.check_nightly(self.label, mock, log, env)

    def test_verify_requires_both_ephemeral_certificates(self):
        version = package.nightly_version(self.label)
        with tempfile.TemporaryDirectory() as scratch:
            tpk = Path(scratch) / "package.tpk"
            with zipfile.ZipFile(tpk, "w") as archive:
                archive.writestr(package.MANIFEST,
                    f'<manifest xmlns="http://tizen.org/ns/packages" package="{package.PACKAGE_ID}" '
                    f'version="{version}" api-version="6.0"><ui-application><label>{package.NIGHTLY_LABEL}</label>'
                    '</ui-application></manifest>')
                for name, cert in (("author-signature.xml", b"author"), ("signature1.xml", b"dist")):
                    archive.writestr(name, '<Signature xmlns="http://www.w3.org/2000/09/xmldsig#"><KeyInfo><X509Data>'
                                     f'<X509Certificate>{base64.b64encode(cert).decode()}</X509Certificate></X509Data></KeyInfo></Signature>')
            package.verify_nightly(tpk, version, b"author", b"dist")
            with self.assertRaisesRegex(package.PackageError, "author-signature.xml"):
                package.verify_nightly(tpk, version, b"sdk-default", b"dist")
            with self.assertRaisesRegex(package.PackageError, "signature1.xml"):
                package.verify_nightly(tpk, version, b"author", b"sdk-default")

    def test_throwaway_key_uses_a_tizen_sdk_compatible_envelope(self):
        with tempfile.TemporaryDirectory() as scratch, \
                unittest.mock.patch.object(package.subprocess, "run") as run, \
                unittest.mock.patch.object(package.subprocess, "check_output", return_value=b"cert"):
            package.throwaway_certificate(Path(scratch), "author", SECRET)
        command = run.call_args.args[0]
        for flag in ("-keypbe", "-certpbe"):
            self.assertEqual(command[command.index(flag) + 1], "PBE-SHA1-3DES")
        self.assertEqual(command[command.index("-macalg") + 1], "sha1")
        self.assertNotIn(SECRET, command)
        self.assertEqual(run.call_args.kwargs["env"]["PLX_EPHEMERAL_PASSWORD"], SECRET)


class HostProject(unittest.TestCase):
    def test_every_package_reference_is_an_exact_version(self):
        # `8.0.*`, `[8.0,)` or a missing version would let a restore on a later day build a different host than the one the emulator ran
        text = (HERE.parent / "tizen" / "host" / "PlxHost" / "PlxHost.csproj").read_text(encoding="utf-8")
        refs = re.findall(r'<PackageReference\s+Include="([^"]+)"([^>]*)>', text)
        self.assertEqual(sorted(name for name, _ in refs), ["Tizen.NET", "Tizen.NET.Sdk"])
        for name, rest in refs:
            version = re.search(r'Version="([^"]*)"', rest)
            self.assertIsNotNone(version, f"{name} has no Version attribute")
            self.assertRegex(version.group(1), r"^[0-9]+(\.[0-9]+){2,3}$", f"{name} is not an exact version")
        pinned = dict((name, re.search(r'Version="([^"]*)"', rest).group(1)) for name, rest in refs)
        self.assertEqual(pinned, {"Tizen.NET": "8.0.0.15631", "Tizen.NET.Sdk": "1.1.9"})


class Staging(unittest.TestCase):
    def test_stage_writes_the_payload_and_the_asked_for_files_only(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = fake_root(Path(tmp))
            so = Path(tmp) / "libplx.so"
            so.write_bytes(b"elf")
            host = package.stage(root, so, None, None)
            self.assertEqual((host / "lib/libplx.so").read_bytes(), b"elf")
            self.assertTrue((host / "res/appfont.ttf").is_file())
            self.assertFalse((host / "res/dev-boot.json").exists())
            self.assertFalse((host / "res/log-host.txt").exists())
            host = package.stage(root, so, "10.0.2.2:32499", "10.0.2.2:9999")
            self.assertTrue((host / "res/dev-boot.json").is_file())
            self.assertEqual((host / "res/log-host.txt").read_text(), "10.0.2.2:9999\n")
            host = package.stage(root, so, None, None)  # a stale dev-boot of the last staging must not ride along
            self.assertFalse((host / "res/dev-boot.json").exists())
            self.assertFalse((host / "res/log-host.txt").exists())

    def test_the_notices_and_licence_texts_are_staged_next_to_one_another(self):
        # THIRD-PARTY-NOTICES.md names its texts as `licenses/<name>`; the package must hold both, the FreeType credit and zlib among them
        with tempfile.TemporaryDirectory() as tmp:
            root = fake_root(Path(tmp))
            so = Path(tmp) / "libplx.so"
            so.write_bytes(b"elf")
            host = package.stage(root, so, None, None)
            self.assertEqual((host / "res/THIRD-PARTY-NOTICES.md").read_bytes(), b"notice THIRD-PARTY-NOTICES.md")
            for name in ("LICENSE", "LICENSING.md", "TRADEMARKS.md", "Zlib.txt", "FreeType.txt"):
                self.assertTrue((host / "res/licenses" / name).is_file(), name)
            (root / "licenses/Zlib.txt").unlink()  # a text that went away must not ride along from the last staging
            (root / "licenses/Zlib.txt").parent.joinpath("Extra.txt").write_bytes(b"x")
            host = package.stage(root, so, None, None)
            self.assertFalse((host / "res/licenses/Zlib.txt").exists())
            self.assertTrue((host / "res/licenses/Extra.txt").is_file())

    def test_a_checkout_without_the_notices_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = fake_root(Path(tmp))
            so = Path(tmp) / "libplx.so"
            so.write_bytes(b"elf")
            (root / package.NOTICES_FILE).unlink()
            with self.assertRaisesRegex(package.PackageError, "THIRD-PARTY-NOTICES.md is missing"):
                package.stage(root, so, None, None)

    def test_the_real_checkout_has_every_file_the_staging_copies(self):
        root = HERE.parent
        for name in (package.NOTICES_FILE, *package.LICENSE_DOCS, "licenses/Zlib.txt", "licenses/FreeType.txt"):
            self.assertTrue((root / name).is_file(), name)

    def test_stage_refuses_a_missing_library(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(package.PackageError):
                package.stage(fake_root(Path(tmp)), Path(tmp) / "none.so", None, None)


class PackageCheck(unittest.TestCase):
    def make(self, tmp: Path, names, so_bytes=b"elf") -> tuple[Path, Path]:
        so = tmp / "libplx.so"
        so.write_bytes(b"elf")
        tpk = tmp / "p.tpk"
        with zipfile.ZipFile(tpk, "w") as z:
            for name in names:
                z.writestr(name, so_bytes if name == "lib/libplx.so" else "x")
        return tpk, so

    def test_complete_package_passes(self):
        with tempfile.TemporaryDirectory() as tmp:
            tpk, so = self.make(Path(tmp), package.REQUIRED_IN_TPK)
            package.verify_tpk(tpk, so, None, None)

    def test_a_package_without_the_notices_is_refused(self):
        for gone in ("res/THIRD-PARTY-NOTICES.md", "res/licenses/FreeType.txt", "res/licenses/Zlib.txt"):
            with tempfile.TemporaryDirectory() as tmp:
                tpk, so = self.make(Path(tmp), [n for n in package.REQUIRED_IN_TPK if n != gone])
                with self.assertRaisesRegex(package.PackageError, re.escape(gone)):
                    package.verify_tpk(tpk, so, None, None)

    def test_the_exact_bin_and_lib_set_passes(self):
        with tempfile.TemporaryDirectory() as tmp:
            tpk, so = self.make(Path(tmp), (*package.REQUIRED_IN_TPK, "bin/PlxHost.pdb", "PlxHost.deps.json", "bin/", "lib/"))
            package.verify_tpk(tpk, so, None, None)

    def test_an_extra_assembly_is_refused_until_it_has_a_notices_entry(self):
        with tempfile.TemporaryDirectory() as tmp:
            tpk, so = self.make(Path(tmp), (*package.REQUIRED_IN_TPK, "bin/Foo.dll"))
            with self.assertRaisesRegex(package.PackageError, r"bin/Foo\.dll.*section 2\.8"):
                package.verify_tpk(tpk, so, None, None)

    def test_an_extra_native_library_is_refused_until_it_has_a_notices_entry(self):
        with tempfile.TemporaryDirectory() as tmp:
            tpk, so = self.make(Path(tmp), (*package.REQUIRED_IN_TPK, "lib/libfoo.so"))
            with self.assertRaisesRegex(package.PackageError, r"lib/libfoo\.so.*section 2\.8"):
                package.verify_tpk(tpk, so, None, None)

    def test_missing_member_is_named(self):
        with tempfile.TemporaryDirectory() as tmp:
            tpk, so = self.make(Path(tmp), [n for n in package.REQUIRED_IN_TPK if n != "res/OFL.txt"])
            with self.assertRaisesRegex(package.PackageError, "OFL.txt"):
                package.verify_tpk(tpk, so, None, None)

    def test_a_stale_dev_boot_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            tpk, so = self.make(Path(tmp), (*package.REQUIRED_IN_TPK, "res/dev-boot.json"))
            with self.assertRaisesRegex(package.PackageError, "carries res/dev-boot.json"):
                package.verify_tpk(tpk, so, None, None)

    def test_a_requested_log_file_must_be_there(self):
        with tempfile.TemporaryDirectory() as tmp:
            tpk, so = self.make(Path(tmp), package.REQUIRED_IN_TPK)
            with self.assertRaisesRegex(package.PackageError, "lacks res/log-host.txt"):
                package.verify_tpk(tpk, so, None, "10.0.2.2:9")

    def test_packaged_library_must_be_the_verified_one(self):
        with tempfile.TemporaryDirectory() as tmp:
            tpk, so = self.make(Path(tmp), package.REQUIRED_IN_TPK, so_bytes=b"other")
            with self.assertRaisesRegex(package.PackageError, "differs"):
                package.verify_tpk(tpk, so, None, None)


class LogReceiver(unittest.TestCase):
    """tools/tizen-log-receiver.py's sequence bookkeeping: a gap is the only evidence a UDP log is incomplete."""

    def test_in_order_lines_report_nothing(self):
        tracker = receiver.Tracker()
        self.assertEqual([tracker.note("a", f"{n} line") for n in range(3)], [None, None, None])
        self.assertEqual(tracker.lost, 0)

    def test_a_gap_counts_the_lost_datagrams(self):
        tracker = receiver.Tracker()
        tracker.note("a", "0 x")
        report = tracker.note("a", "4 x")
        self.assertIn("3 datagram(s) lost", report)
        self.assertEqual(tracker.lost, 3)

    def test_the_host_stream_and_the_library_stream_count_apart(self):
        tracker = receiver.Tracker()
        self.assertIsNone(tracker.note("a", "0 lib"))
        self.assertIsNone(tracker.note("a", "h0 host"))
        self.assertIsNone(tracker.note("a", "h1 host"))
        self.assertIsNone(tracker.note("a", "1 lib"))

    def test_a_restart_is_not_a_loss(self):
        tracker = receiver.Tracker()
        tracker.note("a", "0 x")
        tracker.note("a", "1 x")
        self.assertIn("restarted", tracker.note("a", "0 x"))
        self.assertEqual((tracker.lost, tracker.restarts), (0, 1))


if __name__ == "__main__":
    unittest.main()
