//! The whole module against an in-memory slim: the four events, the transcript post, resuming a long
//! one, the silent-call rule and the prune.

use serde_json::{json, Value};

use crate::fake::FakeHost;
use crate::handle;

const SESSION: &str = "call-audio-s1";
const MIN: i64 = 60_000;
const DAY: i64 = 24 * 60 * MIN;

fn run(host: &FakeHost, command: &str, input: &str, settings: Value, now_ms: i64) -> Value {
    host.next_run();
    let request = json!({"command": command, "input": input, "settings": settings, "now_ms": now_ms});
    serde_json::from_slice(&handle(host, request.to_string().as_bytes())).unwrap()
}

fn event(host: &FakeHost, name: &str, data: Value, at_ms: i64, settings: Value) -> Value {
    let mut data = data;
    data["session_id"] = json!(SESSION);
    data["channel_id"] = json!("c1");
    let input = json!({"event": name, "id": "x", "at_ms": at_ms, "data": data}).to_string();
    run(host, "@event", &input, settings, at_ms)
}

fn started(host: &FakeHost, at: i64) {
    event(host, "call.audio.started", json!({"started_at_ms": at, "included": ["a", "b"]}), at, json!({}));
}

fn segment(host: &FakeHost, n: i64, user: &str, text: &str) -> Value {
    let data = json!({"segment_id": format!("call-audio:s:{n}"), "user_id": user, "start_ms": n * 1000, "end_ms": n * 1000 + 800, "text": text});
    event(host, "call.audio.segment", data, 1000 + n, json!({}))
}

fn stopped(host: &FakeHost, sent: i64, at: i64, settings: Value) -> Value {
    let data = json!({"stopped_at_ms": at, "reason": "stopped", "segments_sent": sent});
    event(host, "call.audio.stopped", data, at, settings)
}

#[test]
fn a_call_is_kept_segment_by_segment_and_posted_when_it_stops() {
    let host = FakeHost::default();
    started(&host, 1000);
    assert_eq!(segment(&host, 1, "u-ada", "hello everyone")["ok"], true);
    segment(&host, 2, "u-bob", "hi ada");
    segment(&host, 3, "u-ada", "shall we start");
    assert_eq!(host.segments.borrow().len(), 3);

    let reply = stopped(&host, 3, 5 * MIN, json!({}));
    assert_eq!(reply["ok"], true, "{reply}");
    let posts = host.posts.borrow();
    assert_eq!(posts.len(), 1);
    assert!(posts[0].contains("**Speaker 1:** hello everyone"), "{}", posts[0]);
    assert!(posts[0].contains("**Speaker 2:** hi ada\n**Speaker 1:** shall we start"), "{}", posts[0]);
    assert!(reply.get("run_after").is_none());
    let row = host.session(SESSION);
    assert_eq!((row["ended"].clone(), row["reason"].clone(), row["posted"].clone()), (json!(1), json!("stopped"), json!(1)));
}

#[test]
fn the_same_segment_delivered_twice_is_kept_once() {
    let host = FakeHost::default();
    started(&host, 1);
    segment(&host, 1, "u", "once");
    assert_eq!(segment(&host, 1, "u", "once")["ok"], true);
    assert_eq!(host.segments.borrow().len(), 1);
}

#[test]
fn the_member_column_marks_who_spoke() {
    let manifest: Value = serde_json::from_str(include_str!("../manifest.json")).unwrap();
    let tables = manifest["data"]["tables"].as_array().unwrap();
    let segments = tables.iter().find(|t| t["name"] == "segments").unwrap();
    let user = segments["columns"].as_array().unwrap().iter().find(|c| c["name"] == "user_id").unwrap();
    assert_eq!(user["member"], true);
}

#[test]
fn a_gap_is_reported_when_the_host_sent_more_than_arrived() {
    let host = FakeHost::default();
    started(&host, 1);
    segment(&host, 1, "u", "one");
    stopped(&host, 4, MIN, json!({}));
    assert!(host.posts.borrow()[0].contains("3 lines were missed"), "{}", host.posts.borrow()[0]);
}

#[test]
fn post_when_done_off_keeps_the_text_and_posts_nothing() {
    let host = FakeHost::default();
    started(&host, 1);
    segment(&host, 1, "u", "one");
    let reply = stopped(&host, 1, MIN, json!({"post_when_done": false}));
    assert_eq!(reply["ok"], true);
    assert!(host.posts.borrow().is_empty());
    assert_eq!(host.segments.borrow().len(), 1);
}

