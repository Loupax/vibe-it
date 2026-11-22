#!/usr/bin/env python3
"""Generate wpt-th/subset.txt (QWPT6, R64 2.1/2.5) from tools/wpt-dirs.txt and the WPT manifest of the pin.

Usage (from scratchy/):   python3 tools/wpt-subset.py CHECKOUT [--out wpt-th/subset.txt]

CHECKOUT is a checkout made by tools/wpt-checkout.sh. WPT's own manifest code lives in CHECKOUT/tools, which
the sparse checkout does not contain: run `git -C CHECKOUT sparse-checkout add tools` once (about 12 MB).
The manifest is built by WPT's tools/manifest in-process from the files on disk (no `./wpt` launcher, no
virtualenv, no pip, no sockets) and is written to CHECKOUT/MANIFEST.json (a cache; never committed).

What goes in (phase 1, "testharness files a static server can serve"): every testharness URL of the AREAS
below, except
  - timeout=long files outside dom/* (nightly; QWPT6c keeps the dom/* ones: with the virtual clock they are cheap), and the legacy multi-byte encoding variants (encoding/legacy-mb-*, nightly),
  - files that call a testdriver function scratchy's vendor does not implement (see DRIVER_DONE), html/editing testdriver users,
  - `.https.` / `.sub.` / worker-variant URLs, and files whose source mentions a `.py` handler (wptserve). The
    WPTSERVE_AREAS (fetch/*, xhr; QWPT8) keep the `.py` pages: they run with `--server wptserve`; `.h2.` pages stay out.
The skipped counts per reason are printed on stderr, so the numbers in the PR can be reproduced.
"""
import argparse
import os
import re
import sys
from collections import Counter, OrderedDict

# area name -> directory prefixes (longest prefix wins; a prefix matches a whole path segment run).
AREAS = OrderedDict([
    ("dom/nodes", ["dom/nodes"]),
    ("dom/events", ["dom/events"]),
    ("dom/ranges", ["dom/ranges"]),
    ("dom/observable", ["dom/observable"]),
    ("dom/other", ["dom"]),
    ("domparsing", ["domparsing"]),
    ("webidl", ["webidl"]),
    ("uievents", ["uievents"]),
    ("html/webappapis", ["html/webappapis"]),
    ("resize-observer", ["resize-observer"]),
    ("intersection-observer", ["intersection-observer"]),
    ("html/semantics/scripting-1", ["html/semantics/scripting-1"]),
    ("html/dom", ["html/dom"]),
    ("html/editing", ["html/editing"]),
    ("html/canvas/element", ["html/canvas/element"]),
    ("html/canvas/offscreen", ["html/canvas/offscreen"]),
    ("html/canvas/other", ["html/canvas"]),
    ("css/cssom", ["css/cssom"]),
    ("css/cssom-view", ["css/cssom-view"]),
    ("encoding", ["encoding"]),
    ("url", ["url"]),
    ("streams", ["streams"]),
    ("hr-time", ["hr-time"]),
    ("webmessaging", ["webmessaging"]),
    ("requestidlecallback", ["requestidlecallback"]),
    ("custom-elements", ["custom-elements"]),
    ("shadow-dom", ["shadow-dom"]),
    # QWPT8: areas that need WPT's Python server (wptserve): `.py` handlers stay in. Run them with `--server wptserve`.
    ("fetch/api", ["fetch/api"]),
    ("fetch/other", ["fetch"]),
    ("xhr", ["xhr"]),
])

# Areas served by wptserve (QWPT8), not by wire's static mount: the `.py` filter does not apply to them.
WPTSERVE_AREAS = {"fetch/api", "fetch/other", "xhr"}

# testdriver users (QWPT6d, QWPT6e, QWPT7). Since QWPT7 the runner serves scratchy's own testdriver-vendor.js
# (tools/wpt/testdriver-vendor.js: trusted input through src/wpt_input.rs), which implements the calls in DRIVER_DONE.
# A page whose source calls any OTHER `test_driver.<name>` (window rects, cookies, bidi, permissions, ...) is left out
# (counted per name on stderr). QWPT6e dropped every caller (111 URLs); QWPT7 keeps all but those.
# html/editing (QWPT6d, 54 dnd files) keeps its older, broader rule: any `testdriver` mention drops the page (drag and drop).
SRC_TESTDRIVER_AREAS = {"html/editing"}
DRIVER_CALL = re.compile(rb"\btest_driver\.([A-Za-z_]+)")
DRIVER_DONE = {"click", "send_keys", "action_sequence", "Actions", "bless", "get_computed_label", "get_computed_role",
               "set_test_context", "message_test"}

