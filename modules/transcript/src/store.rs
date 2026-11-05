//! The module's two tables, as thin helpers over `data.store`, and the posting call.

use serde_json::{json, Map, Value};

use crate::host::Host;

/// Host calls one run may make; the test host enforces it.
#[cfg(test)]
pub const CALLS_PER_RUN: u32 = 64;
/// Posts one run may make; the host refuses a fourth.
pub const POSTS_PER_RUN: usize = 3;

/// A session's row in `sessions`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Session {
    pub session_id: String,
    pub channel_id: String,
    pub started_ms: i64,
    pub stopped_ms: i64,
    pub reason: String,
    pub segments_sent: i64,
    pub last_event_ms: i64,
    /// Transcript posts already made.
    pub posted: i64,
    /// The `_id` of the last segment already posted, so a continuation resumes after it.
    pub cursor: i64,
    /// Speakers in order of first speaking, comma separated, so "Speaker n" means the same all through.
    pub speakers: String,
    pub ended: bool,
}

/// A segment's row in `segments`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub id: i64,
    pub user_id: String,
    pub start_ms: i64,
    pub text: String,
}

pub struct Store<'a, H: Host> {
    host: &'a H,
}

impl<'a, H: Host> Store<'a, H> {
    pub fn new(host: &'a H) -> Self {
        Store { host }
    }

    fn send(&self, request: Value) -> Result<Value, String> {
        self.host.call(&request).map_err(|e| explain(&e))
    }

    fn data(&self, op: &str, table: &str, rest: Value) -> Result<Value, String> {
        let mut request = json!({"capability": "data.store", "op": op, "table": table});
        if let (Some(base), Some(extra)) = (request.as_object_mut(), rest.as_object()) {
            base.extend(extra.clone());
        }
        self.send(request)
    }

    /// Makes the session's row if it is not there and notes an event arrived at `now_ms`.
    pub fn touch_session(&self, session: &str, channel: &str, now_ms: i64) -> Result<(), String> {
        let values = json!({"session_id": session, "channel_id": channel, "last_event_ms": now_ms});
        let zeroed: Map<String, Value> = ["started_ms", "stopped_ms", "segments_sent", "posted", "cursor", "ended"]
            .into_iter()
            .map(|col| (col.to_string(), json!(0)))
            .collect();
        self.data("upsert", "sessions", json!({"values": values, "on": "session_id", "add": zeroed})).map(|_| ())
    }

    pub fn set_started(&self, session: &str, started_ms: i64) -> Result<(), String> {
        self.data("update", "sessions", json!({"values": {"started_ms": started_ms}, "where": [eq("session_id", session)]})).map(|_| ())
    }

    pub fn add_segment(&self, session: &str, segment_id: &str, user: &str, start_ms: i64, end_ms: i64, text: &str) -> Result<(), String> {
        let row = json!({"session_id": session, "segment_id": segment_id, "user_id": user, "start_ms": start_ms, "end_ms": end_ms, "body": text});
        match self.data("insert", "segments", json!({"values": row})) {
            // The same event delivered twice is one segment.
            Err(e) if e.contains("already holds") => Ok(()),
            other => other.map(|_| ()),
        }
    }

    pub fn end_session(&self, session: &str, stopped_ms: i64, reason: &str, segments_sent: i64) -> Result<(), String> {
        let values = json!({"stopped_ms": stopped_ms, "reason": reason, "segments_sent": segments_sent, "ended": 1});
        self.data("update", "sessions", json!({"values": values, "where": [eq("session_id", session)]})).map(|_| ())
    }

    pub fn session(&self, session: &str) -> Result<Option<Session>, String> {
        let reply = self.data("get", "sessions", json!({"where": [eq("session_id", session)]}))?;
        Ok(reply["rows"].get(0).map(parse_session))
    }

    /// Sessions that never ended and have heard nothing since `before_ms`.
    pub fn stale_sessions(&self, before_ms: i64) -> Result<Vec<Session>, String> {
        let filter = json!([{"col": "ended", "op": "eq", "val": 0}, {"col": "last_event_ms", "op": "lt", "val": before_ms}]);
        let reply = self.data("query", "sessions", json!({"where": filter, "limit": 5}))?;
        Ok(reply["rows"].as_array().map(Vec::as_slice).unwrap_or(&[]).iter().map(parse_session).collect())
    }

