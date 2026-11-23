#!/usr/bin/env python3
"""Checks index.json against each module's manifest and pinned wasm, or, for a connector, what it may declare.

Run from the repo root: python3 scripts/check-catalogue.py [--base REF]

With --base, also refuses any change to a wasm that REF already published, and
any manifest edit that keeps the version REF had, because an installed module
only sees a change when its version is bumped.
"""
import hashlib
import json
import pathlib
import re
import subprocess
import sys

root = pathlib.Path(__file__).resolve().parent.parent
problems = []


def fail(module, message):
    problems.append(f"{module}: {message}")


def check_changelog(mid, m):
    """Release notes, held to the limits slim-m's server enforces."""
    changelog = m.get("changelog", [])
    if not isinstance(changelog, list):
        return fail(mid, "changelog is not a list")
    if changelog and changelog[0].get("version") != m["version"]:
        fail(mid, f"the newest changelog entry is not version {m['version']}")
    if len(changelog) > 20:
        fail(mid, "changelog has more than 20 entries; the server keeps only the newest 20")
    for entry in changelog:
        version, changes = entry.get("version"), entry.get("changes", [])
        if not isinstance(version, str) or not 1 <= len(version.strip()) <= 64:
            fail(mid, "a changelog entry needs a version of 1 to 64 characters")
        if not isinstance(changes, list) or len(changes) > 20:
            fail(mid, f"changelog {version} needs at most 20 lines")
            continue
        for line in changes:
            if not isinstance(line, str) or not 1 <= len(line.strip()) <= 300 or any(ord(c) < 32 or 0x200E <= ord(c) <= 0x202E or 0x2066 <= ord(c) <= 0x2069 for c in line):
                fail(mid, f"changelog {version} has a line that is empty, over 300 characters, or holds hidden characters")


CONNECTOR_API = 7
# What a connector may not declare: a sandbox the host runs, or one the host starts on its own (slim-m connectors.md).
CONNECTOR_REFUSED = ("artifact", "runtime", "capabilities", "network", "schedules", "subscriptions", "interceptors", "permission_hooks", "data", "call_audio", "slots")
CONNECTOR_POINTS = ("command", "slash-command", "panel")
PINNED_IMAGE = re.compile(r"[a-z0-9][a-z0-9./_-]*@sha256:[0-9a-f]{64}")
PERMISSION_NAME = re.compile(r"[a-z][a-z_]*")


def is_connector(m):
    return m.get("provider") == "connector"


def check_connector(mid, m):
    """A connector has no wasm to pin, so what it may and may not declare is the whole check; the server has the last word."""
    if not isinstance(m.get("addon_api"), int) or m["addon_api"] < CONNECTOR_API:
        fail(mid, f"a connector needs \"addon_api\": {CONNECTOR_API} or higher, or an older server installs it as a module with no artifact")
    for key in CONNECTOR_REFUSED:
        if key in m:
            fail(mid, f"a connector declares no {key}; the server refuses the manifest")
    for e in m.get("extension_points", []):
        if e["kind"] not in CONNECTOR_POINTS:
            fail(mid, f"extension point {e['name']} is a {e['kind']}, which a connector cannot declare")
    for s in m.get("settings", []):
        if s.get("type") == "secret" and "hosts" in s:
            fail(mid, f"secret setting {s['key']} takes no hosts, since the host sends a connector the value itself")
    connector = m.get("connector")
    if not isinstance(connector, dict) or set(connector) - {"permissions", "host_run"}:
        return fail(mid, "connector must be an object with permissions and, optionally, host_run")
    names = connector.get("permissions")
    if not isinstance(names, list) or not names or not all(isinstance(n, str) and PERMISSION_NAME.fullmatch(n) for n in names) or len(set(names)) != len(names):
        fail(mid, "connector.permissions must list each core permission once, by its snake_case name")
    image = (connector.get("host_run") or {}).get("image")
    if "host_run" in connector and not (isinstance(image, str) and PINNED_IMAGE.fullmatch(image)):
        fail(mid, "connector.host_run.image must be <registry>/<path>@sha256:<64 hex>, never a tag")
    if not (root / "modules" / mid / "README.md").is_file():
        fail(mid, "has no README.md; an admin reads it before approving what the connector asks for")


