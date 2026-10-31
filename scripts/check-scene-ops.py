#!/usr/bin/env python3
# Validates the scenes a module actually emits against slim-m's renderer.
#
# The renderer drops an op it cannot parse and falls back on a colour name it
# does not know, both silently. So a module can build, pass its unit tests,
# run correctly under wasmi, and still draw a board with no numbers on it -
# which is exactly how minesweeper 0.1.0 shipped. Nothing in the toolchain
# compared the emitted JSON against the client that has to read it.
#
# This does. Point it at a built module and the frames you want checked:
#
#   scripts/check-scene-ops.py modules/minesweeper/0.1.0/module.wasm play ""
#
# It is deliberately a copy of the contract rather than an import of it: the
# renderer lives in the slim-m repo, and a registry that could only be
# validated from a checkout of another repo would not get validated.
import json
import pathlib
import re
import subprocess
import sys

# client/packages/app/lib/src/widgets/module_scene_painter.dart, resolveSceneColor.
COLOURS = {
    "bg", "surface", "sunken", "accent", "accent-soft",
    "muted", "text", "border", "danger",
}

# client/packages/app/lib/src/widgets/module_scene_parse.dart, _parseOp. The value is
# the set of keys that op actually reads; anything else is ignored, and a key
# the op needs but does not get makes the whole op vanish.
OPS = {
    "cells": {"cols", "rows", "data", "palette", "gap", "tap", "alt", "tap_batch", "x", "y", "w", "h"},
    "rect": {"x", "y", "w", "h", "fill", "grad", "stroke", "sw", "r", "tap", "alt", "sweep"},
    "circle": {"cx", "cy", "r", "fill", "grad", "stroke", "sw", "tap", "sweep"},
    "line": {"x1", "y1", "x2", "y2", "stroke", "sw", "sweep"},
    "text": {"x", "y", "s", "fill", "size", "align", "sweep"},
    "path": {"d", "fill", "stroke", "sw", "tap"},
    "input": {"submit", "x", "y", "w", "value", "placeholder", "max"},
    "image": {"x", "y", "w", "h", "b64", "tap"},
    "notes": {"notes"},
}

# An op that is dropped entirely when this key is missing.
REQUIRED = {"cells": "data", "text": "s", "notes": "notes", "path": "d", "input": "submit", "image": "b64"}

# Keys that name a colour, so a typo falls back rather than failing.
COLOUR_KEYS = {"fill", "stroke"}

# module_scene_painter.dart, _parseHex: only 3, 6 or 8 hex digits are a colour.
HEX_COLOUR = re.compile(r"^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$")

# module_scene_path.dart: the step and string ceilings, and the numbers each command takes.
MAX_PATH_STEPS = 512
MAX_PATH_CHARS = 32768
PATH_ARGS = {"M": 2, "L": 2, "H": 1, "V": 1, "C": 6, "Q": 4, "Z": 0}
PATH_TOKEN = re.compile(r"[A-Za-z]|[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?")

# module_scene_sweep.dart, SceneSweep.maxPerScene: later animated ops draw still.
MAX_SWEEPS = 8


def is_colour(value):
    return value in COLOURS or bool(HEX_COLOUR.match(value))


def path_steps(d):
    """How many steps the client would parse out of a path's `d` string."""
    steps = 0
    command = None
    pending = 0
    for token in PATH_TOKEN.findall(d):
        if token.isalpha():
            command = token.upper()
            if command not in PATH_ARGS:
                return steps
            pending = PATH_ARGS[command]
            steps += 1 if command == "Z" else 0
            continue
        if command is None or PATH_ARGS[command] == 0:
            return steps
        pending -= 1
        if pending == 0:
            steps += 1
            pending = PATH_ARGS[command]
    return steps


def manifest_limits(wasm):
    """The manifest's runtime.limits for modules/<id>/<version>/module.wasm, or {}."""
    try:
        manifest = json.loads((pathlib.Path(wasm).parent.parent / "manifest.json").read_text())
        return manifest["runtime"]["limits"]
    except (OSError, KeyError, ValueError):
        return {}


