#!/usr/bin/env python3
"""C-YANK live check: `y` / `:yank` copy the page's selection to the desktop clipboard (KEYS.md T54,
T183-T186; PROTOCOL.md "Selection"), inside the headless nested dwl (web/tools/nest.sh), never on the
user's display or clipboard.

    web/shell/test/c_yank_live.py --out .qwork/cyank --inject .qwork/inject/release/inject

The page (pages/cyank.html) has buttons that make a selection with a script (`selectAllChildren`, or
`setSelectionRange` on a focused field) and report in the title what the page itself reads as selected;
the test presses `y` and reads the nested desktop's clipboard with wl-paste (WAYLAND_DISPLAY = the
nest's socket). XDG_CONFIG_HOME is a temporary dir under --out: there is no init.js until the mapping
steps write one (`web.map("n", "yy", ":let @+ = @%<CR>")`) and `:source` it. Steps marked (observed)
only report what happened. Run it with a private D-Bus (`dbus-run-session
--config-file=web/tools/notify-probe/private-bus.conf -- ...`). Exit 0 when every step passed."""

import argparse
import functools
import os
import re
import shutil
import subprocess
import sys
import threading
import time
import urllib.parse

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True
import live  # noqa: E402
import livelib  # noqa: E402
from c3_live import keys_for  # noqa: E402
from c6_live import DesktopClipboard  # noqa: E402

SHELL = live.SHELL
WEB = live.WEB
Fail = live.Fail
BAR = r"\] bar: "
YY_MAP = 'web.map("n", "yy", ":let @+ = @%<CR>");\n'
MAP_TIMEOUT_MS = 1000  # config.rs (C-MAPTIMEOUT has not landed)

