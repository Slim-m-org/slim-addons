#!/usr/bin/env python3
"""Plays each game module through scripted games and checks what every frame says.

Usage: python3 scripts/play-modules.py <run-module-binary> [--only <module id>] [--show]

A script, or one scenario in it, may set `entropy`, the run's seed (default
"play-modules"), so a dealt board is the same every run, and `settings`, values
for the manifest's settings. A fixed entropy also repeats an id a module derives
from it, so steps that make two things in one scenario give each its own `now_ms`.

A module with an `app` extension point keeps its play script in
`modules/<id>/play.json`. A script is a list of scenarios, each a list of steps
played in order against the real wasm through run-module, with the data.store
and kv.store the module is given kept between steps. Every step sends one
action the way the client does (the action plus the state the last frame
carried), then checks the frame it gets back:

    {"command": "play", "scenarios": [
      {"name": "x wins the top row", "steps": [
        {"as": "a1", "do": "p0", "expect": {"status_has": "O to move", "taps_lack": ["p0"]}},
        {"as": "b2", "do": "p3", "save": "mid"},
        {"as": "a1", "do": "p1", "state": "@mid"}]}]}

Step keys: `as` (caller id, carried forward), `do` (the action; leave it out for
the launch frame), `tick` (run a scheduled command instead, as the module's own
principal), `raw` (a literal input, for input that is not a frame at
all), `state` (a literal state, or `@name` for one a step saved; otherwise the
state the last frame carried), `fresh` (send no state), `save`, `now_ms`.
Expect keys: `ok`, `error_has`, `status`, `status_has`, `controls`,
`controls_has`, `controls_lack`, `input`, `live`, `taps_has`, `taps_lack`, `text_has`,
`text_lack`, `state`, `state_has`, `state_unchanged`, `output_has`, `posts_has`, `posts_count`. `"ok": null` accepts either.

Beyond what a step expects, every frame must pass check-scene-ops and every
run must finish inside the manifest's fuel, so a hostile state is a failure
only when it hangs or breaks the scene. `--show` prints each frame.
"""
import importlib.util
import json
import os
import pathlib
import subprocess
import sys
import tempfile

root = pathlib.Path(__file__).resolve().parent.parent
STEP_KEYS = {"as", "do", "raw", "state", "fresh", "save", "now_ms", "expect", "tick"}
TICK = "@tick:"
EXPECT_KEYS = {"ok", "error_has", "status", "status_has", "controls", "controls_has", "controls_lack", "input", "live",
               "taps_has", "taps_lack", "text_has", "text_lack", "state", "state_has", "state_unchanged", "output_has", "posts_has",
               "posts_count"}
DEFAULT_CALLER = "a1"
DEFAULT_NOW_MS = 1_700_000_000_000


