//! The flows end to end through the ABI's request reader, against an in-memory host.

use serde_json::{json, Value};

use crate::host::fake::FakeHost;
use crate::{handle, store};

const NOW: i64 = 1_764_000_000_000; // 2025-11-24 16:00 UTC
const HOUR: i64 = 3_600_000;
const DAY: i64 = 24 * HOUR;

fn call(host: &FakeHost, command: &str, input: &str, caller: &str, now_ms: i64) -> Value {
    call_in(host, command, input, caller, now_ms, "chan-1")
}

fn call_in(host: &FakeHost, command: &str, input: &str, caller: &str, now_ms: i64, channel: &str) -> Value {
    let mut who = json!({"id": caller});
    if let Some(name) = name_of(caller) {
        who["name"] = json!(name);
    }
    let request = json!({"command": command, "input": input, "caller": who, "entropy": "abcdef0123456789", "now_ms": now_ms, "context": {"channel_id": channel}});
    serde_json::from_slice(&handle(host, request.to_string().as_bytes())).unwrap()
}

/// The display name slim would send for these callers; the others come with none.
fn name_of(caller: &str) -> Option<&'static str> {
    match caller {
        "host-1" => Some("Hana"),
        "bob-id" => Some("Bob"),
        "carol-id" => Some("Carol"),
        _ => None,
    }
}

fn press(host: &FakeHost, id: &str, action: &str, caller: &str, now_ms: i64) -> Value {
    call(host, "event", &json!({"action": action, "state": id}).to_string(), caller, now_ms)
}

fn scene(answer: &Value) -> Value {
    assert_eq!(answer["ok"], true, "{answer}");
    serde_json::from_str(answer["output"].as_str().unwrap()).unwrap()
}

fn texts(answer: &Value) -> Vec<String> {
    scene(answer)["ops"].as_array().unwrap().iter().filter_map(|o| o["s"].as_str().map(str::to_owned)).collect()
}

/// An event planned by `host-1` for `when`, returning its id.
fn plan(host: &FakeHost, when: &str) -> String {
    let answer = call(host, "event", &format!("Game night | {when} | Voice 1"), "host-1", NOW);
    scene(&answer)["state"].as_str().unwrap().to_owned()
}

fn stored(host: &FakeHost, id: &str) -> crate::event::Event {
    store::load(host, id).unwrap().unwrap()
}

#[test]
fn launching_with_nothing_typed_is_the_form() {
    let host = FakeHost::default();
    let launched = scene(&call(&host, "event", "", "a", NOW));
    assert_eq!(launched["controls"], json!([]));
    assert!(launched["ops"].as_array().unwrap().iter().any(|o| o["op"] == "input" && o["submit"] == "plan"));
}

#[test]
fn submitting_the_form_plans_the_event_the_same_way_typing_the_command_does() {
    let host = FakeHost::default();
    let answer = press(&host, "", "plan:Game night | in 3h | Voice 1", "host-1", NOW);
    assert!(texts(&answer).contains(&"starts in 3h 0m".to_owned()));
    assert_eq!(store::ids(&host).unwrap().len(), 1);
    let again = press(&host, "", "plan:nonsense", "host-1", NOW);
    assert!(scene(&again)["status"].as_str().unwrap().contains("title | when"));
}

#[test]
fn planning_posts_the_event_counts_down_and_arms_the_reminder() {
    let host = FakeHost::default();
    let answer = call(&host, "event", "Game night | in 3h | Voice 1", "host-1", NOW);
    let shown = texts(&answer);
    assert!(shown.contains(&"Game night".to_owned()) && shown.contains(&"starts in 3h 0m".to_owned()) && shown.contains(&"at Voice 1".to_owned()), "{shown:?}");
    assert!(shown.contains(&"Going (1)".to_owned()), "the host is going");
    let id = scene(&answer)["state"].as_str().unwrap().to_owned();
    assert_eq!(answer["run_after"], json!({"command": "tick", "key": id, "input": id, "after_ms": 2 * HOUR}));
    let event = stored(&host, &id);
    assert_eq!((event.n, event.at, event.ch.as_str()), (1, NOW + 3 * HOUR, "chan-1"));
}

