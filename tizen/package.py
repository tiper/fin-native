#!/usr/bin/env python3
"""Stage and package the Tizen build (`make PLATFORM=tizen tizen-stage | tizen-signing | tizen-tpk`; docs/tizen.md).

    python3 tizen/package.py stage         --so PATH [--mock-host HOST:PORT] [--log-host IPV4:PORT]
    python3 tizen/package.py check-signing
    python3 tizen/package.py tpk           --so PATH [--mock-host HOST:PORT] [--log-host IPV4:PORT]

`stage` fills the .NET host project (tizen/host/PlxHost/) with everything the package carries that is not source: the library, the
font and its licence, the icon, and the two dev-boot inputs. A dev-boot input that was not asked for is NOT written and a stale copy
from an earlier run is removed, so the package is exactly what the command line named (the log channel is off and there is no dev
boot unless the builder asked). `tpk` stages, builds with `dotnet`, signs, and checks the package it got.

THE SIGNING INPUTS are environment variables, read here and nowhere else:

    PLX_TIZEN_AUTHOR_P12   path of the author certificate (.p12)        PLX_TIZEN_AUTHOR_PASS   its password
    PLX_TIZEN_DIST_P12     path of the distributor certificate (.p12)   PLX_TIZEN_DIST_PASS     its password

A Samsung television or emulator installs only a package signed with a certificate profile (an author certificate and a
distributor one bound to the device); the SDK's bundled default certificates are expired and are never used. The passwords never
appear on a command line (make would echo one, `ps` would show one): they go to MSBuild as environment properties of this one
child process, escaped for MSBuild, and are never printed. No certificate or password is ever committed (.gitignore: *.p12, *.pfx).
Without all four the command stops BEFORE anything is built and names the missing variables, never a value.

Standard library only; Python 3.10 or newer.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HOST_DIR = Path("tizen/host/PlxHost")
PROJECT = "PlxHost.csproj"
# The package id of the emulator package (a Tizen package id is ten alphanumerics; the application id is the same string): a neutral
# name chosen for the emulator MVP. A retail package id is assigned by Samsung's seller process, not chosen here.
PACKAGE_ID = "plxntizen0"
TFM = "tizen80"

SIGNING_VARS = ("PLX_TIZEN_AUTHOR_P12", "PLX_TIZEN_AUTHOR_PASS", "PLX_TIZEN_DIST_P12", "PLX_TIZEN_DIST_PASS")

# The placeholder the dev boot signs in with: the mock Plex server (tests/mock_pms.py) "accepts, never checks" the token, and nothing
# real is ever put here. A real credential in this file would be published with the package.
DEV_BOOT_TOKEN = "plxnative-emulator-placeholder-token"

PAYLOAD = (
    ("pkg/appfont.ttf", "res/appfont.ttf"),
    ("pkg/appfont-bold.ttf", "res/appfont-bold.ttf"),
    ("pkg/OFL.txt", "res/OFL.txt"),
    ("pkg/dev/largeIcon.png", f"shared/res/{PACKAGE_ID}.png"),
)
# The notices travel WITH the package, as they do in the .ipk (Makefile, APP_FILES and LICENSE_FILES): this package statically links SDL2 and
# SDL2_ttf (zlib) and FreeType (the FreeType License, which requires the credit to be shown in the documentation shipped with the product),
# so a copy in the repository does not discharge it for someone who received only the .tpk. THIRD-PARTY-NOTICES.md names its licence texts
# as `licenses/<name>`, so they are staged next to it: res/THIRD-PARTY-NOTICES.md and res/licenses/ (the project's own LICENSE, LICENSING.md
# and TRADEMARKS.md and every licenses/*.txt, the set the .ipk's `licenses/` holds).
NOTICES_FILE = "THIRD-PARTY-NOTICES.md"
LICENSE_DOCS = ("LICENSE", "LICENSING.md", "TRADEMARKS.md")
# The generated, gitignored files of the host project, removed before every staging.
GENERATED = ("lib", "bin", "obj", "res/dev-boot.json", "res/log-host.txt", "res/appfont.ttf", "res/appfont-bold.ttf", "res/OFL.txt",
             f"res/{NOTICES_FILE}", "res/licenses", f"shared/res/{PACKAGE_ID}.png")
# What bin/ and lib/ may hold: nothing but our own assembly (and its symbols) and the library. The four packages PlxHost.deps.json lists carry no
# runtime or native assets into the package: Tizen.NET and Tizen.NET.API8 are referenced and provided by the device's runtime,
# Tizen.NET.Sdk is build-time only, and Microsoft.NETCore.Platforms is a metadata package with no assemblies. THIRD-PARTY-NOTICES.md section 2.8
# says so: a new assembly or native library needs an entry there FIRST.
ALLOWED_BIN = ("bin/PlxHost.dll", "bin/PlxHost.pdb")
ALLOWED_LIB = ("lib/libplx.so",)
REQUIRED_IN_TPK = ("tizen-manifest.xml", "bin/PlxHost.dll", "lib/libplx.so", "res/appfont.ttf", "res/OFL.txt",
                   f"res/{NOTICES_FILE}", "res/licenses/LICENSE", "res/licenses/Zlib.txt", "res/licenses/FreeType.txt",
                   "author-signature.xml", "signature1.xml")


class PackageError(Exception):
    """A refusal with the reason, printed without a traceback."""


def log_host(value: str) -> str:
    """`IPV4:PORT`: the UDP log channel takes an IPv4 literal only (a blocking name lookup has no place in a logger)."""
    match = re.fullmatch(r"((?:\d{1,3}\.){3}\d{1,3}):(\d{1,5})", value)
    if not match or any(int(octet) > 255 for octet in match.group(1).split(".")) or not 1 <= int(match.group(2)) <= 65535:
        raise PackageError(f"TIZEN_LOG_HOST / --log-host must be IPV4:PORT, e.g. 10.0.2.2:9999 (got: {value})")
    return f"{match.group(1)}:{int(match.group(2))}"


def mock_host(value: str) -> str:
    """`HOST:PORT`, HOST an IPv4 literal or a DNS name, never a URL: the dev boot builds `http://HOST:PORT` itself."""
    match = re.fullmatch(r"([A-Za-z0-9.-]+):(\d{1,5})", value)
    if not match or not 1 <= int(match.group(2)) <= 65535:
        raise PackageError(f"TIZEN_MOCK_HOST / --mock-host must be HOST:PORT, e.g. 10.0.2.2:32499 (no http://, no path; got: {value})")
    host = match.group(1)
    if re.fullmatch(r"[0-9.]+", host):
        if not re.fullmatch(r"(?:\d{1,3}\.){3}\d{1,3}", host) or any(int(octet) > 255 for octet in host.split(".")):
            raise PackageError(f"--mock-host {host} is not an IPv4 address (four numbers 0-255)")
    elif not re.fullmatch(r"[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?(?:\.[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?)*", host):
        raise PackageError(f"--mock-host {host} is not a host name (letters, digits, '-' and '.', no empty label)")
    return f"{host}:{int(match.group(2))}"


def dev_boot_json(mock: str) -> str:
    """The dev-boot file's text: the members of `plx_host_start`'s `launch_json` that name the server and the placeholder identity
    (rust-modules/platform-tizen/include/plx_host.h, "LAUNCH JSON": `pms_host`, `pms_port` and the dev-only `dev` object). The host adds its own
    members (the directories, the locale, the log address) and splices these in, so the file is one JSON object and nothing else."""
    host, _, port = mock_host(mock).rpartition(":")
    return json.dumps({"pms_host": host, "pms_port": int(port),
                       "dev": {"token": DEV_BOOT_TOKEN, "pms_origin": f"http://{host}:{port}"}}, indent=2) + "\n"


def missing_signing(env: dict[str, str]) -> list[str]:
    return [name for name in SIGNING_VARS if not env.get(name)]


def check_signing(env: dict[str, str]) -> tuple[Path, Path]:
    """The two certificate paths, or a PackageError that names the missing variables and prints no value."""
    missing = missing_signing(env)
    if missing:
        raise PackageError(
            "refusing to build the .tpk: no signing material. Missing: " + " ".join(missing) + "\n"
            "  A Tizen television or emulator installs only a package signed with a Samsung certificate profile (an author\n"
            "  certificate and a distributor one bound to the device); the SDK's bundled default certificates are expired and are\n"
            "  never used. Set the four variables (the passwords only in the environment, never on a command line) and run again:\n"
            "    PLX_TIZEN_AUTHOR_P12=/path/author.p12 PLX_TIZEN_AUTHOR_PASS=...\n"
            "    PLX_TIZEN_DIST_P12=/path/distributor.p12 PLX_TIZEN_DIST_PASS=...   make PLATFORM=tizen tizen-tpk\n"
            "  (tizen-so and tizen-verify need none of this.)")
    paths = []
    for name in ("PLX_TIZEN_AUTHOR_P12", "PLX_TIZEN_DIST_P12"):
        path = Path(env[name]).expanduser()
        if not path.is_file():
            raise PackageError(f"{name} names no file")
        paths.append(path.resolve())
    return paths[0], paths[1]


def msbuild_escape(value: str) -> str:
    """MSBuild's own escaping, `%` first: left raw, a `%41` or `@(x)` in a password is decoded or expanded on the way to the Sign task."""
    for char, code in (("%", "%25"), ("*", "%2A"), ("?", "%3F"), ("@", "%40"), ("$", "%24"), ("(", "%28"), (")", "%29"), (";", "%3B"), ("'", "%27")):
        value = value.replace(char, code)
    return value


