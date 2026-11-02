//! An in-memory slim for tests: it answers `data.store` and `log.read` the way the host does,
//! including the unique columns, the 64-call budget and the 200-row log page.

use std::cell::{Cell, RefCell};

use serde_json::{json, Map, Value};

use crate::host::Host;
use crate::store::CALLS_PER_RUN;

type Row = Map<String, Value>;

#[derive(Default)]
pub struct FakeHost {
    pub xp: RefCell<Vec<Row>>,
    pub meta: RefCell<Vec<Row>>,
    pub log: RefCell<Vec<Value>>,
    pub calls: Cell<u32>,
    /// Refuse every call whose request has this `op` (or `capability`), once armed.
    pub fail_op: RefCell<Option<String>>,
}

impl FakeHost {
    pub fn with_xp(members: &[(&str, i64)]) -> FakeHost {
        let host = FakeHost::default();
        for (user, xp) in members {
            host.xp.borrow_mut().push(row(user, *xp));
        }
        host
    }

    /// Starts a new run: the call budget resets, the data stays.
    pub fn next_run(&self) {
        self.calls.set(0);
    }

    pub fn push_session(&self, id: i64, user: &str, joined: i64, left: i64, alone: i64) {
        self.log.borrow_mut().push(json!({
            "id": id, "user_id": user, "channel_id": "c", "joined_at_ms": joined, "left_at_ms": left, "alone_ms": alone
        }));
    }

    pub fn xp_of(&self, user: &str) -> i64 {
        self.xp.borrow().iter().find(|r| r["user_id"] == user).map_or(0, |r| r["xp"].as_i64().unwrap())
    }

    pub fn field(&self, user: &str, col: &str) -> i64 {
        self.xp.borrow().iter().find(|r| r["user_id"] == user).map_or(0, |r| r[col].as_i64().unwrap())
    }

    pub fn cursor(&self) -> i64 {
        self.meta.borrow().iter().find(|r| r["name"] == "voice_cursor").map_or(0, |r| r["val"].as_i64().unwrap())
    }
}

fn row(user: &str, xp: i64) -> Row {
    let v = json!({"_id": 0, "user_id": user, "xp": xp, "voice_ms": 0, "messages": 0, "last_session": 0, "last_msg_ms": 0});
    v.as_object().unwrap().clone()
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
    fn log_read(&self, request: &Value) -> Result<Value, String> {
        let after = request["after"].as_i64().unwrap_or(0);
        let limit = request["limit"].as_i64().unwrap_or(100).clamp(1, 200) as usize;
        let rows: Vec<Value> =
            self.log.borrow().iter().filter(|r| r["id"].as_i64().unwrap() > after).take(limit).cloned().collect();
        let next = rows.last().map_or(after, |r| r["id"].as_i64().unwrap());
        Ok(json!({"ok": true, "rows": rows, "next": next}))
    }

    fn data_store(&self, request: &Value) -> Result<Value, String> {
        let table = if request["table"] == "xp" { &self.xp } else { &self.meta };
        let unique = if request["table"] == "xp" { "user_id" } else { "name" };
        let mut rows = table.borrow_mut();
        let filters = &request["where"];
        let mut changed = 0;
        let mut out: Vec<Row> = Vec::new();
        match request["op"].as_str().unwrap() {
            "insert" => {
                let batch: Vec<Row> = match request.get("rows") {
                    Some(rows) => rows.as_array().unwrap().iter().map(|r| r.as_object().unwrap().clone()).collect(),
                    None => vec![request["values"].as_object().unwrap().clone()],
                };
                assert!(batch.len() <= 50, "the host takes at most 50 rows in one insert");
                let clash = |row: &Row| rows.iter().any(|r| r[unique] == row[unique]);
                if batch.iter().any(clash) || (1..batch.len()).any(|i| batch[..i].iter().any(|p| p[unique] == batch[i][unique])) {
                    return refusal("a unique column already holds that value");
                }
                changed = batch.len();
                rows.extend(batch);
            }
            "upsert" => {
                let values = request["values"].as_object().unwrap().clone();
                assert_eq!(request["on"], unique, "the fake upserts on the table's one unique column");
                let add = request.get("add").and_then(Value::as_object).cloned().unwrap_or_default();
                assert!(add.keys().all(|c| !values.contains_key(c)), "a column cannot be in both values and add");
                match rows.iter_mut().find(|r| r[unique] == values[unique]) {
                    Some(existing) => {
                        existing.extend(values);
                        add_to(existing, &add);
                    }
                    None => {
                        let mut fresh = values;
                        add_to(&mut fresh, &add);
                        rows.push(fresh);
                    }
                }
                changed = 1;
            }
            "update" => {
                let values = request.get("values").and_then(Value::as_object).cloned().unwrap_or_default();
                let add = request.get("add").and_then(Value::as_object).cloned().unwrap_or_default();
                assert!(add.keys().all(|c| !values.contains_key(c)), "a column cannot be in both values and add");
                for r in rows.iter_mut().filter(|r| matches(r, filters)) {
                    r.extend(values.clone());
                    add_to(r, &add);
                    changed += 1;
                }
            }
            "increment" => {
                let (col, by) = (request["col"].as_str().unwrap(), request["by"].as_i64().unwrap());
                for r in rows.iter_mut().filter(|r| matches(r, filters)) {
                    r[col] = json!(r[col].as_i64().unwrap_or(0) + by);
                    changed += 1;
                }
            }
            "get" | "query" => out = rows.iter().filter(|r| matches(r, filters)).cloned().collect(),
            other => return refusal(&format!("the fake does not do {other}")),
        }
        Ok(json!({"ok": true, "rows": out, "changed": changed, "count": out.len()}))
    }
}

impl Host for FakeHost {
    fn call(&self, request: &Value) -> Result<Value, String> {
        self.calls.set(self.calls.get() + 1);
        if self.calls.get() > CALLS_PER_RUN {
            return refusal("data.store call budget for this run is exhausted");
        }
        let armed = self.fail_op.borrow().clone();
        if armed.is_some_and(|op| request["op"] == op.as_str() || request["capability"] == op.as_str()) {
            return refusal("data.store is unavailable");
        }
        match request["capability"].as_str() {
            Some("log.read") => self.log_read(request),
            Some("data.store") => self.data_store(request),
            _ => refusal("capability not available"),
        }
    }
}
