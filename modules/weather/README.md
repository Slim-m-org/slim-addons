# Weather

`/weather Berlin` answers with the current conditions for a place, from [Open-Meteo](https://open-meteo.com).
It is the reference module for the `net.fetch` capability: it shows the manifest's `network` block, two requests in one run, and how to turn a host refusal into something a person can act on.

## Command

| command | permission | what it does |
| --- | --- | --- |
| `/weather place` | use | looks the place up, then answers `Berlin, Germany: overcast, 12.3°C (feels like 10.1°C), wind 14 km/h` |

The answer is posted into the channel as the person who asked, with a "via Weather" footer.

## Why it needs `net.fetch`

A module cannot reach the internet on its own.
Weather asks for `net.fetch` and names exactly where it wants to go:

```json
"capabilities": ["command.register", "net.fetch"],
"network": { "hosts": ["geocoding-api.open-meteo.com", "api.open-meteo.com"] }
```

The Dock install screen shows those two hosts and a switch that starts off.
Only those hosts are reachable, nothing but the place name the person typed is sent, and no key or account is involved.
Open-Meteo is free for non-commercial use; see their terms before putting this in front of a lot of people.

## How it works

1. `GET https://geocoding-api.open-meteo.com/v1/search?name=<place>&count=1` finds the first matching place.
2. `GET https://api.open-meteo.com/v1/forecast?latitude=..&longitude=..&current=...` gives its current conditions.

That is two of the four requests a run may make.
The place name is percent-encoded byte by byte, so any name is one safe query value.
A non-200 answer or a body that is not the expected JSON becomes a plain sentence, never a trap.

A run can take as long as two slow requests, so the manifest asks for `wall_ms: 8000`.
slim gives a module that can fetch at least six seconds anyway, and answers `net.fetch timed out` to the module while it still has time to reply; Weather turns that into "The weather service is too slow right now".

## Without the approval

If an admin has not approved `net.fetch`, the command answers "An admin needs to allow Weather to reach the internet in the Dock."

## Testing

```bash
cd modules/weather && cargo test --locked
scripts/package-module.sh modules/weather
```

Try it offline with canned responses (the hosts in the file are the hosts the run may reach):

```bash
tools/run-module/target/release/run-module --fetch-file scripts/fixtures/weather-fetch.json modules/weather/0.1.0/module.wasm now Berlin
```
