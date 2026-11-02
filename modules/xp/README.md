# XP and levels

Members earn XP for time in voice calls, and a level bar shows on their profile with a short `Lv N` badge next to their name.
It is the reference module for the module platform's phase 1 contracts: `events`, `data.store`, `log.read`, `schedule`, settings and slots (slim-m decision 0091).

## How XP works

- A voice session pays `xp_per_minute` for each whole minute.
  With `only_with_others` on (the default), time alone in the call does not count.
- A message pays `xp_per_message` at most once per `message_cooldown_s` per member.
  It is off by default, and bots never earn.
- Level 1 starts at zero XP.
  Climbing from level `n` to `n + 1` costs `100 * n` on the standard curve, `50 * n` on gentle and `100 * n * n` on steep.
  The cap is level 999.
- A member with no XP has no section and no badge.
- Settings apply when a session is counted, not when it happened.

## Why it does not lose or double count

A pushed event is only a trigger.
On every `voice.session.closed` event and every 5 minutes, the module reads the closed sessions after its cursor from `log.read`, awards them, and moves the cursor only after the writes succeeded.
Each member row also remembers the last session id taken for it, and a session is claimed and paid in one conditional `update` (set `last_session`, add to `xp` and `voice_ms`, where `last_session` is lower), so a lost cursor write or two runs overlapping cannot pay one session twice and a crash cannot take the claim without the payment.
A pass reads the rows of up to 50 members with one `in` query, makes the missing ones with one multi-row `insert`, and then spends one call per member.
A run has 64 host calls, so a backlog drains over several runs, about 50 members per pass.
A message costs two calls: an `upsert` that adds nothing to a row that exists, and the conditional `update` that takes the cooldown slot and pays.

## Data

| table | columns |
| --- | --- |
| `xp` | `user_id` (unique), `xp` (indexed), `voice_ms`, `messages` (messages that paid), `last_session`, `last_msg_ms` |
| `meta` | `name` (unique), `val`; holds `voice_cursor` |

The cursor column is `name`, not `key`: the host refuses SQLite keywords as column names.

## Capabilities

| capability | what for |
| --- | --- |
| `log.read` | reads when members were in calls, so no call is missed while the server was down |
| `events` | is told when a call ends and when a message is posted |
| `data.store` | keeps each member's XP |
| `schedule` | runs `catchup` every 5 minutes |

## Layout

`levels.rs` and `award.rs` are pure and carry the unit tests for the curves and the XP rules.
`store.rs` is the thin layer over `data.store` and `log.read`, `sync.rs` the catch-up and message awards, `slot.rs` the `slot/1` output, and `fake.rs` an in-memory slim the tests run against.

## Testing

```bash
(cd modules/xp && cargo test)
```

The end-to-end run starts a real `slimm-server` with this repository as the Dock's dev origin (`SLIMM_DEV_MODULE_DIR`), installs the module as the admin, plays calls through signed LiveKit webhooks, and checks the profile and badge slots, a restart and a call whose event was missed.
It needs about three minutes, because the server stamps sessions with its own clock and a voice minute is a real minute.

```bash
scripts/package-module.sh modules/xp          # rebuild the wasm the dev origin serves
python3 scripts/e2e-xp.py --server /path/to/slimm-server
```

Use a server build whose version has the module platform.
