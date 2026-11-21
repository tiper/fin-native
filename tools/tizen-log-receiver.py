#!/usr/bin/env python3
"""Receiver for the Tizen app's UDP log channel (docs/tizen.md, "Log channel"): prints each line and reports lost datagrams.

    python3 tools/tizen-log-receiver.py [--port PORT] [--bind ADDRESS]      # default 0.0.0.0:9999, Ctrl-C to stop

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
import re
import socket
import sys

LINE = re.compile(r"(h?)(\d+) ")


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
    args = parser.parse_args(argv)
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind((args.bind, args.port))
    print(f"listening on udp/{args.bind}:{args.port}", file=sys.stderr, flush=True)
    tracker = Tracker()
    try:
        while True:
            data, peer = sock.recvfrom(4096)
            text = data.decode("utf-8", "replace")
            report = tracker.note(peer[0], text)
            if report:
                print(report, flush=True)
            print(text, end="" if text.endswith("\n") else "\n", flush=True)
    except KeyboardInterrupt:
        print(f"\n{tracker.summary()}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
