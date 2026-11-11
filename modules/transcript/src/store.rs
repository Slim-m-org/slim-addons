//! The module's three tables and the one post call, as thin helpers over `data.store` and `message.post`.
//!
//! A run may make 64 data calls and 3 posts; `Store` counts both so a caller stops before the host refuses.

use std::cell::Cell;
use std::collections::HashMap;

use serde_json::{json, Value};

use crate::host::Host;

pub const DATA_CALLS_PER_RUN: u32 = 64;
pub const POSTS_PER_RUN: u32 = 3;
/// A page of rows is at most this many; the host refuses a bigger `limit`.
pub const PAGE: i64 = 200;

/// A call the module was told about, with how far its transcript has been posted.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Call {
    pub session_id: String,
    pub started_ms: i64,
    pub stopped_ms: i64,
    pub last_ms: i64,
    pub state: String,
    pub reason: String,
    pub sent: i64,
    pub seen: i64,
    pub dropped: i64,
    pub speakers: i64,
    pub posted: i64,
    pub parts: i64,
}

/// A person in a call: the number they hold once they spoke, and the name the host sent, if it sent one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Person {
    pub ord: i64,
    pub name: String,
}

/// One stored utterance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub user_id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub body: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PostError {
    /// Not approved, or no channel was picked for the module: the transcript waits.
    Unavailable,
    Refused(String),
}

pub struct Store<'a, H: Host> {
    host: &'a H,
    data_used: Cell<u32>,
    posts_used: Cell<u32>,
}

impl<'a, H: Host> Store<'a, H> {
    pub fn new(host: &'a H) -> Self {
        Store { host, data_used: Cell::new(0), posts_used: Cell::new(0) }
    }

    pub fn data_left(&self) -> u32 {
        DATA_CALLS_PER_RUN.saturating_sub(self.data_used.get())
    }

    pub fn posts_left(&self) -> u32 {
        POSTS_PER_RUN.saturating_sub(self.posts_used.get())
    }

    fn data(&self, op: &str, table: &str, rest: Value) -> Result<Value, String> {
        self.data_used.set(self.data_used.get() + 1);
        let mut request = json!({"capability": "data.store", "op": op, "table": table});
        if let (Some(base), Some(extra)) = (request.as_object_mut(), rest.as_object()) {
            base.extend(extra.clone());
        }
        self.host.call(&request)
    }

    pub fn post(&self, content: &str) -> Result<(), PostError> {
        self.posts_used.set(self.posts_used.get() + 1);
        match self.host.call(&json!({"capability": "message.post", "content": content})) {
            Ok(_) => Ok(()),
            Err(e) if ["not approved", "needs a channel", "not available"].iter().any(|m| e.contains(m)) => Err(PostError::Unavailable),
            Err(e) => Err(PostError::Refused(e)),
        }
    }

    /// Records a call; false when the host already holds one with this session id.
    pub fn insert_call(&self, session_id: &str, started_ms: i64) -> Result<bool, String> {
        let values = json!({
            "session_id": session_id, "started_ms": started_ms, "stopped_ms": 0, "last_ms": started_ms, "state": "on",
            "reason": "", "sent": -1, "seen": 0, "dropped": 0, "speakers": 0, "posted": 0, "parts": 0
        });
        match self.data("insert", "calls", json!({"values": values})) {
            Ok(_) => Ok(true),
            Err(e) if e.contains("unique") => Ok(false),
            Err(e) => Err(e),
        }
    }

    pub fn call(&self, session_id: &str) -> Result<Option<Call>, String> {
        let reply = self.data("get", "calls", json!({"where": [eq("session_id", session_id)]}))?;
        Ok(rows(&reply).first().map(parse_call))
    }

    pub fn calls_in_state(&self, state: &str, order: &str, limit: i64) -> Result<Vec<Call>, String> {
        let order = json!([{"col": "started_ms", "dir": order}]);
        let reply = self.data("query", "calls", json!({"where": [eq("state", state)], "order": order, "limit": limit}))?;
        Ok(rows(&reply).iter().map(parse_call).collect())
    }

