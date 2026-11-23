//! events: post an event with a start time, collect Going, Maybe and Can't, and remind the channel before it starts.
//! `lib.rs` is the ABI shim and request reader.
//!
//! Conforms to slim's module ABI v1 (see the deployment's docs/modules/building-modules.md): exports
//! `memory`, `alloc(len) -> ptr` and `run(in_ptr, in_len) -> packed(out_ptr, out_len)`. The one thing
//! it imports is `slim.host_call`, for `kv.store` and `message.post`. It never traps on bad input.

mod command;
mod event;
mod host;
mod scene;
mod store;
mod time;

use std::alloc::{alloc as std_alloc, Layout};

use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
struct Request {
    command: String,
    #[serde(default)]
    input: String,
    #[serde(default)]
    caller: Caller,
    #[serde(default)]
    entropy: String,
    #[serde(default)]
    now_ms: i64,
    #[serde(default)]
    context: Context,
}

#[derive(Deserialize, Default)]
struct Caller {
    #[serde(default)]
    id: String,
    /// The display name members see, sent by a host at add-on level 7 or newer.
    #[serde(default)]
    name: String,
}

#[derive(Deserialize, Default)]
struct Context {
    #[serde(default)]
    channel_id: String,
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

fn handle(host: &dyn host::Host, request: &[u8]) -> Vec<u8> {
    let parsed: Request = match serde_json::from_slice(request) {
        Ok(req) => req,
        Err(err) => return failure(format!("invalid request: {err}")),
    };
    let call = command::Call {
        command: &parsed.command,
        input: &parsed.input,
        caller: &parsed.caller.id,
        name: &parsed.caller.name,
        entropy: &parsed.entropy,
        now_ms: parsed.now_ms,
        channel: &parsed.context.channel_id,
    };
    match command::run(host, &call) {
        Ok(reply) => {
            let mut answer = json!({"ok": true, "output": reply.output});
            if let Some(run_after) = reply.run_after {
                answer["run_after"] = run_after;
            }
            serialize(&answer)
        }
        Err(message) => failure(message),
    }
}

fn failure(error: String) -> Vec<u8> {
    serialize(&json!({"ok": false, "error": error}))
}

fn serialize(answer: &Value) -> Vec<u8> {
    serde_json::to_vec(answer).unwrap_or_else(|_| br#"{"ok":false,"error":"failed to serialize the answer"}"#.to_vec())
}

fn raw_alloc(len: usize) -> *mut u8 {
    if len == 0 {
        return std::ptr::NonNull::<u8>::dangling().as_ptr();
    }
    let layout = Layout::from_size_align(len, 1).expect("valid layout");
    unsafe { std_alloc(layout) }
}

#[cfg(test)]
mod tests;
