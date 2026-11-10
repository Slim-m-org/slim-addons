#!/usr/bin/env python3
"""Unit tests for play-modules.py: that a script's expectations are really held against the frames.

Run from the repo root: python3 -m unittest discover -s scripts -p 'test_*.py'
SCRIPTS_DIR points the tests at another copy of the scripts, for mutation checks.
"""
import contextlib
import importlib.util
import io
import json
import os
import pathlib
import tempfile
import unittest
from unittest import mock

here = pathlib.Path(os.environ.get("SCRIPTS_DIR", pathlib.Path(__file__).resolve().parent))


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, here / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


play = load("play_modules", "play-modules.py")
scene_ops = load("check_scene_ops", "check-scene-ops.py")


def scene(status="X to move", controls=("new game",), state="s0", taps=("p0",), has_input=False, live=True):
    ops = [{"op": "rect", "tap": t} for t in taps]
    if has_input:
        ops.append({"op": "input", "submit": "guess"})
    ops.append({"op": "text", "s": "Board", "x": 0, "y": 0})
    return {"ok": True, "output": json.dumps({"$slim": "scene/1", "status": status, "controls": list(controls),
                                               "state": state, "ops": ops, "live": live})}


class Counter:
    """A stand-in module: each action adds one to the state, and `bad` is refused."""

    def __init__(self, **overrides):
        self.overrides = overrides
        self.seen = []

    def __call__(self, caller, payload, now_ms):
        self.seen.append((caller, payload, now_ms))
        if payload and not payload.startswith("{"):
            return {"ok": False, "error": "not a frame"}
        request = json.loads(payload) if payload else {"action": "", "state": ""}
        if request["action"] == "bad":
            return {"ok": False, "error": "unknown action: bad"}
        count = int(request["state"] or 0) + 1
        return scene(status=f"move {count}", state=str(count), **self.overrides)


def problems(steps, runner=None):
    runner = runner or Counter()
    return play.play_scenario(runner, {"name": "t", "steps": steps}, scene_ops.check_scene)


class ExpectationsTest(unittest.TestCase):
    def test_a_frame_that_matches_every_expectation_passes(self):
        steps = [{"as": "a1", "do": "go", "expect": {"status": "move 1", "controls": ["new game"], "input": False, "taps_has": ["p0"],
                                                      "taps_lack": ["p1"], "text_has": ["Board"], "state": "1", "live": True}}]
        self.assertEqual(problems(steps), [])

    def test_each_wrong_expectation_is_reported(self):
        wrong = {"status": "nope", "status_has": "zzz", "controls": ["x"], "controls_has": ["x"], "controls_lack": ["new game"],
                 "input": True, "taps_has": ["p9"], "taps_lack": ["p0"], "text_has": ["zzz"], "text_lack": ["Board"],
                 "state": "9", "state_has": "9", "live": False}
        for key, value in wrong.items():
            found = problems([{"as": "a1", "do": "go", "expect": {key: value}}])
            self.assertTrue(found, f"expecting {key}={value!r} of a frame that does not match it still passed")

    def test_a_refusal_is_a_failure_unless_the_step_expects_one(self):
        self.assertTrue(problems([{"as": "a1", "do": "bad"}]))
        self.assertEqual(problems([{"as": "a1", "do": "bad", "expect": {"ok": False, "error_has": "unknown action"}}]), [])
        self.assertTrue(problems([{"as": "a1", "do": "bad", "expect": {"ok": False, "error_has": "something else"}}]))
        self.assertEqual(problems([{"as": "a1", "do": "bad", "expect": {"ok": None}}]), [])

    def test_a_reply_that_is_not_a_scene_fails_a_scene_expectation(self):
        runner = lambda caller, payload, now_ms: {"ok": True, "output": "plain text"}
        self.assertTrue(problems([{"as": "a1", "do": "go", "expect": {"status": "x"}}], runner))
        self.assertEqual(problems([{"as": "a1", "do": "go"}], runner), [])

    def test_every_frame_must_pass_the_scene_check(self):
        broken = lambda caller, payload, now_ms: {"ok": True, "output": json.dumps(
            {"$slim": "scene/1", "ops": [{"op": "rect", "fill": "not-a-colour"}], "state": "s"})}
        self.assertTrue(any("not a scene colour" in p for _, p in problems([{"as": "a1", "do": "go"}], broken)))