PY_REF = re.compile(rb"""\.py(?:["'?#\s)]|$)""")


def area_of(path):
    best, best_len = None, -1
    for area, prefixes in AREAS.items():
        for p in prefixes:
            if (path == p or path.startswith(p + "/")) and len(p) > best_len:
                best, best_len = area, len(p)
    return best


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("checkout")
    ap.add_argument("--out", default="wpt-th/subset.txt")
    a = ap.parse_args()
    root = os.path.abspath(a.checkout)
    if not os.path.isdir(os.path.join(root, "tools", "manifest")):
        sys.exit("wpt-subset: %s/tools/manifest missing: run `git -C %s sparse-checkout add tools`" % (root, root))
    sys.path.insert(0, os.path.join(root, "tools"))
    sys.path.insert(0, os.path.join(root, "tools", "third_party", "html5lib"))
    sys.path.insert(0, os.path.join(root, "tools", "third_party", "six"))
    import localpaths  # noqa: F401  (WPT's own sys.path setup)
    from manifest import manifest as wptmanifest

    m = wptmanifest.load_and_update(root, os.path.join(root, "MANIFEST.json"), "/", parallel=False)
    skipped = Counter()
    per_area = OrderedDict((k, set()) for k in AREAS)
    for _typ, path, items in m.itertypes("testharness"):
        area = area_of(path)
        if area is None:
            continue
        base = path.rsplit("/", 1)[-1]
        if path.startswith("encoding/legacy-mb-"):
            skipped["legacy-mb encoding (nightly)"] += len(items)
            continue
        src = None
        for item in items:
            url = item.url.lstrip("/")
            if item.timeout == "long" and not area.startswith("dom/"):
                skipped["timeout=long (nightly)"] += 1
                continue
            if any(k == "script" and "testdriver" in v for k, v in (item.script_metadata or [])):
                skipped["testdriver (phase 2)"] += 1
                continue
            if area in WPTSERVE_AREAS and re.search(r"\.h2\.", base):
                skipped[".h2. (HTTP/2 server, QWPT9)"] += 1
                continue
            if ".https." in base or ".sub." in base:
                skipped[".https./.sub. (server features)"] += 1
                continue
            if re.search(r"\.(?:service|shared)?worker\.", url.split("?")[0].rsplit("/", 1)[-1]):
                skipped["worker variant (NOT-RUN)"] += 1
                continue
            if src is None:
                try:
                    with open(os.path.join(root, path), "rb") as f:
                        src = f.read()
                except OSError:
                    src = b""
            if area in SRC_TESTDRIVER_AREAS and b"testdriver" in src:
                skipped["testdriver script tag (phase 2, %s)" % area] += 1
                continue
            missing = sorted({m.decode() for m in DRIVER_CALL.findall(src)} - DRIVER_DONE)
            if missing:
                skipped["test_driver.%s (not in the vendor)" % "/".join(missing)] += 1
                continue
            if area not in WPTSERVE_AREAS and PY_REF.search(src):
                skipped[".py handler (wptserve)"] += 1
                continue
            per_area[area].add(url)
    total = 0
    with open(a.out, "w") as out:
        out.write("# Generated by tools/wpt-subset.py from tools/wpt-dirs.txt + the WPT manifest of the pin (QWPT6); do not edit.\n")
        out.write("# Paths are page URLs relative to the WPT checkout. `# area: NAME` starts an area; expected results go to\n")
        out.write("# wpt-th/expected/<NAME with / -> ->.txt. Not here: timeout=long, legacy-mb encodings, unimplemented testdriver calls, .https./.sub.,\n")
        out.write("# worker variants, files using .py handlers (see the script header).\n")
        for area, urls in per_area.items():
            if not urls:
                continue
            out.write("# area: %s\n" % area)
            for u in sorted(urls):
                out.write(u + "\n")
            total += len(urls)
            print("%-30s %5d" % (area, len(urls)), file=sys.stderr)
    print("total %d URLs" % total, file=sys.stderr)
    for k, v in sorted(skipped.items()):
        print("skipped %-40s %5d" % (k, v), file=sys.stderr)


if __name__ == "__main__":
    main()