#[test]
fn a_bad_plan_shows_the_form_with_what_was_wrong_and_stores_nothing() {
    let host = FakeHost::default();
    for (text, wrong) in [("Game night", "title | when"), (" | in 2h", "needs a title"), ("Game | someday", "when:"), ("Game | in 2h | a | b", "title | when"),
        ("Game | 2025-11-01 10:00", "already passed")] {
        let answer = call(&host, "event", text, "a", NOW);
        assert!(scene(&answer)["status"].as_str().unwrap().contains(wrong), "{text}: {answer}");
        assert!(answer.get("run_after").is_none());
    }
    assert!(store::ids(&host).unwrap().is_empty());
}

#[test]
fn people_answer_change_their_answer_and_see_who_is_going_by_name() {
    let host = FakeHost::default();
    let id = plan(&host, "in 5h");
    press(&host, &id, "Going", "bob-id", NOW);
    press(&host, &id, "Maybe", "carol-id", NOW);
    let frame = press(&host, &id, "Can't", "bob-id", NOW);
    assert_eq!(scene(&frame)["status"], "1 going, 1 maybe, 1 can't");
    assert!(texts(&frame).contains(&"Maybe (1)".to_owned()));
    let shown = texts(&press(&host, &id, "Going", "bob-id", NOW));
    assert!(shown.contains(&"Hana, Bob".to_owned()), "{shown:?}");
}

#[test]
fn a_member_slim_sent_no_name_for_is_listed_by_a_short_id_and_a_later_name_replaces_it() {
    let host = FakeHost::default();
    let id = plan(&host, "in 5h");
    let shown = texts(&press(&host, &id, "Going", "dave-id", NOW));
    assert!(shown.contains(&"Hana, member dave".to_owned()), "{shown:?}");
    let request = json!({"command": "event", "input": json!({"action": "Going", "state": id}).to_string(), "caller": {"id": "dave-id", "name": "Dave"}, "now_ms": NOW});
    let again: Value = serde_json::from_slice(&handle(&host, request.to_string().as_bytes())).unwrap();
    assert!(texts(&again).contains(&"Hana, Dave".to_owned()));
    assert!(!texts(&press(&host, &id, "refresh", "dave-id", NOW)).contains(&"nobody yet".to_owned()));
}

#[test]
fn the_old_name_box_is_gone_and_a_name_action_just_redraws() {
    let host = FakeHost::default();
    let id = plan(&host, "in 5h");
    let frame = press(&host, &id, "Going", "bob-id", NOW);
    assert!(scene(&frame)["ops"].as_array().unwrap().iter().all(|o| o["submit"] != "name"));
    let redrawn = press(&host, &id, "name:Hacker", "bob-id", NOW);
    assert!(!texts(&redrawn).contains(&"Hacker".to_owned()));
}

#[test]
fn an_event_that_has_started_or_was_cancelled_stops_taking_answers() {
    let host = FakeHost::default();
    let id = plan(&host, "in 5h");
    let late = press(&host, &id, "Going", "bob", NOW + 5 * HOUR);
    assert!(late["error"].as_str().unwrap().contains("has started"));
    assert!(texts(&press(&host, &id, "refresh", "bob", NOW + 5 * HOUR + 600_000)).contains(&"started 10m ago".to_owned()));
    let other = plan(&host, "in 6h");
    press(&host, &other, CANCEL, "host-1", NOW);
    assert!(press(&host, &other, "Going", "bob", NOW)["error"].as_str().unwrap().contains("cancelled"));
}

const CANCEL: &str = crate::scene::CANCEL;

