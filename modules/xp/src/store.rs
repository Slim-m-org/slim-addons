//! The module's tables and the voice log, as thin helpers over `data.store` and `log.read`.
//!
//! A run may make 64 host calls; `Store` counts them so a caller can stop before the host refuses.

use std::cell::Cell;
use std::collections::HashMap;

use serde_json::{json, Value};

use crate::award::Session;
use crate::host::Host;

/// Host calls one run may make, shared by `data.store`, `kv.store` and `log.read`.
pub const CALLS_PER_RUN: u32 = 64;
const CURSOR_KEY: &str = "voice_cursor";

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

    pub fn member(&self, user: &str) -> Result<Option<Member>, String> {
        let reply = self.data("get", "xp", json!({"where": [eq("user_id", user)]}))?;
        Ok(reply["rows"].get(0).map(|row| Member {
            xp: row["xp"].as_i64().unwrap_or(0),
            last_session: row["last_session"].as_i64().unwrap_or(0),
        }))
    }

    /// Makes the member's zeroed row if it is not there; losing a race to another run is fine.
    pub fn ensure_member(&self, user: &str) -> Result<(), String> {
        let zero = json!({"user_id": user, "xp": 0, "voice_ms": 0, "messages": 0, "last_session": 0, "last_msg_ms": 0});
        match self.data("insert", "xp", json!({"values": zero})) {
            Err(e) if e.contains("already holds") => Ok(()),
            other => other.map(|_| ()),
        }
    }

    /// Takes the member's session `id` for awarding; false when that session or a later one was taken already.
    pub fn claim_session(&self, user: &str, id: i64) -> Result<bool, String> {
        let filter = json!([eq("user_id", user), {"col": "last_session", "op": "lt", "val": id}]);
        let reply = self.data("update", "xp", json!({"values": {"last_session": id}, "where": filter}))?;
        Ok(reply["changed"].as_i64().unwrap_or(0) > 0)
    }

    /// Takes the member's message slot at `at_ms`; false while the cooldown from the last award is still running.
    pub fn claim_message(&self, user: &str, at_ms: i64, cooldown_ms: i64) -> Result<bool, String> {
        let earliest = at_ms.saturating_sub(cooldown_ms);
        let filter = json!([eq("user_id", user), {"col": "last_msg_ms", "op": "lte", "val": earliest}]);
        let reply = self.data("update", "xp", json!({"values": {"last_msg_ms": at_ms}, "where": filter}))?;
        Ok(reply["changed"].as_i64().unwrap_or(0) > 0)
    }

    pub fn add(&self, user: &str, col: &str, by: i64) -> Result<(), String> {
        if by == 0 {
            return Ok(());
        }
        self.data("increment", "xp", json!({"col": col, "by": by, "where": [eq("user_id", user)]})).map(|_| ())
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
