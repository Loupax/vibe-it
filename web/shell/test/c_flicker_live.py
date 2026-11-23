#!/usr/bin/env python3
"""C-FLICKER live check: fixed elements must not flicker while a page scrolls on the dmabuf path.

    web/shell/test/c_flicker_live.py --out .qwork/c-flicker [--seconds 20] [--compare]

The bug (user, 2026-11-23): scrolling YouTube made its fixed header and sidebar flicker, only with
dmabuf (C6d) on an NVIDIA desktop. The engine commits a dmabuf as soon as its drawing is queued
on the GPU; the shell sampled it before the GPU had finished, so the layers drawn last (fixed
header, sidebar, overlays) were missing from some frames. The fix (`src/sync.rs`,
`src/syncobj.rs`) holds such commits until the buffer is ready (explicit sync, else the
dmabuf's implicit fence) and releases buffers only after the shell's GPU has read them.

The check runs the shell on the GPU nest (web/tools/gpu-nest.sh: KWin --virtual on the GPU,
dmabuf offered; no window on the desktop), loads test/pages/flicker.html (a magenta fixed header
and sidebar over a long page, under 24 full-window backdrop-blur layers so every engine frame
costs the GPU milliseconds: a frame sampled early shows no header) and scrolls it with wheel
events over Chromium's DevTools protocol (the GPU nest has no input injection; the host gets
`--remote-debugging-port`). The shell's pixel probe (`WEB_PROBE`, `src/probe.rs`) reads the
header and sidebar back from every frame it renders. PASS: at least --min-frames frames and
every one of them has both regions magenta; once with explicit sync (linux-drm-syncobj-v1, which
CEF 156 uses when offered) and once with WEB_NO_SYNCOBJ=1 (the dmabuf's implicit fence).

--compare also runs it with WEB_NO_BUFFER_SYNC=1 (the shell before the fix) and expects
torn frames there (shows the check detects the bug). Exit 0 on PASS; 2 = SKIP (no GPU path:
the shell did not offer dmabuf, e.g. a software renderer). Screenshots/logs go to --out (never
commit them). Needs web/shell/target/debug/web and web/host-cef/target/debug/web-host-cef."""

import argparse
import base64
import functools
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import threading
import time
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True
import live  # noqa: E402

WEB = live.WEB
# Logical shell-window rectangles inside the page's fixed header and sidebar (flicker.html).
PROBE = "100,10,500,36;10,100,50,500"
PROBE_RX = re.compile(r"probe: frame (\d+)((?: \| [^|]+)+)")
ENTRY_RX = re.compile(r"min (\d+),(\d+),(\d+) max (\d+),(\d+),(\d+)")


def magenta(entry):
    """A probe entry whose every pixel is (close to) the page's #f0f."""
    m = ENTRY_RX.search(entry)
    if not m:
        return False
    r0, g0, b0, r1, g1, b1 = map(int, m.groups())
    return r0 > 200 and b0 > 200 and g1 < 60


def judge(lines):
    """(frames, torn, examples) over the probe lines between the page's "flicker scrolling" and
    "flicker done" titles, from the first frame showing the fixed parts on (page loaded)."""
    frames = torn = 0
    examples = []
    on = started = False
    for line in lines:
        if '"title":"flicker scrolling"' in line:
            on = True
        elif '"title":"flicker done' in line:
            on = False
        m = PROBE_RX.search(line)
        if not (on and m):
            continue
        ok = all(magenta(e) for e in m[2].split("|")[1:])
        started = started or ok
        if not started:
            continue
        frames += 1
        if not ok:
            torn += 1
            if len(examples) < 5:
                examples.append(line.strip())
    return frames, torn, examples


