//! An in-memory slim for tests: `data.store` over one table, and `message.post` kept as a list.

use std::cell::{Cell, RefCell};

use serde_json::{json, Map, Value};

use crate::host::Host;

type Row = Map<String, Value>;

#[derive(Default)]
pub struct FakeHost {
    pub plays: RefCell<Vec<Row>>,
    pub posts: RefCell<Vec<String>>,
    pub calls: Cell<u32>,
    /// Refuse `data.store` as a host that has not approved it does.
    pub refuse: Cell<bool>,
    /// Refuse `message.post` only.
    pub refuse_post: Cell<bool>,
}

fn matches(row: &Row, filters: &Value) -> bool {
    filters.as_array().map_or(true, |all| {
        all.iter().all(|f| {
            assert_eq!(f["op"], "eq", "the fake only does eq");
            row[f["col"].as_str().unwrap()] == f["val"]
        })
    })
}

impl Host for FakeHost {
    fn call(&self, request: &Value) -> Result<Value, String> {
        self.calls.set(self.calls.get() + 1);
        if self.calls.get() > 128 {
            return Err("call budget for this run is exhausted".to_owned());
        }
        match request["capability"].as_str() {
            Some("message.post") => {
                if self.refuse_post.get() {
                    return Err("capability not approved: message.post".to_owned());
                }
                self.posts.borrow_mut().push(request["content"].as_str().unwrap().to_owned());
                Ok(json!({"ok": true}))
            }
            Some("data.store") => self.data(request),
            _ => Err("capability not available".to_owned()),
        }
    }
}

impl FakeHost {
    fn data(&self, request: &Value) -> Result<Value, String> {
        if self.refuse.get() {
            return Err("capability not approved: data.store".to_owned());
        }
        assert_eq!(request["table"], "plays");
        let mut rows = self.plays.borrow_mut();
        let mut out: Vec<Row> = Vec::new();
        match request["op"].as_str().unwrap() {
            "upsert" => {
                let values = request["values"].as_object().unwrap().clone();
                match rows.iter_mut().find(|r| r["k"] == values["k"]) {
                    Some(existing) => existing.extend(values),
                    None => rows.push(values),
                }
            }
            "get" | "query" => {
                out = rows.iter().filter(|r| matches(r, &request["where"])).cloned().collect();
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
        Ok(json!({"ok": true, "rows": out, "count": out.len()}))
    }
}
