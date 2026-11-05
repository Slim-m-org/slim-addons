#!/usr/bin/env python3
"""End-to-end run of the undergrowth module against a real slimm-server (stdlib only).

Usage: flock ~/.slimm-scratch/tmp/e2e.lock python3 scripts/e2e-undergrowth.py --server path/to/slimm-server

It starts the server on a free port in a private temp directory with this repo as the Dock's
dev origin, installs undergrowth as the admin with every capability it asks for, launches it
into a channel and plays it the way the client does, through the shared code-run route. It
checks concurrent presses, the host's own redraws (refresh_ms), weather from messages and
reactions, the private panel, profile linking, a season ending into a posted result, the
almanac, and a huge world, and prints the server's own fuel and timing for every command.
Needs about two minutes, most of it a short season running out in real time.
"""
import argparse
import json
import os
import shutil
import signal
import socket
import sqlite3
import statistics
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
CAPABILITIES = ["data.store", "events", "log.read", "schedule", "message.post"]
SHORT_SEASON = {"season_ticks": 200, "tick_ms": 150, "redraw_s": 2}
MODULE = "undergrowth"
# The reaction the e2e leaves, written as an escape so no literal emoji sits in the source.
MUSHROOM = "\U0001F344"


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
            "SLIMM_LIVEKIT_API_KEY": "APIugtestkey",
            "SLIMM_LIVEKIT_API_SECRET": "an-undergrowth-test-secret-of-32-chars",
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


def call(server, method, path, body=None, token=None):
    data = json.dumps(body).encode() if body is not None else None
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    req = urllib.request.Request(server.base + path, data=data, headers=headers, method=method)
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


class Checks:
    failed = 0

    def expect(self, label, good, detail=""):
        print(f"{'ok  ' if good else 'FAIL'} {label}" + (f": {detail}" if detail and not good else (f" ({detail})" if detail else "")))
        Checks.failed += not good


def wait_for(what, check, timeout=60, every=0.25):
    end = time.time() + timeout
    while time.time() < end:
        value = check()
        if value:
            return value
        time.sleep(every)
    raise SystemExit(f"timed out waiting for {what}")


class Game:
    """One launched app message, played through the shared code-run route like the client does."""

    def __init__(self, server, channel, token):
        self.server, self.channel = server, channel
        msg = ok(server, "POST", f"/channels/{channel}/messages/apps", {"id": uuid7(), "content": "", "module_id": MODULE, "command": MODULE}, token)
        self.mid = msg["id"]
        self.first = self.press(token, None)

    def press(self, token, action, timed=None):
        payload = "" if action is None else json.dumps({"action": action, "state": ""})
        started = time.time()
        status, reply = call(self.server, "POST", f"/messages/{self.mid}/blocks/0/run", {"module_id": MODULE, "command": MODULE, "input": payload}, token)
        if timed is not None:
            timed.append((time.time() - started) * 1000)
        if status != 200:
            return {"http": status, "reply": reply}
        if not reply.get("ok"):
            return {"error": reply.get("error")}
        return json.loads(reply["output"])

    def stored(self, token):
        """The frame slim stored for the message, and when it was stored."""
        for m in ok(self.server, "GET", f"/channels/{self.channel}/messages?limit=50", token=token):
            if m["id"] == self.mid:
                runs = [r for r in m.get("code_runs", []) if r["block_index"] == 0]
                if runs and runs[0]["ok"]:
                    return json.loads(runs[0]["output"]), runs[0]["ran_at"]
        return None, None


def header(scene):
    """The game header out of a scene's state."""
    return json.loads(scene["state"].rsplit("|", 1)[0])


def module_rows(workdir, sql):
    path = workdir / "modules" / f"{MODULE}.db"
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


def setup(server):
    admin = ok(server, "POST", "/auth/register", {"username": "alice", "display_name": "Alice", "password": "pw-" + uuid7(), "device_name": "e2e"})
    token = admin["access_token"]
    people = {"alice": (admin["user_id"], token)}
    for name in ("bob", "cleo"):
        invite = ok(server, "POST", "/invites", {}, token)["code"]
        reply = ok(server, "POST", "/auth/register", {"username": name, "display_name": name.title(), "password": "pw-" + uuid7(), "device_name": "e2e", "invite_code": invite})
        people[name] = (reply["user_id"], reply["access_token"])
    channels = {n: ok(server, "POST", "/channels", {"name": n, "kind": "text"}, token)["id"] for n in ("garden", "almanac")}
    return token, people, channels


def install(server, token, almanac):
    review = ok(server, "GET", f"/space/dock/modules/{MODULE}?source=dev", token=token)
    ok(server, "POST", f"/space/dock/modules/{MODULE}/install?source=dev",
       {"version": review["version"], "approved_host_capabilities": CAPABILITIES, "reviewed_digest": review.get("review_digest")}, token)
    ok(server, "PUT", f"/space/dock/modules/{MODULE}/schedule-channel", {"channel_id": almanac}, token)
    ok(server, "PUT", f"/space/dock/modules/{MODULE}/settings", {"values": SHORT_SEASON}, token)
    ok(server, "POST", f"/space/dock/modules/{MODULE}/enable", None, token)
    roles = ok(server, "GET", "/roles", token=token)
    everyone = next(r["id"] for r in roles if r["is_everyone"])
    admin = next(r["id"] for r in roles if r["name"] == "admin")
    ok(server, "PUT", f"/roles/{everyone}/module-permissions/{MODULE}/play", None, token)
    ok(server, "PUT", f"/roles/{admin}/module-permissions/{MODULE}/tend", None, token)


