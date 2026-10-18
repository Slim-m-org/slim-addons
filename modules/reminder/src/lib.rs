//! reminder: say when and what, and the channel is told when it is time.
//! `lib.rs` is the ABI shim and command router.
//!
//! Conforms to slim's module ABI v1 (see the deployment's
//! docs/modules/building-modules.md): exports `memory`, `alloc(len) -> ptr`
//! and `run(in_ptr, in_len) -> packed(out_ptr, out_len)`. The one thing it
//! imports is `slim.host_call`, for `kv.store` and `message.post`. It never traps on bad input;
//! it always answers with `{"ok":true,"output":...}` or `{"ok":false,"error":...}`.

mod command;
#[cfg(test)]
mod fake;
mod host;
mod parse;
mod store;

use std::alloc::{alloc as std_alloc, Layout};

use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Request {
    command: String,
    #[serde(default)]
    input: String,
    /// Fresh randomness for this run; makes two reminders set in one millisecond differ.
    #[serde(default)]
    entropy: String,
    /// The host's clock, UTC epoch milliseconds; zero when the host gives none.
    #[serde(default)]
    now_ms: i64,
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
    let answer = command::run(host, &parsed.command, &parsed.input, (parsed.now_ms, &parsed.entropy));
    match answer {
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
    use super::*;
    use crate::fake::FakeHost;

    fn answer(host: &FakeHost, command: &str, input: &str, now_ms: i64) -> serde_json::Value {
        let request = serde_json::json!({"command": command, "input": input, "caller": {"id": "c"}, "entropy": "abc123", "now_ms": now_ms});
        serde_json::from_slice(&handle(host, request.to_string().as_bytes())).unwrap()
    }

    #[test]
    fn each_command_answers_in_the_abi_shape() {
        let host = FakeHost::default();
        let now = 1_700_000_000_000;
        assert_eq!(answer(&host, "remind", "5m a", now)["ok"], true);
        assert!(answer(&host, "list", "", now)["output"].as_str().unwrap().contains("a"));
        assert_eq!(answer(&host, "due", "", now + 6 * 60_000)["output"], "Posted 1 reminder.");
        assert_eq!(answer(&host, "cancel", "1", now)["ok"], false);
    }

    #[test]
    fn a_bad_command_or_request_answers_an_error() {
        let host = FakeHost::default();
        assert_eq!(answer(&host, "nope", "", 1)["ok"], false);
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&handle(&host, b"{")).unwrap()["ok"], false);
    }

    #[test]
    fn a_request_from_before_the_clock_existed_still_parses() {
        let host = FakeHost::default();
        let old = br#"{"command":"list","input":"","caller":{"id":"c"}}"#;
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&handle(&host, old)).unwrap()["output"], "No reminders waiting.");
    }
}
