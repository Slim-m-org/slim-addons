//! `log.read` for the runner: durable logs served from `--log-file`, a JSON
//! object of log name to rows in the order they closed. The cursor is the
//! 1-based position in that order, as slim's is a close-order cursor and not a row `id`.

use std::collections::BTreeMap;

use serde_json::{json, Value};

const DEFAULT_ROWS: i64 = 100;
const MAX_ROWS: i64 = 200;

#[derive(Default)]
pub struct Logs(BTreeMap<String, Vec<Value>>);

impl Logs {
    pub fn parse(text: &str) -> Result<Logs, String> {
        let Value::Object(map) = serde_json::from_str::<Value>(text).map_err(|e| format!("log file is not JSON: {e}"))? else {
            return Err("log file must be a JSON object of log name to rows".to_owned());
        };
        let mut logs = BTreeMap::new();
        for (name, rows) in map {
            let rows = rows.as_array().ok_or_else(|| format!("log {name} must be a list of rows"))?;
            logs.insert(name, rows.clone());
        }
        Ok(Logs(logs))
    }

    pub fn handle(&self, request: &Value) -> Value {
        let Some(name) = request["log"].as_str() else { return refusal("log.read needs a log") };
        let Some(rows) = self.0.get(name) else { return refusal(&format!("unknown log: {name}")) };
        let after = request["after"].as_i64().unwrap_or(0).max(0);
        let limit = request["limit"].as_i64().unwrap_or(DEFAULT_ROWS).clamp(1, MAX_ROWS);
        let page: Vec<&Value> = rows.iter().skip(after as usize).take(limit as usize).collect();
        let next = after + page.len() as i64;
        json!({"ok": true, "rows": page, "next": next})
    }
}

pub fn refusal(message: &str) -> Value {
    json!({"ok": false, "error": message})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn logs() -> Logs {
        let rows: Vec<Value> = (1..=250).map(|i| json!({"id": 1000 + i})).collect();
        Logs(BTreeMap::from([("voice.sessions".to_owned(), rows)]))
    }

    fn shape(reply: &Value) -> (Option<usize>, Option<i64>) {
        (reply["rows"].as_array().map(Vec::len), reply["next"].as_i64())
    }

    #[test]
    fn a_page_is_capped_and_the_cursor_moves_by_position_not_id() {
        assert_eq!(shape(&logs().handle(&json!({"log": "voice.sessions", "limit": 100000}))), (Some(200), Some(200)));
        assert_eq!(shape(&logs().handle(&json!({"log": "voice.sessions", "after": 200}))), (Some(50), Some(250)));
        assert_eq!(shape(&logs().handle(&json!({"log": "voice.sessions", "after": 250}))), (Some(0), Some(250)));
    }

    #[test]
    fn an_unnamed_or_unknown_log_is_refused() {
        assert!(logs().handle(&json!({})).to_string().contains("needs a log"));
        assert!(logs().handle(&json!({"log": "nope"})).to_string().contains("unknown log: nope"));
    }
}
