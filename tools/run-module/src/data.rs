//! `data.store` for the runner: the module's declared tables held in memory,
//! with the same ops, bounds and refusals as slim's host (`module_runtime/data*.rs`
//! in slim-m is the authority). `--data-file <path>` keeps the tables between runs.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

type Row = Map<String, Value>;

const MAX_TEXT_BYTES: usize = 4 * 1024;
const MAX_ROWS: i64 = 200;
const MAX_OFFSET: i64 = 10_000;
const MAX_INSERT_ROWS: usize = 50;
const MAX_CONDITIONS: usize = 16;
const MAX_IN_VALUES: usize = 100;
const MAX_ORDER: usize = 4;
const MAX_LIKE_BYTES: usize = 256;
const MAX_LIKE_TOTAL: usize = 1024;
const MAX_STEP: i64 = 1 << 53;
const ID: &str = "_id";

#[derive(Clone, Copy, PartialEq)]
enum Ty {
    Int,
    Real,
    Text,
    Bool,
}

struct Col {
    name: String,
    ty: Ty,
    unique: bool,
}

#[derive(Default)]
struct Table {
    cols: Vec<Col>,
    rows: Vec<Row>,
    next_id: i64,
}

/// The tables a manifest's `data` block declares, and what they hold.
pub struct Data {
    tables: BTreeMap<String, Table>,
    max_bytes: usize,
}

struct Cond {
    col: String,
    op: String,
    val: Value,
}

impl Data {
    /// `None` when the manifest declares no tables.
    pub fn from_manifest(manifest: &Value) -> Option<Data> {
        let declared = manifest["data"]["tables"].as_array()?;
        let mut tables = BTreeMap::new();
        for t in declared {
            let cols = t["columns"].as_array()?.iter().map(parse_col).collect::<Option<Vec<_>>>()?;
            tables.insert(t["name"].as_str()?.to_owned(), Table { cols, rows: Vec::new(), next_id: 1 });
        }
        let mb = manifest["data"]["max_mb"].as_u64().unwrap_or(8);
        Some(Data { tables, max_bytes: usize::try_from(mb).unwrap_or(8) * 1024 * 1024 })
    }

    /// Restores what `dump` wrote; a table the manifest no longer declares is dropped.
    pub fn load(&mut self, text: &str) {
        let Ok(Value::Object(saved)) = serde_json::from_str::<Value>(text) else { return };
        for (name, table) in &mut self.tables {
            let Some(rows) = saved[name]["rows"].as_array() else { continue };
            table.rows = rows.iter().filter_map(|r| r.as_object().cloned()).collect();
            table.next_id = saved[name]["next_id"].as_i64().unwrap_or(1);
        }
    }

    pub fn dump(&self) -> String {
        let all: Map<String, Value> =
            self.tables.iter().map(|(n, t)| (n.clone(), json!({"next_id": t.next_id, "rows": t.rows}))).collect();
        Value::Object(all).to_string()
    }

    /// Answers one request in slim's reply shape.
    pub fn handle(&mut self, request: &Value) -> Value {
        match self.run(request) {
            Ok(reply) => reply,
            Err(message) => json!({"ok": false, "error": message}),
        }
    }

    fn run(&mut self, request: &Value) -> Result<Value, String> {
        let name = request["table"].as_str().unwrap_or_default();
        let max_bytes = self.max_bytes;
        let table = self.tables.get_mut(name).ok_or("unknown table")?;
        let op = request["op"].as_str().unwrap_or_default();
        if request.get("add").is_some() && !matches!(op, "upsert" | "update") {
            return Err("add is for upsert and update".into());
        }
        if request.get("rows").is_some() && op != "insert" {
            return Err("rows is for insert".into());
        }
        let outcome = match op {
            "insert" => table.insert(request),
            "upsert" => table.upsert(request),
            "update" => table.update(request),
            "delete" => table.delete(request),
            "get" => table.select(request, true),
            "query" => table.select(request, false),
            "count" => table.count(request),
            "sum" => table.sum(request),
            "increment" => table.increment(request),
            _ => Err("unknown data.store op".into()),
        }?;
        if outcome.writes && table.bytes() > max_bytes {
            return Err("data.store is full for this module".into());
        }
        Ok(outcome.reply())
    }
}

fn parse_col(c: &Value) -> Option<Col> {
    let ty = match c["type"].as_str()? {
        "int" => Ty::Int,
        "real" => Ty::Real,
        "text" => Ty::Text,
        "bool" => Ty::Bool,
        _ => return None,
    };
    Some(Col { name: c["name"].as_str()?.to_owned(), ty, unique: c["unique"].as_bool().unwrap_or(false) })
}

