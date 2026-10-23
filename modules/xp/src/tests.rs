//! The whole module against an in-memory slim: events, catch-up, idempotence and the slots.

use serde_json::{json, Value};

use crate::fake::FakeHost;
use crate::handle;

const MIN: i64 = 60_000;

fn run(host: &FakeHost, command: &str, input: Value, settings: Value) -> Value {
    host.next_run();
    let request = json!({"command": command, "input": input.to_string(), "settings": settings, "now_ms": 1});
    serde_json::from_slice(&handle(host, request.to_string().as_bytes())).unwrap()
}

fn voice_event(host: &FakeHost) -> Value {
    run(host, "@event", json!({"event": "voice.session.closed", "id": "voice-session:1", "at_ms": 1, "data": {}}), json!({}))
}

fn message(host: &FakeHost, user: &str, at_ms: i64, is_bot: bool, settings: Value) -> Value {
    let data = json!({"author_id": user, "created_at_ms": at_ms, "is_bot": is_bot, "content": "hi"});
    run(host, "@event", json!({"event": "message.created", "id": "message:1", "at_ms": at_ms, "data": data}), settings)
}

#[test]
fn a_voice_event_awards_every_session_after_the_cursor() {
    let host = FakeHost::default();
    host.push_session(1, "ada", 0, 10 * MIN, 0);
    host.push_session(2, "bob", 0, 3 * MIN, MIN);
    assert_eq!(voice_event(&host)["ok"], true);
    assert_eq!((host.xp_of("ada"), host.xp_of("bob")), (50, 10));
    assert_eq!(host.cursor(), 2);
    assert_eq!(host.field("ada", "voice_ms"), 10 * MIN);
}

#[test]
fn the_same_sessions_are_never_awarded_twice() {
    let host = FakeHost::default();
    host.push_session(1, "ada", 0, 10 * MIN, 0);
    voice_event(&host);
    voice_event(&host);
    run(&host, "catchup", json!(""), json!({}));
    assert_eq!(host.xp_of("ada"), 50);
}

#[test]
fn a_lost_cursor_write_does_not_pay_a_session_again() {
    let host = FakeHost::default();
    host.push_session(1, "ada", 0, 10 * MIN, 0);
    voice_event(&host);
    host.meta.borrow_mut().clear();
    host.push_session(2, "ada", 0, 2 * MIN, 0);
    voice_event(&host);
    assert_eq!(host.xp_of("ada"), 60);
    assert_eq!(host.cursor(), 2);
}

#[test]
fn a_failed_write_keeps_the_cursor_so_the_next_pass_retries() {
    let host = FakeHost::default();
    host.push_session(1, "ada", 0, 10 * MIN, 0);
    *host.fail_op.borrow_mut() = Some("increment".into());
    assert_eq!(voice_event(&host)["ok"], false);
    assert_eq!(host.cursor(), 0);
    *host.fail_op.borrow_mut() = None;
    assert_eq!(voice_event(&host)["ok"], true);
    assert_eq!(host.cursor(), 1);
}

#[test]
fn catch_up_resumes_from_the_cursor_after_a_restart() {
    let host = FakeHost::default();
    host.push_session(1, "ada", 0, 5 * MIN, 0);
    run(&host, "catchup", json!(""), json!({}));
    host.push_session(2, "ada", 0, 5 * MIN, 0);
    host.push_session(3, "bob", 0, 5 * MIN, 0);
    let reply = run(&host, "catchup", json!(""), json!({}));
    assert_eq!(reply["output"], "Awarded 2 sessions.");
    assert_eq!((host.xp_of("ada"), host.xp_of("bob")), (50, 25));
}

#[test]
fn settings_decide_the_rate_and_whether_alone_time_counts() {
    let host = FakeHost::default();
    host.push_session(1, "ada", 0, 10 * MIN, 10 * MIN);
    run(&host, "catchup", json!(""), json!({"xp_per_minute": 2, "only_with_others": false}));
    assert_eq!(host.xp_of("ada"), 20);
    host.push_session(2, "ada", 0, 10 * MIN, 10 * MIN);
    run(&host, "catchup", json!(""), json!({}));
    assert_eq!(host.xp_of("ada"), 20);
}

