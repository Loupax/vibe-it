#!/usr/bin/env python3
"""C-CLOSE-POLITE live check (issue #757): closing web asks pages with `beforeunload`; "Stay" keeps
web running, "Leave" closes it with status 0; SIGTERM is a forced quit. Headless nested dwl
(web/tools/nest.sh), never the user's display; a PRIVATE D-Bus session bus
(web/tools/notify-probe/private-bus.conf), never the user's session bus.

    web/shell/test/c_close_polite_live.py --out .qwork/close-polite --inject .qwork/inject/release/inject

The window's close request is sent the way the desktop sends it: dwl's killclient binding (Logo+q
in the user's dwl config) makes dwl send xdg_toplevel.close, which winit reports as CloseRequested,
the same event the X button of a decorated window gives. The "Leave site?" dialog is answered with
the keyboard (Esc = "Stay", Enter = "Leave", Chrome's defaults) while the shell is in INPUT, so the
keys reach the page window that draws the dialog. Each run starts a fresh web on
test/pages/beforeunload.html, clicks into its field and types (Chromium shows the dialog only after
an interaction). Steps:
  1. window close, Stay: web keeps running 10 s, the page is intact, no `killed it`; window close
     again, Leave: the host exits, then web with status 0
  2. the same with SIGINT (Ctrl-C)
  3. the same with `:q` (typed in NORMAL after Esc Esc; `i` then returns to INPUT so the answer
     keys reach the dialog)
  4. two buffers, only the shown one (beforeunload.html) interacted with: window close, Stay: only
     that buffer remains
  5. SIGTERM: `terminating: SIGTERM`, the host logs `SIGTERM: forced quit` and answers the page's
     beforeunload itself (no dialog window), web exits 0
  6. no beforeunload page: the window close ends web quickly (time reported)
  7. SIGTERM while the dialog of a polite close is on screen: web exits 0 within ~2 s
Exit 0 when every step passed. Needs what test/live.py needs."""

