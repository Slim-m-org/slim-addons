#!/usr/bin/env python3
"""End-to-end run of the xp module against a real slimm-server (stdlib only).

Usage: flock ~/.slimm-scratch/tmp/e2e.lock python3 scripts/e2e-xp.py --server target/debug/slimm-server

It starts the server on a free port in a private temp directory with this repo as the Dock's
dev origin, installs xp as the admin, drives voice through signed LiveKit webhooks, and checks
the profile and badge slots. It restarts the server midway to prove nothing is counted twice.
Needs about three minutes: a voice minute is a real minute, because the server stamps sessions
with its own clock.
"""
import argparse
import base64
import hashlib
import hmac
import json
import os
import shutil
import signal
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
API_KEY, API_SECRET = "APIxptestkey", "an-xp-test-secret-of-at-least-32-chars"
MINUTE_S = 62
SETTINGS = {"xp_per_minute": 100, "xp_per_message": 3, "message_cooldown_s": 60}
CAPABILITIES = ["events", "data.store", "log.read", "schedule"]


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def uuid7():
    ms = int(time.time() * 1000)
    raw = bytearray(ms.to_bytes(6, "big") + os.urandom(10))
    raw[6] = 0x70 | (raw[6] & 0x0F)
    raw[8] = 0x80 | (raw[8] & 0x3F)
    return str(uuid.UUID(bytes=bytes(raw)))


class Server:
    def __init__(self, binary, workdir):
        self.binary, self.workdir, self.port, self.proc = binary, workdir, free_port(), None
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
            "SLIMM_LIVEKIT_API_KEY": API_KEY,
            "SLIMM_LIVEKIT_API_SECRET": API_SECRET,
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


def call(server, method, path, body=None, token=None, raw=None, headers=None):
    data = raw if raw is not None else (json.dumps(body).encode() if body is not None else None)
    h = {"Content-Type": "application/json", **(headers or {})}
    if token:
        h["Authorization"] = f"Bearer {token}"
    req = urllib.request.Request(server.base + path, data=data, headers=h, method=method)
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            text = r.read()
            return r.status, (json.loads(text) if text else None)
    except urllib.error.HTTPError as e:
        text = e.read()
        try:
            return e.code, json.loads(text)
        except ValueError:
            return e.code, text.decode(errors="replace")


def ok(server, method, path, body=None, token=None):
    status, reply = call(server, method, path, body, token)
    if status >= 300:
        raise SystemExit(f"{method} {path} -> {status}: {reply}")
    return reply


def webhook(server, event, room, who=None):
    body = {"event": event, "room": {"name": f"channel-{room}"}}
    if who:
        body["participant"] = {"identity": who[0], "sid": who[1]}
    raw = json.dumps(body).encode()
    b64 = lambda b: base64.urlsafe_b64encode(b).rstrip(b"=")
    claims = {"iss": API_KEY, "exp": int(time.time()) + 300, "sha256": base64.b64encode(hashlib.sha256(raw).digest()).decode()}
    signing = b64(b'{"alg":"HS256","typ":"JWT"}') + b"." + b64(json.dumps(claims).encode())
    token = signing + b"." + b64(hmac.new(API_SECRET.encode(), signing, hashlib.sha256).digest())
    status, _ = call(server, "POST", "/voice/webhook", raw=raw, headers={"Authorization": token.decode(), "Content-Type": "application/webhook+json"})
    if status != 204:
        raise SystemExit(f"webhook {event} -> {status}")


def module_rows(workdir, sql):
    path = workdir / "modules" / "xp.db"
    if not path.exists():
        return []
    try:
        db = sqlite3.connect(f"file:{path}?mode=ro", uri=True, timeout=5)
        try:
            return db.execute(sql).fetchall()
        finally:
            db.close()
    except sqlite3.Error:
        return []


def wait_for(what, check, timeout=40):
    end = time.time() + timeout
    while time.time() < end:
        value = check()
        if value:
            return value
        time.sleep(0.5)
    raise SystemExit(f"timed out waiting for {what}")


def totals(workdir):
    return dict(module_rows(workdir, "SELECT user_id, xp FROM xp"))


def cursor(workdir):
    rows = module_rows(workdir, "SELECT val FROM meta WHERE name='voice_cursor'")
    return rows[0][0] if rows else 0


class Checks:
    failed = 0

    def expect(self, label, got, want):
        good = got == want
        print(f"{'ok  ' if good else 'FAIL'} {label}" + ("" if good else f": got {got!r}, want {want!r}"))
        Checks.failed += not good


def setup(server, workdir):
    admin = ok(server, "POST", "/auth/register", {"username": "alice", "display_name": "Alice", "password": "pw-" + uuid7(), "device_name": "e2e"})
    token = admin["access_token"]
    people = {"alice": (admin["user_id"], token)}
    for name in ("bob", "cleo"):
        invite = ok(server, "POST", "/invites", {}, token)["code"]
        reply = ok(server, "POST", "/auth/register", {"username": name, "display_name": name.title(), "password": "pw-" + uuid7(), "device_name": "e2e", "invite_code": invite})
        people[name] = (reply["user_id"], reply["access_token"])
    channels = {n: ok(server, "POST", "/channels", {"name": n, "kind": k}, token)["id"] for n, k in (("room-one", "voice"), ("room-two", "voice"), ("chat", "text"))}
    return token, people, channels


