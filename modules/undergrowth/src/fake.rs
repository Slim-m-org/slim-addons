//! An in-memory slim for tests. It reads the tables from this module's own manifest, so a
//! column the manifest does not declare fails the test the way the real host would refuse it,
//! and it answers `data.store`, `log.read` and `message.post` with slim's budgets.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use crate::host::Host;

type Row = Map<String, Value>;

pub struct FakeHost {
    pub tables: RefCell<BTreeMap<String, Vec<Row>>>,
    columns: BTreeMap<String, Vec<(String, bool)>>,
    next_id: Cell<i64>,
    pub calls: Cell<u32>,
    pub log: RefCell<Vec<Value>>,
    pub posts: RefCell<Vec<String>>,
    /// Refuse every `data.store` call, as a host that never approved it does.
    pub refuse_data: Cell<bool>,
}

impl Default for FakeHost {
    fn default() -> Self {
        let manifest: Value = serde_json::from_str(include_str!("../manifest.json")).expect("manifest is JSON");
        let mut columns = BTreeMap::new();
        let mut tables = BTreeMap::new();
        for t in manifest["data"]["tables"].as_array().expect("tables") {
            let name = t["name"].as_str().unwrap().to_owned();
            let cols = t["columns"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| (c["name"].as_str().unwrap().to_owned(), c["unique"].as_bool().unwrap_or(false)))
                .collect();
            columns.insert(name.clone(), cols);
            tables.insert(name, Vec::new());
        }
        FakeHost {
            tables: RefCell::new(tables),
            columns,
            next_id: Cell::new(1),
            calls: Cell::new(0),
            log: RefCell::new(Vec::new()),
            posts: RefCell::new(Vec::new()),
            refuse_data: Cell::new(false),
        }
    }
}

impl FakeHost {
    /// A new run: the call budget resets, the data stays.
    pub fn next_run(&self) {
        self.calls.set(0);
    }

    pub fn rows(&self, table: &str) -> Vec<Row> {
        self.tables.borrow()[table].clone()
    }

    fn data(&self, req: &Value) -> Result<Value, String> {
        if self.refuse_data.get() {
            return Err("capability not approved: data.store".into());
        }
        let table = req["table"].as_str().ok_or("no table")?;
        let cols = self.columns.get(table).ok_or(format!("no such table {table}"))?;
        for key in ["values", "add"] {
            for col in req[key].as_object().into_iter().flat_map(|m| m.keys()) {
                if !cols.iter().any(|(c, _)| c == col) {
                    return Err(format!("no such column {table}.{col}"));
                }
            }
        }
        let mut all = self.tables.borrow_mut();
        let rows = all.get_mut(table).unwrap();
        let unique: Vec<&String> = cols.iter().filter(|c| c.1).map(|c| &c.0).collect();
        let filters = &req["where"];
        let mut changed = 0;
        let mut out = Vec::new();
        match req["op"].as_str().unwrap_or("") {
            "insert" => {
                let batch: Vec<Value> = match req["rows"].as_array() {
                    Some(rs) => rs.clone(),
                    None => vec![req["values"].clone()],
                };
                if batch.len() > 50 {
                    return Err("at most 50 rows".into());
                }
                for values in batch {
                    let mut row = fresh(cols);
                    row.extend(values.as_object().cloned().unwrap_or_default());
                    if unique.iter().any(|u| row[*u] != Value::Null && rows.iter().any(|r| r[*u] == row[*u])) {
                        return Err("a unique column already holds that value".into());
                    }
                    let id = self.next_id.get();
                    self.next_id.set(id + 1);
                    row.insert("_id".into(), json!(id));
                    out.push(json!({"_id": id}).as_object().unwrap().clone());
                    rows.push(row);
                    changed += 1;
                }
            }
            "upsert" => {
                let on = req["on"].as_str().ok_or("upsert needs on")?;
                let values = req["values"].as_object().cloned().unwrap_or_default();
                let add = req["add"].as_object().cloned().unwrap_or_default();
                let at = rows.iter().position(|r| r[on] == values[on]);
                let k = match at {
                    Some(k) => k,
                    None => {
                        let mut row = fresh(cols);
                        let id = self.next_id.get();
                        self.next_id.set(id + 1);
                        row.insert("_id".into(), json!(id));
                        rows.push(row);
                        rows.len() - 1
                    }
                };
                rows[k].extend(values);
                add_to(&mut rows[k], &add);
                if !add.is_empty() {
                    out.push(rows[k].clone());
                }
                changed = 1;
            }
            "update" => {
                let values = req["values"].as_object().cloned().unwrap_or_default();
                let add = req["add"].as_object().cloned().unwrap_or_default();
                for r in rows.iter_mut().filter(|r| matches(r, filters)) {
                    r.extend(values.clone());
                    add_to(r, &add);
                    changed += 1;
                }
            }
            "delete" => {
                let before = rows.len();
                rows.retain(|r| !matches(r, filters));
                changed = before - rows.len();
            }
            "get" | "query" | "count" | "sum" => {
                out = rows.iter().filter(|r| matches(r, filters)).cloned().collect();
                if let Some(order) = req["order"].as_array().and_then(|o| o.first()) {
                    let col = order["col"].as_str().unwrap().to_owned();
                    out.sort_by(|a, b| cmp(&a[&col], &b[&col]));
                    if order["dir"] == "desc" {
                        out.reverse();
                    }
                }
                if req["op"] == "count" {
                    return Ok(json!({"ok": true, "rows": [], "changed": 0, "count": out.len()}));
                }
                if req["op"] == "sum" {
                    let col = req["col"].as_str().unwrap();
                    let total: i64 = out.iter().filter_map(|r| r[col].as_i64()).sum();
                    return Ok(json!({"ok": true, "rows": [], "changed": 0, "count": total}));
                }
                let limit = if req["op"] == "get" { 1 } else { req["limit"].as_u64().unwrap_or(200) as usize };
                out.truncate(limit);
            }
            other => return Err(format!("the fake does not do {other}")),
        }
        Ok(json!({"ok": true, "rows": out, "changed": changed, "count": out.len()}))
    }
}

