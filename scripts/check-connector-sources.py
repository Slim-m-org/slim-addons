#!/usr/bin/env python3
"""Checks that each connector's manifest here still says what its source says.

Run from the repo root: python3 scripts/check-connector-sources.py [--source-dir DIR]

A connector's logic lives in another repo, and the commands and settings its manifest names
have to match that code, so the source repo owns the manifest and this one publishes a copy.
scripts/connector-sources.json maps each connector module to its source file; the copy is
compared as JSON, so formatting may differ and meaning may not. By default the source is read
from the repo's main branch on raw.githubusercontent.com; --source-dir reads <repo name>/<path>
under DIR instead, for a local checkout and for the tests.
"""
import json
import pathlib
import sys
import urllib.request

root = pathlib.Path(__file__).resolve().parent.parent
RAW = "https://raw.githubusercontent.com/{repo}/main/{path}"
TIMEOUT_SECONDS = 20
problems = []


def fail(module, message):
    problems.append(f"{module}: {message}")


def connectors():
    found = set()
    for path in (root / "modules").glob("*/manifest.json"):
        if json.loads(path.read_text()).get("provider") == "connector":
            found.add(path.parent.name)
    return found


def read_source(entry, source_dir):
    if source_dir is not None:
        return (source_dir / entry["repo"].split("/")[-1] / entry["path"]).read_text()
    with urllib.request.urlopen(RAW.format(**entry), timeout=TIMEOUT_SECONDS) as answer:
        return answer.read().decode()


def main(argv):
    source_dir = pathlib.Path(argv[argv.index("--source-dir") + 1]) if "--source-dir" in argv else None
    sources = json.loads((root / "scripts" / "connector-sources.json").read_text())
    here = connectors()
    for mid in sorted(here - sources.keys()):
        fail(mid, "is a connector with no entry in scripts/connector-sources.json, so nothing says where its manifest comes from")
    for mid in sorted(sources.keys() - here):
        fail(mid, "is in scripts/connector-sources.json but is not a connector module here")
    for mid in sorted(here & sources.keys()):
        try:
            source = json.loads(read_source(sources[mid], source_dir))
        except (OSError, ValueError) as err:
            fail(mid, f"could not read its source {sources[mid]['repo']}/{sources[mid]['path']} ({type(err).__name__}: {err})")
            continue
        if source != json.loads((root / "modules" / mid / "manifest.json").read_text()):
            fail(mid, f"manifest.json differs from {sources[mid]['repo']}/{sources[mid]['path']}; copy the source here and bump the version")
    for p in problems:
        print(p, file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
