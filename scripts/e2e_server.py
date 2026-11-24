"""The slimm-server process the e2e scripts drive, shared so each resolves --server the same way."""
import os
import signal
import socket
import subprocess
import time
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Server:
    def __init__(self, binary, workdir, livekit_key="APIxptestkey", livekit_secret="an-xp-test-secret-of-at-least-32-chars"):
        # The server runs with cwd=workdir, where a relative path to the binary would not resolve.
        self.binary, self.workdir, self.port, self.proc = os.path.abspath(binary), workdir, free_port(), None
        self.livekit = (livekit_key, livekit_secret)
        self.base = f"http://127.0.0.1:{self.port}"

    def start(self):
        env = {
            **os.environ,
            "SLIMM_PORT": str(self.port),
            "SLIMM_DATABASE_PATH": str(self.workdir / "slimm.db"),
            "SLIMM_ATTACHMENTS_DIR": str(self.workdir / "media"),
            "SLIMM_MODULES_DIR": str(self.workdir / "modules"),
            "SLIMM_DEV_MODULE_DIR": str(REPO),
            "SLIMM_LIVEKIT_URL": "ws://127.0.0.1:1",
            "SLIMM_LIVEKIT_API_KEY": self.livekit[0],
            "SLIMM_LIVEKIT_API_SECRET": self.livekit[1],
            "SLIMM_LOG": "info",
        }
        log = open(self.workdir / "server.log", "ab")
        self.proc = subprocess.Popen([self.binary], env=env, stdout=log, stderr=log, cwd=self.workdir)
        for _ in range(100):
            try:
                urllib.request.urlopen(f"{self.base}/healthz", timeout=1)
                return
            except OSError:
                time.sleep(0.2)
        raise SystemExit("the server did not come up; see server.log in " + str(self.workdir))

    def stop(self):
        if self.proc and self.proc.poll() is None:
            self.proc.send_signal(signal.SIGTERM)
            try:
                self.proc.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.proc.kill()
