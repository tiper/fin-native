#!/usr/bin/env python3
"""Press D-pad keys on the dev television through LG's network remote (SSAP), stdlib only.

    tools/tv-net-remote.py pair               # first run: accept the prompt on the TV
    tools/tv-net-remote.py front              # the app in front
    tools/tv-net-remote.py keys RIGHT,DOWN    # one press per second

This is the channel LG's phone remote uses (the second-screen API, `wss://<tv>:3001`), so unlike
`/tmp/plxnative-remote` (`rust-modules/src/remote.rs`, debug builds only) it reaches a STABLE
install. Measured on a webOS 6 C1 (13 Nov 2026): every key reached the stable app's event log.
That FIFO's note that injected input never reaches the app is about uinput and keymanager on
webOS 4.5; this path was not tried there.

Only UP, DOWN, LEFT, RIGHT and BACK are sent: no OK, so nothing can start a playback or change
a watch state on a real server. Take the TV lock first (the `tv-lock` skill).

The address comes from `tools/tv-config.sh host` (or `TV_HOST`). Pairing makes the TV issue a
client key, kept beside the per-user address as `ssap-client-key` (mode 600). It is a credential:
never commit or print it.
"""
import base64
import json
import os
import socket
import ssl
import struct
import subprocess
import sys
import time
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
ALLOWED = ("UP", "DOWN", "LEFT", "RIGHT", "BACK")
# The stock LG remote-app manifest the open-source webOS remote libraries send; `serial` is LG's
# public value, not this TV's.
MANIFEST = {
    "manifestVersion": 1,
    "appVersion": "1.0",
    "signed": {
        "created": "20140509", "appId": "com.lge.test", "vendorId": "com.lge",
        "localizedAppNames": {"": "LG Remote App"}, "localizedVendorNames": {"": "LG Electronics"},
        "permissions": ["TEST_SECURE", "READ_INSTALLED_APPS", "READ_RUNNING_APPS",
                        "CONTROL_INPUT_JOYSTICK", "CONTROL_MOUSE_AND_KEYBOARD", "READ_CURRENT_CHANNEL"],
        "serial": "2f930e2d2cfe083771f68e4fe7bb07",
    },
    "permissions": ["READ_INSTALLED_APPS", "READ_RUNNING_APPS", "CONTROL_INPUT_JOYSTICK",
                    "CONTROL_MOUSE_AND_KEYBOARD", "READ_CURRENT_CHANNEL"],
    "signatures": [{"signatureVersion": 1, "signature": "unsigned"}],
}


def tv_config(what):
    return subprocess.run([str(TOOLS / "tv-config.sh"), what], capture_output=True, text=True).stdout.strip()


def key_file():
    return Path(tv_config("dir")) / "ssap-client-key"


def frame(text, mask=None):
    """One masked client text frame (RFC 6455)."""
    data = text.encode()
    head = bytearray([0x81])
    n = len(data)
    if n < 126:
        head.append(0x80 | n)
    elif n < 65536:
        head += bytes([0x80 | 126]) + struct.pack(">H", n)
    else:
        head += bytes([0x80 | 127]) + struct.pack(">Q", n)
    mask = mask or os.urandom(4)
    return bytes(head) + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(data))


