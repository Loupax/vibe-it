#!/usr/bin/env python3
"""C1 check: drive web-host-cef over its control socket (protocol v0, web/PROTOCOL.md)
and check every event. Run it inside a nested compositor (scratchy/tools/nest.sh), never
on the user's display:

    WAYLAND_DISPLAY=wayland-1 web/host-cef/test/drive.py --out .qwork/c1-direct
    WAYLAND_DISPLAY=wayland-1 web/host-cef/test/drive.py --out .qwork/c1-proxy --proxy 127.0.0.1:PORT

It plays the shell: listens on the control socket, starts the host through run.sh
(memory-capped scope), serves the test pages from 127.0.0.1 (test/pages, plus a slow
URL for `stop`), sends commands and waits for the events each one must produce.
With --proxy the host gets `--proxy` and Chromium's `--proxy-bypass-list=<-loopback>`,
so the local test pages go through the proxy too (Chromium bypasses loopback by
default). Pages call window.open / click a target=_blank link from script, so the
run passes `--disable-popup-blocking` (no user gesture without input injection).

Writes into --out: events.jsonl (every event with its time), host.log (the host's
stderr), fixture.log (requests the test page server saw). Exit 0 when every step
passed. The host's scope is stopped at the end in any case."""

import argparse
import functools
import http.server
import json
import os
import socket
import subprocess
import sys
import threading
import time
import urllib.parse

HERE = os.path.dirname(os.path.abspath(__file__))
HOST_DIR = os.path.dirname(HERE)

# Every event type of protocol v0 the run must see (load per phase and kind).
REQUIRED = [
    "hello", "tab-created", "tab-closed", "url", "title",
    "load start new", "load end new", "load start reload", "load end reload",
    "load start traverse", "load end traverse",
    "load start same-document", "load end same-document",
    "editable-focused true", "editable-focused false", "find-result", "crashed", "error",
]


def event_key(ev):
    """The REQUIRED name an event counts for."""
    e = ev.get("event")
    if e == "load":
        return f"load {ev['phase']} {ev['kind']}"
    if e == "editable-focused":
        return f"editable-focused {'true' if ev['focused'] else 'false'}"
    return e


def missing_events(events):
    seen = {event_key(ev) for ev in events}
    return [r for r in REQUIRED if r not in seen]


def announced_before_shown(host_log_lines, tab):
    """True when the host logged sending tab-created <tab> before showing its window."""
    created = shown = None
    for i, line in enumerate(host_log_lines):
        if created is None and f"event: tab-created {tab}" in line:
            created = i
        if shown is None and f"window of tab {tab} shown" in line:
            shown = i
    return created is not None and shown is not None and created < shown


class Fail(Exception):
    pass


class Pages(http.server.SimpleHTTPRequestHandler):
    """Serves test/pages; /slow answers after 10 s (for `stop`)."""

    def __init__(self, *a, log=None, **kw):
        self._log = log
        super().__init__(*a, **kw)

    def do_GET(self):
        if self.path.startswith("/slow"):
            time.sleep(10)
            try:
                body = b"<title>slow</title>slow"
                self.send_response(200)
                self.send_header("Content-Type", "text/html")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            except OSError:
                pass
            return
        super().do_GET()

    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def log_message(self, fmt, *args):
        if self._log:
            self._log.write(f"{time.strftime('%H:%M:%S')} {self.address_string()} {fmt % args}\n")
            self._log.flush()


