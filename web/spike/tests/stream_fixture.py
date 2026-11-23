#!/usr/bin/env python3
"""W5 fixture: streaming and large bodies, on loopback, behind your own wire.

    python3 stream_fixture.py PORT LOGFILE [VIDEO.webm]

Routes (http://localhost:PORT):
  /video.html          <video> that seeks around and logs how long each seek took (console.log)
  /video.webm          VIDEO with Range support (206 + Content-Range); every request is logged
  /big.bin?mb=150&rate=5   deterministic body (1 MiB pattern repeated), Content-Length,
                       Content-Disposition: attachment, throttled to `rate` MB/s
  /big.sha256?mb=150   the expected SHA-256 of /big.bin?mb=150
  /cut.bin?mb=50&cut=10&rate=20   announces mb MB, sends `cut` MB, then drops the connection
  /longpoll?s=65       holds the request open `s` seconds, then answers
  /poll.html           long-poll + a 1 s ping loop, logs gaps (console.log)
  /ping                answers at once
"""
import hashlib, random, sys, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse, parse_qs

PORT = int(sys.argv[1])
LOG = open(sys.argv[2], "a", buffering=1)
VIDEO = sys.argv[3] if len(sys.argv) > 3 else None
BLOCK = random.Random(1).randbytes(1 << 20)
T0 = time.time()


def log(msg):
    LOG.write(f"[{time.time() - T0:8.2f}] {msg}\n")


def sha_for(mb):
    h = hashlib.sha256()
    for _ in range(mb):
        h.update(BLOCK)
    return h.hexdigest()


VIDEO_PAGE = """<!doctype html><title>video</title><video id=v muted autoplay width=640 controls></video>
<script>
const v = document.getElementById('v');
v.src = new URLSearchParams(location.search).get('src') || '/video.webm';
const T = performance.now();
const say = m => console.log('VIDEO +' + Math.round(performance.now() - T) + 'ms ' + m);
v.addEventListener('loadedmetadata', () => { say('metadata duration=' + v.duration); targets = fr.map(f => Math.floor(f * v.duration)); });
v.addEventListener('waiting', () => say('waiting at ' + v.currentTime.toFixed(2)));
v.addEventListener('error', () => say('ERROR ' + (v.error && v.error.code) + ' ' + (v.error && v.error.message)));
const fr = [0.5, 0.08, 0.83, 0.25, 0.92, 0.04, 0.67, 0.17];
let targets = [];
let i = 0;
function seek() {
  if (i >= targets.length) { say('DONE'); return; }
  const to = targets[i++], t0 = performance.now(), from = v.currentTime;
  v.currentTime = to;
  // resumed = time advanced at least 0.3 s past the target after the seek
  const iv = setInterval(() => {
    if (!v.seeking && v.readyState >= 3 && v.currentTime >= to + 0.3) {
      clearInterval(iv);
      say('seek ' + from.toFixed(1) + ' -> ' + to + ' resumed after ' + Math.round(performance.now() - t0) + ' ms');
      setTimeout(seek, 1500);
    } else if (performance.now() - t0 > 20000) { clearInterval(iv); say('seek to ' + to + ' STALLED >20s'); setTimeout(seek, 500); }
  }, 20);
}
v.addEventListener('playing', function once() { v.removeEventListener('playing', once); say('playing'); setTimeout(seek, 3000); });
</script>"""

POLL_PAGE = """<!doctype html><title>poll</title><pre id=o></pre>
<script>
const T = performance.now();
const say = m => console.log('POLL +' + Math.round(performance.now() - T) + 'ms ' + m);
const s = new URLSearchParams(location.search).get('s') || '65';
fetch('/longpoll?s=' + s).then(r => r.text()).then(t => say('LONGPOLL answered: ' + t.trim()), e => say('LONGPOLL FAILED ' + e));
let n = 0, last = performance.now(), worst = 0;
const iv = setInterval(async () => {
  const t0 = performance.now();
  try { await (await fetch('/ping?n=' + n++)).text(); } catch (e) { say('PING FAILED ' + e); }
  worst = Math.max(worst, performance.now() - t0);
  if (n % 15 == 0) say('pings ' + n + ', worst ping ' + Math.round(worst) + ' ms');
}, 1000);
</script>"""