def stage_notices(root: Path, host: Path) -> None:
    """Copy the notices and every licence text into the host project's `res/` (see NOTICES_FILE)."""
    sources = [(root / NOTICES_FILE, host / "res" / NOTICES_FILE)]
    sources += [(root / name, host / "res/licenses" / name) for name in LICENSE_DOCS]
    sources += [(path, host / "res/licenses" / path.name) for path in sorted((root / "licenses").glob("*.txt"))]
    for src, target in sources:
        if not src.is_file():
            raise PackageError(f"{src.relative_to(root)} is missing from the checkout")
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(src, target)


def stage(root: Path, so: Path, mock: str | None, log: str | None) -> Path:
    """Fill the host project. Returns the project directory."""
    host = root / HOST_DIR
    if not so.is_file():
        raise PackageError(f"{so} is missing; build it first: make PLATFORM=tizen tizen-so tizen-verify")
    for name in GENERATED:
        target = host / name
        if target.is_dir() and not target.is_symlink():
            shutil.rmtree(target)
        elif target.exists() or target.is_symlink():
            target.unlink()
    (host / "lib").mkdir(parents=True)
    shutil.copyfile(so, host / "lib/libplx.so")
    for source, target in PAYLOAD:
        src = root / source
        if not src.is_file():
            raise PackageError(f"{source} is missing from the checkout")
        (host / target).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(src, host / target)
    stage_notices(root, host)
    if mock is not None:
        (host / "res/dev-boot.json").write_text(dev_boot_json(mock_host(mock)))
        print(f"dev boot: the mock Plex server http://{mock_host(mock)} (res/dev-boot.json; a placeholder token, never a credential)")
    else:
        print("dev boot: none (no TIZEN_MOCK_HOST): res/dev-boot.json is not written")
    if log is not None:
        (host / "res/log-host.txt").write_text(log_host(log) + "\n")
        print(f"log channel: UDP to {log_host(log)} (res/log-host.txt)")
    else:
        print("log channel: off (no TIZEN_LOG_HOST): res/log-host.txt is not written")
    return host


