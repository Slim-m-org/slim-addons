# Reminder

`/remind 30m take the bread out` sets a reminder, and the channel is told when it is time.
It is the reference module for the `schedule` capability and for the host's clock (`now_ms`), with `kv.store` and `message.post` alongside.

## Commands

| command | permission | what it does |
| --- | --- | --- |
| `/remind 30m text` | use | sets a reminder; `m`, `h` or `d`, from 1 minute to 30 days |
| `/reminders` | use | lists what is waiting, soonest first, with how long is left |
| `/unremind 2` | use | cancels a reminder by its number in that list |
| `due` (no slash command) | timer | posts what has come due, now; the timer calls it, nobody needs to |

Up to 20 reminders wait in a space at once, each up to 200 characters, shared by everyone.
A module is never told who is asking beyond an opaque id, so a reminder does not mention a person.
Say who it is for in the text.

## Why it needs the capabilities it asks for

| capability | what for |
| --- | --- |
| `kv.store` | remembers the reminders between runs; a module has no memory of its own |
| `schedule` | the host runs `due` every 5 minutes with nobody invoking it |
| `message.post` | `due` posts what is due, as Reminder, into the channel the admin picked |

The Dock install screen shows a switch for each, off by default, and a channel picker for the schedule.
Nothing posts until an admin picks a channel.
Reminders can be late by up to five minutes: that is the shortest interval a schedule may have.

## The clock

A module has no clock of its own.
The request carries `now_ms`, the host's UTC time in milliseconds, and Reminder adds the delay to it to get a due time.
The due time is part of the stored id (`0001700000000000-ab12cd`), so the ids sort in due order and the index needs no second sort.
A host that gives no clock answers "This server does not give modules the time" instead of storing a reminder due in 1970.

## How a tick works

1. `due` reads the index and loads reminders in due order, stopping at the first one not yet due, at most ten.
2. It posts them as one message, `Reminder: ...` per line.
3. Only after the post succeeded does it delete them.

A post the host refuses (rate limit, a missing approval) leaves the reminders in place, and the failed run shows in the module's Activity card in the Dock with the reason, so a reminder that never arrives is visible.

## Storage

| key | value |
| --- | --- |
| `r:<id>` | the reminder's text |
| `i` | the ids, one per line |

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
