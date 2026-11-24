//! An in-memory slim for tests: it answers the `data.store` calls the game record makes,
//! including the unique column, `lt` deletes and the 64-call budget.

use std::cell::{Cell, RefCell};

use serde_json::{json, Map, Value};

use crate::host::Host;

type Row = Map<String, Value>;

#[derive(Default)]
pub struct FakeHost {
    pub games: RefCell<Vec<Row>>,
    pub calls: Cell<u32>,
    /// Refuse every call once armed, as a host that has not approved `data.store` does.
    pub refuse: Cell<bool>,
}

fn matches(row: &Row, filters: &Value) -> bool {
    filters.as_array().map_or(true, |all| {
        all.iter().all(|f| {
            let (cell, val) = (&row[f["col"].as_str().unwrap()], &f["val"]);
            match f["op"].as_str().unwrap() {
                "eq" => cell == val,
                "lt" => cell.as_i64() < val.as_i64(),
                other => panic!("the fake does not do {other}"),
            }
        })
    })
}

impl Host for FakeHost {
    fn call(&self, request: &Value) -> Result<Value, String> {
        self.calls.set(self.calls.get() + 1);
        if self.refuse.get() {
            return Err("capability not approved: data.store".to_owned());
        }
        if self.calls.get() > 64 {
            return Err("data.store call budget for this run is exhausted".to_owned());
        }
        let mut rows = self.games.borrow_mut();
        let filters = &request["where"];
        let mut out: Vec<Row> = Vec::new();
        let mut changed = 0;
        match request["op"].as_str().unwrap() {
            "insert" => {
                let values = request["values"].as_object().unwrap().clone();
                if rows.iter().any(|r| r["k"] == values["k"]) {
                    return Err("a unique column already holds that value".to_owned());
                }
                rows.push(values);
                changed = 1;
            }
            "upsert" => {
                let values = request["values"].as_object().unwrap().clone();
                match rows.iter_mut().find(|r| r["k"] == values["k"]) {
                    Some(existing) => existing.extend(values),
                    None => rows.push(values),
                }
                changed = 1;
            }
            "delete" => {
                let before = rows.len();
                rows.retain(|r| !matches(r, filters));
                changed = before - rows.len();
            }
            "get" => out = rows.iter().filter(|r| matches(r, filters)).take(1).cloned().collect(),
            other => return Err(format!("the fake does not do {other}")),
        }
        Ok(json!({"ok": true, "rows": out, "changed": changed, "count": out.len()}))
    }
}