RANGE_PAGE = """<!doctype html><title>range</title><script>
const say = m => console.log('RANGE ' + m);
(async () => {
  for (const r of ['bytes=100-199', 'bytes=62324736-', 'bytes=-1000', 'bytes=0-']) {
    try {
      const res = await fetch('/video.webm', {headers: {Range: r}});
      let n = 0;
      if (r == 'bytes=0-') { const rd = res.body.getReader(); const { value } = await rd.read(); n = value.length; rd.cancel(); }
      else n = (await res.arrayBuffer()).byteLength;
      say(r + ' -> ' + res.status + ' Content-Range=' + res.headers.get('content-range') + ' Content-Length=' + res.headers.get('content-length') + ' body=' + n);
    } catch (e) { say(r + ' FAILED ' + e); }
  }
})();
</script>"""


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def send_bytes(self, code, body, ctype):
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def throttled(self, total, rate_mb):
        sent = 0
        t0 = time.time()
        while sent < total:
            off = sent % len(BLOCK)
            chunk = BLOCK[off : off + min(256 * 1024, total - sent, len(BLOCK) - off)]
            self.wfile.write(chunk)
            sent += len(chunk)
            ahead = sent / (rate_mb * 1048576) - (time.time() - t0)
            if ahead > 0:
                time.sleep(ahead)
        return sent

    def do_GET(self):
        u = urlparse(self.path)
        q = {k: v[0] for k, v in parse_qs(u.query).items()}
        if u.path == "/video.html":
            return self.send_bytes(200, VIDEO_PAGE.encode(), "text/html")
        if u.path == "/poll.html":
            return self.send_bytes(200, POLL_PAGE.encode(), "text/html")
        if u.path == "/range.html":
            return self.send_bytes(200, RANGE_PAGE.encode(), "text/html")
        if u.path == "/ping":
            return self.send_bytes(200, b"pong", "text/plain")
        if u.path == "/longpoll":
            s = float(q.get("s", "65"))
            log(f"longpoll start {s}s")
            time.sleep(s)
            log("longpoll answer")
            return self.send_bytes(200, f"answered after {s}s\n".encode(), "text/plain")
        if u.path == "/big.sha256":
            return self.send_bytes(200, sha_for(int(q.get("mb", "150"))).encode(), "text/plain")
        if u.path in ("/big.bin", "/cut.bin"):
            mb = int(q.get("mb", "150"))
            rate = float(q.get("rate", "5"))
            total = mb << 20
            send = (int(q.get("cut", "10")) << 20) if u.path == "/cut.bin" else total
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(total))
            self.send_header("Content-Disposition", f'attachment; filename="{u.path[1:]}"')
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            log(f"{u.path} start total={total} will_send={send} rate={rate}MB/s")
            try:
                n = self.throttled(send, rate)
                log(f"{u.path} sent {n}")
            except (BrokenPipeError, ConnectionResetError):
                log(f"{u.path} client went away")
            if send < total:
                self.close_connection = True
            return
        if u.path == "/video.webm" and VIDEO:
            size = __import__("os").path.getsize(VIDEO)
            rng = self.headers.get("Range")
            start, end, code = 0, size - 1, 200
            if rng and rng.startswith("bytes="):
                a, _, b = rng[6:].partition("-")
                if a == "":
                    start = max(0, size - int(b))
                else:
                    start = int(a)
                    end = int(b) if b else size - 1
                end = min(end, size - 1)
                code = 206
            log(f"video GET Range={rng!r} -> {code} {start}-{end}/{size}")
            self.send_response(code)
            self.send_header("Content-Type", "video/webm")
            self.send_header("Accept-Ranges", "bytes")
            self.send_header("Content-Length", str(end - start + 1))
            if code == 206:
                self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            try:
                with open(VIDEO, "rb") as f:
                    f.seek(start)
                    left = end - start + 1
                    while left > 0:
                        d = f.read(min(256 * 1024, left))
                        if not d:
                            break
                        self.wfile.write(d)
                        left -= len(d)
            except (BrokenPipeError, ConnectionResetError):
                log(f"video range {start}-{end} aborted by client (normal on seek)")
            return
        self.send_bytes(404, b"not found", "text/plain")


ThreadingHTTPServer.daemon_threads = True
ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
