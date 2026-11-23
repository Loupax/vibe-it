#!/usr/bin/env python3
"""C6c bench: frame pacing in the headless nested dwl (web/tools/nest.sh), never on the user's display.

    web/shell/test/c6c_bench.py --out .qwork/c6c [--web PATH] [--modes idle,raf,css,video]

For each mode (test/pages/c6c.html: idle, raf = requestAnimationFrame only, css = a CSS
animation, video = a generated 30 fps WebM) it starts the shell on the nest, lets the page
measure rAF (and video frame) intervals for 10 s, and samples the shell's CPU time (utime+stime
of the shell process) over the same window. Prints one line per mode. Run it under
`flock /tmp/scratchy-perf.lock` (the harness does not take the lock itself) and note `uptime`.
Needs ffmpeg for the video mode."""

import argparse
import functools
import os
import shutil
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.dont_write_bytecode = True
import live  # noqa: E402

WEB = live.WEB


def cpu_s(pid):
    with open(f"/proc/{pid}/stat") as f:
        parts = f.read().rsplit(")", 1)[1].split()
    return (int(parts[11]) + int(parts[12])) / os.sysconf("SC_CLK_TCK")


def run_mode(a, mode, base, wl, out, x11=None):
    state = os.path.join(out, "state-" + mode)
    shutil.rmtree(state, ignore_errors=True)
    env = dict(os.environ, WAYLAND_DISPLAY=wl)
    env.pop("WAYLAND_SOCKET", None)
    env.pop("DISPLAY", None)
    if x11:
        env.pop("WAYLAND_DISPLAY")
        env["DISPLAY"] = x11
    unit = f"web-c6c-bench-{os.getpid()}-{mode}"
    cmd = ["systemd-run", "--user", "--scope", "-q", f"--unit={unit}", "-p", "MemoryMax=6G", "-p", "MemorySwapMax=0",
           "bash", "-c", 'ulimit -c 0; exec "$@"', "web", a.web, "--state-dir", state, "--host-bin", a.host_bin,
           f"{base}/c6c.html#{mode}"]
    proc = subprocess.Popen(cmd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    log = live.Log(proc, os.path.join(out, f"web-{mode}.log"))
    try:
        log.wait(r'"title":"c6c ' + mode + r' running', timeout=60)
        c0, t0 = cpu_s(proc.pid), time.monotonic()
        m = log.wait(r'"title":"c6c ' + mode + r' done([^"]*)"', timeout=30)
        c1, t1 = cpu_s(proc.pid), time.monotonic()
        cpu = 100 * (c1 - c0) / (t1 - t0)
        return f"{mode:6s} shell CPU {cpu:5.1f}% of one core over {t1 - t0:.1f}s |{m[1]}"
    finally:
        subprocess.run(["systemctl", "--user", "stop", unit + ".scope"], check=False, capture_output=True)
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            pass


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--web", default=os.path.join(live.SHELL, "target/debug/web"))
    ap.add_argument("--host-bin", default=os.path.join(WEB, "host-cef/target/debug/web-host-cef"))
    ap.add_argument("--modes", default="idle,raf,css,video")
    ap.add_argument("--x11", action="store_true", help="run the shell as an X11 window (the nest's Xwayland): no desktop frame callbacks")
    a = ap.parse_args()
    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)
    pages = os.path.join(out, "pages")
    os.makedirs(pages, exist_ok=True)
    shutil.copy(os.path.join(HERE, "pages", "c6c.html"), pages)
    webm = os.path.join(pages, "c6c.webm")
    if not os.path.exists(webm):
        subprocess.run(["ffmpeg", "-v", "error", "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=30", "-t", "4",
                        "-c:v", "libvpx", "-b:v", "500k", "-pix_fmt", "yuv420p", webm], check=True)
    wl = subprocess.run([os.path.join(WEB, "tools/nest.sh")], check=True, capture_output=True, text=True).stdout.strip()
    page_log = open(os.path.join(out, "pages.log"), "w", buffering=1)
    srv = live.Server(("127.0.0.1", 0), functools.partial(live.Pages, directory=pages, log=page_log))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_address[1]}"
    x11 = None
    if a.x11:
        # Xwayland starts lazily on the first X11 client, so its DISPLAY is known from the start.
        x11 = subprocess.run([os.path.join(WEB, "tools/nest.sh"), "--x11-display"], check=True, capture_output=True,
                             text=True).stdout.strip()
    for mode in a.modes.split(","):
        print(run_mode(a, mode, base, wl, out, x11), flush=True)


if __name__ == "__main__":
    main()