def without_changelog(manifest):
    return {k: v for k, v in manifest.items() if k != "changelog"}


index = json.loads((root / "index.json").read_text())
listed = {}
for row in index["modules"]:
    if row["id"] in listed:
        fail(row["id"], "is listed in index.json more than once")
    listed[row["id"]] = row
on_disk = {p.parent.name for p in (root / "modules").glob("*/manifest.json") if p.parent.name != "_template"}
for missing in sorted(on_disk - listed.keys()):
    fail(missing, "has a manifest but no index.json row")
for ghost in sorted(listed.keys() - on_disk):
    fail(ghost, "is in index.json but has no manifest")

for mid in sorted(on_disk & listed.keys()):
    m = json.loads((root / "modules" / mid / "manifest.json").read_text())
    row = listed[mid]
    for key in ("name", "version", "summary"):
        if m.get(key) != row.get(key):
            fail(mid, f"index.json {key} differs from the manifest")
    ref = row.get("ref")
    if ref is not None and not (isinstance(ref, str) and re.fullmatch(r"[0-9a-f]{40}|[A-Za-z0-9][A-Za-z0-9._-]{0,63}", ref) and ".." not in ref):
        fail(mid, f"index.json ref {ref!r} is not a commit sha or a tag name")
    if m["id"] != mid:
        fail(mid, "manifest id does not match its directory")
    if is_connector(m):
        check_connector(mid, m)
    elif "artifact" not in m:
        fail(mid, "has no artifact; only a \"provider\": \"connector\" manifest may leave it out")
    else:
        wasm = root / m["artifact"]["path"]
        expected_path = f"modules/{mid}/{m['version']}/module.wasm"
        if m["artifact"]["path"] != expected_path:
            fail(mid, f"artifact path {m['artifact']['path']} is not {expected_path}, so version {m['version']} would ship another version's wasm")
        if not wasm.is_file():
            fail(mid, f"artifact {m['artifact']['path']} is missing")
        elif hashlib.sha256(wasm.read_bytes()).hexdigest() != m["artifact"]["sha256"]:
            fail(mid, "artifact sha256 does not match the pinned value")
    keys = {p["key"] for p in m.get("permissions", [])}
    names = {e["name"] for e in m["extension_points"] if e["kind"] == "command"}
    for e in m["extension_points"]:
        if e.get("permission") not in keys:
            fail(mid, f"extension point {e['name']} uses an undeclared permission")
        if e["kind"] in ("slash-command", "code-block-runner", "app", "canvas-tile") and e.get("command") not in names:
            fail(mid, f"extension point {e['name']} runs an undeclared command")
        if e["kind"] == "canvas-tile" and m.get("addon_api", 0) < 6:
            fail(mid, f"canvas tile {e['name']} needs \"addon_api\": 6, or a server refuses the install")
    check_changelog(mid, m)

def at_base(ref, path):
    done = subprocess.run(["git", "show", f"{ref}:{path}"], cwd=root, capture_output=True)
    return done.stdout if done.returncode == 0 else None


def check_against_base(ref):
    listing = subprocess.run(["git", "ls-tree", "-r", "--name-only", ref, "modules/"], cwd=root, capture_output=True, text=True, check=True)
    for path in listing.stdout.split():
        parts = path.split("/")
        if len(parts) == 4 and parts[3] == "module.wasm" and parts[1] != "_template":
            now = root / path
            if not now.is_file():
                fail(parts[1], f"{path} was published and is now gone")
            elif now.read_bytes() != at_base(ref, path):
                fail(parts[1], f"{path} changed after it was published; ship a new version instead")
        if len(parts) == 3 and parts[2] == "manifest.json" and parts[1] != "_template":
            old = json.loads(at_base(ref, path))
            now = root / path
            # Release notes are read fresh from the registry, so editing only them needs no version bump.
            if now.is_file() and without_changelog(json.loads(now.read_text())) != without_changelog(old) and json.loads(now.read_text())["version"] == old["version"]:
                fail(parts[1], f"manifest changed but version {old['version']} did not")


if "--base" in sys.argv:
    check_against_base(sys.argv[sys.argv.index("--base") + 1])

for p in problems:
    print(p, file=sys.stderr)
sys.exit(1 if problems else 0)
