#!/usr/bin/env python3
"""C-NOTAB check: no way to open a second tab or window from the page. Run it inside a
nested compositor (web/tools/nest.sh), never on the user's display:

    (cd web/tools/inject && cargo build --release)
    WAYLAND_DISPLAY=wayland-1 web/host-cef/test/notab.py --out .qwork/notab

It plays the shell like drive.py (control socket, memory-capped host, test pages from
127.0.0.1) and uses real input through web/tools/inject (nested compositor):
  - Ctrl+click, Shift+click and Ctrl+Shift+click on a link, and Ctrl+Enter in a form that
    targets _blank: no event at all (no new page, the page is not navigated either), and the
    host log says the popup was dropped;
  - a middle-click on a link (C-MIDCLICK): `open-link` with its URL, no new page, no navigation;
  - a plain click on a target=_blank link (C-FOCUSEXISTING): `blank-link` with its URL, no new
    page (the shell would open it), no navigation;
  - a plain click on a target=_blank rel=opener link and a plain Enter in the target=_blank form
    open a new page (C-POPUPS: `tab-created` with `opener`, closed again with `close-tab`);
    nothing else does;
  - right-click on a link, an image, selected text, the page and an editable field: no context
    menu at all (C-NOMENU): the host log says it was suppressed and a grim screenshot taken
    before and after the click is identical (no popup surface); the page still receives its
    `contextmenu` event, and a page's own `contextmenu` handler (a custom menu) still fires.
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

# The host's line for a dropped new-tab/window request: `popup Drop: ...` (on_before_popup)
# or `open-url-from-tab Drop: ...` (on_open_url_from_tab; what real Ctrl/middle/Shift
# clicks on a link go through, NEST-HEADLESS live run).
DROP = " Drop: "


def suppressed_lines(host_log_lines):
    """The host's `context menu: suppressed` / `run suppressed` lines."""
    return [l for l in host_log_lines if "context menu: " in l and "suppressed" in l]


def menu_shown_lines(host_log_lines):
    """Old-style `context menu: removed [...]; remaining [...]` lines: a menu got through."""
    return [l for l in host_log_lines if re.search(r"context menu: removed \[", l)]


def ppm_diff(a, b, masks=((90, 615, 430, 665), (90, 90, 520, 320), (550, 90, 870, 200))):
    """Number of pixels that differ between two P6 screenshots, ignoring the rectangle
    `masks` (x0, y0, x1, y1): the form's input (blinking caret) and the three links (their focus ring comes and goes when focus moves). A menu popup would
    paint far more than that rectangle (it opens below and right of the click)."""
    def parse(d):
        _, size, _, body = d.split(b"\n", 3)
        w, h = map(int, size.split())
        return w, h, body
    w, h, pa = parse(a)
    w2, h2, pb = parse(b)
    if (w, h) != (w2, h2):
        return w * h
    n = 0
    for y in range(h):
        ra, rb = pa[y * w * 3:(y + 1) * w * 3], pb[y * w * 3:(y + 1) * w * 3]
        if ra == rb:
            continue
        for x in range(w):
            if ra[x * 3:x * 3 + 3] != rb[x * 3:x * 3 + 3] and not any(m[0] <= x < m[2] and m[1] <= y < m[3] for m in masks):
                n += 1
    return n