#[test]
fn only_the_host_cancels_and_cancelling_clears_the_reminder() {
    let host = FakeHost::default();
    let id = plan(&host, "in 5h");
    assert!(press(&host, &id, CANCEL, "bob", NOW)["error"].as_str().unwrap().contains("only the host"));
    assert!(!stored(&host, &id).x);
    let done = press(&host, &id, CANCEL, "host-1", NOW);
    assert!(texts(&done).contains(&"cancelled".to_owned()));
    assert_eq!(done["run_after"], json!({"command": "tick", "key": id, "cancel": true}));
    assert_eq!(call(&host, "tick", &id, "bot", NOW + 5 * HOUR)["output"], "Nothing to remind.");
}

#[test]
fn the_host_edits_the_time_and_the_reminder_moves_with_it() {
    let host = FakeHost::default();
    let id = plan(&host, "in 5h");
    let moved = press(&host, &id, "edit: | in 8h | ", "host-1", NOW);
    assert_eq!(moved["run_after"]["after_ms"], 7 * HOUR);
    let event = stored(&host, &id);
    assert_eq!((event.t.as_str(), event.w.as_str(), event.at), ("Game night", "Voice 1", NOW + 8 * HOUR));
    let renamed = press(&host, &id, "edit:Board games", "host-1", NOW);
    assert!(texts(&renamed).contains(&"Board games".to_owned()));
    assert!(press(&host, &id, "edit:Hijack", "bob", NOW)["error"].as_str().unwrap().contains("only the host"));
    assert!(press(&host, &id, "edit: | soon", "host-1", NOW)["error"].as_str().unwrap().contains("when:"));
    assert_eq!(stored(&host, &id).at, NOW + 8 * HOUR);
}

#[test]
fn an_edit_after_the_reminder_went_out_sends_a_new_one() {
    let host = FakeHost::default();
    let id = plan(&host, "in 5h");
    call(&host, "tick", &id, "bot", NOW + 4 * HOUR);
    assert_eq!(host.posts.borrow().len(), 1);
    press(&host, &id, "edit: | in 9h", "host-1", NOW + 4 * HOUR);
    call(&host, "tick", &id, "bot", NOW + 12 * HOUR);
    assert_eq!(host.posts.borrow().len(), 2);
}

#[test]
fn the_reminder_mentions_everyone_going_by_caller_id_and_only_them() {
    let host = FakeHost::default();
    let id = plan(&host, "2025-11-25 20:00 +02:00");
    let bob = "b".repeat(64);
    press(&host, &id, "Going", &bob, NOW);
    press(&host, &id, "Maybe", "carol-id", NOW);
    let due = stored(&host, &id).rem;
    assert_eq!(due, NOW + 25 * HOUR);
    let early = call(&host, "tick", &id, "bot", NOW + HOUR);
    assert_eq!((early["output"].clone(), early["run_after"]["after_ms"].clone()), (json!("Not yet."), json!(due - NOW - HOUR)));
    assert!(host.posts.borrow().is_empty());
    assert_eq!(call(&host, "tick", &id, "bot", due)["output"], "Posted the reminder.");
    let posts = host.posts.borrow().clone();
    assert_eq!(posts.len(), 1);
    let post = &posts[0];
    assert!(post.starts_with("Reminder: Game night (#1) starts in 1h 0m at Voice 1, Tue 25 Nov 2025, 20:00 UTC+02:00. 2 going: {{mention:host-1}} {{mention:"), "{post}");
    assert!(post.contains(&format!("{{{{mention:{bob}}}}}")) && !post.contains("carol"), "{post}");
    assert!(!post.contains('@') && !post.contains("Bob") && !post.contains("Hana"), "names are the host's to write: {post}");
    assert_eq!(call(&host, "tick", &id, "bot", due + 1000)["output"], "Nothing to remind.");
    assert_eq!(host.posts.borrow().len(), 1);
}

