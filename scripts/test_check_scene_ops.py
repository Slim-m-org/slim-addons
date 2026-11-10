#!/usr/bin/env python3
"""Unit tests for check-scene-ops.py: the contract table it keeps for the renderer.

Run from the repo root: python3 -m unittest discover -s scripts -p 'test_*.py'
SCRIPTS_DIR points the tests at another copy of the scripts, for mutation checks.
"""
import importlib.util
import json
import os
import pathlib
import tempfile
import unittest

here = pathlib.Path(os.environ.get("SCRIPTS_DIR", pathlib.Path(__file__).resolve().parent))
spec = importlib.util.spec_from_file_location("check_scene_ops", here / "check-scene-ops.py")
scene_ops = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scene_ops)


def scene(*ops, **extra):
    return json.dumps({"$slim": "scene/1", "ops": list(ops), **extra})


class CheckSceneTest(unittest.TestCase):
    def test_a_good_scene_has_no_problems(self):
        good = scene({"op": "rect", "x": 0, "y": 0, "w": 5, "h": 5, "fill": "#abc"}, background="surface")
        self.assertEqual(scene_ops.check_scene(good), [])

    def test_sweep_is_read_by_the_four_ops_that_move(self):
        sweep = {"secs": 1, "dx": 5}
        ops = [
            {"op": "rect", "x": 0, "y": 0, "w": 1, "h": 1, "sweep": sweep},
            {"op": "circle", "cx": 0, "cy": 0, "r": 1, "sweep": sweep},
            {"op": "line", "x1": 0, "y1": 0, "x2": 1, "y2": 1, "sweep": sweep},
            {"op": "text", "x": 0, "y": 0, "s": "a", "sweep": sweep},
        ]
        self.assertEqual(scene_ops.check_scene(scene(*ops)), [])

    def test_a_ninth_sweep_is_reported(self):
        ops = [{"op": "rect", "x": i, "y": 0, "w": 1, "h": 1, "sweep": {"secs": 1}} for i in range(9)]
        problems = scene_ops.check_scene(scene(*ops))
        self.assertEqual(len(problems), 1)
        self.assertIn("more than 8 sweeps", problems[0])

    def test_a_malformed_hex_colour_is_reported(self):
        for bad in ("#12", "#zzzzzz", "#12345", "#"):
            op = {"op": "rect", "x": 0, "y": 0, "w": 1, "h": 1, "fill": bad}
            self.assertEqual(len(scene_ops.check_scene(scene(op))), 1, bad)
        for good in ("#123", "#112233", "#11223344"):
            op = {"op": "rect", "x": 0, "y": 0, "w": 1, "h": 1, "fill": good}
            self.assertEqual(scene_ops.check_scene(scene(op)), [], good)

    def test_a_bad_background_is_a_message_not_a_crash(self):
        self.assertEqual(len(scene_ops.check_scene(scene(background=7))), 1)
        self.assertEqual(len(scene_ops.check_scene(scene(background="#12"))), 1)

    def test_a_non_object_op_or_scene_is_a_message_not_a_crash(self):
        self.assertEqual(len(scene_ops.check_scene(scene(5))), 1)
        self.assertEqual(len(scene_ops.check_scene("[1, 2]")), 1)

    def test_a_path_over_the_step_cap_is_reported(self):
        long = "M0 0 " + "L1 1 " * 600
        problems = scene_ops.check_scene(scene({"op": "path", "d": long}))
        self.assertEqual(len(problems), 1)
        self.assertIn("512 path steps", problems[0])

    def test_a_path_at_the_cap_passes(self):
        exact = "M0 0 " + "L1 1 " * 510 + "Z"
        self.assertEqual(scene_ops.check_scene(scene({"op": "path", "d": exact})), [])

    def test_implicit_repeats_count_as_steps(self):
        self.assertEqual(scene_ops.path_steps("M0 0 1 1 2 2 3 3"), 4)
        self.assertEqual(scene_ops.path_steps("M0 0 H5 V5 Z"), 4)


def only_op(op):
    return scene_ops.check_scene(scene(op))


