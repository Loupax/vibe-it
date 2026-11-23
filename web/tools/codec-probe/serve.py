# C-WEBEX capability probe server: serves probe.html; the page POSTs its findings, saved as
# OUT/<tag>-<name>.json.  Usage: python3 -I serve.py PORT [OUT_DIR]   (OUT_DIR default: ./out next to this file;
# use one under .qwork/). Open http://127.0.0.1:PORT/probe.html?tag=NAME in the browser to test.
import http.server, sys, os, re, urllib.parse
D = os.path.dirname(os.path.abspath(__file__))
OUT = sys.argv[2] if len(sys.argv) > 2 else os.path.join(D, "out")
class H(http.server.SimpleHTTPRequestHandler):
    def __init__(s, *a, **k): super().__init__(*a, directory=D, **k)
    def do_POST(s):
        q = urllib.parse.parse_qs(urllib.parse.urlparse(s.path).query)
        n = int(s.headers.get('content-length', 0)); body = s.rfile.read(n)
        os.makedirs(OUT, exist_ok=True)
        name = re.sub(r'[^A-Za-z0-9_-]', '_', q['tag'][0] + '-' + q['name'][0])
        open(os.path.join(OUT, name + '.json'), 'wb').write(body)
        s.send_response(204); s.end_headers()
    def log_message(s, *a): pass
http.server.ThreadingHTTPServer(('127.0.0.1', int(sys.argv[1])), H).serve_forever()
