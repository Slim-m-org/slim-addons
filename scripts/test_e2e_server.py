#!/usr/bin/env python3
"""Unit tests for e2e_server.py: a relative --server path must still launch, whatever the server's own cwd.

Run from the repo root: python3 -m unittest discover -s scripts -p 'test_*.py'
SCRIPTS_DIR points the tests at another copy of the scripts, for mutation checks.
"""
import importlib.util
import os
import pathlib
import shutil
import stat
import sys
import tempfile
import unittest

scripts = pathlib.Path(os.environ.get("SCRIPTS_DIR", pathlib.Path(__file__).resolve().parent))
sys.path.insert(0, str(scripts))


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, scripts / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ServerTest(unittest.TestCase):
    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp)
        self.cwd = os.getcwd()
        self.addCleanup(os.chdir, self.cwd)

    def test_a_relative_binary_is_resolved_against_the_callers_directory(self):
        binary = self.tmp / "bin" / "fake-server"
        binary.parent.mkdir()
        binary.write_text("#!/bin/sh\nexit 0\n")
        binary.chmod(binary.stat().st_mode | stat.S_IEXEC)
        workdir = self.tmp / "work"
        workdir.mkdir()
        os.chdir(self.tmp)
        server = load("e2e_server", "e2e_server.py").Server("bin/fake-server", workdir)
        self.assertEqual(server.binary, str(binary))
        server.proc = __import__("subprocess").Popen([server.binary], cwd=workdir)
        self.assertEqual(server.proc.wait(timeout=10), 0)

    def test_all_four_scripts_share_the_one_server_class(self):
        xp = load("e2e_xp", "e2e-xp.py")
        ug = load("e2e_undergrowth", "e2e-undergrowth.py")
        shared = load("e2e_server", "e2e_server.py")
        self.assertEqual(xp.Server.__module__, shared.Server.__module__)
        self.assertEqual(ug.e2e_server.Server.__module__, shared.Server.__module__)
        for name in ("e2e-transcript.py", "e2e-word-filter.py"):
            self.assertNotIn("class Server", (scripts / name).read_text())
        for name in ("e2e-xp.py", "e2e-undergrowth.py"):
            self.assertNotIn("class Server", (scripts / name).read_text())


if __name__ == "__main__":
    unittest.main()
