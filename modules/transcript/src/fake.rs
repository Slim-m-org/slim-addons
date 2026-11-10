//! An in-memory slim for tests: `data.store` built from this module's own manifest, and `message.post` with the host's limits.
//!
//! It holds the rules the host holds: declared columns and types, unique columns, the 64-call data budget, the
//! 3-post budget, 200-row pages, offsets up to 10,000, and a 4,000-character message.

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::HashMap;

use serde_json::{json, Map, Value};

use crate::host::Host;
use crate::store::{DATA_CALLS_PER_RUN, POSTS_PER_RUN};

type Row = Map<String, Value>;

const MANIFEST: &str = include_str!("../manifest.json");

struct Table {
    columns: HashMap<String, String>,
    unique: Vec<String>,
    rows: Vec<Row>,
    next_id: i64,
}

pub struct FakeHost {
    tables: RefCell<HashMap<String, Table>>,
    data_calls: Cell<u32>,
    posts_this_run: Cell<u32>,
    /// Every message the module posted, across runs.
    pub posts: RefCell<Vec<String>>,
    /// Posts refuse with `message.post needs a channel`, as before an admin picks one.
    pub no_channel: Cell<bool>,
    /// Posts refuse with this text once armed.
    pub refuse_posts: RefCell<Option<String>>,
}

impl Default for FakeHost {
    fn default() -> Self {
        let manifest: Value = serde_json::from_str(MANIFEST).unwrap();
        let tables = manifest["data"]["tables"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                let cols = t["columns"].as_array().unwrap();
                let columns = cols.iter().map(|c| (c["name"].as_str().unwrap().to_string(), c["type"].as_str().unwrap().to_string())).collect();
                let unique = cols.iter().filter(|c| c["unique"] == true).map(|c| c["name"].as_str().unwrap().to_string()).collect();
                (t["name"].as_str().unwrap().to_string(), Table { columns, unique, rows: Vec::new(), next_id: 1 })
            })
            .collect();
        FakeHost {
            tables: RefCell::new(tables),
            data_calls: Cell::new(0),
            posts_this_run: Cell::new(0),
            posts: RefCell::default(),
            no_channel: Cell::new(false),
            refuse_posts: RefCell::default(),
        }
    }
}

impl FakeHost {
    /// Starts a new run: the call budgets reset, the data stays.
    pub fn next_run(&self) {
        self.data_calls.set(0);
        self.posts_this_run.set(0);
    }

    pub fn rows(&self, table: &str) -> Vec<Row> {
        self.tables.borrow()[table].rows.clone()
    }

    pub fn call(&self, session: &str) -> Row {
        self.rows("calls").into_iter().find(|r| r["session_id"] == session).expect("the call is stored")
    }

    pub fn has_call(&self, session: &str) -> bool {
        self.rows("calls").iter().any(|r| r["session_id"] == session)
    }

    fn data(&self, request: &Value) -> Result<Value, String> {
        self.data_calls.set(self.data_calls.get() + 1);
        if self.data_calls.get() > DATA_CALLS_PER_RUN {
            return Err("data.store call budget for this run is exhausted".to_string());
        }
        let mut tables = self.tables.borrow_mut();
        let table = tables.get_mut(request["table"].as_str().unwrap()).ok_or("no such table")?;
        let filters = request.get("where").cloned().unwrap_or(json!([]));
        assert!(filters.as_array().unwrap().len() <= 16, "at most 16 conditions");
        match request["op"].as_str().unwrap() {
            "insert" => table.insert(request),
            "upsert" => table.upsert(request),
            "update" => table.update(request, &filters),
            "delete" => {
                let before = table.rows.len();
                table.rows.retain(|r| !matches(r, &filters));
                Ok(json!({"ok": true, "changed": before - table.rows.len()}))
            }
            "get" => Ok(table.select(request, &filters, 1)),
            "query" => Ok(table.select(request, &filters, 200)),
            other => Err(format!("the fake does not do {other}")),
        }
    }

    fn post(&self, request: &Value) -> Result<Value, String> {
        self.posts_this_run.set(self.posts_this_run.get() + 1);
        if self.posts_this_run.get() > POSTS_PER_RUN {
            return Err("message.post budget for this run is exhausted".to_string());
        }
        if self.no_channel.get() {
            return Err("message.post needs a channel".to_string());
        }
        if let Some(why) = self.refuse_posts.borrow().clone() {
            return Err(why);
        }
        let content = request["content"].as_str().ok_or("message.post needs content")?;
        if content.trim().is_empty() || content.chars().count() > 4000 {
            return Err("content is empty or too long".to_string());
        }
        self.posts.borrow_mut().push(content.to_string());
        Ok(json!({"ok": true, "message_id": format!("m{}", self.posts.borrow().len())}))
    }
}

impl Table {
    fn check(&self, values: &Row) -> Result<(), String> {
        for (col, value) in values {
            let ty = self.columns.get(col).ok_or_else(|| format!("no column {col}"))?;
            let fits = match ty.as_str() {
                "int" => value.is_i64(),
                "bool" => value.is_boolean(),
                _ => value.as_str().is_some_and(|s| s.len() <= 4096),
            };
            if !fits {
                return Err(format!("{col} must be a {ty}"));
            }
        }
        Ok(())
    }