def parallel(jobs):
    """Runs every job at once and returns their results in order."""
    results = [None] * len(jobs)
    gate = threading.Barrier(len(jobs))

    def worker(k, job):
        gate.wait()
        results[k] = job()

    threads = [threading.Thread(target=worker, args=(k, j)) for k, j in enumerate(jobs)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    return results


def pct(values, p):
    ordered = sorted(values)
    return round(ordered[min(len(ordered) - 1, int(len(ordered) * p))]) if ordered else 0


def run(binary, keep):
    workdir = Path(tempfile.mkdtemp(prefix="undergrowth-e2e-", dir=os.environ.get("TMPDIR")))
    server = Server(binary, workdir)
    checks = Checks()
    try:
        server.start()
        token, people, ch = setup(server)
        install(server, token, ch["almanac"])
        tok = {n: people[n][1] for n in people}
        game = Game(server, ch["garden"], token)
        checks.expect("the launch frame is a scene that asks to be redrawn", game.first.get("refresh_ms") == 2000, json.dumps(game.first)[:120])

        print("phase 1: three players join at the same instant, then storm the board with lures")
        joins = parallel([lambda n=n: game.press(tok[n], "join") for n in ("alice", "bob", "cleo")])
        frame = game.press(token, "refresh")
        checks.expect("all three joins landed", len(header(frame)["c"]) == 3, [c["n"] for c in header(frame)["c"]])
        checks.expect("each join was answered", all("status" in j for j in joins), joins)
        timings, refused = [], []
        jobs = [lambda n=n, k=k: game.press(tok[n], f"lure:{5 + k},{(k * 7) % 60}", timings) for n in ("alice", "bob", "cleo") for k in range(12)]
        storm = parallel(jobs)
        refused = [s for s in storm if "http" in s or "error" in s]
        logged = module_rows(workdir, "SELECT count(*) FROM moves WHERE act='lure'")[0][0]
        checks.expect("every lure that was accepted reached the log", logged == len(storm) - len(refused), f"{logged} logged, {len(refused)} refused of {len(storm)}")
        raced, _ = game.stored(token)
        last = module_rows(workdir, "SELECT max(_id) FROM moves")[0][0]
        print(f"     the frame slim kept after the storm had replayed {header(raced)['mv']} of {last} moves (last write wins)")
        frame = game.press(token, "refresh")
        checks.expect("the next frame has replayed the whole log", header(frame)["mv"] == last, f"mv {header(frame)['mv']} of {last}")
        print(f"     36 simultaneous presses: p50 {pct(timings, 0.5)} ms, p95 {pct(timings, 0.95)} ms, max {pct(timings, 1.0)} ms; refused {[r.get('http') or r.get('error') for r in refused][:5]}")

        print("phase 2: nobody presses; the host redraws the board on its own")
        _, first_at = game.stored(token)
        seen = {first_at}
        end = time.time() + 9
        while time.time() < end:
            _, at = game.stored(token)
            seen.add(at)
            time.sleep(0.2)
        stamps = sorted(seen)
        gaps = [b - a for a, b in zip(stamps, stamps[1:])]
        checks.expect("the board was redrawn without a press", len(stamps) >= 3, f"{len(stamps) - 1} redraws in 9 s, gaps {gaps}")
        stored, _ = game.stored(token)
        print(f"     redraw gaps (ms): {gaps}; tick now {header(stored)['tick']}")

        print("phase 3: the space feeds the soil")
        sent = time.time()
        for k in range(4):
            ok(server, "POST", f"/channels/{ch['garden']}/messages", {"id": uuid7(), "content": f"a message to feed the soil {k}"}, tok["bob"])
        chat = ok(server, "POST", f"/channels/{ch['garden']}/messages", {"id": uuid7(), "content": "react to me"}, tok["bob"])
        ok(server, "PUT", f"/messages/{chat['id']}/reactions/{urllib.parse.quote(MUSHROOM)}", None, tok["cleo"])
        wait_for("four leaves and the rain in the stored frame", lambda: (lambda s: s and header(s)["leaves"] >= 5 and header(s).get("rain"))(game.stored(token)[0]), timeout=30)
        checks.expect("messages fell as leaves and a reaction brought rain", True, f"{time.time() - sent:.1f} s from the posts to the board")

        print("phase 4: my colony is private and its burst lands once")
        before = game.stored(token)[1]
        panel = game.press(tok["alice"], "my colony")
        checks.expect("the panel is private", panel.get("private") is True and any(o["op"] == "image" for o in panel["ops"]))
        name = next(o["s"] for o in panel["ops"] if o["op"] == "text")
        burst = game.press(tok["alice"], "burst")
        checks.expect("the burst was taken", burst.get("status", "").startswith(("your colony surges", "a burst takes")), burst.get("status"))
        frame = game.press(tok["alice"], "back")
        mine = next(c for c in header(frame)["c"] if c["n"] == name)
        checks.expect("the burst reached the shared board", mine.get("b", 0) > 0 or "a burst takes" in burst.get("status", ""), mine)
        image = next(o["b64"] for o in panel["ops"] if o["op"] == "image")
        print(f"     the panel frame is {len(json.dumps(panel))} bytes, its portrait {len(image)} base64 characters; the stored frame did not move: {game.stored(token)[1] != before or 'redraws continue'}")

        print("phase 5: alice links her profile")
        code_panel = game.press(tok["alice"], "link profile")
        code = next(o["s"].split()[1] for o in code_panel["ops"] if o["op"] == "text" and o["s"].startswith("Post ug-"))
        ok(server, "POST", f"/channels/{ch['garden']}/messages", {"id": uuid7(), "content": f"linking {code}"}, tok["alice"])
        wait_for("alice's link", lambda: module_rows(workdir, f"SELECT 1 FROM players WHERE user_id='{people['alice'][0]}'"), timeout=20)
        sections = ok(server, "GET", f"/space/dock/slots/profile/{people['alice'][0]}", token=token)["sections"]
        checks.expect("alice's profile shows the undergrowth", "fed the undergrowth" in json.dumps(sections), json.dumps(sections)[:200])
        bobs = ok(server, "GET", f"/space/dock/slots/profile/{people['bob'][0]}", token=token)["sections"]
        checks.expect("bob, who only chatted, shows his leaves", "fed the undergrowth 5 leaves" in json.dumps(bobs), json.dumps(bobs)[:200])

        print("phase 6: the short season runs out and is announced once")
        wait_for("the season to end", lambda: (lambda s: s and header(s)["over"])(game.stored(token)[0]), timeout=90, every=1)
        stored, _ = game.stored(token)
        checks.expect("a finished board stops asking to be redrawn", "refresh_ms" not in stored)
        posts = wait_for("the herald", lambda: [m for m in ok(server, "GET", f"/channels/{ch['almanac']}/messages?limit=20", token=token) if "Season 1 of the undergrowth is over" in m["content"]], timeout=40)
        checks.expect("the result was posted once, as the module", len(posts) == 1, posts[0]["content"])
        print(f"     {posts[0]['content']}")
        records = module_rows(workdir, "SELECT seasons, wins FROM players WHERE seasons > 0")
        checks.expect("every player has the season on record", len(records) == 3 and sum(w for _, w in records) == 1, records)
        ids = ",".join(people[n][0] for n in people)
        badges = ok(server, "GET", f"/space/dock/slots/badges?ids={ids}", token=token)["badges"]
        print(f"     champion badges: { {n: [b['nodes'][0]['text'] for b in badges.get(people[n][0], [])] for n in people} }")

        print("phase 7: the almanac by hand")
        reply = ok(server, "POST", f"/modules/{MODULE}/commands/almanac", {"input": "", "channel_id": ch["almanac"]}, token)
        checks.expect("the almanac counted the day", reply.get("ok") and "leaves fell" in reply.get("output", ""), reply)

        print("phase 8: a huge world, a long sleep and a fast play loop")
        ok(server, "PUT", f"/space/dock/modules/{MODULE}/settings", {"values": {"world": "huge", "tick_ms": 150, "redraw_s": 60, "season_ticks": 50000}}, token)
        huge = Game(server, ch["garden"], token)
        parallel([lambda n=n: huge.press(tok[n], "join") for n in ("alice", "bob", "cleo")])
        time.sleep(7)
        catch_up = []
        frame = huge.press(tok["alice"], "refresh", catch_up)
        checks.expect("a 40-tick catch-up on a 128 by 80 world comes back", "state" in frame, frame)
        print(f"     40 ticks of a huge world: {round(catch_up[0])} ms over HTTP, frame {len(json.dumps(frame))} bytes")
        loop, refusals = [], []
        for _ in range(40):
            got = huge.press(tok["bob"], "step", loop)
            if "state" not in got:
                refusals.append(got)
            time.sleep(0.13)
        checks.expect("a play loop at the client's pace is never refused", not refusals, refusals[:3])
        print(f"     play loop: p50 {pct(loop, 0.5)} ms, p95 {pct(loop, 0.95)} ms over {len(loop)} steps")

        print("server-side numbers per command (from GET /space/dock/installed):")
        installed = ok(server, "GET", "/space/dock/installed", token=token)
        mine = next(m for m in installed if m["id"] == MODULE)
        for stat in mine.get("stats", mine.get("run_stats", [])):
            print(f"     {stat['command']:12} runs {stat['run_count']:>4}  failures {stat['failure_count']:>3}  max {stat['max_duration_ms']:>5} ms  max fuel {stat['max_fuel_used']:>12,}")
        size = (workdir / "modules" / f"{MODULE}.db").stat().st_size
        print(f"     module database: {size} bytes")
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