def run_module(runner, wasm, command, payload, caller=None):
    limits = manifest_limits(wasm)
    argv = [runner]
    if caller is not None:
        argv += ["--caller", caller]
    if "fuel" in limits:
        argv += ["--fuel", str(limits["fuel"])]
    # The manifest's wall limit with slack for a cold CI runner, so a hang fails rather than stalls.
    timeout = limits.get("wall_ms", 1000) / 1000 * 10 + 10
    try:
        out = subprocess.run(argv + [wasm, command, payload], capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        raise SystemExit(f"module run did not finish in {timeout:g}s")
    if out.returncode != 0:
        raise SystemExit(f"module run failed: {out.stderr.strip()}")
    answer = json.loads(out.stdout)
    if not answer.get("ok"):
        raise SystemExit(f"module answered an error: {answer.get('error')}")
    return answer["output"]


def check_scene(raw):
    """Every problem with one emitted scene, as a list of sentences."""
    problems = []
    try:
        scene = json.loads(raw)
    except json.JSONDecodeError:
        return ["output is not JSON, so it renders as plain text"]
    if not isinstance(scene, dict) or scene.get("$slim") != "scene/1":
        return ["output is not tagged scene/1, so it renders as plain text"]

    background = scene.get("background")
    if isinstance(background, str) and background and not is_colour(background):
        problems.append(f"background: unknown colour {background!r}")
    elif background is not None and not isinstance(background, str):
        problems.append("background: not a string, so it is ignored")

    ops = scene.get("ops", [])
    if not isinstance(ops, list):
        return problems + ["ops: not a list, so nothing draws"]
    sweeping = 0
    for index, op in enumerate(ops):
        if not isinstance(op, dict):
            problems.append(f"ops[{index}]: not an object, the renderer skips it")
            continue
        kind = op.get("op")
        where = f"ops[{index}] ({kind})"
        if kind not in OPS:
            problems.append(f"{where}: unknown op, the renderer drops it")
            continue
        need = REQUIRED.get(kind)
        if need and need not in op:
            problems.append(
                f"{where}: no {need!r} key, so the renderer drops this op entirely"
            )
        if "sweep" in op:
            sweeping += 1
            if sweeping == MAX_SWEEPS + 1:
                problems.append(f"{where}: more than {MAX_SWEEPS} sweeps in one scene, the rest draw still")
        if kind == "path" and isinstance(op.get("d"), str):
            if len(op["d"]) > MAX_PATH_CHARS:
                problems.append(f"{where}: d is over {MAX_PATH_CHARS} characters, the renderer drops the op")
            elif path_steps(op["d"]) > MAX_PATH_STEPS:
                problems.append(f"{where}: more than {MAX_PATH_STEPS} path steps, the tail is dropped")
        for key, value in op.items():
            if key == "op":
                continue
            if key not in OPS[kind]:
                problems.append(f"{where}: key {key!r} is not read by this op")
            if key in COLOUR_KEYS and isinstance(value, str) and not is_colour(value):
                problems.append(
                    f"{where}: {key} {value!r} is not a scene colour; "
                    "it silently falls back"
                )
    return problems


def main(argv):
    args = argv[1:]
    caller = None
    if args[:1] == ["--caller"] and len(args) > 1:
        caller, args = args[1], args[2:]
    if len(args) < 3:
        print(
            "usage: check-scene-ops.py [--caller ID] <runner> <module.wasm> <command> [input ...]",
            file=sys.stderr,
        )
        return 2
    runner, wasm, command = args[0], args[1], args[2]
    inputs = args[3:] or [""]

    failed = 0
    for payload in inputs:
        label = payload[:40] or "<launch>"
        problems = check_scene(run_module(runner, wasm, command, payload, caller))
        if problems:
            failed += 1
            print(f"FAIL {label}")
            for problem in problems:
                print(f"  - {problem}")
        else:
            print(f"ok   {label}")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
