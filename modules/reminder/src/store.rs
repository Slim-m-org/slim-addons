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

/// Every pending reminder, soonest first. An id whose text is gone is skipped.
pub fn all<H: Host>(host: &H) -> Result<Vec<Reminder>, KvError> {
    let mut found = Vec::new();
    for id in ids(host)? {
        if let Some(reminder) = load(host, &id)? {
            found.push(reminder);
        }
    }
    Ok(found)
}

/// `Ok(None)` when the space is already holding [`MAX_PENDING`].
pub fn add<H: Host>(host: &H, due_ms: i64, salt: &str, text: &str) -> Result<Option<String>, KvError> {
    let mut all_ids = ids(host)?;
    if all_ids.len() >= MAX_PENDING {
        return Ok(None);
    }
    let salt: String = salt.chars().filter(char::is_ascii_alphanumeric).take(6).collect();
    let id = format!("{due_ms:013}-{salt}");
    host.set(&key(&id), text)?;
    all_ids.push(id.clone());
    all_ids.sort();
    save_ids(host, &all_ids)?;
    Ok(Some(id))
}

pub fn remove<H: Host>(host: &H, id: &str) -> Result<(), KvError> {
    let mut remaining = ids(host)?;
    remaining.retain(|existing| existing != id);
    save_ids(host, &remaining)?;
    host.delete(&key(id))
}
