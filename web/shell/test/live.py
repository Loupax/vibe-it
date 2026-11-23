#!/usr/bin/env python3
"""C2 live check: browse a test page in web-shell with injected mouse and keyboard, inside
the nested dwl (web/tools/nest.sh), never on the user's display.

    web/shell/test/live.py --out .qwork/c2-wayland            # shell as a Wayland window
    web/shell/test/live.py --out .qwork/c2-x11 --x11          # shell as an X11 window (nested Xwayland)
    web/shell/test/live.py --out .qwork/c2-proxy --proxy      # also through a local forward proxy

The shell runs in a memory-capped systemd scope (it starts web-host-cef, so the scope covers
every CEF process) that is stopped at the end in any case. Pages come from test/pages on
127.0.0.1. Each step injects input (web/tools/inject) and waits for what it must cause:
page titles (the page reports clicks, typing, select, scroll, fullscreen in its title,
which the host sends as `title` events the shell logs), cursor changes and popups (the
shell's log), and the page server's request log. Screenshots of the test page go into
--out (never commit them). Exit 0 when every step passed.

Needs: web/shell/target/debug/web, web/host-cef/target/debug/web-host-cef (or --web,
--host-bin), the inject tool (--inject; build web/tools/inject), grim, dwl (and Xwayland
for --x11).

C6b: at a nest scale (`NEST_SCALE=2 web/shell/test/live.py ...`, passed on to nest.sh) the
coordinates stay logical: the output's logical size is grim's pixels over the nest's scale
(nest.sh's registry), and pixel checks on screenshots multiply by it."""

import argparse
import functools
import http.server
import os
import queue
import re
import shutil
import socketserver
import struct
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
SHELL = os.path.dirname(HERE)
WEB = os.path.dirname(SHELL)
BAR_BG = (24, 24, 24)


class Fail(Exception):
    pass


class Pages(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, log=None, **kw):
        self._log = log
        super().__init__(*a, **kw)

    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def log_message(self, fmt, *args):
        self._log.write(f"{time.strftime('%H:%M:%S')} {fmt % args}\n")
        self._log.flush()


