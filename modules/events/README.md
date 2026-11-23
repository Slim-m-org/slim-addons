# Events

Post an event, collect Going, Maybe and Can't, and remind the channel before it starts.
Plan one from the apps menu, or type `/event Game night | tomorrow 8pm | Voice 1`.

## How it works

- The plan is `title | when | where`, and the place is optional free text (a voice channel's name works).
  The card shows the start time in the zone you typed it in, and a countdown that redraws every 30 seconds while someone has it open.
- Times are UTC unless you add a zone: `in 2h`, `in 1h30m`, `today 9pm`, `tomorrow 8pm`, `2026-12-05 20:00`, `2026-12-05 20:00 +02:00`.
  A start must be in the future and at most a year away.
- Going, Maybe and Can't are buttons; answering again moves your answer.
  The host is counted as going.
  Answers close when the event starts.
- The module knows people only by an opaque id, so the list shows `member 3f9c` until someone types a name into the box on the card.
- The host can cancel or edit from the card (`title | when | where`, a blank part keeps what was there).
  Anyone holding the Manage permission can use `/event-admin` to list events in the channel, `/event-admin cancel 3` or `/event-admin edit 3 title | when | where`.
  A card that was changed this way redraws the next time someone opens or presses it.
- About an hour before the start the module posts a reminder with the title, the time and who is going.
  An event planned less than 75 minutes ahead is reminded ten minutes before, and one planned less than about 12 minutes ahead is not reminded.
  Moving the start re-arms the reminder, and cancelling clears it.
- Up to 20 events are kept at once, with 50 answers each, in `kv.store`.
  An event is forgotten a day after it starts, and a daily sweep clears them even when nobody plans another.

## What it needs

- Add-on API 3 or newer (`run_after` timers and a scene that redraws itself).
- Capabilities `kv.store`, `message.post` and `schedule`; an admin approves each and chooses the channel reminders go to on the Dock screen.
- Permissions `events` (plan and answer) and `manage` (`/event-admin` and the sweep by hand).

## What it cannot do

- It cannot mention the people who are going, or know their real names: slim gives a module an opaque id and nothing else.
- A reminder is posted into the one channel the admin chose for the module, not into the channel the event was planned in.
- It cannot link a voice channel; the place is only text.
- Its timers are the host's one-shot `run_after`: a timer is not retried if the server was down when it came due.
