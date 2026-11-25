#!/usr/bin/env python3
"""QMAPI3: merge the per-page `--missing-api-report` JSON files written by corpus.sh into Markdown tables.

usage: merge.py PAGES.tsv DIR [TOP]     (PAGES.tsv: `name<TAB>url` per line; DIR/<name>.json per page)
"""
import json, sys


def load(path):
    # a report may be preceded by log lines; take the first line that parses as the report object
    try:
        text = open(path, encoding="utf-8", errors="replace").read()
    except OSError:
        return None
    for line in text.splitlines():
        if line.startswith("{"):
            try:
                return json.loads(line)
            except ValueError:
                pass
    return None


def add(table, key, page, hits, extra=""):
    e = table.setdefault(key, {"sites": {}, "hits": 0, "extra": extra})
    e["sites"][page] = e["sites"].get(page, 0) + hits
    e["hits"] += hits
    if not e["extra"]:
        e["extra"] = extra


def cell(s):
    return str(s).replace("|", "\\|").replace("\n", " ")


def main():
    pages_tsv, d = sys.argv[1], sys.argv[2]
    top = int(sys.argv[3]) if len(sys.argv) > 3 else 40
    pages = [l.rstrip("\n").split("\t") for l in open(pages_tsv) if l.strip()]
    urls = dict(pages)
    api, css, errs, status = {}, {}, {}, []
    for name, url in pages:
        j = load("%s/%s.json" % (d, name))
        if j is None:
            status.append((name, url, "no report", 0, 0))
            continue
        m = j.get("missing_api", {})
        for e in m.get("members", []):
            add(api, "%s.%s (%s)" % (e["iface"], e["name"], e["kind"]), name, e["hits"], e.get("script", ""))
        for e in m.get("errors", []):
            add(errs, "%s %s" % (e["kind"], e["name"]), name, e["count"])
        for e in j.get("css", []):
            add(css, "%s %s" % (e["kind"], e["name"]), name, e["count"], e.get("example", ""))
        status.append((name, url, j.get("outcome", "?"), len(m.get("members", [])), len(j.get("css", []))))
    print("| page | URL | outcome | missing members | css entries |\n|---|---|---|---|---|")
    for s in status:
        print("| %s | %s | %s | %s | %s |" % tuple(cell(x) for x in s))
    for title, table, ex in (("JS members (a hit is one lookup that found nothing)", api, "example script"),
                             ("CSS (what the parser dropped or evaluated false)", css, "example"),
                             ("Errors by kind", errs, "")):
        print("\n### %s: %d distinct, top %d by sites then hits\n" % (title, len(table), top))
        print("| API | sites | total hits | example URL | which sites |" + (" %s |" % ex if ex else ""))
        print("|---|---|---|---|---|" + ("---|" if ex else ""))
        r = sorted(table.items(), key=lambda kv: (-len(kv[1]["sites"]), -kv[1]["hits"], kv[0]))[:top]
        for k, e in r:
            first = max(e["sites"], key=lambda s: e["sites"][s])
            sites = ",".join(sorted(e["sites"], key=lambda s: -e["sites"][s]))
            line = "| %s | %d | %d | %s | %s |" % (cell(k), len(e["sites"]), e["hits"], cell(urls[first]), sites)
            if ex:
                line += " %s |" % cell(e["extra"][:120])
            print(line)


main()
