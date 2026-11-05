//! Undergrowth: a fungal turf war living under a slim space. `lib.rs` is the ABI shim and the
//! command router.
//!
//! Conforms to slim's module ABI v1: exports `memory`, `alloc(len) -> ptr` and
//! `run(in_ptr, in_len) -> packed(out_ptr, out_len)`, and imports only `slim.host_call` for
//! `data.store`, `log.read` and `message.post`. It never traps on bad input.

mod b64;
mod colony;
mod events;
#[cfg(test)]
mod fake;
mod game;
mod herald;
mod host;
mod moves;
mod panel;
mod play;
mod png;
mod rng;
mod scene;
mod settings;
mod sim;
mod slot;
mod song;
mod store;
mod words;
mod world;

use std::alloc::{alloc as std_alloc, Layout};

use serde::Deserialize;
use serde_json::{json, Value};

use host::Host;
use settings::Settings;
use store::Store;

/// How long a profile link code stays good.
pub const LINK_TTL_MS: i64 = 15 * 60_000;

#[derive(Deserialize)]
struct Request {
    command: String,
    #[serde(default)]
    input: String,
    #[serde(default)]
    caller: Option<Caller>,
    #[serde(default)]
    entropy: String,
    #[serde(default)]
    now_ms: i64,
    #[serde(default)]
    settings: Value,
}

#[derive(Deserialize)]
struct Caller {
    #[serde(default)]
    id: String,
}

/// Reserves `len` bytes in this module's own linear memory and returns a pointer to them.
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

pub(crate) fn handle<H: Host>(host: &H, request: &[u8]) -> Vec<u8> {
    let req: Request = match serde_json::from_slice(request) {
        Ok(req) => req,
        Err(err) => return err_body(format!("invalid request: {err}")),
    };
    let settings = Settings::from_value(&req.settings);
    let store = Store::new(host);
    let caller = req.caller.as_ref().map_or("", |c| c.id.as_str());
    let answer = match req.command.as_str() {
        "undergrowth" => {
            let run = play::Run { caller, now_ms: req.now_ms, entropy: &req.entropy, settings: &settings };
            let answer = play::surface(&store, &req.input, &run);
            let mut body = json!({"ok": true, "output": answer.output});
            if let Some(timer) = answer.run_after {
                body["run_after"] = timer;
            }
            return body.to_string().into_bytes();
        }
        "herald" => herald::herald(&store, &req.input),
        "almanac" => herald::almanac(&store, &settings, req.now_ms),
        "@event" => events::on_event(&store, &settings, &req.input, req.now_ms),
        "@slot" => slot::answer(&store, &req.input),
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
