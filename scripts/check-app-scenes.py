#!/usr/bin/env python3
"""Launches every app in the catalogue, then answers each thing its scene offers, and checks every frame.

Usage: python3 scripts/check-app-scenes.py <run-module-binary>

A launch frame draws only what exists before anyone acts: word-guess has no text
ops until a guess, tic-tac-toe has no marks until a move. So from the launch
scene this takes every control, every distinct tap target and every input the
scene offers, sends each back as an action carrying the scene's own state, and
runs the reply through check-scene-ops too. The launch and the replies are sent
as two different callers, because seat-based modules answer an anonymous caller
with a refusal rather than a board.
"""
import importlib.util
import json
import pathlib
import sys

root = pathlib.Path(__file__).resolve().parent.parent
CALLERS = ("alice", "bob")
# Enough to touch every distinct target of the biggest board here; a scene of cells is one tap target per cell.
MAX_FOLLOW_UPS = 40
SAMPLE_TEXT = "abcde"


def load_scene_ops():
    spec = importlib.util.spec_from_file_location("check_scene_ops", root / "scripts" / "check-scene-ops.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def follow_ups(raw):
    """The actions a scene offers, as the client would send them, with the scene's state."""
    try:
        scene = json.loads(raw)
    except json.JSONDecodeError:
        return []
    if not isinstance(scene, dict):
        return []
    # The play control only toggles the client's timer; what the module is sent is step.
    actions = ["step" if c == "play" else c for c in scene.get("controls", []) if isinstance(c, str)]
    for op in scene.get("ops", []):
        if not isinstance(op, dict):
            continue
        tap = op.get("tap")
        if isinstance(tap, str):
            actions.append(f"{tap}:0,0" if op.get("op") == "cells" else tap)
        if op.get("op") == "input" and isinstance(op.get("submit"), str):
            actions.append(f"{op['submit']}:{SAMPLE_TEXT}")
    seen = list(dict.fromkeys(actions))[:MAX_FOLLOW_UPS]
    state = scene.get("state", "")
    return [json.dumps({"action": action, "state": state}) for action in seen]


def check_app(scene_ops, runner, wasm, command):
    """Every problem across the launch frame and the frames its offers lead to, as (label, problem)."""
    found = []
    launch = scene_ops.run_module(runner, wasm, command, "", CALLERS[0])
    frames = [("<launch>", launch)]
    for payload in follow_ups(launch):
        for caller in CALLERS:
            frames.append((f"{caller} {payload[:50]}", scene_ops.run_module(runner, wasm, command, payload, caller)))
    for label, raw in frames:
        found += [(label, problem) for problem in scene_ops.check_scene(raw)]
    return found, len(frames)


def main():
    runner = sys.argv[1]
    scene_ops = load_scene_ops()
    failed = False
    for manifest in sorted((root / "modules").glob("*/manifest.json")):
        if manifest.parent.name == "_template":
            continue
        m = json.loads(manifest.read_text())
        if m.get("provider") == "connector":
            continue
        for command in sorted({e["command"] for e in m["extension_points"] if e["kind"] in ("app", "canvas-tile")}):
            wasm = str(root / m["artifact"]["path"])
            problems, count = check_app(scene_ops, runner, wasm, command)
            print(f"{m['id']} {command}: {'FAILED' if problems else 'ok'} ({count} frames)")
            for label, problem in problems:
                print(f"  {label}: {problem}")
            failed = failed or bool(problems)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
