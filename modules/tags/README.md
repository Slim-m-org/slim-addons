# Tags

A space keeps short named snippets - rules, FAQs, links - and anyone allowed can drop one into the channel.
It is the tags feature of bots like YAGPDB and Carl-bot, as a module.

## Commands

| command | permission | what it does |
| --- | --- | --- |
| `/tag` | use | posts the list of tag names |
| `/tag name` | use | posts that tag as you, with a "via Tags" footer |
| `/tagadd name text` | manage | adds a tag |
| `/tagedit name text` | manage | replaces a tag's text |
| `/tagremove name` | manage | removes a tag |
| Tags app (`/tags`) | use | a board of tag names; tap one to read it |

## Why the commands are split this way

A module is never told the caller's permissions.
The only gate is the `permission` declared on each extension point, and a slash command maps to exactly one command with the text after the keyword as its input.
So a single `/tags add ...` command cannot be use for some verbs and manage for others.
Each manage verb is its own slash command (`/tagadd`, `/tagedit`, `/tagremove`) under `manage`, and reading stays under `use`.

The app is under `use` and is read-only.
An app frame is stored and shown to everyone in the channel, and it runs through the shared run route with the viewer's identity, so a manage action on it would not be told apart from a read.

## Posting

The composer posts a slash command's output as a plain message from the person.
So `/tag name` calls `message.post` (which adds the footer) and answers with empty output, so nothing is posted twice.
If `message.post` is not approved, or there is no channel, the text becomes the output instead, which the composer posts as the person's own message without the footer.
A post that the host refuses (slow mode, rate limit, no send permission) is shown as an error and is not retried.
The manage commands post a one-line confirmation when they can, and stay quiet otherwise.

The app cannot post: app frames run without `message.post`.
The board shows a tag and says so in its status line.

## Storage

`kv.store`, which allows keys up to 256 bytes, values up to 4 KiB, 256 entries and 64 KiB in all, and 64 calls per run.

| key | value |
| --- | --- |
| `t:<name>` | the tag's text |
| `i` | the sorted tag names, one per line |

Listing is one read and posting a tag is one read.
Adding or removing a tag is four calls, editing is two.
The tag key is the truth and the index only feeds listing and the count, so an index that fell behind (two people adding at the same moment can do it) never hides a tag from `/tag name`.

Limits, chosen to leave headroom under the host's:

- 40 tags
- names of 1 to 32 characters: lowercase letters, digits and hyphens, starting and ending with a letter or digit; input is lowercased
- text of up to 1000 characters and 1500 bytes, since 40 tags at 1500 bytes plus the keys and the index come to about 62 KB of the 64 KiB

## Approvals

The Dock install screen has a switch per capability, off by default.
Without `kv.store` every command answers "An admin needs to allow Tags to store data in the Dock."
Without `message.post` posting falls back as described above.

## Testing

```bash
cd modules/tags && cargo test --locked
scripts/package-module.sh modules/tags
```

`tools/run-module` drives the wasm as the server does, with `kv.store` always approved and held in memory (`--kv-file` keeps it between runs).
It has no `message.post`, so a run there shows the no-`message.post` fallback.
The posting path is covered by the unit tests, which swap in a fake host that enforces the same `kv.store` limits.
