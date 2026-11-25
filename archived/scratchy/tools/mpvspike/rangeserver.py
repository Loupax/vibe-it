# Tiny Range-capable static file server for the QM1b spike. Usage:
#   python3 -I rangeserver.py DIR PORT   (binds 127.0.0.1 only)
# Every served request is logged to stderr: "REQ <range header> -> <status> <bytes>".
import http.server, os, sys

ROOT, PORT = sys.argv[1], int(sys.argv[2])

class H(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.0"
    def log_message(self, *a):
        pass
    def do_GET(self):
        p = os.path.join(ROOT, os.path.basename(self.path))
        if not os.path.isfile(p):
            self.send_error(404); return
        size = os.path.getsize(p)
        rng = self.headers.get("Range")
        lo, hi, status = 0, size - 1, 200
        if rng and rng.startswith("bytes="):
            a, _, b = rng[6:].partition("-")
            lo = int(a) if a else max(0, size - int(b))
            hi = min(int(b), size - 1) if a and b else size - 1
            if lo >= size:
                self.send_response(416)
                self.send_header("Content-Range", "bytes */%d" % size)
                self.end_headers(); return
            status = 206
        n = hi - lo + 1
        self.send_response(status)
        self.send_header("Content-Type", "video/mp4")
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Length", str(n))
        if status == 206:
            self.send_header("Content-Range", "bytes %d-%d/%d" % (lo, hi, size))
        self.end_headers()
        with open(p, "rb") as f:
            f.seek(lo)
            self.wfile.write(f.read(n))
        sys.stderr.write("REQ %s -> %d %d\n" % (rng, status, n)); sys.stderr.flush()

http.server.ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
