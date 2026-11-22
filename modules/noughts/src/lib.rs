//! noughts: noughts and crosses kept as a room's shared state (slim-m decision
//! 0131). Each move is a `@room` input; the host holds the board between
//! moves, runs one move at a time, and sends every new board to the whole call.
//! `lib.rs` is the ABI shim; the game is in `game.rs`.
//!
//! Conforms to slim's module ABI v1: exports `memory`, `alloc(len) -> ptr` and
//! `run(in_ptr, in_len) -> packed(out_ptr, out_len)`, and imports nothing.

mod game;
mod room;
mod scene;

use std::alloc::{alloc as std_alloc, Layout};

use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Request {
    command: String,
    input: String,
    #[serde(default)]
    caller: Caller,
}

#[derive(Deserialize, Default)]
struct Caller {
    #[serde(default)]
    id: String,
}

#[derive(Serialize)]
struct Response {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[no_mangle]
pub extern "C" fn alloc(len: i32) -> i32 {
    raw_alloc(len.max(0) as usize) as i32
}

#[no_mangle]
pub extern "C" fn run(in_ptr: i32, in_len: i32) -> i64 {
    let request = unsafe { std::slice::from_raw_parts(in_ptr as *const u8, in_len as usize) };
    let body = handle(request);
    let out_len = body.len();
    let out_ptr = raw_alloc(out_len.max(1));
    unsafe { std::ptr::copy_nonoverlapping(body.as_ptr(), out_ptr, out_len) };
    (((out_ptr as u64) << 32) | (out_len as u64 & 0xFFFF_FFFF)) as i64
}

fn handle(request: &[u8]) -> Vec<u8> {
    let response = match serde_json::from_slice::<Request>(request) {
        Ok(req) if req.command == room::COMMAND => Response {
            ok: true,
            output: Some(room::answer(&req.input, &req.caller.id)),
            error: None,
        },
        Ok(req) => Response {
            ok: false,
            output: None,
            error: Some(format!("unknown command: {}", req.command)),
        },
        Err(err) => Response {
            ok: false,
            output: None,
            error: Some(format!("invalid request: {err}")),
        },
    };
    serde_json::to_vec(&response)
        .unwrap_or_else(|_| br#"{"ok":false,"error":"failed to serialize"}"#.to_vec())
}

fn raw_alloc(len: usize) -> *mut u8 {
    let layout = Layout::from_size_align(len.max(1), 1).expect("valid layout");
    unsafe { std_alloc(layout) }
}
