#!/usr/bin/env python3
"""P-COOKIES check: web-host-cef blocks third-party cookies by default (web/STASH.md
"Cookies", web/PROTOCOL.md "What every engine host implements").

    web/host-cef/test/cookies.py --out .qwork/cookies            # headless (no display needed)

Two sites served over https by one local server (a self-signed certificate, so
`SameSite=None; Secure` cookies, the only kind a third party can use, are possible):
site-a.test (the embedding page) and site-b.test (the third party). Chromium resolves
`*.test` to the server with `--host-resolver-rules`, and ignores the certificate.

Two runs, each a fresh host process on a fresh profile:
  block  the default: third-party cookies blocked.
  allow  `--allow-third-party-cookies`: the control, proving the page really can set
         and read a third-party cookie when the policy lets it (else "blocked" would
         prove nothing).
In each run:
  1. site-b as the top-level page logs in (cookies by header, by script, and a
     `Partitioned` one); it reads them back.
  2. site-a embeds a site-b iframe and a site-b image. The frame and the image try to
     set cookies (header and script, plain and `Partitioned`) and report which of
     site-b's cookies they can read, in script and as the Cookie header the server got.
  3. site-b as the top-level page again: it must have its own cookies and none of the
     third-party context's plain ones.
  4. site-a's embed once more: a `Partitioned` cookie from step 2 is read back in the
     frame (CHIPS), when the browser supports it in the third-party context.
Writes into --out/<run>/: events-*.jsonl, host-*.log, profile/; server.log in --out.
Exit 0 when every check passed. Every host scope is stopped at the end."""

import argparse
import http.server
import json
import os
import shutil
import ssl
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import forget  # noqa: E402  (its Host class drives one host process over the control socket)
from drive import Fail  # noqa: E402

# site-b's own cookies (set when it is the top-level page).
OWN = ("sid", "jsid", "chips")
# What the third-party context (frame / image on site-a) tries to set on site-b.
TP_PLAIN = ("tp_hdr", "tp_js", "tp_img")
TP_PART = ("tpp_hdr", "tpp_js")

LOGIN = """<!doctype html><meta charset="utf-8"><title>login</title><script>
document.cookie = 'jsid=1; path=/; max-age=86400; SameSite=None; Secure';
document.title = 'login ' + document.cookie;
</script>"""

CHECK = """<!doctype html><meta charset="utf-8"><title>check</title><script>
document.title = 'check ' + JSON.stringify({host: location.host, cookie: document.cookie, ls: localStorage.getItem('k')});
</script>"""

# site-b inside site-a's page: set cookies, report what it can read.
FRAME = """<!doctype html><meta charset="utf-8"><script>
const before = document.cookie;
// Storage partitioning: what an earlier visit under this top-level site stored.
const ls0 = localStorage.getItem('k');
localStorage.setItem('k', location.ancestorOrigins[0]);
document.cookie = 'tp_js=1; path=/; max-age=86400; SameSite=None; Secure';
document.cookie = 'tpp_js=1; path=/; max-age=86400; SameSite=None; Secure; Partitioned';
parent.postMessage(JSON.stringify({before, after: document.cookie, ls0}), '*');
</script>"""

EMBED = """<!doctype html><meta charset="utf-8"><title>embed</title>
<iframe src="https://site-b.test/frame"></iframe>
<img src="https://site-b.test/pixel" onload="done()" onerror="done()">
<script>
let img = false, frame = null;
function finish() { if (img && frame) document.title = 'embed ' + frame; }
function done() { img = true; finish(); }
addEventListener('message', e => { frame = e.data; finish(); });
</script>"""


def cookie_names(header):
    """Names in a Cookie header or document.cookie string."""
    return {p.split("=", 1)[0].strip() for p in (header or "").split(";") if p.strip()}


