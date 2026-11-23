#!/usr/bin/env python3
"""C-NOTAB check: no way to open a second tab or window from the page. Run it inside a
nested compositor (web/tools/nest.sh), never on the user's display:

    (cd web/tools/inject && cargo build --release)
    WAYLAND_DISPLAY=wayland-1 web/host-cef/test/notab.py --out .qwork/notab

It plays the shell like drive.py (control socket, memory-capped host, test pages from
127.0.0.1) and uses real input through web/tools/inject (nested compositor):
  - Ctrl+click, middle-click, Shift+click and Ctrl+Shift+click on a link, and Ctrl+Enter
    in a form that targets _blank: no event at all (no new page, the page is not
    navigated either), and the host log says the popup was dropped;
  - a plain click on a target=_blank link and a plain Enter in that form still load in
    the same page;
  - right-click on a link, an image, selected text and the page: the host log lists the
    menu (what was removed, what remains) and no remaining item opens a tab or window.
Writes events.jsonl, host.log, fixture.log into --out. Exit 0 when every step passed."""

import argparse
import functools
import http.server
import os
import re
import socket
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import drive  # noqa: E402
from drive import Fail, Pages, Shell, is_ev  # noqa: E402

HOST_DIR = os.path.dirname(HERE)
REPO = os.path.dirname(os.path.dirname(HOST_DIR))
INJECT = os.path.join(REPO, "web", "tools", "inject", "target", "release", "inject")

# Menu labels that must not remain (lowercase substrings); see newtab.rs MENU_REMOVE.
FORBIDDEN = [
    "new tab", "new window", "incognito", "in profile", "view page source", "view source",
    "inspect", "search google", "search the web", "search with google", "search image", "lens",
    "open link", "open image", "open video", "open audio", "open in",
]


# The host's line for a dropped new-tab/window request: `popup Drop: ...` (on_before_popup)
# or `open-url-from-tab Drop: ...` (on_open_url_from_tab; what real Ctrl/middle/Shift
# clicks on a link go through, NEST-HEADLESS live run).
DROP = " Drop: "


def forbidden_labels(remaining):
    """The labels in a logged remaining-items list that open a tab, window or devtools.
    `remaining` is the host's text: `id:label, id:label, Sub[id:label, ...]`."""
    labels = re.findall(r"(?:^|[\[,]\s*)(?:\d+:)?([^\[\],]+)", remaining)
    return [l.strip() for l in labels if any(f in l.lower() for f in FORBIDDEN)]


def context_menus(host_log_lines):
    """[(removed, remaining)] of every `context menu:` line of the host log."""
    out = []
    for line in host_log_lines:
        m = re.search(r"context menu: removed \[(.*?)\]; remaining \[(.*)\]$", line)
        if m:
            out.append((m.group(1), m.group(2)))
    return out


def run(a):
    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)
    if not os.path.exists(INJECT):
        sys.exit(f"notab.py: build the injector first: (cd web/tools/inject && cargo build --release): {INJECT}")
    sh = Shell(out)
    fixture_log = open(os.path.join(out, "fixture.log"), "w")
    handler = functools.partial(Pages, directory=os.path.join(HERE, "pages"), log=fixture_log)
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    srv.daemon_threads = True
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_address[1]}/"
    sock_path = os.path.join(out, "control.sock")
    if os.path.exists(sock_path):
        os.unlink(sock_path)
    lsock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    lsock.bind(sock_path)
    lsock.listen(1)
    lsock.settimeout(60)
    unit = f"web-host-notab-{os.getpid()}"
    # No --disable-popup-blocking: the clicks are real user gestures.
    cmd = [os.path.join(HOST_DIR, "run.sh"), "--control", sock_path, "--url", base + "links.html"] + a.host_arg
    env = dict(os.environ, WEB_HOST_UNIT=unit, WEB_HOST_PROFILE=os.path.join(out, "profile"))
    host_log_path = os.path.join(out, "host.log")
    host_log = open(host_log_path, "w")
    sh.note(f"starting host: {' '.join(cmd)} (unit {unit}.scope)")
    proc = subprocess.Popen(cmd, env=env, stdout=host_log, stderr=subprocess.STDOUT)
    ok = False
    try:
        conn, _ = lsock.accept()
        threading.Thread(target=sh.reader, args=(conn,), daemon=True).start()
        steps(sh, conn, base, a, host_log_path)
        ok = True
    except (Fail, AssertionError, KeyError, TypeError) as e:
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
    sh.note("PASS" if ok else "FAILED")
    return 0 if ok else 1


