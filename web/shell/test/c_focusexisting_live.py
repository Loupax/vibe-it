#!/usr/bin/env python3
"""C-FOCUSEXISTING live check (KEYS.md T170, T105; SCRIPTING.md `web.buffers.focusExisting`) in the
headless nested dwl (web/tools/nest.sh), never on the user's display. Real clicks through web/tools/inject.

    web/shell/test/c_focusexisting_live.py --out .qwork/c-focusexisting --inject web/tools/inject/target/release/inject

Every web it starts gets a temporary XDG_CONFIG_HOME (with a test profiles/default/init.js), XDG_STATE_HOME,
XDG_DATA_HOME and --state-dir under --out: never the user's. Pages: test/pages/fe-a.html (red, target=_blank
links to B, to B#x, to A itself, and a rel=opener one to B) and fe-b.html (blue; its title says whether it
has window.opener).

Session 1, init.js `web.buffers.focusExisting = true;`:
  - Enter on the B link in NORMAL after a `/` search for its text (T18: a trusted click; no buffer has B):
    `blank-link`, a new buffer 2 (a `new-tab`, not a popup), no opener;
  - back to buffer 1 (Ctrl-^), a click on the B link: buffer 2 is shown, `already open: [2] fe B opener=no`,
    no new page; Ctrl-^ returns to buffer 1;
  - the B#x link: a new buffer (exact URL only);
  - the link to A itself (target=_BLANK): only the message, `already open: [1] fe A`;
  - a middle-click on the B link: a new buffer (`open-link`), the setting is ignored;
  - the link to /fe-redir (a 302 to fe-b.html?r=1; C-FOCUSEXISTING-REDIRECT): a new buffer 5 whose `url` event
    carries `original` = the link's URL; back to buffer 1 and a second click: buffer 5 is shown, `already open`;
  - the rel=opener link: Chromium's popup path as before (`tab-created` with `opener`, window.opener kept).
Session 2, init.js without the setting: a click on the B link twice opens two new buffers.
Exit 0 when every step passed. Needs what test/live.py needs, and PIL."""

import argparse
import functools
import os
import re
import shutil
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True  # no __pycache__ in the source tree
import live  # noqa: E402
import livelib  # noqa: E402
from c3_live import keys_for  # noqa: E402

SHELL = live.SHELL
WEB = live.WEB
Fail = live.Fail
RED, BLUE = (255, 0, 0), (0, 0, 255)
# Page coordinates (fe-a.html) of the link centres.
AT = {"tob": (190, 72), "tobx": (190, 172), "toa": (190, 272), "opener": (190, 372), "redir": (190, 472)}


class RedirPages(live.Pages):
    """test/pages plus /fe-redir: a 302 to fe-b.html?r=1 (C-FOCUSEXISTING-REDIRECT)."""

    def do_GET(self):
        if self.path == "/fe-redir":
            self.send_response(302)
            self.send_header("Location", "/fe-b.html?r=1")
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        super().do_GET()


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--web", default=os.path.join(SHELL, "target/debug/web"))
    ap.add_argument("--host-bin", default=os.path.join(WEB, "host-cef/target/debug/web-host-cef"))
    ap.add_argument("--inject", required=True, help="the inject binary (build web/tools/inject)")
    a = ap.parse_args()
    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)

    wl = subprocess.run([os.path.join(WEB, "tools/nest.sh")], check=True, capture_output=True, text=True).stdout.strip()
    nest_env = dict(os.environ, WAYLAND_DISPLAY=wl)

    page_log = open(os.path.join(out, "pages.log"), "w", buffering=1)
    srv = live.Server(("127.0.0.1", 0), functools.partial(RedirPages, directory=os.path.join(HERE, "pages"), log=page_log))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_address[1]}"
    url_a, url_b = f"{base}/fe-a.html", f"{base}/fe-b.html"
    results = []
    try:
        session(a, out, "on", "web.buffers.focusExisting = true;\n", wl, nest_env, url_a, url_b, results, steps_on)
        session(a, out, "off", "web.buffers.deleteConfirmation = true;\n", wl, nest_env, url_a, url_b, results, steps_off)
    finally:
        srv.shutdown()
    failed = [r for r in results if not r[1]]
    print(f"{len(results) - len(failed)}/{len(results)} steps passed; logs in {out}/<session>/web.log")
    return 1 if failed else 0


