//! An in-memory `data.store` for tests: insert, query, count, update and delete over `nights` and `votes`.

use std::cell::RefCell;

use serde_json::{json, Map, Value};

use crate::host::Host;

type Row = Map<String, Value>;

#[derive(Default)]
pub struct FakeHost {
    pub nights: RefCell<Vec<Row>>,
    pub votes: RefCell<Vec<Row>>,
}

fn matches(row: &Row, filters: &Value) -> bool {
    filters.as_array().is_none_or(|all| {
        all.iter().all(|f| {
            let (cell, val) = (&row[f["col"].as_str().unwrap()], &f["val"]);
            match f["op"].as_str().unwrap() {
                "eq" => cell == val,
                "lt" => cell.as_i64() < val.as_i64(),
                "gte" => cell.as_i64() >= val.as_i64(),
                other => panic!("the fake does not do {other}"),
            }
        })
    })
}

impl Host for FakeHost {
    fn call(&self, request: &Value) -> Result<Value, String> {
        let table = if request["table"] == "nights" { &self.nights } else { &self.votes };
        let mut rows = table.borrow_mut();
        let filters = &request["where"];
        let mut changed = 0;
        let mut out: Vec<Row> = Vec::new();
        match request["op"].as_str().unwrap() {
            "insert" => rows.push(request["values"].as_object().unwrap().clone()),
            "upsert" => rows.push(request["values"].as_object().unwrap().clone()),
            "update" => {
                for r in rows.iter_mut().filter(|r| matches(r, filters)) {
                    r.extend(request["values"].as_object().unwrap().clone());
                    changed += 1;
                }
            }
            "delete" => {
                let before = rows.len();
                rows.retain(|r| !matches(r, filters));
                changed = before - rows.len();
            }
            "query" | "get" | "count" => {
                out = rows.iter().filter(|r| matches(r, filters)).cloned().collect();
                if request["order"][0]["col"] == "created" {
                    out.sort_by_key(|r| r["created"].as_i64());
                }
                if let Some(limit) = request["limit"].as_u64() {
                    out.truncate(limit as usize);
                }
            }
            other => return Err(format!("the fake does not do {other}")),
        }
        Ok(json!({"ok": true, "rows": out, "changed": changed, "count": out.len()}))
    }
}