#[test]
fn a_long_transcript_posts_three_messages_and_asks_to_be_run_again_for_the_rest() {
    let host = FakeHost::default();
    started(&host, 1);
    for n in 1..=150 {
        segment(&host, n, if n % 2 == 0 { "a" } else { "b" }, &format!("this is sentence number {n} of a very long call that goes on"));
    }
    let reply = stopped(&host, 150, 30 * MIN, json!({}));
    assert_eq!(host.posts.borrow().len(), 3, "{reply}");
    let after = &reply["run_after"];
    assert_eq!(after["command"], "post");
    assert_eq!(after["input"], SESSION);

    let mut rounds = 0;
    let mut last = reply;
    while last.get("run_after").is_some() {
        rounds += 1;
        assert!(rounds < 10, "never finishes");
        last = run(&host, "post", SESSION, json!({}), (30 + rounds) * MIN);
        assert_eq!(last["ok"], true, "{last}");
    }
    assert!(host.posts.borrow().len() > 3);
    let all = host.posts.borrow().join("\n");
    for n in 1..=150 {
        assert_eq!(all.matches(&format!("sentence number {n} of")).count(), 1, "{n} must be posted exactly once");
    }
    assert_eq!(all.matches("Transcript of the call").count(), 1, "one heading");
    let row = host.session(SESSION);
    assert_eq!(row["posted"].as_i64().unwrap() as usize, host.posts.borrow().len());
}

#[test]
fn a_missing_posting_channel_is_explained_not_swallowed() {
    let host = FakeHost::default();
    started(&host, 1);
    host.no_poster.set(true);
    let reply = stopped(&host, 0, MIN, json!({}));
    assert_eq!(reply["ok"], false);
    assert!(reply["error"].as_str().unwrap().contains("Pick a channel"), "{reply}");
}

#[test]
fn a_call_silent_for_ten_minutes_counts_as_ended() {
    let host = FakeHost::default();
    started(&host, 1000);
    segment(&host, 1, "u", "last words");
    let early = run(&host, "tidy", "", json!({}), 5 * MIN);
    assert_eq!(early["output"], "Nothing to do.");
    assert!(host.posts.borrow().is_empty());

    let late = run(&host, "tidy", "", json!({}), 20 * MIN);
    assert_eq!(late["ok"], true, "{late}");
    assert_eq!(host.posts.borrow().len(), 1);
    assert!(host.posts.borrow()[0].contains("last words"));
    assert_eq!(host.session(SESSION)["reason"], "lost");
    assert_eq!(run(&host, "tidy", "", json!({}), 21 * MIN)["output"], "Nothing to do.");
}

#[test]
fn the_daily_prune_removes_old_transcripts_and_leaves_new_ones() {
    let host = FakeHost::default();
    started(&host, 1);
    segment(&host, 1, "u", "old");
    stopped(&host, 1, MIN, json!({"post_when_done": false}));
    let first = run(&host, "tidy", "", json!({"keep_days": 30}), 10 * DAY);
    assert!(first["output"].as_str().unwrap().contains("Pruned 0"), "{first}");
    assert_eq!(host.segments.borrow().len(), 1, "ten days old is kept for thirty");

    let second = run(&host, "tidy", "", json!({"keep_days": 30}), 40 * DAY);
    assert!(second["output"].as_str().unwrap().contains("Pruned 1"), "{second}");
    assert!(host.segments.borrow().is_empty());
    assert!(host.sessions.borrow().is_empty());
}

#[test]
fn the_prune_runs_once_a_day_not_every_tick() {
    let host = FakeHost::default();
    run(&host, "tidy", "", json!({}), 40 * DAY);
    let again = run(&host, "tidy", "", json!({}), 40 * DAY + 15 * MIN);
    assert_eq!(again["output"], "Nothing to do.");
}

#[test]
fn unknown_commands_and_bad_input_are_errors_not_traps() {
    let host = FakeHost::default();
    assert_eq!(run(&host, "nope", "", json!({}), 1)["ok"], false);
    assert_eq!(run(&host, "@event", "not json", json!({}), 1)["ok"], false);
    assert_eq!(handle(&host, b"garbage").is_empty(), false);
    let other = event(&host, "message.created", json!({}), 1, json!({}));
    assert_eq!(other["ok"], true);
}