class Socket:
    """Just enough WebSocket for SSAP: send text, read text, ignore control frames."""

    def __init__(self, url):
        scheme, rest = url.split("://", 1)
        hostport, _, path = rest.partition("/")
        host, port = hostport.rsplit(":", 1)
        raw = socket.create_connection((host, int(port)), timeout=10)
        if scheme == "wss":
            ctx = ssl.create_default_context()
            ctx.check_hostname = False
            ctx.verify_mode = ssl.CERT_NONE  # the TV's certificate is self-signed
            raw = ctx.wrap_socket(raw, server_hostname=host)
        self.s = raw
        nonce = base64.b64encode(os.urandom(16)).decode()
        self.s.sendall((f"GET /{path} HTTP/1.1\r\nHost: {hostport}\r\nUpgrade: websocket\r\n"
                        f"Connection: Upgrade\r\nSec-WebSocket-Key: {nonce}\r\n"
                        "Sec-WebSocket-Version: 13\r\n\r\n").encode())
        reply = b""
        while b"\r\n\r\n" not in reply:
            chunk = self.s.recv(1)
            if not chunk:
                raise OSError("handshake closed")
            reply += chunk
        if b" 101 " not in reply.split(b"\r\n", 1)[0]:
            raise OSError(f"handshake refused: {reply.splitlines()[0]!r}")

    def send(self, text):
        self.s.sendall(frame(text))

    def _exact(self, n):
        out = b""
        while len(out) < n:
            chunk = self.s.recv(n - len(out))
            if not chunk:
                raise OSError("closed")
            out += chunk
        return out

    def recv(self, timeout):
        self.s.settimeout(timeout)
        while True:
            head = self._exact(2)
            op, n = head[0] & 0x0F, head[1] & 0x7F
            if n == 126:
                n = struct.unpack(">H", self._exact(2))[0]
            elif n == 127:
                n = struct.unpack(">Q", self._exact(8))[0]
            mask = self._exact(4) if head[1] & 0x80 else None
            body = self._exact(n)
            if mask:
                body = bytes(b ^ mask[i % 4] for i, b in enumerate(body))
            if op == 0x1:
                return body.decode()
            if op == 0x8:
                raise OSError("closed by the TV")


def register(ws, wait):
    payload = {"forcePairing": False, "pairingType": "PROMPT", "manifest": MANIFEST}
    path = key_file()
    if path.exists():
        payload["client-key"] = path.read_text().strip()
    ws.send(json.dumps({"type": "register", "id": "register", "payload": payload}))
    deadline = time.time() + wait
    while time.time() < deadline:
        msg = json.loads(ws.recv(max(1, deadline - time.time())))
        if msg.get("type") == "registered":
            path.parent.mkdir(parents=True, exist_ok=True)
            fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
            with os.fdopen(fd, "w") as f:
                f.write(msg["payload"]["client-key"])
            return True
        if msg.get("type") == "error":
            print(f"pairing refused: {msg.get('error')}", file=sys.stderr)
            return False
        if msg.get("payload", {}).get("pairingType") == "PROMPT":
            print("accept the prompt on the TV", flush=True)
    print("pairing timed out", file=sys.stderr)
    return False


def request(ws, uri):
    ws.send(json.dumps({"type": "request", "id": uri, "uri": uri}))
    while True:
        msg = json.loads(ws.recv(10))
        if msg.get("id") == uri:
            return msg.get("payload", {})


def checked_keys(arg):
    names = [n.strip().upper() for n in arg.split(",") if n.strip()]
    refused = [n for n in names if n not in ALLOWED]
    if refused or not names:
        sys.exit(f"tv-net-remote: only {', '.join(ALLOWED)} (refused: {refused or 'none given'})")
    return names


def main(argv):
    if len(argv) < 2 or argv[1] not in ("pair", "front", "keys") or (argv[1] == "keys") != (len(argv) == 3):
        sys.exit(__doc__)
    names = checked_keys(argv[2]) if argv[1] == "keys" else []
    host = os.environ.get("TV_HOST") or tv_config("host")
    if not host:
        sys.exit("no TV configured: put its address in ~/.config/plxnative/tv-host (or set TV_HOST)")
    ws = Socket(f"wss://{host}:3001/")
    if not register(ws, 60 if argv[1] == "pair" else 10):
        sys.exit(1)
    if argv[1] == "pair":
        print("paired; key saved", flush=True)
    elif argv[1] == "front":
        print(request(ws, "ssap://com.webos.applicationManager/getForegroundAppInfo").get("appId", "?"))
    else:
        pointer = Socket(request(ws, "ssap://com.webos.service.networkinput/getPointerInputSocket")["socketPath"])
        for i, name in enumerate(names):
            if i:
                time.sleep(1)
            pointer.send(f"type:button\nname:{name}\n\n")
            print(f"{time.strftime('%H:%M:%S')} {name}", flush=True)
        time.sleep(0.5)


if __name__ == "__main__":
    main(sys.argv)
