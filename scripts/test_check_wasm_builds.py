#!/usr/bin/env python3
"""Unit tests for check-wasm-builds.py: the allowlist rules and the lockfile rule, against a stubbed build.

Run from the repo root: python3 -m unittest discover -s scripts -p 'test_*.py'
SCRIPTS_DIR points the tests at another copy of the scripts, for mutation checks.
"""
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import unittest

from fixture_repo import Repo

scripts = pathlib.Path(os.environ.get("SCRIPTS_DIR", pathlib.Path(__file__).resolve().parent))

# Stands in for build-wasm.sh: "building" a module copies its built.bin, and a `fail` file breaks the build.
STUB_BUILD = """#!/usr/bin/env bash
[[ -f "$1/fail" ]] && { echo "boom" >&2; exit 1; }
cp "$1/built.bin" "$2"
"""


def sha256(data):
    return hashlib.sha256(data).hexdigest()


class Registry(unittest.TestCase):
    """A stub registry in a throwaway repo; the tests are in the subclasses."""

    def setUp(self):
        self.repo = Repo()
        self.addCleanup(self.repo.cleanup)
        self.repo.write("scripts/check-wasm-builds.py", (scripts / "check-wasm-builds.py").read_text())
        build = self.repo.write("scripts/build-wasm.sh", STUB_BUILD)
        build.chmod(0o755)
        self.allow([])
        self.module("m")

    def allow(self, modules):
        self.repo.write("scripts/wasm-unreproduced.txt", "# a comment\n" + "".join(f"{m}  # why\n" for m in modules))

    def module(self, mid, version="0.1.0", built=b"wasm", pinned=None, lock=True):
        manifest = {"id": mid, "version": version, "artifact": {"sha256": sha256(built if pinned is None else pinned)}}
        self.repo.write(f"modules/{mid}/manifest.json", json.dumps(manifest))
        self.repo.write(f"modules/{mid}/built.bin", built)
        self.repo.write(f"modules/{mid}/src/lib.rs", "// source")
        if lock:
            self.repo.write(f"modules/{mid}/Cargo.lock", "")
        elif (self.repo.path / f"modules/{mid}/Cargo.lock").exists():
            self.repo.remove(f"modules/{mid}/Cargo.lock")

    def check(self, *extra):
        return subprocess.run([sys.executable, "scripts/check-wasm-builds.py", *extra], cwd=self.repo.path, capture_output=True, text=True)

    def check_base(self):
        return self.check("--base", "HEAD")


class WasmBuildsTest(Registry):
    def test_a_module_that_rebuilds_to_its_pin_passes(self):
        done = self.check()
        self.assertEqual(done.returncode, 0, done.stderr)

    def test_a_rebuild_that_differs_from_the_pin_is_refused(self):
        self.module("m", built=b"new bytes", pinned=b"old bytes")
        done = self.check()
        self.assertEqual(done.returncode, 1)
        self.assertIn("a rebuild gives sha256", done.stderr)

    def test_a_failed_build_is_reported_with_its_output(self):
        self.repo.write("modules/m/fail", "")
        done = self.check()
        self.assertEqual(done.returncode, 1)
        self.assertIn("the wasm build failed", done.stderr)
        self.assertIn("boom", done.stderr)

    def test_a_module_with_no_cargo_lock_is_refused(self):
        self.module("m", lock=False)
        done = self.check()
        self.assertEqual(done.returncode, 1)
        self.assertIn("m: has no Cargo.lock", done.stderr)

    def test_an_unreproduced_module_may_differ_until_it_rebuilds_to_its_pin(self):
        self.module("m", built=b"new bytes", pinned=b"old bytes")
        self.allow(["m"])
        self.assertEqual(self.check().returncode, 0)
        self.module("m")
        done = self.check()
        self.assertEqual(done.returncode, 1)
        self.assertIn("remove it from scripts/wasm-unreproduced.txt", done.stderr)

    def test_an_allowlist_entry_that_is_not_a_module_is_refused(self):
        self.allow(["ghost"])
        done = self.check()
        self.assertEqual(done.returncode, 1)
        self.assertIn("ghost: is in scripts/wasm-unreproduced.txt but is not a module", done.stderr)


class AgainstBaseTest(Registry):
    """--base REF: the allowlist only shrinks, and a listed module may not change source without a new version."""

    def setUp(self):
        super().setUp()
        self.module("old", built=b"new bytes", pinned=b"old bytes")
        self.allow(["old"])
        self.repo.commit()

    def test_nothing_changed_passes(self):
        done = self.check_base()
        self.assertEqual(done.returncode, 0, done.stderr)

    def test_a_new_allowlist_entry_is_refused(self):
        self.module("m", built=b"new bytes", pinned=b"old bytes")
        self.allow(["old", "m"])
        done = self.check_base()
        self.assertEqual(done.returncode, 1)
        self.assertIn("m: was added to scripts/wasm-unreproduced.txt", done.stderr)

    def test_a_listed_module_that_base_never_had_is_refused_without_a_crash(self):
        self.module("fresh", built=b"new bytes", pinned=b"old bytes")
        self.allow(["old", "fresh"])
        done = self.check_base()
        self.assertEqual(done.returncode, 1)
        self.assertIn("fresh: was added to", done.stderr)
        self.assertNotIn("Traceback", done.stderr)

    def test_a_listed_module_whose_source_changed_under_the_same_version_is_refused(self):
        self.repo.write("modules/old/src/lib.rs", "// edited")
        done = self.check_base()
        self.assertEqual(done.returncode, 1)
        self.assertIn("old: source changed since HEAD but version 0.1.0 did not", done.stderr)

    def test_a_listed_module_whose_lockfile_changed_under_the_same_version_is_refused(self):
        self.repo.write("modules/old/Cargo.lock", "# bumped")
        done = self.check_base()
        self.assertEqual(done.returncode, 1)
        self.assertIn("old: source changed since HEAD", done.stderr)

    def test_a_listed_module_whose_source_changed_with_a_new_version_passes(self):
        self.module("old", version="0.2.0", built=b"newer bytes", pinned=b"older bytes")
        self.repo.write("modules/old/src/lib.rs", "// edited")
        done = self.check_base()
        self.assertEqual(done.returncode, 0, done.stderr)

    def test_a_listed_module_whose_manifest_alone_changed_passes(self):
        self.module("old", built=b"new bytes", pinned=b"other pin")
        done = self.check_base()
        self.assertEqual(done.returncode, 0, done.stderr)


if __name__ == "__main__":
    unittest.main()
