#!/usr/bin/env python3
"""End-to-end run of the transcript module against a real slimm-server (needs `pip install websockets`).

Usage: flock ~/.slimm-scratch/tmp/e2e.lock python3 scripts/e2e-transcript.py --server target/debug/slimm-server

It starts the server on a free port in a private temp directory with this repository as the Dock's dev
origin, and a stand-in for the call-audio listener that speaks `listener/1` over a real WebSocket. It
installs the module as the admin, puts two members in a call through signed LiveKit webhooks, has both
agree, plays scripted speech from the listener, and reads the transcript the module posts. No audio and
no whisper model are involved: the listener's job starts after speech has become text.
"""
import argparse
import asyncio
import importlib.util
import json
import os
import shutil
import sys
import tempfile
import threading
import time
from pathlib import Path

import websockets

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("e2e_xp", HERE / "e2e-xp.py")
xp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(xp)

SECRET = "e2e-listener-secret-of-at-least-32-characters"
CAPABILITIES = ["call.audio", "data.store", "message.post", "schedule"]


class FakeListener:
    """The server's other end of `listener/1`: records every frame and sends the ones a test scripts."""

    def __init__(self, port):
        self.port, self.received, self.socket, self.loop = port, [], None, None
        self.ready = threading.Event()
        threading.Thread(target=self._serve, daemon=True).start()
        self.ready.wait(10)

    def _serve(self):
        asyncio.run(self._main())

    async def _main(self):
        self.loop = asyncio.get_running_loop()

        def guard(connection, request):
            if request.headers.get("Authorization") != f"Bearer {SECRET}":
                return connection.respond(401, "no\n")
            return None

        async def handler(socket):
            self.socket = socket
            await socket.send(json.dumps({"v": 1, "type": "ready", "version": "e2e", "engine": "fake", "model": "fake.bin", "max_sessions": 2}))
            try:
                async for message in socket:
                    self.received.append(json.loads(message))
            except websockets.ConnectionClosed:
                pass

        async with websockets.serve(handler, "127.0.0.1", self.port, process_request=guard):
            self.ready.set()
            await asyncio.Future()

    def say(self, frame):
        future = asyncio.run_coroutine_threadsafe(self.socket.send(json.dumps({"v": 1, **frame})), self.loop)
        future.result(10)

    def frames(self, kind):
        return [f for f in self.received if f.get("type") == kind]


def setup(server):
    admin = xp.ok(server, "POST", "/auth/register", {"username": "alice", "display_name": "Alice", "password": "pw-" + xp.uuid7(), "device_name": "e2e"})
    token = admin["access_token"]
    people = {"alice": (admin["user_id"], token)}
    invite = xp.ok(server, "POST", "/invites", {}, token)["code"]
    bob = xp.ok(server, "POST", "/auth/register", {"username": "bob", "display_name": "Bob", "password": "pw-" + xp.uuid7(), "device_name": "e2e", "invite_code": invite})
    people["bob"] = (bob["user_id"], bob["access_token"])
    room = xp.ok(server, "POST", "/channels", {"name": "room", "kind": "voice"}, token)["id"]
    chat = xp.ok(server, "POST", "/channels", {"name": "notes", "kind": "text"}, token)["id"]
    return token, people, room, chat


def install(server, token, room, chat):
    review = xp.ok(server, "GET", "/space/dock/modules/transcript?source=dev", token=token)
    body = {"version": review["version"], "approved_host_capabilities": CAPABILITIES, "reviewed_digest": review.get("review_digest")}
    xp.ok(server, "POST", "/space/dock/modules/transcript/install?source=dev", body, token)
    xp.ok(server, "PUT", "/space/dock/modules/transcript/schedule-channel", {"channel_id": chat}, token)
    xp.ok(server, "PUT", "/space/dock/modules/transcript/call-audio-channels", {"channel_ids": [room]}, token)
    xp.ok(server, "POST", "/space/dock/modules/transcript/enable", None, token)
    everyone = next(r["id"] for r in xp.ok(server, "GET", "/roles", token=token) if r.get("is_everyone"))
    xp.ok(server, "PUT", f"/roles/{everyone}/module-permissions/transcript/post", None, token)


