#!/usr/bin/env python3
"""Checks that tools/run-module runs the wasmi version slim-m's server runs.

Run from the repo root: python3 scripts/check-wasmi-version.py [--slim-lock FILE]

The CI gates that hold a module to its fuel and wall limits run it on run-module, so an
engine that drifts from the server's tests something production does not run. By default
slim-m's Cargo.lock is read from its main branch on raw.githubusercontent.com; --slim-lock
reads a local file instead, for a checkout and for the tests.
"""
import pathlib
import sys
import tomllib
import urllib.request

root = pathlib.Path(__file__).resolve().parent.parent
SLIM_LOCK = "https://raw.githubusercontent.com/Slim-m-org/slim-m/main/Cargo.lock"
TIMEOUT_SECONDS = 20


def wasmi_versions(lock_text):
    packages = tomllib.loads(lock_text).get("package", [])
    return sorted(p["version"] for p in packages if p["name"] == "wasmi")


def read_slim_lock(path):
    if path is not None:
        lock = pathlib.Path(path).resolve()
        if lock.suffix != ".lock" or not lock.is_file():
            raise SystemExit(f"--slim-lock must name a .lock file, got {path}")
        return lock.read_text()
    with urllib.request.urlopen(SLIM_LOCK, timeout=TIMEOUT_SECONDS) as answer:
        return answer.read().decode()


def main(argv):
    path = argv[argv.index("--slim-lock") + 1] if "--slim-lock" in argv else None
    ours = wasmi_versions((root / "tools/run-module/Cargo.lock").read_text())
    theirs = wasmi_versions(read_slim_lock(path))
    if len(ours) != 1 or len(theirs) != 1:
        print(f"expected one wasmi in each lockfile, found run-module {ours} and slim-m {theirs}", file=sys.stderr)
        return 1
    if ours != theirs:
        print(f"run-module locks wasmi {ours[0]} but slim-m's server runs {theirs[0]}; bump tools/run-module so CI tests the engine production runs", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