    /// The segments after `after_id`, in the order they were spoken.
    pub fn segments_after(&self, session: &str, after_id: i64, limit: i64) -> Result<Vec<Segment>, String> {
        let filter = json!([eq("session_id", session), {"col": "_id", "op": "gt", "val": after_id}]);
        let order = json!([{"col": "_id", "dir": "asc"}]);
        let reply = self.data("query", "segments", json!({"where": filter, "order": order, "limit": limit}))?;
        let rows = reply["rows"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        Ok(rows
            .iter()
            .map(|r| Segment {
                id: r["_id"].as_i64().unwrap_or(0),
                user_id: r["user_id"].as_str().unwrap_or("").to_string(),
                start_ms: r["start_ms"].as_i64().unwrap_or(0),
                text: r["body"].as_str().unwrap_or("").to_string(),
            })
            .collect())
    }

    pub fn count_segments(&self, session: &str) -> Result<i64, String> {
        Ok(self.data("count", "segments", json!({"where": [eq("session_id", session)]}))?["count"].as_i64().unwrap_or(0))
    }

    pub fn set_progress(&self, session: &str, posted: i64, cursor: i64, speakers: &str) -> Result<(), String> {
        let values = json!({"posted": posted, "cursor": cursor, "speakers": speakers});
        self.data("update", "sessions", json!({"values": values, "where": [eq("session_id", session)]})).map(|_| ())
    }

    /// Removes sessions that ended before `before_ms` and their segments; returns how many sessions.
    pub fn prune(&self, before_ms: i64) -> Result<i64, String> {
        let old = json!([{"col": "ended", "op": "eq", "val": 1}, {"col": "stopped_ms", "op": "lt", "val": before_ms}]);
        let reply = self.data("query", "sessions", json!({"where": old, "limit": 50}))?;
        let rows = reply["rows"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        let ids: Vec<String> = rows.iter().filter_map(|r| r["session_id"].as_str().map(str::to_string)).collect();
        if ids.is_empty() {
            return Ok(0);
        }
        self.data("delete", "segments", json!({"where": [{"col": "session_id", "op": "in", "val": ids}]}))?;
        self.data("delete", "sessions", json!({"where": [{"col": "session_id", "op": "in", "val": ids}]}))?;
        Ok(ids.len() as i64)
    }

    pub fn last_prune(&self) -> Result<i64, String> {
        let reply = self.data("get", "meta", json!({"where": [eq("name", "last_prune")]}))?;
        Ok(reply["rows"][0]["val"].as_i64().unwrap_or(0))
    }

    pub fn set_last_prune(&self, at_ms: i64) -> Result<(), String> {
        self.data("upsert", "meta", json!({"values": {"name": "last_prune", "val": at_ms}, "on": "name"})).map(|_| ())
    }

    /// Posts to the posting channel the admin chose, as this module.
    pub fn post(&self, content: &str) -> Result<(), String> {
        self.send(json!({"capability": "message.post", "content": content})).map(|_| ())
    }
}

fn eq(col: &str, val: &str) -> Value {
    json!({"col": col, "op": "eq", "val": val})
}

fn parse_session(row: &Value) -> Session {
    let int = |key: &str| row[key].as_i64().unwrap_or(0);
    let text = |key: &str| row[key].as_str().unwrap_or("").to_string();
    Session {
        session_id: text("session_id"),
        channel_id: text("channel_id"),
        started_ms: int("started_ms"),
        stopped_ms: int("stopped_ms"),
        reason: text("reason"),
        segments_sent: int("segments_sent"),
        last_event_ms: int("last_event_ms"),
        posted: int("posted"),
        cursor: int("cursor"),
        speakers: text("speakers"),
        ended: int("ended") == 1,
    }
}

/// Host refusals in words the admin reading the Dock can act on.
fn explain(host_error: &str) -> String {
    if host_error.contains("not approved") {
        "An admin needs to approve Call transcript in the Dock (hearing calls, data, and posting).".to_string()
    } else if host_error.contains("no poster") || host_error.contains("channel") && host_error.contains("post") {
        "Pick a channel for Call transcript to post in, in the Dock.".to_string()
    } else {
        format!("Call transcript could not reach slim ({host_error})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_capability_is_explained_for_the_admin() {
        assert!(explain("capability not approved: message.post").contains("An admin needs to approve"));
        assert!(explain("data.store is unavailable").contains("could not reach slim"));
    }
}
