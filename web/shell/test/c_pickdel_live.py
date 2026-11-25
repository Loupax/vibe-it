#!/usr/bin/env python3
"""C-PICKDEL live check: Ctrl-d in the picker (KEYS.md "Deleting from a picker", T158-T163) in the
headless nested dwl (web/tools/nest.sh), never on the user's display.

    web/shell/test/c_pickdel_live.py --out .qwork/c-pickdel --inject .qwork/inject/release/inject

The state directory (--out/state) gets a seeded history file before the shell starts. Steps:
`:historySearch` opens the picker; Ctrl-d asks (the bar logs `delete history entry <url>? (y/n)` and
the pick line ends with ` confirm`); `n` cancels (`not deleted`, the file is as it was, the picker is
still open); Ctrl-d, `y` deletes: the row is gone from the log's `pick` line (one less row, the
selection at the same position) and every line of that URL is gone from the history file, the others
are kept. Exit 0 when every step passed. Needs what test/live.py needs."""

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
PROBE = os.path.join(WEB, "tools/notify-probe")
# C-PICKDEL-MAP (T167-T168): "p" mappings from the profile's init.js (a temporary XDG_CONFIG_HOME).
INIT_JS = """\
web.map("p", "<C-x>", row => { if (row.source === "history") web.cmd("historyDelete " + row.url); });
web.map("p", "<C-e>", row => web.cmd("echo " + JSON.stringify(row)));
"""
Fail = live.Fail
PICK = re.compile(r"pick history \[(\d+)/(\d+) of (\d+)\] (\S*)( confirm)?")
MSG = re.compile(r" msg: (.*)$")  # C-PICKMSG: the message drawn beside the filter line


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--web", default=os.path.join(SHELL, "target/debug/web"))
    ap.add_argument("--host-bin", default=os.path.join(WEB, "host-cef/target/debug/web-host-cef"))
    ap.add_argument("--inject", required=True, help="the inject binary (build web/tools/inject)")
    a = ap.parse_args()
    if not os.environ.get("C_PICKDEL_PRIVATE_BUS"):
        env = dict(os.environ, C_PICKDEL_PRIVATE_BUS="1")
        conf = os.path.join(PROBE, "private-bus.conf")
        os.execvpe("dbus-run-session", ["dbus-run-session", f"--config-file={conf}", "--", sys.executable, "-I"] + sys.argv, env)
    bus = os.environ.get("DBUS_SESSION_BUS_ADDRESS", "")
    if not bus or re.search(r"/run/user/\d+/bus", bus):
        print(f"refusing: not a private bus ({bus!r})", file=sys.stderr)
        return 2
    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)
    state = os.path.join(out, "state")
    shutil.rmtree(state, ignore_errors=True)
    os.makedirs(state, mode=0o700)

    config = os.path.join(out, "config")
    os.makedirs(os.path.join(config, "web", "profiles", "default"), exist_ok=True)
    with open(os.path.join(config, "web", "profiles", "default", "init.js"), "w") as f:
        f.write(INIT_JS)

    wl = subprocess.run([os.path.join(WEB, "tools/nest.sh")], check=True, capture_output=True, text=True).stdout.strip()
    nest_env = dict(os.environ, WAYLAND_DISPLAY=wl)
    shell_env = dict(os.environ, WAYLAND_DISPLAY=wl, XDG_CONFIG_HOME=config,
                     XDG_STATE_HOME=os.path.join(out, "xdg-state"), XDG_DATA_HOME=os.path.join(out, "xdg-data"))
    shell_env.pop("WAYLAND_SOCKET", None)
    shell_env.pop("DISPLAY", None)

    page_log = open(os.path.join(out, "pages.log"), "w", buffering=1)
    srv = live.Server(("127.0.0.1", 0), functools.partial(live.Pages, directory=os.path.join(HERE, "pages"), log=page_log))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_address[1]}"

    # The page that is loaded (pick.html) adds its own visit; the rest is seeded. beta.invalid/docs is
    # visited twice (two lines to remove), alpha once.
    seed = [
        (1003, "https://alpha.invalid/", "Alpha home"),
        (1004, "https://beta.invalid/docs", "Beta docs"),
        (1005, "https://beta.invalid/docs", "Beta docs"),
        (1006, "https://gamma.invalid/x", "Gamma x"),
    ]
    hist = os.path.join(state, "history")
    with open(hist, "w") as f:
        os.chmod(f.name, 0o600)
        for ms, url, title in seed:
            f.write(f"{ms}\t{url}\t{title}\n")

    unit = f"web-c-pickdel-live-{os.getpid()}"
    cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
           "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", state, "--host-bin", a.host_bin,
           f"{base}/pick.html"]
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

    def history_lines():
        with open(hist) as f:
            return f.read().splitlines()

    try:
        log.wait(r'"title":"pick start"', timeout=60)
        g = livelib.settle(log, shot)

        def inject(*cmds):
            subprocess.run([a.inject, str(g.ow), str(g.oh)] + list(cmds), env=nest_env, check=True)

        def last_bar(since):
            for l in reversed(log.lines[since:]):
                if "] bar: " in l:
                    return l
            raise Fail("no bar line")

        def pick(line):
            m = PICK.search(line)
            if not m:
                raise Fail(f"not the picker's line: {line}")
            return int(m[1]), int(m[2]), int(m[3]), m[4], bool(m[5])

        state0 = {}

        def open_picker():
            k = log.mark()
            inject(*keys_for(":historySearch"), "key:ret", "wait:300")
            log.wait(r"\] bar: >\| pick history", k, 6)
            time.sleep(0.15)
            sel, n, total, url, asking = pick(last_bar(k))
            if asking or sel != 1:
                raise Fail(f"the picker opens at [{sel}] asking={asking}")
            state0.update(total=total, url=url)
            return f"{total} rows, first {url}"

        step("open :historySearch over the seeded history", open_picker)

        def select_docs():
            # The first row is a site root made up from its pages (no line of its own to delete); select
            # the page row, which has real lines.
            for _ in range(state0["total"]):
                k = log.mark()
                inject("hold:c", "key:n", "release", "wait:200")
                time.sleep(0.2)
                sel, n, total, url, asking = pick(last_bar(k))
                if url.endswith("/docs"):
                    state0.update(url=url, sel=sel)
                    return f"selected [{sel}] {url}"
            raise Fail("no /docs row")

        step("move to the beta.invalid/docs page row", select_docs)

        def ctrl_d_asks():
            before = history_lines()
            k = log.mark()
            inject("hold:c", "key:d", "release", "wait:300")
            log.wait(r"delete history entry " + re.escape(state0["url"]) + r"\? \(y/n\)", k, 6)
            time.sleep(0.15)
            line = last_bar(k)
            sel, n, total, url, asking = pick(line)
            if not asking or total != state0["total"] or url != state0["url"]:
                raise Fail(f"asking={asking}, [{sel}/{n} of {total}] {url}: {line}")
            if history_lines() != before:
                raise Fail("the history file changed before the answer")
            return f"asked about {url}; list unchanged"

        step("T158 Ctrl-d asks, nothing is deleted yet", ctrl_d_asks)

        def n_cancels():
            before = history_lines()
            k = log.mark()
            inject("key:n", "wait:300")
            log.wait(r"\] bar: >\| pick history \[\d+/\d+ of \d+\] \S* msg: not deleted$", k, 6)
            time.sleep(0.15)
            line = last_bar(k)
            sel, n, total, url, asking = pick(line)
            if asking or total != state0["total"]:
                raise Fail(f"after n: asking={asking}, total {total}: {line}")
            if "] bar: >|" not in line:
                raise Fail(f"the n was typed: {line}")
            if history_lines() != before:
                raise Fail("the history file changed after n")
            m = MSG.search(line)
            if not m or m[1] != "not deleted":
                raise Fail(f"no `not deleted` beside the filter line: {line}")
            return "the question is gone, the picker is open, the file unchanged, `not deleted` beside the filter line"

        step("T161 n cancels and is not typed", n_cancels)

        def y_deletes():
            before = history_lines()
            url = state0["url"]
            want_gone = [l for l in before if l.split("\t")[1] == url]
            want_kept = [l for l in before if l.split("\t")[1] != url]
            k = log.mark()
            inject("hold:c", "key:d", "release", "wait:200", "key:y", "wait:400")
            log.wait(r"\] bar: >\| pick history \[\d+/\d+ of \d+\] \S* msg: history entry removed: ", k, 6)
            time.sleep(0.2)
            line = last_bar(k)
            sel, n, total, newurl, asking = pick(line)
            m = MSG.search(line)
            if not m or not m[1].startswith(f"history entry removed: {url} ("):
                raise Fail(f"no removal message beside the filter line: {line}")
            if asking:
                raise Fail(f"still asking: {line}")
            after = history_lines()
            if any(l in after for l in want_gone):
                raise Fail(f"{url} is still in the file: {after}")
            if after != want_kept:
                raise Fail(f"other lines changed: {after} vs {want_kept}")
            # Its site root was made up from that one page, so it goes too (two rows less).
            if total != state0["total"] - 2 or sel != state0["sel"] or newurl == url:
                raise Fail(f"after y: [{sel}/{n} of {total}] {newurl} (was {state0['total']} rows, {url})")
            return f"{url}: {len(want_gone)} line(s) gone from the file, the others kept; rows {state0['total']} -> {total}, selected [{sel}] {newurl}; message `{m[1]}`"

        step("T160/T163 Ctrl-d, y deletes the row and the file's lines, the list refreshes", y_deletes)

        def p_map_row():
            k = log.mark()
            inject("hold:c", "key:e", "release", "wait:300")
            log.wait(r"pick-map <C-e>", k, 6)
            log.wait(r"\] bar: >\| pick history .* msg: .*\"source\":\"history\"", k, 6)
            time.sleep(0.15)
            line = last_bar(k)
            if '"label":' not in line or '"detail":' not in line or '"url":' not in line:
                raise Fail(f"row fields missing: {line}")
            return "echo of the row beside the filter line: " + MSG.search(line)[1][:90]

        step("T167 a mapped <C-e> gets the row (source, label, detail, url)", p_map_row)

        def p_map_delete():
            # Move to the gamma page row (a real line of its own), then Ctrl-x: deleted, no question.
            for _ in range(state0["total"] + 1):
                k = log.mark()
                inject("hold:c", "key:n", "release", "wait:200")
                time.sleep(0.2)
                sel, n, total, url, asking = pick(last_bar(k))
                if url.endswith("/x"):
                    break
            else:
                raise Fail("no /x row")
            before = history_lines()
            k = log.mark()
            inject("hold:c", "key:x", "release", "wait:400")
            log.wait(r"pick-map <C-x>", k, 6)
            log.wait(r"\] bar: >\| pick history \[\d+/\d+ of \d+\] \S* msg: history entry removed: https://gamma.invalid/x", k, 6)
            time.sleep(0.2)
            line = last_bar(k)
            sel2, n2, total2, url2, asking2 = pick(line)
            after = history_lines()
            if asking2 or any("gamma.invalid/x" in l for l in after) or len(after) != len(before) - 1:
                raise Fail(f"after Ctrl-x: asking={asking2}, file {after}")
            if total2 >= total:
                raise Fail(f"the list did not refresh: {total} -> {total2}")
            return f"gamma.invalid/x deleted at once; rows {total} -> {total2}"

        step("T167/T168 a mapped <C-x> deletes the row without asking and the list refreshes", p_map_delete)

        def esc():
            k = log.mark()
            inject("key:esc", "wait:300")
            log.wait(r"\] bar: NORMAL", k, 5)
            return "closed"

        step("Esc closes", esc)
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
