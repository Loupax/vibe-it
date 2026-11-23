#!/usr/bin/env python3
"""P-FORGET check: `forget-site` and `--no-window --forget-site` of web-host-cef
(web/PROTOCOL.md, web/STASH.md "Forget site") against local test sites.

    web/host-cef/test/forget.py --out .qwork/forget            # headless (no display needed)
    WAYLAND_DISPLAY=wayland-1 web/host-cef/test/forget.py --out .qwork/forget-wl --wayland

It serves three hostnames from one local server: site-a.test and sub.site-a.test (the
site that is forgotten) and site-b.test (kept). Chromium resolves them to the server
with `--host-resolver-rules` (no /etc/hosts), and treats them as secure contexts
(`--unsafely-treat-insecure-origin-as-secure`) so Cache Storage and service workers
work over http. On each host a fake login (`/login`) sets cookies (one HttpOnly, one
from script; site-a also a `Domain=site-a.test` cookie), localStorage,
sessionStorage, an IndexedDB database, a Cache Storage entry and a service worker that
answers navigations itself (an offline "app shell"). `/check` reports what the page
can see in its title; the server records the cookies each `/check` request carried.

Phases, each with a fresh host process on the same profile:
  1. log in on the three hosts, check all is there; `forget-site` site-a (sent as a
     URL of a subdomain); check site-a and sub.site-a have nothing, site-b everything.
  2. restart: check again from fresh loads (what was forgotten stays gone on disk).
  3. `--no-window --forget-site site-b.test`: start, forget, exit 0, `forgot` on stdout.
  4. restart: site-b has nothing now.
Writes into --out: events-<phase>.jsonl, host-<phase>.log, server.log, profile/.
Exit 0 when every check passed. Every host scope is stopped at the end."""

import argparse
import http.server
import json
import os
import shutil
import socket
import subprocess
import sys
import threading
import time
import urllib.parse

HERE = os.path.dirname(os.path.abspath(__file__))
HOST_DIR = os.path.dirname(HERE)
sys.path.insert(0, HERE)

from drive import Fail, Shell, is_ev  # noqa: E402

# web-host-cef clears the whole HTTP cache (Chromium has no per-site removal it can
# reach; PROTOCOL.md "forget-site"), so the kept site loses its cached responses too.
KEEPS_OTHER_HTTP_CACHE = False

FORGOTTEN = ["site-a.test", "sub.site-a.test"]
KEPT = ["site-b.test"]
HOSTS = FORGOTTEN + KEPT

LOGIN = """<!doctype html><meta charset="utf-8"><title>login</title><script>
(async () => {
  document.cookie = 'js=1; path=/; max-age=86400';
  localStorage.setItem('k', 'v');
  sessionStorage.setItem('s', 'v');
  await new Promise((ok, bad) => {
    const r = indexedDB.open('db', 1);
    r.onupgradeneeded = () => r.result.createObjectStore('o');
    r.onsuccess = () => { const db = r.result; const t = db.transaction('o', 'readwrite');
      t.objectStore('o').put('v', 'k'); t.oncomplete = () => { db.close(); ok(); }; t.onerror = bad; };
    r.onerror = bad;
  });
  const c = await caches.open('c'); await c.put('/cached', new Response('x'));
  await navigator.serviceWorker.register('/sw.js'); await navigator.serviceWorker.ready;
  await (await fetch('/cached.js')).text();  // into the HTTP cache (max-age)
  document.title = 'stored ' + location.host;
})().catch(e => { document.title = 'store failed ' + e; });
</script>"""

CHECK = """<!doctype html><meta charset="utf-8"><title>check</title><script>
(async () => {
  const r = {host: location.host, cookie: document.cookie, ls: localStorage.getItem('k'),
             ss: sessionStorage.getItem('s')};
  r.idb = (await indexedDB.databases()).map(d => d.name);
  r.caches = await caches.keys();
  r.sw = (await navigator.serviceWorker.getRegistrations()).length;
  await (await fetch('/cached.js')).text();  // from the HTTP cache, unless it was cleared
  document.title = 'check ' + JSON.stringify(r);
})().catch(e => { document.title = 'check failed ' + e; });
</script>"""

