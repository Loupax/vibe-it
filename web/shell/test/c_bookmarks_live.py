#!/usr/bin/env python3
"""C-BOOKMARKS live check: `:bookmarkAdd`, `:bookmarkSearch`, `:bookmarkDelete` and `:go`'s ghost
(KEYS.md "Bookmarks", T69-T76) in the headless nested dwl (web/tools/nest.sh), never on the user's display.

    web/shell/test/c_bookmarks_live.py --out .qwork/c-bookmarks --inject .qwork/inject/release/inject

The state directory (--out/state) starts with a history file where pick3 has three visits. Steps: add
two bookmarks (the start page and pick2, each with the page's own title, read back from
--out/state/bookmarks); `:bookmarkAdd` again updates and does not duplicate; `:bookmarkSearch` lists them
(newest first), typing filters, Enter loads the chosen URL in the page; `:bookmarkDelete` removes the
current page; `:go <host>/pick` ghosts the bookmark (pick2) although the history ranks pick3 higher; with
no bookmark left the ghost is the history's pick3. Exit 0 when every step passed. Needs what
test/live.py needs."""

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
    hostport = f"127.0.0.1:{srv.server_address[1]}"
    base = f"http://{hostport}"
    start, two, three = f"{base}/pick.html", f"{base}/pick2.html", f"{base}/pick3.html"

    # pick3 (never loaded) has the most visits: on history alone `:go <host>/pick` ghosts it.
    with open(os.path.join(state, "history"), "w") as f:
        os.chmod(f.name, 0o600)
        for ms in (1000, 1001, 1002):
            f.write(f"{ms}\t{three}\tTarget three\n")

    unit = f"web-c-bookmarks-live-{os.getpid()}"
    cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
           "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", state, "--host-bin", a.host_bin, start]
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

    def bookmarks():
        try:
            return open(os.path.join(state, "bookmarks")).read().splitlines()
        except FileNotFoundError:
            return []

    try:
        log.wait(r'"title":"pick start"', timeout=60)
        g = livelib.settle(log, shot)
        print(f"output {g.ow}x{g.oh} at scale {g.scale}, shell window {g.ww}x{g.wh}")

        def inject(*cmds):
            subprocess.run([a.inject, str(g.ow), str(g.oh)] + list(cmds), env=nest_env, check=True)

        def command(line, wait_for, timeout=6):
            """Runs `:line` (Delete hides any ghost so only what was typed runs) and waits for the
            status-bar line matching `wait_for`; returns it."""
            k = log.mark()
            inject(*keys_for(":" + line), "key:del", "key:ret", "wait:200")
            return bar(k, wait_for, timeout)

        def bar(since, pat, timeout=6):
            log.wait(r"\] bar: " + pat, since, timeout)
            time.sleep(0.1)
            for l in reversed(log.lines[since:]):
                if re.search(r"\] bar: " + pat, l):
                    return l
            raise Fail(f"no bar line {pat}")

        def t69():
            command("bookmarkAdd", r"NORMAL \| bookmarked: " + re.escape(start))
            if bookmarks() != [f"{start}\tpick start"]:
                raise Fail(f"file: {bookmarks()}")
            k = log.mark()
            inject(*keys_for(f":go {two}"), "key:del", "key:ret", "wait:300")
            log.wait(r'"title":"pick target two"', k, 15)
            time.sleep(0.5)
            command("bookmarkAdd", r"NORMAL \| bookmarked: " + re.escape(two))
            if bookmarks() != [f"{start}\tpick start", f"{two}\tpick target two"]:
                raise Fail(f"file: {bookmarks()}")
            return "two bookmarks, each with the page's title: " + " / ".join(bookmarks())

        step("T69 :bookmarkAdd on two pages", t69)

        def t70():
            command("bookmarkAdd", r"NORMAL \| bookmark updated: " + re.escape(two))
            if len(bookmarks()) != 2:
                raise Fail(f"file: {bookmarks()}")
            leftovers = [n for n in os.listdir(state) if n.startswith(".bookmarks.tmp")]
            if leftovers:
                raise Fail(f"temp files left: {leftovers}")
            return "a second add updated the line, no duplicate, no temp file"

        step("T70 :bookmarkAdd again updates", t70)

        def t73():
            k = log.mark()
            inject(*keys_for(":bookmarkSearch"), "key:del", "key:ret", "wait:300")
            l = bar(k, r">\| pick bookmarks \[1/2 of 2\] " + re.escape(two))
            k = log.mark()
            inject(*keys_for("start"), "wait:200")
            l = bar(k, r">start\| pick bookmarks \[1/1 of 2\] " + re.escape(start))
            k = log.mark()
            inject("key:ret", "wait:300")
            log.wait(r'"title":"pick start"', k, 15)
            bar(k, r"NORMAL")
            return "newest first (pick2), `start` leaves pick.html, Enter loaded it"

        step("T73 :bookmarkSearch lists, filters, Enter opens", t73)

        def t71():
            time.sleep(0.5)
            command("bookmarkDelete", r"NORMAL \| bookmark removed: " + re.escape(start))
            if bookmarks() != [f"{two}\tpick target two"]:
                raise Fail(f"file: {bookmarks()}")
            return "the current page's bookmark is gone, the other stays"

        step("T71 :bookmarkDelete removes the current page", t71)

        def t75():
            word = f"go {hostport}/pick"
            k = log.mark()
            inject(*keys_for(":" + word), "wait:300")
            l = bar(k, re.escape(":" + word) + r"\|2\.html")
            k = log.mark()
            inject("key:esc", "wait:200")
            command("bookmarkDelete " + two, r"NORMAL \| bookmark removed: " + re.escape(two))
            if bookmarks():
                raise Fail(f"file: {bookmarks()}")
            k = log.mark()
            inject(*keys_for(":" + word), "wait:300")
            bar(k, re.escape(":" + word) + r"\|3\.html")
            inject("key:esc", "wait:200")
            return "ghost `2.html` (the bookmark) over the history's pick3; after the delete `3.html`"

        step("T75/T76 :go ghost prefers the bookmark; :bookmarkDelete <url>", t75)
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
