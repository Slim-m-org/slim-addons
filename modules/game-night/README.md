# Game night

Ping a group, vote on the time that works, say In, Maybe or Out, and get a reminder before the start.
Plan one from the apps menu, or type `/night Valorant | fri 8pm, sat 9pm | Gamers`.

## How it works

- The plan is `game | times | role`.
  Times are comma separated, up to six: `fri 8pm`, `tomorrow 19:30`, `today 9pm` or `2026-11-20 18:00`.
  A weekday that has already passed today means next week, and a start can be at most 60 days away.
  The role is optional; with one, the module posts `@[Role] Valorant - who is up for it? ...` as the person who planned it.
- Everyone taps the times that work (tap again to take a vote back) and answers In, Maybe or Out.
  The last answer a member gives is the one that counts.
- The host (whoever planned it) closes the poll.
  The time with the most votes is chosen, the earlier one on a tie, and from then on only answers can change.
  The host can also cancel.
- A reminder is posted to the channel an admin chose for the module, `remind_minutes` before the start (15 by default, 0 turns it off), mentioning the role with how many are in and how many maybe.
  A night is reminded once.
  A reminder that came due after the start is dropped.
- Nights, with their votes, are forgotten 30 days after they were planned.

## Settings

| setting | default | what it does |
| --- | --- | --- |
| `utc_offset_minutes` | 0 | the offset the typed times are read in, so `8pm` means 8pm where your group is |
| `remind_minutes` | 15 | how long before the start the reminder goes out; 0 skips it |

## What it cannot do

A module knows members only by an opaque id.
It can count answers, but it cannot show who said In or mention the people who did, so a reminder mentions the whole role.
That is the reason the `events` bot in slim-bots (Going buttons, reminders that mention only the people going) is the better fit for a one-off plan with named attendees; this module is for finding a time.

## Data

| table | columns |
| --- | --- |
| `nights` | `night_id` (unique), `game`, `role`, `host`, `slots` (JSON of label and start), `chosen` (-1 open, -2 cancelled, else the time's index), `remind_at` (indexed), `reminded`, `created` (indexed) |
| `votes` | `vote_key` (unique), `night_id` (indexed), `voter`, `kind` (`slot` or `answer`), `val` |

A time vote is a row keyed `night|member|s<i>`, so a second tap is a refused insert that becomes a delete.
An answer is a row keyed `night|member|r`, upserted, so the last answer wins.
The reminder is claimed with a conditional `update` before it is posted, so two overlapping runs cannot post it twice.
A refused post is reported in the run's output and never fails the run, because a failing scheduled run counts toward disabling the module.

## Capabilities

| capability | what for |
| --- | --- |
| `data.store` | keeps nights and votes |
| `message.post` | posts the ping and the reminder |
| `schedule` | runs `remind` every 5 minutes |

## Layout

`parse.rs` reads the plan and the times and is pure.
`night.rs` is the thin layer over `data.store`, `play.rs` the taps, `scene.rs` the frames, `remind.rs` the timer.
`play.json` is the script `scripts/play-modules.py` plays through the wasm, including the reminders.

## Testing

```bash
(cd modules/game-night && cargo test)
cargo build --release --manifest-path tools/run-module/Cargo.toml
python3 scripts/play-modules.py tools/run-module/target/release/run-module --only game-night
```