    /// Calls in `state` that nothing has touched since `before_ms`.
    pub fn calls_idle_since(&self, state: &str, before_ms: i64, limit: i64) -> Result<Vec<Call>, String> {
        let filters = json!([eq("state", state), {"col": "last_ms", "op": "lt", "val": before_ms}]);
        let reply = self.data("query", "calls", json!({"where": filters, "limit": limit}))?;
        Ok(rows(&reply).iter().map(parse_call).collect())
    }

    /// Finished calls (posted or not) that nothing has touched since `before_ms`.
    pub fn finished_calls_idle_since(&self, before_ms: i64, limit: i64) -> Result<Vec<Call>, String> {
        let filters = json!([{"col": "state", "op": "ne", "val": "on"}, {"col": "last_ms", "op": "lt", "val": before_ms}]);
        let reply = self.data("query", "calls", json!({"where": filters, "limit": limit}))?;
        Ok(rows(&reply).iter().map(parse_call).collect())
    }

    /// Moves a call from one state to the next; false when another run already did.
    pub fn move_call(&self, session_id: &str, (from, to): (&str, &str), values: Value) -> Result<bool, String> {
        let mut set = values.as_object().cloned().unwrap_or_default();
        set.insert("state".to_string(), json!(to));
        let reply = self.data("update", "calls", json!({"values": set, "where": [eq("session_id", session_id), eq("state", from)]}))?;
        Ok(reply["changed"].as_i64().unwrap_or(0) == 1)
    }

    pub fn bump_call(&self, session_id: &str, values: Value, add: Value) -> Result<(), String> {
        self.data("update", "calls", json!({"values": values, "add": add, "where": [eq("session_id", session_id)]})).map(|_| ())
    }

    /// Takes the next `lines` of the transcript for this run; false when another run took them first.
    pub fn claim_lines(&self, session_id: &str, (from, lines): (i64, i64)) -> Result<bool, String> {
        let filters = json!([eq("session_id", session_id), {"col": "posted", "op": "eq", "val": from}]);
        let reply = self.data("update", "calls", json!({"values": {"posted": from + lines}, "add": {"parts": 1}, "where": filters}))?;
        Ok(reply["changed"].as_i64().unwrap_or(0) == 1)
    }

    /// Gives a claim back after the post failed, so the next run posts those lines.
    pub fn release_lines(&self, session_id: &str, (from, lines): (i64, i64)) -> Result<(), String> {
        let filters = json!([eq("session_id", session_id), {"col": "posted", "op": "eq", "val": from + lines}]);
        self.data("update", "calls", json!({"values": {"posted": from}, "add": {"parts": -1}, "where": filters})).map(|_| ())
    }

    pub fn delete_call(&self, session_id: &str) -> Result<(), String> {
        for table in ["lines", "people", "calls"] {
            self.data("delete", table, json!({"where": [eq("session_id", session_id)]}))?;
        }
        Ok(())
    }

    /// Adds people to a call with the names the host gave them, at most 50 to a host call.
    pub fn insert_people(&self, session_id: &str, users: &[(String, String)]) -> Result<(), String> {
        for batch in users.chunks(50) {
            let rows: Vec<Value> = batch.iter().map(|(user, name)| person(session_id, (user, name), true)).collect();
            self.data("insert", "people", json!({"rows": rows}))?;
        }
        Ok(())
    }

    pub fn set_included(&self, session_id: &str, (user, name): (&str, &str), included: bool) -> Result<(), String> {
        self.data("upsert", "people", json!({"values": person(session_id, (user, name), included), "on": "pkey"})).map(|_| ())
    }

