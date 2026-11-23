#!/usr/bin/env python3
"""C4-SPIKE driver (experiment only). Plays the shell on the control socket, serves pages/,
starts web-host-cef (release build with the spike module) in a memory-capped scope inside the
headless nested dwl, and runs a list of steps:

  send:<line>      a raw control line ({base} = the page server's URL)
  sleep:<s>        wait
  wait:<regex>     wait (up to 20 s) for a host log line matching regex (since the last wait/send)
  click:<x>,<y>    real pointer click at output coordinates (web/tools/inject)
  inject:<args>    raw inject args, space separated (e.g. mods:64 ev:3 mods:0)
  mem              log the scope's memory.current and the process count
  shot:<name>      grim screenshot into --out

  c4drive.py --out DIR [--mode views|windows] [--block host|chrome] [--url URL] STEP...
"""
import argparse, functools, http.server, os, re, socket, subprocess, sys, threading, time

HERE = os.path.dirname(os.path.abspath(__file__))
WEB = os.path.abspath(os.path.join(HERE, "../.."))
WT = os.path.dirname(WEB)


class Quiet(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *a):
        pass


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--mode", default="views")
    ap.add_argument("--block", default="chrome")
    ap.add_argument("--url", default="{base}/probe.html?tag=a")
    ap.add_argument("--switch", action="append", default=[])
    ap.add_argument("steps", nargs="*")
    a = ap.parse_args()
    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)
    wl = subprocess.run([os.path.join(WEB, "tools/nest.sh")], check=True, capture_output=True, text=True).stdout.strip()
    size = open(os.path.join(WT, ".qwork/nest/size")).read().strip().split("x")
    env = dict(os.environ, WAYLAND_DISPLAY=wl, WEB_HOST_C4=a.mode, WEB_HOST_C4_BLOCK=a.block)
    env.pop("DISPLAY", None)
    env.pop("WAYLAND_SOCKET", None)

    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Quiet, directory=os.path.join(HERE, "pages")))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_address[1]}"

    sock_path = os.path.join(out, "control.sock")
    if os.path.exists(sock_path):
        os.unlink(sock_path)
    ls = socket.socket(socket.AF_UNIX)
    ls.bind(sock_path)
    ls.listen(1)
    unit = f"c4spike-{os.getpid()}"
    profile = os.path.join(WT, ".qwork/c4/profile")
    os.makedirs(profile, exist_ok=True)
    env.update(WEB_HOST_BIN=os.path.join(WEB, "host-cef/target/release/web-host-cef"), WEB_HOST_PROFILE=profile, WEB_HOST_UNIT=unit)
    cmd = [os.path.join(WEB, "host-cef/run.sh"), "--control", sock_path, "--url", a.url.format(base=base),
           "--mute-audio", "--autoplay-policy=no-user-gesture-required"] + a.switch
    logf = open(os.path.join(out, "host.log"), "w", buffering=1)
    lines = []
    t0 = time.monotonic()
    proc = subprocess.Popen(cmd, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)

    def pump():
        for l in proc.stdout:
            l = l.rstrip("\n")
            lines.append(l)
            logf.write(f"{time.monotonic() - t0:7.2f} {l}\n")
    threading.Thread(target=pump, daemon=True).start()
    ls.settimeout(30)
    conn, _ = ls.accept()
    evf = conn.makefile("r")
    threading.Thread(target=lambda: [lines.append("EV " + l.strip()) or logf.write(f"{time.monotonic() - t0:7.2f} EV {l.strip()}\n") for l in evf], daemon=True).start()

    def say(m):
        print(f"{time.monotonic() - t0:7.2f} {m}", flush=True)
        logf.write(f"{time.monotonic() - t0:7.2f} STEP {m}\n")

    mark = 0
    try:
        time.sleep(4)
        for s in a.steps:
            op, _, arg = s.partition(":")
            say(s)
            if op == "send":
                mark = len(lines)
                conn.sendall((arg.format(base=base) + "\n").encode())
            elif op == "sleep":
                time.sleep(float(arg))
            elif op == "wait":
                end = time.monotonic() + 20
                rx = re.compile(arg)
                while time.monotonic() < end:
                    hit = [l for l in lines[mark:] if rx.search(l)]
                    if hit:
                        say("  hit: " + hit[0][-200:])
                        break
                    time.sleep(0.1)
                else:
                    say("  TIMEOUT")
                mark = len(lines)
            elif op == "click":
                x, y = arg.split(",")
                subprocess.run([os.path.join(WEB, "tools/inject/target/release/inject"), size[0], size[1], f"move:{x},{y}", "click"], env=env, check=True)
            elif op == "inject":
                subprocess.run([os.path.join(WEB, "tools/inject/target/release/inject"), size[0], size[1]] + arg.split(), env=env, check=True)
            elif op == "mem":
                cg = subprocess.run(["systemctl", "--user", "show", "-p", "ControlGroup", "--value", f"{unit}.scope"], capture_output=True, text=True).stdout.strip()
                cur = int(open(f"/sys/fs/cgroup{cg}/memory.current").read())
                procs = len(open(f"/sys/fs/cgroup{cg}/cgroup.procs").read().split())
                anon = [l for l in open(f"/sys/fs/cgroup{cg}/memory.stat") if l.startswith("anon ")][0].split()[1]
                say(f"  mem: current={cur / 2**20:.0f} MiB anon={int(anon) / 2**20:.0f} MiB procs={procs}")
            elif op == "shot":
                subprocess.run(["grim", os.path.join(out, arg)], env=env)
    finally:
        try:
            conn.sendall(b'{"cmd":"quit"}\n')
        except OSError:
            pass
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            subprocess.run(["systemctl", "--user", "stop", f"{unit}.scope"])
        subprocess.run(["systemctl", "--user", "stop", f"{unit}.scope"], stderr=subprocess.DEVNULL)
        say("done")


if __name__ == "__main__":
    main()