#[test]
fn a_backlog_bigger_than_one_run_drains_over_several_without_loss() {
    let host = FakeHost::default();
    for id in 1..=40 {
        host.push_session(id, &format!("user{id}"), 0, 2 * MIN, 0);
    }
    let mut runs = 0;
    while host.cursor() < 40 {
        assert_eq!(run(&host, "catchup", json!(""), json!({}))["ok"], true);
        runs += 1;
        assert!(runs < 10, "the cursor stopped moving at {}", host.cursor());
    }
    assert!(runs > 1);
    assert!((1..=40).all(|id| host.xp_of(&format!("user{id}")) == 10));
}

#[test]
fn a_busy_member_with_many_sessions_is_one_batch() {
    let host = FakeHost::default();
    for id in 1..=150 {
        host.push_session(id, "ada", 0, MIN, 0);
    }
    run(&host, "catchup", json!(""), json!({}));
    assert_eq!(host.xp_of("ada"), 150 * 5);
    assert_eq!(host.cursor(), 150);
}

#[test]
fn a_message_pays_once_per_cooldown() {
    let host = FakeHost::default();
    let on = json!({"xp_per_message": 3, "message_cooldown_s": 60});
    assert_eq!(message(&host, "ada", 1_000_000, false, on.clone())["output"], "Paid.");
    assert_eq!(message(&host, "ada", 1_030_000, false, on.clone())["output"], "No XP.");
    assert_eq!(message(&host, "ada", 1_059_999, false, on.clone())["output"], "No XP.");
    assert_eq!(message(&host, "ada", 1_060_000, false, on.clone())["output"], "Paid.");
    assert_eq!(host.xp_of("ada"), 6);
    assert_eq!(host.field("ada", "messages"), 2);
}

#[test]
fn a_replayed_message_does_not_pay_again() {
    let host = FakeHost::default();
    let on = json!({"xp_per_message": 3});
    message(&host, "ada", 5_000_000, false, on.clone());
    message(&host, "ada", 5_000_000, false, on.clone());
    message(&host, "ada", 4_000_000, false, on);
    assert_eq!(host.xp_of("ada"), 3);
}

#[test]
fn the_cooldown_is_per_member() {
    let host = FakeHost::default();
    let on = json!({"xp_per_message": 2});
    message(&host, "ada", 1_000_000, false, on.clone());
    message(&host, "bob", 1_000_001, false, on);
    assert_eq!((host.xp_of("ada"), host.xp_of("bob")), (2, 2));
}

#[test]
fn bots_and_the_default_setting_earn_nothing_and_cost_nothing() {
    let host = FakeHost::default();
    message(&host, "bot", 1_000_000, true, json!({"xp_per_message": 5}));
    message(&host, "ada", 1_000_000, false, json!({}));
    assert!(host.xp.borrow().is_empty());
    assert_eq!(host.calls.get(), 0);
}

#[test]
fn unsubscribed_events_and_bad_input_do_not_trap() {
    let host = FakeHost::default();
    let other = json!({"event": "member.joined", "id": "x", "at_ms": 1, "data": {}});
    assert_eq!(run(&host, "@event", other, json!({}))["ok"], true);
    assert_eq!(run(&host, "@event", json!("{"), json!({}))["ok"], true);
    assert_eq!(run(&host, "nope", json!(""), json!({}))["ok"], false);
    assert_eq!(serde_json::from_slice::<Value>(&handle(&host, b"{")).unwrap()["ok"], false);
}

#[test]
fn a_slot_run_reports_levels_from_the_stored_totals() {
    let host = FakeHost::default();
    host.push_session(1, "ada", 0, 30 * MIN, 0);
    voice_event(&host);
    let ask = json!({"slot": "profile.section", "name": "xp", "user_ids": ["ada", "bob"], "viewer_id": "v"});
    let reply = run(&host, "@slot", ask, json!({}));
    let items: Value = serde_json::from_str(reply["output"].as_str().unwrap()).unwrap();
    assert_eq!(items["items"]["ada"]["fallback"], "Level 2");
    assert!(items["items"].get("bob").is_none());
}

#[test]
fn the_curve_setting_changes_the_level() {
    let host = FakeHost::with_xp(&[("ada", 300)]);
    let ask = json!({"slot": "member.badge", "name": "level", "user_ids": ["ada"], "viewer_id": "v"});
    let badge = |curve: &str| {
        let reply = run(&host, "@slot", ask.clone(), json!({"curve": curve}));
        let items: Value = serde_json::from_str(reply["output"].as_str().unwrap()).unwrap();
        items["items"]["ada"]["nodes"][0]["text"].as_str().unwrap().to_string()
    };
    assert_eq!((badge("gentle"), badge("standard"), badge("steep")), ("Lv 4".into(), "Lv 3".into(), "Lv 2".into()));
}
