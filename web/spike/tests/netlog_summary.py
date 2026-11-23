#!/usr/bin/env python3
"""W6: summarise a Chromium net log (--log-net-log) as "what the network stack itself tried".

Requests answered by the spike's handler never reach Chromium's network stack, so every
URL_REQUEST / socket / DNS / QUIC / WebSocket entry in the net log is a path that BYPASSED
the handler. Prints one line per distinct attempt with its outcome.

Usage: netlog_summary.py NETLOG.json [--all]
A log cut off by a killed process (no closing brackets) is repaired before parsing.
"""
import json
import sys


def load(path):
    text = open(path, encoding="utf-8", errors="replace").read()
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        pass
    # Cut off mid-stream: drop the last partial event and close the array/object.
    cut = text.rstrip().rstrip(",")
    for _ in range(64):
        i = cut.rfind("},")
        if i < 0:
            break
        try:
            return json.loads(cut[: i + 1] + "]}")
        except json.JSONDecodeError:
            cut = cut[:i]
    raise SystemExit(f"{path}: cannot repair the net log")


def main():
    path = sys.argv[1]
    show_all = "--all" in sys.argv
    log = load(path)
    c = log["constants"]
    etypes = {v: k for k, v in c["logEventTypes"].items()}
    stypes = {v: k for k, v in c["logSourceType"].items()}
    errors = {v: k for k, v in c.get("netError", {}).items()}
    sources = {}
    for e in log["events"]:
        src = e["source"]
        sid = src["id"]
        s = sources.setdefault(sid, {"type": stypes.get(src["type"], src["type"]), "what": set(), "errors": set(), "events": set()})
        name = etypes.get(e["type"], e["type"])
        s["events"].add(name)
        p = e.get("params") or {}
        for key in ("url", "host", "address", "remote_address", "group_id", "group_name", "server_id", "proxy_info", "proxy_server", "proxy_chain", "hostname", "origin", "dns_query_type"):
            v = p.get(key)
            if v and isinstance(v, (str, int)):
                s["what"].add(f"{key}={v}")
            elif isinstance(v, dict):
                s["what"].add(f"{key}={json.dumps(v, sort_keys=True)}")
        if "host" in p and isinstance(p["host"], dict):
            s["what"].add(f"host={p['host']}")
        ne = p.get("net_error")
        if isinstance(ne, int) and ne != 0:
            s["errors"].add(f"{ne} {errors.get(ne, '')}".strip())
    interesting = (
        "URL_REQUEST", "SOCKET", "TRANSPORT_CONNECT_JOB", "SSL_CONNECT_JOB", "HTTP_PROXY_CONNECT_JOB",
        "SOCKS_CONNECT_JOB", "HOST_RESOLVER_IMPL_JOB", "HOST_RESOLVER_MANAGER_JOB", "HOST_RESOLVER_IMPL_REQUEST",
        "HOST_RESOLVER_MANAGER_REQUEST", "DNS_TRANSACTION", "DNS_OVER_HTTPS", "QUIC_SESSION", "QUIC_CONNECTION_MIGRATION",
        "UDP_SOCKET", "WEB_SOCKET", "HTTP_STREAM_JOB", "HTTP_STREAM_JOB_CONTROLLER", "HTTP2_SESSION",
        "CONNECT_JOB", "PROXY_RESOLUTION", "PAC_FILE_DECIDER", "WEB_TRANSPORT_CLIENT",
        "NETWORK_QUALITY_ESTIMATOR", "HTTP_STREAM_POOL_ATTEMPT_MANAGER", "HTTP_STREAM_POOL_GROUP",
        "TCP_STREAM_ATTEMPT", "TLS_STREAM_ATTEMPT",
    )
    lines = {}
    for sid, s in sorted(sources.items()):
        if not show_all and not any(s["type"].startswith(t) for t in interesting):
            continue
        what = ", ".join(sorted(s["what"])) or "-"
        err = "; ".join(sorted(s["errors"])) or "ok/none"
        key = f"{s['type']:<34} {what}  -> {err}"
        lines[key] = lines.get(key, 0) + 1
    for k, n in sorted(lines.items()):
        print(f"{n:3}x {k}")
    print(f"# {len(sources)} sources, {len(log['events'])} events", file=sys.stderr)


if __name__ == "__main__":
    main()
