#!/usr/bin/env python3
"""Unit tests for the C1 check's helpers (no CEF needed):
python3 -m unittest discover -s web/host-cef/test"""

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import drive  # noqa: E402
import forget  # noqa: E402
import fwd_proxy  # noqa: E402


class EventKeys(unittest.TestCase):
    def test_keys(self):
        self.assertEqual(drive.event_key({"event": "load", "phase": "end", "kind": "same-document"}), "load end same-document")
        self.assertEqual(drive.event_key({"event": "editable-focused", "focused": False}), "editable-focused false")
        self.assertEqual(drive.event_key({"event": "url", "tab": 1, "url": "x"}), "url")

    def test_missing(self):
        evs = [{"event": e} for e in ("hello", "tab-created", "tab-closed", "url", "title", "load-error", "find-result", "crashed", "error",
                                      "popup-blocked")]
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


class BackgroundTraffic(unittest.TestCase):
    LOG = (
        "10:00:00 listening 127.0.0.1:3128\n"
        "10:00:01 GET http://127.0.0.1:8000/a.html -> HTTP/1.0 200 OK\n"
        "10:00:01 CONNECT example.com:443 opened\n"
        "10:00:02 CONNECT example.com:443 tunnel closed (10 B up, 20 B down)\n"
        "10:00:03 GET http://nx-c1-check.invalid/ -> 502 (gaierror)\n"
        "10:00:04 CONNECT update.googleapis.com:443 opened\n"
        "10:00:05 GET http://clients2.google.com/time/1/current?cup2key=x -> HTTP/1.1 200 OK\n"
        "10:00:06 CONNECT [::1]:8443 opened\n"
        "10:00:07 REFUSED GET /x (not an absolute http URI: '/x')\n"
        "10:00:08 error: OSError()\n"
    )

    def test_hosts_in_order_without_duplicates(self):
        self.assertEqual(
            drive.proxy_log_hosts(self.LOG),
            ["127.0.0.1", "example.com", "nx-c1-check.invalid", "update.googleapis.com", "clients2.google.com", "::1"])

    def test_foreign_hosts(self):
        allowed = ["127.0.0.1", "example.com", "nx-c1-check.invalid"]
        self.assertEqual(drive.foreign_hosts(self.LOG, allowed), ["update.googleapis.com", "clients2.google.com", "::1"])
        self.assertEqual(drive.foreign_hosts(self.LOG, allowed + ["googleapis.com", "google.com", "::1"]), [])

    def test_subdomain_matches_but_suffix_does_not(self):
        log = "10:00:00 CONNECT www.example.com:443 opened\n10:00:00 CONNECT notexample.com:443 opened\n"
        self.assertEqual(drive.foreign_hosts(log, ["example.com"]), ["notexample.com"])

    def test_empty_log(self):
        self.assertEqual(drive.foreign_hosts("", ["example.com"]), [])


class ForgetChecks(unittest.TestCase):
    FULL = {"server_cookie": "sid=a-secret; hsid=1; js=1", "cookie": "sid=a-secret; js=1", "ls": "v", "ss": "v",
            "idb": ["db"], "caches": ["c"], "sw": 1, "http_cached": True}
    EMPTY = {"server_cookie": "", "cookie": "", "ls": None, "ss": None, "idb": [], "caches": [], "sw": 0,
             "http_cached": False}

    def test_full(self):
        self.assertEqual(forget.stored_everything(self.FULL, "a"), [])
        self.assertEqual(len(forget.stored_nothing(self.FULL, "a")), 8)

    def test_empty(self):
        self.assertEqual(forget.stored_nothing(self.EMPTY, "a"), [])
        self.assertEqual(len(forget.stored_everything(self.EMPTY, "a")), 9)

    def test_http_cache_optional(self):
        r = dict(self.FULL, http_cached=False)
        self.assertEqual(forget.stored_everything(r, "a"), ["a: HTTP cache entry missing"])
        self.assertEqual(forget.stored_everything(r, "a", http=False), [])
        self.assertEqual(forget.stored_nothing(dict(self.EMPTY, http_cached=True), "a"), ["a: HTTP cache entry still there"])

    def test_session_optional(self):
        r = dict(self.EMPTY, ss="v")
        self.assertEqual(forget.stored_nothing(r, "a"), ["a: sessionStorage still there"])
        self.assertEqual(forget.stored_nothing(r, "a", session=False), [])
        r = dict(self.FULL, ss=None)
        self.assertEqual(forget.stored_everything(r, "a", session=False), [])

    def test_switches(self):
        s = forget.chromium_switches(4242)
        self.assertIn("--host-resolver-rules=MAP *.test 127.0.0.1:4242", s)
        self.assertIn("http://sub.site-a.test", s[1])


if __name__ == "__main__":
    unittest.main()
