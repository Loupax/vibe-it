#!/usr/bin/env python3
"""C4 live check: buffers (KEYS.md "Buffers", T77-T91) in the headless nested dwl (web/tools/nest.sh),
never on the user's display.

    web/shell/test/c4_live.py --out .qwork/c4 --inject .qwork/inject/release/inject

Two buffers walked through open, switch, describe, search and kill. Buffer 1 is a red page, buffer 2 a
blue one (test/pages/c4.html: the page's colour shows in screenshots which buffer the shell draws; its
title says whether the host shows the page or hides it, `<name> visible|hidden`). Steps: a second
`web <url>` opens buffer 2 and shows it (the host hides buffer 1); the status bar shows `[N]`;
`:buffer 1`, `:bufferNext`, `:bp`, Ctrl-^ switch; `:bufferDescribe` sets a description that `:ls`
lists and filters and `:b <partial>` finds; a search in one buffer is kept apart from the other's;
middle-click and Ctrl-click on a link open nothing; `:bd` kills buffer 2 (its page closes, buffer 1
shown), `:bd` on the last buffer leaves an `about:blank` buffer; `:q` closes the page and the host
exits. Exit 0 when every step passed. Needs what test/live.py needs, and PIL."""

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

    wl = subprocess.run([os.path.join(WEB, "tools/nest.sh")], check=True, capture_output=True, text=True).stdout.strip()
    nest_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    shell_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    shell_env.pop("WAYLAND_SOCKET", None)
    shell_env.pop("DISPLAY", None)

    page_log = open(os.path.join(out, "pages.log"), "w", buffering=1)
    srv = live.Server(("127.0.0.1", 0), functools.partial(live.Pages, directory=os.path.join(HERE, "pages"), log=page_log))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_address[1]}"
    one, two = f"{base}/c4.html?c=red&n=one", f"{base}/c4.html?c=blue&n=two"

    unit = f"web-c4-live-{os.getpid()}"
    cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
           "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", state, "--host-bin", a.host_bin, one]
    proc = subprocess.Popen(cmd, env=shell_env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    log = live.Log(proc, os.path.join(out, "web.log"))
    results = []

    def step(name, fn):
        t0 = time.monotonic()
        try:
            detail = fn() or ""
            results.append((name, True, detail))
            print(f"  ok   {name} ({time.monotonic() - t0:.1f}s) {detail}", flush=True)
        except Fail as e:
            results.append((name, False, str(e)))
            print(f"  FAIL {name}: {e}", flush=True)

    def shot(name):
        p = os.path.join(out, name)
        subprocess.run(["grim", p], env=nest_env, check=True)
        return p

    try:
        log.wait(r'"title":"one visible"', timeout=60)
        g = livelib.settle(log, shot)
        print(f"output {g.ow}x{g.oh} at scale {g.scale}, shell window {g.ww}x{g.wh}", flush=True)
        from PIL import Image

        def colour(name):
            """The colour in the middle of the page area, from a screenshot."""
            time.sleep(0.4)
            im = Image.open(shot(name)).convert("RGB")
            x, y = (g.bx + g.ww // 2) * g.scale, (g.by + g.wh // 3) * g.scale
            return im.getpixel((int(x), int(y)))

        def shows(c, name):
            got = colour(name)
            if got != c:
                raise Fail(f"{name}: the page area is {got}, not {c}")

        def inject(*cmds):
            subprocess.run([a.inject, str(g.ow), str(g.oh)] + list(cmds), env=nest_env, check=True)

        def bar(since, pat, timeout=8, what="bar"):
            log.wait(r"(\] |\+)" + what + ": " + pat, since, timeout)

        def command(line, wait_for=None, timeout=8):
            """Runs `:line` (Delete hides a ghost, so what was typed runs); waits for a bar line."""
            k = log.mark()
            inject(*keys_for(":" + line), "key:del", "key:ret", "wait:200")
            if wait_for:
                bar(k, wait_for, timeout)
            return k

        def tab_of(title):
            m = None
            for l in log.lines:
                mm = re.search(r'host event: \{"event":"title","tab":(\d+),"title":"' + re.escape(title) + '"', l)
                if mm:
                    m = int(mm[1])
            if m is None:
                raise Fail(f"no title {title!r} in the log")
            return m

        tabs = {}

        def open_second():
            shows(RED, "b1-start.png")
            log.wait(r"bar url: \[1\] " + re.escape(one), 0, 5)
            k = log.mark()
            r = subprocess.run([a.web, "--state-dir", state, "--host-bin", a.host_bin, two], capture_output=True, text=True,
                               timeout=60, env=shell_env)
            if r.returncode != 0 or "opened" not in r.stdout:
                raise Fail(f"second web: exit {r.returncode} {r.stdout!r} {r.stderr[-300:]!r}")
            log.wait(r"buffer 2 of 2 is current", k, 10)
            log.wait(r'"title":"two visible"', k, 30)
            log.wait(r'"title":"one hidden"', k, 10)
            log.wait(r"buffer 2 shown: page window of tab \d+", k, 10)
            bar(k, r"\[2\] " + re.escape(two), what="bar url")
            bar(k, r"NORMAL \| opened .* \(from another web\) in buffer 2")
            tabs["one"], tabs["two"] = tab_of("one visible"), tab_of("two visible")
            if tabs["one"] == tabs["two"]:
                raise Fail(f"both buffers are tab {tabs['one']}")
            log.wait(r"page window is (\d+)x(\d+)", k, 10)
            shows(BLUE, "b2-open.png")
            return f"buffer 2 = tab {tabs['two']} shown (blue), buffer 1 = tab {tabs['one']} hidden by the host; bar [2]"

        step("T86/T85 a second `web <url>` opens buffer 2; the bar shows [2]", open_second)

        def switch():
            k = command("buffer 1")
            log.wait(r"buffer 1 of 2 is current", k, 5)
            log.wait(r'"title":"one visible"', k, 10)
            log.wait(r'"title":"two hidden"', k, 10)
            bar(k, r"\[1\] " + re.escape(one), what="bar url")
            shows(RED, "b1-switch.png")
            k = command("bufferNext")
            log.wait(r"buffer 2 of 2 is current", k, 5)
            log.wait(r'"title":"two visible"', k, 10)
            shows(BLUE, "b2-next.png")
            k = command("bp")
            log.wait(r"buffer 1 of 2 is current", k, 5)
            shows(RED, "b1-prev.png")
            k = log.mark()
            inject("hold:cs", "key:6", "release", "wait:200")
            log.wait(r"buffer 2 of 2 is current", k, 5)
            shows(BLUE, "b2-ctrl-caret.png")
            return ":buffer 1 (red, two hidden), :bufferNext (blue), :bp (red), Ctrl-^ (blue)"

        step("T78/T80/T84 :buffer N, :bufferNext, :bp, Ctrl-^", switch)

        def ghost():
            k = log.mark()
            inject(*keys_for(":bufferNe"), "wait:300")
            bar(k, r":bufferNe\|w")
            inject("key:esc", "wait:200")
            # `:b` typed in full and Enter with no Delete: the alias runs, not the ghost's first `b...` name.
            k = log.mark()
            inject(*keys_for(":b"), "wait:200", "key:ret", "wait:300")
            bar(k, r"NORMAL \| buffer 2: two visible")
            if os.path.exists(os.path.join(state, "bookmarks")):
                raise Fail("`:b` ran :bookmarkAdd")
            return "`:bufferNe` ghosts `w`; `:b` Enter names buffer 2 (no bookmark written)"

        step("T90 the ghost offers long names, an alias runs as typed", ghost)

        def describe():
            command("bufferDescribe the blue one", r"NORMAL \| buffer 2: the blue one")
            k = log.mark()
            inject(*keys_for(":ls"), "key:del", "key:ret", "wait:300")
            bar(k, r">\| pick buffers \[1/2 of 2\] buffer 1")
            k = log.mark()
            inject(*keys_for("blue"), "wait:200")
            bar(k, r">blue\| pick buffers \[1/1 of 2\] buffer 2")
            shot("ls-blue.png")
            k = log.mark()
            inject("key:esc", "wait:200")
            command("b 1")
            k = log.mark()
            inject(*keys_for(":bufferList"), "key:del", "key:ret", "wait:300", *keys_for("blue"), "wait:200", "key:ret", "wait:300")
            log.wait(r"buffer 2 of 2 is current", k, 5)
            shows(BLUE, "b2-picked.png")
            command("b 1")
            k = command("b blue")
            log.wait(r"buffer 2 of 2 is current", k, 5)
            k = command("b nothing-like-this", r"NORMAL \| web: no buffer matches nothing-like-this")
            return "described; :ls lists both, `blue` leaves buffer 2, Enter shows it; :b blue finds it"

        step("T83/T77/T79 :bufferDescribe, :ls filter by description, :b <partial>", describe)

        def search():
            k = log.mark()
            inject("key:slash", *keys_for("needle"), "key:ret", "wait:300")
            bar(k, r"NORMAL \| \[1/3\]")
            k = log.mark()
            inject("key:n", "wait:300")
            bar(k, r"NORMAL \| \[2/3\]")
            k = command("b 1")
            log.wait(r"buffer 1 of 2 is current", k, 5)
            k = log.mark()
            inject("key:n", "wait:300")
            bar(k, r"NORMAL \| No previous search")
            k = command("b 2")
            log.wait(r"buffer 2 of 2 is current", k, 5)
            k = log.mark()
            inject("key:n", "wait:300")
            bar(k, r"NORMAL \| \[3/3\]")
            inject("key:esc", "wait:200")
            return "buffer 2 searched to [2/3]; buffer 1 has no search; back in buffer 2 `n` gives [3/3]"

        step("T87 the search is per buffer", search)

        def no_new_tab():
            k = command("b 1")
            log.wait(r"buffer 1 of 2 is current", k, 5)
            time.sleep(0.5)
            # The link is in the first line of the page: 40 px margin, 20 px text.
            lx, ly = g.bx + 180, g.by + 52
            # In INPUT, so the page gets the Ctrl key too (NORMAL keeps modifiers from the page).
            k = log.mark()
            inject("key:i", "wait:200", f"move:{lx},{ly}", "wait:100", "mclick", "wait:400", "hold:c", "click", "release",
                   "wait:800", "key:esc", "wait:50", "key:esc", "wait:300")
            time.sleep(1.0)
            created = [l for l in log.lines[k:] if "host event:" in l and ('"tab-created"' in l or "n=linked" in l)]
            if created:
                raise Fail(f"a click opened something: {created}")
            bar(k, r"NORMAL \|")
            shows(RED, "b1-after-clicks.png")
            return "in INPUT, middle-click and Ctrl-click on the link: no tab-created, no load in any page"

        step("T92 Ctrl/middle-click open nothing", no_new_tab)

        def forget():
            # Both buffers are on 127.0.0.1: forgetting it reloads both (the shown one and the hidden one).
            k = command("forget-site", r"NORMAL \| forgot .* ok", timeout=30)
            for t in (tabs["one"], tabs["two"]):
                log.wait(r'host event: \{"event":"load","tab":' + str(t) + r',"phase":"end","status":200,"kind":"reload"', k, 15)
            log.wait(r"reloading 127.0.0.1 \(buffer 1\)", k, 1)
            log.wait(r"reloading 127.0.0.1 \(buffer 2\)", k, 1)
            return "forget-site 127.0.0.1: buffer 1 (shown) and buffer 2 (hidden) both reloaded"

        step("T88 :forget-site reloads every buffer on the site", forget)

        def crash():
            k = command("b 2")
            log.wait(r"buffer 2 of 2 is current", k, 5)
            k = command("go chrome://crash")
            log.wait(r'host event: \{"event":"crashed","tab":' + str(tabs["two"]), k, 30)
            bar(k, r"NORMAL \| web: the page crashed")
            k = command("b 1")
            log.wait(r'"title":"one visible"', k, 10)
            shows(RED, "b1-after-crash.png")
            k = command("b 2", r"NORMAL \| web: the page crashed; :refresh loads it again")
            k = command("refresh")
            log.wait(r'"title":"two visible"', k, 30)
            shows(BLUE, "b2-after-refresh.png")
            return "buffer 2's page crashed; buffer 1 kept working; back in 2 the bar says so and :refresh loads it"

        step("T89 a crash concerns its buffer only", crash)

        def kill():
            # Already in buffer 2: `:b 2` names it.
            command("b 2", r"NORMAL \| buffer 2: two visible")
            k = command("bd", r"NORMAL \| buffer 2 killed")
            log.wait(r'"event":"tab-closed","tab":' + str(tabs["two"]), k, 10)
            log.wait(r"buffer 1 of 1 is current", k, 5)
            log.wait(r'"title":"one visible"', k, 10)
            shows(RED, "b1-after-bd.png")
            k = command("bufferKill", r"NORMAL \| buffer 1 killed")
            log.wait(r"buffer 3 of 1 is current", k, 5)
            log.wait(r'"event":"tab-closed","tab":' + str(tabs["one"]), k, 10)
            log.wait(r'"event":"tab-created","tab":\d+,"req":\d+', k, 10)
            log.wait(r"buffer 3 shown: page window of tab \d+", k, 10)
            bar(k, r"\[3\] ", what="bar url")
            c = colour("b3-blank.png")
            if c in (RED, BLUE):
                raise Fail(f"the about:blank buffer shows {c}")
            return f"buffer 2 killed (tab {tabs['two']} closed), buffer 1 shown; the last kill left buffer 3 (about:blank, {c})"

        step("T81/T82 :bd kills buffer 2, then the last buffer leaves about:blank", kill)

        def new_buffer():
            # Buffer 3 (about:blank) is current. `:new` opens buffer 4 empty with `:go ` typed.
            k = command("new")
            log.wait(r"buffer 4 of 2 is current", k, 5)
            log.wait(r'"cmd":"new-tab"', k, 5)
            log.wait(r"buffer 4 shown: page window of tab \d+", k, 10)
            bar(k, r"\[4\] ", what="bar url")
            bar(k, r":go \|$")
            k = log.mark()
            inject(*keys_for(one), "key:del", "key:ret", "wait:200")
            log.wait(r'"title":"one visible"', k, 30)
            shows(RED, "b4-new-go.png")
            bar(k, r"\[4\] " + re.escape(one), what="bar url")
            # `:bufferNew <url>` opens buffer 5 straight on that page; Ctrl-^ comes back to buffer 4.
            k = command("bufferNew " + two)
            log.wait(r"buffer 5 of 3 is current", k, 5)
            log.wait(r'"title":"two visible"', k, 30)
            shows(BLUE, "b5-new-url.png")
            k = log.mark()
            inject("hold:cs", "key:6", "release", "wait:200")
            log.wait(r"buffer 4 of 3 is current", k, 5)
            shows(RED, "b4-ctrl-caret.png")
            return ":new -> blank buffer 4 + :go prompt, typed URL shown (red); :bufferNew URL -> buffer 5 (blue); Ctrl-^ back to 4"

        step("T103/T104 :new opens a buffer (with the :go prompt, or a URL)", new_buffer)

        def quit_all():
            k = log.mark()
            r = subprocess.run([a.web, "--state-dir", state, "--host-bin", a.host_bin, two], capture_output=True, text=True,
                               timeout=60, env=shell_env)
            if r.returncode != 0:
                raise Fail(f"second web: exit {r.returncode}")
            log.wait(r'"title":"two visible"', k, 30)
            log.wait(r"buffer 6 of 4 is current", k, 5)
            k = command("q")
            code = proc.wait(timeout=30)
            if code != 0:
                raise Fail(f"web exited {code}")
            log.wait(r"quit: closing 4 page\(s\)", k, 5)
            closed = [l for l in log.lines[k:] if 'host event: {"event":"tab-closed"' in l]
            if len(closed) != 4:
                raise Fail(f"tab-closed events: {closed}")
            log.wait(r"engine host exited: exit status: 0", k, 5)
            if any("command: {\"cmd\":\"new-tab\"" in l for l in log.lines[k:]):
                raise Fail("the shell opened a page while quitting")
            return "four buffers: quit closed all pages (four tab-closed), web and its host exited 0"

        step("T91 :q closes every page and the host exits", quit_all)
    finally:
        subprocess.run(["systemctl", "--user", "stop", f"{unit}.scope"], capture_output=True)
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            pass
        srv.shutdown()

    failed = [r for r in results if not r[1]]
    print(f"{len(results) - len(failed)}/{len(results)} steps passed; log in {out}/web.log")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