class Server(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True


class Log:
    """Follows the shell's stderr; wait() finds a line matching a regex after a mark."""

    def __init__(self, proc, path):
        self.lines = []
        self.cv = threading.Condition()
        self.file = open(path, "w", buffering=1)
        threading.Thread(target=self._read, args=(proc,), daemon=True).start()

    def _read(self, proc):
        for raw in proc.stdout:
            line = raw.decode("utf-8", "replace").rstrip("\n")
            self.file.write(line + "\n")
            with self.cv:
                self.lines.append(line)
                self.cv.notify_all()

    def mark(self):
        with self.cv:
            return len(self.lines)

    def wait(self, pattern, since=0, timeout=15):
        rx = re.compile(pattern)
        end = time.monotonic() + timeout
        with self.cv:
            while True:
                for line in self.lines[since:]:
                    m = rx.search(line)
                    if m:
                        return m
                left = end - time.monotonic()
                if left <= 0:
                    raise Fail(f"timed out waiting for /{pattern}/")
                self.cv.wait(left)


def png_size(path):
    with open(path, "rb") as f:
        head = f.read(24)
    return struct.unpack(">II", head[16:24])


def shell_scale(lines):
    """C6b: the shell window's scale as the shell last logged it (`window WxH, page ..., scale S`)."""
    found = [re.search(r"\] window \d+x\d+, page .*, scale ([\d.]+)", line) for line in lines]
    found = [f for f in found if f]
    return float(found[-1][1]) if found else 1.0


def nest_scale():
    """C6b: the nested output's scale (NEST_SCALE, recorded by nest.sh in its registry).
    Screenshots are logical px times this, whatever the shell's own scale (an X11 shell sees
    1 and Xwayland scales its window up)."""
    reg = os.environ.get("NEST_REG") or os.path.join(os.path.dirname(WEB), ".qwork", "nest")
    try:
        with open(os.path.join(reg, "scale")) as f:
            return float(f.read().strip() or 1)
    except (OSError, ValueError):
        return 1.0


def output_size(png, scale):
    """The nested output's logical size (inject's coordinates) from a grim shot (pixels)."""
    pw, ph = png_size(png)
    return round(pw / scale), round(ph / scale)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--x11", action="store_true", help="run the shell as an X11 window (nested Xwayland)")
    ap.add_argument("--proxy", action="store_true", help="route the page through host-cef/test/fwd_proxy.py")
    ap.add_argument("--web", default=os.path.join(SHELL, "target/debug/web"))
    ap.add_argument("--host-bin", default=os.path.join(WEB, "host-cef/target/debug/web-host-cef"))
    ap.add_argument("--inject", required=True, help="the inject binary (build web/tools/inject)")
    a = ap.parse_args()
    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)
    # A fresh profile each run (a permission decided last time would skip the bubble).
    shutil.rmtree(os.path.join(out, "state"), ignore_errors=True)

    wl = subprocess.run([os.path.join(WEB, "tools/nest.sh")], check=True, capture_output=True, text=True).stdout.strip()
    nest_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    shell_env = dict(os.environ)
    shell_env.pop("WAYLAND_SOCKET", None)
    if a.x11:
        disp = subprocess.run([os.path.join(WEB, "tools/nest.sh"), "--x11-display"], check=True,
                              capture_output=True, text=True).stdout.strip()
        if not disp:
            raise SystemExit("the nested dwl has no Xwayland (DISPLAY unknown; try web/tools/nest.sh --new)")
        shell_env.pop("WAYLAND_DISPLAY", None)
        shell_env["DISPLAY"] = disp
        print(f"nested dwl {wl}, shell on its Xwayland {disp}")
    else:
        shell_env.pop("DISPLAY", None)
        shell_env["WAYLAND_DISPLAY"] = wl
        print(f"nested dwl {wl}, shell as a Wayland client")

    page_log = open(os.path.join(out, "pages.log"), "w", buffering=1)
    srv = Server(("127.0.0.1", 0), functools.partial(Pages, directory=os.path.join(HERE, "pages"), log=page_log))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_address[1]}"

    proxy = None
    web_args = ["--state-dir", os.path.join(out, "state"), "--host-bin", a.host_bin, f"{base}/c2.html"]
    if a.proxy:
        proxy_log = os.path.join(out, "proxy.log")
        proxy = subprocess.Popen([sys.executable, os.path.join(WEB, "host-cef/test/fwd_proxy.py"), "--log", proxy_log],
                                 stdout=subprocess.PIPE, text=True)
        port = proxy.stdout.readline().split(":")[-1].strip()
        web_args = ["--proxy", f"127.0.0.1:{port}"] + web_args + ["--", "--proxy-bypass-list=<-loopback>"]

    unit = f"web-c2-live-{os.getpid()}"
    cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
           "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web] + web_args
    proc = subprocess.Popen(cmd, env=shell_env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    log = Log(proc, os.path.join(out, "web.log"))
    results = []

    def shot(name):
        p = os.path.join(out, name)
        subprocess.run(["grim", p], env=nest_env, check=True)
        return p

    def step(name, fn):
        t0 = time.monotonic()
        try:
            detail = fn() or ""
            results.append((name, True, detail))
            print(f"  ok   {name} ({time.monotonic() - t0:.1f}s) {detail}")
        except Fail as e:
            results.append((name, False, str(e)))
            print(f"  FAIL {name}: {e}")

    try:
        log.wait(r"host event: .*\"title\":\"c2 ready:", timeout=60)
        log.wait(r"page window is (\d+)x(\d+)", timeout=10)
        time.sleep(0.5)
        # The last window size the shell logged is the one dwl gave it.
        sizes = [re.search(r"\] window (\d+)x(\d+), page", l) for l in log.lines]
        ww, wh = [int(x) for x in [s for s in sizes if s][-1].groups()]
        scale = nest_scale()
        ow, oh = output_size(shot("start.png"), scale)
        # dwl centres its single tiled window inside a border.
        bx, by = (ow - ww) // 2, (oh - wh) // 2
        print(f"output {ow}x{oh} at scale {scale} (the shell's {shell_scale(log.lines)}), shell window {ww}x{wh} at {bx},{by}")

        def inject(*cmds, fullscreen=False):
            ox, oy = (0, 0) if fullscreen else (bx, by)
            args = []
            for c in cmds:
                mm = re.fullmatch(r"move:(-?\d+),(-?\d+)", c)
                args.append(f"move:{int(mm[1]) + ox},{int(mm[2]) + oy}" if mm else c)
            subprocess.run([a.inject, str(ow), str(oh)] + args, env=nest_env, check=True)

        def title(pattern, since, timeout=10):
            return log.wait(r'host event: \{"event":"title","tab":\d+,"title":"' + pattern, since, timeout)

        def page_fills():
            last = [re.search(r"page window is (\d+)x(\d+)", l) for l in log.lines]
            pw, ph = [int(x) for x in [s for s in last if s][-1].groups()]
            if (pw, ph) != (ww, wh - 24):
                raise Fail(f"page window {pw}x{ph}, want {ww}x{wh - 24} (shell window minus the 24 px bar)")
            return f"page {pw}x{ph} in a {ww}x{wh} window"
        step("page fills the window minus the bar", page_fills)

        def desktop():
            kind = "an X11" if a.x11 else "a Wayland"
            log.wait(f"the shell is {kind} window on the desktop", 0)
            return f"{kind} window; the engine host speaks Wayland to the shell"
        step("shell window kind", desktop)

        def pairing():
            log.wait(r"page window paired with tab \d+|paired with tab \d+")
            return "tab-created paired with the host's toplevel"
        step("pairing", pairing)

        def click():
            k = log.mark()
            inject("move:172,72", "wait:150", "click", "wait:300")
            title("clicked:1", k)
        step("mouse click", click)

        def typing():
            k = log.mark()
            inject("move:240,172", "wait:150", "click", "wait:400")
            log.wait(r'"editable-focused","tab":\d+,"focused":true', k)
            inject("type:hello web", "wait:400")
            title("typed:hello web", k)
        step("click into a text field and type", typing)

        def cursors():
            k = log.mark()
            inject("move:172,272", "wait:500")
            log.wait(r"cursor: named pointer", k)
            k = log.mark()
            inject("move:630,120", "wait:500")
            log.wait(r"cursor: named crosshair", k)
            k = log.mark()
            inject("move:630,320", "wait:700")
            log.wait(r"cursor: surface", k)
            shot("cursor-image.png")
            k = log.mark()
            inject("move:600,%d" % (wh - 10), "wait:400")
            log.wait(r"cursor: named default", k)
            return "pointer, crosshair, image cursor drawn by the shell, default over the bar"
        step("cursor shapes", cursors)

        def select():
            k = log.mark()
            inject("move:172,372", "wait:150", "click", "wait:900")
            g = log.wait(r"popup at (-?\d+),(-?\d+) (\d+)x(\d+)", k)
            shot("select-open.png")
            inject("key:down", "wait:200", "key:ret", "wait:500")
            title("select:b", k)
            log.wait(r"popup closed", k)
            return f"popup at {g[1]},{g[2]} {g[3]}x{g[4]}, option b chosen with the keyboard"
        step("select popup", select)

        def scroll():
            k = log.mark()
            inject("move:600,600", "wait:100", "scroll:3", "wait:600")
            t = title(r"scrolled:(\d+)", k)
            if int(t[1]) <= 0:
                raise Fail("did not scroll down")
            k = log.mark()
            inject("scroll:-3", "wait:600")
            title("scrolled:0", k)
            return f"down to {t[1]} and back"
        step("wheel scroll", scroll)

        def select_right_edge():
            # C2-LIVE-REFRESH (the context menu this step used to open went with C-NOMENU): a <select>
            # flush with the right edge whose dropdown is wider than the select; the shell's popup
            # constraint must keep it inside the window. Opened twice: closed with Esc, and by a click outside.
            def open_it():
                k = log.mark()
                inject("move:%d,770" % (ww - 60), "wait:100", "click", "wait:900")
                return k, log.wait(r"popup at (-?\d+),(-?\d+) (\d+)x(\d+)", k)
            k, g = open_it()
            x, w = int(g[1]), int(g[3])
            if x < 0 or x + w > ww or w <= 268:
                inject("key:esc", "wait:400")  # close it, so the later steps still run
                if w <= 268:
                    raise Fail(f"dropdown {w}px wide is not wider than the select, so the constraint was not exercised")
                raise Fail(f"dropdown {x}..{x + w} not inside the window width {ww}")
            shot("select-right-edge.png")
            k = log.mark()
            inject("key:down", "wait:200", "key:ret", "wait:500")
            title("select2:r2", k)
            log.wait(r"popup closed", k)
            k, g2 = open_it()
            k = log.mark()
            inject("move:600,300", "wait:100", "click", "wait:600")
            log.wait(r"popup closed", k)
            return f"dropdown at {g[1]},{g[2]} {g[3]}x{g[4]} kept inside the {ww}px window; option chosen with the keyboard; a click outside closes it"
        step("select dropdown near the right edge", select_right_edge)

        def fullscreen():
            k = log.mark()
            inject("move:172,472", "wait:150", "click", "wait:1200")
            log.wait(r"fullscreen on \(page asked\)", k)
            t = title(r"fullscreen:true:(\d+)x(\d+)", k)
            fm = log.wait(r"page window is (%d)x(%d)" % (ow, oh), k)
            p = shot("fullscreen.png")
            detail = f"page {fm[1]}x{fm[2]}"
            try:
                from PIL import Image
                px = Image.open(p).convert("RGB").getpixel((int(ow * scale) // 2, int(oh * scale) - 3))
                if px == BAR_BG:
                    raise Fail("the bar is still drawn in fullscreen")
                detail += ", no bar"
            except ImportError:
                pass
            k = log.mark()
            inject("key:esc", "wait:1200", fullscreen=True)
            log.wait(r"fullscreen off", k)
            title("fullscreen:false", k)
            return detail + "; Esc left fullscreen"
        step("page fullscreen", fullscreen)

        def alert():
            k = log.mark()
            inject("move:172,572", "wait:150", "click", "wait:1000")
            shot("alert.png")
            inject("key:ret", "wait:600")
            title("alert:closed", k)
            return "the engine draws it inside the page window; Enter closed it"
        step("JS alert", alert)

        def permission():
            k = log.mark()
            inject("move:172,672", "wait:150", "click", "wait:1200")
            shot("permission.png")
            inject("move:231,145", "wait:200", "click", "wait:800")
            title("permission:denied", k)
            return "notification permission bubble (drawn inside the page window); Block clicked"
        step("permission bubble", permission)

        def form():
            k = log.mark()
            inject("move:400,172", "wait:150", "click", "wait:300", "key:end", "type: x", "key:ret", "wait:1200")
            log.wait(r'"url","tab":\d+,"url":"[^"]*/second\.html\?q=hello\+web\+x"', k)
            title("c2 second page", k)
            with open(os.path.join(out, "pages.log")) as f:
                if "GET /second.html?q=hello+web+x" not in f.read():
                    raise Fail("the page server did not see the form's request")
            return "typed text submitted with Enter; the server saw ?q=hello+web+x"
        step("form submit with Enter", form)

        def link():
            k = log.mark()
            inject("move:190,70", "wait:300", "click", "wait:1200")
            log.wait(r'"url","tab":\d+,"url":"[^"]*/c2\.html"', k)
            title("c2 ready", k)
            return "clicked a link back to the first page"
        step("follow a link", link)

        if a.proxy:
            def via_proxy():
                with open(os.path.join(out, "proxy.log")) as f:
                    text = f.read()
                if "/c2.html" not in text or "/second.html" not in text:
                    raise Fail("the proxy log lacks the page requests")
                return "the proxy log has the page's requests"
            step("--proxy passed through", via_proxy)
        else:
            step("no --proxy: direct", lambda: (log.wait(r"no --proxy: direct connections", 0), "")[1])
        def sigterm():
            k = log.mark()
            proc.send_signal(15)
            log.wait(r"quitting: SIGTERM", k)
            log.wait(r"engine host exited: exit status: 0", k)
            try:
                code = proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                raise Fail("the shell did not exit")
            if code != 0:
                raise Fail(f"the shell exited with {code}")
            return "the shell sent quit, the host exited 0, then the shell"
        step("quit on SIGTERM", sigterm)
    except Fail as e:
        results.append(("start", False, str(e)))
        print(f"  FAIL start: {e}")
    finally:
        subprocess.run(["systemctl", "--user", "stop", f"{unit}.scope"], capture_output=True)
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            pass
        if proxy:
            proxy.terminate()
            proxy.wait(timeout=5)
        srv.shutdown()

    failed = [r for r in results if not r[1]]
    print(f"{len(results) - len(failed)}/{len(results)} steps passed; log in {out}/web.log")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