    /// Whether a person is in the call's agreed set and the speaker number they hold; `None` when they were never in it.
    pub fn speaker(&self, session_id: &str, user: &str) -> Result<Option<(bool, i64)>, String> {
        let reply = self.data("get", "people", json!({"where": [eq("pkey", &format!("{session_id}:{user}"))]}))?;
        Ok(rows(&reply).first().map(|r| (r["included"].as_bool().unwrap_or(false), r["ord"].as_i64().unwrap_or(0))))
    }

    pub fn set_speaker(&self, session_id: &str, user: &str, ord: i64) -> Result<(), String> {
        let filters = json!([eq("pkey", &format!("{session_id}:{user}"))]);
        self.data("update", "people", json!({"values": {"ord": ord}, "where": filters})).map(|_| ())
    }

    /// Speaker numbers and names by user id; number zero for someone who never spoke, an empty name when none was sent.
    pub fn speakers(&self, session_id: &str) -> Result<HashMap<String, Person>, String> {
        let reply = self.data("query", "people", json!({"where": [eq("session_id", session_id)], "limit": PAGE}))?;
        Ok(rows(&reply)
            .iter()
            .filter_map(|r| {
                let person = Person { ord: r["ord"].as_i64().unwrap_or(0), name: r["name"].as_str().unwrap_or("").to_string() };
                Some((r["user_id"].as_str()?.to_string(), person))
            })
            .collect())
    }

    /// False when the host already holds this segment.
    pub fn insert_line(&self, (session_id, segment_id): (&str, &str), line: &Line) -> Result<bool, String> {
        let values = json!({
            "segment_id": segment_id, "session_id": session_id, "user_id": line.user_id,
            "start_ms": line.start_ms, "end_ms": line.end_ms, "body": line.body
        });
        match self.data("insert", "lines", json!({"values": values})) {
            Ok(_) => Ok(true),
            Err(e) if e.contains("unique") => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// A page of a call's lines in time order, starting `offset` lines in.
    pub fn lines(&self, session_id: &str, offset: i64) -> Result<Vec<Line>, String> {
        let order = json!([{"col": "start_ms", "dir": "asc"}, {"col": "_id", "dir": "asc"}]);
        let reply = self.data("query", "lines", json!({"where": [eq("session_id", session_id)], "order": order, "limit": PAGE, "offset": offset}))?;
        Ok(rows(&reply).iter().map(parse_line).collect())
    }
}

fn eq(col: &str, val: &str) -> Value {
    json!({"col": col, "op": "eq", "val": val})
}

/// A name is kept only when one was sent, so a later event without it never blanks a name already held.
fn person(session_id: &str, (user, name): (&str, &str), included: bool) -> Value {
    let mut row = json!({"pkey": format!("{session_id}:{user}"), "session_id": session_id, "user_id": user, "included": included});
    if !name.is_empty() {
        row["name"] = json!(name);
    }
    row
}

fn rows(reply: &Value) -> &[Value] {
    reply["rows"].as_array().map(Vec::as_slice).unwrap_or(&[])
}

fn parse_call(row: &Value) -> Call {
    let int = |col: &str| row[col].as_i64().unwrap_or(0);
    let text = |col: &str| row[col].as_str().unwrap_or("").to_string();
    Call {
        session_id: text("session_id"),
        started_ms: int("started_ms"),
        stopped_ms: int("stopped_ms"),
        last_ms: int("last_ms"),
        state: text("state"),
        reason: text("reason"),
        sent: row["sent"].as_i64().unwrap_or(-1),
        seen: int("seen"),
        dropped: int("dropped"),
        speakers: int("speakers"),
        posted: int("posted"),
        parts: int("parts"),
    }
}

fn parse_line(row: &Value) -> Line {
    Line {
        user_id: row["user_id"].as_str().unwrap_or("").to_string(),
        start_ms: row["start_ms"].as_i64().unwrap_or(0),
        end_ms: row["end_ms"].as_i64().unwrap_or(0),
        body: row["body"].as_str().unwrap_or("").to_string(),
    }
}
