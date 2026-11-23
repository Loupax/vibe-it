#!/usr/bin/env python3
"""A tiny HTTP forward proxy for the C1 check: plain requests (absolute URI) and
CONNECT tunnels, one thread per client, one log line per request or tunnel.
Test tool only: no auth, no caching, no keep-alive on the plain path.

Usage: fwd_proxy.py [--host 127.0.0.1] [--port 0] --log FILE [--pid-file FILE]
Prints "listening HOST:PORT" on stdout once ready."""

import argparse
import select
import socket
import sys
import threading
import time
from urllib.parse import urlsplit

LOG_LOCK = threading.Lock()


def split_target(method, target):
    """(host, port, origin-form path) for a proxy request line's target.
    CONNECT takes host:port; everything else an absolute http:// URI."""
    if method == "CONNECT":
        host, _, port = target.rpartition(":")
        if not host or not port.isdigit():
            raise ValueError(f"bad CONNECT target {target!r}")
        return host.strip("[]"), int(port), None
    u = urlsplit(target)
    if u.scheme != "http" or not u.hostname:
        raise ValueError(f"not an absolute http URI: {target!r}")
    path = u.path or "/"
    if u.query:
        path += "?" + u.query
    return u.hostname, u.port or 80, path


def read_head(sock):
    data = b""
    while b"\r\n\r\n" not in data:
        chunk = sock.recv(65536)
        if not chunk:
            return None, b""
        data += chunk
        if len(data) > 1 << 20:
            return None, b""
    head, _, rest = data.partition(b"\r\n\r\n")
    return head.decode("latin-1"), rest


def pipe(a, b):
    """Copies both ways until either side closes. Returns bytes (a->b, b->a)."""
    up = down = 0
    socks = [a, b]
    while True:
        r, _, _ = select.select(socks, [], [], 120)
        if not r:
            break
        for s in r:
            data = s.recv(65536)
            if not data:
                return up, down
            (b if s is a else a).sendall(data)
            if s is a:
                up += len(data)
            else:
                down += len(data)
    return up, down


class Proxy:
    def __init__(self, log_path):
        self.log = open(log_path, "a", buffering=1)

    def logline(self, text):
        with LOG_LOCK:
            self.log.write(f"{time.strftime('%H:%M:%S')} {text}\n")

    def handle(self, client):
        try:
            head, rest = read_head(client)
            if head is None:
                return
            lines = head.split("\r\n")
            method, target, version = lines[0].split(" ", 2)
            try:
                host, port, path = split_target(method, target)
            except ValueError as e:
                self.logline(f"REFUSED {lines[0]} ({e})")
                client.sendall(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")
                return
            try:
                up = socket.create_connection((host, port), timeout=20)
                up.settimeout(None)
            except OSError as e:
                self.logline(f"{method} {target} -> 502 ({e})")
                client.sendall(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                return
            if method == "CONNECT":
                client.sendall(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                if rest:
                    up.sendall(rest)
                n_up, n_down = pipe(client, up)
                self.logline(f"CONNECT {target} tunnel closed ({n_up} B up, {n_down} B down)")
            else:
                headers = [h for h in lines[1:] if not h.lower().startswith(("proxy-connection:", "connection:", "keep-alive:"))]
                req = f"{method} {path} {version}\r\n" + "".join(h + "\r\n" for h in headers) + "Connection: close\r\n\r\n"
                up.sendall(req.encode("latin-1") + rest)
                first = up.recv(65536)
                status = first.split(b"\r\n", 1)[0].decode("latin-1", "replace")
                self.logline(f"{method} {target} -> {status}")
                client.sendall(first)
                pipe(client, up)
            up.close()
        except Exception as e:  # a test tool: log and drop the connection
            self.logline(f"error: {e!r}")
        finally:
            client.close()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=0)
    ap.add_argument("--log", required=True)
    ap.add_argument("--pid-file")
    a = ap.parse_args()
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind((a.host, a.port))
    srv.listen(64)
    if a.pid_file:
        import os
        with open(a.pid_file, "w") as f:
            f.write(f"{os.getpid()}\n")
    proxy = Proxy(a.log)
    host, port = srv.getsockname()[:2]
    proxy.logline(f"listening {host}:{port}")
    print(f"listening {host}:{port}", flush=True)
    while True:
        c, _ = srv.accept()
        threading.Thread(target=proxy.handle, args=(c,), daemon=True).start()


if __name__ == "__main__":
    sys.exit(main())
