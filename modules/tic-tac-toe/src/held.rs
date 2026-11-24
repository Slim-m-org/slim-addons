//! The module's own record of each running game, so the board and the two seats come from what this
//! module wrote and not from the state a client echoes back. The echoed state is only a lookup key.

use serde_json::{json, Value};

use crate::host::Host;

/// A record nobody has touched for this long is dropped the next time a game starts.
const KEEP_MS: i64 = 24 * 60 * 60 * 1000;

/// A game id: sixteen hex characters folded from who started it, when, and the game it replaces.
pub fn new_id(caller: &str, now_ms: i64, previous: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in format!("{caller}|{now_ms}|{previous}").bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// An id is lowercase hex, which also keeps it from breaking the `|` framing of the state.
pub fn valid_id(id: &str) -> bool {
    id.len() == 16 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn call<H: Host>(host: &H, op: &str, rest: Value) -> Result<Value, String> {
    let mut request = json!({"capability": "data.store", "op": op, "table": "games"});
    if let (Some(base), Some(extra)) = (request.as_object_mut(), rest.as_object()) {
        base.extend(extra.clone());
    }
    host.call(&request)
}

/// The state this module last wrote for `id`; `None` when it holds no such game.
pub fn load<H: Host>(host: &H, id: &str) -> Result<Option<String>, String> {
    let reply = call(host, "get", json!({"where": [{"col": "k", "op": "eq", "val": id}]}))?;
    Ok(reply["rows"][0]["state"].as_str().map(str::to_owned))
}

/// Keeps `state` as the record for `id`. A game's first move claims its id and takes another when
/// a different game already holds it, so two launches by one caller never share a record; the id
/// that ended up holding the game is returned.
pub fn store<H: Host>(host: &H, id: &str, state: &dyn Fn(&str) -> String, first: bool, now_ms: i64) -> Result<String, String> {
    let row = |id: &str| json!({"k": id, "state": state(id), "at": now_ms});
    if !first {
        call(host, "upsert", json!({"values": row(id), "on": "k"}))?;
        return Ok(id.to_owned());
    }
    call(host, "delete", json!({"where": [{"col": "at", "op": "lt", "val": now_ms - KEEP_MS}]}))?;
    let mut id = id.to_owned();
    for _ in 0..3 {
        match call(host, "insert", json!({"values": row(&id)})) {
            Ok(_) => return Ok(id),
            Err(why) if why.contains("already holds") => id = new_id(&id, now_ms, "taken"),
            Err(why) => return Err(why),
        }
    }
    Err("could not find a free place to keep the game".to_owned())
}