#[test]
fn a_reminder_mentions_at_most_eight_and_counts_the_rest() {
    let host = FakeHost::default();
    let id = plan(&host, "in 5h");
    for i in 0..11 {
        press(&host, &id, "Going", &format!("guest-{i:02}"), NOW);
    }
    call(&host, "tick", &id, "bot", stored(&host, &id).rem);
    let post = host.posts.borrow()[0].clone();
    assert_eq!(post.matches("{{mention:").count(), 8, "{post}");
    assert!(post.contains("12 going:") && post.contains("and 4 more."), "{post}");
}

#[test]
fn a_reminder_that_cannot_be_posted_is_not_marked_sent() {
    let host = FakeHost { refuse_posts: true, ..FakeHost::default() };
    let id = plan(&host, "in 5h");
    let failed = call(&host, "tick", &id, "bot", NOW + 4 * HOUR);
    assert_eq!(failed["ok"], false);
    assert!(failed["error"].as_str().unwrap().contains("could not post the reminder"));
    assert!(!stored(&host, &id).r);
}

#[test]
fn an_event_months_away_re_arms_in_week_long_steps_and_one_starting_soon_gets_a_short_reminder() {
    let host = FakeHost::default();
    let far = call(&host, "event", "Convention | in 60d | Hall", "host-1", NOW);
    assert_eq!(far["run_after"]["after_ms"], 7 * DAY);
    let id = scene(&far)["state"].as_str().unwrap().to_owned();
    assert_eq!(call(&host, "tick", &id, "bot", NOW + 7 * DAY)["run_after"]["after_ms"], 7 * DAY);
    let soon = call(&host, "event", "Quick one | in 40m", "host-1", NOW);
    assert_eq!(soon["run_after"]["after_ms"], 30 * 60_000);
    let now = call(&host, "event", "Right now | in 5m", "host-1", NOW);
    assert_eq!(now["run_after"], json!({"command": "tick", "key": scene(&now)["state"], "cancel": true}));
}

#[test]
fn many_events_in_one_channel_each_keep_their_own_answers_and_numbers() {
    let host = FakeHost::default();
    let ids: Vec<String> = (0..3).map(|i| scene(&call_in(&host, "event", &format!("Event {i} | in {}h", i + 5), "host-1", NOW, "chan-1"))["state"].as_str().unwrap().to_owned()).collect();
    assert_eq!(ids.len(), 3);
    assert_ne!(ids[0], ids[1]);
    press(&host, &ids[1], "Going", "bob", NOW);
    assert_eq!(stored(&host, &ids[0]).counts(), [1, 0, 0]);
    assert_eq!(stored(&host, &ids[1]).counts(), [2, 0, 0]);
    assert_eq!(stored(&host, &ids[2]).n, 3);
}

#[test]
fn the_store_has_a_ceiling_and_old_events_are_swept_to_make_room() {
    let host = FakeHost::default();
    let mut first = String::new();
    for i in 0..store::MAX_EVENTS {
        let request = json!({"command": "event", "input": format!("E{i} | in 2h"), "caller": {"id": "h"}, "entropy": format!("entropy{i:04}xyz"), "now_ms": NOW});
        let answer: Value = serde_json::from_slice(&handle(&host, request.to_string().as_bytes())).unwrap();
        if i == 0 {
            first = scene(&answer)["state"].as_str().unwrap().to_owned();
        }
    }
    let full = call(&host, "event", "One more | in 2h", "h", NOW);
    assert_eq!(full["ok"], false);
    assert!(full["error"].as_str().unwrap().contains("already planned"));
    let later = NOW + 3 * DAY;
    assert_eq!(call(&host, "tick", "", "bot", later)["output"], format!("Removed {} finished events.", store::MAX_EVENTS));
    assert!(store::load(&host, &first).unwrap().is_none());
    assert_eq!(call(&host, "event", "Fresh | in 2h", "h", later)["ok"], true);
}

