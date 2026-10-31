//! The scoreboard: each player's best time per preset level, and the name they chose to show,
//! kept in the module's own database. The module is told only an opaque caller id, so a name is
//! something a player types, not something the host supplies.

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::host::Host;

/// Names are short and plain: no control or direction characters, nothing a row could be spoofed with.
pub const MAX_NAME: usize = 16;
pub const SHOWN: i64 = 10;
/// Host calls one run may spend on data.store and kv.store together.
const BUDGET: u32 = 64;

pub struct Scores<'a, H: Host> {
    host: &'a H,
    used: std::cell::Cell<u32>,
}

/// One scoreboard line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub player: String,
    pub ms: i64,
}

impl<'a, H: Host> Scores<'a, H> {
    pub fn new(host: &'a H) -> Self {
        Scores { host, used: std::cell::Cell::new(0) }
    }

    fn data(&self, op: &str, table: &str, rest: Value) -> Result<Value, String> {
        if self.used.get() >= BUDGET {
            return Err("this run used all its data calls".to_owned());
        }
        self.used.set(self.used.get() + 1);
        let mut request = json!({"capability": "data.store", "op": op, "table": table});
        if let (Some(base), Some(extra)) = (request.as_object_mut(), rest.as_object()) {
            base.extend(extra.clone());
        }
        self.host.call(&request).map_err(|e| explain(&e))
    }

    /// Keeps `ms` for `player` on `level` when it beats what they had; true when the board changed.
    pub fn record(&self, level: &str, player: &str, ms: i64, seed: u64, now_ms: i64) -> Result<bool, String> {
        let key = format!("{level}|{player}");
        let held = self.data("get", "best", json!({"where": [eq("k", &key)]}))?;
        let row = &held["rows"][0];
        let seed = seed as i64;
        if row.is_null() {
            let values = json!({"k": key, "lvl": level, "player": player, "ms": ms, "seed": seed, "won_at": now_ms});
            return match self.data("insert", "best", json!({"values": values})) {
                Err(e) if e.contains("already holds") => Ok(false),
                other => other.map(|_| true),
            };
        }
        if row["ms"].as_i64().is_some_and(|best| ms < best) {
            let values = json!({"ms": ms, "seed": seed, "won_at": now_ms});
            self.data("update", "best", json!({"values": values, "where": [eq("k", &key)]}))?;
            return Ok(true);
        }
        Ok(false)
    }

