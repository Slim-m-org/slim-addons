//! The ABI edge: what slim sends in and what it reads back.

use super::*;
use crate::fake::FakeHost;

#[test]
fn a_request_that_is_not_json_is_an_error_not_a_trap() {
    let reply: Value = serde_json::from_slice(&handle(&FakeHost::default(), b"\xff not json")).unwrap();
    assert_eq!(reply["ok"], false);
    assert!(reply["error"].as_str().unwrap().starts_with("invalid request"));
}

#[test]
fn a_request_with_no_caller_or_clock_still_launches() {
    let reply: Value = serde_json::from_slice(&handle(&FakeHost::default(), br#"{"command":"undergrowth"}"#)).unwrap();
    assert_eq!(reply["ok"], true);
    assert!(reply["output"].as_str().unwrap().contains("scene/1"));
    assert!(reply.get("run_after").is_none());
}

#[test]
fn settings_reach_the_world_it_deals() {
    let req = json!({"command": "undergrowth", "input": "", "entropy": "abc", "now_ms": 1, "settings": {"world": "small", "redraw_s": 9}});
    let reply: Value = serde_json::from_slice(&handle(&FakeHost::default(), req.to_string().as_bytes())).unwrap();
    let scene: Value = serde_json::from_str(reply["output"].as_str().unwrap()).unwrap();
    assert_eq!(scene["refresh_ms"], 9000);
    let grid = scene["ops"].as_array().unwrap().iter().find(|o| o["op"] == "cells").unwrap().clone();
    assert_eq!((grid["cols"].as_u64(), grid["rows"].as_u64()), (Some(48), Some(30)));
}
