# C-WALLET: tiny local HTTP server for the probe: sets one persistent cookie and logs the Cookie header it gets back (arg 2: log file), serves a blank page.
import sys, http.server
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if len(sys.argv) > 2 and self.path == '/':
            open(sys.argv[2], 'a').write('GET / Cookie: %r\n' % self.headers.get('Cookie'))
        self.send_response(200)
        self.send_header('Content-Type', 'text/html')
        self.send_header('Set-Cookie', 'probe=SECRETVALUE123456; Max-Age=86400; Path=/')
        self.end_headers()
        self.wfile.write(b'<!doctype html><title>wallet probe</title><p>cookie set</p>')
    def log_message(self, *a): pass
http.server.HTTPServer(('127.0.0.1', int(sys.argv[1])), H).serve_forever()
