//! Word filter: masks or blocks the words and patterns an admin lists, before a message
//! is stored. `lib.rs` is the ABI shim and command router.
//!
//! Conforms to slim's module ABI v1: exports `memory`, `alloc(len) -> ptr` and
//! `run(in_ptr, in_len) -> packed(out_ptr, out_len)`, and imports nothing.
//! It never traps on bad input; a rule that does not compile is an `ok:false`, which the
//! server counts and shows on the module's health card.

mod intercept;
mod rules;
mod weight;
mod words;

use std::alloc::{alloc as std_alloc, Layout};

use serde::Deserialize;
use serde_json::{json, Value};

use rules::Rules;

#[derive(Deserialize)]
struct Request {
    command: String,
    #[serde(default)]
    input: String,
    #[serde(default)]
    settings: Value,
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
    let body = handle(request);

    let ptr = raw_alloc(body.len());
    unsafe { std::ptr::copy_nonoverlapping(body.as_ptr(), ptr, body.len()) };
    (((ptr as u64) << 32) | (body.len() as u64 & 0xFFFF_FFFF)) as i64
}

fn handle(request: &[u8]) -> Vec<u8> {
    let parsed: Request = match serde_json::from_slice(request) {
        Ok(req) => req,
        Err(err) => return err_body(format!("invalid request: {err}")),
    };
    let answer = Rules::from_settings(&parsed.settings).and_then(|rules| match parsed.command.as_str() {
        "@intercept" => intercept::answer(&rules, &parsed.input),
        "filter-test" => Ok(test_text(&rules, &parsed.input)),
        other => Err(format!("unknown command {other}")),
    });
    match answer {
        Ok(output) => json!({"ok": true, "output": output}).to_string().into_bytes(),
        Err(message) => err_body(message),
    }
}

/// Lets an admin try text against the current rules without sending it.
fn test_text(rules: &Rules, text: &str) -> String {
    if text.trim().is_empty() {
        return "Type some text after the command to see what the filter does with it.".into();
    }
    if !rules.matches(text) {
        return "Nothing in that is filtered.".into();
    }
    match rules.action {
        rules::Action::Mask => format!("It would be sent as: {}", rules.mask(text)),
        rules::Action::Block => format!("It would be refused: {}", rules.reason),
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
mod tests {
    use super::*;

    fn run_json(request: Value) -> Value {
        serde_json::from_slice(&handle(request.to_string().as_bytes())).unwrap()
    }

    #[test]
    fn a_broken_rule_fails_the_run_so_the_server_counts_it() {
        let out = run_json(json!({"command": "@intercept", "input": "{}", "settings": {"patterns": "("}}));
        assert_eq!(out["ok"], json!(false));
    }

    #[test]
    fn the_test_command_shows_the_masked_text() {
        let out = run_json(json!({"command": "filter-test", "input": "oh darn", "settings": {"words": "darn"}}));
        assert_eq!(out["output"], json!("It would be sent as: oh ****"));
    }
}
