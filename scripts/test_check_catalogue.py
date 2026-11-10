#!/usr/bin/env python3
"""Unit tests for check-catalogue.py, run against a throwaway copy of the repo layout.

Run from the repo root: python3 -m unittest discover -s scripts -p 'test_*.py'
SCRIPTS_DIR points the tests at another copy of the scripts, for mutation checks.
"""
import json
import os
import pathlib
import hashlib
import shutil
import subprocess
import sys
import tempfile
import unittest

from fixture_repo import Repo

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


def sha256(data):
    return hashlib.sha256(data).hexdigest()


class AgainstBaseTest(unittest.TestCase):
    """--base REF: what a pull request may not do to a module the base commit already published."""

    def setUp(self):
        self.repo = Repo()
        self.addCleanup(self.repo.cleanup)
        self.repo.write("scripts/check-catalogue.py", (scripts / "check-catalogue.py").read_text())
        self.publish("m", "0.1.0", b"wasm one")
        self.publish("_template", "0.1.0", b"template wasm")
        self.write_index(version="0.1.0")
        self.repo.commit()

    def publish(self, mid, version, wasm, **manifest_extra):
        path = f"modules/{mid}/{version}/module.wasm"
        self.repo.write(path, wasm)
        self.write_manifest(mid, version, wasm, **manifest_extra)

    def write_manifest(self, mid, version, wasm, summary="does a thing", changelog=None):
        manifest = {
            "id": mid, "name": mid, "version": version, "summary": summary,
            "artifact": {"path": f"modules/{mid}/{version}/module.wasm", "sha256": sha256(wasm)},
            "permissions": [{"key": "use"}],
            "extension_points": [{"kind": "command", "name": "run", "permission": "use"}],
        }
        if changelog is not None:
            manifest["changelog"] = changelog
        self.repo.write(f"modules/{mid}/manifest.json", json.dumps(manifest))

    def write_index(self, version, summary="does a thing"):
        row = {"id": "m", "name": "m", "version": version, "summary": summary}
        self.repo.write("index.json", json.dumps({"modules": [row]}))

    def check(self, *extra):
        return subprocess.run([sys.executable, "scripts/check-catalogue.py", *extra], cwd=self.repo.path, capture_output=True, text=True)

    def check_base(self):
        return self.check("--base", "HEAD")

    def test_nothing_changed_passes(self):
        done = self.check_base()
        self.assertEqual(done.returncode, 0, done.stderr)

    def test_a_published_wasm_that_changes_is_refused(self):
        self.publish("m", "0.1.0", b"wasm two")
        self.assertEqual(self.check().returncode, 0, "the pin follows the new bytes, so only --base can object")
        done = self.check_base()
        self.assertEqual(done.returncode, 1)
        self.assertIn("modules/m/0.1.0/module.wasm changed after it was published", done.stderr)

    def test_a_published_wasm_that_is_deleted_is_refused(self):
        self.repo.remove("modules/m/0.1.0/module.wasm")
        done = self.check_base()
        self.assertEqual(done.returncode, 1)
        self.assertIn("modules/m/0.1.0/module.wasm was published and is now gone", done.stderr)

    def test_a_manifest_edit_that_keeps_the_version_is_refused(self):
        self.write_manifest("m", "0.1.0", b"wasm one", summary="does another thing")
        self.write_index(version="0.1.0", summary="does another thing")
        self.assertEqual(self.check().returncode, 0, "the index follows the manifest, so only --base can object")
        done = self.check_base()
        self.assertEqual(done.returncode, 1)
        self.assertIn("manifest changed but version 0.1.0 did not", done.stderr)

    def test_editing_only_the_changelog_needs_no_version_bump(self):
        self.write_manifest("m", "0.1.0", b"wasm one", changelog=[{"version": "0.1.0", "changes": ["first"]}])
        done = self.check_base()
        self.assertEqual(done.returncode, 0, done.stderr)

    def test_a_new_version_beside_the_published_one_passes(self):
        self.publish("m", "0.2.0", b"wasm two")
        self.write_index(version="0.2.0")
        done = self.check_base()
        self.assertEqual(done.returncode, 0, done.stderr)

    def test_the_template_may_change_freely(self):
        self.publish("_template", "0.1.0", b"another template wasm", summary="reworded")
        done = self.check_base()
        self.assertEqual(done.returncode, 0, done.stderr)


if __name__ == "__main__":
    unittest.main()