def install(server, token, chat):
    listing = ok(server, "GET", "/space/dock/modules?source=dev", token=token)
    assert any(m["id"] == "xp" for m in listing), listing
    review = ok(server, "GET", "/space/dock/modules/xp?source=dev", token=token)
    ok(server, "POST", "/space/dock/modules/xp/install?source=dev", {"version": review["version"], "approved_host_capabilities": CAPABILITIES, "reviewed_digest": review.get("review_digest")}, token)
    ok(server, "PUT", "/space/dock/modules/xp/schedule-channel", {"channel_id": chat}, token)
    ok(server, "PUT", "/space/dock/modules/xp/settings", {"values": SETTINGS}, token)
    ok(server, "POST", "/space/dock/modules/xp/enable", None, token)


def call_session(server, people, plan, seconds):
    """Joins every (person, channel) pair, stays `seconds`, then leaves them all."""
    sids = {}
    for who, room in plan:
        sids[who] = "PA_" + uuid7()[-12:]
        webhook(server, "participant_joined", room, (people[who][0], sids[who]))
    time.sleep(seconds)
    for who, room in plan:
        webhook(server, "participant_left", room, (people[who][0], sids[who]))


def slots(server, token, people, order):
    ids = ",".join(people[n][0] for n in order)
    profile = {n: ok(server, "GET", f"/space/dock/slots/profile/{people[n][0]}", token=token)["sections"] for n in order}
    badges = ok(server, "GET", f"/space/dock/slots/badges?ids={ids}", token=token)["badges"]
    return profile, badges


def progress_of(sections):
    for section in sections:
        for node in section["nodes"]:
            for child in node.get("children", [node]):
                if child["type"] == "progress":
                    return child
    return None


def badge_text(badges, user_id):
    return [n["text"] for b in badges.get(user_id, []) for n in b["nodes"]]


def run(binary, keep):
    workdir = Path(tempfile.mkdtemp(prefix="xp-e2e-", dir=os.environ.get("TMPDIR")))
    server = Server(binary, workdir)
    checks = Checks()
    try:
        server.start()
        token, people, ch = setup(server, workdir)
        install(server, token, ch["chat"])
        uid = {n: people[n][0] for n in people}

        print("phase 1: alice and bob share a call, cleo is alone, alice chats twice")
        for _ in range(2):
            ok(server, "POST", f"/channels/{ch['chat']}/messages", {"id": uuid7(), "content": "hello"}, people["alice"][1])
        wait_for("alice's message XP", lambda: totals(workdir).get(uid["alice"]) == 3)
        call_session(server, people, [("alice", ch["room-one"]), ("bob", ch["room-one"]), ("cleo", ch["room-two"])], MINUTE_S)
        wait_for("the voice cursor to reach 3", lambda: cursor(workdir) >= 3)
        got = totals(workdir)
        checks.expect("alice: one voice minute + one message in the cooldown", got.get(uid["alice"]), 103)
        checks.expect("bob: one voice minute", got.get(uid["bob"]), 100)
        checks.expect("cleo: alone, nothing", got.get(uid["cleo"], 0), 0)
        profile, badges = slots(server, token, people, ["alice", "bob", "cleo"])
        checks.expect("alice level bar", progress_of(profile["alice"]), {"type": "progress", "value": 3, "max": 200, "label": "3 / 200 XP"})
        checks.expect("bob level bar", (progress_of(profile["bob"]) or {}).get("label"), "0 / 200 XP")
        checks.expect("cleo has no section", profile["cleo"], [])
        checks.expect("alice badge", badge_text(badges, uid["alice"]), ["Lv 2"])
        checks.expect("cleo has no badge", badge_text(badges, uid["cleo"]), [])

        print("restart, then phase 2: alice and cleo share a call that ends while the module is off")
        before = cursor(workdir)
        server.stop()
        server.start()
        ok(server, "POST", "/space/dock/modules/xp/disable", None, token)
        call_session(server, people, [("alice", ch["room-one"]), ("cleo", ch["room-one"])], MINUTE_S)
        time.sleep(2)
        checks.expect("a disabled module is not told", cursor(workdir), before)
        ok(server, "POST", "/space/dock/modules/xp/enable", None, token)
        ok(server, "POST", f"/channels/{ch['chat']}/messages", {"id": uuid7(), "content": "back again"}, people["alice"][1])
        call_session(server, people, [("bob", ch["room-two"])], 1)
        wait_for("catch-up to read the missed call", lambda: cursor(workdir) >= before + 3)
        wait_for("alice's message XP after the restart", lambda: totals(workdir).get(uid["alice"]) == 206)
        got = totals(workdir)
        checks.expect("alice continues from the cursor", got.get(uid["alice"]), 206)
        checks.expect("bob is not counted twice", got.get(uid["bob"]), 100)
        checks.expect("cleo earns for the missed call", got.get(uid["cleo"]), 100)
        profile, badges = slots(server, token, people, ["alice", "cleo"])
        checks.expect("alice level bar after restart", (progress_of(profile["alice"]) or {}).get("label"), "106 / 200 XP")
        checks.expect("alice badge after restart", badge_text(badges, uid["alice"]), ["Lv 2"])
        checks.expect("cleo badge", badge_text(badges, uid["cleo"]), ["Lv 2"])
    finally:
        server.stop()
        if keep:
            print("kept", workdir)
        else:
            shutil.rmtree(workdir, ignore_errors=True)
    print("FAILED" if Checks.failed else "all checks passed")
    return 1 if Checks.failed else 0


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--server", required=True, help="path to a slimm-server binary")
    parser.add_argument("--keep", action="store_true", help="keep the temp directory (database, module file, log)")
    args = parser.parse_args()
    sys.exit(run(args.server, args.keep))
