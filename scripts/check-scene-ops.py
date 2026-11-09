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
# validated from a checkout of another repo would not get validated. The files
# named below are in slim-m, under client/packages/app/lib/src/features/modules/widgets/;
# re-sync this table from them when the parser changes.
import json
import math
import pathlib
import re
import subprocess
import sys

# module_scene_painter.dart, resolveSceneColor.
COLOURS = {
    "bg", "surface", "sunken", "accent", "accent-soft",
    "muted", "text", "border", "danger",
}

# module_scene_parse.dart, _parseOp. The value is
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

# An op that is dropped entirely when this key is missing or is not a string (a list, for notes).
REQUIRED = {"cells": "data", "text": "s", "notes": "notes", "path": "d", "input": "submit", "image": "b64"}
REQUIRED_TYPE = {"notes": list}

# Keys that name a colour, so a typo falls back rather than failing.
COLOUR_KEYS = {"fill", "stroke"}

# module_scene_painter.dart, _parseHex: only 3, 6 or 8 hex digits are a colour.
HEX_COLOUR = re.compile(r"^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$")

# module_scene_path.dart: the step and string ceilings, and the numbers each command takes.
MAX_PATH_STEPS = 512
MAX_PATH_CHARS = 32768
PATH_ARGS = {"M": 2, "L": 2, "H": 1, "V": 1, "C": 6, "Q": 4, "Z": 0}
_NUMBER = r"[-+]?(?:\d+\.?\d*|\.\d+)"
_EXPONENT = r"(?:[eE][-+]?\d+)?"
PATH_TOKEN = re.compile(rf"[A-Za-z]|{_NUMBER}{_EXPONENT}")

# module_scene_sweep.dart, SceneSweep.maxPerScene: later animated ops draw still.
MAX_SWEEPS = 8

# module_scene.dart, ImageOp: the base64 length ceiling, and how many images draw in one scene.
MAX_IMAGE_CHARS = 88 * 1024
MAX_IMAGES = 8

# What Dart's base64Decode accepts: both alphabets, whole groups of four, padding only at the end.
BASE64 = re.compile(r"(?:[A-Za-z0-9+/_-]{4})*(?:[A-Za-z0-9+/_-]{2}==|[A-Za-z0-9+/_-]{3}=)?")


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


def run_budget(wasm):
    """(runner flags, timeout in seconds) that hold a run to the manifest's fuel and wall limits."""
    limits = manifest_limits(wasm)
    flags = ["--fuel", str(int(limits["fuel"]))] if "fuel" in limits else []
    # The manifest's wall limit with slack for a cold CI runner, so a hang fails rather than stalls.
    return flags, limits.get("wall_ms", 1000) / 1000 * 10 + 10


def run_module(runner, wasm, command, payload, caller=None):
    argv = [runner]
    if caller is not None:
        argv += ["--caller", caller]
    fuel, timeout = run_budget(wasm)
    try:
        out = subprocess.run(argv + fuel + [wasm, command, payload], capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        raise SystemExit(f"module run did not finish in {timeout:g}s")
    if out.returncode != 0:
        raise SystemExit(f"module run failed: {out.stderr.strip()}")
    answer = json.loads(out.stdout)
    if not answer.get("ok"):
        raise SystemExit(f"module answered an error: {answer.get('error')}")
    return answer["output"]


def check_background(background):
    """The scene's background colour, which the renderer ignores when it is not one."""
    if isinstance(background, str) and background and not is_colour(background):
        return [f"background: unknown colour {background!r}"]
    if background is not None and not isinstance(background, str):
        return ["background: not a string, so it is ignored"]
    return []


def check_path(where, d):
    """A path the renderer would drop or cut short."""
    if len(d) > MAX_PATH_CHARS:
        return [f"{where}: d is over {MAX_PATH_CHARS} characters, the renderer drops the op"]
    steps = path_steps(d)
    if steps == 0:
        return [f"{where}: d has no complete step, so the renderer drops the op"]
    if steps > MAX_PATH_STEPS:
        return [f"{where}: more than {MAX_PATH_STEPS} path steps, the tail is dropped"]
    return []


def check_input(where, op):
    """An input the renderer would drop for having nothing to submit."""
    return [] if op["submit"] else [f"{where}: submit is empty, so the renderer drops this op entirely"]


def image_problems(where, op):
    """Why the renderer drops this image; empty means it decodes."""
    encoded = op["b64"]
    if not encoded:
        return [f"{where}: b64 is empty, so the renderer drops this op entirely"]
    if len(encoded) > MAX_IMAGE_CHARS:
        return [f"{where}: b64 is over {MAX_IMAGE_CHARS} characters, the renderer drops the op"]
    if not BASE64.fullmatch(encoded):
        return [f"{where}: b64 is not base64 the client can decode, so the renderer drops the op"]
    return []


def valid_note(note):
    """Whether the renderer keeps this note: a positive pitch and length, and a start at or after zero."""
    def number(key, fallback):
        value = note.get(key) if isinstance(note, dict) else None
        finite = isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)
        return value if finite else fallback

    return number("f", 0) > 0 and number("t", -1) >= 0 and number("d", 0) > 0


