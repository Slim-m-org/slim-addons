#!/usr/bin/env python3
"""Rebuilds every module's wasm and compares it to the one its manifest pins.

Run from the repo root: python3 scripts/check-wasm-builds.py [--base REF]

check-catalogue.py only ties a wasm to the sha256 in its own manifest, so a
module whose source changed while its wasm did not would pass. This builds the
manifest's version with scripts/build-wasm.sh (pinned toolchain, remapped
paths, so the bytes do not depend on the machine) and fails on any difference.
A module with no committed Cargo.lock fails too: it would build against whatever
crates the registry serves that day.

scripts/wasm-unreproduced.txt lists the modules whose committed wasm was built
before that recipe existed and so cannot match yet. For those, --base falls
back to refusing a source change that does not come with a new version, and the
list may only shrink: a listed module that now matches must be removed, and
--base refuses a new entry.
"""
import hashlib
import json
import pathlib
import subprocess
import sys
import tempfile

root = pathlib.Path(__file__).resolve().parent.parent
allow_file = "scripts/wasm-unreproduced.txt"
problems = []


def fail(module, message):
    problems.append(f"{module}: {message}")


def read_allowlist(text):
    lines = (line.split("#")[0].strip() for line in text.splitlines())
    return {line for line in lines if line}


def git(*args):
    return subprocess.run(["git", *args], cwd=root, capture_output=True, text=True)


def built_sha(module_dir, out):
    done = subprocess.run([str(root / "scripts/build-wasm.sh"), str(module_dir), str(out)], capture_output=True, text=True)
    if done.returncode != 0:
        return None, done.stderr[-2000:]
    return hashlib.sha256(out.read_bytes()).hexdigest(), ""


def source_changed_without_new_version(base, mid, version):
    old = git("show", f"{base}:modules/{mid}/manifest.json")
    if old.returncode != 0:
        return False
    if json.loads(old.stdout)["version"] != version:
        return False
    paths = [f"modules/{mid}/{p}" for p in ("src", "Cargo.toml", "Cargo.lock", ".cargo")]
    return bool(git("diff", "--name-only", base, "--", *paths).stdout.strip())


base = sys.argv[sys.argv.index("--base") + 1] if "--base" in sys.argv else None
allow = read_allowlist((root / allow_file).read_text())
modules = sorted(p.parent.name for p in (root / "modules").glob("*/manifest.json") if p.parent.name != "_template")

for ghost in sorted(allow - set(modules)):
    fail(ghost, f"is in {allow_file} but is not a module")

if base:
    old_list = git("show", f"{base}:{allow_file}")
    if old_list.returncode == 0:
        for added in sorted(allow - read_allowlist(old_list.stdout)):
            fail(added, f"was added to {allow_file}; rebuild it with scripts/package-module.sh instead")

with tempfile.TemporaryDirectory() as tmp:
    for mid in modules:
        manifest = json.loads((root / "modules" / mid / "manifest.json").read_text())
        if not (root / "modules" / mid / "Cargo.lock").is_file():
            fail(mid, "has no Cargo.lock; run cargo generate-lockfile in it and commit the result, because an unlocked build is not reproducible")
            continue
        sha, error = built_sha(root / "modules" / mid, pathlib.Path(tmp) / f"{mid}.wasm")
        if sha is None:
            fail(mid, f"the wasm build failed:\n{error}")
        elif sha == manifest["artifact"]["sha256"]:
            if mid in allow:
                fail(mid, f"rebuilds to its pinned wasm, so remove it from {allow_file}")
        elif mid not in allow:
            fail(mid, f"a rebuild gives sha256 {sha}, not the pinned {manifest['artifact']['sha256']}; run scripts/package-module.sh and bump the version")
        elif base and source_changed_without_new_version(base, mid, manifest["version"]):
            fail(mid, f"source changed since {base} but version {manifest['version']} did not; run scripts/package-module.sh and bump it")

for p in problems:
    print(p, file=sys.stderr)
sys.exit(1 if problems else 0)
