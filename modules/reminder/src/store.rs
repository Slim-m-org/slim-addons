//! Pending reminders in `kv.store`: one key per reminder and an index of ids.
//! An id is the due time then a few characters of the run's entropy, so the
//! index sorts by when each is due and two made in one millisecond differ.

use crate::host::{Host, KvError};

const INDEX: &str = "i";
/// Pending reminders a space may hold: 20 at 200 characters stays well inside `kv.store`.
pub const MAX_PENDING: usize = 20;

pub struct Reminder {
    pub id: String,
    pub due_ms: i64,
    pub text: String,
}

fn key(id: &str) -> String {
    format!("r:{id}")
}

pub fn ids<H: Host>(host: &H) -> Result<Vec<String>, KvError> {
    let mut ids: Vec<String> = host
        .get(INDEX)?
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    ids.sort();
    Ok(ids)
}

fn save_ids<H: Host>(host: &H, ids: &[String]) -> Result<(), KvError> {
    host.set(INDEX, &ids.join("\n"))
}

/// The due time an id carries.
fn due_of(id: &str) -> i64 {
    id.split('-').next().and_then(|d| d.parse().ok()).unwrap_or(0)
}

pub fn load<H: Host>(host: &H, id: &str) -> Result<Option<Reminder>, KvError> {
    Ok(host.get(&key(id))?.map(|text| Reminder { id: id.to_string(), due_ms: due_of(id), text }))
}

/// What the index names: the reminders still stored, soonest first, and the ids whose text is gone.
pub struct Pending {
    pub live: Vec<Reminder>,
    pub dead: Vec<String>,
}

pub fn pending<H: Host>(host: &H) -> Result<Pending, KvError> {
    let mut found = Pending { live: Vec::new(), dead: Vec::new() };
    for id in ids(host)? {
        match load(host, &id)? {
            Some(reminder) => found.live.push(reminder),
            None => found.dead.push(id),
        }
    }
    Ok(found)
}

/// Every pending reminder, soonest first. An id whose text is gone is skipped.
pub fn all<H: Host>(host: &H) -> Result<Vec<Reminder>, KvError> {
    Ok(pending(host)?.live)
}

/// `Ok(None)` when the space already holds [`MAX_PENDING`] reminders that can still be listed.
/// The index is read again just before it is written, so a change made meanwhile is not overwritten.
pub fn add<H: Host>(host: &H, due_ms: i64, salt: &str, text: &str) -> Result<Option<String>, KvError> {
    let Pending { live, dead } = pending(host)?;
    if live.len() >= MAX_PENDING {
        return Ok(None);
    }
    let salt: String = salt.chars().filter(char::is_ascii_alphanumeric).take(6).collect();
    let id = format!("{due_ms:013}-{salt}");
    host.set(&key(&id), text)?;
    let mut all_ids = ids(host)?;
    all_ids.retain(|existing| !dead.contains(existing));
    all_ids.push(id.clone());
    all_ids.sort();
    all_ids.dedup();
    save_ids(host, &all_ids)?;
    Ok(Some(id))
}

/// Takes `gone` out of the index in one write, then deletes what each one stored (nothing, for a dead id).
pub fn remove<H: Host>(host: &H, gone: &[String]) -> Result<(), KvError> {
    if gone.is_empty() {
        return Ok(());
    }
    let mut remaining = ids(host)?;
    remaining.retain(|existing| !gone.contains(existing));
    save_ids(host, &remaining)?;
    gone.iter().try_for_each(|id| host.delete(&key(id)))
}