class Server(http.server.BaseHTTPRequestHandler):
    log = None
    seen = {}  # path -> Cookie header of the last request for it
    lock = threading.Lock()

    def do_GET(self):
        host = (self.headers.get("Host") or "").split(":")[0]
        path = urllib.parse.urlsplit(self.path).path
        cookie = self.headers.get("Cookie") or ""
        Server.log.write(f"{time.strftime('%H:%M:%S')} GET {host}{self.path} cookie={cookie!r} "
                         f"site-for-cookies/referer={self.headers.get('Referer')!r}\n")
        Server.log.flush()
        with Server.lock:
            Server.seen[f"{host}{path}"] = cookie
        extra = []
        tp = "Path=/; Max-Age=86400; SameSite=None; Secure"
        if host == "site-b.test" and path == "/login":
            body = LOGIN
            extra.append(("Set-Cookie", f"sid=1; {tp}"))
            extra.append(("Set-Cookie", f"chips=1; {tp}; Partitioned"))
        elif host == "site-b.test" and path == "/check":
            body = CHECK
        elif host == "site-b.test" and path == "/frame":
            body = FRAME
            extra.append(("Set-Cookie", f"tp_hdr=1; {tp}"))
            extra.append(("Set-Cookie", f"tpp_hdr=1; {tp}; Partitioned"))
        elif host == "site-b.test" and path == "/pixel":
            body = ""
            extra.append(("Set-Cookie", f"tp_img=1; {tp}"))
        elif host != "site-b.test" and path == "/embed":  # site-a and site-c
            body = EMBED
        else:
            body = "<title>other</title>other"
        data = body.encode()
        self.send_response(200)
        self.send_header("Content-Type", "image/gif" if path == "/pixel" else "text/html")
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Cache-Control", "no-store")
        for k, v in extra:
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, fmt, *args):
        pass


def make_cert(out):
    key, crt = os.path.join(out, "key.pem"), os.path.join(out, "cert.pem")
    if not os.path.exists(crt):
        subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", key,
                        "-out", crt, "-days", "2", "-subj", "/CN=test", "-addext",
                        "subjectAltName=DNS:site-a.test,DNS:site-b.test"],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return crt, key


def chromium_switches(port):
    return [f"--host-resolver-rules=MAP *.test 127.0.0.1:{port}", "--ignore-certificate-errors"]


def judge(mode, own_before, frame, frame_hdr, pixel_hdr, own_after, frame_again, chips_visible):
    """Problems (a list of strings) in one run's observations; pure, unit-tested.
    mode: "block" or "allow". Every other argument is a set of cookie names, except
    chips_visible (True/False/None: the partitioned cookie was seen in the third-party
    context when it was embedded again; None: not asked)."""
    bad = []
    if own_before != set(OWN):
        bad.append(f"site-b top-level logged in but sees {sorted(own_before)}, expected {sorted(OWN)}")
    tp_set = set(TP_PLAIN) | set(TP_PART)
    if mode == "block":
        # The embedded site-b sees none of the cookies it set as a top-level page ...
        leaked = (frame["before"] | frame_hdr | pixel_hdr) & set(OWN)
        if leaked:
            bad.append(f"third-party context could read site-b's cookies {sorted(leaked)}")
        # ... cannot set plain cookies, by header or script ...
        stored = frame["after"] & set(TP_PLAIN)
        if stored:
            bad.append(f"third-party context set plain cookies {sorted(stored)}")
        # ... and what it tried to set never reaches site-b's own jar.
        landed = own_after & (set(TP_PLAIN) | set(TP_PART))
        if landed:
            bad.append(f"site-b's own jar got third-party cookies {sorted(landed)}")
        if own_after != set(OWN):
            bad.append(f"site-b top-level lost or gained cookies: {sorted(own_after)}")
    else:
        # The control: the same page really can use third-party cookies when allowed.
        if not set(OWN) - {"chips"} <= frame_hdr:
            bad.append(f"control: frame request lacked site-b's cookies ({sorted(frame_hdr)})")
        if not {"tp_hdr", "tp_js"} <= frame["after"]:
            bad.append(f"control: frame could not set cookies ({sorted(frame['after'])})")
        if not {"tp_hdr", "tp_js", "tp_img"} <= own_after:
            bad.append(f"control: site-b top-level lacks the third-party cookies ({sorted(own_after)})")
    return bad


