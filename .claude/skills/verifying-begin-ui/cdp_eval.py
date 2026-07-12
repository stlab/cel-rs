#!/usr/bin/env python3
"""Evaluate a JS expression in a running Chromium/Edge tab via raw DevTools Protocol.

No dependencies beyond the stdlib - use this when node/Playwright/websocket-client
aren't installed. Usage:

    python3 cdp_eval.py <target-id> <js-expression>

<target-id> is the bare "id" field (NOT the full path - see below) from:

    curl -s http://127.0.0.1:9222/json | grep -A2 '"url": ".*8080'

The browser must be launched with --remote-debugging-port=9222.

IMPORTANT (git-bash on Windows): pass the bare ID, not "/devtools/page/<ID>".
An argument starting with "/" is silently rewritten by MSYS path conversion
(e.g. into "C:/Program Files/Git/devtools/page/<ID>") before this native
python.exe ever sees it, which corrupts the WebSocket handshake and hangs
with zero output - no exception, just a dead socket read. Building the path
internally from a slash-free ID sidesteps this entirely.
"""
import json
import os
import socket
import struct
import sys

CDP_HOST = "127.0.0.1"
CDP_PORT = 9222


def _recv_exact(sock: socket.socket, n: int) -> bytes:
    data = b""
    while len(data) < n:
        chunk = sock.recv(n - len(data))
        if not chunk:
            raise RuntimeError("socket closed while reading")
        data += chunk
    return data


def evaluate(target_id: str, expression: str, timeout: float = 10.0) -> dict:
    devtools_path = f"/devtools/page/{target_id}"
    sock = socket.create_connection((CDP_HOST, CDP_PORT), timeout=timeout)
    sock.settimeout(timeout)

    key = os.urandom(16)
    import base64

    req = (
        f"GET {devtools_path} HTTP/1.1\r\n"
        f"Host: {CDP_HOST}:{CDP_PORT}\r\n"
        "Upgrade: websocket\r\nConnection: Upgrade\r\n"
        f"Sec-WebSocket-Key: {base64.b64encode(key).decode()}\r\n"
        "Sec-WebSocket-Version: 13\r\n\r\n"
    )
    sock.sendall(req.encode())
    resp = b""
    while b"\r\n\r\n" not in resp:
        resp += sock.recv(4096)

    msg = json.dumps(
        {
            "id": 1,
            "method": "Runtime.evaluate",
            "params": {"expression": expression, "returnByValue": True},
        }
    ).encode()
    mask = os.urandom(4)
    masked = bytes(b ^ mask[i % 4] for i, b in enumerate(msg))
    length = len(msg)
    if length < 126:
        header = struct.pack("!BB", 0x81, 0x80 | length)
    elif length < 65536:
        header = struct.pack("!BBH", 0x81, 0x80 | 126, length)
    else:
        header = struct.pack("!BBQ", 0x81, 0x80 | 127, length)
    sock.sendall(header + mask + masked)

    for _ in range(50):
        b0, b1 = _recv_exact(sock, 2)
        length = b1 & 0x7F
        if length == 126:
            (length,) = struct.unpack("!H", _recv_exact(sock, 2))
        elif length == 127:
            (length,) = struct.unpack("!Q", _recv_exact(sock, 8))
        body = _recv_exact(sock, length)
        try:
            obj = json.loads(body.decode(errors="replace"))
        except ValueError:
            continue
        if obj.get("id") == 1:
            return obj
    raise RuntimeError("no id:1 response after 50 frames")


if __name__ == "__main__":
    result = evaluate(sys.argv[1], sys.argv[2])
    print(json.dumps(result, indent=2))