class Shell:
    """The shell's end of the control socket."""

    def __init__(self, out):
        self.out = out
        self.events = []  # (t_ms, event dict)
        self.cursor = 0
        self.cv = threading.Condition()
        self.closed = False
        self.t0 = time.monotonic()
        self.transcript = open(os.path.join(out, "events.jsonl"), "w", buffering=1)

    def t(self):
        return int((time.monotonic() - self.t0) * 1000)

    def note(self, text):
        line = f"[{self.t():6d} ms] {text}"
        print(line, flush=True)
        self.transcript.write(json.dumps({"t_ms": self.t(), "note": text}) + "\n")

    def reader(self, conn):
        buf = conn.makefile("r", encoding="utf-8")
        for line in buf:
            line = line.strip()
            if not line:
                continue
            ev = json.loads(line)
            t = self.t()
            self.transcript.write(json.dumps({"t_ms": t, "recv": ev}) + "\n")
            print(f"[{t:6d} ms] <- {line}", flush=True)
            with self.cv:
                self.events.append((t, ev))
                self.cv.notify_all()
        with self.cv:
            self.closed = True
            self.cv.notify_all()

    def send(self, conn, cmd):
        line = json.dumps(cmd)
        t = self.t()
        self.transcript.write(json.dumps({"t_ms": t, "send": cmd}) + "\n")
        print(f"[{t:6d} ms] -> {line}", flush=True)
        conn.sendall((line + "\n").encode())

    def expect(self, desc, pred, timeout=20.0):
        """Waits for the next event (after the cursor) matching pred; moves the cursor past it."""
        deadline = time.monotonic() + timeout
        with self.cv:
            while True:
                for i in range(self.cursor, len(self.events)):
                    if pred(self.events[i][1]):
                        self.cursor = i + 1
                        return self.events[i][1]
                left = deadline - time.monotonic()
                if left <= 0 or self.closed:
                    raise Fail(f"timed out waiting for: {desc}")
                self.cv.wait(left)

    def seen_since(self, mark, desc, pred, timeout=20.0, before=None):
        """Waits for an event matching pred at index >= mark (and < before, if given),
        whatever the cursor; does not move the cursor."""
        deadline = time.monotonic() + timeout
        with self.cv:
            while True:
                end = len(self.events) if before is None else before
                if any(pred(ev) for _, ev in self.events[mark:end]):
                    return
                left = deadline - time.monotonic()
                if before is not None or left <= 0 or self.closed:
                    raise Fail(f"not seen: {desc}")
                self.cv.wait(left)

    def none_since(self, mark, pred):
        with self.cv:
            return not any(pred(ev) for _, ev in self.events[mark:])


def is_ev(name, **fields):
    def p(ev):
        return ev.get("event") == name and all(
            (v(ev.get(k)) if callable(v) else ev.get(k) == v) for k, v in fields.items())
    return p


def run(a):
    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)
    sh = Shell(out)

    # Test page server.
    fixture_log = open(os.path.join(out, "fixture.log"), "w")
    handler = functools.partial(Pages, directory=os.path.join(HERE, "pages"), log=fixture_log)
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    srv.daemon_threads = True
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_address[1]}/"
    sh.note(f"test pages at {base}")

    # Control socket: the shell listens, the host connects.
    sock_path = os.path.join(out, "control.sock")
    if os.path.exists(sock_path):
        os.unlink(sock_path)
    lsock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    lsock.bind(sock_path)
    lsock.listen(1)
    lsock.settimeout(60)

    unit = f"web-host-c1-{os.getpid()}"
    cmd = [os.path.join(HOST_DIR, "run.sh")] + (["--x11"] if a.x11 else []) + [
        "--control", sock_path, "--url", base + "a.html", "--disable-popup-blocking"]
    if a.proxy:
        cmd += ["--proxy", a.proxy, "--proxy-bypass-list=<-loopback>"]
    env = dict(os.environ, WEB_HOST_UNIT=unit, WEB_HOST_PROFILE=os.path.join(out, "profile"))
    host_log_path = os.path.join(out, "host.log")
    host_log = open(host_log_path, "w")
    sh.note(f"starting host: {' '.join(cmd)} (unit {unit}.scope)")
    proc = subprocess.Popen(cmd, env=env, stdout=host_log, stderr=subprocess.STDOUT)
    ok = False
    try:
        conn, _ = lsock.accept()
        threading.Thread(target=sh.reader, args=(conn,), daemon=True).start()
        steps(sh, conn, base, a)
        ok = True
    except Fail as e:
        sh.note(f"FAIL: {e}")
    finally:
        try:
            proc.wait(timeout=15)
        except subprocess.TimeoutExpired:
            sh.note(f"host still running: stopping {unit}.scope")
            subprocess.run(["systemctl", "--user", "stop", f"{unit}.scope"], check=False)
            proc.wait(timeout=15)
        srv.shutdown()
        lsock.close()
        if os.path.exists(sock_path):
            os.unlink(sock_path)
    host_log.close()
    if ok:
        with open(host_log_path) as f:
            lines = f.read().splitlines()
        tab = next(ev["tab"] for _, ev in sh.events if ev.get("event") == "tab-created")
        if not announced_before_shown(lines, tab):
            sh.note("FAIL: tab-created was not sent before the window was shown")
            ok = False
        else:
            sh.note(f"tab-created {tab} was sent before its window was shown (host.log)")
        miss = missing_events([ev for _, ev in sh.events])
        if miss:
            sh.note(f"FAIL: events never seen: {miss}")
            ok = False
        else:
            sh.note(f"all {len(REQUIRED)} required events seen")
        if proc.returncode != 0:
            sh.note(f"FAIL: host exit code {proc.returncode}")
            ok = False
        if a.proxy_log:
            with open(a.proxy_log) as f:
                plog = f.read()
            ext = urllib.parse.urlsplit(a.external)
            want = [f"GET {base}a.html ", f"GET {base}b.html?via=open ", f"GET {base}b.html?via=blank ",
                    f"CONNECT {ext.hostname}:{ext.port or 443} "]
            miss = [w.strip() for w in want if w not in plog]
            if miss:
                sh.note(f"FAIL: not in the proxy log: {miss}")
                ok = False
            else:
                sh.note(f"proxy log has the test page requests and the {ext.hostname} tunnel")
    sh.note("PASS" if ok else "FAILED")
    return 0 if ok else 1


