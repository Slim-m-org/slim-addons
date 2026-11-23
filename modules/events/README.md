# Events

Post an event, collect Going, Maybe and Can't, and remind the people who are going before it starts.
Plan one from the apps menu, or type `/event Game night | tomorrow 8pm | Voice 1`.

## How it works

- The plan is `title | when | where`, and the place is optional free text (a voice channel's name works).
  The card shows the start time in the zone you typed it in, and a countdown that redraws every 30 seconds while someone has it open.
- Times are UTC unless you add a zone: `in 2h`, `in 1h30m`, `today 9pm`, `tomorrow 8pm`, `2026-12-05 20:00`, `2026-12-05 20:00 +02:00`.
  A start must be in the future and at most a year away.
- Going, Maybe and Can't are buttons; answering again moves your answer.
  The host is counted as going.
  Answers close when the event starts.
- The list shows each person by the name other members see for them in the space (`caller.name`, a snapshot taken when they last pressed an answer).
  A host that sends no name shows `member 3f9c`, the start of their opaque id, until they answer again with one.
- The host can cancel or edit from the card (`title | when | where`, a blank part keeps what was there).
  Anyone holding the Manage permission can use `/event-admin` to list events in the channel, `/event-admin cancel 3` or `/event-admin edit 3 title | when | where`.
  A card that was changed this way redraws the next time someone opens or presses it.
- About an hour before the start the module posts a reminder in the channel the event was planned in, with the title, the time and a mention of everyone who is going.
  It mentions the first eight and counts the rest, under the host's cap of ten mentions in a message, and writes each mention as `{{mention:<caller id>}}`, which the host turns into a real mention only for a member who can see the channel.
  Do Not Disturb and mute settings apply as for any mention.
  An event planned less than 75 minutes ahead is reminded ten minutes before, and one planned less than about 12 minutes ahead is not reminded.
  Moving the start re-arms the reminder, and cancelling clears it.
- Up to 20 events are kept at once, with 25 answers each, in `kv.store`.
  Each answer keeps the whole caller id, which the mentions need, and that is why it is 25 and not 50.
  An event is forgotten a day after it starts, and a daily sweep clears them even when nobody plans another.

## What it needs

- Add-on API 7 (`caller.name`, `{{mention:<caller id>}}` and a timer that posts in the channel it was set in).
- Capabilities `kv.store`, `message.post` and `schedule`; an admin approves each.
  No schedule channel is needed: the reminder goes to the event's own channel, as the module's own account, so that account must be allowed to send there.
- Permissions `events` (plan and answer) and `manage` (`/event-admin` and the sweep by hand).

## What it cannot do

- It cannot mention someone who cannot see the channel: the host leaves that token as plain text.
- It cannot mention more than ten people in one message, so a reminder for a larger crowd names eight and counts the rest.
- A name is a snapshot, so it can be out of date until the person answers again.
- It cannot link a voice channel; the place is only text.
- Its timers are the host's one-shot `run_after`: a timer is not retried if the server was down when it came due.
