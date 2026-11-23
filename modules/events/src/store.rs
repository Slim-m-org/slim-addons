//! The events kept in `kv.store`: one value per event, an index of their ids and a counter for numbering.

use crate::event::Event;
use crate::host::Host;

/// Live events at once: 20 events of up to about 3 KiB fit the 64 KiB a module's store holds.
pub const MAX_EVENTS: usize = 20;
/// A finished event stays this long so its message can still be drawn.
const KEEP_AFTER_START_MS: i64 = 24 * 3_600_000;
const INDEX: &str = "idx";
const COUNTER: &str = "seq";

fn key(id: &str) -> String {
    format!("e:{id}")
}

pub fn load(host: &dyn Host, id: &str) -> Result<Option<Event>, String> {
    Ok(host.get(&key(id))?.and_then(|raw| serde_json::from_str(&raw).ok()))
}

pub fn save(host: &dyn Host, id: &str, event: &Event) -> Result<(), String> {
    let raw = serde_json::to_string(event).map_err(|_| "could not store the event".to_owned())?;
    host.set(&key(id), &raw)
}

pub fn ids(host: &dyn Host) -> Result<Vec<String>, String> {
    Ok(host.get(INDEX)?.unwrap_or_default().split(',').filter(|s| !s.is_empty()).map(str::to_owned).collect())
}

fn write_ids(host: &dyn Host, ids: &[String]) -> Result<(), String> {
    host.set(INDEX, &ids.join(","))
}

/// Every stored event with its id, dropping ids whose value is gone.
pub fn all(host: &dyn Host) -> Result<Vec<(String, Event)>, String> {
    let mut found = Vec::new();
    for id in ids(host)? {
        if let Some(event) = load(host, &id)? {
            found.push((id, event));
        }
    }
    Ok(found)
}

/// Removes events that started more than a day ago; returns how many went.
pub fn prune(host: &dyn Host, now_ms: i64) -> Result<usize, String> {
    let mut kept = Vec::new();
    let mut removed = 0;
    for id in ids(host)? {
        match load(host, &id)? {
            Some(event) if event.at + KEEP_AFTER_START_MS >= now_ms => kept.push(id),
            Some(_) => {
                host.delete(&key(&id))?;
                removed += 1;
            }
            None => removed += 1,
        }
    }
    if removed > 0 {
        write_ids(host, &kept)?;
    }
    Ok(removed)
}

/// Stores a new event under `id` (made unique if taken), numbering it; refuses when the module already holds the most it can.
pub fn create(host: &dyn Host, id: &str, mut event: Event, now_ms: i64) -> Result<(String, Event), String> {
    prune(host, now_ms)?;
    let mut live = ids(host)?;
    if live.len() >= MAX_EVENTS {
        return Err(format!("{MAX_EVENTS} events are already planned; cancel one or wait for one to pass"));
    }
    let number = host.get(COUNTER)?.and_then(|n| n.parse::<u32>().ok()).unwrap_or(0) + 1;
    event.n = number;
    let id = if live.iter().any(|taken| taken == id) { format!("{id}{number}") } else { id.to_owned() };
    save(host, &id, &event)?;
    host.set(COUNTER, &number.to_string())?;
    live.push(id.clone());
    write_ids(host, &live)?;
    Ok((id, event))
}
