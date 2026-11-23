#!/usr/bin/env python3
"""Receiver for the Tizen app's UDP log channel (docs/tizen.md, "Log channel"): prints each line and reports lost datagrams.

    python3 tools/tizen-log-receiver.py [--port PORT] [--bind ADDRESS] [--save FILE]
    python3 tools/tizen-log-receiver.py --stamp IN.tpk [--out OUT.tpk] [--address IPV4] [--port PORT]

Stamping makes a copy of the package with the local log destination. Install the copy through Apps2Samsung so it is signed again.
The package sends one datagram per log line when it was built with TIZEN_LOG_HOST=IPV4:PORT (res/log-host.txt): from the Tizen TV
emulator the build machine's loopback is 10.0.2.2, so the usual value is `10.0.2.2:PORT` with this script listening on PORT of the
build machine. The .NET host's lines are `h<seq> <line>` (a failed library load can only be reported from C#), and a library line that
carries a leading `<seq> ` is counted the same way; a line with neither prefix is printed and not counted. Each stream counts from 0 at
every app start, so a sequence that goes DOWN is a restart and a sequence that skips is datagrams lost (UDP never retransmits, and a gap
is the only evidence the log is incomplete). Each is reported on its own `--` line, and the exit summary counts them: a run whose log is
claimed as evidence should end with `lost 0`.

`nc -klu PORT` shows the same lines without the sequence check. Standard library only; Python 3.10 or newer.
"""
from __future__ import annotations

import argparse
import ipaddress
import re
import shutil
import socket
import sys
import tempfile
import zipfile
from contextlib import nullcontext
from pathlib import Path

LINE = re.compile(r"(h?)(\d+) ")
LOG_ENTRY = "res/log-host.txt"


def log_target(address: str, port: int) -> str:
    ipv4 = ipaddress.IPv4Address(address)
    if not 1 <= port <= 65535:
        raise ValueError("UDP port must be in 1..65535")
    return f"{ipv4}:{port}"


def local_address() -> str:
    # UDP connect selects a route without sending a packet.
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.connect(("192.0.2.1", 9))
        return sock.getsockname()[0]


def stamp(source: Path, output: Path, target: str, replace: bool = False) -> None:
    if source.resolve() == output.resolve():
        raise ValueError("choose an output path different from the original package")
    if output.suffix != ".tpk":
        raise ValueError("the output path must end in .tpk")
    with zipfile.ZipFile(source) as original:
        names = original.namelist()
        if "tizen-manifest.xml" not in names or len(names) != len(set(names)):
            raise ValueError("expected a Tizen package with a manifest and unique entries")
        if LOG_ENTRY in names and not replace:
            raise ValueError(f"the package already carries {LOG_ENTRY}. Use --replace to change it")
        with tempfile.TemporaryDirectory(prefix=".tizen-stamp-", dir=output.parent) as scratch:
            candidate = Path(scratch) / "package.tpk"
            with zipfile.ZipFile(candidate, "w", compression=zipfile.ZIP_DEFLATED) as package:
                package.comment = original.comment
                for entry in original.infolist():
                    if entry.filename != LOG_ENTRY:
                        with original.open(entry) as data, package.open(entry, "w") as dest:
                            shutil.copyfileobj(data, dest)
                package.writestr(LOG_ENTRY, target + "\n")
            # Exclusive creation keeps a previously stamped copy intact, including on filesystems
            # without hard links.
            dest = output.open("xb")
            try:
                with dest, candidate.open("rb") as data:
                    shutil.copyfileobj(data, dest)
            except OSError:
                output.unlink(missing_ok=True)
                raise


class Tracker:
    """Per-(sender, stream) sequence bookkeeping, apart from the socket so it can be tested."""

    def __init__(self) -> None:
        self.expect: dict[tuple[str, str], int] = {}
        self.lost = 0
        self.restarts = 0
        self.lines = 0

    def note(self, sender: str, text: str) -> str | None:
        """Account for one datagram; return a report line when its sequence number is not the next one."""
        self.lines += 1
        match = LINE.match(text)
        if not match:
            return None
        stream, seq = match.group(1), int(match.group(2))
        key = (sender, stream)
        want = self.expect.get(key, 0)
        self.expect[key] = seq + 1
        if seq > want:
            self.lost += seq - want
            return f"-- {seq - want} datagram(s) lost before {stream}{seq:06d}"
        if seq < want:
            self.restarts += 1
            return f"-- {stream}{seq:06d} after {stream}{want - 1:06d}: app restarted or reordered"
        return None

    def summary(self) -> str:
        return f"{self.lines} datagram(s); lost {self.lost}; restarts or reorderings {self.restarts}"


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--port", type=int, default=9999, help="UDP port to listen on (default 9999)")
    parser.add_argument("--bind", default="0.0.0.0", help="address to listen on (default all IPv4 interfaces)")
    parser.add_argument("--save", type=Path, help="also save received lines to this file")
    parser.add_argument("--stamp", type=Path, help="copy a TPK with this computer's UDP log destination")
    parser.add_argument("--out", type=Path, help="stamped copy (default adds -with-logs before .tpk)")
    parser.add_argument("--address", help="IPv4 address the TV can reach (default uses the local route)")
    parser.add_argument("--replace", action="store_true", help="replace the log destination already in the input")
    args = parser.parse_args(argv)
    try:
        if args.stamp:
            output = args.out or args.stamp.with_name(args.stamp.stem + "-with-logs.tpk")
            target = log_target(args.address or local_address(), args.port)
            stamp(args.stamp, output, target, args.replace)
            print(f"Stamped {output} to log to {target}. Install this copy through Apps2Samsung to sign it again.")
            if not args.save:
                return 0
        elif args.out or args.address or args.replace:
            parser.error("--out, --address and --replace require --stamp")
        log_target(args.bind, args.port)
        receive(args.bind, args.port, args.save)
    except (OSError, ValueError, zipfile.BadZipFile) as error:
        parser.exit(1, f"{parser.prog}: {error}\n")
    return 0


def receive(bind: str, port: int, save: Path | None) -> None:
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.bind((bind, port))
        with (save.open("x", encoding="utf-8") if save else nullcontext()) as log:
            print(f"listening on udp/{bind}:{port}", file=sys.stderr, flush=True)
            tracker = Tracker()
            try:
                while True:
                    data, peer = sock.recvfrom(4096)
                    line = data.decode("utf-8", "replace").rstrip("\n")
                    report = tracker.note(peer[0], line)
                    for text in (report, line):
                        if text is not None:
                            print(text, flush=True)
                            if log:
                                print(text, file=log, flush=True)
            except KeyboardInterrupt:
                print(f"\n{tracker.summary()}", file=sys.stderr)
                if log:
                    print(tracker.summary(), file=log)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
