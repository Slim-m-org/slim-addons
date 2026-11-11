#!/usr/bin/env python3
"""Runs every command-only module's commands through the wasm abi with a fixed input.

Usage: python3 scripts/check-command-modules.py <run-module-binary>

check-app-scenes.py only launches commands an `app` extension point names, so a
module with only text commands never had its alloc/run packing executed by CI.
Each command here must answer ok:true with the expected output; a command with
no entry in SAMPLES fails, so a new module has to add one. Runs are held to the
manifest's fuel and wall limits, as production holds them.
"""
import importlib.util
import json
import pathlib
import subprocess
import sys
import tempfile

root = pathlib.Path(__file__).resolve().parent.parent
# One reminder, set before the commands that need one to exist.
BREAD = "30m take the bread out"

# (module, command) -> (input, what the output must start with; "" means the output must be empty)
SAMPLES = {
    ("calc", "eval"): ("2 * (3 + 4)", "2 * (3 + 4) = 14"),
    ("code-exec", "run"): ("1 + 1", "2"),
    ("cron-explain", "explain"): ("*/15 * * * *", "minute       0, 15, 30, 45\nhour         every hour\n"),
    ("dice", "roll"): ("2d6+3", "2d6+3 = "),
    ("game-night", "remind"): ("", "Reminded 0."),
    ("hashkit", "sha256"): ("abc", "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
    ("hashkit", "base64"): ("hi", "aGk="),
    ("hashkit", "hex"): ("hi", "6869"),
    ("json-tools", "format"): ('{"a":1}', '{\n  "a": 1\n}'),
    ("markdown-tools", "stats"): ("hello world", "Words: 2\n"),
    ("plot", "bars"): ("3, 7, 2", '{"$slim":"scene/1"'),
    ("reminder", "remind"): (BREAD, "Reminder set for 30 minutes from now: take the bread out"),
    ("reminder", "list"): ("", "1. in 30 minutes: take the bread out"),
    ("reminder", "cancel"): ("1", "Cancelled: take the bread out"),
    ("reminder", "due"): ("", "Posted 1 reminder."),
    ("regex-tester", "test"): ("\\d+ :: a1 b22", '2 matches\n1: "1" at 1..2'),
    ("tabletop", "roll"): ("d20 adv+5", "2d20kh1+5 = "),
    ("tabletop", "sheet"): ("help", "Sheet commands:"),
    ("table", "format"): ("a,b\n1,2", "a | b\n--+--\n1 | 2"),
    ("tags", "add"): ("rules Be kind", ""),
    ("tags", "tag"): ("rules", ""),
    ("tags", "edit"): ("rules Be nice", ""),
    ("tags", "remove"): ("rules", ""),
    ("transcript", "transcript"): ("", "No call is being transcribed right now."),
    ("transcript", "flush"): ("", "Posted 0."),
    ("xp", "catchup"): ("", "Awarded 2 sessions."),
    ("word-filter", "filter-test"): ("hello there", "Nothing in that is filtered."),
    ("weather", "now"): ("Berlin", "Berlin, Germany: overcast, 12.3\u00b0C (feels like 10.1\u00b0C), wind 14 km/h"),
    ("undergrowth", "almanac"): ("", "a quiet day; nothing to post"),
    ("undergrowth", "herald"): ("", "no season has been played yet"),
}

# Commands that need stored data first: (module, command) -> commands to run before it, in the same kv store.
SETUP = {
    ("tags", "tag"): [("add", "rules Be kind")],
    ("tags", "edit"): [("add", "rules Be kind")],
    ("tags", "remove"): [("add", "rules Be kind")],
    ("reminder", "list"): [("remind", BREAD)],
    ("reminder", "cancel"): [("remind", BREAD)],
    ("reminder", "due"): [("remind", BREAD)],
}

# What a command posts through message.post, which the runner prints to stderr and a command's output does not carry.
POSTS = {
    ("tags", "add"): "Added tag rules.",
    ("tags", "tag"): "Be kind",
    ("tags", "edit"): "Updated tag rules.",
    ("tags", "remove"): "Removed tag rules.",
}

# The clock every run of a module with a time-dependent command is given.
NOW_MS = 1_700_000_000_000
# Per command, how much later than NOW_MS it runs.
LATER_MS = {("reminder", "due"): 31 * 60_000}
# Flags a module's commands need beyond the kv file; data.store comes from the manifest and needs none.
FETCH_FILES = {"weather": root / "scripts" / "fixtures" / "weather-fetch.json"}
LOG_FILES = {"xp": root / "scripts" / "fixtures" / "xp-log.json"}


def load_scene_ops():
    spec = importlib.util.spec_from_file_location("check_scene_ops", root / "scripts" / "check-scene-ops.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def run_flags(mid, scratch, now_ms):
    flags = ["--kv-file", f"{scratch}/kv", "--data-file", f"{scratch}/data", "--now-ms", str(now_ms)]
    if mid in LOG_FILES:
        flags += ["--log-file", str(LOG_FILES[mid])]
    if mid in FETCH_FILES:
        flags += ["--fetch-file", str(FETCH_FILES[mid])]
    return flags


def run_once(runner, args, timeout):
    """One run as (reply, posts, problem): reply is None when the runner gave no answer, problem says why the run is unusable."""
    try:
        done = subprocess.run([runner, *args], capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return None, [], f"the run did not finish in {timeout:g}s"
    try:
        reply = json.loads(done.stdout)
    except json.JSONDecodeError:
        return None, [], f"no JSON reply ({done.stderr.strip()[:200]})"
    posts = [line.split(": ", 1)[1] for line in done.stderr.splitlines() if line.startswith("message.post #")]
    if done.returncode != 0 or reply.get("ok") is not True:
        return reply, posts, f"the run failed: {reply!r} {done.stderr.strip()[:200]}"
    return reply, posts, None


def output_matches(output, expected):
    # An empty expectation would match anything as a prefix, so it means the output is empty.
    return output.startswith(expected) if expected else output == ""


def check_sample(runner, m, command, wasm, budget):
    """Every problem with one command run on its sample input, after the commands it needs have run."""
    key = (m["id"], command)
    text, expected = SAMPLES[key]
    fuel, timeout = budget
    with tempfile.TemporaryDirectory() as scratch:
        for step, step_input in SETUP.get(key, []):
            _, _, problem = run_once(runner, [*fuel, *run_flags(m["id"], scratch, NOW_MS), wasm, step, step_input], timeout)
            if problem:
                return [f"setup {step!r}: {problem}"]
        now_ms = NOW_MS + LATER_MS.get(key, 0)
        reply, posts, problem = run_once(runner, [*fuel, *run_flags(m["id"], scratch, now_ms), wasm, command, text], timeout)
    if problem:
        return [problem]
    problems = []
    if not output_matches(reply.get("output", ""), expected):
        problems.append(f"expected output starting {expected!r}, got {reply!r}")
    if key in POSTS and POSTS[key] not in posts:
        problems.append(f"expected a message.post of {POSTS[key]!r}, got {posts!r}")
    return problems


def main(runner):
    scene_ops = load_scene_ops()
    problems = []
    for manifest in sorted((root / "modules").glob("*/manifest.json")):
        if manifest.parent.name == "_template":
            continue
        m = json.loads(manifest.read_text())
        apps = {e["command"] for e in m["extension_points"] if e["kind"] == "app"}
        wasm = str(root / m["artifact"]["path"])
        for command in sorted({e["name"] for e in m["extension_points"] if e["kind"] == "command"} - apps):
            if (m["id"], command) not in SAMPLES:
                problems.append(f"{m['id']} {command}: no sample input in SAMPLES")
                continue
            found = check_sample(runner, m, command, wasm, scene_ops.run_budget(wasm))
            problems += [f"{m['id']} {command}: {p}" for p in found]
            if not found:
                print(f"{m['id']} {command}: ok")
    for p in problems:
        print(p, file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1]))
