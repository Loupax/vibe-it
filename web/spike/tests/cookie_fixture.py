#!/usr/bin/env python3
"""Local cookie fixture for web/spike W4 (stdlib only, loopback only).

    cookie_fixture.py PORT LOGFILE

A tiny login flow plus the cookie kinds a real site uses. Browse it as
http://localhost:PORT/ (first party) and it embeds http://127.0.0.1:PORT/ (third
party: a different site). Every request is appended to LOGFILE as one line:
host, path, the NAMES of the cookies the browser sent, and any X-Wire-* request
header names (wire strips those before forwarding, so none can arrive; the
spike's own WEB_SPIKE_LOG_HEADERS shows what it sends to wire).

  /login        form (user=test, pass=test); the spike's WEB_SPIKE_JS submits it
  POST /login   302 -> /home, with Set-Cookie on the redirect hop (like real sites):
                  sid      persistent (Max-Age), HttpOnly, SameSite=Lax   (the "login")
                  persist  persistent
                  sessck   session (no Max-Age/Expires)
                  httponly persistent, HttpOnly
                  secure   persistent, Secure
                  lax / strict / none  persistent, SameSite=Lax / Strict / None+Secure
  /home         "logged in" when a valid sid arrived, else "logged out"; shows
                document.cookie; embeds the third-party /tp and (a second later) /tp2
  /tp           third-party response: Set-Cookie tp_default (no SameSite),
                tp_none (SameSite=None; Secure)
  /tp2          third-party request after /tp: logs which cookies came back
  /other        a plain page
"""
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs

PORT = int(sys.argv[1])
LOG = sys.argv[2]
SID = "sid-test-session-1"
AGE = "Max-Age=86400"


def names(cookie_header):
    return sorted(p.split("=", 1)[0].strip() for p in (cookie_header or "").split(";") if p.strip())


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def record(self):
        host = self.headers.get("Host", "")
        xw = sorted(k for k in self.headers.keys() if k.lower().startswith("x-wire"))
        line = (f"REQ {self.command} host={host} path={self.path} "
                f"cookies={','.join(names(self.headers.get('Cookie'))) or '-'} xwire={','.join(xw) or '-'}\n")
        with open(LOG, "a") as f:
            f.write(line)

    def send(self, status, body=b"", headers=(), ctype="text/html; charset=utf-8"):
        self.send_response(status)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        for k, v in headers:
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(body)

    def other_host(self):
        host = self.headers.get("Host", "")
        name, _, port = host.partition(":")
        return ("127.0.0.1" if name == "localhost" else "localhost") + (":" + port if port else "")

    def do_GET(self):
        self.record()
        path = self.path.split("?")[0]
        if path == "/login":
            self.send(200, b"<title>login</title><form method=post action=/login>"
                      b"<input name=user value=test><input name=pass value=test><button>go</button></form>")
        elif path == "/home":
            sid = None
            for part in (self.headers.get("Cookie") or "").split(";"):
                k, _, v = part.strip().partition("=")
                if k == "sid":
                    sid = v
            state = "logged in as test" if sid == SID else "logged out"
            tp = self.other_host()
            body = (f"<title>{state}</title><h1>{state}</h1><pre id=c></pre>"
                    f"<script>document.getElementById('c').textContent='document.cookie='+document.cookie;"
                    f"</script><img src='http://{tp}/tp'>"
                    f"<script>setTimeout(function(){{var i=new Image();i.src='http://{tp}/tp2';}},1000)</script>").encode()
            self.send(200, body)
        elif path == "/tp":
            self.send(200, b"GIF89a", [("Set-Cookie", f"tp_default=1; {AGE}; Path=/"),
                                      ("Set-Cookie", f"tp_none=1; {AGE}; Path=/; SameSite=None; Secure")],
                      ctype="image/gif")
        elif path == "/tp2":
            self.send(200, b"GIF89a", ctype="image/gif")
        elif path == "/other":
            self.send(200, b"<title>other</title>")
        else:
            self.send(404, b"not found")

    def do_POST(self):
        self.record()
        n = int(self.headers.get("Content-Length") or 0)
        form = parse_qs(self.rfile.read(n).decode())
        if self.path.split("?")[0] == "/login" and form.get("user") == ["test"] and form.get("pass") == ["test"]:
            self.send(302, b"", [
                ("Location", "/home"),
                ("Set-Cookie", f"sid={SID}; {AGE}; Path=/; HttpOnly; SameSite=Lax"),
                ("Set-Cookie", f"persist=1; {AGE}; Path=/"),
                ("Set-Cookie", "sessck=1; Path=/"),
                ("Set-Cookie", f"httponly=1; {AGE}; Path=/; HttpOnly"),
                ("Set-Cookie", f"secure=1; {AGE}; Path=/; Secure"),
                ("Set-Cookie", f"lax=1; {AGE}; Path=/; SameSite=Lax"),
                ("Set-Cookie", f"strict=1; {AGE}; Path=/; SameSite=Strict"),
                ("Set-Cookie", f"none=1; {AGE}; Path=/; SameSite=None; Secure"),
            ])
        else:
            self.send(403, b"bad login")


ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
