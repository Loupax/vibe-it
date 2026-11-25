#!/usr/bin/env python3
"""C-PROFILE and C-SCRIPT-MODULES live check (SCRIPTING.md "Files and profiles", "Modules", STASH.md "State
directory"), in the headless
nested dwl (web/tools/nest.sh), never on the user's display, with a TEMPORARY XDG_CONFIG_HOME and
XDG_STATE_HOME under --out (never the user's), local 127.0.0.1 pages only, on a PRIVATE D-Bus session bus
(web/tools/notify-probe/private-bus.conf):

    web/shell/test/c_profile_live.py --out .qwork/c-profile --host-bin web/host-cef/target/release/web-host-cef

Steps: two profiles (alpha, beta) side by side, each with its own init.js that imports ONE shared module,
`$XDG_CONFIG_HOME/web/lib/common.js` (alpha as `../../lib/common.js`, beta by its absolute path; the module
imports its neighbour `./pages.js`); its `ready` handler opens the shared page, then the profile's own. The
`default` profile's init.js (it would open a third page) is NOT run for alpha or beta (C-SCRIPT-MODULES: one
init file per start). Both shells run at once on their own state directory (`$XDG_STATE_HOME/web/profiles/NAME`,
mode 0700); `web --profile alpha URL` is handed to alpha's instance and not beta's; `web --profile beta
--forget-site` acts on beta (alpha's log stays silent); a bad name is a usage error (exit 2 from `fail`,
nothing started); last, `web` with no --profile is `--profile default`: it runs profiles/default/init.js and
its state is `$XDG_STATE_HOME/web/profiles/default`, nothing directly under `$XDG_STATE_HOME/web`.
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
    for n in ("default", "shared", "alpha", "beta", "alpha-url", "beta-url"):
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

    web_dir = os.path.join(config_home, "web")
    write(os.path.join(web_dir, "profiles", "default", "init.js"), f'web.on("ready", () => web.cmd("new {base}/default.html"));\n')
    write(os.path.join(web_dir, "lib", "pages.js"), f'export const base = "{base}";\n')
    write(os.path.join(web_dir, "lib", "common.js"),
          'import { base } from "./pages.js";\n'
          'export function start(name) {\n'
          '  web.on("ready", () => { web.cmd("new " + base + "/shared.html"); web.cmd("new " + base + "/" + name + ".html"); });\n'
          '}\n')
    common = os.path.join(web_dir, "lib", "common.js")
    write(os.path.join(web_dir, "profiles", "alpha", "init.js"), 'import { start } from "../../lib/common.js";\nstart("alpha");\n')
    write(os.path.join(web_dir, "profiles", "beta", "init.js"), f'import {{ start }} from "{common}";\nstart("beta");\n')

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
        # 1: both run at once, each its own init.js only, both through the one shared lib/common.js
        for p, (proc, log) in shells.items():
            log.wait(r'"url":"' + re.escape(base) + r'/shared\.html"', timeout=90)
            log.wait(r'"url":"' + re.escape(base) + rf'/{p}\.html"', timeout=30)
            log.wait(r"buffer 3 of 3 is current", timeout=30)
            other = "beta" if p == "alpha" else "alpha"
            if any(f"/{other}.html" in l and '"url"' in l for l in log.lines):
                raise Fail(f"{p} opened {other}'s page")
            if any("/default.html" in l for l in log.lines):
                raise Fail(f"{p} ran the default profile's init.js")
            if any("script error" in l for l in log.lines):
                raise Fail(f"{p}: a script error")
        if any(proc.poll() is not None for proc, _ in shells.values()):
            raise Fail("a shell exited")
        print("  ok   alpha and beta run side by side, both import lib/common.js (relative and absolute), "
              "shared then own page; no cross-opening, the default profile's init.js not run", flush=True)

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

        # 6: no --profile is --profile default (its own init.js, its own state directory)
        unit = f"web-c-profile-live-{os.getpid()}-default"
        units.append(unit)
        cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
               "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--host-bin", a.host_bin]
        proc = subprocess.Popen(cmd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        shells["default"] = (proc, live.Log(proc, os.path.join(out, "web-default.log")))
        log = shells["default"][1]
        log.wait(r'"url":"' + re.escape(base) + r'/default\.html"', timeout=90)
        if any("/shared.html" in l for l in log.lines):
            raise Fail("the default profile ran another profile's script")
        d = os.path.join(state_home, "web", "profiles", "default")
        if not os.path.isdir(os.path.join(d, "engines", "cef")):
            raise Fail(f"no engine profile under {d}")
        top = sorted(os.listdir(os.path.join(state_home, "web")))
        if top != ["profiles"]:
            raise Fail(f"$XDG_STATE_HOME/web holds {top}, not only profiles/")
        print("  ok   web (no --profile) ran profiles/default/init.js, state in profiles/default; the base holds only profiles/", flush=True)
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
