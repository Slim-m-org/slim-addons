# pelican

Lists the game servers on a [Pelican](https://pelican.dev) panel with their state, CPU and RAM, and can keep one status message up to date.
It is a connector, not a wasm module: the logic is a bot (`bots/pelican` in [slim-bots](https://github.com/Slim-m-org/slim-bots)) that you run yourself, and this manifest is what lets the Dock install it, show its settings and give it a token.
Nothing in this folder runs inside slim-m.

## What it asks for

The Dock shows the ask before anything is installed, and you can approve less.

- `view_channel` and `send_messages`: the bot has to see the channels it is in to connect, and posts the status message and the power-action log line.
  Without `view_channel` the bot is refused at start and says so.
- A "See game servers" permission, which you give to the roles that may use `/servers` and the Game servers panel.
- Settings, filled in from the Dock: the panel address, a Client API key (a secret, only readable by the bot's own token), whether plain http is allowed for a panel on your own network, which servers to show, a control role, a log channel, a status channel and the refresh interval.

It declares `/servers` and a Game servers panel.
The bot also answers `!servers`, `!server <name>` and, for the control role, `!start`, `!stop` and `!restart`; those are typed commands and are not in the composer's list.
Every power action posts one line to the log channel first, and with no log channel the power commands stay off.

## Running it

This manifest does not ask the host to run the bot, so you start it yourself, anywhere that can reach both slim-m and the panel.

1. In the Dock, install Pelican and approve what you are happy with.
   The install shows a token once; copy it.
   `POST /space/dock/modules/pelican/connector-token` makes a new one if you lose it, and closes the old one.
2. Fill in the settings in the Dock, then enable it.
3. Run the bot with the token:

```bash
git clone https://github.com/Slim-m-org/slim-bots && cd slim-bots/bots/pelican
pip install -r requirements.txt
SLIMM_URL=https://your.space SLIMM_BOT_TOKEN=slimbot_... SLIMM_CONNECTOR=1 python3 bot.py
```

The bot reads its settings from the Dock when it starts, so restart it after changing one.
Until it is running, `/servers` answers "Pelican is offline".
Uninstalling revokes the token and the bot exits.
It needs `slim-m` 0.23.0 or later from PyPI, and a slim-m server at add-on level 7.

## Host-run

A connector can also be started by the server itself, in a sandbox, but only when the operator has turned on the connector supervisor ([operating guide](https://github.com/Slim-m-org/slim-m/blob/main/docs/connectors/operating.md)).
This manifest does not offer that: a host-run manifest names the image and every `host:port` the connector may reach, and your panel's address is not known until you set it.
If a version that does is published here, installing it needs an operator who enabled the supervisor, and a server without one refuses the install.

## Where the manifest comes from

`manifest.json` here is a copy of `bots/pelican/connector.json` in slim-bots, which is the source of truth because the commands and settings it names have to match the bot's code.
Change it there first, copy it here and bump the version in the manifest and `index.json`.
`scripts/check-connector-sources.py` compares the two.
