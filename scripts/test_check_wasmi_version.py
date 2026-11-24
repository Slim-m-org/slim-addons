#!/usr/bin/env python3
"""Unit tests for check-wasmi-version.py: run-module's wasmi must match the version in a given slim-m lockfile.

Run from the repo root: python3 -m unittest discover -s scripts -p 'test_*.py'
SCRIPTS_DIR points the tests at another copy of the scripts, for mutation checks.
"""
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest

scripts = pathlib.Path(os.environ.get("SCRIPTS_DIR", pathlib.Path(__file__).resolve().parent))
repo = pathlib.Path(__file__).resolve().parent.parent


def lock(*versions):
    return "version = 4\n" + "".join(f'\n[[package]]\nname = "wasmi"\nversion = "{v}"\n' for v in versions)


class WasmiVersionTest(unittest.TestCase):
    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp)
        (self.tmp / "scripts").mkdir()
        (self.tmp / "tools/run-module").mkdir(parents=True)
        shutil.copy(scripts / "check-wasmi-version.py", self.tmp / "scripts")
        self.ours = (repo / "tools/run-module/Cargo.lock").read_text()
        (self.tmp / "tools/run-module/Cargo.lock").write_text(self.ours)

    def run_check(self, slim_lock):
        path = self.tmp / "slim.lock"
        path.write_text(slim_lock)
        return subprocess.run([sys.executable, str(self.tmp / "scripts/check-wasmi-version.py"), "--slim-lock", str(path)], capture_output=True, text=True)

    def our_version(self):
        text = self.ours.split('name = "wasmi"\nversion = "')[1]
        return text.split('"')[0]

    def test_the_same_version_passes(self):
        done = self.run_check(lock(self.our_version()))
        self.assertEqual(done.returncode, 0, done.stderr)

    def test_a_different_server_version_is_refused(self):
        done = self.run_check(lock("9.9.9"))
        self.assertEqual(done.returncode, 1)
        self.assertIn("slim-m's server runs 9.9.9", done.stderr)

    def test_a_server_lock_with_no_or_two_wasmi_is_refused(self):
        self.assertEqual(self.run_check(lock()).returncode, 1)
        self.assertEqual(self.run_check(lock(self.our_version(), "1.0.0")).returncode, 1)


if __name__ == "__main__":
    unittest.main()
