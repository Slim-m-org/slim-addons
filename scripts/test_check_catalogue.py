#!/usr/bin/env python3
"""Unit tests for check-catalogue.py, run against a throwaway copy of the repo layout.

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
repo = pathlib.Path(__file__).resolve().parent.parent


class CatalogueTest(unittest.TestCase):
    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp)
        (self.tmp / "scripts").mkdir()
        shutil.copy(scripts / "check-catalogue.py", self.tmp / "scripts")
        shutil.copy(repo / "index.json", self.tmp)
        for source in sorted((repo / "modules").glob("*/manifest.json")):
            target = self.tmp / "modules" / source.parent.name
            target.mkdir(parents=True)
            shutil.copy(source, target)
            for version in source.parent.iterdir():
                if version.is_dir() and version.name[0].isdigit():
                    (target / version.name).symlink_to(version)

    def run_check(self):
        return subprocess.run([sys.executable, str(self.tmp / "scripts" / "check-catalogue.py")], capture_output=True, text=True)

    def edit_manifest(self, module, change):
        path = self.tmp / "modules" / module / "manifest.json"
        manifest = json.loads(path.read_text())
        change(manifest)
        path.write_text(json.dumps(manifest))

    def test_an_archived_module_may_not_stay_listed(self):
        archived = self.tmp / "archive" / "dice"
        archived.mkdir(parents=True)
        shutil.copy(self.tmp / "modules" / "dice" / "manifest.json", archived)
        done = self.run_check()
        self.assertEqual(done.returncode, 1)
        self.assertIn("archived but still listed", done.stderr)

    def test_the_repo_as_it_stands_passes(self):
        done = self.run_check()
        self.assertEqual(done.returncode, 0, done.stderr)

    def set_ref(self, ref):
        index = json.loads((self.tmp / "index.json").read_text())
        index["modules"][0]["ref"] = ref
        (self.tmp / "index.json").write_text(json.dumps(index))

    def test_an_index_ref_must_be_a_sha_or_a_tag_name(self):
        for good in ("3f9c1c0b1a2d4e5f60718293a4b5c6d7e8f90a1b", "dice-v0.3.3", "v1.2"):
            self.set_ref(good)
            self.assertEqual(self.run_check().returncode, 0, good)
        for bad in ("../main", "a/b", "", "has space", "x..y", 7):
            self.set_ref(bad)
            done = self.run_check()
            self.assertEqual(done.returncode, 1, repr(bad))
            self.assertIn("is not a commit sha or a tag name", done.stderr)

    def test_a_duplicated_index_row_is_refused(self):
        index = json.loads((self.tmp / "index.json").read_text())
        index["modules"].append(dict(index["modules"][0], version="9.9.9"))
        (self.tmp / "index.json").write_text(json.dumps(index))
        done = self.run_check()
        self.assertEqual(done.returncode, 1)
        self.assertIn("more than once", done.stderr)

    def test_a_version_bump_that_keeps_the_old_artifact_path_is_refused(self):
        def bump(manifest):
            manifest["version"] = "9.9.9"

        self.edit_manifest("table", bump)
        index = json.loads((self.tmp / "index.json").read_text())
        for row in index["modules"]:
            if row["id"] == "table":
                row["version"] = "9.9.9"
        (self.tmp / "index.json").write_text(json.dumps(index))
        done = self.run_check()
        self.assertEqual(done.returncode, 1)
        self.assertIn("another version's wasm", done.stderr)

    def test_a_changelog_whose_newest_entry_is_another_version_is_refused(self):
        def stale(manifest):
            manifest["changelog"] = [{"version": "0.0.1", "changes": ["old"]}]

        self.edit_manifest("table", stale)
        done = self.run_check()
        self.assertEqual(done.returncode, 1)
        self.assertIn("newest changelog entry", done.stderr)

    def test_a_changelog_line_with_a_hidden_character_is_refused(self):
        def hidden(manifest):
            manifest["changelog"][0]["changes"] = ["a\u202eb"]

        self.edit_manifest("table", hidden)
        done = self.run_check()
        self.assertEqual(done.returncode, 1)
        self.assertIn("hidden characters", done.stderr)


if __name__ == "__main__":
    unittest.main()
