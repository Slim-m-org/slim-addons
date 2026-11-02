# Undergrowth

A fungal turf war that lives under a space.
Up to six members each grow a colony through a procedurally dealt slab of soil, fight over nutrients, and fruit mushrooms that sing.
The space feeds the soil: messages fall as leaves, reactions bring rain, time in calls brings a warm spell, and someone joining wakes a wild rot nobody plays.

It is meant to push the module platform: a shared scene that runs in real time, moves stored in `data.store` so simultaneous presses are never lost, the host redrawing the board on its own (`refresh_ms`), a private panel with a rendered PNG, sound, drag input, events, a durable log, a timer, a schedule, settings and profile slots, all in one module.

## How it plays

- Launch it from the apps menu and press **join** to root a colony near the surface.
- **Tap** the soil to lure your hyphae toward it, or **drag** to lay a trail of up to eight waypoints.
- **Long press** (or right click) your own hyphae to fruit a mushroom for 40 energy.
  A mushroom scores while it stands and throws spores that root outposts nearby.
- Hyphae drink nutrients for energy, harden with age, and take a rival's cell when more of yours surround it than its strength.
  Cells cut off from their heart and every outpost wither, so cutting a rival's network is a strategy.
- A colony that cannot pay its upkeep dies back, so swallowing half the soil is not a safe place to sit.
- **my colony** opens a panel only you see: your numbers, a rendered portrait of your network, a name field, **burst** (30 energy, a growth surge) and **spore shot** (60, an outpost at your last waypoint).
- **song** plays up to 32 standing mushrooms, spread across the soil, left to right, with a playhead sweeping it.
  Sound only plays for the person who pressed.
- A season ends after a set number of ticks.
  The results are posted to the module's channel once, and **new season** deals fresh soil and roots everyone again.

## The world runs on the clock

Every frame advances the world by the real time since the last one, at `tick_ms` per tick and at most 40 ticks a frame.
A tap, the host's redraw and someone's play loop therefore all agree on how fast the season goes, and a board nobody looked at for an hour wakes in one bounded step.
The frame asks the host to redraw it every `redraw_s` seconds, and the host keeps doing that for half an hour after the last press.

## Why two people pressing at once both count

On a shared surface slim hands every press the state of the last frame it stored, and stores whichever frame finishes last.
Two presses on the same frame would each build on it and one would vanish.
So a press never edits the board directly: it is inserted into the `moves` table, and every frame replays the moves after the last one its board has taken in, including its own.
Whichever frame slim keeps, the next one replays whatever it missed.
**new season** is a move too, carrying the season it starts, and its soil is seeded from the move's id, so every frame that replays it deals the same soil.
A finished season's final standings are written by the same conditional update that claims its end, so the posted results match the recorded ones, and a slower frame cannot write over them.
A press whose action is not a string still gets the stored board and only redraws it; a press with no `action` key is a launch, because slim only swaps in the stored board when the key is there.
The same cursor pattern reads the `inbox` the events fill, so every running game gets each leaf once.

## The space feeding the soil

| event | effect |
| --- | --- |
| `message.created` | a leaf of nutrients in the topsoil, and a point for the author's profile |
| `reaction.changed` | rain that speeds growth in a band of columns for 40 ticks |
| `member.joined` | wakes the wild rot, or feeds it if it is awake |
| `voice.session.closed` | read from the `voice.sessions` log after a stored cursor: a warm spell for the minutes spent with others, and credit on the profile |

An event only drops a row in the inbox.
Calls are counted from the durable log, so a call whose event was missed is still counted once, by the next event or the daily almanac.

## Profiles

A module is told who presses only by an opaque id, while events and profile slots use account ids, and nothing links the two.
**link profile** in the panel shows a code; posting it in any channel within 15 minutes links that member's account to their colony, so their record and the champion badge show on their profile.
Leaves and call time show on every member's profile with no linking, since those come from events.

## Data

| table | what it holds |
| --- | --- |
| `moves` | every press, by game, replayed by each frame |
| `inbox` | weather from events, read by every game after its own cursor |
| `games` | standings, and which season ended and was announced, so each happens once |
| `players` | each player's record across seasons, and the account they linked |
| `links` | short-lived profile link codes |
| `feeders` | leaves and call minutes per account |
| `facts` | the call-log cursor, the champion and the almanac's day |

`user_id` columns are marked `member`, so a deleted account takes its rows with it.

## Capabilities

| capability | what for |
| --- | --- |
| `data.store` | the move log, the inbox and every record above |
| `events` | messages, reactions, joins and finished calls |
| `log.read` | finished calls, so none is missed or counted twice |
| `schedule` | the daily almanac, and the one-shot timer that announces a season |
| `message.post` | posting the results and the almanac, as the module, in the channel an admin picks |

## Limits it was built against

| | small 48x30 | medium 64x40 | large 96x60 | huge 128x80 |
| --- | --- | --- | --- | --- |
| launch, fuel | 12M | 20M | 44M | 77M |
| a 40-tick catch-up, fuel | 55M | 95M | 182M | 237M |
| a stored frame | 11 KB | 15 KB | 27 KB | 43 KB |
| state inside it | 5 KB | 8 KB | 16 KB | 28 KB |

Measured with `run-module --fuel`, on crowded boards; the manifest allows 600M fuel and 3 s.
A stored frame may be 64 KiB, so the huge world is the largest this state format carries.

Two paths in slim itself can still lose the board, and the module cannot guard against either:

- slim hands a press the stored state only when its input has an `action` key, so a run posted with input `""` or `"{}"` on a board that already exists reaches the module as a launch and deals fresh soil.
- A press that fails on the host (the shared run slots stay busy past their wait, the time limit is hit, or the module crashes) is stored as the frame, so the next press gets no state and deals fresh soil.
  Six players running the play loop on a huge world, or other modules sharing the run slots, make this more likely.

## Layout

`world.rs` is the soil and its generation, `sim.rs` the tick, `moves.rs` what players and weather do to it, and `game.rs` the header that rides in `state`.
`play.rs` runs one press start to finish, `scene.rs` draws the shared frame, `panel.rs` the private one and its portrait, `png.rs` a small PNG writer with real deflate, and `song.rs` the notes.
`events.rs`, `slot.rs` and `herald.rs` are the event, profile and posting paths, `store.rs` the thin layer over `data.store`, and `fake.rs` an in-memory slim that reads its tables from the manifest.

## Testing

```bash
(cd modules/undergrowth && cargo test)
python3 scripts/play-modules.py tools/run-module/target/release/run-module --only undergrowth
```

The end-to-end run starts a real `slimm-server` with this repository as the Dock's dev origin, installs the module with every capability, and plays it through the shared code-run route: simultaneous joins and a storm of 36 presses, the host's own redraws, leaves and rain from real messages and reactions, the private panel, profile linking, a short season ending into one posted result, the almanac, and a huge world with a fast play loop.
It prints the server's own fuel and timing per command and takes about two minutes.

```bash
scripts/package-module.sh modules/undergrowth
python3 scripts/e2e-undergrowth.py --server /path/to/slimm-server
```
