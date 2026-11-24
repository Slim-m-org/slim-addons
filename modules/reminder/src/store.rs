//! Pending reminders in `kv.store`: one key per reminder and an index of ids.
//! An id is the due time then a few characters of the run's entropy, so the
//! index sorts by when each is due and two made in one millisecond differ.

use crate::host::{Host, KvError};

const INDEX: &str = "i";
/// Pending reminders a space may hold: 30 at 200 characters stays well inside `kv.store`
/// and under the 32 timers a module may have waiting.
pub const MAX_PENDING: usize = 30;
/// Pending reminders one member may hold, so one member cannot fill the space.
pub const MAX_PER_CALLER: usize = 5;

/// A stored value is this marker, the owner, the channel it was set in and the text, split by `SEP`.
/// A reminder set before owners were kept is the bare text, with no owner and no channel.
const MARK: &str = "v2";
const SEP: char = '\u{1f}';

pub struct Reminder {
    pub id: String,
    pub due_ms: i64,
    pub owner: String,
    pub channel: String,
    pub text: String,
}

fn encode(owner: &str, channel: &str, text: &str) -> String {
    let clean = |s: &str| s.chars().filter(|c| !c.is_control()).collect::<String>();
    format!("{MARK}{SEP}{}{SEP}{}{SEP}{text}", clean(owner), clean(channel))
}

fn decode(id: &str, value: String) -> Reminder {
    let due_ms = due_of(id);
    let mut parts = value.splitn(4, SEP);
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(MARK), Some(owner), Some(channel), Some(text)) => {
            Reminder { id: id.to_string(), due_ms, owner: owner.to_string(), channel: channel.to_string(), text: text.to_string() }
        }
        _ => Reminder { id: id.to_string(), due_ms, owner: String::new(), channel: String::new(), text: value },
    }
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
    Ok(host.get(&key(id))?.map(|value| decode(id, value)))
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

/// Every pending reminder a member set, soonest first.
pub fn of<H: Host>(host: &H, owner: &str) -> Result<Vec<Reminder>, KvError> {
    Ok(all(host)?.into_iter().filter(|r| r.owner == owner).collect())
}

pub struct New<'a> {
    pub due_ms: i64,
    pub salt: &'a str,
    pub owner: &'a str,
    pub channel: &'a str,
    pub text: &'a str,
}

pub enum Added {
    Id(String),
    OwnerFull,
    SpaceFull,
}

/// Refuses when the member or the space already holds its limit of reminders that can still be listed.
/// The index is read again just before it is written, so a change made meanwhile is not overwritten.
pub fn add<H: Host>(host: &H, new: &New) -> Result<Added, KvError> {
    let Pending { live, dead } = pending(host)?;
    if live.iter().filter(|r| r.owner == new.owner).count() >= MAX_PER_CALLER {
        return Ok(Added::OwnerFull);
    }
    if live.len() >= MAX_PENDING {
        return Ok(Added::SpaceFull);
    }
    let (due_ms, text) = (new.due_ms, encode(new.owner, new.channel, new.text));
    let salt = new.salt;
    let salt: String = salt.chars().filter(char::is_ascii_alphanumeric).take(6).collect();
    let id = format!("{due_ms:013}-{salt}");
    host.set(&key(&id), &text)?;
    let mut all_ids = ids(host)?;
    all_ids.retain(|existing| !dead.contains(existing));
    all_ids.push(id.clone());
    all_ids.sort();
    all_ids.dedup();
    save_ids(host, &all_ids)?;
    Ok(Added::Id(id))
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