def build_tpk(root: Path, so: Path, mock: str | None, log: str | None, env: dict[str, str]) -> Path:
    author, dist = check_signing(env)
    host = stage(root, so, mock, log)
    dotnet_root = Path(env.get("DOTNET_ROOT") or Path.home() / ".dotnet")
    # GNU make exports a command-line `PLATFORM=tizen` into every recipe, and MSBuild reads the environment as properties, case-insensitively:
    # left in, it builds as Platform=tizen and writes bin/tizen/Release/ and obj/tizen/, where the glob below finds nothing. Drop it, and pin
    # the property on the command (a global property outranks the environment).
    build_env = {k: v for k, v in env.items() if k.upper() != "PLATFORM"}
    build_env.update({"DOTNET_ROOT": str(dotnet_root), "PATH": f"{dotnet_root}{os.pathsep}{env.get('PATH', '')}",
                      "DOTNET_CLI_TELEMETRY_OPTOUT": "1", "DOTNET_NOLOGO": "1",
                      # MSBuild environment properties scoped to this one child: the SDK's TizenPrepareCertificate falls back to its bundled
                      # (expired) certificates only when these paths are EMPTY, which check_signing has ruled out.
                      "AuthorPath": msbuild_escape(str(author)), "AuthorPass": msbuild_escape(env["PLX_TIZEN_AUTHOR_PASS"]),
                      "DistributorPath": msbuild_escape(str(dist)), "DistributorPass": msbuild_escape(env["PLX_TIZEN_DIST_PASS"])})
    if shutil.which("dotnet", path=build_env["PATH"]) is None:
        raise PackageError("no dotnet: install a user-local .NET 8 SDK (dotnet-install.sh --channel 8.0 --install-dir ~/.dotnet), "
                           "set DOTNET_ROOT if elsewhere, and the Tizen.NET.Sdk package restores from NuGet (docs/tizen.md, Toolchain)")
    # No build servers: a long-lived msbuild/compiler process would keep the environment (and the passwords in it) alive.
    command = ["dotnet", "build", PROJECT, "-c", "Release", "-nologo", "-nodeReuse:false", "-p:UseSharedCompilation=false", "-p:Platform=AnyCPU"]
    print("dotnet build -c Release (Tizen.NET.Sdk, " + TFM + "; signed with the supplied certificates)", flush=True)
    result = subprocess.run(command, cwd=host, env=build_env)
    if result.returncode != 0:
        raise PackageError(f"dotnet build failed (exit {result.returncode})")
    built = sorted((host / "bin/Release" / TFM).glob("*.tpk"))
    if not built:
        raise PackageError("dotnet produced no .tpk")
    out = root / "tizen/out"
    out.mkdir(parents=True, exist_ok=True)
    tpk = out / f"PlxNative-{so.parent.name}.tpk"
    shutil.copyfile(built[0], tpk)
    verify_tpk(tpk, so, mock, log)
    return tpk


