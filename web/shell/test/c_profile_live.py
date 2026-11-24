#!/usr/bin/env python3
"""C-PROFILE live check (SCRIPTING.md "Files and profiles", STASH.md "State directory"), in the headless
nested dwl (web/tools/nest.sh), never on the user's display, with a TEMPORARY XDG_CONFIG_HOME and
XDG_STATE_HOME under --out (never the user's), local 127.0.0.1 pages only, on a PRIVATE D-Bus session bus
(web/tools/notify-probe/private-bus.conf):

    web/shell/test/c_profile_live.py --out .qwork/c-profile --host-bin web/host-cef/target/release/web-host-cef

Steps: two profiles (alpha, beta) side by side, each with its own init.js whose `ready` handler opens a
distinct page, plus a global init.js that opens a third; both shells run at once on their own state
directory (`$XDG_STATE_HOME/web/profiles/NAME`, mode 0700); the global script ran first in each;
`web --profile alpha URL` is handed to alpha's instance and not beta's; `web --profile beta --forget-site`
acts on beta (alpha's log stays silent); a bad name is a usage error (exit 2 from `fail`, nothing started).
No keyboard injection is needed. Exit 0 when every step passed."""

import argparse
import functools
import os
import re
import shutil
import stat
import subprocess
import sys
import threading

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True  # no __pycache__ in the source tree
import live  # noqa: E402

