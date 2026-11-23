#!/usr/bin/env python3
"""Unit tests for notab.py's log parsing (no CEF): python3 -m unittest discover -s web/host-cef/test"""

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import notab  # noqa: E402


class Menus(unittest.TestCase):
    def test_forbidden(self):
        r = "100:Back, 101:Forward, 50100:Open link in new tab, Sub[1:Copy, 2:Inspect]"
        self.assertEqual(notab.forbidden_labels(r), ["Open link in new tab", "Inspect"])
        self.assertEqual(notab.forbidden_labels("100:Back, 102:Reload, 113:Copy"), [])

    def test_menu_lines(self):
        lines = ["noise", "[web-host-cef +5ms] context menu: removed [A(1), B(2)]; remaining [100:Back]"]
        self.assertEqual(notab.context_menus(lines), [("A(1), B(2)", "100:Back")])


if __name__ == "__main__":
    unittest.main()
