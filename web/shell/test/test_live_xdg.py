#!/usr/bin/env python3
"""C-LIVE-XDG guard: a live script must not start the shell with the user's XDG homes (their init.js would
load). Every script in this directory that spawns the shell must import `live` (or `livelib`, which does),
whose import gives the process empty temporary XDG_CONFIG_HOME/XDG_STATE_HOME/XDG_DATA_HOME; the desktop
check shell script sets its own. Run: python3 -m unittest web/shell/test/test_live_xdg.py"""
import glob
import os
import re
import subprocess
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
VARS = ("XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_DATA_HOME")


class LiveXdg(unittest.TestCase):
    def test_every_spawning_script_imports_live(self):
        seen = 0
        for p in sorted(glob.glob(os.path.join(HERE, "*.py"))):
            name = os.path.basename(p)
            if name in ("live.py", "test_live_xdg.py"):
                continue
            src = open(p).read()
            if "subprocess.Popen" not in src:
                continue
            seen += 1
            self.assertRegex(src, r"(?m)^\s*import (live|livelib)\b",
                             f"{name} starts processes without importing live (temporary XDG homes)")
        self.assertGreater(seen, 15)

    def test_no_script_unsets_isolation(self):
        for p in sorted(glob.glob(os.path.join(HERE, "*.py"))):
            if os.path.basename(p) == "test_live_xdg.py":
                continue
            src = open(p).read()
            for v in VARS:
                self.assertFalse(re.search(rf"environ\.pop\(\s*[\"']{v}", src), f"{p} removes {v}")

    def test_livelib_imports_live(self):
        self.assertRegex(open(os.path.join(HERE, "livelib.py")).read(), r"(?m)^import live\b")

    def test_import_live_sets_temp_homes(self):
        env = {k: v for k, v in os.environ.items() if k not in VARS and k != "LIVE_XDG_ISOLATED"}
        for v in VARS:
            env[v] = "/home/someone/real"
        code = "import sys; sys.path.insert(0, %r); import live, os; print(*[os.environ[v] for v in %r])" % (HERE, VARS)
        out = subprocess.run([sys.executable, "-I", "-c", code], env=env, capture_output=True, text=True, check=True)
        paths = out.stdout.split()
        self.assertEqual(len(paths), 3)
        for p in paths:
            self.assertNotIn("/home/someone", p)
            self.assertIn("wxdg-", p)

    def test_desktop_check_sets_xdg(self):
        src = open(os.path.join(HERE, "c6d_desktop_check.sh")).read()
        self.assertIn("export XDG_CONFIG_HOME=", src)
        self.assertIn("XDG_STATE_HOME=", src)
        self.assertIn("XDG_DATA_HOME=", src)
        self.assertIn("mktemp -d", src)


if __name__ == "__main__":
    unittest.main()