fn fresh(cols: &[(String, bool)]) -> Row {
    cols.iter().map(|(c, _)| (c.clone(), Value::Null)).collect()
}

fn cmp(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
        _ => a.as_str().unwrap_or("").cmp(b.as_str().unwrap_or("")),
    }
}

fn matches(row: &Row, filters: &Value) -> bool {
    filters.as_array().is_none_or(|all| {
        all.iter().all(|f| {
            let (cell, val) = (row.get(f["col"].as_str().unwrap()).unwrap_or(&Value::Null), &f["val"]);
            let order = cmp(cell, val);
            match f["op"].as_str().unwrap() {
                "eq" => cell == val,
                "ne" => cell != val,
                "lt" => !cell.is_null() && order.is_lt(),
                "lte" => !cell.is_null() && order.is_le(),
                "gt" => !cell.is_null() && order.is_gt(),
                "gte" => !cell.is_null() && order.is_ge(),
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

impl Host for FakeHost {
    fn call(&self, req: &Value) -> Result<Value, String> {
        self.calls.set(self.calls.get() + 1);
        if self.calls.get() > 64 {
            return Err("the store call budget for this run is spent".into());
        }
        match req["capability"].as_str() {
            Some("data.store") => self.data(req),
            Some("log.read") => {
                let after = req["after"].as_i64().unwrap_or(0) as usize;
                let limit = req["limit"].as_u64().unwrap_or(100).min(200) as usize;
                let log = self.log.borrow();
                let rows: Vec<Value> = log.iter().skip(after).take(limit).cloned().collect();
                Ok(json!({"ok": true, "rows": rows, "next": after + rows.len()}))
            }
            Some("message.post") => {
                let mut posts = self.posts.borrow_mut();
                if posts.len() >= 3 {
                    return Err("message.post budget spent".into());
                }
                posts.push(req["content"].as_str().unwrap_or("").to_owned());
                Ok(json!({"ok": true, "message_id": "m"}))
            }
            _ => Err("capability not approved".into()),
        }
    }
}