# Button centres in the page (pages/cyank.html: fixed at left 10, top 10 + 40 i, 180x30).
BUTTONS = {name: (100, 25 + 40 * i) for i, name in enumerate(["para", "first", "input", "area", "pass", "fake", "none"])}
FRAME_BUTTON = (220 + 1 + 80, 420 + 1 + 20)  # the iframe's button (left 5, top 5, 150x30)
EMPTY = (700, 700)  # page background, nothing there


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--web", default=os.path.join(SHELL, "target/debug/web"))
    ap.add_argument("--host-bin", default=os.path.join(WEB, "host-cef/target/debug/web-host-cef"))
    ap.add_argument("--inject", required=True, help="the inject binary (build web/tools/inject)")
    a = ap.parse_args()
    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)
    state = os.path.join(out, "state")
    shutil.rmtree(state, ignore_errors=True)
    os.makedirs(state, mode=0o700)
    config = os.path.join(out, "config")
    shutil.rmtree(config, ignore_errors=True)
    profile = os.path.join(config, "web", "profiles", "default")
    os.makedirs(profile)

    wl = subprocess.run([os.path.join(WEB, "tools/nest.sh")], check=True, capture_output=True, text=True).stdout.strip()
    nest_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    nest_env.pop("DISPLAY", None)
    shell_env = dict(os.environ, WAYLAND_DISPLAY=wl, XDG_CONFIG_HOME=config)
    shell_env.pop("WAYLAND_SOCKET", None)
    shell_env.pop("DISPLAY", None)
    desk = DesktopClipboard(False, nest_env, None)

    page_log = open(os.path.join(out, "pages.log"), "w", buffering=1)
    srv = live.Server(("127.0.0.1", 0), functools.partial(live.Pages, directory=os.path.join(HERE, "pages"), log=page_log))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    page_url = f"http://127.0.0.1:{srv.server_address[1]}/cyank.html"

    unit = f"web-cyank-live-{os.getpid()}"
    cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
           "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", state, "--host-bin", a.host_bin, page_url]
    proc = subprocess.Popen(cmd, env=shell_env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    log = live.Log(proc, os.path.join(out, "web.log"))
    results = []
    undecided = {}

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
        log.wait(r'"title":"cy start"', timeout=60)
        geo = livelib.settle(log, shot)
        ow, oh, bx, by = geo.ow, geo.oh, geo.bx, geo.by

        def inject(*cmds):
            args = []
            for c in cmds:
                mm = re.fullmatch(r"move:(-?\d+),(-?\d+)", c)
                args.append(f"move:{int(mm[1]) + bx},{int(mm[2]) + by}" if mm else c)
            subprocess.run([a.inject, str(ow), str(oh)] + args, env=nest_env, check=True)

        def bar(pattern, since, timeout=6):
            return log.wait(BAR + pattern, since, timeout)

        def bar_now():
            for l in reversed(log.lines):
                if "] bar: " in l:
                    return l.split("bar: ", 1)[1]
            return ""

        def to_normal():
            if not bar_now().startswith("NORMAL"):
                inject("key:esc", "wait:60", "key:esc", "wait:400")
            if not bar_now().startswith("NORMAL"):
                raise Fail(f"Esc Esc left the bar at {bar_now()!r}")

        def click(xy):
            inject(f"move:{xy[0]},{xy[1]}", "wait:150", "click", "wait:300")

        def select(name, xy=None):
            """Click button `name` (or the point `xy`); returns the text the page reports as selected."""
            to_normal()
            k = log.mark()
            click(xy or BUTTONS[name])
            m = log.wait(r'"title":"cy ' + name + r' n=\d+ sel=([^"]*)"', k)
            return urllib.parse.unquote(m[1])

        def sends(since, what="selection"):
            return [l for l in log.lines[since:] if re.search(r'\[web \+\d+ms\] command: \{"cmd":"' + what + '"', l)]

        def yank(keys=("key:y",)):
            """`y` (or `keys`) in NORMAL; returns the bar's message once the host's answer came (the
            shell logs the bar only when it changes, so the same message twice is read from the
            last bar line)."""
            to_normal()
            k = log.mark()
            inject(*keys, "wait:300")
            log.wait(r"\] host event: selection \d+ \d+ \d+ chars", k)
            if not sends(k):
                raise Fail("no selection command sent")
            time.sleep(0.3)
            m = re.match(r"NORMAL \| (Copied to clipboard|web: [^|]*)$", bar_now())
            if not m:
                raise Fail(f"the bar shows {bar_now()!r}")
            return m[1]

        def expect_copy(want, keys=("key:y",)):
            desk.set("old")
            msg = yank(keys)
            if msg != "Copied to clipboard":
                raise Fail(f"the bar says {msg!r}, want 'Copied to clipboard'")
            desk.wait_for(want)

        # The first click into the page window activates it and may not reach the page: one on the
        # empty background first.
        click(EMPTY)

        def para():
            want = select("para")
            if "\n" not in want:
                raise Fail(f"the page's selection {want!r} has no newline across the two blocks")
            expect_copy(want)
            return f"two <p> selected, y: the desktop holds {want!r} (newline kept)"
        step("T54: y copies the paragraph text across two blocks", para)

        def command_yank():
            want = select("first")
            expect_copy(want, keys=keys_for(":yank") + ["key:ret"])
            return f":yank Enter: the desktop holds {want!r}"
        step("T183: :yank copies the selection", command_yank)

        def input_part():
            want = select("input")
            expect_copy(want)
            return f"focused <input>, setSelectionRange(6, 11), Esc Esc, y: {want!r}"
        step("T54: an <input>'s selected part only", input_part)

        def area_part():
            want = select("area")
            if want != "one\nline two":
                raise Fail(f"the page reports {want!r}")
            expect_copy(want)
            return f"focused <textarea>, setSelectionRange(5, 17), Esc Esc, y: {want!r}"
        step("T54: a <textarea>'s selected part only", area_part)

        def password():
            select("pass")
            desk.set("old")
            msg = yank()
            if msg != "web: nothing selected":
                raise Fail(f"the bar says {msg!r}")
            time.sleep(0.3)
            if desk.get() != "old":
                raise Fail(f"the clipboard changed to {desk.get()!r}")
            if any("secret123" in l for l in log.lines):
                raise Fail("the password is in the log")
            return "focused password field, all selected, y: 'web: nothing selected', clipboard kept"
        step("T185: a password field gives nothing", password)

        def nothing():
            select("none")
            desk.set("old")
            msg = yank()
            if msg != "web: nothing selected":
                raise Fail(f"the bar says {msg!r}")
            return "no selection, y: 'web: nothing selected'"
        step("T185: nothing selected", nothing)

        def search_open(query):
            k = log.mark()
            inject("key:slash", "wait:150", *keys_for(query), "key:ret", "wait:900")
            bar(r"NORMAL \| (?:search hit \w+, continuing at \w+  )?\[(\d+)/(\d+)\]", k)

        def find_esc():
            before = select("first")
            search_open("zebrafish")
            inject("key:esc", "wait:500")
            expect_copy("zebrafish")
            return f"selection {before!r}, /zebrafish Enter, Esc (T7), y: the desktop holds 'zebrafish' (the decided difference from vim)"
        step("/word Enter Esc y copies the match", find_esc)

        def find_open():
            before = select("first")
            search_open("zebrafish")
            if "[" not in bar_now():
                raise Fail(f"the find is not open: {bar_now()!r}")
            desk.set("old")
            msg = yank()
            time.sleep(0.3)
            got = desk.get()
            undecided["result"] = (before, msg, got)
            inject("key:esc", "wait:400")
            return f"(observed) selection {before!r}, /zebrafish Enter (find open), y: bar {msg!r}, desktop holds {got!r}"
        step("/word Enter y with the find still open (observed)", find_open)

        def fake():
            want = select("fake")
            expect_copy(want)
            return f"window.getSelection, document.getSelection, Selection.prototype.toString replaced by the page; y: {want!r}"
        step("a page faking getSelection still gives the real text", fake)

        def frame():
            # A selection in the main frame first: the focused frame's must win.
            select("first")
            to_normal()
            k = log.mark()
            click(FRAME_BUTTON)
            want = urllib.parse.unquote(log.wait(r'"title":"cy frame n=\d+ sel=([^"]*)"', k)[1])
            expect_copy(want)
            return f"a click into the iframe (its button selects its text): y gives {want!r}, not the main frame's"
        step("the focused frame's selection (iframe)", frame)

        def log_rule():
            # The host's and the shell's logs show the length, never the text.
            host = [l for l in log.lines if re.search(r"selection \d+ \d+ \d+ chars", l)]
            if not host:
                raise Fail("no `selection <tab> <req> <n> chars` line in the logs")
            leaks = [l for l in log.lines if '"event":"selection"' in l or "one\\nline two" in l]
            if leaks:
                raise Fail(f"selected text in the log: {leaks[0]}")
            return f"{len(host)} `selection ... chars` lines, no selected text logged (e.g. {host[-1].split('] ', 1)[1]!r})"
        step("PROTOCOL.md Selection: the log holds lengths only", log_rule)

        # T118 with the user's `yy` mapping (the built-in one was removed).
        def source_map():
            with open(os.path.join(profile, "init.js"), "w") as f:
                f.write(YY_MAP)
            # Back to the main frame (the previous step left the iframe focused).
            click(EMPTY)
            to_normal()
            k = log.mark()
            inject(*keys_for(":source"), "key:ret", "wait:500")
            bar(r"NORMAL \| sourced 1 file", k)
            return "init.js with web.map(\"n\", \"yy\", \":let @+ = @%<CR>\"), :source"
        step("init.js: the yy mapping", source_map)

        def mapped_yy():
            select("para")
            desk.set("old")
            k = log.mark()
            inject("key:y", "wait:100", "key:y", "wait:300")
            desk.wait_for(page_url)
            if bar_now() != "NORMAL | Copied to clipboard":
                raise Fail(f"the bar shows {bar_now()!r}")
            time.sleep(MAP_TIMEOUT_MS / 1000 + 0.5)
            if sends(k):
                raise Fail(f"yy also asked for the selection: {sends(k)}")
            return f"y y: the desktop holds the URL {page_url}; no selection command, then or {MAP_TIMEOUT_MS + 500} ms later"
        step("T118: yy (mapped) copies the URL", mapped_yy)

        def mapped_lone_y():
            want = select("para")
            desk.set("old")
            k = log.mark()
            t0 = time.monotonic()
            inject("key:y")
            while not sends(k) and time.monotonic() - t0 < 5:
                time.sleep(0.02)
            waited = time.monotonic() - t0
            if not sends(k):
                raise Fail("a lone y never yanked")
            if waited < MAP_TIMEOUT_MS / 1000 * 0.8:
                raise Fail(f"a lone y yanked after {waited:.2f} s, before the mapping timeout")
            desk.wait_for(want)
            if bar_now() != "NORMAL | Copied to clipboard":
                raise Fail(f"the bar shows {bar_now()!r}")
            return f"a lone y: the selection command after {waited:.2f} s (MAP_TIMEOUT_MS {MAP_TIMEOUT_MS}), the desktop holds {want!r}"
        step("T118: a lone y yanks after the mapping timeout", mapped_lone_y)

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

    if "result" in undecided:
        before, msg, got = undecided["result"]
        print(f"UNDECIDED /word Enter (find open) y: selection before {before!r}; bar {msg!r}; clipboard {got!r}")
    failed = [r for r in results if not r[1]]
    print(f"{len(results) - len(failed)}/{len(results)} steps passed; log in {out}/web.log")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
