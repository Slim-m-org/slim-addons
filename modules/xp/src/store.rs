//! The module's tables and the voice log, as thin helpers over `data.store` and `log.read`.
//!
//! A run may make 64 host calls; `Store` counts them so a caller can stop before the host refuses.

use std::cell::Cell;
use std::collections::HashMap;

use serde_json::{json, Map, Value};

use crate::award::Session;
use crate::host::Host;

/// Host calls one run may make, shared by `data.store`, `kv.store` and `log.read`.
pub const CALLS_PER_RUN: u32 = 64;
const CURSOR_KEY: &str = "voice_cursor";
/// The counters a new member row starts at.
const ZEROED: [&str; 5] = ["xp", "voice_ms", "messages", "last_session", "last_msg_ms"];

/// One member's row in the `xp` table.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Member {
    pub xp: i64,
    pub last_session: i64,
}

pub struct Store<'a, H: Host> {
    host: &'a H,
    used: Cell<u32>,
}

impl<'a, H: Host> Store<'a, H> {
    pub fn new(host: &'a H) -> Self {
        Store { host, used: Cell::new(0) }
    }

    pub fn calls_left(&self) -> u32 {
        CALLS_PER_RUN.saturating_sub(self.used.get())
    }

    fn send(&self, request: Value) -> Result<Value, String> {
        self.used.set(self.used.get() + 1);
        self.host.call(&request).map_err(|e| explain(&e))
    }

    fn data(&self, op: &str, table: &str, rest: Value) -> Result<Value, String> {
        let mut request = json!({"capability": "data.store", "op": op, "table": table});
        if let (Some(base), Some(extra)) = (request.as_object_mut(), rest.as_object()) {
            base.extend(extra.clone());
        }
        self.send(request)
    }

    pub fn cursor(&self) -> Result<i64, String> {
        let reply = self.data("get", "meta", json!({"where": [{"col": "name", "op": "eq", "val": CURSOR_KEY}]}))?;
        Ok(reply["rows"][0]["val"].as_i64().unwrap_or(0))
    }

    pub fn set_cursor(&self, value: i64) -> Result<(), String> {
        let values = json!({"name": CURSOR_KEY, "val": value});
        self.data("upsert", "meta", json!({"values": values, "on": "name"})).map(|_| ())
    }

    /// Closed sessions after `cursor`, in id order, and the cursor to continue from.
    pub fn sessions_after(&self, cursor: i64, limit: i64) -> Result<(Vec<Session>, i64), String> {
        let reply = self.send(json!({"capability": "log.read", "log": "voice.sessions", "after": cursor, "limit": limit}))?;
        let rows = reply["rows"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        let sessions = rows.iter().filter_map(parse_session).collect();
        Ok((sessions, reply["next"].as_i64().unwrap_or(cursor)))
    }

    /// The rows of the members that have one, in one call.
    pub fn members(&self, users: &[String]) -> Result<HashMap<String, Member>, String> {
        let reply = self.data("query", "xp", json!({"where": [{"col": "user_id", "op": "in", "val": users}], "limit": 200}))?;
        let rows = reply["rows"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        Ok(rows
            .iter()
            .filter_map(|row| {
                let member = Member { xp: row["xp"].as_i64()?, last_session: row["last_session"].as_i64().unwrap_or(0) };
                Some((row["user_id"].as_str()?.to_string(), member))
            })
            .collect())
    }

    /// Makes zeroed rows for members known to have none, in one call. A run that made one first
    /// refuses the whole batch, so those are made one by one instead.
    pub fn create_members(&self, users: &[String]) -> Result<(), String> {
        if users.is_empty() {
            return Ok(());
        }
        let rows: Vec<Value> = users.iter().map(|user| zero_row(user)).collect();
        match self.data("insert", "xp", json!({"rows": rows})) {
            Err(e) if e.contains("already holds") => users.iter().try_for_each(|user| self.ensure_member(user)),
            other => other.map(|_| ()),
        }
    }

    /// Makes the member's zeroed row if it is not there: an upsert that adds nothing to a row that exists.
    pub fn ensure_member(&self, user: &str) -> Result<(), String> {
        let add: Map<String, Value> = ZEROED.iter().map(|col| (col.to_string(), json!(0))).collect();
        self.data("upsert", "xp", json!({"values": {"user_id": user}, "on": "user_id", "add": add})).map(|_| ())
    }

    /// Takes the member's session `id` and pays for it in one statement; false when that session or a later one was taken already.
    pub fn pay_session(&self, user: &str, id: i64, xp: i64, voice_ms: i64) -> Result<bool, String> {
        let filter = json!([eq("user_id", user), {"col": "last_session", "op": "lt", "val": id}]);
        let request = json!({"values": {"last_session": id}, "add": {"xp": xp, "voice_ms": voice_ms}, "where": filter});
        Ok(self.data("update", "xp", request)?["changed"].as_i64().unwrap_or(0) > 0)
    }

    /// Takes the member's message slot at `at_ms` and pays for it in one statement; false while the cooldown from the last award is still running.
    pub fn pay_message(&self, user: &str, at_ms: i64, cooldown_ms: i64, xp: i64) -> Result<bool, String> {
        let earliest = at_ms.saturating_sub(cooldown_ms);
        let filter = json!([eq("user_id", user), {"col": "last_msg_ms", "op": "lte", "val": earliest}]);
        let request = json!({"values": {"last_msg_ms": at_ms}, "add": {"xp": xp, "messages": 1}, "where": filter});
        Ok(self.data("update", "xp", request)?["changed"].as_i64().unwrap_or(0) > 0)
    }

    /// XP totals for the members that have a row.
    pub fn totals(&self, users: &[String]) -> Result<HashMap<String, i64>, String> {
        let filter = json!([{"col": "user_id", "op": "in", "val": users}]);
        let reply = self.data("query", "xp", json!({"where": filter, "limit": 100}))?;
        let rows = reply["rows"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        Ok(rows
            .iter()
            .filter_map(|row| Some((row["user_id"].as_str()?.to_string(), row["xp"].as_i64()?)))
            .collect())
    }
}

fn zero_row(user: &str) -> Value {
    json!({"user_id": user, "xp": 0, "voice_ms": 0, "messages": 0, "last_session": 0, "last_msg_ms": 0})
}

fn eq(col: &str, val: &str) -> Value {
    json!({"col": col, "op": "eq", "val": val})
}

fn parse_session(row: &Value) -> Option<Session> {
    Some(Session {
        id: row["id"].as_i64()?,
        user_id: row["user_id"].as_str()?.to_string(),
        joined_at_ms: row["joined_at_ms"].as_i64()?,
        left_at_ms: row["left_at_ms"].as_i64()?,
        alone_ms: row["alone_ms"].as_i64().unwrap_or(0),
    })
}

/// Host refusals in words the admin reading the Dock can act on.
fn explain(host_error: &str) -> String {
    if host_error.contains("not approved") {
        "An admin needs to approve XP and levels in the Dock (events, data, and call history).".to_string()
    } else {
        format!("XP and levels could not reach slim ({host_error})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_capability_is_explained_for_the_admin() {
        assert!(explain("capability not approved: data.store").contains("An admin needs to approve"));
        assert!(explain("data.store is unavailable").contains("could not reach slim"));
    }

    #[test]
    fn a_row_without_a_left_time_is_not_a_session() {
        assert_eq!(parse_session(&json!({"id": 1, "user_id": "u", "joined_at_ms": 1})), None);
    }
}