#[derive(Default)]
struct Outcome {
    rows: Vec<Row>,
    changed: usize,
    scalar: Option<Value>,
    writes: bool,
}

impl Outcome {
    fn reply(self) -> Value {
        let count = self.scalar.unwrap_or_else(|| json!(self.rows.len()));
        json!({"ok": true, "rows": self.rows, "changed": self.changed, "count": count})
    }
}

impl Table {
    fn col(&self, name: &str) -> Option<&Col> {
        self.cols.iter().find(|c| c.name == name)
    }

    fn ty(&self, name: &str) -> Result<Ty, String> {
        if name == ID {
            return Ok(Ty::Int);
        }
        self.col(name).map(|c| c.ty).ok_or_else(|| "unknown column".to_owned())
    }

    fn bytes(&self) -> usize {
        self.rows.iter().map(|r| Value::Object(r.clone()).to_string().len()).sum()
    }

    fn fresh_row(&self, id: i64) -> Row {
        let mut row: Row = self.cols.iter().map(|c| (c.name.clone(), Value::Null)).collect();
        row.insert(ID.into(), json!(id));
        row
    }

    fn assignments(&self, values: &Map<String, Value>) -> Result<Vec<(String, Value)>, String> {
        let mut out = Vec::new();
        for (name, value) in values {
            if name == ID {
                return Err("_id cannot be set".into());
            }
            out.push((name.clone(), coerce(self.ty(name)?, value)?));
        }
        Ok(out)
    }

    fn additions(&self, add: Option<&Value>) -> Result<Vec<(String, Value)>, String> {
        let mut out = Vec::new();
        for (name, value) in add.and_then(Value::as_object).into_iter().flatten() {
            let ty = match self.col(name) {
                Some(c) if matches!(c.ty, Ty::Int | Ty::Real) => c.ty,
                _ => return Err("add must name declared int or real columns".into()),
            };
            if value.is_null() {
                return Err("add amounts cannot be null".into());
            }
            let amount = coerce(ty, value)?;
            if out_of_range(&amount) {
                return Err("add amount is out of range".into());
            }
            out.push((name.clone(), amount));
        }
        Ok(out)
    }

    fn commit(&mut self, rows: Vec<Row>) -> Result<(), String> {
        for col in self.cols.iter().filter(|c| c.unique) {
            let mut seen: Vec<&Value> = Vec::new();
            for row in rows.iter().map(|r| &r[&col.name]).filter(|v| !v.is_null()) {
                if seen.contains(&row) {
                    return Err("a unique column already holds that value".into());
                }
                seen.push(row);
            }
        }
        self.rows = rows;
        Ok(())
    }

    fn insert(&mut self, req: &Value) -> Result<Outcome, String> {
        let batch: Vec<Map<String, Value>> = match (req.get("rows"), req.get("values")) {
            (Some(_), Some(_)) => return Err("values and rows cannot be combined".into()),
            (Some(rows), None) => {
                let list = rows.as_array().filter(|a| (1..=MAX_INSERT_ROWS).contains(&a.len()));
                let list = list.ok_or_else(|| format!("rows needs 1 to {MAX_INSERT_ROWS} rows"))?;
                list.iter().map(|r| r.as_object().cloned().ok_or("malformed data.store request")).collect::<Result<_, _>>()?
            }
            (None, values) => vec![values.and_then(Value::as_object).cloned().ok_or("this op needs values")?],
        };
        let mut built = Vec::new();
        for values in &batch {
            built.push(self.assignments(values)?);
        }
        if req.get("rows").is_some() {
            let names = |row: &Vec<(String, Value)>| row.iter().map(|c| c.0.clone()).collect::<std::collections::BTreeSet<_>>();
            if names(&built[0]).is_empty() {
                return Err("each row needs at least one value".into());
            }
            if built.iter().any(|row| names(row) != names(&built[0])) {
                return Err("every row must set the same columns".into());
            }
        }
        let mut rows = self.rows.clone();
        let mut ids = Vec::new();
        let mut next = self.next_id;
        for cols in built {
            let mut row = self.fresh_row(next);
            row.extend(cols);
            ids.push(Row::from_iter([(ID.to_owned(), json!(next))]));
            rows.push(row);
            next += 1;
        }
        self.commit(rows)?;
        self.next_id = next;
        Ok(Outcome { changed: ids.len(), rows: ids, writes: true, ..Outcome::default() })
    }

