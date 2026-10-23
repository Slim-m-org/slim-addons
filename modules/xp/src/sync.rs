//! Awarding XP: a push is only a trigger, the voice log is the truth.
//!
//! Every pass reads the closed sessions after the cursor, awards them, and moves the cursor
//! only once the writes succeeded. A session is awarded at most once even if the cursor
//! write is lost, because each member remembers the last session id taken for them.

use serde_json::Value;

use crate::award::{session_xp, Session};
use crate::host::Host;
use crate::settings::Settings;
use crate::store::Store;

/// Rows one `log.read` asks for; the host caps it at 200.
const PAGE: i64 = 200;
/// Calls a member costs at most: read, create, claim, and two increments.
const CALLS_PER_MEMBER: u32 = 5;
/// Calls a pass spends besides the members: the log read and the cursor write.
const CALLS_PER_PASS: u32 = 2;
/// A total never grows by more than this in one write; the host refuses larger increments.
const MAX_INCREMENT: i64 = 1 << 53;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub sessions: usize,
    pub cursor: i64,
}

/// Reads everything after the cursor and awards it, as many passes as this run's call budget allows.
pub fn catch_up<H: Host>(store: &Store<H>, settings: &Settings) -> Result<Report, String> {
    let mut report = Report { cursor: store.cursor()?, ..Report::default() };
    loop {
        let members = store.calls_left().saturating_sub(CALLS_PER_PASS) / CALLS_PER_MEMBER;
        if members == 0 {
            break;
        }
        let (rows, next) = store.sessions_after(report.cursor, PAGE)?;
        if rows.is_empty() {
            break;
        }
        let (batch, resume) = take_members(&rows, members as usize, next);
        for (user, sessions) in group_by_member(batch) {
            award_member(store, settings, &user, &sessions)?;
        }
        store.set_cursor(resume)?;
        report.sessions += batch.len();
        report.cursor = resume;
        if resume == next && rows.len() < PAGE as usize {
            break;
        }
    }
    Ok(report)
}

/// The leading rows that touch at most `members` members, and the cursor that follows them.
fn take_members(rows: &[Session], members: usize, next: i64) -> (&[Session], i64) {
    let mut seen: Vec<&str> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        if !seen.contains(&row.user_id.as_str()) {
            if seen.len() == members {
                return (&rows[..i], rows[i - 1].id);
            }
            seen.push(&row.user_id);
        }
    }
    (rows, next)
}

fn group_by_member(rows: &[Session]) -> Vec<(String, Vec<&Session>)> {
    let mut groups: Vec<(String, Vec<&Session>)> = Vec::new();
    for row in rows {
        match groups.iter_mut().find(|(user, _)| *user == row.user_id) {
            Some((_, sessions)) => sessions.push(row),
            None => groups.push((row.user_id.clone(), vec![row])),
        }
    }
    groups
}

fn award_member<H: Host>(store: &Store<H>, settings: &Settings, user: &str, sessions: &[&Session]) -> Result<(), String> {
    let taken = match store.member(user)? {
        Some(member) => member.last_session,
        None => {
            store.ensure_member(user)?;
            0
        }
    };
    let fresh: Vec<&&Session> = sessions.iter().filter(|s| s.id > taken).collect();
    let Some(top) = fresh.iter().map(|s| s.id).max() else {
        return Ok(());
    };
    if !store.claim_session(user, top)? {
        return Ok(());
    }
    let xp: i64 = fresh.iter().fold(0, |sum, s| sum.saturating_add(session_xp(s, settings)));
    let voice_ms: i64 = fresh.iter().fold(0, |sum, s| sum.saturating_add(s.duration_ms()));
    store.add(user, "xp", xp.min(MAX_INCREMENT))?;
    store.add(user, "voice_ms", voice_ms.min(MAX_INCREMENT))
}

/// The fields of a `message.created` event the award needs.
pub struct Message<'a> {
    pub author_id: &'a str,
    pub created_at_ms: i64,
    pub is_bot: bool,
}

impl<'a> Message<'a> {
    pub fn from_event(data: &'a Value) -> Option<Message<'a>> {
        Some(Message {
            author_id: data["author_id"].as_str()?,
            created_at_ms: data["created_at_ms"].as_i64()?,
            is_bot: data["is_bot"].as_bool().unwrap_or(false),
        })
    }
}

/// Awards a message once per cooldown per member; true when it paid.
pub fn award_message<H: Host>(store: &Store<H>, settings: &Settings, message: &Message) -> Result<bool, String> {
    if message.is_bot || settings.xp_per_message == 0 {
        return Ok(false);
    }
    store.ensure_member(message.author_id)?;
    let cooldown_ms = settings.message_cooldown_s.saturating_mul(1000);
    if !store.claim_message(message.author_id, message.created_at_ms, cooldown_ms)? {
        return Ok(false);
    }
    store.add(message.author_id, "xp", settings.xp_per_message)?;
    store.add(message.author_id, "messages", 1)?;
    Ok(true)
}
