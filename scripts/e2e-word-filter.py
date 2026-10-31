#!/usr/bin/env python3
"""End-to-end run of the word-filter module against a real slimm-server (stdlib only).

Usage: python3 scripts/e2e-word-filter.py --server target/debug/slimm-server

It starts the server on a free port in a private temp directory with this repo as the Dock's
dev origin, installs word-filter as the admin, and checks what a member's sends and edits are
stored as: masked, blocked with the reason, untouched in an exempt channel, and untouched once
the module is uninstalled. Takes a few seconds.
"""
import argparse
import importlib.util
import shutil
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("e2e_xp", HERE / "e2e-xp.py")
assert spec is not None and spec.loader is not None
xp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(xp)

MODULE = "word-filter"
SETTINGS = {"words": "darn, free crypto", "patterns": r"\d{3}-\d{4}", "action": "mask"}


def setup(server):
    admin = xp.ok(server, "POST", "/auth/register", {"username": "alice", "display_name": "Alice", "password": "pw-" + xp.uuid7(), "device_name": "e2e"})
    token = admin["access_token"]
    invite = xp.ok(server, "POST", "/invites", {}, token)["code"]
    bob = xp.ok(server, "POST", "/auth/register", {"username": "bob", "display_name": "Bob", "password": "pw-" + xp.uuid7(), "device_name": "e2e", "invite_code": invite})
    chat = xp.ok(server, "POST", "/channels", {"name": "chat", "kind": "text"}, token)["id"]
    free = xp.ok(server, "POST", "/channels", {"name": "free", "kind": "text"}, token)["id"]
    return token, bob["access_token"], chat, free


def install(server, token, settings):
    review = xp.ok(server, "GET", f"/space/dock/modules/{MODULE}?source=dev", token=token)
    xp.ok(server, "POST", f"/space/dock/modules/{MODULE}/install?source=dev", {"version": review["version"], "approved_host_capabilities": ["interceptors"], "reviewed_digest": review.get("review_digest")}, token)
    xp.ok(server, "PUT", f"/space/dock/modules/{MODULE}/settings", {"values": settings}, token)
    xp.ok(server, "POST", f"/space/dock/modules/{MODULE}/enable", None, token)


def send(server, token, channel, content):
    return xp.call(server, "POST", f"/channels/{channel}/messages", {"id": xp.uuid7(), "content": content}, token)


def stored(server, token, channel, message_id):
    return xp.ok(server, "GET", f"/channels/{channel}/messages/{message_id}", token=token)["content"]


def run(binary, keep):
    workdir = Path(tempfile.mkdtemp(prefix="e2e-word-filter-"))
    server = xp.Server(binary, workdir)
    check = xp.Checks()
    try:
        server.start()
        admin, bob, chat, free = setup(server)
        install(server, admin, {**SETTINGS, "exempt_channels": free})

        status, sent = send(server, bob, chat, "oh darn, call 555-1234 for FREE crypto")
        check.expect("a send is stored masked", (status, stored(server, bob, chat, sent["id"])), (200, "oh ****, call ******** for **** ******"))
        status, clean = send(server, bob, chat, "all good here")
        check.expect("a clean send is untouched", stored(server, bob, chat, clean["id"]), "all good here")
        xp.ok(server, "PATCH", f"/channels/{chat}/messages/{clean['id']}", {"content": "darn it"}, bob)
        check.expect("an edit is stored masked", stored(server, bob, chat, clean["id"]), "**** it")
        status, exempt = send(server, bob, free, "darn")
        check.expect("an exempt channel is untouched", stored(server, bob, free, exempt["id"]), "darn")

        xp.ok(server, "PUT", f"/space/dock/modules/{MODULE}/settings", {"values": {**SETTINGS, "action": "block", "block_reason": "Keep it clean."}}, admin)
        status, reply = send(server, bob, chat, "darn")
        check.expect("block mode refuses the send", status, 403)
        check.expect("the sender sees the reason", "Keep it clean." in str(reply), True)
        status, _ = send(server, bob, chat, "fine words")
        check.expect("block mode lets a clean send through", status, 200)

        xp.ok(server, "DELETE", f"/space/dock/modules/{MODULE}/install", None, admin)
        status, after = send(server, bob, chat, "darn")
        check.expect("after uninstall nothing is filtered", (status, stored(server, bob, chat, after["id"]) if status == 200 else None), (200, "darn"))
    finally:
        server.stop()
        if keep or check.failed:
            print("workdir kept at", workdir)
        else:
            shutil.rmtree(workdir, ignore_errors=True)
    return check.failed


def main():
    parser = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    parser.add_argument("--server", required=True)
    parser.add_argument("--keep", action="store_true")
    args = parser.parse_args()
    failed = run(args.server, args.keep)
    print("all checks passed" if not failed else f"{failed} check(s) failed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
