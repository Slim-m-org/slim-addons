# slim-addons

The official module registry for [slim-m](https://github.com/Slim-m-org/slim-m).

A slim-m space installs modules from here through the Dock (space settings, admin only).
slim-m ships the module *system*; each module here ships its own behavior. See
slim-m's `docs/decisions/0021-modules-and-the-dock.md`.

## Layout

- `index.json` - the list the Dock browses. A row may carry `ref`, a commit sha or tag, and the Dock then reads that version's manifest and wasm from that fixed commit instead of `main`.
- `modules/<id>/manifest.json` - one module's contract (permissions it adds, host
  capabilities it asks for, runtime backend it needs, and its artifact + sha256).
- `modules/<id>/<version>/module.wasm` - the built module artifact, pinned by version.

Nothing here runs until an admin installs it into a space, and then only with the
permissions and capabilities the admin approves.

## Building your own module

Start from [`modules/_template/`](modules/_template/) - a copyable starter that
builds as-is, with a text command, a slash command, and a launchable interactive
app. Its README walks through making it yours, and
[`scripts/package-module.sh`](scripts/package-module.sh) builds the wasm, places
it at its versioned path, and pins its SHA-256 into the manifest.

The full contract - the ABI, the manifest, every extension-point kind, the scene
contract, limits, and publishing - is in slim-m's
[`docs/modules/building-modules.md`](https://github.com/Slim-m-org/slim-m/blob/main/docs/modules/building-modules.md).

## Running a module before you install it

`tools/run-module` drives a module exactly the way slim does - same ABI, same
wasmi version, and the same refusal of a module that imports anything but `slim.host_call` - so you
can see what yours actually emits without installing it into a space.

```bash
cargo build --release --manifest-path tools/run-module/Cargo.toml
R=tools/run-module/target/release/run-module

# a command, with its input
$R modules/spirograph/0.1.0/module.wasm spiro ''

# an interactive frame, the way a tap arrives
$R modules/spirograph/0.1.0/module.wasm spiro '{"action":"random","state":"34,13,21"}'
```

A module that needs to know who is acting gets a caller with `--caller <id>`:

```bash
$R --caller alice modules/tic-tac-toe/0.2.0/module.wasm play ''
```

A module that uses `kv.store` can run here too: the runner answers `slim.host_call` with an in-memory store that enforces slim's bounds, and `--kv-file <path>` keeps it between runs.

A module that uses `net.fetch` runs here against canned responses, so an author can iterate before pushing anything.
`--fetch-file <path>` is a JSON object keyed by full URL:

```json
{ "https://api.example.com/v1/up": { "status": 200, "content_type": "application/json", "body": "{\"up\":true}" } }
```

The hosts in that file are the hosts the run may reach, the way `network.hosts` is what an admin approves.
Any other host is refused with slim's own error, a listed host with no entry at that URL answers 404, and the same limits apply: four requests, 64 KiB out, 256 KiB back, sixteen headers.
Nothing opens a socket.

`message.post` is not refused here: each post is printed to stderr (`message.post #1: ...`) and answered with a made-up message id, up to the three a run may make.

The request carries `now_ms`, the host's clock.
`--now-ms <n>` sets it, which is how you test "9am" without waiting for it.
`--tick <command>` runs a command the way slim's timer does, with empty input and no person calling:

```bash
$R --kv-file /tmp/r --now-ms 1700000000000 modules/reminder/0.1.0/module.wasm remind '30m bread'
$R --kv-file /tmp/r --now-ms 1700001900000 --tick due modules/reminder/0.1.0/module.wasm
```

`--fuel <n>` meters the run and fails it with `out of fuel` when the budget is spent, the way slim refuses a module over its limit.
The scripts below pass the manifest's `runtime.limits.fuel` and a timeout built from `wall_ms`.
Without `--fuel` the run is unmetered, and memory is never limited here, so a module that runs here can still be refused there for being too expensive.
The manifest's `runtime.limits` is the authority on that.

### A reactive module: data, logs, settings, events and slots

A module that keeps `data.store` tables, reads `log.read`, has `settings`, subscribes to events or fills slots runs here too, with no server.
The runner reads the manifest from beside the wasm (`modules/<id>/manifest.json`), or from `--manifest <path>`, and builds the tables it declares in memory with slim's ops, bounds and refusals (unique columns, 64 store calls shared with `kv.store`, a file cap from `max_mb`).
`--data-file <path>` keeps the tables between runs.

`log.read` serves the logs in `--log-file <path>`, a JSON object of log name to rows in the order they closed.
The cursor is the position in that list, as slim's is a close-order cursor and not a row `id`.

```json
{ "voice.sessions": [{ "id": 1, "user_id": "alice", "channel_id": "c", "joined_at_ms": 0, "left_at_ms": 600000, "alone_ms": 0 }] }
```

Every run carries `settings`: the manifest's defaults, with `--setting key=value` (repeatable) laid over them and checked against the declared type and range.
`--event <name> '<data json>'` runs `@event` the way slim delivers a subscribed event, as the module's own principal; it refuses an event the manifest does not subscribe to, and one missing a `data` field.
It knows every event up to add-on API 5, and slim-m's `docs/modules/event-fixtures/*.json` hold a ready `data` for each API 5 one.
`--slot <kind> <name> <user,ids>` runs `@slot` as the viewer `--caller` names, and fails the run when the answer breaks the `slot/1` contract (an unknown node type, no `fallback`, an answer for a member nobody asked about).

```bash
$R --data-file /tmp/d --log-file sessions.json --tick catchup modules/xp/0.1.0/module.wasm
$R --data-file /tmp/d --event message.created '{"message_id":"m1","channel_id":"c","author_id":"u","content":"hi","created_at_ms":1700000000000,"is_bot":false}' --setting xp_per_message=3 modules/xp/0.1.0/module.wasm
$R --data-file /tmp/d --slot member.badge level alice,bob --caller viewer modules/xp/0.1.0/module.wasm
```

What it does not do: deliver events from a queue, cache a slot answer for 30 seconds, or enforce the wall clock.
Install on a dev deployment for those (`scripts/e2e-xp.py` plays signed webhooks against a real server).

### Checking a scene against the renderer

If your module draws, `scripts/check-scene-ops.py` reads its output the way the
client's parser does and reports anything that would be silently dropped - an op
that does not exist, a key on the wrong op, a colour that is not a scene colour.
Pass it the runner, the wasm, the command, and as many inputs as you want
covered:

```bash
python3 scripts/check-scene-ops.py "$R" modules/spirograph/0.1.0/module.wasm spiro \
  '' '{"action":"random","state":"34,13,21"}' 'not json at all'
```

This is worth running on malformed input too. A scene op the client drops does
not fail loudly anywhere - it just does not draw - which is how three of these
shipped before the check existed.

### Playing a game through its script

`scripts/play-modules.py` plays each app that keeps a `modules/<id>/play.json` through the real wasm: a list of scenarios, each a list of steps, where a step sends one action the way the client does (the action plus the state the last frame carried) and states what the frame must say.
It covers a win, a loss, a draw, input that names nothing, a move out of turn, a third player, a double submit, a reload from a saved state, two players answering from one stale board, and a hostile state, and it fails a frame that breaks `check-scene-ops.py` or a run that exhausts the manifest's fuel.

```bash
python3 scripts/play-modules.py "$R"                      # every script
python3 scripts/play-modules.py "$R" --only word-guess --show   # one module, printing each frame
```

The script's own header lists the step and expectation keys, and a misspelt key fails rather than checking nothing.
`scripts/check-module-tests.py` requires a script for every app, except those listed in `scripts/play-unwritten.txt`, which only shrinks.
The scripts assert behaviour, not wording, where the wording is not the point: they hold on a refused move that the board and turn did not change.

## Checks

CI (`.github/workflows/ci.yml`) runs on every pull request and push to main.
It runs `cargo test` for each module, `scripts/check-wasm-builds.py`, then `scripts/check-catalogue.py`, then `scripts/check-app-scenes.py`, which launches every app and checks the frame for each control, tap target and input its scene offers, as two different callers, then `scripts/check-command-modules.py`, which runs each command-only module through the wasm abi with a fixed input, then `scripts/play-modules.py`, which plays each game through its script.
Right after the `cargo test` loop, `python3 -m unittest discover -s scripts -p 'test_*.py'` runs the unit tests for these scripts and `cargo test` in `tools/run-module` runs the runner's own; `scripts/check-module-tests.py` fails a module whose `src` has no `#[test]`.

Each module keeps its logic in plain Rust with unit tests, so `cd modules/<id> && cargo test` runs them with no wasm involved.

`python3 scripts/check-catalogue.py` confirms every module has an `index.json` row that matches its manifest, that each pinned wasm still hashes to its `sha256`, and that every extension point names a permission and command the manifest declares.
With `--base origin/main` it also fails if a wasm that main already published was changed, or a manifest was edited without bumping its version.
Release notes are the one exception: `changelog` in a manifest lists what each version changed, newest first (`[{"version": "0.1.1", "changes": ["..."]}]`), and can be edited without a version bump because slim-m reads it fresh to show on its Modules screen.
The checker requires the newest entry to be the manifest's own version and holds each entry to the server's limits: at most 20 entries of 20 lines, each line 1 to 300 characters with no hidden characters.
When you ship a new version, add its entry at the top.

`python3 scripts/check-wasm-builds.py` rebuilds each module's wasm with [`scripts/build-wasm.sh`](scripts/build-wasm.sh) and fails if it differs from the one the manifest pins, so a source change cannot ship without a rebuilt wasm and a new version.
The build uses the toolchain pinned in `rust-toolchain.toml` and remaps every absolute path (cargo home, toolchain, checkout) to a fixed one, so the same source gives the same bytes on any machine.
`scripts/package-module.sh` uses the same recipe.
Rustup reads the pin automatically; a machine without that exact toolchain installed needs `rustup toolchain install 1.94.1` first.

Wasm files published before that recipe cannot match a rebuild.
They are listed in `scripts/wasm-unreproduced.txt`, which only shrinks: rebuild a module with `scripts/package-module.sh`, bump its version, and delete its line.
For a listed module, CI on a pull request instead fails a change to `src/`, `Cargo.toml`, `Cargo.lock` or `.cargo/` that keeps the version.
