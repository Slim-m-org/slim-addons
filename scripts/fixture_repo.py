"""A throwaway git repo for the script tests: files written under a temp dir, committed on demand."""
import os
import pathlib
import shutil
import subprocess
import tempfile


class Repo:
    def __init__(self):
        self.path = pathlib.Path(tempfile.mkdtemp())
        self.git("init", "-q", "-b", "main")

    def cleanup(self):
        shutil.rmtree(self.path, ignore_errors=True)

    def write(self, relative, content):
        target = self.path / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(content if isinstance(content, bytes) else content.encode())
        return target

    def remove(self, relative):
        (self.path / relative).unlink()

    def git(self, *args):
        # An inherited GIT_DIR would point these at the real repo when the tests run inside a hook.
        env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
        identity = ["-c", "user.name=test", "-c", "user.email=test@example.test", "-c", "commit.gpgsign=false"]
        return subprocess.run(["git", *identity, *args], cwd=self.path, env=env, check=True, capture_output=True)

    def commit(self, message="base"):
        self.git("add", ".")
        self.git("commit", "-q", "-m", message)
