#!/usr/bin/env python3
"""Unit tests for the C1 check's helpers (no CEF needed):
python3 -m unittest discover -s web/host-cef/test"""

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import drive  # noqa: E402
import fwd_proxy  # noqa: E402


class EventKeys(unittest.TestCase):
    def test_keys(self):
        self.assertEqual(drive.event_key({"event": "load", "phase": "end", "kind": "same-document"}), "load end same-document")
        self.assertEqual(drive.event_key({"event": "editable-focused", "focused": False}), "editable-focused false")
        self.assertEqual(drive.event_key({"event": "url", "tab": 1, "url": "x"}), "url")

    def test_missing(self):
        evs = [{"event": e} for e in ("hello", "tab-created", "tab-closed", "url", "title", "load-error", "find-result", "crashed", "error")]
        evs += [{"event": "editable-focused", "focused": f} for f in (True, False)]
        evs += [{"event": "load", "phase": p, "kind": k} for p in ("start", "end") for k in ("new", "reload", "traverse")]
        self.assertEqual(drive.missing_events(evs), ["load start same-document", "load end same-document"])
        evs += [{"event": "load", "phase": p, "kind": "same-document"} for p in ("start", "end")]
        self.assertEqual(drive.missing_events(evs), [])


class Announce(unittest.TestCase):
    def test_order(self):
        log = ["[web-host-cef +1ms] event: tab-created 1", "[web-host-cef +2ms] window of tab 1 shown"]
        self.assertTrue(drive.announced_before_shown(log, 1))
        self.assertFalse(drive.announced_before_shown(list(reversed(log)), 1))
        self.assertFalse(drive.announced_before_shown(log[:1], 1))
        self.assertFalse(drive.announced_before_shown(log, 2))


class Matching(unittest.TestCase):
    def test_is_ev(self):
        p = drive.is_ev("load", phase="end", status=200)
        self.assertTrue(p({"event": "load", "phase": "end", "status": 200, "kind": "new"}))
        self.assertFalse(p({"event": "load", "phase": "start", "status": 0}))
        q = drive.is_ev("error", message=lambda m: "zoom" in m)
        self.assertTrue(q({"event": "error", "message": "zoom factor 9"}))


class ProxyTargets(unittest.TestCase):
    def test_split(self):
        self.assertEqual(fwd_proxy.split_target("CONNECT", "example.com:443"), ("example.com", 443, None))
        self.assertEqual(fwd_proxy.split_target("CONNECT", "[::1]:8443"), ("::1", 8443, None))
        self.assertEqual(fwd_proxy.split_target("GET", "http://127.0.0.1:8000/a.html?do=x"), ("127.0.0.1", 8000, "/a.html?do=x"))
        self.assertEqual(fwd_proxy.split_target("GET", "http://a.test"), ("a.test", 80, "/"))
        for m, t in [("CONNECT", "example.com"), ("GET", "/a.html"), ("GET", "https://a.test/")]:
            with self.assertRaises(ValueError):
                fwd_proxy.split_target(m, t)


if __name__ == "__main__":
    unittest.main()