def steps(sh, conn, base, a):
    A, B = base + "a.html", base + "b.html"
    send = functools.partial(sh.send, conn)

    def nav(kind, url=None, title=None, status=200, timeout=20.0):
        """Waits for the next `load end <status> <kind>`; `url` must have been sent before
        it (since the last checked event), `title` at any time after that point."""
        mark = sh.cursor
        sh.expect(f"load end {status} {kind}", is_ev("load", phase="end", kind=kind, status=status), timeout)
        if url:
            sh.seen_since(mark, f"url {url} before its load end", is_ev("url", url=url), before=sh.cursor)
        if title:
            sh.seen_since(mark, f"title {title}", is_ev("title", title=title))

    hello = sh.expect("hello", is_ev("hello"))
    if hello["protocol"] != 0:
        raise Fail(f"protocol {hello['protocol']}")
    tab = sh.expect("tab-created", is_ev("tab-created"), 60)["tab"]

    sh.note(f"step: first page ({A})")
    sh.expect("load start new", is_ev("load", phase="start", kind="new"), 60)
    nav("new", A, "C1 page A")

    sh.note("step: open B")
    send({"cmd": "open", "url": B})
    sh.expect("load start new", is_ev("load", phase="start", kind="new"))
    nav("new", B, "C1 page B")

    sh.note("step: back, forward")
    send({"cmd": "back"})
    sh.expect("load start traverse", is_ev("load", phase="start", kind="traverse"))
    nav("traverse", A, "C1 page A")
    send({"cmd": "forward"})
    sh.expect("load start traverse", is_ev("load", phase="start", kind="traverse"))
    nav("traverse", B, "C1 page B")

    sh.note("step: reload")
    send({"cmd": "reload"})
    sh.expect("load start reload", is_ev("load", phase="start", kind="reload"))
    nav("reload")
    send({"cmd": "reload", "hard": True})
    nav("reload")

    sh.note("step: same-document navigation (location.hash from the page)")
    send({"cmd": "open", "url": A + "?do=hash"})
    nav("new", A + "?do=hash")
    mark = sh.cursor
    nav("same-document", A + "?do=hash#frag", status=0)
    sh.seen_since(mark, "load start same-document before its end",
                  is_ev("load", phase="start", kind="same-document"), before=sh.cursor)

    sh.note("step: same-document navigation (history.pushState from the page)")
    send({"cmd": "open", "url": A + "?do=push"})
    nav("new", A + "?do=push")
    nav("same-document", base + "a.html?pushed=1", status=0)

    sh.note("step: open a fragment of the current page (same-document), then back")
    send({"cmd": "open", "url": base + "a.html?pushed=1#x"})
    nav("same-document", base + "a.html?pushed=1#x", status=0)
    send({"cmd": "back"})
    nav("same-document", base + "a.html?pushed=1", status=0)

    sh.note("step: editable focus (the page focuses, then blurs, its text input)")
    send({"cmd": "open", "url": A + "?do=focus"})
    nav("new", A + "?do=focus")
    sh.expect("editable-focused true", is_ev("editable-focused", focused=True))
    sh.expect("editable-focused false", is_ev("editable-focused", focused=False))

    sh.note("step: find")
    send({"cmd": "find", "query": "needle", "dir": "next"})
    sh.expect("find-result 1 3", is_ev("find-result", index=1, count=3))
    send({"cmd": "find", "query": "needle", "dir": "next"})
    sh.expect("find-result 2 3", is_ev("find-result", index=2, count=3))
    send({"cmd": "find", "query": "needle", "dir": "prev"})
    sh.expect("find-result 1 3", is_ev("find-result", index=1, count=3))
    send({"cmd": "find", "query": "no-such-word", "dir": "next"})
    sh.expect("find-result 0 0", is_ev("find-result", index=0, count=0))
    send({"cmd": "find-clear"})

    sh.note("step: zoom")
    mark = len(sh.events)
    send({"cmd": "zoom", "factor": 1.5})
    send({"cmd": "zoom", "factor": 9})
    sh.expect("error for zoom 9", is_ev("error", message=lambda m: "zoom" in (m or "")))
    if not sh.none_since(mark, lambda ev: ev.get("event") == "error" and "1.5" in ev.get("message", "")):
        raise Fail("zoom 1.5 was refused")
    send({"cmd": "zoom", "factor": 1.0})

    sh.note("step: window.open from the page loads in the same page")
    mark = len(sh.events)
    send({"cmd": "open", "url": A + "?do=popup"})
    nav("new", A + "?do=popup")
    nav("new", base + "b.html?via=open", "C1 page B")

    sh.note("step: target=_blank link loads in the same page")
    send({"cmd": "open", "url": A + "?do=blank"})
    nav("new", A + "?do=blank", "C1 page A")
    nav("new", base + "b.html?via=blank", "C1 page B")
    if not sh.none_since(mark, is_ev("tab-created")):
        raise Fail("a popup created a second page")

    sh.note("step: stop a slow load")
    send({"cmd": "open", "url": base + "slow"})
    time.sleep(1.0)
    send({"cmd": "stop"})
    sh.expect("load end with ERR_ABORTED", is_ev("load", phase="end", error="ERR_ABORTED"), 5)

    sh.note("step: a host that does not resolve (error page; through a proxy: the proxy's 502)")
    send({"cmd": "open", "url": "http://nx-c1-check.invalid/"})
    sh.expect("load start new", is_ev("load", phase="start", kind="new"))
    ev = sh.expect("load end", is_ev("load", phase="end", kind="new"))
    if not (ev.get("error") == "ERR_NAME_NOT_RESOLVED" or ev["status"] >= 500):
        raise Fail(f"unresolvable host ended with {ev}")
    time.sleep(2.0)  # the engine retries error pages on its own; that must not show up
    if not sh.none_since(sh.cursor, is_ev("load")):
        raise Fail("load events after the error page with no command")

    sh.note(f"step: external page {a.external}")
    send({"cmd": "open", "url": a.external})
    nav("new", a.external, timeout=30)

    sh.note("step: bad commands")
    send({"cmd": "new-tab"})
    sh.expect("error for new-tab", is_ev("error"))
    conn.sendall(b"not json\n")
    sh.expect("error for not json", is_ev("error"))

    sh.note("step: renderer crash (chrome://crash), then open a page again")
    send({"cmd": "open", "url": "chrome://crash"})
    sh.expect("crashed", is_ev("crashed", tab=tab), 30)
    send({"cmd": "open", "url": A})
    nav("new", A, timeout=30)

    sh.note("step: quit")
    send({"cmd": "quit"})
    sh.expect("tab-closed", is_ev("tab-closed", tab=tab), 20)
    with sh.cv:
        sh.cv.wait_for(lambda: sh.closed, 20)
    if not sh.closed:
        raise Fail("host did not close the control socket after quit")
    tabs = {ev["tab"] for _, ev in sh.events if "tab" in ev}
    if tabs != {tab}:
        raise Fail(f"events name more than one page: {sorted(tabs)}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True, help="output dir (use one under .qwork/)")
    ap.add_argument("--proxy", help="host:port of an HTTP forward proxy you started")
    ap.add_argument("--proxy-log", help="that proxy's log (fwd_proxy.py --log): check the page requests are in it")
    ap.add_argument("--x11", action="store_true", help="run the host on X11 (DISPLAY)")
    ap.add_argument("--external", default="https://example.com/", help="a public page to load")
    a = ap.parse_args()
    if not a.x11 and not os.environ.get("WAYLAND_DISPLAY"):
        sys.exit("drive.py: set WAYLAND_DISPLAY to a nested compositor (scratchy/tools/nest.sh)")
    return run(a)


if __name__ == "__main__":
    sys.exit(main())
