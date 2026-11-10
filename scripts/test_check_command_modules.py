#!/usr/bin/env python3
"""Unit tests for check-command-modules.py: that a sample run is held to its limits and its expectations, against a fake runner.

Run from the repo root: python3 -m unittest discover -s scripts -p 'test_*.py'
SCRIPTS_DIR points the tests at another copy of the scripts, for mutation checks.
"""
import importlib.util
import json
import os
import pathlib
import stat
import tempfile
import unittest
from unittest import mock

here = pathlib.Path(os.environ.get("SCRIPTS_DIR", pathlib.Path(__file__).resolve().parent))
spec = importlib.util.spec_from_file_location("check_command_modules", here / "check-command-modules.py")
commands = importlib.util.module_from_spec(spec)
spec.loader.exec_module(commands)

# A runner that appends its argv to $FAKE_LOG, then behaves as the command (the second to last argument) says.
FAKE_RUNNER = """#!/usr/bin/env python3
import json, os, sys, time
argv = sys.argv[1:]
with open(os.environ["FAKE_LOG"], "a") as log:
    log.write(json.dumps(argv) + "\\n")
command, text = argv[-2], argv[-1]
if command == "hang":
    time.sleep(30)
if command == "crash":
    print("it fell over", file=sys.stderr)
    sys.exit(1)
if command == "refuse":
    print(json.dumps({"ok": False, "error": "no"}))
    sys.exit(0)
if command == "post":
    print("fuel used: 3", file=sys.stderr)
    print("message.post #1: Added tag rules.", file=sys.stderr)
    print(json.dumps({"ok": True, "output": ""}))
    sys.exit(0)
print(json.dumps({"ok": True, "output": "echo " + text}))
"""

MODULE = {"id": "fake"}
BUDGET = (["--fuel", "1234"], 5)


class SampleTest(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.runner = pathlib.Path(tmp.name) / "runner"
        self.runner.write_text(FAKE_RUNNER)
        self.runner.chmod(self.runner.stat().st_mode | stat.S_IEXEC)
        self.log = pathlib.Path(tmp.name) / "log"
        patch = mock.patch.dict(os.environ, {"FAKE_LOG": str(self.log)})
        patch.start()
        self.addCleanup(patch.stop)

    def sample(self, command, text, expected, setup=(), posts=None, budget=BUDGET):
        key = ("fake", command)
        extra = {"SAMPLES": {key: (text, expected)}, "SETUP": {key: list(setup)}, "POSTS": {key: posts} if posts else {}}
        with mock.patch.multiple(commands, **extra):
            return commands.check_sample(str(self.runner), MODULE, command, "m/0.1.0/module.wasm", budget)

    def runs(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def test_a_matching_run_passes_and_is_held_to_the_manifests_fuel(self):
        self.assertEqual(self.sample("say", "hi", "echo hi"), [])
        self.assertEqual(self.runs()[0][:2], ["--fuel", "1234"])

    def test_output_that_does_not_start_with_the_expectation_is_reported(self):
        problems = self.sample("say", "hi", "echo bye")
        self.assertEqual(len(problems), 1)
        self.assertIn("expected output starting 'echo bye'", problems[0])

    def test_an_empty_expectation_means_the_output_is_empty(self):
        self.assertEqual(self.sample("post", "", ""), [])
        self.assertEqual(len(self.sample("say", "", "")), 1)

    def test_the_posted_message_is_checked(self):
        self.assertEqual(self.sample("post", "", "", posts="Added tag rules."), [])
        problems = self.sample("post", "", "", posts="Removed tag rules.")
        self.assertEqual(len(problems), 1)
        self.assertIn("expected a message.post of 'Removed tag rules.'", problems[0])

    def test_a_run_that_does_not_finish_is_reported_instead_of_stalling(self):
        problems = self.sample("hang", "", "", budget=([], 0.3))
        self.assertEqual(len(problems), 1)
        self.assertIn("did not finish in 0.3s", problems[0])

    def test_a_setup_step_that_fails_stops_the_sample_and_says_so(self):
        for step in ("crash", "refuse"):
            self.log.write_text("")
            problems = self.sample("say", "hi", "echo hi", setup=[(step, "")])
            self.assertEqual(len(problems), 1, step)
            self.assertIn(f"setup {step!r}", problems[0])
            self.assertEqual(len(self.runs()), 1, "the command itself must not run after a failed setup")

    def test_setup_steps_run_first_with_the_same_limits(self):
        self.assertEqual(self.sample("say", "hi", "echo hi", setup=[("say", "first")]), [])
        runs = self.runs()
        self.assertEqual([r[-2:] for r in runs], [["say", "first"], ["say", "hi"]])
        self.assertTrue(all(r[:2] == ["--fuel", "1234"] for r in runs))

    def test_a_refused_or_crashed_command_is_reported(self):
        for command in ("crash", "refuse"):
            self.assertEqual(len(self.sample(command, "", "")), 1, command)


class RealTableTest(unittest.TestCase):
    def test_every_post_a_sample_expects_belongs_to_a_sample(self):
        self.assertLessEqual(commands.POSTS.keys(), commands.SAMPLES.keys())

    def test_the_tags_commands_expect_a_post_not_just_ok(self):
        tags = {key for key in commands.SAMPLES if key[0] == "tags" and commands.SAMPLES[key][1] == ""}
        self.assertEqual(tags, set(commands.POSTS))


if __name__ == "__main__":
    unittest.main()
