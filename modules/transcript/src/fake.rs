//! An in-memory slim for tests: `data.store` over the module's three tables with the host's unique
//! columns and call budget, and `message.post` with its three-per-run limit.

use std::cell::{Cell, RefCell};

use serde_json::{json, Map, Value};

use crate::host::Host;
use crate::store::{CALLS_PER_RUN, POSTS_PER_RUN};

pub type Row = Map<String, Value>;

#[derive(Default)]
pub struct FakeHost {
    pub sessions: RefCell<Vec<Row>>,
    pub segments: RefCell<Vec<Row>>,
    pub meta: RefCell<Vec<Row>>,
    pub posts: RefCell<Vec<String>>,
    pub calls: Cell<u32>,
    posts_this_run: Cell<usize>,
    next_id: Cell<i64>,
    /// Refuse every post, as a module with no posting channel is.
    pub no_poster: Cell<bool>,
}

impl FakeHost {
    /// Starts a new run: the budgets reset, the data stays.
    pub fn next_run(&self) {
        self.calls.set(0);
        self.posts_this_run.set(0);
    }

    fn take_id(&self) -> i64 {
        self.next_id.set(self.next_id.get() + 1);
        self.next_id.get()
    }

    pub fn session(&self, id: &str) -> Row {
        self.sessions.borrow().iter().find(|r| r["session_id"] == id).cloned().expect("a session row")
    }
}

fn matches(row: &Row, filters: &Value) -> bool {
    filters.as_array().map_or(true, |all| {
        all.iter().all(|f| {
            let (cell, val) = (&row[f["col"].as_str().unwrap()], &f["val"]);
            match f["op"].as_str().unwrap() {
                "eq" => cell == val,
                "lt" => cell.as_i64() < val.as_i64(),
                "lte" => cell.as_i64() <= val.as_i64(),
                "gt" => cell.as_i64() > val.as_i64(),
                "in" => val.as_array().unwrap().contains(cell),
                other => panic!("the fake does not do {other}"),
            }
        })
    })
}

fn add_to(row: &mut Row, add: &Map<String, Value>) {
    for (col, by) in add {
        let now = row.get(col).and_then(Value::as_i64).unwrap_or(0);
        row.insert(col.clone(), json!(now + by.as_i64().unwrap()));
    }
}

fn refusal(message: &str) -> Result<Value, String> {
    Err(message.to_string())
}

impl FakeHost {
    fn data_store(&self, request: &Value) -> Result<Value, String> {
        let (table, unique) = match request["table"].as_str().unwrap() {
            "sessions" => (&self.sessions, "session_id"),
            "segments" => (&self.segments, "segment_id"),
            _ => (&self.meta, "name"),
        };
        let mut rows = table.borrow_mut();
        let filters = &request["where"];
        let (mut changed, mut count) = (0, 0);
        let mut out: Vec<Row> = Vec::new();
        match request["op"].as_str().unwrap() {
            "insert" => {
                let mut row = request["values"].as_object().unwrap().clone();
                row.insert("_id".into(), json!(self.take_id()));
                if rows.iter().any(|r| r[unique] == row[unique]) {
                    return refusal("a unique column already holds that value");
                }
                rows.push(row);
                changed = 1;
            }
            "upsert" => {
                let values = request["values"].as_object().unwrap().clone();
                assert_eq!(request["on"], unique, "the fake upserts on the table's one unique column");
                let add = request.get("add").and_then(Value::as_object).cloned().unwrap_or_default();
                assert!(add.keys().all(|c| !values.contains_key(c)), "a column cannot be in both values and add");
                match rows.iter_mut().find(|r| r[unique] == values[unique]) {
                    Some(existing) => existing.extend(values),
                    None => {
                        let mut fresh = values;
                        fresh.insert("_id".into(), json!(self.take_id()));
                        add_to(&mut fresh, &add);
                        rows.push(fresh);
                    }
                }
                changed = 1;
            }
            "update" => {
                let values = request.get("values").and_then(Value::as_object).cloned().unwrap_or_default();
                for r in rows.iter_mut().filter(|r| matches(r, filters)) {
                    r.extend(values.clone());
                    changed += 1;
                }
            }
            "delete" => {
                let before = rows.len();
                rows.retain(|r| !matches(r, filters));
                changed = before - rows.len();
            }
            "count" => count = rows.iter().filter(|r| matches(r, filters)).count(),
            "get" | "query" => {
                out = rows.iter().filter(|r| matches(r, filters)).cloned().collect();
                if request["order"].is_array() {
                    out.sort_by_key(|r| r["_id"].as_i64().unwrap());
                }
                out.truncate(if request["op"] == "get" { 1 } else { request["limit"].as_u64().unwrap_or(200) as usize });
            }
            other => return refusal(&format!("the fake does not do {other}")),
        }
        Ok(json!({"ok": true, "rows": out, "changed": changed, "count": if count > 0 { count } else { out.len() }}))
    }
}

impl Host for FakeHost {
    fn call(&self, request: &Value) -> Result<Value, String> {
        self.calls.set(self.calls.get() + 1);
        if self.calls.get() > CALLS_PER_RUN {
            return refusal("call budget for this run is exhausted");
        }
        match request["capability"].as_str() {
            Some("data.store") => self.data_store(request),
            Some("message.post") => {
                if self.no_poster.get() {
                    return refusal("this module has no poster here");
                }
                if self.posts_this_run.get() >= POSTS_PER_RUN {
                    return refusal("message.post budget for this run is exhausted");
                }
                self.posts_this_run.set(self.posts_this_run.get() + 1);
                self.posts.borrow_mut().push(request["content"].as_str().unwrap().to_string());
                Ok(json!({"ok": true, "message_id": "m"}))
            }
            _ => refusal("capability not available"),
        }
    }
}
