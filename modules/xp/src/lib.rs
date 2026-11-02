//! XP and levels: members earn XP for time in voice calls and show a level bar on their profile.
//! `lib.rs` is the ABI shim and command router.
//!
//! Conforms to slim's module ABI v1: exports `memory`, `alloc(len) -> ptr` and
//! `run(in_ptr, in_len) -> packed(out_ptr, out_len)`, and imports only `slim.host_call`
//! for `data.store` and `log.read`. It never traps on bad input.

mod award;
#[cfg(test)]
mod fake;
mod host;
mod levels;
mod settings;
mod slot;
mod store;
mod sync;

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
        "@event" => on_event(&store, &settings, &parsed.input),
        "@slot" => on_slot(&store, &settings, &parsed.input),
        "catchup" => sync::catch_up(&store, &settings).map(|r| format!("Awarded {} sessions.", r.sessions)),
        other => Err(format!("unknown command {other}")),
    };
    match answer {
        Ok(output) => json!({"ok": true, "output": output}).to_string().into_bytes(),
        Err(message) => err_body(message),
    }
}

/// A voice event is only a trigger for a catch-up; a message pays on its own.
fn on_event<H: Host>(store: &Store<H>, settings: &Settings, input: &str) -> Result<String, String> {
    let event: Value = serde_json::from_str(input).map_err(|_| "the event was not JSON".to_string())?;
    match event["event"].as_str() {
        Some("voice.session.closed") => sync::catch_up(store, settings).map(|r| format!("Awarded {} sessions.", r.sessions)),
        Some("message.created") => match sync::Message::from_event(&event["data"]) {
            Some(message) => sync::award_message(store, settings, &message).map(|paid| if paid { "Paid." } else { "No XP." }.to_string()),
            None => Err("the message event had no author or time".to_string()),
        },
        _ => Ok("Ignored.".to_string()),
    }
}

fn on_slot<H: Host>(store: &Store<H>, settings: &Settings, input: &str) -> Result<String, String> {
    let input: Value = serde_json::from_str(input).map_err(|_| "the slot request was not JSON".to_string())?;
    slot::answer(store, settings.curve, &input)
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
