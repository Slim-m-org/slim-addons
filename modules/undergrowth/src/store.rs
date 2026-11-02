//! The module's own database, through `data.store`: the move log every frame replays, the
//! inbox the space's events fill, the season records and the profile links.

use serde_json::{json, Value};

use crate::host::Host;
use crate::moves::{Move, Weather};

/// The most moves and weather one frame takes in; the rest wait for the next frame.
const PAGE: u32 = 40;

pub struct Store<'a, H: Host> {
    host: &'a H,
}

fn cond(col: &str, op: &str, val: Value) -> Value {
    json!({"col": col, "op": op, "val": val})
}

impl<'a, H: Host> Store<'a, H> {
    pub fn new(host: &'a H) -> Self {
        Store { host }
    }

    fn data(&self, mut req: Value) -> Result<Value, String> {
        req["capability"] = json!("data.store");
        self.host.call(&req)
    }

    fn rows(reply: &Value) -> Vec<Value> {
        reply["rows"].as_array().cloned().unwrap_or_default()
    }

    pub fn log_move(&self, game: &str, mv: &Move, now_ms: i64) -> Result<i64, String> {
        let values = json!({"game": game, "pid": mv.pid, "act": mv.act, "arg": mv.arg, "at_ms": now_ms});
        let reply = self.data(json!({"op": "insert", "table": "moves", "values": values}))?;
        Ok(reply["rows"][0]["_id"].as_i64().unwrap_or(0))
    }

    /// Each move's id, when it was logged, and the move.
    pub fn moves_after(&self, game: &str, after: i64) -> Result<Vec<(i64, i64, Move)>, String> {
        let reply = self.data(json!({
            "op": "query", "table": "moves",
            "where": [cond("game", "eq", json!(game)), cond("_id", "gt", json!(after))],
            "order": [{"col": "_id", "dir": "asc"}], "limit": PAGE
        }))?;
        Ok(Self::rows(&reply)
            .iter()
            .map(|r| {
                let text = |k: &str| r[k].as_str().unwrap_or("").to_owned();
                (r["_id"].as_i64().unwrap_or(0), r["at_ms"].as_i64().unwrap_or(0), Move { pid: text("pid"), act: text("act"), arg: text("arg") })
            })
            .collect())
    }

    pub fn add_weather(&self, items: &[Weather], now_ms: i64) -> Result<(), String> {
        if items.is_empty() {
            return Ok(());
        }
        let rows: Vec<Value> =
            items.iter().map(|w| json!({"kind": w.kind, "at": w.at, "amount": w.amount, "at_ms": now_ms})).collect();
        self.data(json!({"op": "insert", "table": "inbox", "rows": rows})).map(|_| ())
    }

    pub fn weather_after(&self, after: i64) -> Result<Vec<(i64, Weather)>, String> {
        let reply = self.data(json!({
            "op": "query", "table": "inbox", "where": [cond("_id", "gt", json!(after))],
            "order": [{"col": "_id", "dir": "asc"}], "limit": PAGE
        }))?;
        Ok(Self::rows(&reply)
            .iter()
            .map(|r| {
                let n = |k: &str| r[k].as_i64().unwrap_or(0).clamp(0, i64::from(u32::MAX)) as u32;
                (r["_id"].as_i64().unwrap_or(0), Weather { kind: r["kind"].as_str().unwrap_or("").to_owned(), at: n("at"), amount: n("amount") })
            })
            .collect())
    }

    /// The newest inbox row, so a new game starts after the weather that came before it.
    pub fn weather_head(&self) -> Result<i64, String> {
        let reply = self.data(json!({"op": "query", "table": "inbox", "order": [{"col": "_id", "dir": "desc"}], "limit": 1}))?;
        Ok(Self::rows(&reply).first().and_then(|r| r["_id"].as_i64()).unwrap_or(0))
    }

    /// Makes sure the game has a row, without touching what it holds.
    fn ensure_game(&self, game: &str) -> Result<(), String> {
        self.data(json!({"op": "upsert", "table": "games", "on": "game", "values": {"game": game}, "add": {"ended": 0, "posted": 0}}))
            .map(|_| ())
    }

