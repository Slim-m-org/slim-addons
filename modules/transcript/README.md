# Call transcript

An opt-in transcript of a voice call.
When everyone in a call has agreed, the module writes down what is said and posts a readable transcript in the channel an admin picked when the call ends.
It is the first consumer of the `call.audio` capability (slim-m decision 0105, add-on API 4).

## What a transcript looks like

```text
**Call transcript** - 2023-11-14 22:13 UTC, 10 min

[0:05] Speaker 1: Okay, so the plan for Friday is the release. Anyone blocked on review?
[0:14] Speaker 2: No, I merged mine this morning.

_End of transcript._
```

- Lines are in the order they were said, timed from the start of the call (`m:ss`, or `h:mm:ss` from the first hour).
- A speaker's lines that follow each other, less than the merge gap apart, read as one paragraph.
- A long call is posted as several messages, each under the 4,000 character limit, titled `(part 2)` and on.
- The last message says `End of transcript.`, and says so when segments never arrived or the call stopped early.
- Speech can never mention anyone: `@` is swapped for a look-alike.

## Speakers are numbers, not names

slim tells a module only a user id, and gives it no way to turn that into a display name.
So speakers are `Speaker 1`, `Speaker 2` and so on, in the order the module first heard them.
Names need slim to send a display name with the call events, or a host call that resolves a member.

## Consent

The module hears nothing until an admin has installed it, ticked the call-audio warning, chosen the voice channels it may hear, and everyone in a call has said yes.
It adds its own checks on top of the host's:

- A call the module was never told had started is ignored, along with everything said in it.
- Only people in the call's agreed set are written down.
  A person who is taken out of it, by saying no, is not written down from then on.
- A call that was declined is deleted, and nothing is posted.
- A call nobody spoke in posts nothing.

## Posting

The channel is the module's posting channel, picked on its Dock page (the same one a timer uses).
The module posts as itself.
With no channel picked the transcript waits, and the timer posts it as soon as one is.

A run may post three messages, so a long transcript finishes on the timer, every 5 minutes.
The number of lines already posted lives on the call and moves with one conditional write before each post.
Two runs that reach the same call, the end of the call and the timer, never post the same lines twice.
A post the host refuses gives its lines back, and the next run tries again.

`/transcript` posts what has been said so far in a call that is on, as the member who ran it, in the channel they ran it from.
It posts at most three messages, says so when there is more, and does not change what the final transcript posts.
It needs the `post` permission, which an admin grants.

## What it keeps

| table | holds |
| --- | --- |
| `calls` | one row per call: times, state (`on`, `done`, `posted`), why it stopped, how many segments arrived, were sent and were dropped, how many lines are posted |
| `people` | who is in the agreed set of a call and the speaker number they hold |
| `lines` | each utterance: speaker, start, end and text, at most 1,000 characters |

User ids are in columns marked `member`, so deleting an account removes their lines.
Calls are deleted `keep_days` after they ended (30 by default, 1 to 90), whether or not they were posted.
A call stops taking lines at 9,000, because a query reads at most 10,000 rows in.
The timer also closes a call that has heard nothing for 30 minutes, which is how a call survives a server restart that never sent `call.audio.stopped`.

## Settings

| key | what |
| --- | --- |
| `keep_days` | how long a transcript stays stored after the call |
| `merge_gap_s` | how close one speaker's lines must be to read as one paragraph |
| `timestamps` | whether each paragraph starts with the time into the call |

## Capabilities

| capability | what for |
| --- | --- |
| `call.audio` | receives the call's text, one event per utterance |
| `data.store` | keeps the call until it is posted, and for the days the admin set |
| `message.post` | posts the transcript |
| `schedule` | posts the later parts of a long transcript, closes quiet calls and deletes old ones |
| `command.register` | `/transcript` and the timer's `flush` |

## Layout

`ingest.rs` handles the four events, `post.rs` cuts a call into messages and keeps the posted count, `render.rs` is the pure layout (labels, clock, merging, the length budget), `command.rs` is `/transcript` and the timer, and `store.rs` is the thin layer over `data.store` and `message.post`.
`fake.rs` is an in-memory slim built from this module's own manifest, with the host's budgets and limits.

## Testing

```bash
(cd modules/transcript && cargo test)
```

The tests send synthetic `call.audio.*` events and read what the module posts.
They cover ordering, merging, posting once, consent, the three-post budget, the message limit, a missing channel, retention and the manifest.

The built wasm also runs through the runner, which knows the four events:

```bash
R=tools/run-module/target/release/run-module
W=modules/transcript/0.1.0/module.wasm
$R --data-file /tmp/t --event call.audio.started '{"session_id":"s1","channel_id":"c1","started_at_ms":1700000000000,"included":["u1"]}' $W
$R --data-file /tmp/t --event call.audio.segment '{"session_id":"s1","channel_id":"c1","segment_id":"call-audio:s1:2","user_id":"u1","start_ms":1700000005000,"end_ms":1700000008000,"text":"hello","language":"en"}' $W
$R --data-file /tmp/t --event call.audio.stopped '{"session_id":"s1","channel_id":"c1","stopped_at_ms":1700000600000,"reason":"call_ended","segments_sent":1}' $W
```

The end-to-end run starts a real `slimm-server` with this repository as the Dock's dev origin, and a stand-in for the call-audio listener that speaks `listener/1` over a real WebSocket.
It installs the module as the admin, puts two members in a call through signed LiveKit webhooks, has both agree, plays scripted speech from the listener (including a speaker who never agreed), runs `/transcript`, stops the call, and reads the transcript the module posted.
No audio and no whisper model are involved; it checks everything from the text segments on.
It needs `pip install websockets` and about a minute.

```bash
scripts/package-module.sh modules/transcript   # rebuild the wasm the dev origin serves
python3 scripts/e2e-transcript.py --server /path/to/slimm-server
```
