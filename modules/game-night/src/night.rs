//! A game night and what members said about it, kept in `data.store`.
//!
//! `nights` holds the plan and when to remind; `votes` holds one row per member per time they tapped
//! (`night|member|s<i>`, unique, so a second tap is a refused insert that turns into a delete) and one
//! row per member for their answer (`night|member|r`, an upsert, so the last answer wins).

use serde_json::{json, Value};

use crate::host::Host;
use crate::parse::Slot;

pub const OPEN: i64 = -1;
pub const CANCELLED: i64 = -2;
const UNIQUE_CLASH: &str = "unique column";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Night {
    pub id: String,
    pub game: String,
    pub role: String,
    pub host: String,
    pub slots: Vec<Slot>,
    pub chosen: i64,
    pub remind_at: i64,
    pub created: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    In = 0,
    Maybe = 1,
    Out = 2,
}

impl Answer {
    pub fn parse(word: &str) -> Option<Answer> {
        match word {
            "in" => Some(Answer::In),
            "maybe" => Some(Answer::Maybe),
            "out" => Some(Answer::Out),
            _ => None,
        }
    }
}

/// What a viewer sees of a night: the tallies, and what they themselves picked.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Tally {
    pub per_slot: Vec<i64>,
    pub answers: [i64; 3],
    pub mine_slots: Vec<usize>,
    pub mine_answer: Option<i64>,
}

pub struct Store<'a, H: Host> {
    host: &'a H,
}

impl<'a, H: Host> Store<'a, H> {
    pub fn new(host: &'a H) -> Self {
        Store { host }
    }

    fn data(&self, mut request: Value) -> Result<Value, String> {
        request["capability"] = json!("data.store");
        self.host.call(&request)
    }

    pub fn create(&self, night: &Night) -> Result<(), String> {
        let slots: Vec<Value> = night.slots.iter().map(|s| json!({"l": s.label, "t": s.start_ms})).collect();
        let values = json!({"night_id": night.id, "game": night.game, "role": night.role, "host": night.host,
            "slots": Value::Array(slots).to_string(), "chosen": OPEN, "remind_at": 0, "reminded": false, "created": night.created});
        self.data(json!({"op": "insert", "table": "nights", "values": values})).map(|_| ())
    }

    pub fn load(&self, id: &str) -> Result<Option<Night>, String> {
        let reply = self.data(json!({"op": "get", "table": "nights", "where": [{"col": "night_id", "op": "eq", "val": id}]}))?;
        Ok(reply["rows"].get(0).and_then(night_of))
    }

    pub fn tally(&self, night: &Night, viewer: &str) -> Result<Tally, String> {
        let mut tally = Tally::default();
        for index in 0..night.slots.len() {
            tally.per_slot.push(self.count(&night.id, "slot", Some(index as i64))?);
        }
        for (index, answer) in [Answer::In, Answer::Maybe, Answer::Out].into_iter().enumerate() {
            tally.answers[index] = self.count(&night.id, "answer", Some(answer as i64))?;
        }
        let mine = self.data(json!({"op": "query", "table": "votes", "where": [
            {"col": "night_id", "op": "eq", "val": night.id}, {"col": "voter", "op": "eq", "val": viewer}]}))?;
        for row in mine["rows"].as_array().into_iter().flatten() {
            match (row["kind"].as_str(), row["val"].as_i64()) {
                (Some("slot"), Some(v)) if v >= 0 => tally.mine_slots.push(v as usize),
                (Some("answer"), v) => tally.mine_answer = v,
                _ => {}
            }
        }
        Ok(tally)
    }

    fn count(&self, id: &str, kind: &str, val: Option<i64>) -> Result<i64, String> {
        let mut filters = vec![json!({"col": "night_id", "op": "eq", "val": id}), json!({"col": "kind", "op": "eq", "val": kind})];
        filters.extend(val.map(|v| json!({"col": "val", "op": "eq", "val": v})));
        let reply = self.data(json!({"op": "count", "table": "votes", "where": filters}))?;
        Ok(reply["count"].as_i64().unwrap_or(0))
    }

    /// Taps a time: votes for it, or takes the vote back when it was already cast.
    pub fn toggle_slot(&self, night: &str, voter: &str, slot: usize) -> Result<(), String> {
        let key = format!("{night}|{voter}|s{slot}");
        let row = json!({"vote_key": key, "night_id": night, "voter": voter, "kind": "slot", "val": slot});
        match self.data(json!({"op": "insert", "table": "votes", "values": row})) {
            Ok(_) => Ok(()),
            Err(why) if why.contains(UNIQUE_CLASH) => {
                self.data(json!({"op": "delete", "table": "votes", "where": [{"col": "vote_key", "op": "eq", "val": key}]})).map(|_| ())
            }
            Err(why) => Err(why),
        }
    }