def load_scene_ops():
    spec = importlib.util.spec_from_file_location("check_scene_ops", root / "scripts" / "check-scene-ops.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def summarize(reply):
    """The parts of a reply a step can expect something of; scene is None when it is not a scene."""
    out = {"ok": reply.get("ok") is True, "error": reply.get("error") or "", "scene": None,
           "output": reply.get("output") or "", "posts": reply.get("posts", [])}
    try:
        scene = json.loads(reply.get("output") or "")
    except json.JSONDecodeError:
        return out
    if not isinstance(scene, dict) or scene.get("$slim") != "scene/1":
        return out
    ops = [op for op in scene.get("ops", []) if isinstance(op, dict)]
    out["scene"] = scene
    out["status"] = scene.get("status", "")
    out["controls"] = [c for c in scene.get("controls", []) if isinstance(c, str)]
    out["taps"] = sorted({op["tap"] for op in ops if isinstance(op.get("tap"), str)})
    out["input"] = any(op.get("op") == "input" for op in ops)
    out["texts"] = [op["s"] for op in ops if op.get("op") == "text" and isinstance(op.get("s"), str)]
    out["state"] = scene.get("state")
    out["live"] = scene.get("live")
    return out


def listed(value):
    return value if isinstance(value, list) else [value]


def check_expect(got, expect, previous_state):
    """Every way the frame differs from what the step expected."""
    problems = [f"unknown expectation {key!r}, which nothing would check" for key in sorted(set(expect) - EXPECT_KEYS)]
    wanted_ok = expect.get("ok", True)
    if wanted_ok is not None and got["ok"] != wanted_ok:
        problems.append(f"ok is {got['ok']} ({got['error'] or 'no error'}), expected {wanted_ok}")
    if "error_has" in expect and expect["error_has"] not in got["error"]:
        problems.append(f"error {got['error']!r} lacks {expect['error_has']!r}")
    needs_scene = any(k not in ("ok", "error_has", "output_has", "posts_has", "posts_count") for k in expect)
    if "output_has" in expect and expect["output_has"] not in got["output"]:
        problems.append(f"output {got['output']!r} lacks {expect['output_has']!r}")
    problems += [f"no post with {part!r} in {got['posts']}" for part in listed(expect.get("posts_has", [])) if not any(part in p for p in got["posts"])]
    if "posts_count" in expect and len(got["posts"]) != expect["posts_count"]:
        problems.append(f"{len(got['posts'])} posts {got['posts']}, expected {expect['posts_count']}")
    if got["scene"] is None:
        return problems + (["the reply is not a scene"] if needs_scene and got["ok"] else [])
    status, controls = got["status"], got["controls"]
    if "status" in expect and status != expect["status"]:
        problems.append(f"status {status!r}, expected {expect['status']!r}")
    for part in listed(expect.get("status_has", [])):
        if part not in status:
            problems.append(f"status {status!r} lacks {part!r}")
    if "controls" in expect and controls != expect["controls"]:
        problems.append(f"controls {controls}, expected {expect['controls']}")
    problems += [f"controls {controls} lack {c!r}" for c in expect.get("controls_has", []) if c not in controls]
    problems += [f"controls {controls} have {c!r}" for c in expect.get("controls_lack", []) if c in controls]
    if "live" in expect and got["live"] != expect["live"]:
        problems.append(f"live is {got['live']}, expected {expect['live']}")
    if "input" in expect and got["input"] != expect["input"]:
        problems.append(f"input present is {got['input']}, expected {expect['input']}")
    problems += [f"no tap target {t!r}" for t in expect.get("taps_has", []) if t not in got["taps"]]
    problems += [f"tap target {t!r} is offered" for t in expect.get("taps_lack", []) if t in got["taps"]]
    shown = "\n".join(got["texts"])
    problems += [f"no text {t!r}" for t in expect.get("text_has", []) if t not in shown]
    problems += [f"text {t!r} is shown" for t in expect.get("text_lack", []) if t in shown]
    state = got["state"]
    if "state" in expect and state != expect["state"]:
        problems.append(f"state {state!r}, expected {expect['state']!r}")
    if "state_has" in expect and expect["state_has"] not in (state or ""):
        problems.append(f"state {state!r} lacks {expect['state_has']!r}")
    if expect.get("state_unchanged") and state != previous_state:
        problems.append(f"state changed from {previous_state!r} to {state!r}")
    return problems


def step_payload(step, carried, saved):
    if "tick" in step:
        return TICK + step["tick"]
    if "raw" in step:
        return step["raw"]
    if "do" not in step:
        return ""
    state = carried
    if step.get("fresh"):
        state = ""
    if "state" in step:
        state = saved.get(step["state"][1:], "") if step["state"].startswith("@") else step["state"]
    return json.dumps({"action": step["do"], "state": state or ""})


def play_scenario(run, scenario, check_scene):
    """Plays one scenario; `run(caller, payload, now_ms)` answers the reply dict. Problems as (step label, text)."""
    found = []
    carried, saved, caller, now_ms = None, {}, DEFAULT_CALLER, scenario.get("now_ms", DEFAULT_NOW_MS)
    for index, step in enumerate(scenario["steps"], 1):
        label = f"step {index} ({step.get('as', caller)} {step.get('tick', step.get('do', step.get('raw', 'launch')))!r:.40})"
        unknown = sorted(set(step) - STEP_KEYS)
        if unknown:
            found.append((label, f"unknown step key {unknown[0]!r}, which nothing would read"))
            break
        caller = step.get("as", caller)
        now_ms = step.get("now_ms", now_ms)
        try:
            reply = run(caller, step_payload(step, carried, saved), now_ms)
        except RuntimeError as err:
            found.append((label, str(err)))
            break
        got = summarize(reply)
        found += [(label, p) for p in check_expect(got, step.get("expect", {}), carried)]
        if got["scene"] is not None:
            found += [(label, p) for p in check_scene(reply["output"])]
            if isinstance(got["state"], str):
                carried = got["state"]
        if "save" in step:
            saved[step["save"]] = carried or ""
    return found


def runner_for(runner, wasm, manifest, scratch, script):
    limits = manifest["runtime"]["limits"]
    timeout = limits.get("wall_ms", 1000) / 1000 * 10 + 10
    # A fixed entropy makes a module that rolls dice or deals a board play the same game every run.
    env = {**os.environ, "RUN_MODULE_ENTROPY": script.get("entropy", "play-modules")}
    settings = [arg for key, value in script.get("settings", {}).items() for arg in ("--setting", f"{key}={value}")]

    def run(caller, payload, now_ms, command):
        common = ["--fuel", str(limits["fuel"]), "--now-ms", str(now_ms), "--kv-file", f"{scratch}/kv", "--data-file", f"{scratch}/data", *settings]
        if payload.startswith(TICK):
            argv = [runner, "--caller", caller, *common, "--tick", payload[len(TICK):], wasm]
        else:
            argv = [runner, "--caller", caller, *common, wasm, command, payload]
        try:
            done = subprocess.run(argv, capture_output=True, text=True, timeout=timeout, env=env)
        except subprocess.TimeoutExpired:
            raise RuntimeError(f"the run did not finish in {timeout:g}s")
        except OSError as err:
            raise RuntimeError(f"the runner could not start: {err}")
        if done.returncode != 0:
            raise RuntimeError(f"the runner failed: {done.stderr.strip()[:200]}")
        reply = json.loads(done.stdout)
        reply["posts"] = [line.split(": ", 1)[1] for line in done.stderr.splitlines() if line.startswith("message.post #")]
        return reply

    return run


def play_module(runner, manifest_path, scene_ops, show):
    manifest = json.loads(manifest_path.read_text())
    script = json.loads((manifest_path.parent / "play.json").read_text())
    wasm = str(root / manifest["artifact"]["path"])
    results = []
    for scenario in script["scenarios"]:
        with tempfile.TemporaryDirectory() as scratch:
            run = runner_for(runner, wasm, manifest, scratch, {**script, **scenario})

            def traced(caller, payload, now_ms, run=run):
                reply = run(caller, payload, now_ms, script["command"])
                if show:
                    print(f"    {caller} <- {payload[:100]}\n    -> {json.dumps(summarize(reply), default=str)[:400]}")
                return reply

            problems = play_scenario(traced, scenario, scene_ops.check_scene)
        results.append((scenario["name"], problems))
    return results


def parse_args(args):
    """(runner, only, show) from the command line, or None when it is not one this script understands."""
    only = None
    if "--only" in args:
        at = args.index("--only")
        only = args[at + 1] if at + 1 < len(args) else None
        if not only or only.startswith("--"):
            return None
    positional = [a for i, a in enumerate(args) if not a.startswith("--") and (i == 0 or args[i - 1] != "--only")]
    if len(positional) != 1:
        return None
    return positional[0], only, "--show" in args


def main(argv):
    parsed = parse_args(argv[1:])
    if parsed is None:
        print(next(part for part in __doc__.split("\n\n") if part.startswith("Usage:")), file=sys.stderr)
        return 2
    runner, only, show = parsed
    plays = sorted((root / "modules").glob("*/play.json"))
    if only and only not in {play.parent.name for play in plays}:
        print(f"no play.json for module {only!r}; scripts exist for: {', '.join(play.parent.name for play in plays)}", file=sys.stderr)
        return 2
    scene_ops = load_scene_ops()
    failed = False
    for play in plays:
        if only and play.parent.name != only:
            continue
        print(play.parent.name)
        for name, problems in play_module(runner, play.parent / "manifest.json", scene_ops, show):
            print(f"  {'FAILED' if problems else 'ok'}  {name}")
            for label, problem in problems:
                print(f"      {label}: {problem}")
            failed = failed or bool(problems)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
