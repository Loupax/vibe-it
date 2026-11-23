#!/usr/bin/env python3
"""Unit tests for notab.py's log parsing (no CEF): python3 -m unittest discover -s web/host-cef/test"""

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import notab  # noqa: E402


class Menus(unittest.TestCase):
    def test_suppressed_lines(self):
        lines = [
            "noise",
            "[web-host-cef +5ms] context menu: suppressed",
            "[web-host-cef +6ms] context menu: run suppressed",
            "context menu: removed [A(1)]; remaining [100:Back]",
        ]
        self.assertEqual(notab.suppressed_lines(lines), lines[1:3])

    def test_menu_shown_lines(self):
        lines = ["noise", "[web-host-cef +5ms] context menu: removed [A(1), B(2)]; remaining [100:Back]", "context menu: suppressed"]
        self.assertEqual(notab.menu_shown_lines(lines), [lines[1]])


if __name__ == "__main__":
    unittest.main()