    pub fn answer(&self, night: &str, voter: &str, answer: Answer) -> Result<(), String> {
        let row = json!({"vote_key": format!("{night}|{voter}|r"), "night_id": night, "voter": voter, "kind": "answer", "val": answer as i64});
        self.data(json!({"op": "upsert", "table": "votes", "on": "vote_key", "values": row})).map(|_| ())
    }

    /// Settles on a time, once: false when the night was already closed or cancelled.
    pub fn close(&self, id: &str, chosen: usize, remind_at: Option<i64>) -> Result<bool, String> {
        let values = json!({"chosen": chosen, "remind_at": remind_at.unwrap_or(0), "reminded": remind_at.is_none()});
        self.settle(id, values, OPEN)
    }

    pub fn cancel(&self, id: &str) -> Result<bool, String> {
        let reply = self.data(json!({"op": "update", "table": "nights", "values": {"chosen": CANCELLED}, "where": [
            {"col": "night_id", "op": "eq", "val": id}, {"col": "chosen", "op": "gte", "val": OPEN}]}))?;
        Ok(reply["changed"].as_i64() == Some(1))
    }

    fn settle(&self, id: &str, values: Value, from: i64) -> Result<bool, String> {
        let reply = self.data(json!({"op": "update", "table": "nights", "values": values, "where": [
            {"col": "night_id", "op": "eq", "val": id}, {"col": "chosen", "op": "eq", "val": from}]}))?;
        Ok(reply["changed"].as_i64() == Some(1))
    }

    /// Chosen nights whose reminder time has come and that have not been reminded.
    pub fn due(&self, now_ms: i64, limit: i64) -> Result<Vec<Night>, String> {
        let reply = self.data(json!({"op": "query", "table": "nights", "limit": limit, "order": [{"col": "remind_at", "dir": "asc"}], "where": [
            {"col": "chosen", "op": "gte", "val": 0}, {"col": "reminded", "op": "eq", "val": false}, {"col": "remind_at", "op": "lte", "val": now_ms}]}))?;
        Ok(reply["rows"].as_array().into_iter().flatten().filter_map(night_of).collect())
    }

    /// Takes the reminder for a night; false when another run already has.
    pub fn claim_reminder(&self, id: &str) -> Result<bool, String> {
        let reply = self.data(json!({"op": "update", "table": "nights", "values": {"reminded": true}, "where": [
            {"col": "night_id", "op": "eq", "val": id}, {"col": "reminded", "op": "eq", "val": false}]}))?;
        Ok(reply["changed"].as_i64() == Some(1))
    }

    /// Forgets nights created before `before_ms` along with their votes, a few at a time.
    pub fn prune(&self, before_ms: i64, limit: i64) -> Result<i64, String> {
        let old = self.data(json!({"op": "query", "table": "nights", "limit": limit, "where": [{"col": "created", "op": "lt", "val": before_ms}]}))?;
        let mut gone = 0;
        for row in old["rows"].as_array().into_iter().flatten() {
            let id = row["night_id"].as_str().unwrap_or_default();
            self.data(json!({"op": "delete", "table": "votes", "where": [{"col": "night_id", "op": "eq", "val": id}]}))?;
            self.data(json!({"op": "delete", "table": "nights", "where": [{"col": "night_id", "op": "eq", "val": id}]}))?;
            gone += 1;
        }
        Ok(gone)
    }
}

fn night_of(row: &Value) -> Option<Night> {
    let slots: Vec<Value> = serde_json::from_str(row["slots"].as_str()?).ok()?;
    let slots = slots.iter().map(|s| Some(Slot { label: s["l"].as_str()?.to_owned(), start_ms: s["t"].as_i64()? })).collect::<Option<Vec<_>>>()?;
    Some(Night {
        id: row["night_id"].as_str()?.to_owned(),
        game: row["game"].as_str()?.to_owned(),
        role: row["role"].as_str().unwrap_or_default().to_owned(),
        host: row["host"].as_str()?.to_owned(),
        slots,
        chosen: row["chosen"].as_i64()?,
        remind_at: row["remind_at"].as_i64()?,
        created: row["created"].as_i64()?,
    })
}