def session(a, out, name, init, wl, nest_env, url_a, url_b, results, steps):
    """One web, with a fresh config/state/data home and `init` as profiles/default/init.js."""
    d = os.path.join(out, name)
    shutil.rmtree(d, ignore_errors=True)
    for sub in ("config/web/profiles/default", "state", "data", "web-state"):
        os.makedirs(os.path.join(d, sub), mode=0o700)
    with open(os.path.join(d, "config/web/profiles/default/init.js"), "w") as f:
        f.write(init)
    env = dict(os.environ, WAYLAND_DISPLAY=wl, XDG_CONFIG_HOME=os.path.join(d, "config"),
               XDG_STATE_HOME=os.path.join(d, "state"), XDG_DATA_HOME=os.path.join(d, "data"))
    env.pop("WAYLAND_SOCKET", None)
    env.pop("DISPLAY", None)
    unit = f"web-c-focusexisting-live-{name}-{os.getpid()}"
    cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
           "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", os.path.join(d, "web-state"),
           "--host-bin", a.host_bin, url_a]
    proc = subprocess.Popen(cmd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    log = live.Log(proc, os.path.join(d, "web.log"))

    def step(title, fn):
        t0 = time.monotonic()
        try:
            detail = fn() or ""
            results.append((title, True, detail))
            print(f"  ok   [{name}] {title} ({time.monotonic() - t0:.1f}s) {detail}", flush=True)
        except Fail as e:
            results.append((title, False, str(e)))
            print(f"  FAIL [{name}] {title}: {e}", flush=True)

    def shot(n):
        p = os.path.join(d, n)
        subprocess.run(["grim", p], env=nest_env, check=True)
        return p

    try:
        log.wait(r'"title":"fe A"', timeout=60)
        g = livelib.settle(log, shot)
        from PIL import Image

        def shows(c, n):
            time.sleep(0.4)
            im = Image.open(shot(n)).convert("RGB")
            x, y = (g.bx + g.ww * 2 // 3) * g.scale, (g.by + g.wh // 3) * g.scale
            got = im.getpixel((int(x), int(y)))
            if got != c:
                raise Fail(f"{n}: the page area is {got}, not {c}")

        def inject(*cmds):
            subprocess.run([a.inject, str(g.ow), str(g.oh)] + list(cmds), env=nest_env, check=True)

        def click(sel, button="click"):
            x, y = AT[sel]
            inject(f"move:{g.bx + x},{g.by + y}", "wait:100", button, "wait:300")

        def bar(since, pat, timeout=8):
            return log.wait(r"\] bar: " + pat, since, timeout)

        def current(since, num, of, timeout=10):
            log.wait(rf"buffer {num} of {of} is current", since, timeout)

        def no_new_page(since, what):
            time.sleep(1.0)
            bad = [l for l in log.lines[since:] if '"cmd":"new-tab"' in l or '"event":"tab-created"' in l]
            if bad:
                raise Fail(f"{what}: a new page: {bad}")

        def back_to_1():
            k = log.mark()
            inject("hold:cs", "key:6", "release", "wait:300")  # Ctrl-^
            current(k, 1, len(buffers))
            shows(RED, f"a-{log.mark()}.png")

        buffers = [1]

        def ctx():
            return {"log": log, "step": step, "inject": inject, "click": click, "bar": bar, "current": current,
                    "no_new_page": no_new_page, "shows": shows, "buffers": buffers, "url_a": url_a, "url_b": url_b,
                    "url_redir": url_a.rsplit("/", 1)[0] + "/fe-redir",
                    "url_redir_final": url_a.rsplit("/", 1)[0] + "/fe-b.html?r=1",
                    "back_to_1": back_to_1, "keys_for": keys_for}

        # The nest keeps the pointer where the last run left it: a click there without a motion first
        # would find no pointer focus in the new window. Move it out and into the page first.
        inject("move:2,2", "wait:100", f"move:{g.bx + g.ww * 2 // 3},{g.by + g.wh // 3}", "wait:300")
        shows(RED, "start.png")
        steps(ctx())

        def quit_all():
            k = log.mark()
            inject(*keys_for(":q"), "key:del", "key:ret", "wait:200")
            code = proc.wait(timeout=30)
            if code != 0:
                raise Fail(f"web exited {code}")
            log.wait(r"engine host exited: exit status: 0", k, 5)
            return "web and its host exited 0"

        step("quit", quit_all)
    except Fail as e:
        results.append(("session " + name, False, str(e)))
        print(f"  FAIL [{name}] {e}", flush=True)
    finally:
        subprocess.run(["systemctl", "--user", "stop", f"{unit}.scope"], capture_output=True)
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            pass


def blank_event(c, since, url):
    c["log"].wait(r'host event: \{"event":"blank-link","tab":\d+,"url":"' + re.escape(url) + '"', since, 10)


def steps_on(c):
    log, click, bar, current, buffers = c["log"], c["click"], c["bar"], c["current"], c["buffers"]

    def enter_first():
        # Enter on a link counts as a click (a trusted `click`): a `/` search for the B link's text, Enter
        # (the first match), Enter again (T18: the find ends, which focuses the link, and the page gets the
        # Enter). First, on the fresh page: after a mouse click on the link and a buffer switch, the find's
        # end did not reliably move the focus in the nest.
        c["inject"](*c["keys_for"]("/link one"), "key:ret", "wait:500")
        k = log.mark()
        c["inject"]("key:ret", "wait:300")
        blank_event(c, k, c["url_b"])
        m = log.wait(r'host event: \{"event":"tab-created","tab":(\d+),"req":\d+\}', k, 15)
        buffers.append(2)
        current(k, 2, 2)
        log.wait(r'host event: \{"event":"title","tab":' + m.group(1) + r',"title":"fe B opener=no"', k, 15)
        c["shows"](BLUE, "b-new.png")
        c["back_to_1"]()
        return f"Enter: blank-link, new-tab -> buffer 2 (tab {m.group(1)}, no opener); Ctrl-^ back to buffer 1"

    c["step"]("T170 setting on, no buffer has the URL (Enter on the link): a new buffer", enter_first)

    def again():
        k = log.mark()
        click("tob")
        blank_event(c, k, c["url_b"])
        current(k, 2, 2)
        bar(k, r"NORMAL \| already open: \[2\] fe B opener=no")
        c["shows"](BLUE, "b-switched.png")
        c["no_new_page"](k, "the second click")
        k = log.mark()
        c["inject"]("hold:cs", "key:6", "release", "wait:300")
        current(k, 1, 2)
        c["shows"](RED, "a-again.png")
        return "buffer 2 shown, `already open: [2] fe B opener=no`, no new page; Ctrl-^ returns to buffer 1"

    c["step"]("T170 setting on, a buffer has the URL: switch, alternate, message", again)

    def fragment():
        k = log.mark()
        click("tobx")
        blank_event(c, k, c["url_b"] + "#x")
        buffers.append(3)
        current(k, 3, 3)
        c["back_to_1"]()
        return "B#x is not B: a new buffer 3"

    c["step"]("T170 exact URL only: a #fragment opens a new buffer", fragment)

    def itself():
        k = log.mark()
        click("toa")
        blank_event(c, k, c["url_a"])
        bar(k, r"NORMAL \| already open: \[1\] fe A")
        c["no_new_page"](k, "the link to the page itself")
        if any("is current" in l for l in log.lines[k:]):
            raise Fail("the buffer changed")
        c["shows"](RED, "a-itself.png")
        return "target=_BLANK to the page on screen: only `already open: [1] fe A`"

    c["step"]("T170 the match is the buffer on screen: only the message", itself)

    def middle():
        k = log.mark()
        click("tob", "mclick")
        log.wait(r'host event: \{"event":"open-link","tab":\d+,"url":"' + re.escape(c["url_b"]) + '"', k, 10)
        buffers.append(4)
        current(k, 4, 4)
        if any('"event":"blank-link"' in l for l in log.lines[k:]):
            raise Fail("a middle-click sent blank-link")
        c["shows"](BLUE, "b-middle.png")
        c["back_to_1"]()
        return "middle-click: open-link, a new buffer 4 although buffer 2 shows B"

    c["step"]("T105 a middle-click ignores the setting", middle)

    def redirect():
        # C-FOCUSEXISTING-REDIRECT: /fe-redir answers 302 to /fe-b.html?r=1; the buffer shows the target, and
        # the host's `url` event carries the requested URL as `original`.
        k = log.mark()
        click("redir")
        blank_event(c, k, c["url_redir"])
        m = log.wait(r'host event: \{"event":"tab-created","tab":(\d+),"req":\d+\}', k, 15)
        buffers.append(5)
        current(k, 5, 5)
        log.wait(r'host event: \{"event":"url","tab":' + m.group(1) + r',"url":"' + re.escape(c["url_redir_final"])
                 + r'","original":"' + re.escape(c["url_redir"]) + '"', k, 15)
        log.wait(r'host event: \{"event":"title","tab":' + m.group(1) + r',"title":"fe B opener=no"', k, 15)
        c["shows"](BLUE, "b-redir.png")
        c["back_to_1"]()
        k = log.mark()
        click("redir")
        blank_event(c, k, c["url_redir"])
        current(k, 5, 5)
        bar(k, r"NORMAL \| already open: \[5\] fe B opener=no")
        c["no_new_page"](k, "the second click on the redirecting link")
        c["shows"](BLUE, "b-redir-switched.png")
        c["back_to_1"]()
        return "302 link: `url` carried original; the second click switched to buffer 5, no new page"

    c["step"]("T170 redirect: a buffer matches the URL its page was requested as", redirect)

    def opener():
        k = log.mark()
        click("opener")
        m = log.wait(r'host event: \{"event":"tab-created","tab":(\d+),"opener":\d+,"cause":"popup"', k, 15)
        log.wait(r'host event: \{"event":"title","tab":' + m.group(1) + r',"title":"fe B opener=yes"', k, 15)
        if any('"event":"blank-link"' in l for l in log.lines[k:]):
            raise Fail("a rel=opener link sent blank-link")
        buffers.append(6)
        current(k, 6, 6)
        return f"rel=opener: Chromium's popup (tab {m.group(1)}, window.opener kept), not blank-link"

    c["step"]("T94 a rel=opener target=_blank link stays a popup", opener)


def steps_off(c):
    log, click, current, buffers = c["log"], c["click"], c["current"], c["buffers"]

    def twice():
        for n in (2, 3):
            k = log.mark()
            click("tob")
            blank_event(c, k, c["url_b"])
            log.wait(r'host event: \{"event":"tab-created","tab":(\d+),"req":\d+\}', k, 15)
            buffers.append(n)
            current(k, n, n)
            if any("already open" in l for l in log.lines[k:]):
                raise Fail("already open with the setting off")
            c["shows"](BLUE, f"b-off-{n}.png")
            c["back_to_1"]()
        return "two clicks on the B link: buffers 2 and 3 (the setting is off)"

    c["step"]("T170 setting off: every click opens a new buffer", twice)


if __name__ == "__main__":
    sys.exit(main())