#[test]
fn a_frame_for_a_removed_event_says_so() {
    let host = FakeHost::default();
    let gone = press(&host, "nosuchid", "Going", "bob", NOW);
    assert_eq!(scene(&gone)["controls"], json!([]));
    assert!(texts(&gone).contains(&"this event has been removed".to_owned()));
}

#[test]
fn managers_list_cancel_and_edit_events_by_number_in_their_channel() {
    let host = FakeHost::default();
    plan(&host, "in 5h");
    call_in(&host, "event", "Elsewhere | in 6h", "host-2", NOW, "chan-2");
    let listed = call(&host, "event-admin", "", "mgr", NOW);
    let output = listed["output"].as_str().unwrap();
    assert!(output.starts_with("#1 Game night - ") && output.contains("starts in 5h 0m - 1 going") && !output.contains("Elsewhere"), "{output}");
    let edited = call(&host, "event-admin", "edit #1 Renamed | | ", "mgr", NOW);
    assert!(edited["output"].as_str().unwrap().starts_with("Updated #1: Renamed."), "{edited}");
    let cancelled = call(&host, "event-admin", "cancel 1", "mgr", NOW);
    assert!(cancelled["output"].as_str().unwrap().starts_with("Cancelled #1"));
    assert_eq!(cancelled["run_after"]["cancel"], true);
    assert!(call(&host, "event-admin", "cancel 1", "mgr", NOW)["error"].as_str().unwrap().contains("already cancelled"));
    assert!(call(&host, "event-admin", "cancel 2", "mgr", NOW)["error"].as_str().unwrap().contains("no event #2 here"));
    assert!(call(&host, "event-admin", "cancel soon", "mgr", NOW)["error"].as_str().unwrap().contains("by its number"));
    assert!(call(&host, "event-admin", "frobnicate", "mgr", NOW)["error"].as_str().unwrap().contains("use: /event-admin"));
    assert!(call_in(&host, "event-admin", "", "mgr", NOW, "chan-3")["output"].as_str().unwrap().contains("No events"));
}

#[test]
fn the_host_and_older_servers_are_handled_without_trapping() {
    let host = FakeHost::default();
    assert_eq!(serde_json::from_slice::<Value>(&handle(&host, b"{")).unwrap()["ok"], false);
    let old = br#"{"command":"event","input":"","caller":{"id":"c"}}"#;
    assert_eq!(serde_json::from_slice::<Value>(&handle(&host, old)).unwrap()["ok"], true);
    assert_eq!(call(&host, "nope", "", "a", NOW)["ok"], false);
    let nobody = call(&host, "event", "Game | in 2h", "", NOW);
    assert_eq!(nobody["ok"], false);
}

#[test]
fn an_event_no_index_names_is_listed_counted_and_pruned() {
    let host = FakeHost::default();
    let id = plan(&host, "in 2h");
    let mut lost = stored(&host, &id);
    lost.n = 7;
    store::save(&host, "racer", &lost).unwrap();
    assert_eq!(store::all(&host).unwrap().len(), 2);
    let listed = call(&host, "event-admin", "list", "bot", NOW);
    assert!(listed["output"].as_str().unwrap().contains("#7"), "{listed}");
    for i in 0..store::MAX_EVENTS - 2 {
        store::save(&host, &format!("filler{i}"), &lost).unwrap();
    }
    let full = call(&host, "event", "One more | in 2h", "h", NOW);
    assert!(full["error"].as_str().unwrap().contains("already planned"), "{full}");
    assert_eq!(call(&host, "tick", "", "bot", NOW + 3 * DAY)["output"], format!("Removed {} finished events.", store::MAX_EVENTS));
    assert!(host.kv.borrow().is_empty());
}

#[test]
fn an_event_that_no_longer_parses_is_deleted_when_pruned() {
    let host = FakeHost::default();
    host.kv.borrow_mut().insert("e:broken".into(), "{not json".into());
    assert_eq!(store::prune(&host, NOW).unwrap(), 1);
    assert!(host.kv.borrow().is_empty());
}