def read_log(path):
    with open(path) as f:
        return f.read().splitlines()


def steps(sh, conn, base, a, host_log_path):
    send = lambda c: sh.send(conn, c)  # noqa: E731
    W, H = a.size

    def inject(*cmds):
        r = subprocess.run([INJECT, str(W), str(H), *cmds], check=False, timeout=60)
        if r.returncode != 0:
            raise Fail(f"inject {cmds} exited {r.returncode}")

    def at(sel):
        # Centres of the boxes in links.html (window coordinates; dwl border ignored).
        return {"plain": (300, 140), "blank": (300, 260), "img": (200, 400), "txt": (300, 540),
                "input": (150, 632), "page": (800, 800)}[sel]

    def move(sel):
        x, y = at(sel)
        return f"move:{x},{y}"

    sh.expect("hello", is_ev("hello"))
    sh.expect("tab-created", is_ev("tab-created"), 60)
    sh.expect("load end new", is_ev("load", phase="end", kind="new"), 60)
    time.sleep(1.0)
    inject(move("page"), "click", "wait:300")  # focus the page

    def quiet(what, *cmds, log_has=None, saw=None):
        """Runs input that must open nothing: no event afterwards but the page's input
        diagnostics (ev: titles) and focus changes (clicking the form's input focuses it)."""
        n = len(sh.events)
        lines = len(read_log(host_log_path))
        inject(*cmds)
        time.sleep(1.5)
        with sh.cv:
            got = [ev for _, ev in sh.events[n:]]
        # The page reports the input it saw in its title (links.html): not a navigation.
        seen = [ev["title"] for ev in got if ev.get("event") == "title" and ev["title"].startswith("ev:")]
        extra = [ev for ev in got if not (ev.get("event") == "title" and ev["title"].startswith("ev:"))
                 and ev.get("event") != "editable-focused"]
        if extra:
            raise Fail(f"{what}: events after the input: {extra}")
        if saw and saw not in seen:
            raise Fail(f"{what}: the page never saw '{saw}' (saw {seen}): the input did not arrive")
        new = read_log(host_log_path)[lines:]
        hit = [l.split("] ", 1)[-1] for l in new if log_has and log_has in l]
        if log_has and not hit:
            raise Fail(f"{what}: host log has no '{log_has}' line since the input; new lines: {new[-6:]}")
        sh.note(f"ok: {what} (page saw {seen}): nothing opened, the page was not navigated ({hit[0] if hit else 'no log check'})")

    sh.note("step: user new-tab gestures on a link open nothing and do not navigate")
    quiet("ctrl+click", "hold:c", move("plain"), "click", "wait:200", "release", log_has=DROP)
    quiet("middle-click", move("plain"), "mclick", "wait:200", log_has=DROP)
    quiet("shift+click", "hold:s", move("plain"), "click", "wait:200", "release", log_has=DROP)
    quiet("ctrl+shift+click", "hold:cs", move("plain"), "click", "wait:200", "release")
    quiet("ctrl+shift+click on a target=_blank link", "hold:cs", move("blank"), "click", "wait:200", "release")
    quiet("ctrl+click on a target=_blank link", "hold:c", move("blank"), "click", "wait:200", "release", log_has=DROP)
    quiet("middle-click on a target=_blank link", move("blank"), "mclick", "wait:200", log_has=DROP)

    sh.note("step: Ctrl+Enter in a form that targets _blank opens nothing")
    # Chromium dispatches no keypress for Ctrl+Enter, so there is no implicit submission
    # and nothing for the host to drop (NEST-HEADLESS live run); check the key arrived.
    quiet("ctrl+enter in the form", move("input"), "click", "wait:200", "key:c-ret", "wait:200",
          saw="ev:keydown:Enter:c")

    sh.note("step: plain target=_blank click still loads in the same page")
    # The url event comes before its load end (drive.py nav()): look back from the load end.
    mark = sh.cursor
    inject(move("blank"), "click", "wait:200")
    sh.expect("load end new via=blank", is_ev("load", phase="end", kind="new"), 20)
    sh.seen_since(mark, "url via=blank", is_ev("url", url=base + "b.html?via=blank"), before=sh.cursor)
    send({"cmd": "back"})
    sh.expect("load end traverse", is_ev("load", phase="end", kind="traverse"), 20)
    time.sleep(0.5)

    sh.note("step: plain Enter in the form still loads in the same page")
    mark = sh.cursor
    inject(move("input"), "click", "wait:200", "key:ret", "wait:200")
    sh.expect("load end new via=form", is_ev("load", phase="end", kind="new"), 20)
    sh.seen_since(mark, "url via=form", is_ev("url", url=lambda u: (u or "").startswith(base + "b.html?via=form")),
                  before=sh.cursor)
    send({"cmd": "back"})
    sh.expect("load end traverse", is_ev("load", phase="end", kind="traverse"), 20)
    time.sleep(0.5)

    sh.note("step: context menus (link, image, selected text, page)")
    n_before = len(context_menus(read_log(host_log_path)))
    for what, pre in (("link", [move("plain")]), ("target=_blank link", [move("blank")]), ("image", [move("img")]),
                      ("selected text", [move("txt"), "click", "wait:100", "key:c-a"]),
                      ("page", [move("page")])):
        lines = len(read_log(host_log_path))
        inject(*pre, "rclick", "wait:1200", "key:esc", "wait:400")
        menus = context_menus(read_log(host_log_path)[lines:])
        if len(menus) != 1:
            raise Fail(f"context menu on {what}: expected one menu line in the host log, got {len(menus)}")
        removed, remaining = menus[0]
        bad = forbidden_labels(remaining)
        sh.note(f"context menu on {what}: removed [{removed}]")
        sh.note(f"context menu on {what}: remaining [{remaining}]")
        if bad:
            raise Fail(f"context menu on {what} still has {bad}")
    if len(context_menus(read_log(host_log_path))) - n_before != 5:
        raise Fail("not every right-click gave a context menu")
    if sum(1 for _, e in sh.events if e.get("event") == "tab-created") != 1:
        raise Fail("a second page was created")

    sh.note("step: quit")
    send({"cmd": "quit"})
    sh.expect("tab-closed", is_ev("tab-closed"), 20)
    with sh.cv:
        sh.cv.wait_for(lambda: sh.closed, 20)
    if sum(1 for _, e in sh.events if e.get("event") == "tab-created") != 1:
        raise Fail("more than one tab-created")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True, help="output dir (use one under .qwork/)")
    ap.add_argument("--host-arg", action="append", default=[], help="extra switch for the host (repeatable)")
    ap.add_argument("--size", default="960x1006", help="the nested compositor's output size WxH (for inject)")
    a = ap.parse_args()
    a.size = tuple(int(v) for v in a.size.split("x"))
    if not os.environ.get("WAYLAND_DISPLAY"):
        sys.exit("notab.py: set WAYLAND_DISPLAY to a nested compositor (web/tools/nest.sh)")
    return run(a)


if __name__ == "__main__":
    sys.exit(main())