    /// Writes standings, or claims the season's end too; only while `season` has not been claimed.
    fn write_standings(&self, game: &str, (season, tick, end): (u32, u32, u32), (standings, now_ms): (&str, i64), claim: bool) -> Result<bool, String> {
        self.ensure_game(game)?;
        let mut values = json!({"season": season, "tick": tick, "end_tick": end, "standings": standings, "updated_ms": now_ms});
        if claim {
            values["ended"] = json!(season);
        }
        let reply = self.data(json!({
            "op": "update", "table": "games", "values": values,
            "where": [cond("game", "eq", json!(game)), cond("ended", "lt", json!(season))]
        }))?;
        Ok(reply["changed"].as_i64() == Some(1))
    }

    /// Standings of a season still being played; a season already claimed as ended keeps the ones it ended with.
    pub fn save_standings(&self, game: &str, clock: (u32, u32, u32), standings: &str, now_ms: i64) -> Result<(), String> {
        self.write_standings(game, clock, (standings, now_ms), false).map(|_| ())
    }

    /// Marks `season` of `game` ended with these final standings; true only for the one run that got there first.
    pub fn claim_season_end(&self, game: &str, clock: (u32, u32, u32), standings: &str, now_ms: i64) -> Result<bool, String> {
        self.write_standings(game, clock, (standings, now_ms), true)
    }

    /// Marks the ended season's results posted; true only for the one run that should post them.
    pub fn claim_posting(&self, game: &str) -> Result<Option<Value>, String> {
        let reply = self.data(json!({"op": "get", "table": "games", "where": [cond("game", "eq", json!(game))]}))?;
        let Some(row) = Self::rows(&reply).into_iter().next() else { return Ok(None) };
        let ended = row["ended"].as_i64().unwrap_or(0);
        if ended == 0 || row["posted"].as_i64().unwrap_or(0) >= ended {
            return Ok(None);
        }
        let claim = self.data(json!({
            "op": "update", "table": "games", "values": {"posted": ended},
            "where": [cond("game", "eq", json!(game)), cond("posted", "lt", json!(ended))]
        }))?;
        Ok((claim["changed"].as_i64() == Some(1)).then_some(row))
    }

    pub fn latest_game(&self) -> Result<Option<Value>, String> {
        let reply = self.data(json!({"op": "query", "table": "games", "order": [{"col": "updated_ms", "dir": "desc"}], "limit": 1}))?;
        Ok(Self::rows(&reply).into_iter().next())
    }

    /// Adds one finished season to a player's record.
    pub fn record_season(&self, pid: &str, name: &str, (won, cells, mushrooms): (bool, u32, u32), now_ms: i64) -> Result<(), String> {
        let add = json!({"seasons": 1, "wins": i64::from(won), "mushrooms": mushrooms, "best_cells": 0});
        self.data(json!({"op": "upsert", "table": "players", "on": "pid", "values": {"pid": pid, "name": name, "last_ms": now_ms}, "add": add}))?;
        self.data(json!({
            "op": "update", "table": "players", "values": {"best_cells": cells},
            "where": [cond("pid", "eq", json!(pid)), cond("best_cells", "lt", json!(cells))]
        }))
        .map(|_| ())
    }

    pub fn player(&self, pid: &str) -> Result<Option<Value>, String> {
        let reply = self.data(json!({"op": "get", "table": "players", "where": [cond("pid", "eq", json!(pid))]}))?;
        Ok(Self::rows(&reply).into_iter().next())
    }

    pub fn players_of_users(&self, users: &[String]) -> Result<Vec<Value>, String> {
        let reply = self.data(json!({"op": "query", "table": "players", "where": [cond("user_id", "in", json!(users))], "limit": 100}))?;
        Ok(Self::rows(&reply))
    }

    pub fn feeders(&self, users: &[String]) -> Result<Vec<Value>, String> {
        let reply = self.data(json!({"op": "query", "table": "feeders", "where": [cond("user_id", "in", json!(users))], "limit": 100}))?;
        Ok(Self::rows(&reply))
    }

