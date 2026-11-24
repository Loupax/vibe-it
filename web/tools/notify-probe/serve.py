# C-BELL spike: serves probe.html and sw.js from 127.0.0.1 and saves each run's findings
# (POST /result?tag=NAME) as OUT/NAME.json.  Usage: python3 -I serve.py PORT OUT_DIR
import http.server, sys, os, re, urllib.parse
D = os.path.dirname(os.path.abspath(__file__))
OUT = sys.argv[2]
class H(http.server.SimpleHTTPRequestHandler):
    def __init__(s, *a, **k): super().__init__(*a, directory=D, **k)
    def do_POST(s):
        q = urllib.parse.parse_qs(urllib.parse.urlparse(s.path).query)
        body = s.rfile.read(int(s.headers.get('content-length', 0)))
        os.makedirs(OUT, exist_ok=True)
        name = re.sub(r'[^A-Za-z0-9_-]', '_', q.get('tag', ['run'])[0])
        open(os.path.join(OUT, name + '.json'), 'wb').write(body)
        print('result', name, body.decode(errors='replace'), flush=True)
        s.send_response(204); s.end_headers()
    def log_message(s, fmt, *a): print('http', fmt % a, flush=True)
http.server.ThreadingHTTPServer(('127.0.0.1', int(sys.argv[1])), H).serve_forever()
