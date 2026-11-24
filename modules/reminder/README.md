# Reminder

`/remind 30m take the bread out` sets a reminder, and the channel is told when it is time.
It is the reference module for the `schedule` capability and for the host's clock (`now_ms`), with `kv.store` and `message.post` alongside.

## Commands

| command | permission | what it does |
| --- | --- | --- |
| `/remind 30m text` | use | sets a reminder; `m`, `h` or `d`, from 1 minute to 30 days |
| `/reminders` | use | lists your reminders, soonest first, with how long is left |
| `/unremind 2` | use | cancels a reminder by its number in that list |
| `due` (no slash command) | timer | posts what has come due, now; the timers call it, nobody needs to |

Each member can have 5 reminders waiting and a space up to 30, each up to 200 characters.
A reminder belongs to the member who set it: `/reminders` lists only theirs and `/unremind` cancels only theirs.
It is posted in the channel it was set in, by a timer armed for that reminder (add-on API 7), so text set in a private channel never reaches another one.
A module is never told who is asking beyond an opaque id, so a reminder does not mention a person.
Say who it is for in the text.
A reminder set before owners were kept has no owner or channel; it is posted in the channel an admin picked and nobody can list it.

## Why it needs the capabilities it asks for

| capability | what for |
| --- | --- |
| `kv.store` | remembers the reminders between runs; a module has no memory of its own |
| `schedule` | the host runs `due` for each reminder at its time, and every 5 minutes for old ones with no channel |
| `message.post` | `due` posts what is due, as Reminder, into the channel it was set in |

The Dock install screen shows a switch for each, off by default, and a channel picker for the schedule.
Nothing posts until an admin picks a channel.
A reminder armed from a channel fires at its own time; the five-minute schedule only covers old ones with no channel.

## The clock

A module has no clock of its own.
The request carries `now_ms`, the host's UTC time in milliseconds, and Reminder adds the delay to it to get a due time.
The due time is part of the stored id (`0001700000000000-ab12cd`), so the ids sort in due order.
A host that gives no clock answers "This server does not give modules the time" instead of storing a reminder due in 1970.

## How a tick works

1. `due` lists the reminder keys and loads reminders in due order, stopping at the first one not yet due, at most ten.
2. It posts them as one message, `Reminder: ...` per line.
3. Only after the post succeeded does it delete them.

A post the host refuses (rate limit, a missing approval) leaves the reminders in place, and the failed run shows in the module's Activity card in the Dock with the reason, so a reminder that never arrives is visible.

## Storage

| key | value |
| --- | --- |
| `r:<id>` | the owner, the channel and the text of the reminder; found by listing the keys |

## Testing

```bash
cd modules/reminder && cargo test --locked
scripts/package-module.sh modules/reminder
```

Run a tick offline by setting the clock yourself; `--kv-file` keeps the reminders between runs:

```bash
R=tools/run-module/target/release/run-module
W=modules/reminder/0.1.0/module.wasm
$R --kv-file /tmp/r --now-ms 1700000000000 $W remind '30m take the bread out'
$R --kv-file /tmp/r --now-ms 1700001900000 --tick due $W
```

The second command prints the `message.post` it would have made on stderr.