def messages(server, token, channel):
    return xp.ok(server, "GET", f"/channels/{channel}/messages", token=token)


def posted(server, token, chat, prefix):
    return [m for m in messages(server, token, chat) if str(m.get("content", "")).startswith(prefix)]


def run(binary, keep):
    workdir = Path(tempfile.mkdtemp(prefix="transcript-e2e-", dir=os.environ.get("TMPDIR")))
    server, checks = xp.Server(binary, workdir), xp.Checks()
    listener = FakeListener(xp.free_port())
    original = xp.Server.start

    def start(self):
        os.environ["SLIMM_LISTENER_URL"] = f"ws://127.0.0.1:{listener.port}"
        os.environ["SLIMM_LISTENER_SECRET"] = SECRET
        original(self)

    xp.Server.start = start
    try:
        server.start()
        token, people, room, chat = setup(server)
        install(server, token, room, chat)
        alice, bob = people["alice"], people["bob"]
        sids = {n: "PA_" + xp.uuid7()[-12:] for n in people}
        for n in people:
            xp.webhook(server, "participant_joined", room, (people[n][0], sids[n]))

        print("alice asks, bob agrees")
        xp.ok(server, "POST", f"/channels/{room}/voice/call-audio", {"module_id": "transcript"}, alice[1])
        xp.ok(server, "PUT", f"/channels/{room}/voice/call-audio/consent", {"allow": True}, bob[1])
        xp.wait_for("the listener to be told to start", lambda: listener.frames("start"))
        start_frame = listener.frames("start")[0]
        session = start_frame["session"]
        checks.expect("the listener is to hear exactly the two who agreed", sorted(start_frame["include"]), sorted([alice[0], bob[0]]))
        listener.say({"type": "started", "session": session})

        print("speech, with a person who never agreed")
        script = [(alice[0], 1000, 3500, "Okay, so the plan for Friday is the release."), (alice[0], 4000, 6000, "Anyone blocked on review?"),
                  (bob[0], 8000, 10500, "No, I merged mine this morning."), ("not-in-the-call", 11000, 12000, "I never agreed to this.")]
        for who, start_ms, end_ms, text in script:
            listener.say({"type": "segment", "session": session, "identity": who, "start_ms": start_ms, "end_ms": end_ms, "text": text, "language": "en"})
        time.sleep(3)

        print("the command posts the call so far")
        reply = xp.ok(server, "POST", "/modules/transcript/commands/transcript", {"input": "", "channel_id": chat, "voice_channel_id": room}, alice[1])
        checks.expect("the command answers ok", reply.get("ok"), True)
        so_far = xp.wait_for("the so-far message", lambda: posted(server, token, chat, "**Call transcript so far**"))
        print("---\n" + so_far[0]["content"] + "\n---")
        checks.expect("so far holds alice's merged paragraph", "Alice: Okay, so the plan for Friday is the release. Anyone blocked on review?" in so_far[0]["content"], True)

        print("alice stops the call")
        xp.ok(server, "DELETE", f"/channels/{room}/voice/call-audio", token=alice[1])
        final = xp.wait_for("the final transcript", lambda: posted(server, token, chat, "**Call transcript** -"), timeout=60)
        text = final[0]["content"]
        print("---\n" + text + "\n---")
        checks.expect("it is posted once", len(final), 1)
        checks.expect("it holds both speakers by name", "Alice:" in text and "Bob: No, I merged mine this morning." in text, True)
        checks.expect("the person outside the agreed set is not in it", "never agreed" in text, False)
        checks.expect("it ends the transcript", text.rstrip().endswith("_End of transcript._"), True)
        time.sleep(7)
        checks.expect("the timer does not post it again", len(posted(server, token, chat, "**Call transcript** -")), 1)
    finally:
        server.stop()
        if keep:
            print("kept", workdir)
        else:
            shutil.rmtree(workdir, ignore_errors=True)
    print("FAILED" if xp.Checks.failed else "all checks passed")
    return 1 if xp.Checks.failed else 0


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--server", required=True, help="path to a slimm-server binary")
    parser.add_argument("--keep", action="store_true", help="keep the temp directory (database, module file, log)")
    args = parser.parse_args()
    sys.exit(run(args.server, args.keep))
