#!/usr/bin/env python3
"""Unit tests for check-connector-sources.py: the copy must say what its source says, read from a local source dir.

Run from the repo root: python3 -m unittest discover -s scripts -p 'test_*.py'
SCRIPTS_DIR points the tests at another copy of the scripts, for mutation checks.
"""
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest

scripts = pathlib.Path(os.environ.get("SCRIPTS_DIR", pathlib.Path(__file__).resolve().parent))
MANIFEST = {"schema": 1, "id": "c", "name": "C", "version": "1.0.0", "provider": "connector", "addon_api": 7, "connector": {"permissions": ["view_channel"]}}
SOURCES = {"c": {"repo": "Slim-m-org/slim-bots", "path": "bots/c/connector.json"}}


class ConnectorSourcesTest(unittest.TestCase):
    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp)
        (self.tmp / "scripts").mkdir()
        shutil.copy(scripts / "check-connector-sources.py", self.tmp / "scripts")
        self.sources(SOURCES)
        self.copy(MANIFEST)
        self.source(MANIFEST)

    def sources(self, entries):
        (self.tmp / "scripts" / "connector-sources.json").write_text(json.dumps(entries))

    def copy(self, manifest, mid="c"):
        target = self.tmp / "modules" / mid
        target.mkdir(parents=True, exist_ok=True)
        (target / "manifest.json").write_text(json.dumps(manifest))

    def source(self, manifest):
        target = self.tmp / "src" / "slim-bots" / "bots" / "c"
        target.mkdir(parents=True, exist_ok=True)
        (target / "connector.json").write_text(json.dumps(manifest, indent=2))

    def run_check(self):
        args = [sys.executable, str(self.tmp / "scripts" / "check-connector-sources.py"), "--source-dir", str(self.tmp / "src")]
        return subprocess.run(args, capture_output=True, text=True)

    def test_a_copy_that_says_what_its_source_says_passes_whatever_its_formatting(self):
        done = self.run_check()
        self.assertEqual(done.returncode, 0, done.stderr)

    def test_a_copy_that_drifted_is_refused(self):
        self.source(dict(MANIFEST, connector={"permissions": ["view_channel", "send_messages"]}))
        done = self.run_check()
        self.assertEqual(done.returncode, 1)
        self.assertIn("c: manifest.json differs from Slim-m-org/slim-bots/bots/c/connector.json", done.stderr)

    def test_a_connector_with_no_source_entry_is_refused(self):
        self.sources({})
        done = self.run_check()
        self.assertEqual(done.returncode, 1)
        self.assertIn("no entry in scripts/connector-sources.json", done.stderr)

    def test_an_entry_for_a_module_that_is_not_a_connector_is_refused(self):
        self.sources({**SOURCES, "dice": {"repo": "Slim-m-org/slim-bots", "path": "x"}})
        self.copy({"id": "dice", "version": "1.0.0"}, "dice")
        done = self.run_check()
        self.assertEqual(done.returncode, 1)
        self.assertIn("dice: is in scripts/connector-sources.json but is not a connector module", done.stderr)

    def test_an_unreadable_source_is_refused_not_skipped(self):
        shutil.rmtree(self.tmp / "src")
        done = self.run_check()
        self.assertEqual(done.returncode, 1)
        self.assertIn("could not read its source", done.stderr)

    def test_the_repo_names_a_source_for_every_connector_it_publishes(self):
        repo = pathlib.Path(__file__).resolve().parent.parent
        published = {p.parent.name for p in (repo / "modules").glob("*/manifest.json") if json.loads(p.read_text()).get("provider") == "connector"}
        self.assertEqual(published, set(json.loads((repo / "scripts" / "connector-sources.json").read_text())))


if __name__ == "__main__":
    unittest.main()