class DroppedOpsTest(unittest.TestCase):
    """What slim-m's module_scene_parse.dart drops without a word, so a module emitting it draws nothing."""

    def test_an_input_with_nothing_to_submit_is_reported(self):
        self.assertEqual(only_op({"op": "input", "submit": "guess", "x": 0, "y": 0, "w": 50}), [])
        for bad in ("", 5, None):
            problems = only_op({"op": "input", "submit": bad})
            self.assertEqual(len(problems), 1, repr(bad))
        self.assertIn("submit is empty", only_op({"op": "input", "submit": ""})[0])

    def test_a_path_with_no_complete_step_is_reported(self):
        for good in ("M0 0", "M0 0 L1 1", "Z", "L1 1"):
            self.assertEqual(only_op({"op": "path", "d": good}), [], good)
        for bad in ("", "M", "M0", "X1 2", "   ", "1 2 3"):
            problems = only_op({"op": "path", "d": bad})
            self.assertEqual(len(problems), 1, repr(bad))
            self.assertIn("no complete step", problems[0])

    def test_an_image_the_client_cannot_decode_is_reported(self):
        for good in ("YQ==", "YWJj", "iVBORw0KGgo=", "Pz8_Pz8-"):
            self.assertEqual(only_op({"op": "image", "b64": good}), [], good)
        for bad in ("", "YQ", "YWJj\nZGVm", "YWJj ZGVm", "YW!j", "YQ==YQ==", "===="):
            self.assertEqual(len(only_op({"op": "image", "b64": bad})), 1, repr(bad))

    def test_an_image_over_the_length_ceiling_is_reported(self):
        at_cap = "A" * scene_ops.MAX_IMAGE_CHARS
        self.assertEqual(only_op({"op": "image", "b64": at_cap}), [])
        problems = only_op({"op": "image", "b64": at_cap + "AAAA"})
        self.assertEqual(len(problems), 1)
        self.assertIn("over 90112 characters", problems[0])

    def test_a_ninth_decodable_image_is_reported_and_undecodable_ones_do_not_count(self):
        good = {"op": "image", "b64": "YQ=="}
        broken = {"op": "image", "b64": "YQ"}
        eight = scene_ops.check_scene(scene(*[good] * 8, *[broken] * 3))
        self.assertEqual(len(eight), 3)
        self.assertTrue(all("b64 is not base64" in p for p in eight))
        problems = scene_ops.check_scene(scene(*[good] * 9))
        self.assertEqual(len(problems), 1)
        self.assertIn("more than 8 images", problems[0])
        self.assertIn("ops[8]", problems[0])

    def test_a_required_key_of_the_wrong_type_is_reported(self):
        cases = [{"op": "cells", "data": 5}, {"op": "text", "s": 7}, {"op": "path", "d": ["M0 0"]},
                 {"op": "input", "submit": 1}, {"op": "image", "b64": 1}, {"op": "notes", "notes": "c4"}]
        for op in cases:
            problems = only_op(op)
            self.assertEqual(len(problems), 1, op)
            self.assertIn("is not a", problems[0])
        self.assertIn("no 'data' key", only_op({"op": "cells"})[0])

    def test_notes_with_nothing_playable_are_reported(self):
        playable = {"f": 440, "t": 0, "d": 0.5}
        self.assertEqual(only_op({"op": "notes", "notes": [playable]}), [])
        self.assertEqual(only_op({"op": "notes", "notes": [{"f": 0, "t": 0, "d": 1}, playable]}), [])
        for bad in ([], [{"f": 0, "t": 0, "d": 1}], [{"f": 440, "t": -1, "d": 1}], [{"f": 440, "t": 0, "d": 0}],
                    [{"f": True, "t": 0, "d": 1}], ["c4"], [{"t": 0, "d": 1}]):
            self.assertEqual(len(only_op({"op": "notes", "notes": bad})), 1, bad)


class RunBudgetTest(unittest.TestCase):
    def test_the_manifests_fuel_and_wall_limits_set_the_flags_and_timeout(self):
        with tempfile.TemporaryDirectory() as tmp:
            version = pathlib.Path(tmp) / "m" / "0.1.0"
            version.mkdir(parents=True)
            (version.parent / "manifest.json").write_text(json.dumps({"runtime": {"limits": {"fuel": 5000, "wall_ms": 2000}}}))
            self.assertEqual(scene_ops.run_budget(str(version / "module.wasm")), (["--fuel", "5000"], 30))
            self.assertEqual(scene_ops.run_budget(str(pathlib.Path(tmp) / "none" / "0.1.0" / "module.wasm")), ([], 20))


if __name__ == "__main__":
    unittest.main()