def verify_tpk(tpk: Path, so: Path, mock: str | None, log: str | None) -> None:
    """The package holds what it must, the dev-boot files exactly when asked for, and the library that was verified."""
    with zipfile.ZipFile(tpk) as package:
        names = set(package.namelist())
        for need in REQUIRED_IN_TPK:
            if need not in names:
                raise PackageError(f"{tpk.name} lacks {need}")
        for name, want in (("res/dev-boot.json", mock), ("res/log-host.txt", log)):
            if (want is not None) != (name in names):
                raise PackageError(f"{tpk.name} {'lacks' if want is not None else 'carries'} {name} "
                                   f"{'although it was asked for' if want is not None else 'although it was not asked for (stale staging?)'}")
        for prefix, allowed in (("bin/", ALLOWED_BIN), ("lib/", ALLOWED_LIB)):
            extra = sorted(n for n in names if n.startswith(prefix) and not n.endswith("/") and n not in allowed)
            if extra:
                raise PackageError(f"{tpk.name} carries {', '.join(extra)} besides {', '.join(allowed)}: a new assembly or native library "
                                   f"needs a THIRD-PARTY-NOTICES.md section 2.8 entry first (and an update of ALLOWED_BIN/ALLOWED_LIB)")
        if package.read("lib/libplx.so") != so.read_bytes():
            raise PackageError(f"the packaged libplx.so differs from {so}")
    print(f"package: {tpk} ({tpk.stat().st_size} bytes)")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("stage", "tpk"):
        p = sub.add_parser(name)
        p.add_argument("--so", required=True, type=Path)
        p.add_argument("--mock-host")
        p.add_argument("--log-host")
    sub.add_parser("check-signing")
    args = parser.parse_args(argv)
    try:
        if args.command == "check-signing":
            check_signing(dict(os.environ))
            print("signing: the four PLX_TIZEN_* variables are set")
        elif args.command == "stage":
            stage(ROOT, args.so, args.mock_host, args.log_host)
        else:
            build_tpk(ROOT, args.so, args.mock_host, args.log_host, dict(os.environ))
    except PackageError as error:
        print(f"tizen/package.py: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