# An "app shell" service worker: answers every navigation except /login and /check
# itself, as offline-first sites do.
SW = """self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', e => e.waitUntil(clients.claim()));
self.addEventListener('fetch', e => {
  const p = new URL(e.request.url).pathname;
  if (e.request.mode === 'navigate' && p !== '/login' && p !== '/check')
    e.respondWith(new Response('<title>from the service worker</title>app shell',
                               {headers: {'Content-Type': 'text/html'}}));
});"""


class Server(http.server.BaseHTTPRequestHandler):
    log = None
    cookies = {}  # host -> Cookie header of the last /check
    hits = {}  # host -> requests for /cached.js (a response the HTTP cache keeps an hour)
    lock = threading.Lock()

    def do_GET(self):
        host = (self.headers.get("Host") or "").split(":")[0]
        path = urllib.parse.urlsplit(self.path).path
        cookie = self.headers.get("Cookie") or ""
        Server.log.write(f"{time.strftime('%H:%M:%S')} GET {host}{self.path} cookie={cookie!r}\n")
        Server.log.flush()
        extra = []
        if path == "/login":
            body, ctype = LOGIN, "text/html"
            extra.append(("Set-Cookie", f"sid={host}-secret; Path=/; Max-Age=86400"))
            extra.append(("Set-Cookie", "hsid=1; Path=/; HttpOnly; Max-Age=86400"))
            if host == "site-a.test":
                extra.append(("Set-Cookie", "wide=1; Path=/; Domain=site-a.test; Max-Age=86400"))
        elif path == "/check":
            with Server.lock:
                Server.cookies[host] = cookie
            body, ctype = CHECK, "text/html"
        elif path == "/sw.js":
            body, ctype = SW, "text/javascript"
        elif path == "/cached.js":
            with Server.lock:
                Server.hits[host] = Server.hits.get(host, 0) + 1
            body, ctype = "// cached", "text/javascript"
        else:
            body, ctype = "<title>other</title>other", "text/html"
        data = body.encode()
        self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Cache-Control", "max-age=3600" if path == "/cached.js" else "no-store")
        for k, v in extra:
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, fmt, *args):
        pass


def chromium_switches(port):
    origins = ",".join(f"http://{h}" for h in HOSTS)
    return [f"--host-resolver-rules=MAP *.test 127.0.0.1:{port}",
            f"--unsafely-treat-insecure-origin-as-secure={origins}"]


class Host:
    """One host process with a control socket (the shell's end)."""

    def __init__(self, a, out, phase, port, extra=()):
        self.out, self.phase = out, phase
        self.sh = Shell(out)
        self.sh.transcript = open(os.path.join(out, f"events-{phase}.jsonl"), "w", buffering=1)
        self.sock_path = os.path.join(out, "control.sock")
        if os.path.exists(self.sock_path):
            os.unlink(self.sock_path)
        self.lsock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.lsock.bind(self.sock_path)
        self.lsock.listen(1)
        self.lsock.settimeout(60)
        self.unit = f"web-host-forget-{os.getpid()}-{phase}"
        self.proc = start(a, out, phase, self.unit, ["--control", self.sock_path] + chromium_switches(port) + list(extra))
        conn, _ = self.lsock.accept()
        self.conn = conn
        threading.Thread(target=self.sh.reader, args=(conn,), daemon=True).start()
        self.sh.expect("hello", is_ev("hello"))
        self.tab = self.sh.expect("tab-created", is_ev("tab-created"), 60)["tab"]

    def send(self, cmd):
        self.sh.send(self.conn, cmd)

    def title(self, url, prefix, timeout=30):
        """Opens url and returns the first title starting with prefix."""
        self.send({"cmd": "open", "url": url})
        ev = self.sh.expect(f"title {prefix}...", is_ev("title", title=lambda t: (t or "").startswith(prefix)
                                                       or " failed " in (t or "")), timeout)
        if not ev["title"].startswith(prefix):
            raise Fail(f"{url}: {ev['title']}")
        return ev["title"]

    def check(self, host):
        with Server.lock:
            hits = Server.hits.get(host, 0)
        t = self.title(f"http://{host}/check", "check {")
        r = json.loads(t[len("check "):])
        with Server.lock:
            r["server_cookie"] = Server.cookies.get(host, "")
            # True: /cached.js came from the HTTP cache (the server saw no request).
            r["http_cached"] = Server.hits.get(host, 0) == hits
        self.sh.note(f"check {host}: {json.dumps(r)}")
        return r

    def quit(self):
        self.send({"cmd": "quit"})
        self.sh.expect("tab-closed", is_ev("tab-closed"), 20)
        finish(self.proc, self.unit, self.sh)
        self.lsock.close()
        if os.path.exists(self.sock_path):
            os.unlink(self.sock_path)