SHELL = live.SHELL
WEB = live.WEB
PROBE = os.path.join(WEB, "tools/notify-probe")
Fail = live.Fail


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--web", default=os.path.join(SHELL, "target/debug/web"))
    ap.add_argument("--host-bin", default=os.path.join(WEB, "host-cef/target/debug/web-host-cef"))
    a = ap.parse_args()
    if not os.environ.get("C_PROFILE_PRIVATE_BUS"):
        env = dict(os.environ, C_PROFILE_PRIVATE_BUS="1")
        conf = os.path.join(PROBE, "private-bus.conf")
        os.execvpe("dbus-run-session", ["dbus-run-session", f"--config-file={conf}", "--", sys.executable, "-I"] + sys.argv, env)
    bus = os.environ.get("DBUS_SESSION_BUS_ADDRESS", "")
    if not bus or re.search(r"/run/user/\d+/bus", bus):
        print(f"refusing: not a private bus ({bus!r})", file=sys.stderr)
        return 2

    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)
    state_home = os.path.join(out, "xdg-state")
    config_home = os.path.join(out, "xdg-config")
    shutil.rmtree(state_home, ignore_errors=True)
    shutil.rmtree(config_home, ignore_errors=True)
    os.makedirs(state_home, mode=0o700)

    pages = os.path.join(out, "pages")
    os.makedirs(pages, exist_ok=True)
    for n in ("global", "alpha", "beta", "alpha-url", "beta-url"):
        with open(os.path.join(pages, f"{n}.html"), "w") as f:
            f.write(f"<title>page-{n}</title><p>{n}</p>")
    page_log = open(os.path.join(out, "pages.log"), "w", buffering=1)
    srv = live.Server(("127.0.0.1", 0), functools.partial(live.Pages, directory=pages, log=page_log))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_address[1]}"

    def write(path, text):
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w") as f:
            f.write(text)

    write(os.path.join(config_home, "web", "init.js"), f'web.on("ready", () => web.cmd("new {base}/global.html"));\n')
    for p in ("alpha", "beta"):
        write(os.path.join(config_home, "web", "profiles", p, "init.js"), f'web.on("ready", () => web.cmd("new {base}/{p}.html"));\n')

    wl = subprocess.run([os.path.join(WEB, "tools/nest.sh")], check=True, capture_output=True, text=True).stdout.strip()
    env = dict(os.environ, WAYLAND_DISPLAY=wl, XDG_STATE_HOME=state_home, XDG_CONFIG_HOME=config_home)
    for k in ("WAYLAND_SOCKET", "DISPLAY", "WEB_STATE_DIR"):
        env.pop(k, None)

    units = []
    shells = {}
    for p in ("alpha", "beta"):
        unit = f"web-c-profile-live-{os.getpid()}-{p}"
        units.append(unit)
        cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
               "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--host-bin", a.host_bin, "--profile", p]
        proc = subprocess.Popen(cmd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        shells[p] = (proc, live.Log(proc, os.path.join(out, f"web-{p}.log")))

    def cli(*args):
        r = subprocess.run([a.web, "--host-bin", a.host_bin, *args], capture_output=True, text=True, timeout=90, env=env)
        print(f"  web {' '.join(args)}: exit {r.returncode} stdout {r.stdout.strip()!r} stderr {r.stderr.strip()[-200:]!r}", flush=True)
        return r

    ok = False
    try:
        # 1: both run at once, each loaded its own script after the global one
        for p, (proc, log) in shells.items():
            log.wait(r'"url":"' + re.escape(base) + r'/global\.html"', timeout=90)
            log.wait(r'"url":"' + re.escape(base) + rf'/{p}\.html"', timeout=30)
            log.wait(r"buffer 3 of 3 is current", timeout=30)
            other = "beta" if p == "alpha" else "alpha"
            if any(f"/{other}.html" in l and '"url"' in l for l in log.lines):
                raise Fail(f"{p} opened {other}'s page")
            gi = next(i for i, l in enumerate(log.lines) if "/global.html" in l and '"url"' in l)
            pi = next(i for i, l in enumerate(log.lines) if f"/{p}.html" in l and '"url"' in l)
            if gi > pi:
                raise Fail(f"{p}: the profile script ran before the global one")
        if any(proc.poll() is not None for proc, _ in shells.values()):
            raise Fail("a shell exited")
        print("  ok   alpha and beta run side by side; global then profile page in each, no cross-opening", flush=True)

        # 2: state directories
        for p in ("alpha", "beta"):
            d = os.path.join(state_home, "web", "profiles", p)
            if not os.path.isdir(os.path.join(d, "engines", "cef")):
                raise Fail(f"no engine profile under {d}")
            for x in (os.path.join(state_home, "web", "profiles"), d):
                mode = stat.S_IMODE(os.stat(x).st_mode)
                if mode != 0o700:
                    raise Fail(f"{x} is {mode:o}, not 700")
        if os.path.exists(os.path.join(state_home, "web", "engines")):
            raise Fail("a profile touched the default state directory")
        print("  ok   $XDG_STATE_HOME/web/profiles/{alpha,beta}/engines/cef exist, 0700; the default dir is untouched", flush=True)

        # 3: a URL goes to its own profile's instance
        ma, mb = shells["alpha"][1].mark(), shells["beta"][1].mark()
        r = cli("--profile", "alpha", f"{base}/alpha-url.html")
        if r.returncode != 0 or "opened" not in r.stdout or str(shells["alpha"][0].pid) not in r.stdout:
            raise Fail("web --profile alpha URL: wrong exit or message")
        shells["alpha"][1].wait(r'"url":"' + re.escape(base) + r'/alpha-url\.html"', ma, 30)
        shells["alpha"][1].wait(r"opened .*alpha-url\.html \(from another web\)", ma, 10)
        if any("alpha-url" in l for l in shells["beta"][1].lines[mb:]):
            raise Fail("beta received alpha's URL")
        mb = shells["beta"][1].mark()
        r = cli("--profile=beta", f"{base}/beta-url.html")
        shells["beta"][1].wait(r'"url":"' + re.escape(base) + r'/beta-url\.html"', mb, 30)
        if any("beta-url" in l for l in shells["alpha"][1].lines[ma:]):
            raise Fail("alpha received beta's URL")
        print("  ok   web --profile alpha URL reached alpha, --profile=beta URL reached beta; neither saw the other's", flush=True)

        # 4: forget-site acts on the named profile
        ma, mb = shells["alpha"][1].mark(), shells["beta"][1].mark()
        r = cli("--profile", "beta", "--forget-site", "127.0.0.1")
        if r.returncode not in (0, 1):
            raise Fail(f"--forget-site exit {r.returncode}")
        shells["beta"][1].wait(r"forget", mb, 15)
        if any("forget" in l.lower() for l in shells["alpha"][1].lines[ma:]):
            raise Fail("alpha's shell saw beta's forget-site")
        print("  ok   web --profile beta --forget-site went through beta's instance; alpha stayed silent", flush=True)

        # 5: a bad name is a usage error and starts nothing
        before = os.listdir(os.path.join(state_home, "web", "profiles"))
        r = cli("--profile", "../x")
        if r.returncode == 0 or "--profile" not in r.stderr or sorted(os.listdir(os.path.join(state_home, "web", "profiles"))) != sorted(before):
            raise Fail("bad profile name was accepted")
        print("  ok   --profile ../x: refused with a usage message, no directory made", flush=True)
        ok = True
    finally:
        for u in units:
            subprocess.run(["systemctl", "--user", "stop", f"{u}.scope"], capture_output=True)
        for proc, _ in shells.values():
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                pass
        srv.shutdown()
    print("PASS" if ok else "FAIL", f"; logs in {out}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