def judge_partitions(mode, frame, other_cookies, other_ls, back_ls, top_ls):
    """Problems in the partitioning of what the frame stored (both modes: storage
    partitioning is on whatever the cookie policy). frame: the first embed under
    site-a (its "before" cookie names); other_*: the same frame under site-c;
    back_ls: under site-a again; top_ls: site-b as the top-level page."""
    bad = []
    if other_ls is not None:
        bad.append(f"frame under site-c read localStorage {other_ls!r} stored under site-a")
    if back_ls != "https://site-a.test":
        bad.append(f"frame under site-a lost its own partition's localStorage ({back_ls!r})")
    if top_ls is not None:
        bad.append(f"site-b top-level read the frame's localStorage {top_ls!r}")
    part = set(TP_PART)
    if mode == "block" and other_cookies & part:
        bad.append(f"Partitioned cookies of site-a's partition seen under site-c: {sorted(other_cookies & part)}")
    return bad


def check_header(host, path):
    with Server.lock:
        return cookie_names(Server.seen.get(f"{host}{path}"))


class ShortSockHost(forget.Host):
    """forget.Host with its control socket in a short path: AF_UNIX paths are limited to
    ~107 bytes and a worktree's .qwork path is longer. The profile stays under out."""

    def __init__(self, a, out, phase, port, extra=()):
        self.short = tempfile.mkdtemp(prefix="ck")
        real_join = os.path.join
        orig = forget.os.path.join

        def join(*p):
            return real_join(self.short, p[-1]) if p[-1] == "control.sock" else real_join(*p)
        forget.os.path.join = join
        try:
            super().__init__(a, out, phase, port, extra)
        finally:
            forget.os.path.join = orig


def run_mode(a, out, port, mode):
    rdir = os.path.join(out, mode)
    if os.path.isdir(rdir):
        shutil.rmtree(rdir)
    os.makedirs(rdir)
    extra = ["--allow-third-party-cookies"] if mode == "allow" else []
    extra += a.chromium
    h = ShortSockHost(a, rdir, mode, port, extra)
    try:
        h.title("https://site-b.test/login", "login ")
        own_before = cookie_names(json.loads(h.title("https://site-b.test/check", "check {")[6:])["cookie"])
        h.sh.note(f"{mode}: site-b top-level sees {sorted(own_before)}")
        t = h.title("https://site-a.test/embed", "embed {")
        frame = json.loads(t[len("embed "):])
        frame = {"before": cookie_names(frame["before"]), "after": cookie_names(frame["after"])}
        frame_hdr = check_header("site-b.test", "/frame")
        pixel_hdr = check_header("site-b.test", "/pixel")
        h.sh.note(f"{mode}: frame read before={sorted(frame['before'])} after={sorted(frame['after'])}; "
                  f"frame request carried {sorted(frame_hdr)}, image request {sorted(pixel_hdr)}")
        own_after = cookie_names(json.loads(h.title("https://site-b.test/check", "check {")[6:])["cookie"])
        chk = json.loads(h.title("https://site-b.test/check", "check {")[6:])
        top_ls = chk["ls"]
        h.sh.note(f"{mode}: site-b top-level now sees {sorted(own_after)}, localStorage {top_ls!r}")
        # CHIPS: embed again; a partitioned cookie from the first embed is sent to the frame.
        h.title("https://site-a.test/embed", "embed {")
        again = check_header("site-b.test", "/frame")
        chips_header = "tpp_hdr" in again
        chips_js = "tpp_js" in again
        h.sh.note(f"{mode}: second embed: partitioned cookies sent to the frame: header-set={chips_header} "
                  f"script-set={chips_js} (all sent: {sorted(again)})")
        # A third top-level site (site-c) embedding the same frame: its own partition.
        t = h.title("https://site-c.test/embed", "embed {")
        other = json.loads(t[len("embed "):])
        # The frame's own response sets tpp_hdr before its script reads document.cookie,
        # so only the request's Cookie header (and the script-set one) show a leak.
        other_cookies = check_header("site-b.test", "/frame") | (cookie_names(other["before"]) & {"tpp_js"})
        h.sh.note(f"{mode}: embedded under site-c: frame read cookies {other['before']!r}, localStorage {other['ls0']!r}")
        t = h.title("https://site-a.test/embed", "embed {")
        back = json.loads(t[len("embed "):])
        h.sh.note(f"{mode}: embedded under site-a again: localStorage {back['ls0']!r}")
        bad = judge(mode, own_before, frame, frame_hdr, pixel_hdr, own_after, again, None)
        bad += judge_partitions(mode, frame, other_cookies, other["ls0"], back["ls0"], top_ls)
        return bad, {"chips_header": chips_header, "chips_js": chips_js, "again": sorted(again)}
    finally:
        try:
            h.quit()
        except Exception as e:  # the scope is stopped by run() anyway
            print(f"quit failed: {e}", flush=True)
        shutil.rmtree(h.short, ignore_errors=True)