    /// The fastest times on `level`, best first.
    pub fn top(&self, level: &str, limit: i64) -> Result<Vec<Entry>, String> {
        let reply = self.data("query", "best", json!({
            "where": [eq("lvl", level)], "order": [{"col": "ms", "dir": "asc"}], "limit": limit,
        }))?;
        let rows = reply["rows"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        Ok(rows
            .iter()
            .filter_map(|r| Some(Entry { player: r["player"].as_str()?.to_owned(), ms: r["ms"].as_i64()? }))
            .collect())
    }

    /// The names players chose, for those who chose one.
    pub fn names(&self, players: &[String]) -> Result<HashMap<String, String>, String> {
        if players.is_empty() {
            return Ok(HashMap::new());
        }
        let reply = self.data("query", "names", json!({"where": [{"col": "player", "op": "in", "val": players}], "limit": 100}))?;
        let rows = reply["rows"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        Ok(rows
            .iter()
            .filter_map(|r| Some((r["player"].as_str()?.to_owned(), r["name"].as_str()?.to_owned())))
            .collect())
    }

    pub fn set_name(&self, player: &str, name: &str) -> Result<(), String> {
        let values = json!({"player": player, "name": name});
        self.data("upsert", "names", json!({"values": values, "on": "player"})).map(|_| ())
    }
}

/// A name a player typed, trimmed and cut to length; None when nothing printable is left.
pub fn clean_name(typed: &str) -> Option<String> {
    let kept: String = typed
        .chars()
        .filter(|c| !c.is_control() && !is_direction_mark(*c))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let name: String = kept.chars().take(MAX_NAME).collect();
    let name = name.trim().to_owned();
    (!name.is_empty()).then_some(name)
}

fn is_direction_mark(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
}

fn eq(col: &str, val: &str) -> Value {
    json!({"col": col, "op": "eq", "val": val})
}

/// Host refusals in words the admin reading the Dock can act on.
fn explain(host_error: &str) -> String {
    if host_error.contains("not approved") {
        "An admin needs to approve Minesweeper to keep its scoreboard (stored data) in the Dock.".to_owned()
    } else {
        format!("the scoreboard could not be reached ({host_error})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeHost;

    #[test]
    fn a_first_time_is_kept_a_slower_one_is_not_and_a_faster_one_replaces_it() {
        let host = FakeHost::default();
        let scores = Scores::new(&host);
        assert_eq!(scores.record("beginner", "ann", 90_000, 7, 1), Ok(true));
        assert_eq!(scores.record("beginner", "ann", 95_000, 8, 2), Ok(false));
        assert_eq!(host.best_ms("beginner", "ann"), Some(90_000));
        assert_eq!(scores.record("beginner", "ann", 80_000, 9, 3), Ok(true));
        assert_eq!(host.best_ms("beginner", "ann"), Some(80_000));
        assert_eq!(host.best.borrow().len(), 1, "one row per player and level");
    }

    #[test]
    fn each_level_and_each_player_has_a_row_of_their_own() {
        let host = FakeHost::default();
        let scores = Scores::new(&host);
        scores.record("beginner", "ann", 90_000, 1, 1).unwrap();
        scores.record("expert", "ann", 400_000, 1, 1).unwrap();
        scores.record("beginner", "bob", 70_000, 1, 1).unwrap();
        assert_eq!(host.best.borrow().len(), 3);
        assert_eq!(host.best_ms("expert", "ann"), Some(400_000));
    }

    #[test]
    fn the_top_is_fastest_first_limited_and_only_that_level() {
        let host = FakeHost::default();
        let scores = Scores::new(&host);
        for (player, ms) in [("a", 50_000), ("b", 30_000), ("c", 40_000)] {
            scores.record("beginner", player, ms, 1, 1).unwrap();
        }
        scores.record("expert", "z", 1_000, 1, 1).unwrap();
        let top = scores.top("beginner", 2).unwrap();
        assert_eq!(top, vec![Entry { player: "b".into(), ms: 30_000 }, Entry { player: "c".into(), ms: 40_000 }]);
        assert!(scores.top("intermediate", 10).unwrap().is_empty());
    }

    #[test]
    fn a_name_is_stored_replaced_and_read_back_for_those_who_have_one() {
        let host = FakeHost::default();
        let scores = Scores::new(&host);
        scores.set_name("ann", "Ann").unwrap();
        scores.set_name("ann", "Annie").unwrap();
        scores.set_name("bob", "Bob").unwrap();
        assert_eq!(host.names.borrow().len(), 2);
        let names = scores.names(&["ann".to_owned(), "zed".to_owned()]).unwrap();
        assert_eq!(names.get("ann").map(String::as_str), Some("Annie"));
        assert!(!names.contains_key("zed"));
        assert!(scores.names(&[]).unwrap().is_empty());
        assert_eq!(host.calls.get(), 4, "asking for nobody makes no call");
        assert_eq!(host.name_of("bob").as_deref(), Some("Bob"));
    }

    #[test]
    fn a_typed_name_is_cleaned_and_cut() {
        assert_eq!(clean_name("  Ada   Lovelace  ").as_deref(), Some("Ada Lovelace"));
        assert_eq!(clean_name("a\u{0}b\u{202E}c\u{200B}d").as_deref(), Some("abcd"));
        assert_eq!(clean_name(&"x".repeat(40)).map(|n| n.chars().count()), Some(MAX_NAME));
        assert_eq!(clean_name("  \t\n "), None);
        assert_eq!(clean_name("\u{202E}\u{2066}"), None);
    }

    #[test]
    fn a_host_that_has_not_approved_data_store_is_explained() {
        let host = FakeHost::default();
        host.refuse.set(true);
        let why = Scores::new(&host).top("beginner", 10).unwrap_err();
        assert!(why.contains("An admin needs to approve"), "{why}");
    }

    #[test]
    fn a_run_stops_at_its_call_budget_instead_of_being_refused_by_the_host() {
        let host = FakeHost::default();
        let scores = Scores::new(&host);
        for _ in 0..BUDGET {
            scores.names(&["a".to_owned()]).unwrap();
        }
        assert!(scores.names(&["a".to_owned()]).is_err());
        assert_eq!(host.calls.get(), BUDGET);
    }
}