    fn upsert(&mut self, req: &Value) -> Result<Outcome, String> {
        let key = match &req["on"] {
            Value::String(s) => s.clone(),
            Value::Array(a) if a.len() == 1 => a[0].as_str().ok_or("on must name a column")?.to_owned(),
            _ => return Err("upsert needs on: one unique column".into()),
        };
        if !self.col(&key).is_some_and(|c| c.unique) {
            return Err("upsert on must be a declared unique column".into());
        }
        let values = self.assignments(req["values"].as_object().ok_or("this op needs values")?)?;
        let Some((_, key_value)) = values.iter().find(|c| c.0 == key).cloned() else {
            return Err("upsert values must include the on column".into());
        };
        let adds = self.additions(req.get("add"))?;
        reject_overlap(&values, &adds)?;
        if adds.iter().any(|a| a.0 == key) {
            return Err("add cannot name the on column".into());
        }
        let mut rows = self.rows.clone();
        let existing = rows.iter().position(|r| !key_value.is_null() && r[&key] == key_value);
        let (index, changed) = match existing {
            Some(i) => {
                let sets = values.iter().any(|c| c.0 != key) || !adds.is_empty();
                apply(&mut rows[i], values.into_iter().filter(|c| c.0 != key), &adds)?;
                (i, usize::from(sets))
            }
            None => {
                let mut row = self.fresh_row(self.next_id);
                self.next_id += 1;
                row.extend(values);
                apply(&mut row, std::iter::empty(), &adds)?;
                rows.push(row);
                (rows.len() - 1, 1)
            }
        };
        let returned = if adds.is_empty() { Vec::new() } else { vec![rows[index].clone()] };
        self.commit(rows)?;
        Ok(Outcome { rows: returned, changed, writes: true, ..Outcome::default() })
    }

    fn update(&mut self, req: &Value) -> Result<Outcome, String> {
        let adds = self.additions(req.get("add"))?;
        let values = match (req.get("values"), adds.is_empty()) {
            (None, false) => Vec::new(),
            (values, _) => self.assignments(values.and_then(Value::as_object).ok_or("this op needs values")?)?,
        };
        if values.is_empty() && adds.is_empty() {
            return Err("update needs at least one value".into());
        }
        reject_overlap(&values, &adds)?;
        let conds = self.conditions(req)?;
        let mut rows = self.rows.clone();
        let mut changed = 0;
        for row in rows.iter_mut().filter(|r| holds(r, &conds)) {
            apply(row, values.iter().cloned(), &adds)?;
            changed += 1;
        }
        self.commit(rows)?;
        Ok(Outcome { changed, writes: true, ..Outcome::default() })
    }

    fn delete(&mut self, req: &Value) -> Result<Outcome, String> {
        let conds = self.conditions(req)?;
        let before = self.rows.len();
        self.rows.retain(|r| !holds(r, &conds));
        Ok(Outcome { changed: before - self.rows.len(), ..Outcome::default() })
    }

    fn select(&self, req: &Value, first: bool) -> Result<Outcome, String> {
        let conds = self.conditions(req)?;
        let order = self.order(req)?;
        let limit = if first { 1 } else { row_limit(req)? };
        let offset = req["offset"].as_i64().unwrap_or(0);
        if !(0..=MAX_OFFSET).contains(&offset) {
            return Err("offset is out of range".into());
        }
        let mut rows: Vec<&Row> = self.rows.iter().filter(|r| holds(r, &conds)).collect();
        rows.sort_by(|a, b| {
            let by_key = order.iter().map(|(col, desc)| {
                let ord = compare(&a[col], &b[col]);
                if *desc { ord.reverse() } else { ord }
            });
            by_key.fold(Ordering::Equal, Ordering::then).then_with(|| compare(&a[ID], &b[ID]))
        });
        let rows = rows.into_iter().skip(offset as usize).take(limit).cloned().collect();
        Ok(Outcome { rows, ..Outcome::default() })
    }

    fn count(&self, req: &Value) -> Result<Outcome, String> {
        let conds = self.conditions(req)?;
        let n = self.rows.iter().filter(|r| holds(r, &conds)).count();
        Ok(Outcome { scalar: Some(json!(n)), ..Outcome::default() })
    }

    fn numeric(&self, req: &Value) -> Result<(String, Ty), String> {
        let name = req["col"].as_str().ok_or("this op needs col")?;
        match self.col(name) {
            Some(c) if matches!(c.ty, Ty::Int | Ty::Real) => Ok((name.to_owned(), c.ty)),
            _ => Err("col must be a declared int or real column".into()),
        }
    }