def run(a):
    out = os.path.abspath(a.out)
    os.makedirs(out, exist_ok=True)
    crt, key = make_cert(out)
    Server.log = open(os.path.join(out, "server.log"), "w")
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Server)
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.load_cert_chain(crt, key)
    srv.socket = ctx.wrap_socket(srv.socket, server_side=True)
    srv.daemon_threads = True
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    port = srv.server_address[1]
    forget.chromium_switches = chromium_switches
    print(f"test sites site-a.test, site-b.test (https) at 127.0.0.1:{port}", flush=True)
    ok = True
    try:
        for mode in ("block", "allow"):
            with Server.lock:
                Server.seen.clear()
            bad, chips = run_mode(a, out, port, mode)
            print(f"{mode}: {'ok' if not bad else 'PROBLEMS: ' + '; '.join(bad)}; CHIPS {chips}", flush=True)
            ok = ok and not bad
            if mode == "block" and not (chips["chips_header"] or chips["chips_js"]):
                print("note: no Partitioned cookie came back in the third-party context while "
                      "third-party cookies were blocked (CHIPS not usable there)", flush=True)
                if a.require_chips:
                    ok = False
    except (Fail, AssertionError, KeyError, TypeError, ValueError, subprocess.TimeoutExpired) as e:
        print(f"FAIL: {e}", flush=True)
        ok = False
    finally:
        for mode in ("block", "allow"):
            subprocess.run(["systemctl", "--user", "stop", f"web-host-forget-{os.getpid()}-{mode}.scope"],
                           check=False, stderr=subprocess.DEVNULL)
        srv.shutdown()
    print("PASS" if ok else "FAILED", flush=True)
    return 0 if ok else 1


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True, help="output dir (use one under .qwork/)")
    ap.add_argument("--wayland", action="store_true", help="show the page on WAYLAND_DISPLAY (a nested compositor)")
    ap.add_argument("--x11", action="store_true", help="show the page on DISPLAY")
    ap.add_argument("--require-chips", action="store_true", help="fail if a Partitioned cookie does not work")
    a, rest = ap.parse_known_args()
    a.chromium = rest
    if a.wayland and not os.environ.get("WAYLAND_DISPLAY"):
        sys.exit("cookies.py: --wayland needs WAYLAND_DISPLAY (a nested compositor, web/tools/nest.sh)")
    return run(a)


if __name__ == "__main__":
    sys.exit(main())
