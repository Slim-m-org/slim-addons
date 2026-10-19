# poll

A live poll you launch from the apps menu, or with `/poll Question? | option a, option b`.
Two to six options.
Tap a bar to vote, tap another bar to move your vote, and everyone watching sees the bars update.
"close poll" stops further votes and shows the result, and "reset" starts over.

## One vote per person

The host hands a module `caller.id`, an opaque id that is stable for one person in one module (slim-m decision 0038).
The poll keeps one row per id, `id -> option`, and counts the bars from those rows.
Nothing stores a running tally, so a person cannot hold two votes.
Tapping the bar you already chose changes nothing, and tapping a different one moves your row.

A caller with no usable id (empty, not lowercase hex, or longer than 64 characters) cannot vote.
The poll says so instead of counting an anonymous tap.

A poll holds at most 200 voters so the state stays small.
Someone already in can still change their vote once it is full.

## What this does not guarantee

A module keeps nothing between frames, so the question, the options and the voter rows ride in the scene state.
That state is echoed back by each member's client and stored last-write-wins.
Two consequences:

- A member who forges the state string can add voters, remove them, or rewrite the counts.
  The module drops rows with a bad id, a missing option or a repeated id, but it cannot tell a forged valid row from a real one.
- Two people voting at the same moment can lose one vote, because each frame starts from the state that person last saw.

So this stops accidents and casual padding, not a determined member.
Do not use it where the count has to be authoritative.
The same limit applies to tic-tac-toe and connect-four seating.
Any member can also close or reset a poll.

## Permissions

It declares one permission, `vote`, which gates the command, the slash command and the app.
It asks for one host capability, `command.register`, because it registers a command.
It does not post, read or store anything in the space, and it imports nothing from the host.

## State format

Fields are separated by `U+001E`: open flag, question, options, voters.
Options and voters are separated by `U+001D`, and a voter row is `id U+001F option-index`.
Control characters are stripped from the question and options, and ids must be hex, so text cannot collide with the framing.
A poll made before voters were recorded parses with its counts reset to zero.

## Tests

```bash
cd modules/poll
cargo test --locked
```

They cover a vote, a changed vote, a second person, a third option, a closed poll, a caller with no id, a forged or repeated voter row, and the voter cap.
To package after a change, bump the version if it was ever published, then run `scripts/package-module.sh modules/poll`.