def start(a, out, phase, unit, args):
    cmd = [os.path.join(HOST_DIR, "run.sh")] + (["--x11"] if a.x11 else []) + args
    if not a.wayland and not a.x11:
        cmd += ["--ozone-platform=headless"]
    env = dict(os.environ, WEB_HOST_UNIT=unit, WEB_HOST_PROFILE=os.path.join(out, "profile"))
    log = open(os.path.join(out, f"host-{phase}.log"), "w")
    print(f"starting host ({phase}): {' '.join(cmd)} (unit {unit}.scope)", flush=True)
    return subprocess.Popen(cmd, env=env, stdout=subprocess.PIPE, stderr=log, text=True)


def finish(proc, unit, sh=None, timeout=30):
    try:
        proc.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        if sh:
            sh.note(f"host still running: stopping {unit}.scope")
        subprocess.run(["systemctl", "--user", "stop", f"{unit}.scope"], check=False)
        proc.wait(timeout=15)


def stored_everything(r, host, session=True, http=True):
    """Problems if the check of host does not show everything the login stored."""
    bad = []
    if http and not r["http_cached"]:
        bad.append("HTTP cache entry missing")
    for name in ("sid", "hsid"):
        if f"{name}=" not in r["server_cookie"]:
            bad.append(f"cookie {name} not sent")
    if "js=1" not in r["cookie"]:
        bad.append("script cookie missing")
    if r["ls"] != "v":
        bad.append("localStorage missing")
    if session and r["ss"] != "v":
        bad.append("sessionStorage missing")
    if "db" not in r["idb"]:
        bad.append("IndexedDB missing")
    if "c" not in r["caches"]:
        bad.append("Cache Storage missing")
    if r["sw"] < 1:
        bad.append("service worker missing")
    return [f"{host}: {b}" for b in bad]


def stored_nothing(r, host, session=True, http=True):
    """Problems if the check of host shows anything of the login."""
    bad = []
    if http and r["http_cached"]:
        bad.append("HTTP cache entry still there")
    if r["server_cookie"]:
        bad.append(f"cookies still sent: {r['server_cookie']}")
    if r["cookie"]:
        bad.append(f"document.cookie still {r['cookie']!r}")
    if r["ls"] is not None:
        bad.append("localStorage still there")
    if session and r["ss"] is not None:
        bad.append("sessionStorage still there")
    if r["idb"]:
        bad.append(f"IndexedDB still {r['idb']}")
    if r["caches"]:
        bad.append(f"Cache Storage still {r['caches']}")
    if r["sw"]:
        bad.append(f"{r['sw']} service worker(s) still registered")
    return [f"{host}: {b}" for b in bad]


def expect_none(problems, sh):
    if problems:
        raise Fail("; ".join(problems))