def check_notes(where, op):
    """Notes the renderer would drop whole because none of them is playable."""
    return [] if any(valid_note(n) for n in op["notes"]) else [f"{where}: no note has a positive f and d and a t of 0 or more, so the renderer drops this op entirely"]


def check_keys(where, kind, op):
    """Keys this op does not read, and colours that silently fall back."""
    problems = []
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


def check_required(where, kind, op):
    """The key this op cannot be drawn without, which the renderer drops the op for lacking."""
    need = REQUIRED.get(kind)
    if need is None:
        return []
    if need not in op:
        return [f"{where}: no {need!r} key, so the renderer drops this op entirely"]
    expected = REQUIRED_TYPE.get(kind, str)
    if not isinstance(op[need], expected):
        return [f"{where}: {need!r} is not a {expected.__name__}, so the renderer drops this op entirely"]
    return []


def check_content(where, kind, op):
    """What the renderer drops an op for once its required key is there."""
    if kind == "path":
        return check_path(where, op["d"])
    if kind == "input":
        return check_input(where, op)
    if kind == "image":
        return image_problems(where, op)
    if kind == "notes":
        return check_notes(where, op)
    return []


def check_op(index, op):
    """Problems with one op; the op itself is not checked further once it is unusable."""
    if not isinstance(op, dict):
        return [f"ops[{index}]: not an object, the renderer skips it"]
    kind = op.get("op")
    where = f"ops[{index}] ({kind})"
    if kind not in OPS:
        return [f"{where}: unknown op, the renderer drops it"]
    problems = check_required(where, kind, op)
    if not problems:
        problems = check_content(where, kind, op)
    return problems + check_keys(where, kind, op)


def check_images(ops):
    """The first decodable image past the scene's budget, which the renderer drops."""
    drawn = 0
    for index, op in enumerate(ops):
        if isinstance(op, dict) and op.get("op") == "image" and not check_required("", "image", op) and not image_problems("", op):
            drawn += 1
            if drawn == MAX_IMAGES + 1:
                return [f"ops[{index}] (image): more than {MAX_IMAGES} images in one scene, the rest are dropped"]
    return []


def check_sweeps(ops):
    """The first op past the scene's sweep budget, which draws still."""
    sweeping = 0
    for index, op in enumerate(ops):
        if isinstance(op, dict) and op.get("op") in OPS and "sweep" in op:
            sweeping += 1
            if sweeping == MAX_SWEEPS + 1:
                where = f"ops[{index}] ({op.get('op')})"
                return [f"{where}: more than {MAX_SWEEPS} sweeps in one scene, the rest draw still"]
    return []


def check_scene(raw):
    """Every problem with one emitted scene, as a list of sentences."""
    try:
        scene = json.loads(raw)
    except json.JSONDecodeError:
        return ["output is not JSON, so it renders as plain text"]
    if not isinstance(scene, dict) or scene.get("$slim") != "scene/1":
        return ["output is not tagged scene/1, so it renders as plain text"]
    problems = check_background(scene.get("background"))
    ops = scene.get("ops", [])
    if not isinstance(ops, list):
        return problems + ["ops: not a list, so nothing draws"]
    for index, op in enumerate(ops):
        problems += check_op(index, op)
    return problems + check_images(ops) + check_sweeps(ops)


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