    pub fn feed(&self, user: &str, (leaves, warm_min): (i64, i64)) -> Result<(), String> {
        let add = json!({"leaves": leaves, "warm_min": warm_min});
        self.data(json!({"op": "upsert", "table": "feeders", "on": "user_id", "values": {"user_id": user}, "add": add}))
            .map(|_| ())
    }

    pub fn fact(&self, name: &str) -> Result<Option<Value>, String> {
        let reply = self.data(json!({"op": "get", "table": "facts", "where": [cond("name", "eq", json!(name))]}))?;
        Ok(Self::rows(&reply).into_iter().next())
    }

    pub fn set_fact(&self, name: &str, val: &str, num: i64) -> Result<(), String> {
        self.data(json!({"op": "upsert", "table": "facts", "on": "name", "values": {"name": name, "val": val, "num": num}}))
            .map(|_| ())
    }

    /// Moves a numeric fact from `from` to `to`; true only when this run moved it.
    pub fn advance_fact(&self, name: &str, from: i64, to: i64) -> Result<bool, String> {
        self.data(json!({"op": "upsert", "table": "facts", "on": "name", "values": {"name": name}, "add": {"num": 0}}))?;
        let reply = self.data(json!({
            "op": "update", "table": "facts", "values": {"num": to},
            "where": [cond("name", "eq", json!(name)), cond("num", "eq", json!(from))]
        }))?;
        Ok(reply["changed"].as_i64() == Some(1))
    }

    pub fn add_link(&self, code: &str, pid: &str, now_ms: i64) -> Result<(), String> {
        self.data(json!({"op": "insert", "table": "links", "values": {"code": code, "pid": pid, "at_ms": now_ms}})).map(|_| ())
    }

    /// The player a fresh link code names, used up as it is read.
    pub fn take_link(&self, code: &str, now_ms: i64) -> Result<Option<String>, String> {
        let reply = self.data(json!({"op": "get", "table": "links", "where": [cond("code", "eq", json!(code))]}))?;
        let Some(row) = Self::rows(&reply).into_iter().next() else { return Ok(None) };
        self.data(json!({"op": "delete", "table": "links", "where": [cond("code", "eq", json!(code))]}))?;
        let fresh = now_ms - row["at_ms"].as_i64().unwrap_or(0) <= crate::LINK_TTL_MS;
        Ok(fresh.then(|| row["pid"].as_str().unwrap_or("").to_owned()))
    }

    pub fn link_user(&self, pid: &str, user: &str) -> Result<(), String> {
        self.data(json!({"op": "upsert", "table": "players", "on": "pid", "values": {"pid": pid, "user_id": user}, "add": {"seasons": 0}}))
            .map(|_| ())
    }

    pub fn count_since(&self, kind: &str, since_ms: i64) -> Result<i64, String> {
        let reply = self.data(json!({
            "op": "count", "table": "inbox", "where": [cond("kind", "eq", json!(kind)), cond("at_ms", "gt", json!(since_ms))]
        }))?;
        Ok(reply["count"].as_i64().unwrap_or(0))
    }

    pub fn sum_since(&self, kind: &str, since_ms: i64) -> Result<i64, String> {
        let reply = self.data(json!({
            "op": "sum", "table": "inbox", "col": "amount", "where": [cond("kind", "eq", json!(kind)), cond("at_ms", "gt", json!(since_ms))]
        }))?;
        Ok(reply["count"].as_i64().unwrap_or(0))
    }

    pub fn prune(&self, table: &str, before_ms: i64) -> Result<i64, String> {
        let reply = self.data(json!({"op": "delete", "table": table, "where": [cond("at_ms", "lt", json!(before_ms))]}))?;
        Ok(reply["changed"].as_i64().unwrap_or(0))
    }

    pub fn post(&self, content: &str) -> Result<(), String> {
        self.host.call(&json!({"capability": "message.post", "content": content})).map(|_| ())
    }

    pub fn voice_sessions(&self, after: i64, limit: u32) -> Result<(Vec<Value>, i64), String> {
        let reply = self.host.call(&json!({"capability": "log.read", "log": "voice.sessions", "after": after, "limit": limit}))?;
        Ok((Self::rows(&reply), reply["next"].as_i64().unwrap_or(after)))
    }
}