class Cdp:
    """A minimal DevTools-protocol client (stdlib only: one websocket, text frames, masked)."""

    def __init__(self, port, timeout=40):
        end = time.monotonic() + timeout
        while True:
            try:
                targets = json.load(urllib.request.urlopen(f"http://127.0.0.1:{port}/json", timeout=2))
                pages = [t for t in targets if t.get("type") == "page" and "webSocketDebuggerUrl" in t]
                if pages:
                    break
            except OSError:
                pass
            if time.monotonic() > end:
                raise live.Fail("no DevTools page target (is --remote-debugging-port passed to the host?)")
            time.sleep(0.3)
        path = "/" + pages[0]["webSocketDebuggerUrl"].split("/", 3)[3]
        self.sock = socket.create_connection(("127.0.0.1", port), timeout=10)
        key = base64.b64encode(os.urandom(16)).decode()
        self.sock.sendall((f"GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\n"
                           f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n").encode())
        head = b""
        while b"\r\n\r\n" not in head:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise live.Fail("DevTools websocket handshake failed")
            head += chunk
        if b" 101 " not in head.split(b"\r\n")[0]:
            raise live.Fail(f"DevTools websocket refused: {head.split(b'\r\n')[0]!r}")
        self.buf = head.split(b"\r\n\r\n", 1)[1]
        self.id = 0

    def _send(self, text):
        data = text.encode()
        mask = os.urandom(4)
        n = len(data)
        hdr = bytes([0x81]) + (bytes([0x80 | n]) if n < 126 else bytes([0x80 | 126]) + n.to_bytes(2, "big"))
        self.sock.sendall(hdr + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(data)))

    def _read(self, n):
        while len(self.buf) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise live.Fail("DevTools websocket closed")
            self.buf += chunk
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def _recv(self):
        b0, b1 = self._read(2)
        n = b1 & 0x7F
        if n == 126:
            n = int.from_bytes(self._read(2), "big")
        elif n == 127:
            n = int.from_bytes(self._read(8), "big")
        payload = self._read(n)
        return payload.decode(errors="replace") if b0 & 0x0F == 1 else ""

    def call(self, method, params):
        self.id += 1
        self._send(json.dumps({"id": self.id, "method": method, "params": params}))
        while True:
            msg = self._recv()
            if msg and json.loads(msg).get("id") == self.id:
                return json.loads(msg)

    def close(self):
        self.sock.close()


def run(a, name, base, wl, extra_env):
    out = os.path.join(a.out, name)
    os.makedirs(out, exist_ok=True)
    state = os.path.join(out, "state")
    shutil.rmtree(state, ignore_errors=True)
    env = dict(os.environ, WAYLAND_DISPLAY=wl, WEB_PROBE=PROBE, **extra_env)
    for k in ("WAYLAND_SOCKET", "DISPLAY", "WEB_NO_DMABUF", "WEB_NO_SYNCOBJ", "WEB_NO_BUFFER_SYNC"):
        if k not in extra_env:
            env.pop(k, None)
    unit = f"web-c-flicker-{os.getpid()}-{name}"
    url = f"{base}/flicker.html#{a.seconds + 6},wheel,heavy24"
    cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
           "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", state, "--host-bin", a.host_bin, url,
           "--", f"--remote-debugging-port={a.port}", "--remote-allow-origins=*"]
    proc = subprocess.Popen(cmd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    log = live.Log(proc, os.path.join(out, "web.log"))
    try:
        offer = log.wait(r"dmabuf: (offered|not offered.*)", timeout=30)[1]
        sync_line = log.wait(r"explicit sync: (.*)", timeout=10)[1]
        if not offer.startswith("offered"):
            return "skip", f"dmabuf {offer}"
        log.wait(r'"title":"flicker scrolling"', timeout=60)
        cdp = Cdp(a.port)
        try:
            end, n = time.monotonic() + a.seconds, 0
            while time.monotonic() < end:
                # Down for 3 s, up for 3 s: wheel ticks as a mouse gives them, 20 a second.
                dy = 120 if (n // 60) % 2 == 0 else -120
                cdp.call("Input.dispatchMouseEvent", {"type": "mouseWheel", "x": 400, "y": 400, "deltaX": 0, "deltaY": dy})
                n += 1
                time.sleep(0.05)
        finally:
            cdp.close()
        log.wait(r'"title":"flicker done', timeout=20)
        frames, torn, examples = judge(list(log.lines))
        detail = f"{frames} frames, {torn} torn, {n} wheel events, explicit sync: {sync_line}"
        if examples:
            detail += "\n      " + "\n      ".join(examples)
        return ("ok" if torn == 0 and frames >= a.min_frames else "torn" if torn else "few"), detail
    finally:
        subprocess.run(["systemctl", "--user", "stop", unit + ".scope"], check=False, capture_output=True)
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            pass


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--web", default=os.path.join(live.SHELL, "target/debug/web"))
    ap.add_argument("--host-bin", default=os.path.join(WEB, "host-cef/target/debug/web-host-cef"))
    ap.add_argument("--wayland", help="run on this Wayland display instead of a GPU nest from web/tools/gpu-nest.sh")
    ap.add_argument("--seconds", type=int, default=20, help="how long to scroll (default 20)")
    ap.add_argument("--min-frames", type=int, default=150, help="frames the probe must see (default 150)")
    ap.add_argument("--port", type=int, default=9337, help="the engine's DevTools port (default 9337)")
    ap.add_argument("--no-implicit", action="store_true", help="skip the implicit-sync run (WEB_NO_SYNCOBJ=1)")
    ap.add_argument("--compare", action="store_true", help="also run without the fix's guards; expect torn frames")
    a = ap.parse_args()
    a.out = os.path.abspath(a.out)
    os.makedirs(a.out, exist_ok=True)
    wl = a.wayland or subprocess.run([os.path.join(WEB, "tools/gpu-nest.sh")], check=True, capture_output=True,
                                     text=True).stdout.strip()
    page_log = open(os.path.join(a.out, "pages.log"), "w", buffering=1)
    srv = live.Server(("127.0.0.1", 0), functools.partial(live.Pages, directory=os.path.join(HERE, "pages"), log=page_log))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_address[1]}"
    verdict, detail = run(a, "fixed", base, wl, {})
    print(f"{'PASS' if verdict == 'ok' else 'SKIP' if verdict == 'skip' else 'FAIL'} fixed elements while scrolling: {detail}",
          flush=True)
    if verdict == "skip":
        sys.exit(2)
    ok = verdict == "ok"
    if not a.no_implicit:
        # An engine without explicit sync (or WEB_NO_SYNCOBJ): the dmabuf's implicit fence.
        verdict, detail = run(a, "implicit", base, wl, {"WEB_NO_SYNCOBJ": "1"})
        print(f"{'PASS' if verdict == 'ok' else 'FAIL'} the same with implicit sync only (WEB_NO_SYNCOBJ=1): {detail}",
              flush=True)
        ok = ok and verdict == "ok"
    if a.compare:
        verdict, detail = run(a, "unguarded", base, wl, {"WEB_NO_BUFFER_SYNC": "1"})
        print(f"{'PASS' if verdict == 'torn' else 'FAIL'} the check sees the bug without the guards "
              f"(WEB_NO_BUFFER_SYNC=1): {detail}", flush=True)
        ok = ok and verdict == "torn"
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