    fn clash(&self, row: &Row, skip: Option<usize>) -> bool {
        self.unique.iter().any(|u| row.contains_key(u) && self.rows.iter().enumerate().any(|(i, r)| Some(i) != skip && r.get(u) == row.get(u)))
    }

    fn insert(&mut self, request: &Value) -> Result<Value, String> {
        let batch: Vec<Row> = match request.get("rows") {
            Some(rows) => rows.as_array().unwrap().iter().map(|r| r.as_object().unwrap().clone()).collect(),
            None => vec![request["values"].as_object().unwrap().clone()],
        };
        assert!(batch.len() <= 50, "the host takes at most 50 rows in one insert");
        let before = self.rows.len();
        let mut ids = Vec::new();
        for mut row in batch {
            self.check(&row)?;
            if self.clash(&row, None) {
                self.rows.truncate(before);
                return Err("a unique column already holds that value".to_string());
            }
            for col in self.columns.keys() {
                row.entry(col.clone()).or_insert(Value::Null);
            }
            row.insert("_id".to_string(), json!(self.next_id));
            ids.push(json!({"_id": self.next_id}));
            self.next_id += 1;
            self.rows.push(row);
        }
        Ok(json!({"ok": true, "rows": ids, "changed": ids.len()}))
    }

    fn upsert(&mut self, request: &Value) -> Result<Value, String> {
        let values = request["values"].as_object().unwrap().clone();
        let on = request["on"].as_str().unwrap();
        assert!(self.unique.iter().any(|u| u == on) && values.contains_key(on), "upsert needs a unique column in values");
        self.check(&values)?;
        match self.rows.iter_mut().find(|r| r[on] == values[on]) {
            Some(row) => row.extend(values),
            None => {
                let mut row = values;
                for col in self.columns.keys() {
                    row.entry(col.clone()).or_insert(Value::Null);
                }
                row.insert("_id".to_string(), json!(self.next_id));
                self.next_id += 1;
                self.rows.push(row);
            }
        }
        Ok(json!({"ok": true, "changed": 1}))
    }

    fn update(&mut self, request: &Value, filters: &Value) -> Result<Value, String> {
        let values = request.get("values").and_then(Value::as_object).cloned().unwrap_or_default();
        let add = request.get("add").and_then(Value::as_object).cloned().unwrap_or_default();
        assert!(add.keys().all(|c| !values.contains_key(c)), "a column cannot be in both values and add");
        self.check(&values)?;
        let hits: Vec<usize> = (0..self.rows.len()).filter(|&i| matches(&self.rows[i], filters)).collect();
        for &i in &hits {
            let mut next = self.rows[i].clone();
            next.extend(values.clone());
            for (col, by) in &add {
                let now = next.get(col).and_then(Value::as_i64).unwrap_or(0);
                next.insert(col.clone(), json!(now + by.as_i64().unwrap()));
            }
            if self.clash(&next, Some(i)) {
                return Err("a unique column already holds that value".to_string());
            }
            self.rows[i] = next;
        }
        Ok(json!({"ok": true, "changed": hits.len()}))
    }

    fn select(&self, request: &Value, filters: &Value, default_limit: usize) -> Value {
        let mut rows: Vec<Row> = self.rows.iter().filter(|r| matches(r, filters)).cloned().collect();
        let order: Vec<(String, bool)> = request
            .get("order")
            .and_then(Value::as_array)
            .map(|o| o.iter().map(|k| (k["col"].as_str().unwrap().to_string(), k["dir"] == "desc")).collect())
            .unwrap_or_default();
        rows.sort_by(|a, b| {
            order.iter().fold(Ordering::Equal, |acc, (col, desc)| {
                let by = compare(&a[col], &b[col]);
                acc.then(if *desc { by.reverse() } else { by })
            })
        });
        let limit = request.get("limit").and_then(Value::as_u64).map_or(default_limit, |l| {
            assert!((1..=200).contains(&l), "limit is 1 to 200");
            l as usize
        });
        let offset = request.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        assert!(offset <= 10_000, "offset is at most 10,000");
        let rows: Vec<Row> = rows.into_iter().skip(offset).take(limit).collect();
        json!({"ok": true, "rows": rows, "count": rows.len()})
    }
}

fn compare(a: &Value, b: &Value) -> Ordering {
    match (a.as_i64(), b.as_i64()) {
        (Some(x), Some(y)) => x.cmp(&y),
        _ => a.to_string().cmp(&b.to_string()),
    }
}

fn matches(row: &Row, filters: &Value) -> bool {
    filters.as_array().unwrap().iter().all(|f| {
        let (cell, val) = (&row[f["col"].as_str().unwrap()], &f["val"]);
        assert!(!val.is_null(), "a condition value cannot be null");
        let order = compare(cell, val);
        match f["op"].as_str().unwrap() {
            "eq" => cell == val,
            "ne" => cell != val,
            "lt" => order == Ordering::Less,
            "lte" => order != Ordering::Greater,
            "gt" => order == Ordering::Greater,
            "gte" => order != Ordering::Less,
            "in" => val.as_array().unwrap().contains(cell),
            other => panic!("the fake does not do {other}"),
        }
    })
}

impl Host for FakeHost {
    fn call(&self, request: &Value) -> Result<Value, String> {
        match request["capability"].as_str() {
            Some("data.store") => self.data(request),
            Some("message.post") => self.post(request),
            _ => Err("capability not available".to_string()),
        }
    }
}
