#!/usr/bin/env python3
"""C6 live check: the clipboard bridge between the page and the desktop, inside the headless
nested dwl (web/tools/nest.sh), never on the user's display or clipboard.

    web/shell/test/c6_live.py --out .qwork/c6 --inject .qwork/inject/release/inject [--x11]

"The desktop" is the nested dwl: its clipboard is set and read with wl-copy / wl-paste on the
nested WAYLAND_DISPLAY (data-control, so they take no focus), or with xsel on the nested
Xwayland with --x11 (the shell is then an X11 window). The page (pages/c6.html) has a
textarea and reports its value in its title. What happened is read from the shell's log (the
status bar as `bar: ...`, the bridge as `clipboard: ...`, every host event). Each step names
the KEYS.md rows it shows. Exit 0 when every step passed. Needs what c3_live.py needs."""

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
sys.dont_write_bytecode = True
import live  # noqa: E402
from c3_live import keys_for  # noqa: E402

SHELL = live.SHELL
WEB = live.WEB
Fail = live.Fail
BAR = r"\] bar: "


class DesktopClipboard:
    """The nested desktop's clipboard, as the test controls it."""

    def __init__(self, x11, nest_env, x_env):
        self.x11, self.nest_env, self.x_env = x11, nest_env, x_env
        self.procs = []

    def set(self, text, primary=False):
        if text == "":
            if self.x11:
                subprocess.run(["xsel", "-p" if primary else "-b", "-c"], env=self.x_env, check=True)
            else:
                subprocess.run(["wl-copy", "--clear"] + (["-p"] if primary else []), env=self.nest_env, check=True)
            time.sleep(0.4)
            return
        if self.x11:
            p = subprocess.Popen(["xsel", "-p" if primary else "-b", "-i", "-n"], stdin=subprocess.PIPE, env=self.x_env)
            p.stdin.write(text.encode())
            p.stdin.close()
        else:
            p = subprocess.Popen(["wl-copy", "--foreground"] + (["-p"] if primary else []) + ["--", text],
                                 env=self.nest_env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.procs.append(p)
        time.sleep(0.4)

    def get(self, primary=False):
        if self.x11:
            cmd = ["xsel", "-p" if primary else "-b", "-o"]
            env = self.x_env
        else:
            cmd = ["wl-paste", "-n"] + (["-p"] if primary else [])
            env = self.nest_env
        r = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=5)
        return r.stdout

    def wait_for(self, want, primary=False, timeout=5):
        end = time.monotonic() + timeout
        got = None
        while time.monotonic() < end:
            got = self.get(primary)
            if got == want:
                return got
            time.sleep(0.2)
        raise Fail(f"the desktop's {'primary' if primary else 'clipboard'} holds {got!r}, want {want!r}")

    def stop(self):
        for p in self.procs:
            if p.poll() is None:
                p.terminate()
                try:
                    p.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    p.kill()


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--x11", action="store_true", help="run the shell as an X11 window (nested Xwayland)")
    ap.add_argument("--web", default=os.path.join(SHELL, "target/debug/web"))
    ap.add_argument("--host-bin", default=os.path.join(WEB, "host-cef/target/debug/web-host-cef"))
    ap.add_argument("--inject", required=True, help="the inject binary (build web/tools/inject)")
    a = ap.parse_args()
    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)
    state = os.path.join(out, "state")
    shutil.rmtree(state, ignore_errors=True)

    wl = subprocess.run([os.path.join(WEB, "tools/nest.sh")], check=True, capture_output=True, text=True).stdout.strip()
    nest_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    nest_env.pop("DISPLAY", None)
    shell_env = dict(os.environ)
    shell_env.pop("WAYLAND_SOCKET", None)
    x_env = None
    if a.x11:
        disp = subprocess.run([os.path.join(WEB, "tools/nest.sh"), "--x11-display"], check=True,
                              capture_output=True, text=True).stdout.strip()
        if not disp:
            raise SystemExit("the nested dwl has no Xwayland (DISPLAY unknown; try web/tools/nest.sh --new)")
        shell_env.pop("WAYLAND_DISPLAY", None)
        shell_env["DISPLAY"] = disp
        x_env = dict(os.environ, DISPLAY=disp)
        x_env.pop("WAYLAND_DISPLAY", None)
        print(f"nested dwl {wl}, shell on its Xwayland {disp}, clipboard through xsel")
    else:
        shell_env.pop("DISPLAY", None)
        shell_env["WAYLAND_DISPLAY"] = wl
        print(f"nested dwl {wl}, shell as a Wayland client, clipboard through wl-copy/wl-paste")
    desk = DesktopClipboard(a.x11, nest_env, x_env)

    page_log = open(os.path.join(out, "pages.log"), "w", buffering=1)
    srv = live.Server(("127.0.0.1", 0), functools.partial(live.Pages, directory=os.path.join(HERE, "pages"), log=page_log))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_address[1]}"
    page_url = f"{base}/c6.html"

    unit = f"web-c6-live-{os.getpid()}"
    cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
           "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", state, "--host-bin", a.host_bin, page_url]
    proc = subprocess.Popen(cmd, env=shell_env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    log = live.Log(proc, os.path.join(out, "web.log"))
    results = []

    def step(name, fn):
        t0 = time.monotonic()
        try:
            detail = fn() or ""
            results.append((name, True, detail))
            print(f"  ok   {name} ({time.monotonic() - t0:.1f}s) {detail}")
        except Fail as e:
            results.append((name, False, str(e)))
            print(f"  FAIL {name}: {e}")

    def shot(name):
        p = os.path.join(out, name)
        subprocess.run(["grim", p], env=nest_env, check=True)
        return p

    try:
        log.wait(r'"title":"c6 v=copied in the page 7"', timeout=60)
        log.wait(r"page window is (\d+)x(\d+)", timeout=10)
        time.sleep(0.5)
        sizes = [re.search(r"\] window (\d+)x(\d+), page", l) for l in log.lines]
        ww, wh = [int(x) for x in [s for s in sizes if s][-1].groups()]
        ow, oh = live.png_size(shot("start.png"))
        bx, by = (ow - ww) // 2, (oh - wh) // 2

        def inject(*cmds):
            args = []
            for c in cmds:
                mm = re.fullmatch(r"move:(-?\d+),(-?\d+)", c)
                args.append(f"move:{int(mm[1]) + bx},{int(mm[2]) + by}" if mm else c)
            subprocess.run([a.inject, str(ow), str(oh)] + args, env=nest_env, check=True)

        def bar(pattern, since, timeout=6):
            return log.wait(BAR + pattern, since, timeout)

        def title(value, since, timeout=6):
            return log.wait(r'"title":"c6 v=' + re.escape(value) + '"', since, timeout)

        def chord(mods, key):
            return [f"hold:{mods}", f"key:{key}", "release", "wait:300"]

        def bar_now():
            """The last status bar state logged (the shell logs only changes)."""
            for l in reversed(log.lines):
                if "] bar: " in l:
                    return l.split("bar: ", 1)[1]
            return ""

        def to_normal():
            inject("key:esc", "wait:60", "key:esc", "wait:400")
            if not bar_now().startswith("NORMAL | "):
                raise Fail(f"Esc Esc left the bar at {bar_now()!r}")

        def into_field():
            # From NORMAL, a click on the textarea: the page focuses it, the shell switches to
            # INPUT (T29); a field already focused gets `i`.
            to_normal()
            k = log.mark()
            inject("move:200,80", "wait:150", "click", "wait:600")
            if not bar_now().startswith("INPUT | "):
                inject("key:i", "wait:300")
            bar(r"INPUT \| ", k)

        def page_to_desktop():
            desk.set("something else")
            into_field()
            k = log.mark()
            inject(*chord("c", "a"), *chord("c", "c"))
            log.wait(r"clipboard: page clipboard \(\d+ bytes\) to the desktop", k)
            desk.wait_for("copied in the page 7")
            return "Ctrl-A Ctrl-C in the page's textarea: the desktop's clipboard (wl-paste/xsel) holds the text"
        step("page -> desktop: a copy in the page reaches the desktop clipboard", page_to_desktop)

        def desktop_to_line():
            got = []
            for i, (mods, key) in enumerate([("c", "v"), ("cs", "v"), ("s", "ins")]):
                text = f"desk{i}.example/x"
                desk.set(text + "\n")
                to_normal()
                k = log.mark()
                inject(*keys_for(":go "), "wait:100", *chord(mods, key))
                bar(re.escape(":go " + text) + r"\|", k)
                got.append(f"{'+'.join({'c': 'Ctrl', 's': 'Shift'}[m] for m in mods)}-{key}")
                if i == 0:
                    shot("pasted-into-line.png")
                inject("key:esc", "wait:300")
            return f"{', '.join(got)} in the `:` line inserted the desktop's text, newline dropped (T26, Paste)"
        step("desktop -> `:` line: Ctrl-v, Ctrl-Shift-v, Shift-Insert (T26)", desktop_to_line)

        def desktop_to_search():
            desk.set("needle")
            to_normal()
            k = log.mark()
            inject("key:slash", "wait:200", *chord("c", "v"))
            bar(r"/needle\|", k)
            inject("key:esc", "wait:300")
            return "Ctrl-v in the `/` line inserted the desktop's text (T22)"
        step("desktop -> `/` line (T22)", desktop_to_search)

        def desktop_to_page():
            desk.set("pasted from the desktop 9")
            into_field()
            k = log.mark()
            inject(*chord("c", "a"), *chord("c", "v"))
            title("pasted from the desktop 9", k)
            log.wait(r"clipboard: desktop clipboard \(\d+ bytes\) offered to the page", k)
            shot("pasted-into-page.png")
            return "Ctrl-A Ctrl-V in the textarea: the page's field holds the desktop's text (offered on the paste key)"
        step("desktop -> page: Ctrl-v in INPUT pastes the desktop clipboard", desktop_to_page)

        def page_to_line():
            # The vice versa through the `:` line: what the page copied, pasted in the shell.
            into_field()
            inject(*chord("c", "a"), *keys_for("page word"), "wait:300")
            inject(*chord("c", "a"), *chord("c", "c"))
            desk.wait_for("page word")
            to_normal()
            k = log.mark()
            inject(*keys_for(":"), "wait:200", *chord("c", "v"))
            bar(re.escape(":page word") + r"\|", k)
            inject("key:esc", "wait:300")
            return "typed 'page word' in the page, copied it there, Ctrl-v in the `:` line shows it"
        step("page -> `:` line", page_to_line)

        def yy():
            desk.set("old")
            to_normal()
            k = log.mark()
            inject("key:y", "wait:100", "key:y", "wait:400")
            bar(r"NORMAL \| Copied to clipboard", k)
            desk.wait_for(page_url)
            return f"yy: 'Copied to clipboard', the desktop holds {page_url}"
        step("T54 T55: yy copies the URL", yy)

        def let_plus():
            desk.set("old")
            k = log.mark()
            inject(*keys_for(":let @+ = @%"), "key:ret", "wait:400")
            bar(r"NORMAL \| Copied to clipboard", k)
            desk.wait_for(page_url)
            desk.set("")
            k = log.mark()
            inject(*keys_for(":let @+ = @+"), "key:ret", "wait:400")
            bar(r"NORMAL \| web: the clipboard is empty", k)
            desk.set("kept")
            k = log.mark()
            inject(*keys_for(":let @+ = @+"), "key:ret", "wait:400")
            bar(r"NORMAL \| Copied to clipboard", k)
            desk.wait_for("kept")
            return ":let @+ = @% copies the URL; :let @+ = @+ reads @+ ('the clipboard is empty' when it is)"
        step("Registers: :let @+ = @%, @+ readable", let_plus)

        def primary():
            # The primary selection: a selection in the page reaches the desktop's primary; the
            # desktop's primary is offered to the page on a middle click, and Chromium pastes it into
            # the field (C6f: the host runs with --ui-toolkit=fallback, because GTK's
            # gtk-enable-primary-paste, often false, makes Blink skip middle-click paste).
            into_field()
            k = log.mark()
            inject(*chord("c", "a"), *keys_for("prim one"), "wait:200", *chord("c", "a"), "wait:400")
            log.wait(r"clipboard: page primary selection \(\d+ bytes\) to the desktop", k, timeout=6)
            desk.wait_for("prim one", primary=True)
            desk.set("from primary", primary=True)
            k = log.mark()
            inject("key:end", "wait:100", "move:300,100", "wait:150", "mclick", "wait:800")
            log.wait(r"clipboard: desktop primary selection \(\d+ bytes\) offered to the page", k)
            title("prim onefrom primary", k, timeout=6)
            return ("select-all in the page set the desktop's primary (wl-paste -p); a middle click in the field "
                    "pasted the desktop's primary selection at the caret")
        step("primary selection both ways", primary)

    except Fail as e:
        results.append(("start", False, str(e)))
        print(f"  FAIL start: {e}")
    finally:
        subprocess.run(["systemctl", "--user", "stop", f"{unit}.scope"], capture_output=True)
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            pass
        desk.stop()
        srv.shutdown()

    failed = [r for r in results if not r[1]]
    print(f"{len(results) - len(failed)}/{len(results)} steps passed; log in {out}/web.log")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
