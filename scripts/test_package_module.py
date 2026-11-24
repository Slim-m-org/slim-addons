#!/usr/bin/env python3
"""Unit tests for package-module.sh and build-wasm.sh refusals that need no toolchain.

Run from the repo root: python3 -m unittest discover -s scripts -p 'test_*.py'
SCRIPTS_DIR points the tests at another copy of the scripts, for mutation checks.
"""
import json
import os
import pathlib
import shutil
import stat
import subprocess
import tempfile
import unittest

scripts = pathlib.Path(os.environ.get("SCRIPTS_DIR", pathlib.Path(__file__).resolve().parent))


class PackageModuleTest(unittest.TestCase):
    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp)

    def package(self, mid, version):
        module = self.tmp / "module"
        module.mkdir(exist_ok=True)
        (module / "manifest.json").write_text(json.dumps({"id": mid, "version": version}))
        return subprocess.run([str(scripts / "package-module.sh"), str(module)], cwd=self.tmp, capture_output=True, text=True)

    def test_an_id_or_version_that_leaves_the_module_directory_is_refused_before_any_build(self):
        for mid, version in (("m", "../../x"), ("m", "1/2"), ("m", ".."), ("../m", "1.0.0"), ("a/b", "1.0.0"), ("m", "")):
            done = self.package(mid, version)
            self.assertEqual(done.returncode, 1, (mid, version))
            self.assertIn("not a plain path segment", done.stderr)
            self.assertFalse((self.tmp / "modules").exists())

    def test_the_toolchain_is_asked_from_the_module_directory(self):
        # A fake rustc that names its cwd shows which directory the sysroot call ran in.
        bin_dir = self.tmp / "bin"
        bin_dir.mkdir()
        rustc = bin_dir / "rustc"
        rustc.write_text('#!/usr/bin/env bash\npwd >> "$ASKED_FROM"\n[ "$1" = "--print" ] && echo /sysroot || echo "commit-hash: abc"\n')
        rustc.chmod(rustc.stat().st_mode | stat.S_IEXEC)
        module = self.tmp / "module"
        module.mkdir()
        asked = self.tmp / "asked"
        env = {**os.environ, "PATH": f"{bin_dir}:{os.environ['PATH']}", "ASKED_FROM": str(asked)}
        subprocess.run([str(scripts / "build-wasm.sh"), str(module), str(self.tmp / "out.wasm")], cwd=self.tmp, env=env, capture_output=True, text=True)
        self.assertEqual(set(asked.read_text().split()), {str(module.resolve())})


if __name__ == "__main__":
    unittest.main()
