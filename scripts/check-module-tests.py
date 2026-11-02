#!/usr/bin/env python3
"""Fails a module whose src has no #[test], so `cargo test` cannot pass on zero tests.

Run from the repo root: python3 scripts/check-module-tests.py

An app (a module with an `app` extension point) also needs modules/<id>/play.json, the script
scripts/play-modules.py plays through the wasm; scripts/play-unwritten.txt lists the apps that
had none, and may only shrink.

scripts/module-tests-unwritten.txt lists the modules that had no tests when this
check was added. It may only shrink: a listed module that now has a test must be
removed, and a name that is not a module is refused.
"""
import json
import pathlib
import re
import sys

root = pathlib.Path(__file__).resolve().parent.parent
allow_file = "scripts/module-tests-unwritten.txt"
test_attr = re.compile(r"^\s*#\[test\]", re.MULTILINE)
problems = []

lines = ((line.split("#")[0].strip()) for line in (root / allow_file).read_text().splitlines())
allow = {line for line in lines if line}
modules = sorted(p.parent.name for p in (root / "modules").glob("*/Cargo.toml"))

for ghost in sorted(allow - set(modules)):
    problems.append(f"{ghost}: is in {allow_file} but is not a module")

for mid in modules:
    count = sum(len(test_attr.findall(f.read_text())) for f in (root / "modules" / mid / "src").rglob("*.rs"))
    if count == 0 and mid not in allow:
        problems.append(f"{mid}: has no #[test] in src, so `cargo test` passes on nothing; add unit tests")
    if count > 0 and mid in allow:
        problems.append(f"{mid}: now has tests, so remove it from {allow_file}")

play_allow_file = "scripts/play-unwritten.txt"
play_lines = ((line.split("#")[0].strip()) for line in (root / play_allow_file).read_text().splitlines())
play_allow = {line for line in play_lines if line}
for ghost in sorted(play_allow - set(modules)):
    problems.append(f"{ghost}: is in {play_allow_file} but is not a module")
for mid in modules:
    if mid == "_template":
        continue
    manifest = json.loads((root / "modules" / mid / "manifest.json").read_text())
    is_app = any(e.get("kind") == "app" for e in manifest.get("extension_points", []))
    has_play = (root / "modules" / mid / "play.json").is_file()
    if is_app and not has_play and mid not in play_allow:
        problems.append(f"{mid}: is an app with no play.json, so nothing plays it through the wasm; see scripts/play-modules.py")
    if has_play and mid in play_allow:
        problems.append(f"{mid}: now has a play.json, so remove it from {play_allow_file}")

for p in problems:
    print(p, file=sys.stderr)
sys.exit(1 if problems else 0)
