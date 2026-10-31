//! minesweeper: a board launched as an app that a channel plays together, driven entirely by the
//! interactive scene loop, with a scoreboard in the module's own database. `lib.rs` is the ABI shim; the game is in `game.rs`.
//!
//! Conforms to slim's module ABI v1 (see the deployment's
//! docs/modules/building-modules.md): exports `memory`, `alloc(len) -> ptr`
//! and `run(in_ptr, in_len) -> packed(out_ptr, out_len)`, and imports only `slim.host_call`, for
//! the scoreboard's `data.store`.
//! It never traps on bad input; it always answers with
//! `{"ok":true,"output":...}` or `{"ok":false,"error":...}`.

mod board;
#[cfg(test)]
mod fake;
mod game;
mod host;
mod level;
mod rng;
mod scene;
mod scores;

use std::alloc::{alloc as std_alloc, Layout};

use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Request {
    command: String,
    input: String,
    #[serde(default)]
    caller: Caller,
    /// The host's clock when the run started; absent from a host that predates it.
    #[serde(default)]
    now_ms: i64,
}

#[derive(Deserialize, Default)]
struct Caller {
    #[serde(default)]
    id: String,
}

#[derive(Serialize)]
struct OkResponse {
    ok: bool,
    output: String,
}

#[derive(Serialize)]
struct ErrResponse {
    ok: bool,
    error: String,
}

/// Reserves `len` bytes in this module's own linear memory and returns a
/// pointer to them. The host calls this before `run`, to get somewhere to
/// write the request bytes.
#[no_mangle]
pub extern "C" fn alloc(len: i32) -> i32 {
    raw_alloc(len.max(0) as usize) as i32
}

/// Runs the request written at `in_ptr`/`in_len` and returns a packed
/// `(out_ptr << 32) | out_len` pointing at the UTF-8 JSON response.
#[no_mangle]
pub extern "C" fn run(in_ptr: i32, in_len: i32) -> i64 {
    let request = unsafe { std::slice::from_raw_parts(in_ptr as *const u8, in_len as usize) };
    let body = handle(&host::SlimHost, request);

    let out_len = body.len();
    let out_ptr = if out_len == 0 {
        raw_alloc(0)
    } else {
        let ptr = raw_alloc(out_len);
        unsafe { std::ptr::copy_nonoverlapping(body.as_ptr(), ptr, out_len) };
        ptr
    };

    let packed = ((out_ptr as u64) << 32) | (out_len as u64 & 0xFFFF_FFFF);
    packed as i64
}

fn handle<H: host::Host>(host: &H, request: &[u8]) -> Vec<u8> {
    let parsed: Result<Request, _> = serde_json::from_slice(request);
    let parsed = match parsed {
        Ok(req) => req,
        Err(err) => return serialize_err(format!("invalid request: {err}")),
    };

    match game::apply(host, &parsed.command, &parsed.input, &parsed.caller.id, parsed.now_ms) {
        Ok(output) => serialize_ok(output),
        Err(message) => serialize_err(message),
    }
}

fn serialize_ok(output: String) -> Vec<u8> {
    serde_json::to_vec(&OkResponse { ok: true, output })
        .unwrap_or_else(|_| br#"{"ok":false,"error":"failed to serialize output"}"#.to_vec())
}

fn serialize_err(error: String) -> Vec<u8> {
    serde_json::to_vec(&ErrResponse { ok: false, error })
        .unwrap_or_else(|_| br#"{"ok":false,"error":"failed to serialize error"}"#.to_vec())
}

fn raw_alloc(len: usize) -> *mut u8 {
    if len == 0 {
        return std::ptr::NonNull::<u8>::dangling().as_ptr();
    }
    let layout = Layout::from_size_align(len, 1).expect("valid layout");
    unsafe { std_alloc(layout) }
}

#[cfg(test)]
mod tests {
    use super::handle;

    /// What the host sends: the command, its input and the id of whoever tapped.
    fn call(input: &str, caller: &str) -> String {
        let request = serde_json::json!({"command": "play", "input": input, "caller": {"id": caller}});
        String::from_utf8(handle(&crate::fake::FakeHost::default(), request.to_string().as_bytes())).unwrap()
    }

    fn state_of(response: &str) -> String {
        let outer: serde_json::Value = serde_json::from_str(response).unwrap();
        let scene: serde_json::Value = serde_json::from_str(outer["output"].as_str().unwrap()).unwrap();
        scene["state"].as_str().unwrap().to_owned()
    }

    #[test]
    fn two_callers_launching_get_different_boards() {
        let (a, b) = (state_of(&call("", "aaaa")), state_of(&call("", "bbbb")));
        assert_ne!(a, b, "every launch started from the same seed");
    }

    #[test]
    fn one_caller_launching_twice_gets_the_same_board() {
        assert_eq!(state_of(&call("", "aaaa")), state_of(&call("", "aaaa")));
    }

    #[test]
    fn new_game_follows_the_caller_too() {
        let launch = state_of(&call("", "aaaa"));
        let again = |caller: &str| {
            let action = serde_json::json!({"action": "new game", "state": launch}).to_string();
            state_of(&call(&action, caller))
        };
        assert_ne!(again("aaaa"), again("bbbb"));
        assert_ne!(again("aaaa"), launch);
    }

    #[test]
    fn a_request_without_a_caller_still_launches() {
        let response = String::from_utf8(handle(&crate::fake::FakeHost::default(), br#"{"command":"play","input":""}"#)).unwrap();
        assert!(response.contains(r#""ok":true"#), "{response}");
    }
}
