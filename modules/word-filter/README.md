# Word filter

Mask or block the words and patterns an admin lists, before a message is stored.
It checks sends and edits on every path the server runs interceptors on: members, bots, webhooks and other modules' posts.
It needs slim's add-on API 3 and the `interceptors` capability, which an admin approves when installing.

## How it works

- `words` is a list of words or phrases, matched as whole words and ignoring case, so `ass` does not touch `classic`.
  A phrase like `free crypto` matches those words in order, whatever spacing or punctuation is between them.
- `patterns` are regexes, one per line, also ignoring case.
  They run on a linear-time engine (the `regex` crate with no backtracking), so no pattern can blow up on a crafted message.
- `action` picks what happens on a match.
  Mask replaces every matched character with `*` and keeps spaces, so the sender's message is stored with the matches hidden.
  Block refuses the message and shows the sender `block_reason`.
- Channels listed in `exempt_channels` (channel ids, comma separated) are left alone.
- `/filter-test some text` shows what the filter would do with that text, without sending it.

## Settings

| setting | default | what it does |
| --- | --- | --- |
| `words` | empty | up to 100 words or phrases, separated by commas or new lines |
| `patterns` | empty | up to 5 regexes, one per line, each up to 100 characters |
| `action` | `mask` | `mask` or `block` |
| `block_reason` | "That message has a word this server filters." | what a blocked sender sees, up to 200 characters |
| `exempt_channels` | empty | channel ids the filter skips |

## Limits, and why

Each hook run gets 10 million fuel from the server, and the module compiles the admin's rules on every run because it keeps no state.
The caps keep the worst case inside that budget on a 4000-character message:

- At most 5 patterns, each up to 100 characters.
- A bounded repeat like `{1,20}` costs the matcher a state per count, and a repeat inside a repeat multiplies, so a pattern is weighed: each `{n,m}` multiplies what it repeats by `m`, `+` counts as two and `*` as one.
  A pattern may weigh 20 and all patterns together 40; past that the module says which pattern to change.
  Prefer `\S+` to `\S{1,50}`.
- At most 100 words, matched by a token lookup rather than a regex.
- Past 256 pattern matches in one message, the rest of the message is masked whole.

Measured with `run-module --fuel` on the built wasm, every pattern at its cap with 100 words on a 4000-character message took at most 7.2 million fuel.

A rule that does not compile makes the run fail.
The server then lets the message through, counts the failure on the module's page, and turns the module off after five in a row.
An admin who would rather refuse messages than let them through unchecked can set the hook to refuse on failure in the module's Checks card.

## What it cannot do

- Per-role exemptions: the interceptor input carries the message text and channel, not the sender's roles.
- Flag without acting: annotations are stored by the server, but no screen shows them yet.
- Patterns match ASCII case-insensitively; a listed word in another script matches by its lowercase.

## Testing

`cargo test` covers the matching, the caps and both phases.
To try the built module: `run-module --manifest modules/word-filter/manifest.json --setting words=darn modules/word-filter/0.1.0/module.wasm filter-test "oh darn"`.
Against a real server: `python3 scripts/e2e-word-filter.py --server <slimm-server binary>` installs it from this repo and checks masking, blocking, an exempt channel, edits and uninstall.