def run(a):
    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)
    profile = os.path.join(out, "profile")
    if os.path.isdir(profile):
        shutil.rmtree(profile)  # a clean profile per run
    Server.log = open(os.path.join(out, "server.log"), "w")
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Server)
    srv.daemon_threads = True
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    port = srv.server_address[1]
    print(f"test sites {HOSTS} at 127.0.0.1:{port}", flush=True)
    ok = False
    try:
        # Phase 1: log in everywhere, forget site-a while its page is shown.
        h = Host(a, out, "1", port)
        for host in HOSTS:
            h.title(f"http://{host}/login", f"stored {host}")
        for host in KEPT + ["sub.site-a.test", "site-a.test"]:
            expect_none(stored_everything(h.check(host), host), h.sh)
        h.sh.note("all three hosts have cookies, storage and a service worker; forgetting site-a")
        h.send({"cmd": "forget-site", "site": "https://sub.site-a.test/some/page"})
        ev = h.sh.expect("forgot", is_ev("forgot"), 120)
        h.sh.note(f"forgot: {json.dumps(ev)}")
        if ev["result"] != "ok" or ev["domain"] != "site-a.test" or ev["site"] != "https://sub.site-a.test/some/page":
            raise Fail(f"forget-site answered {ev}")
        if ev["cookies"] < 7:  # sid, hsid, js on both hosts + wide
            raise Fail(f"only {ev['cookies']} cookies deleted")
        for o in ("http://site-a.test", "http://sub.site-a.test"):
            if o not in ev["origins"]:
                raise Fail(f"{o} not among the cleared origins")
        if any("site-b" in o for o in ev["origins"]):
            raise Fail("site-b's storage was cleared")
        # sessionStorage lives in the page's session: only the frame that showed
        # site-a was cleared (sub.site-a's entry stays until the page closes).
        expect_none(stored_nothing(h.check("site-a.test"), "site-a.test"), h.sh)
        expect_none(stored_nothing(h.check("sub.site-a.test"), "sub.site-a.test", session=False), h.sh)
        r = h.check("site-b.test")
        expect_none(stored_everything(r, "site-b.test", http=KEEPS_OTHER_HTTP_CACHE), h.sh)
        h.sh.note(f"site-b's HTTP cache entry after forgetting site-a: {'kept' if r['http_cached'] else 'gone'}")
        h.send({"cmd": "forget-site", "site": "com"})
        h.sh.expect("error for forget-site com", is_ev("error", message=lambda m: "forget-site" in (m or "")))
        h.sh.note("phase 1 passed: site-a forgotten, site-b intact")
        h.quit()

        # Phase 2: a fresh process, fresh loads.
        h = Host(a, out, "2", port)
        for host in FORGOTTEN:
            # Phase 1's checks after the forget fetched /cached.js into the cache again.
            expect_none(stored_nothing(h.check(host), host, http=False), h.sh)
        expect_none(stored_everything(h.check("site-b.test"), "site-b.test", session=False), h.sh)
        h.sh.note("phase 2 passed: after a restart site-a still has nothing, site-b everything")
        h.quit()
        with open(os.path.join(out, "profile", "web-host-origins")) as f:
            rec = f.read()
        print(f"origin record after phase 2:\n{rec}", flush=True)

        # Phase 3: --no-window --forget-site.
        unit = f"web-host-forget-{os.getpid()}-3"
        proc = start(a, out, "3", unit, ["--no-window", "--forget-site", "site-b.test"] + chromium_switches(port))
        stdout, _ = proc.communicate(timeout=120)
        print(f"--no-window exit {proc.returncode}, stdout: {stdout.strip()}", flush=True)
        if proc.returncode != 0:
            raise Fail(f"--no-window --forget-site exited {proc.returncode}")
        ev = json.loads(stdout.strip().splitlines()[-1])
        if ev.get("event") != "forgot" or ev["result"] != "ok" or ev["domain"] != "site-b.test" or ev["cookies"] < 3:
            raise Fail(f"--no-window printed {ev}")
        if "site-b.test" in rec and "site-b.test" in open(os.path.join(out, "profile", "web-host-origins")).read():
            raise Fail("site-b is still in the origin record")

        # Phase 4: site-b is gone too.
        h = Host(a, out, "4", port)
        expect_none(stored_nothing(h.check("site-b.test"), "site-b.test"), h.sh)
        h.sh.note("phase 4 passed: --no-window forgot site-b")
        h.quit()
        ok = True
    except (Fail, AssertionError, KeyError, TypeError, ValueError, subprocess.TimeoutExpired) as e:
        print(f"FAIL: {e}", flush=True)
    finally:
        for phase in "1234":
            subprocess.run(["systemctl", "--user", "stop", f"web-host-forget-{os.getpid()}-{phase}.scope"],
                           check=False, stderr=subprocess.DEVNULL)
        srv.shutdown()
    print("PASS" if ok else "FAILED", flush=True)
    return 0 if ok else 1


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True, help="output dir (use one under .qwork/); its profile/ is reused")
    ap.add_argument("--wayland", action="store_true", help="show the page on WAYLAND_DISPLAY (a nested compositor)")
    ap.add_argument("--x11", action="store_true", help="show the page on DISPLAY")
    a = ap.parse_args()
    if a.wayland and not os.environ.get("WAYLAND_DISPLAY"):
        sys.exit("forget.py: --wayland needs WAYLAND_DISPLAY (a nested compositor, scratchy/tools/nest.sh)")
    return run(a)


if __name__ == "__main__":
    sys.exit(main())
