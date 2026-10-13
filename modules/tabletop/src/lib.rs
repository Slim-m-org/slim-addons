//! tabletop: dice with advantage and keep/drop, an initiative tracker scene and
//! per-player character sheets. `lib.rs` is the ABI shim and command router.
//!
//! Conforms to slim's module ABI v1 (see the deployment's
//! docs/modules/building-modules.md): exports `memory`, `alloc(len) -> ptr`
//! and `run(in_ptr, in_len) -> packed(out_ptr, out_len)`. The one thing it
//! imports is `slim.host_call`, for `kv.store`. It never traps on bad input; it
//! always answers with `{"ok":true,"output":...}` or `{"ok":false,"error":...}`.

mod initiative;
mod kv;
mod notation;
mod rng;
mod roll;
mod scene;
mod sheet;

use std::alloc::{alloc as std_alloc, Layout};

use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Request {
    command: String,
    input: String,
    #[serde(default)]
    caller: Caller,
    /// Fresh per run from the host; absent from a host older than the field.
    #[serde(default)]
    entropy: String,
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
/// pointer to them. The host calls this before `run`, and `host_call` replies through it.
#[no_mangle]
pub extern "C" fn alloc(len: i32) -> i32 {
    raw_alloc(len.max(0) as usize) as i32
}

/// Runs the request written at `in_ptr`/`in_len` and returns a packed
/// `(out_ptr << 32) | out_len` pointing at the UTF-8 JSON response.
#[no_mangle]
pub extern "C" fn run(in_ptr: i32, in_len: i32) -> i64 {
    let request = unsafe { std::slice::from_raw_parts(in_ptr as *const u8, in_len as usize) };
    let body = handle(request);

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

fn handle(request: &[u8]) -> Vec<u8> {
    let parsed: Result<Request, _> = serde_json::from_slice(request);
    let parsed = match parsed {
        Ok(req) => req,
        Err(err) => return serialize_err(format!("invalid request: {err}")),
    };
    match route(&parsed) {
        Ok(output) => serialize_ok(output),
        Err(message) => serialize_err(message),
    }
}

fn route(request: &Request) -> Result<String, String> {
    match request.command.as_str() {
        "roll" => roll::run(&request.input, &request.entropy),
        "sheet" => sheet::run(&kv::HostKv, &request.caller.id, &request.input),
        "initiative" => scene::apply(&request.input, &request.entropy),
        other => Err(format!("unknown command: {other}")),
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
    use super::*;

    fn answer(command: &str, input: &str, caller: &str) -> serde_json::Value {
        let request = serde_json::json!({"command": command, "input": input, "caller": {"id": caller}, "entropy": "e"});
        serde_json::from_slice(&handle(request.to_string().as_bytes())).unwrap()
    }

    #[test]
    fn each_command_answers_ok_and_a_bad_one_answers_an_error() {
        assert_eq!(answer("roll", "adv+2", "")["ok"], true);
        assert_eq!(answer("initiative", "", "")["ok"], true);
        assert_eq!(answer("sheet", "help", "")["ok"], true);
        assert_eq!(answer("nope", "", "")["ok"], false);
        assert_eq!(answer("roll", "banana", "")["ok"], false);
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&handle(b"{")).unwrap()["ok"], false);
    }
}
