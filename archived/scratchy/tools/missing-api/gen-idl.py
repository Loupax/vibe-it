#!/usr/bin/env python3
"""QMAPI1: generate src/missing_api_idl.json (interface -> member names) from WPT's interfaces/*.idl.

Usage: tools/missing-api/gen-idl.py [WPT_DIR] > src/missing_api_idl.json
Reads the IDL files of the commit pinned in tools/wpt-pin.txt straight from WPT's git objects
(`git show <pin>:interfaces/X.idl`), so the sparse checkout does not need an `interfaces/` directory
and nothing is vendored. Only the generated JSON is committed.

Output: {"pin": "<hash>",
         "interfaces": {"Name": {"i": "Parent or ''", "m": ["member", ...]}},   (Window-exposed; mixins and
                                                                                partials merged in)
         "namespaces": {"CSS": ["supports", ...]}}
A small purpose-built parser, enough for the Reffy/webref IDL in WPT (no webidl2 / node needed).
"""
import json, os, re, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
WPT = sys.argv[1] if len(sys.argv) > 1 else "/home/loupax/src/wpt"
PIN = open(os.path.join(HERE, "..", "wpt-pin.txt")).read().split()[0]


def git(*a):
    return subprocess.run(["git", "-C", WPT, *a], check=True, capture_output=True, text=True).stdout


def strip_comments(s):
    s = re.sub(r"/\*.*?\*/", "", s, flags=re.S)
    return re.sub(r"//[^\n]*", "", s)


def split_top(s, sep):
    """Split on `sep` outside (), <>, [], {}."""
    out, depth, cur = [], 0, []
    for ch in s:
        if ch in "([<{":
            depth += 1
        elif ch in ")]>}":
            depth -= 1
        if ch == sep and depth == 0:
            out.append("".join(cur))
            cur = []
        else:
            cur.append(ch)
    out.append("".join(cur))
    return [x.strip() for x in out if x.strip()]


def take_extattrs(s):
    """Return (extended attribute text, rest)."""
    s = s.strip()
    ext = ""
    while s.startswith("["):
        depth = 0
        for i, ch in enumerate(s):
            depth += ch == "["
            depth -= ch == "]"
            if depth == 0:
                ext += s[1:i] + ","
                s = s[i + 1:].lstrip()
                break
    return ext, s


def window_exposed(ext):
    m = re.search(r"\bExposed\s*=\s*(\([^)]*\)|[A-Za-z*]+)", ext)
    if not m:
        return True
    v = m.group(1)
    return "*" in v or "Window" in v


SPECIAL = re.compile(r"^(?:(?:getter|setter|deleter|legacycaller)\s+)+")


def members(body):
    out = []
    for st in split_top(body, ";"):
        _, st = take_extattrs(st)
        if not st:
            continue
        first = st.split()[0]
        if st.startswith("constructor(") or first == "constructor" or first == "static":
            continue  # constructor / statics live on the constructor, not the prototype chain
        if first == "const":
            out.append(re.sub(r"\s*=.*", "", st).split()[-1])
            continue
        if re.match(r"(async\s+)?iterable\s*<", st):
            out += ["entries", "keys", "values", "forEach"]
            continue
        m = re.match(r"(readonly\s+)?(maplike|setlike)\s*<", st)
        if m:
            out += ["entries", "keys", "values", "forEach", "has", "size"]
            if not m.group(1):
                out += ["clear", "delete", "set" if m.group(2) == "maplike" else "add"]
            if m.group(2) == "maplike":
                out.append("get")
            continue
        if first == "stringifier":
            st = st[len("stringifier"):].strip()
            if not st:
                out.append("toString")
                continue
        if re.search(r"\battribute\b", st):
            out.append(re.findall(r"[A-Za-z_][A-Za-z0-9_]*", st)[-1])
            continue
        op = SPECIAL.sub("", st)
        m = re.search(r"([A-Za-z_][A-Za-z0-9_]*)\s*\(", op)
        if m and (op == st or re.match(r".*\b\w+\s*\(", op)):
            # a nameless special (`getter T (DOMString x)`) has "(" right after the return type: skip those
            head = op[:m.end()].strip()
            if SPECIAL.match(st) and len(head.split()) < 2:
                continue
            out.append(m.group(1))
    return out


files = [f for f in git("ls-tree", "--name-only", PIN, "interfaces/").split() if f.endswith(".idl")]
ifaces, parent, mixins, includes, nss = {}, {}, {}, [], {}
defn = re.compile(
    r"(?P<kw>partial\s+interface\s+mixin|interface\s+mixin|partial\s+interface|callback\s+interface|interface|partial\s+namespace|namespace)"
    r"\s+(?P<name>\w+)\s*(?::\s*(?P<par>\w+))?\s*\{",
)
for f in files:
    src = strip_comments(git("show", f"{PIN}:{f}"))
    for m in re.finditer(r"(\w+)\s+includes\s+(\w+)\s*;", src):
        includes.append((m.group(1), m.group(2)))
    pos = 0
    while True:
        m = defn.search(src, pos)
        if not m:
            break
        depth, i = 1, m.end()
        while depth:
            depth += {"{": 1, "}": -1}.get(src[i], 0)
            i += 1
        body = src[m.end():i - 1]
        pos = i
        kw, name = m.group("kw").split(), m.group("name")
        # the extended attributes sit between the previous definition / statement and the keyword
        cut = max(src.rfind("}", 0, m.start()), src.rfind(";", 0, m.start()))
        ext = src[cut + 1:m.start()]
        if kw[0] == "callback":
            continue
        is_mixin = "mixin" in kw
        if not window_exposed(ext) and not is_mixin:
            continue
        mem = members(body)
        if "namespace" in kw:
            nss.setdefault(name, set()).update(mem)
        elif is_mixin:
            mixins.setdefault(name, set()).update(mem)
        else:
            ifaces.setdefault(name, set()).update(mem)
            if m.group("par"):
                parent[name] = m.group("par")
for target, mix in includes:
    if target in ifaces and mix in mixins:
        ifaces[target] |= mixins[mix]
out = {
    "pin": PIN,
    "interfaces": {k: {"i": parent.get(k, ""), "m": sorted(v)} for k, v in sorted(ifaces.items())},
    "namespaces": {k: sorted(v) for k, v in sorted(nss.items())},
}
json.dump(out, sys.stdout, separators=(",", ":"))
sys.stdout.write("\n")
