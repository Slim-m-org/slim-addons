//! An in-memory slim for tests: it answers the `data.store` calls the scoreboard makes, including
//! unique columns, ordering and the 64-call budget.

use std::cell::{Cell, RefCell};

use serde_json::{json, Map, Value};

use crate::host::Host;

type Row = Map<String, Value>;

#[derive(Default)]
pub struct FakeHost {
    pub best: RefCell<Vec<Row>>,
    pub names: RefCell<Vec<Row>>,
    pub calls: Cell<u32>,
    /// Refuse every call once armed, as a host that has not approved `data.store` does.
    pub refuse: Cell<bool>,
}

impl FakeHost {
    pub fn best_ms(&self, level: &str, player: &str) -> Option<i64> {
        let key = format!("{level}|{player}");
        self.best.borrow().iter().find(|r| r["k"] == key.as_str()).and_then(|r| r["ms"].as_i64())
    }

    pub fn name_of(&self, player: &str) -> Option<String> {
        self.names.borrow().iter().find(|r| r["player"] == player).and_then(|r| r["name"].as_str().map(str::to_owned))
    }
}

fn matches(row: &Row, filters: &Value) -> bool {
    filters.as_array().map_or(true, |all| {
        all.iter().all(|f| {
            let (cell, val) = (&row[f["col"].as_str().unwrap()], &f["val"]);
            match f["op"].as_str().unwrap() {
                "eq" => cell == val,
                "in" => val.as_array().unwrap().contains(cell),
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
        let (table, unique) = if request["table"] == "best" { (&self.best, "k") } else { (&self.names, "player") };
        let mut rows = table.borrow_mut();
        let filters = &request["where"];
        let mut out: Vec<Row> = Vec::new();
        let mut changed = 0;
        match request["op"].as_str().unwrap() {
            "insert" => {
                let values = request["values"].as_object().unwrap().clone();
                if rows.iter().any(|r| r[unique] == values[unique]) {
                    return Err("a unique column already holds that value".to_owned());
                }
                rows.push(values);
                changed = 1;
            }
            "upsert" => {
                let values = request["values"].as_object().unwrap().clone();
                match rows.iter_mut().find(|r| r[unique] == values[unique]) {
                    Some(existing) => existing.extend(values),
                    None => rows.push(values),
                }
                changed = 1;
            }
            "update" => {
                for r in rows.iter_mut().filter(|r| matches(r, filters)) {
                    r.extend(request["values"].as_object().unwrap().clone());
                    changed += 1;
                }
            }
            "get" | "query" => {
                out = rows.iter().filter(|r| matches(r, filters)).cloned().collect();
                if let Some(order) = request["order"].as_array().and_then(|o| o.first()) {
                    let col = order["col"].as_str().unwrap().to_owned();
                    out.sort_by_key(|r| r[&col].as_i64());
                    if order["dir"] == "desc" {
                        out.reverse();
                    }
                }
                let limit = if request["op"] == "get" { 1 } else { request["limit"].as_u64().unwrap_or(200) as usize };
                out.truncate(limit);
            }
            other => return Err(format!("the fake does not do {other}")),
        }
        Ok(json!({"ok": true, "rows": out, "changed": changed, "count": out.len()}))
    }
}
