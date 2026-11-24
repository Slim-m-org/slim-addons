//! The events kept in `kv.store`: one value per event, found by listing the store's own keys.
//! No shared index or counter is rewritten, so runs that overlap cannot lose each other's events.

use crate::event::Event;
use crate::host::Host;

/// Live events at once: 20 events of up to about 3 KiB fit the 64 KiB a module's store holds.
pub const MAX_EVENTS: usize = 20;
/// A finished event stays this long so its message can still be drawn.
const KEEP_AFTER_START_MS: i64 = 24 * 3_600_000;
const PREFIX: &str = "e:";

fn key(id: &str) -> String {
    format!("{PREFIX}{id}")
}

pub fn load(host: &dyn Host, id: &str) -> Result<Option<Event>, String> {
    Ok(host.get(&key(id))?.and_then(|raw| serde_json::from_str(&raw).ok()))
}

pub fn save(host: &dyn Host, id: &str, event: &Event) -> Result<(), String> {
    let raw = serde_json::to_string(event).map_err(|_| "could not store the event".to_owned())?;
    host.set(&key(id), &raw)
}

pub fn ids(host: &dyn Host) -> Result<Vec<String>, String> {
    let mut ids: Vec<String> = host.keys()?.iter().filter_map(|k| k.strip_prefix(PREFIX)).map(str::to_owned).collect();
    ids.sort();
    Ok(ids)
}

/// One stored value: the event, or `None` when its text no longer parses.
type Slot = (String, Option<Event>);

/// Every stored id with its event; a key deleted since the listing is left out.
fn slots(host: &dyn Host) -> Result<Vec<Slot>, String> {
    let mut found = Vec::new();
    for id in ids(host)? {
        if let Some(raw) = host.get(&key(&id))? {
            found.push((id, serde_json::from_str(&raw).ok()));
        }
    }
    Ok(found)
}

/// Every stored event with its id; a value that does not parse is left out.
pub fn all(host: &dyn Host) -> Result<Vec<(String, Event)>, String> {
    Ok(slots(host)?.into_iter().filter_map(|(id, event)| Some((id, event?))).collect())
}

/// Removes events that started more than a day ago, and values that no longer parse; returns how many went.
pub fn prune(host: &dyn Host, now_ms: i64) -> Result<usize, String> {
    Ok(sweep(host, now_ms)?.0)
}

/// The prune, answering the events it kept as well as how many it removed.
fn sweep(host: &dyn Host, now_ms: i64) -> Result<(usize, Vec<(String, Event)>), String> {
    let (mut removed, mut kept) = (0, Vec::new());
    for (id, event) in slots(host)? {
        match event {
            Some(event) if event.at + KEEP_AFTER_START_MS >= now_ms => kept.push((id, event)),
            _ => {
                host.delete(&key(&id))?;
                removed += 1;
            }
        }
    }
    Ok((removed, kept))
}

fn next_number(events: &[(String, Event)]) -> u32 {
    events.iter().map(|(_, e)| e.n).max().unwrap_or(0) + 1
}

/// Stores a new event under `id` (made unique if taken), numbering it; refuses when the module already holds the most it can.
pub fn create(host: &dyn Host, id: &str, mut event: Event, now_ms: i64) -> Result<(String, Event), String> {
    let (_, live) = sweep(host, now_ms)?;
    if live.len() >= MAX_EVENTS {
        return Err(format!("{MAX_EVENTS} events are already planned; cancel one or wait for one to pass"));
    }
    event.n = next_number(&live);
    let id = if live.iter().any(|(taken, _)| taken == id) { format!("{id}{}", event.n) } else { id.to_owned() };
    save(host, &id, &event)?;
    renumber_if_shared(host, &id, &mut event)?;
    Ok((id, event))
}

/// Two runs that planned an event together can pick the same number; the one that sees the clash takes the next free one.
fn renumber_if_shared(host: &dyn Host, id: &str, event: &mut Event) -> Result<(), String> {
    let others: Vec<(String, Event)> = all(host)?.into_iter().filter(|(other, _)| other != id).collect();
    if others.iter().any(|(_, other)| other.n == event.n) {
        event.n = next_number(&others).max(event.n + 1);
        save(host, id, event)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::fake::FakeHost;

    fn numbered(n: u32) -> Event {
        let mut event: Event = serde_json::from_str(r#"{"n":0,"t":"x","at":9999999999999,"z":0,"by":"h"}"#).unwrap();
        event.n = n;
        event
    }

    #[test]
    fn an_event_that_shares_a_number_with_another_takes_the_next_free_one() {
        let host = FakeHost::default();
        save(&host, "a", &numbered(2)).unwrap();
        let mut mine = numbered(2);
        save(&host, "b", &mine).unwrap();
        renumber_if_shared(&host, "b", &mut mine).unwrap();
        assert_eq!(mine.n, 3);
        assert_eq!(load(&host, "b").unwrap().unwrap().n, 3);
        assert_eq!(load(&host, "a").unwrap().unwrap().n, 2);
    }
}