    fn sum(&self, req: &Value) -> Result<Outcome, String> {
        let (col, ty) = self.numeric(req)?;
        let conds = self.conditions(req)?;
        let rows = self.rows.iter().filter(|r| holds(r, &conds));
        let total = if ty == Ty::Int {
            json!(rows.filter_map(|r| r[&col].as_i64()).sum::<i64>())
        } else {
            json!(rows.filter_map(|r| r[&col].as_f64()).sum::<f64>())
        };
        Ok(Outcome { scalar: Some(total), ..Outcome::default() })
    }

    fn increment(&mut self, req: &Value) -> Result<Outcome, String> {
        let (col, ty) = self.numeric(req)?;
        let by = match (req.get("by"), ty) {
            (None, Ty::Int) => json!(1),
            (Some(v), _) => coerce(ty, v)?,
            (None, _) => return Err("increment needs by".into()),
        };
        if by.is_null() || out_of_range(&by) {
            return Err("by is out of range".into());
        }
        let conds = self.conditions(req)?;
        let mut rows = self.rows.clone();
        let mut changed = 0;
        for row in rows.iter_mut().filter(|r| holds(r, &conds)) {
            add_to(row, &col, &by)?;
            changed += 1;
        }
        self.commit(rows)?;
        Ok(Outcome { changed, writes: true, ..Outcome::default() })
    }

    fn conditions(&self, req: &Value) -> Result<Vec<Cond>, String> {
        let list = match req.get("where") {
            None | Some(Value::Null) => return Ok(Vec::new()),
            Some(Value::Array(list)) => list,
            Some(_) => return Err("malformed data.store request".into()),
        };
        if list.len() > MAX_CONDITIONS {
            return Err("too many where conditions".into());
        }
        let mut like_bytes = 0;
        let mut out = Vec::new();
        for c in list {
            let col = c["col"].as_str().ok_or("malformed data.store request")?.to_owned();
            let op = c["op"].as_str().ok_or("malformed data.store request")?.to_owned();
            let ty = self.ty(&col)?;
            let val = match op.as_str() {
                "in" => self.in_list(ty, &c["val"])?,
                "eq" | "ne" | "gt" | "gte" | "lt" | "lte" => bound(ty, &c["val"])?,
                "like" if ty == Ty::Text => {
                    let pattern = bound(ty, &c["val"])?;
                    like_bytes += pattern.as_str().map_or(0, str::len);
                    if pattern.as_str().map_or(0, str::len) > MAX_LIKE_BYTES || like_bytes > MAX_LIKE_TOTAL {
                        return Err("like patterns are too long".into());
                    }
                    pattern
                }
                _ => return Err("unknown where operator".into()),
            };
            out.push(Cond { col, op, val });
        }
        Ok(out)
    }

    fn in_list(&self, ty: Ty, val: &Value) -> Result<Value, String> {
        let items = val.as_array().filter(|a| (1..=MAX_IN_VALUES).contains(&a.len()));
        let items = items.ok_or_else(|| format!("in needs a list of 1 to {MAX_IN_VALUES} values"))?;
        items.iter().map(|v| bound(ty, v)).collect::<Result<Vec<_>, _>>().map(Value::Array)
    }

    fn order(&self, req: &Value) -> Result<Vec<(String, bool)>, String> {
        let list = req["order"].as_array().cloned().unwrap_or_default();
        if list.len() > MAX_ORDER {
            return Err("too many order entries".into());
        }
        let mut out = Vec::new();
        for entry in &list {
            let col = entry["col"].as_str().ok_or("malformed data.store request")?;
            self.ty(col)?;
            let desc = match entry["dir"].as_str() {
                None | Some("asc") => false,
                Some("desc") => true,
                Some(_) => return Err("order dir must be asc or desc".into()),
            };
            out.push((col.to_owned(), desc));
        }
        Ok(out)
    }
}

fn row_limit(req: &Value) -> Result<usize, String> {
    match req.get("limit").and_then(Value::as_i64) {
        None => Ok(MAX_ROWS as usize),
        Some(n) if (1..=MAX_ROWS).contains(&n) => Ok(n as usize),
        Some(_) => Err(format!("limit must be 1 to {MAX_ROWS}")),
    }
}

fn reject_overlap(values: &[(String, Value)], adds: &[(String, Value)]) -> Result<(), String> {
    if adds.iter().any(|a| values.iter().any(|v| v.0 == a.0)) {
        return Err("a column cannot be in both values and add".into());
    }
    Ok(())
}

fn apply(row: &mut Row, values: impl Iterator<Item = (String, Value)>, adds: &[(String, Value)]) -> Result<(), String> {
    row.extend(values);
    adds.iter().try_for_each(|(col, by)| add_to(row, col, by))
}