def grim(path):
    r = subprocess.run(["grim", "-t", "ppm", path], check=False, timeout=30)
    if r.returncode != 0:
        raise Fail(f"grim exited {r.returncode} (WAYLAND_DISPLAY must be the nested compositor)")
    with open(path, "rb") as f:
        return f.read()


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
    # The clicks are real user gestures (the host adds --disable-popup-blocking itself: it is the blocker).
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
                "input": (150, 632), "page": (800, 800), "cust": (710, 640), "opener": (710, 140)}[sel]

    def move(sel):
        x, y = at(sel)
        return f"move:{x},{y}"

    sh.expect("hello", is_ev("hello"))
    first = sh.expect("tab-created", is_ev("tab-created"), 60)["tab"]
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

    def handed(what, event, url, *cmds):
        """A click the renderer hands to the shell (`open-link`, `blank-link`): exactly that event
        with the link's URL and nothing else but the page's input diagnostics: no new page, the
        page is not navigated."""
        n = len(sh.events)
        inject(*cmds)
        sh.seen_since(n, f"{event} for {what}", is_ev(event, tab=first, url=url), 10)
        time.sleep(1.0)
        with sh.cv:
            got = [ev for _, ev in sh.events[n:]]
        extra = [ev for ev in got if ev.get("event") not in (event, "editable-focused")
                 and not (ev.get("event") == "title" and ev["title"].startswith("ev:"))]
        if extra or sum(1 for ev in got if ev.get("event") == event) != 1:
            raise Fail(f"{what}: events after the input: {got}")
        sh.cursor = len(sh.events)
        sh.note(f"ok: {what}: {event} {url}, nothing opened, the page was not navigated")

    sh.note("step: user new-tab gestures on a link open nothing and do not navigate")
    quiet("ctrl+click", "hold:c", move("plain"), "click", "wait:200", "release", log_has=DROP)
    quiet("shift+click", "hold:s", move("plain"), "click", "wait:200", "release", log_has=DROP)
    quiet("ctrl+shift+click", "hold:cs", move("plain"), "click", "wait:200", "release")
    quiet("ctrl+shift+click on a target=_blank link", "hold:cs", move("blank"), "click", "wait:200", "release")
    quiet("ctrl+click on a target=_blank link", "hold:c", move("blank"), "click", "wait:200", "release", log_has=DROP)

    sh.note("step: a middle-click on a link is handed to the shell (C-MIDCLICK)")
    handed("middle-click", "open-link", base + "b.html?via=plain", move("plain"), "mclick", "wait:200")
    handed("middle-click on a target=_blank link", "open-link", base + "b.html?via=blank",
           move("blank"), "mclick", "wait:200")

    sh.note("step: a plain target=_blank click is handed to the shell (C-FOCUSEXISTING)")
    handed("target=_blank click", "blank-link", base + "b.html?via=blank", move("blank"), "click", "wait:200")

    sh.note("step: Ctrl+Enter in a form that targets _blank opens nothing")
    # Chromium dispatches no keypress for Ctrl+Enter, so there is no implicit submission
    # and nothing for the host to drop (NEST-HEADLESS live run); check the key arrived.
    quiet("ctrl+enter in the form", move("input"), "click", "wait:200", "key:c-ret", "wait:200",
          saw="ev:keydown:Enter:c")

    def popup(what, url, *cmds):
        """C-POPUPS: a plain click (a user gesture) on target=_blank opens a new page; the first
        page is not navigated. The new page is closed again."""
        mark = len(sh.events)
        inject(*cmds)
        new = sh.expect(f"tab-created for {what}", is_ev("tab-created", opener=first, cause="popup"), 20)["tab"]
        sh.seen_since(mark, f"url of the new page ({what})", is_ev("url", tab=new, url=url), 20)
        if not sh.none_since(mark, is_ev("url", tab=first)):
            raise Fail(f"{what}: the first page navigated")
        send({"cmd": "close-tab", "tab": new})
        sh.seen_since(mark, f"tab-closed {new}", is_ev("tab-closed", tab=new), 20)
        sh.cursor = len(sh.events)
        time.sleep(0.8)
        sh.note(f"ok: {what}: a new page (tab {new}), the first page stayed")

    sh.note("step: plain click on a target=_blank rel=opener link opens a new page (C-POPUPS)")
    popup("target=_blank rel=opener click", base + "b.html?via=opener", move("opener"), "click", "wait:200")
    if not sh.none_since(0, is_ev("blank-link", url=base + "b.html?via=opener")):
        raise Fail("the rel=opener link was handed to the shell")

    sh.note("step: plain Enter in the target=_blank form opens a new page (C-POPUPS)")
    popup("Enter in the form", lambda u: (u or "").startswith(base + "b.html?via=form"),
          move("input"), "click", "wait:200", "key:ret", "wait:200")
    inject(move("page"), "click", "wait:300")  # focus the first page again

    sh.note("step: no context menu for any right-click; the page still gets the event")
    shot_dir = os.path.join(os.path.dirname(host_log_path), "shots")
    os.makedirs(shot_dir, exist_ok=True)
    cases = (("link", [move("plain")], "ev:contextmenu"), ("target=_blank link", [move("blank")], "ev:contextmenu"),
             ("image", [move("img")], "ev:contextmenu"),
             ("selected text", [move("txt"), "click", "wait:100", "key:c-a"], "ev:contextmenu"),
             ("page", [move("page"), "click"], "ev:contextmenu"),
             ("editable field", [move("input"), "click", "wait:200"], "ev:contextmenu"),
             ("page's own contextmenu handler", [move("cust")], "ev:custom"))
    for what, pre, want in cases:
        inject(*pre, "wait:500")  # (a link hover shows Chromium's status bubble, which fades in)
        time.sleep(2.0)
        before = grim(os.path.join(shot_dir, "before.ppm"))
        lines = len(read_log(host_log_path))
        n = len(sh.events)
        inject("rclick", "wait:1200")
        time.sleep(0.3)
        after = grim(os.path.join(shot_dir, "after.ppm"))
        new = read_log(host_log_path)[lines:]
        with sh.cv:
            titles = [ev["title"] for _, ev in sh.events[n:] if ev.get("event") == "title"]
        changed = ppm_diff(before, after)
        shown = menu_shown_lines(new)
        sup = suppressed_lines(new)
        sh.note(f"right-click on {what}: host log {[l.split('] ', 1)[-1] for l in sup]}, page titles {titles}, "
                f"screen: {changed} px changed outside the input")
        if shown:
            raise Fail(f"{what}: a menu was built: {shown}")
        if changed:
            raise Fail(f"{what}: the screen changed after the right-click (a menu popup?): see {shot_dir}")
        if not any(t.startswith(want) for t in titles):
            raise Fail(f"{what}: the page never saw '{want}' (saw {titles}): the right-click did not reach it")
        inject("key:esc", "wait:200")
    if not suppressed_lines(read_log(host_log_path)):
        sh.note("note: the host never logged a suppressed menu (Chromium asked for none)")
    if pages_created(sh) != (1, 2):
        raise Fail(f"pages created (other, popups): {pages_created(sh)}, want (1, 2)")

    sh.note("step: quit")
    send({"cmd": "quit"})
    sh.expect("tab-closed", is_ev("tab-closed", tab=first), 20)
    with sh.cv:
        sh.cv.wait_for(lambda: sh.closed, 20)
    if pages_created(sh) != (1, 2):
        raise Fail(f"pages created (other, popups): {pages_created(sh)}, want (1, 2)")


def pages_created(sh):
    """(pages that are not popups, popups): only the first page and the two popups (the rel=opener
    link and the form; a plain target=_blank click is handed to the shell, `blank-link`)."""
    created = [e for _, e in sh.events if e.get("event") == "tab-created"]
    popups = sum(1 for e in created if e.get("opener") is not None)
    return len(created) - popups, popups


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
