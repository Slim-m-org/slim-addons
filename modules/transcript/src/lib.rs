//! Call transcript: keeps what a consented call said and posts it as a readable transcript when the call ends.
//! `lib.rs` is the ABI shim and command router.
//!
//! Conforms to slim's module ABI v1: exports `memory`, `alloc(len) -> ptr` and
//! `run(in_ptr, in_len) -> packed(out_ptr, out_len)`, and imports only `slim.host_call`
//! for `data.store` and `message.post`. It never traps on bad input.

#[cfg(test)]
mod fake;
mod command;
mod host;
mod ingest;
mod post;
mod render;
mod settings;
mod store;

use std::alloc::{alloc as std_alloc, Layout};

use serde::Deserialize;
use serde_json::{json, Value};

use host::Host;
use settings::Settings;
use store::Store;

#[derive(Deserialize)]
struct Request {
    command: String,
    #[serde(default)]
    input: String,
    #[serde(default)]
    settings: Value,
    /// The host's clock, UTC epoch milliseconds; zero when the host gives none.
    #[serde(default)]
    now_ms: i64,
}

/// Reserves `len` bytes in this module's own linear memory and returns a pointer to them.
/// The host calls this before `run`, and `host_call` replies through it.
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

    let ptr = raw_alloc(body.len());
    unsafe { std::ptr::copy_nonoverlapping(body.as_ptr(), ptr, body.len()) };
    (((ptr as u64) << 32) | (body.len() as u64 & 0xFFFF_FFFF)) as i64
}

fn handle<H: Host>(host: &H, request: &[u8]) -> Vec<u8> {
    let parsed: Request = match serde_json::from_slice(request) {
        Ok(req) => req,
        Err(err) => return err_body(format!("invalid request: {err}")),
    };
    let settings = Settings::from_value(&parsed.settings);
    let store = Store::new(host);
    let answer = match parsed.command.as_str() {
        "@event" => ingest::on_event(&store, &settings, &parsed.input),
        "transcript" => command::transcript(&store, &settings),
        "flush" => command::flush(&store, &settings, parsed.now_ms),
        other => Err(format!("unknown command {other}")),
    };
    match answer {
        Ok(output) => json!({"ok": true, "output": output}).to_string().into_bytes(),
        Err(message) => err_body(message),
    }
}

fn err_body(error: String) -> Vec<u8> {
    json!({"ok": false, "error": error}).to_string().into_bytes()
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
