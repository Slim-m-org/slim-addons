#!/usr/bin/env python3
"""Runs every command-only module's commands through the wasm abi with a fixed input.

Usage: python3 scripts/check-command-modules.py <run-module-binary>

check-app-scenes.py only launches commands an `app` extension point names, so a
module with only text commands never had its alloc/run packing executed by CI.
Each command here must answer ok:true with the expected output; a command with
no entry in SAMPLES fails, so a new module has to add one.
"""
import json
import pathlib
import subprocess
import sys
import tempfile

root = pathlib.Path(__file__).resolve().parent.parent
runner = sys.argv[1]

# (module, command) -> (input, what the output must start with)
SAMPLES = {
    ("calc", "eval"): ("2 * (3 + 4)", "2 * (3 + 4) = 14"),
    ("code-exec", "run"): ("1 + 1", "2"),
    ("cron-explain", "explain"): ("*/15 * * * *", "minute       0, 15, 30, 45\nhour         every hour\n"),
    ("dice", "roll"): ("2d6+3", "2d6+3 = "),
    ("hashkit", "sha256"): ("abc", "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
    ("hashkit", "base64"): ("hi", "aGk="),
    ("hashkit", "hex"): ("hi", "6869"),
    ("json-tools", "format"): ('{"a":1}', '{\n  "a": 1\n}'),
    ("markdown-tools", "stats"): ("hello world", "Words: 2\n"),
    ("plot", "bars"): ("3, 7, 2", '{"$slim":"scene/1"'),
    ("reminder", "remind"): ("30m take the bread out", "Reminder set for 30 minutes from now: take the bread out"),
    ("reminder", "list"): ("", "1. in 30 minutes: take the bread out"),
    ("reminder", "cancel"): ("1", "Cancelled: take the bread out"),
    ("reminder", "due"): ("", "Posted 1 reminder."),
    ("regex-tester", "test"): ("\\d+ :: a1 b22", '2 matches\n1: "1" at 1..2'),
    ("tabletop", "roll"): ("d20 adv+5", "2d20kh1+5 = "),
    ("tabletop", "sheet"): ("help", "Sheet commands:"),
    ("table", "format"): ("a,b\n1,2", "a | b\n--+--\n1 | 2"),
    ("tags", "add"): ("rules Be kind", ""),
    # message.post is simulated now, so the text is posted and the output is empty.
    ("tags", "tag"): ("rules", ""),
    ("tags", "edit"): ("rules Be nice", ""),
    ("tags", "remove"): ("rules", ""),
    ("weather", "now"): ("Berlin", "Berlin, Germany: overcast, 12.3\u00b0C (feels like 10.1\u00b0C), wind 14 km/h"),
}

# Commands that need stored data first: (module, command) -> commands to run before it, in the same kv store.
SETUP = {
    ("tags", "tag"): [("add", "rules Be kind")],
    ("tags", "edit"): [("add", "rules Be kind")],
    ("tags", "remove"): [("add", "rules Be kind")],
    ("reminder", "list"): [("remind", "30m take the bread out")],
    ("reminder", "cancel"): [("remind", "30m take the bread out")],
    ("reminder", "due"): [("remind", "30m take the bread out")],
}

# The clock every run of a module with a time-dependent command is given.
NOW_MS = 1_700_000_000_000
# Per command, how much later than NOW_MS it runs.
LATER_MS = {("reminder", "due"): 31 * 60_000}
# Flags a module's commands need beyond the kv file.
FETCH_FILES = {"weather": root / "scripts" / "fixtures" / "weather-fetch.json"}

problems = []
for manifest in sorted((root / "modules").glob("*/manifest.json")):
    if manifest.parent.name == "_template":
        continue
    m = json.loads(manifest.read_text())
    apps = {e["command"] for e in m["extension_points"] if e["kind"] == "app"}
    for command in sorted({e["name"] for e in m["extension_points"] if e["kind"] == "command"} - apps):
        key = (m["id"], command)
        if key not in SAMPLES:
            problems.append(f"{m['id']} {command}: no sample input in SAMPLES")
            continue
        text, expected = SAMPLES[key]
        wasm = root / m["artifact"]["path"]
        with tempfile.TemporaryDirectory() as scratch:
            kv = ["--kv-file", f"{scratch}/kv", "--now-ms", str(NOW_MS)]
            if m["id"] in FETCH_FILES:
                kv += ["--fetch-file", str(FETCH_FILES[m["id"]])]
            for step, step_input in SETUP.get(key, []):
                subprocess.run([runner, *kv, str(wasm), step, step_input], capture_output=True, text=True)
            kv[kv.index("--now-ms") + 1] = str(NOW_MS + LATER_MS.get(key, 0))
            done = subprocess.run([runner, *kv, str(wasm), command, text], capture_output=True, text=True)
        try:
            reply = json.loads(done.stdout)
        except json.JSONDecodeError:
            problems.append(f"{m['id']} {command}: no JSON reply ({done.stderr.strip()[:200]})")
            continue
        output = reply.get("output", "")
        if done.returncode != 0 or reply.get("ok") is not True or not output.startswith(expected):
            problems.append(f"{m['id']} {command}: expected output starting {expected!r}, got {reply!r}")
        else:
            print(f"{m['id']} {command}: ok")

for p in problems:
    print(p, file=sys.stderr)
sys.exit(1 if problems else 0)