fn add_to(row: &mut Row, col: &str, by: &Value) -> Result<(), String> {
    let sum = match (&row[col], by) {
        (Value::Null, by) => by.clone(),
        (now, by) if now.is_i64() && by.is_i64() => {
            json!(now.as_i64().and_then(|n| n.checked_add(by.as_i64()?)).ok_or("integer overflow")?)
        }
        (now, by) => json!(now.as_f64().unwrap_or(0.0) + by.as_f64().unwrap_or(0.0)),
    };
    row.insert(col.to_owned(), sum);
    Ok(())
}

fn out_of_range(amount: &Value) -> bool {
    match amount.as_i64() {
        Some(n) => n.abs() > MAX_STEP,
        None => amount.as_f64().is_none_or(|x| !x.is_finite() || x.abs() > MAX_STEP as f64),
    }
}

fn coerce(ty: Ty, value: &Value) -> Result<Value, String> {
    match (ty, value) {
        (_, Value::Null) => Ok(Value::Null),
        (Ty::Int, Value::Number(n)) if n.is_i64() => Ok(value.clone()),
        (Ty::Int, Value::Number(_)) => Err("value is not an integer".into()),
        (Ty::Real, Value::Number(n)) => n.as_f64().filter(|f| f.is_finite()).map(|f| json!(f)).ok_or("value is not a number".into()),
        (Ty::Bool, Value::Bool(_)) => Ok(value.clone()),
        (Ty::Text, Value::String(s)) if s.len() <= MAX_TEXT_BYTES => Ok(value.clone()),
        (Ty::Text, Value::String(_)) => Err("text value is too large".into()),
        _ => Err("value does not match the column type".into()),
    }
}

/// A condition value: typed like a stored one, and never null.
fn bound(ty: Ty, value: &Value) -> Result<Value, String> {
    match coerce(ty, value)? {
        Value::Null => Err("where values cannot be null".into()),
        v => Ok(v),
    }
}

/// SQLite's order across types: null, then numbers (a bool is 0 or 1), then text.
fn compare(a: &Value, b: &Value) -> Ordering {
    let rank = |v: &Value| match v {
        Value::Null => 0,
        Value::Bool(_) | Value::Number(_) => 1,
        _ => 2,
    };
    let num = |v: &Value| v.as_f64().unwrap_or_else(|| f64::from(u8::from(v.as_bool().unwrap_or(false))));
    match (a, b) {
        (Value::String(x), Value::String(y)) => x.cmp(y),
        _ if rank(a) == 1 && rank(b) == 1 => num(a).total_cmp(&num(b)),
        _ => rank(a).cmp(&rank(b)),
    }
}

fn holds(row: &Row, conds: &[Cond]) -> bool {
    conds.iter().all(|c| {
        let cell = &row[&c.col];
        let ord = compare(cell, &c.val);
        match c.op.as_str() {
            "eq" => !cell.is_null() && ord.is_eq(),
            "ne" => cell.is_null() || ord.is_ne(),
            "gt" => !cell.is_null() && ord.is_gt(),
            "gte" => !cell.is_null() && ord.is_ge(),
            "lt" => !cell.is_null() && ord.is_lt(),
            "lte" => !cell.is_null() && ord.is_le(),
            "in" => !cell.is_null() && c.val.as_array().is_some_and(|l| l.iter().any(|v| compare(cell, v).is_eq())),
            _ => cell.as_str().is_some_and(|s| like(s, c.val.as_str().unwrap_or_default())),
        }
    })
}

/// SQLite `LIKE ... ESCAPE '\'`: `%` any run, `_` one character, ASCII case-insensitive.
fn like(text: &str, pattern: &str) -> bool {
    enum Tok {
        Any,
        One,
        Lit(char),
    }
    let mut toks = Vec::new();
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        toks.push(match c {
            '%' => Tok::Any,
            '_' => Tok::One,
            '\\' => Tok::Lit(chars.next().unwrap_or('\\')),
            c => Tok::Lit(c),
        });
    }
    let text: Vec<char> = text.chars().collect();
    let mut reach = vec![true];
    reach.resize(text.len() + 1, false);
    for tok in &toks {
        let mut next = vec![false; text.len() + 1];
        for i in 0..=text.len() {
            next[i] = match tok {
                Tok::Any => reach[i] || (i > 0 && next[i - 1]),
                Tok::One => i > 0 && reach[i - 1],
                Tok::Lit(c) => i > 0 && reach[i - 1] && c.eq_ignore_ascii_case(&text[i - 1]),
            };
        }
        reach = next;
    }
    reach[text.len()]
}