class StateTest(unittest.TestCase):
    def test_the_last_frames_state_is_sent_back_and_unchanged_is_checked(self):
        runner = Counter()
        found = problems([{"as": "a1", "do": "go"}, {"as": "a1", "do": "go", "expect": {"status": "move 2"}},
                          {"as": "a1", "do": "go", "expect": {"state_unchanged": True}}], runner)
        self.assertEqual([p for _, p in found], ["state changed from '2' to '3'"])
        self.assertEqual(json.loads(runner.seen[2][1])["state"], "2")

    def test_a_saved_state_can_be_sent_again_and_fresh_sends_none(self):
        runner = Counter()
        problems([{"as": "a1", "do": "go", "save": "one"}, {"as": "a1", "do": "go"}, {"as": "b2", "do": "go", "state": "@one"},
                  {"as": "b2", "do": "go", "fresh": True}], runner)
        states = [json.loads(p)["state"] for _, p, _ in runner.seen]
        self.assertEqual(states, ["", "1", "1", ""])

    def test_the_caller_and_the_clock_are_carried_forward(self):
        runner = Counter()
        problems([{"as": "a1", "do": "go", "now_ms": 5}, {"do": "go"}, {"as": "b2", "do": "go"}], runner)
        self.assertEqual([(c, n) for c, _, n in runner.seen], [("a1", 5), ("a1", 5), ("b2", 5)])

    def test_a_launch_step_sends_no_input_and_raw_sends_it_as_is(self):
        runner = Counter()
        problems([{"as": "a1"}, {"as": "a1", "raw": "not json", "expect": {"ok": False}}], runner)
        self.assertEqual([p for _, p, _ in runner.seen], ["", "not json"])


class RunnerFailureTest(unittest.TestCase):
    def test_a_run_that_hangs_or_dies_fails_the_scenario_and_stops_it(self):
        def dies(caller, payload, now_ms):
            raise RuntimeError("the run did not finish in 20s")

        found = problems([{"as": "a1", "do": "go"}, {"as": "a1", "do": "go"}], dies)
        self.assertEqual(len(found), 1)
        self.assertIn("did not finish", found[0][1])


class PostsTest(unittest.TestCase):
    def test_posts_and_output_are_checked_and_a_tick_step_sends_the_tick_marker(self):
        seen = []

        def runner(caller, payload, now_ms):
            seen.append((caller, payload))
            return {"ok": True, "output": "Reminded 1.", "posts": ["@[Gamers] Chess starts in 10 min"]}

        good = {"as": "scheduled", "tick": "remind", "expect": {"output_has": "Reminded 1.", "posts_has": ["Chess starts"], "posts_count": 1}}
        self.assertEqual(problems([good], runner), [])
        self.assertEqual(seen, [("scheduled", "@tick:remind")])
        for wrong in ({"output_has": "Reminded 2."}, {"posts_has": ["Tennis"]}, {"posts_count": 0}):
            self.assertTrue(problems([{"as": "scheduled", "tick": "remind", "expect": wrong}], runner), wrong)


class TypoTest(unittest.TestCase):
    def test_a_misspelt_key_is_a_failure_rather_than_a_check_that_never_ran(self):
        self.assertTrue(problems([{"as": "a1", "do": "go", "expect": {"statuss": "move 1"}}]))
        self.assertTrue(problems([{"as": "a1", "do": "go", "expct": {}}]))


class OnlyTest(unittest.TestCase):
    """--only names one module's script; one that matches nothing is a usage error, not a pass."""

    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        for module in ("alpha", "beta"):
            (pathlib.Path(tmp.name) / "modules" / module).mkdir(parents=True)
            (pathlib.Path(tmp.name) / "modules" / module / "play.json").write_text("{}")
        patch = mock.patch.object(play, "root", pathlib.Path(tmp.name))
        patch.start()
        self.addCleanup(patch.stop)

    def run_main(self, *args):
        played = []
        err = io.StringIO()
        with mock.patch.object(play, "play_module", lambda runner, manifest, ops, show: played.append(manifest.parent.name) or []), \
                mock.patch.object(play, "load_scene_ops", lambda: scene_ops):
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(err):
                code = play.main(["play-modules.py", "runner", *args])
        return code, played, err.getvalue()

    def test_a_module_with_no_script_is_refused_and_lists_the_ones_that_exist(self):
        code, played, err = self.run_main("--only", "alph")
        self.assertEqual((code, played), (2, []))
        self.assertIn("no play.json for module 'alph'", err)
        self.assertIn("alpha, beta", err)

    def test_only_with_no_value_is_a_usage_error(self):
        for args in (("--only",), ("--only", "--show")):
            code, played, err = self.run_main(*args)
            self.assertEqual((code, played), (2, []), args)
            self.assertIn("Usage:", err)

    def test_only_plays_just_the_named_module_and_none_plays_them_all(self):
        self.assertEqual(self.run_main("--only", "beta")[:2], (0, ["beta"]))
        self.assertEqual(self.run_main()[:2], (0, ["alpha", "beta"]))


if __name__ == "__main__":
    unittest.main()
