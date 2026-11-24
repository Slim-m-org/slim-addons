//! The module's own record of each running preset board, so a win is scored from what this module
//! wrote and not from the state a client echoes back. The echoed state is only a lookup key.

use serde_json::{json, Value};

use crate::board::{Board, Move, View};
use crate::host::Host;

/// A record nobody has touched for this long is dropped the next time a game starts.
const KEEP_MS: i64 = 24 * 60 * 60 * 1000;

/// The record's key: a board keeps its seed once the first dig has rolled it.
fn key(board: &Board) -> String {
    format!("{}|{}", board.seed, board.level.code())
}

/// Whether anything beyond flags has happened, which is what a client could forge into a win.
fn progressed(board: &Board) -> bool {
    board.start_ms.is_some() || board.dead || board.revealed.iter().any(|r| *r)
}

fn call<H: Host>(host: &H, op: &str, rest: Value) -> Result<Value, String> {
    let mut request = json!({"capability": "data.store", "op": op, "table": "games"});
    if let (Some(base), Some(extra)) = (request.as_object_mut(), rest.as_object()) {
        base.extend(extra.clone());
    }
    host.call(&request)
}

/// The board to play on. Returns whether it can be scored: a preset board is trusted when it is
/// fresh or this module holds its record; a state with progress and no record is played unscored.
pub fn load<H: Host>(host: &H, echoed: Board) -> (Board, bool) {
    if echoed.level.preset_name().is_none() {
        return (echoed, false);
    }
    if !progressed(&echoed) {
        return (fresh(echoed), true);
    }
    let held = call(host, "get", json!({"where": [{"col": "k", "op": "eq", "val": key(&echoed)}]}));
    let stored = held.ok().and_then(|reply| reply["rows"][0]["state"].as_str().map(Board::from_state));
    match stored {
        Some(mut board) => {
            board.flagging = echoed.flagging;
            board.customizing = echoed.customizing;
            board.view = View::Board;
            (board, true)
        }
        None => {
            let mut board = echoed;
            board.unverifiable = true;
            (board, false)
        }
    }
}

/// A board nobody has dug on keeps only its setup and flags; credited players, a clock and a
/// move log are what the module writes, not what a client may bring.
fn fresh(mut board: Board) -> Board {
    board.players.clear();
    board.moves.retain(|m| matches!(m, Move::Flag(_)));
    board.unverifiable = false;
    board.scored = false;
    board.end_ms = None;
    board
}

/// Keeps `board` as the record for its key once a dig has started it; a no-op for a board that
/// `load` could not trust, so a forged state never becomes a record.
pub fn save<H: Host>(host: &H, board: &Board, trusted: bool, now_ms: i64) {
    if !trusted || !progressed(board) || board.level.preset_name().is_none() {
        return;
    }
    let first = board.moves.iter().filter(|m| matches!(m, Move::Dig(_))).count() <= 1;
    if first {
        let _ = call(host, "delete", json!({"where": [{"col": "at", "op": "lt", "val": now_ms - KEEP_MS}]}));
    }
    let values = json!({"k": key(board), "state": board.state(), "at": now_ms});
    let _ = call(host, "upsert", json!({"values": values, "on": "k"}));
}