import argparse
import functools
import os
import re
import shutil
import signal
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
# dwl's killclient (MODKEY Logo + q): Logo down, modifier state Mod4 (64), q, release.
CLOSE_WINDOW = ["kdown:125", "mods:64", "kdown:16", "kup:16", "mods:0", "kup:125", "wait:300"]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--web", default=os.path.join(SHELL, "target/debug/web"))
    ap.add_argument("--host-bin", default=os.path.join(WEB, "host-cef/target/debug/web-host-cef"))
    ap.add_argument("--inject", required=True, help="the inject binary (build web/tools/inject)")
    ap.add_argument("--only", default="", help="comma-separated step numbers to run (default: all)")
    a = ap.parse_args()
    if not os.environ.get("C_CLOSE_POLITE_PRIVATE_BUS"):
        env = dict(os.environ, C_CLOSE_POLITE_PRIVATE_BUS="1")
        conf = os.path.join(WEB, "tools/notify-probe/private-bus.conf")
        os.execvpe("dbus-run-session", ["dbus-run-session", f"--config-file={conf}", "--", sys.executable, "-I"] + sys.argv, env)
    bus = os.environ.get("DBUS_SESSION_BUS_ADDRESS", "")
    if not bus or re.search(r"/run/user/\d+/bus", bus):
        print(f"refusing: not a private bus ({bus!r})", file=sys.stderr)
        return 2
    only = {s for s in a.only.split(",") if s}

    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)
    state = os.path.join(out, "state")
    shutil.rmtree(state, ignore_errors=True)
    os.makedirs(state, mode=0o700)
    config = os.path.join(out, "config")
    os.makedirs(config, exist_ok=True)

    wl = subprocess.run([os.path.join(WEB, "tools/nest.sh")], check=True, capture_output=True, text=True).stdout.strip()
    nest_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    shell_env = dict(os.environ, WAYLAND_DISPLAY=wl, XDG_CONFIG_HOME=config)
    shell_env.pop("WAYLAND_SOCKET", None)
    shell_env.pop("DISPLAY", None)

    page_log = open(os.path.join(out, "pages.log"), "w", buffering=1)
    srv = live.Server(("127.0.0.1", 0), functools.partial(live.Pages, directory=os.path.join(HERE, "pages"), log=page_log))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_address[1]}"
    print(f"nested dwl {wl}; pages at {base}", flush=True)

    results = []
    units = []

    class Run:
        """One web, started in its own memory-capped scope."""

        def __init__(self, tag, url):
            self.tag = tag
            self.unit = f"web-close-polite-live-{os.getpid()}-{tag}"
            units.append(self.unit)
            cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={self.unit}", "-p", "MemoryMax=6G",
                   "-p", "MemorySwapMax=0", "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web,
                   "--state-dir", state, "--host-bin", a.host_bin, url]
            self.proc = subprocess.Popen(cmd, env=shell_env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            self.log = live.Log(self.proc, os.path.join(out, f"web-{tag}.log"))
            self.geo = None

        def pid(self):
            with open(f"/proc/{self.proc.pid}/comm") as f:
                comm = f.read().strip()
            if comm != "web":
                raise Fail(f"pid {self.proc.pid} is {comm!r}, not web")
            return self.proc.pid

        def shot(self, name):
            p = os.path.join(out, f"{self.tag}-{name}")
            subprocess.run(["grim", p], env=nest_env, check=True)
            return p

        def settle(self):
            self.log.wait(r'"event":"title","tab":\d+,"title":"', timeout=90)
            self.geo = livelib.settle(self.log, self.shot)

        def inject(self, *cmds):
            g = self.geo
            args = []
            for c in cmds:
                mm = re.fullmatch(r"move:(-?\d+),(-?\d+)", c)
                args.append(f"move:{int(mm[1]) + g.bx},{int(mm[2]) + g.by}" if mm else c)
            subprocess.run([a.inject, str(g.ow), str(g.oh)] + args, env=nest_env, check=True)

        def title(self, pattern, since=0, timeout=5):
            return self.log.wait(r'"event":"title","tab":\d+,"title":"' + pattern, since, timeout)

        def interact(self):
            """Click into the field and type: INPUT, and the page counts as interacted with."""
            k = self.log.mark()
            for y in (150, 160):
                self.inject(f"move:200,{y}", "wait:150", "click", "wait:600")
                try:
                    self.title("beforeunload: focused", k, timeout=3)
                    break
                except Fail:
                    continue
            else:
                raise Fail("the click did not focus the field")
            self.inject(*(keys_for("x") + ["wait:400"]))
            self.title("beforeunload: typed x", k)
            self.log.wait(r"\] bar: INPUT \|", k)

        def alive_for(self, secs):
            end = time.monotonic() + secs
            while time.monotonic() < end:
                if self.proc.poll() is not None:
                    raise Fail(f"web exited ({self.proc.returncode}) while the page stayed")
                time.sleep(0.2)

        def exit_code(self, timeout):
            try:
                return self.proc.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                raise Fail(f"web did not exit within {timeout} s")

        def lines(self, since=0):
            return self.log.lines[since:]

        def stop(self):
            subprocess.run(["systemctl", "--user", "stop", f"{self.unit}.scope"], capture_output=True)
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                pass

    def step(num, name, fn):
        if only and num not in only:
            return
        t0 = time.monotonic()
        try:
            detail = fn() or ""
            results.append((name, True, detail))
            print(f"  ok   {num} {name} ({time.monotonic() - t0:.1f}s) {detail}", flush=True)
        except Fail as e:
            results.append((name, False, str(e)))
            print(f"  FAIL {num} {name}: {e}", flush=True)

    def no_kill(r):
        if any("killed it" in l for l in r.lines()):
            raise Fail("the shell killed the host")

    def stay_then_leave(r, close, label):
        """close() asks politely; Esc answers Stay, then close() again and Enter answers Leave."""
        k = r.log.mark()
        close()
        r.log.wait(r"closing politely: " + label, k)
        r.log.wait(r"close-all: asking 1 page\(s\) to close", k)
        time.sleep(1.0)
        r.shot("dialog-1.png")
        r.inject("key:esc", "wait:500")
        r.alive_for(10)
        if any('"event":"tab-closed"' in l for l in r.lines(k)):
            raise Fail("a tab closed although the user stayed")
        # Still the same document: no load or url event since the close.
        moved = [l for l in r.lines(k) if re.search(r'host event: \{"event":"(load|url)"', l)]
        if moved:
            raise Fail(f"the page navigated after Stay: {moved}")
        no_kill(r)
        r.shot("stayed.png")
        k2 = r.log.mark()
        close()
        r.log.wait(r"closing politely: " + label, k2)
        r.log.wait(r"close-all: asking 1 page\(s\) to close", k2)
        time.sleep(1.0)
        r.shot("dialog-2.png")
        t = time.monotonic()
        r.inject("key:ret", "wait:200")
        code = r.exit_code(20)
        took = time.monotonic() - t
        no_kill(r)
        r.log.wait(r'"event":"tab-closed"', k2, 1)
        r.log.wait(r"engine host exited: exit status: 0", k2, 1)
        if code != 0:
            raise Fail(f"web exited with {code}")
        return f"Stay: alive 10 s, page intact; Leave: exit 0 {took:.1f}s after Enter"

    def s1():
        r = Run("x", f"{base}/beforeunload.html")
        try:
            r.settle()
            r.interact()
            return stay_then_leave(r, lambda: r.inject(*CLOSE_WINDOW), "shell window closed")
        finally:
            r.stop()
    step("1", "window close: Stay keeps web, Leave exits 0", s1)

    def s2():
        r = Run("sigint", f"{base}/beforeunload.html")
        try:
            r.settle()
            r.interact()
            return stay_then_leave(r, lambda: os.kill(r.pid(), signal.SIGINT), "SIGINT")
        finally:
            r.stop()
    step("2", "SIGINT: Stay keeps web, Leave exits 0", s2)

    def s3():
        r = Run("q", f"{base}/beforeunload.html")
        try:
            r.settle()
            r.interact()

            def q():
                # Esc Esc to NORMAL, `:q`, then `i` back to INPUT so the answer keys reach the page.
                k = r.log.mark()
                r.inject("key:esc", "wait:80", "key:esc", "wait:400")
                r.log.wait(r"\] bar: NORMAL \|", k)
                r.inject(*(keys_for(":q") + ["key:ret", "wait:600"]))
                r.inject("key:i", "wait:400")
                r.log.wait(r"\] bar: INPUT \|", k)

            return stay_then_leave(r, q, ":q")
        finally:
            r.stop()
    step("3", ":q: Stay keeps web, Leave exits 0", s3)

    def s4():
        r = Run("two", f"{base}/second.html")
        try:
            r.settle()
            k = r.log.mark()
            r.inject(*(keys_for(f":new {base}/beforeunload.html") + ["key:ret", "wait:800"]))
            r.title("beforeunload", k, timeout=15)
            time.sleep(0.5)
            r.interact()
            k = r.log.mark()
            r.inject(*CLOSE_WINDOW)
            r.log.wait(r"closing politely: shell window closed", k)
            r.log.wait(r"close-all: asking 2 page\(s\) to close", k)
            r.log.wait(r'"event":"tab-closed"', k)
            time.sleep(1.0)
            r.shot("dialog.png")
            r.inject("key:esc", "wait:500")
            r.alive_for(5)
            closed = [l for l in r.lines(k) if '"event":"tab-closed"' in l]
            if len(closed) != 1:
                raise Fail(f"{len(closed)} tab-closed events, wanted 1: {closed}")
            r.inject("key:esc", "wait:80", "key:esc", "wait:400")
            k2 = r.log.mark()
            r.inject(*(keys_for(":ls") + ["key:ret", "wait:800"]))
            r.shot("ls.png")
            r.inject("key:esc", "wait:400")
            no_kill(r)
            bars = [l for l in r.lines(k) if "] bar: " in l]
            return f"one tab-closed (second.html), web alive; last bar lines: {bars[-2:]}"
        finally:
            r.stop()
    step("4", "two buffers: Stay keeps only the beforeunload buffer", s4)

    def s5():
        r = Run("term", f"{base}/beforeunload.html")
        try:
            r.settle()
            r.interact()
            k = r.log.mark()
            t = time.monotonic()
            os.kill(r.pid(), signal.SIGTERM)
            code = r.exit_code(20)
            took = time.monotonic() - t
            r.log.wait(r"terminating: SIGTERM", k, 1)
            r.log.wait(r"SIGTERM: forced quit", k, 1)
            no_kill(r)
            if code != 0:
                raise Fail(f"web exited with {code}")
            r.log.wait(r"beforeunload answered Leave without a dialog", k, 1)
            if any("dialog window" in l for l in r.lines(k)):
                raise Fail("a dialog window was shown on the forced quit")
            if any("did not close; ending anyway" in l for l in r.lines(k)):
                raise Fail("the page did not close on the forced quit (the grace timer ended it)")
            return f"exit 0 {took:.1f}s after SIGTERM; the host answered beforeunload itself, no dialog"
        finally:
            r.stop()
    step("5", "SIGTERM: forced quit, exit 0", s5)

    def s7():
        r = Run("termdlg", f"{base}/beforeunload.html")
        try:
            r.settle()
            r.interact()
            k = r.log.mark()
            r.inject(*CLOSE_WINDOW)
            r.log.wait(r"close-all: asking 1 page\(s\) to close", k)
            r.log.wait(r"dialog window", k)
            time.sleep(1.0)
            r.shot("dialog.png")
            t = time.monotonic()
            os.kill(r.pid(), signal.SIGTERM)
            code = r.exit_code(20)
            took = time.monotonic() - t
            r.log.wait(r"SIGTERM: forced quit", k, 1)
            no_kill(r)
            if code != 0:
                raise Fail(f"web exited with {code}")
            grace = any("did not close; ending anyway" in l for l in r.lines(k))
            return (f"exit 0 {took:.1f}s after SIGTERM; "
                    f"{'the open dialog held the page: the host ended after its 2 s grace' if grace else 'the page closed'}")
        finally:
            r.stop()
    step("7", "SIGTERM while a polite close's dialog is open: exit 0 within ~2 s", s7)

    def s8():
        # Informational: the beforeunload page is NOT the buffer on screen. Its dialog is a toplevel
        # of its own; the screenshot shows whether the user can see it. Ended with SIGTERM.
        r = Run("hidden", f"{base}/beforeunload.html")
        try:
            r.settle()
            r.interact()
            r.inject("key:esc", "wait:80", "key:esc", "wait:400")
            k = r.log.mark()
            r.inject(*(keys_for(f":new {base}/second.html") + ["key:ret", "wait:1500"]))
            r.log.wait(r'"event":"tab-created"', k, 10)
            k = r.log.mark()
            r.inject(*CLOSE_WINDOW)
            r.log.wait(r"close-all: asking 2 page\(s\) to close", k)
            time.sleep(1.5)
            shot = r.shot("dialog.png")
            dialog = any("dialog window" in l for l in r.lines(k))
            closed = len([l for l in r.lines(k) if '"event":"tab-closed"' in l])
            if r.proc.poll() is not None:
                raise Fail("web exited although the hidden page asked")
            os.kill(r.pid(), signal.SIGTERM)
            code = r.exit_code(20)
            return (f"dialog window logged: {dialog}; {closed} tab-closed; screenshot {shot}; "
                    f"SIGTERM then: exit {code}")
        finally:
            r.stop()
    step("8", "(info) the beforeunload page is a hidden buffer", s8)

    def s6():
        r = Run("plain", f"{base}/second.html")
        try:
            r.settle()
            k = r.log.mark()
            t = time.monotonic()
            r.inject(*CLOSE_WINDOW)
            code = r.exit_code(20)
            took = time.monotonic() - t
            r.log.wait(r"closing politely: shell window closed", k, 1)
            if code != 0:
                raise Fail(f"web exited with {code}")
            return f"exit 0 {took:.1f}s after the close request"
        finally:
            r.stop()
    step("6", "no beforeunload page: the window close ends web quickly", s6)

    for u in units:
        subprocess.run(["systemctl", "--user", "stop", f"{u}.scope"], capture_output=True)
    srv.shutdown()
    failed = [r for r in results if not r[1]]
    print(f"{len(results) - len(failed)}/{len(results)} steps passed; logs in {out}", flush=True)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
