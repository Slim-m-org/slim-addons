//! Pending reminders in `kv.store`: one key per reminder, listed from the store's own keys.
//! An id is the due time then a few characters of the run's entropy, so the
//! keys sort by when each is due and two made in one millisecond differ.
//! There is no shared index to rewrite, so runs that overlap cannot lose each other's reminders.

use crate::host::{Host, KvError};

const PREFIX: &str = "r:";
/// Pending reminders a space may hold: 20 at 200 characters stays well inside `kv.store`.
pub const MAX_PENDING: usize = 20;

pub struct Reminder {
    pub id: String,
    pub due_ms: i64,
    pub text: String,
}

fn key(id: &str) -> String {
    format!("{PREFIX}{id}")
}

/// The due time an id carries.
fn due_of(id: &str) -> i64 {
    id.split('-').next().and_then(|d| d.parse().ok()).unwrap_or(0)
}

/// Every pending reminder, soonest first. A key deleted between the listing and the read is skipped.
pub fn all<H: Host>(host: &H) -> Result<Vec<Reminder>, KvError> {
    let mut ids: Vec<String> = host.keys()?.iter().filter_map(|k| k.strip_prefix(PREFIX)).map(str::to_string).collect();
    ids.sort();
    let mut found = Vec::new();
    for id in ids {
        if let Some(text) = host.get(&key(&id))? {
            found.push(Reminder { due_ms: due_of(&id), id, text });
        }
    }
    Ok(found)
}

/// `Ok(None)` when the space already holds [`MAX_PENDING`] reminders.
pub fn add<H: Host>(host: &H, due_ms: i64, salt: &str, text: &str) -> Result<Option<String>, KvError> {
    if all(host)?.len() >= MAX_PENDING {
        return Ok(None);
    }
    let salt: String = salt.chars().filter(char::is_ascii_alphanumeric).take(6).collect();
    let id = format!("{due_ms:013}-{salt}");
    host.set(&key(&id), text)?;
    Ok(Some(id))
}

/// Deletes what each of `gone` stored.
pub fn remove<H: Host>(host: &H, gone: &[String]) -> Result<(), KvError> {
    gone.iter().try_for_each(|id| host.delete(&key(id)))
}
