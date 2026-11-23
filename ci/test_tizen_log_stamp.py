#!/usr/bin/env python3
"""The standalone Samsung nightly log tool stamps a copy without altering other entries."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
import zipfile


ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, ROOT / path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


receiver = load("tizen_log_receiver", "tools/tizen-log-receiver.py")
packager = load("tizen_package", "tizen/package.py")


class Stamp(unittest.TestCase):
    def test_stamps_a_copy_and_refuses_clobbering(self):
        with tempfile.TemporaryDirectory() as scratch:
            source = Path(scratch) / "original.tpk"
            stamped = Path(scratch) / "stamped.tpk"
            contents = {"tizen-manifest.xml": b"<manifest/>", "lib/libplx.so": b"\0\xff",
                        "author-signature.xml": b"author", "signature1.xml": b"dist"}
            with zipfile.ZipFile(source, "w") as package:
                for name, data in contents.items():
                    package.writestr(name, data)
            original = source.read_bytes()
            receiver.stamp(source, stamped, "10.0.2.2:5514")
            with zipfile.ZipFile(stamped) as package:
                self.assertEqual(set(package.namelist()), set(contents) | {receiver.LOG_ENTRY})
                self.assertEqual(package.read(receiver.LOG_ENTRY), b"10.0.2.2:5514\n")
                for name, data in contents.items():
                    self.assertEqual(package.read(name), data)
            with self.assertRaises(FileExistsError):
                receiver.stamp(source, stamped, "10.0.2.2:9999")
            with self.assertRaisesRegex(ValueError, "already carries"):
                receiver.stamp(stamped, Path(scratch) / "second.tpk", "10.0.2.2:9999")
            with self.assertRaisesRegex(ValueError, "different"):
                receiver.stamp(source, source, "10.0.2.2:9999")
            second = Path(scratch) / "second.tpk"
            receiver.stamp(stamped, second, "10.0.2.2:9999", replace=True)
            with zipfile.ZipFile(second) as package:
                self.assertEqual(package.read(receiver.LOG_ENTRY), b"10.0.2.2:9999\n")
            self.assertEqual(source.read_bytes(), original)

    def test_requires_a_manifest(self):
        with tempfile.TemporaryDirectory() as scratch:
            source = Path(scratch) / "something.tpk"
            with zipfile.ZipFile(source, "w"):
                pass
            with self.assertRaisesRegex(ValueError, "manifest"):
                receiver.stamp(source, source.with_name("stamped.tpk"), "10.0.2.2:9999")

    def test_target_validation_agrees_with_the_packager(self):
        for addr, port in (("127.0.0.1", 1), ("192.0.2.1", 65535), ("10.0.2.2", 9999)):
            self.assertEqual(receiver.log_target(addr, port), packager.log_host(f"{addr}:{port}"))
        for addr, port in (("example.com", 42), ("::1", 42), ("01.0.0.1", 42),
                           ("10.0.2.256", 42), ("10.0.2.2", 0), ("10.0.2.2", 65536)):
            with self.assertRaises(ValueError):
                receiver.log_target(addr, port)
            with self.assertRaises(packager.PackageError):
                packager.log_host(f"{addr}:{port}")


if __name__ == "__main__":
    unittest.main()
