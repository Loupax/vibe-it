#!/usr/bin/env python3
"""Unit tests for the P-COOKIES check's verdict logic (no CEF needed):
python3 -m unittest discover -s web/host-cef/test"""

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import cookies  # noqa: E402

OWN = {"sid", "jsid", "chips"}
EMPTY = {"before": set(), "after": set()}


class Judge(unittest.TestCase):
    def test_names(self):
        self.assertEqual(cookies.cookie_names("a=1; b=2;c=3"), {"a", "b", "c"})
        self.assertEqual(cookies.cookie_names(""), set())
        self.assertEqual(cookies.cookie_names(None), set())

    def test_blocked_is_clean(self):
        frame = {"before": {"tpp_hdr"}, "after": {"tpp_hdr", "tpp_js"}}
        self.assertEqual(cookies.judge("block", OWN, frame, set(), set(), OWN, set(), None), [])

    def test_block_catches_a_leak_each_way(self):
        frame = {"before": {"sid"}, "after": {"sid", "tp_js"}}
        bad = cookies.judge("block", OWN, frame, {"jsid"}, set(), OWN | {"tp_hdr"}, set(), None)
        text = " ".join(bad)
        self.assertIn("could read", text)
        self.assertIn("set plain cookies", text)
        self.assertIn("own jar got", text)

    def test_block_needs_own_cookies_at_top_level(self):
        bad = cookies.judge("block", {"sid"}, EMPTY, set(), set(), {"sid"}, set(), None)
        self.assertTrue(bad)

    def test_allow_control_must_show_third_party_cookies(self):
        frame = {"before": set(), "after": {"tp_hdr", "tp_js"}}
        ok = cookies.judge("allow", OWN, frame, {"sid", "jsid"}, set(), OWN | {"tp_hdr", "tp_img", "tp_js"}, set(), None)
        self.assertEqual(ok, [])
        # A "blocked" result in the allow run means the test could not tell anything.
        self.assertTrue(cookies.judge("allow", OWN, EMPTY, set(), set(), OWN, set(), None))

    def test_partitions(self):
        self.assertEqual(cookies.judge_partitions("block", EMPTY, set(), None, "https://site-a.test", None), [])
        self.assertTrue(cookies.judge_partitions("block", EMPTY, {"tpp_js"}, None, "https://site-a.test", None))
        self.assertTrue(cookies.judge_partitions("allow", EMPTY, set(), "https://site-a.test", "https://site-a.test", None))
        self.assertTrue(cookies.judge_partitions("allow", EMPTY, set(), None, None, None))
        self.assertTrue(cookies.judge_partitions("allow", EMPTY, set(), None, "https://site-a.test", "x"))


if __name__ == "__main__":
    unittest.main()
