//! The whole module against an in-memory slim: consent, ordering, merging, posting once, and the limits.

use serde_json::{json, Value};

use crate::fake::FakeHost;
use crate::handle;
use crate::post::MAX_LINES;

const S: &str = "sess-1";
const T0: i64 = 1_700_000_000_000;
const SEC: i64 = 1_000;
const MIN: i64 = 60_000;

fn run(host: &FakeHost, command: &str, input: Value, settings: Value, now_ms: i64) -> Value {
    host.next_run();
    let request = json!({"command": command, "input": input.to_string(), "settings": settings, "now_ms": now_ms});
    serde_json::from_slice(&handle(host, request.to_string().as_bytes())).unwrap()
}

fn event(host: &FakeHost, name: &str, data: Value) -> Value {
    let input = json!({"$slim": "event/1", "event": name, "id": "call-audio:x:1", "at_ms": T0, "data": data});
    let reply = run(host, "@event", input, json!({}), T0);
    assert_eq!(reply["ok"], true, "{reply}");
    reply
}

fn start(host: &FakeHost, session: &str, included: &[&str]) {
    let data = json!({"session_id": session, "channel_id": "c1", "started_at_ms": T0, "included": included});
    event(host, "call.audio.started", data);
}

fn say(host: &FakeHost, session: &str, user: &str, (from_s, to_s): (i64, i64), text: &str) -> Value {
    let data = json!({
        "session_id": session, "channel_id": "c1", "user_id": user, "text": text, "language": "en",
        "segment_id": format!("{session}:{user}:{from_s}:{text}"), "start_ms": T0 + from_s * SEC, "end_ms": T0 + to_s * SEC
    });
    event(host, "call.audio.segment", data)
}

fn stop(host: &FakeHost, session: &str, reason: &str, sent: i64) -> Value {
    let data = json!({"session_id": session, "channel_id": "c1", "stopped_at_ms": T0 + 20 * MIN, "reason": reason, "segments_sent": sent});
    event(host, "call.audio.stopped", data)
}

fn flush(host: &FakeHost, now_ms: i64) -> Value {
    run(host, "flush", json!(""), json!({}), now_ms)
}

fn posts(host: &FakeHost) -> Vec<String> {
    host.posts.borrow().clone()
}

#[test]
fn a_call_is_posted_once_when_it_ends_with_numbered_speakers_and_clock_offsets() {
    let host = FakeHost::default();
    start(&host, S, &["ada", "bob"]);
    say(&host, S, "ada", (5, 8), "hello everyone");
    say(&host, S, "bob", (10, 12), "hi ada");
    stop(&host, S, "stopped", 2);
    let posted = posts(&host);
    assert_eq!(posted.len(), 1);
    assert_eq!(
        posted[0],
        "**Call transcript** - 2023-11-14 22:13 UTC, 20 min\n\n[0:05] Speaker 1: hello everyone\n[0:10] Speaker 2: hi ada\n\n_End of transcript._"
    );
}

#[test]
fn lines_are_ordered_by_when_they_were_said_not_when_they_arrived() {
    let host = FakeHost::default();
    start(&host, S, &["ada", "bob"]);
    say(&host, S, "bob", (30, 31), "third");
    say(&host, S, "ada", (10, 11), "first");
    say(&host, S, "bob", (20, 21), "second");
    stop(&host, S, "stopped", 3);
    let body = &posts(&host)[0];
    let (first, second, third) = (body.find("first").unwrap(), body.find("second").unwrap(), body.find("third").unwrap());
    assert!(first < second && second < third, "{body}");
    assert!(body.contains("[0:10] Speaker 2: first") && body.contains("Speaker 1: second third"), "speakers number in the order the module first heard them: {body}");
}

#[test]
fn one_speakers_consecutive_lines_merge_until_a_pause_or_another_voice() {
    let host = FakeHost::default();
    start(&host, S, &["ada", "bob"]);
    say(&host, S, "ada", (0, 2), "one");
    say(&host, S, "ada", (3, 4), "two");
    say(&host, S, "bob", (5, 6), "three");
    say(&host, S, "ada", (7, 8), "four");
    say(&host, S, "ada", (50, 52), "five");
    stop(&host, S, "stopped", 5);
    let body = &posts(&host)[0];
    assert!(body.contains("[0:00] Speaker 1: one two\n[0:05] Speaker 2: three\n[0:07] Speaker 1: four\n[0:50] Speaker 1: five"), "{body}");
}

#[test]
fn the_merge_gap_and_the_timestamps_follow_the_settings() {
    let host = FakeHost::default();
    start(&host, S, &["ada"]);
    say(&host, S, "ada", (0, 2), "one");
    say(&host, S, "ada", (50, 52), "two");
    host.next_run();
    let settings = json!({"merge_gap_s": 60, "timestamps": false});
    let input = json!({"event": "call.audio.stopped", "data": {"session_id": S, "stopped_at_ms": T0 + MIN, "reason": "stopped", "segments_sent": 2}});
    let request = json!({"command": "@event", "input": input.to_string(), "settings": settings, "now_ms": T0});
    handle(&host, request.to_string().as_bytes());
    assert!(posts(&host)[0].contains("\n\nSpeaker 1: one two\n\n"), "{}", posts(&host)[0]);
}

#[test]
fn a_second_stop_event_never_posts_again() {
    let host = FakeHost::default();
    start(&host, S, &["ada"]);
    say(&host, S, "ada", (1, 2), "once");
    stop(&host, S, "stopped", 1);
    let again = stop(&host, S, "stopped", 1);
    assert_eq!(again["output"], "Already ended.");
    assert_eq!(flush(&host, T0 + 25 * MIN)["output"], "Posted 0.");
    assert_eq!(posts(&host).len(), 1);
}

#[test]
fn the_end_of_the_call_and_the_timer_running_over_one_call_post_it_once() {
    let host = FakeHost::default();
    start(&host, S, &["ada"]);
    say(&host, S, "ada", (1, 2), "once");
    host.refuse_posts.replace(Some("message.post rate limit reached".to_string()));
    stop(&host, S, "stopped", 1);
    assert!(posts(&host).is_empty());
    host.refuse_posts.replace(None);
    flush(&host, T0 + 25 * MIN);
    flush(&host, T0 + 30 * MIN);
    assert_eq!(posts(&host).len(), 1, "a refused post is retried by the timer, and only once");
}

#[test]
fn nothing_is_kept_or_posted_for_a_call_that_was_never_started() {
    let host = FakeHost::default();
    say(&host, "ghost", "ada", (1, 2), "who is there");
    stop(&host, "ghost", "stopped", 1);
    assert!(host.rows("calls").is_empty() && host.rows("lines").is_empty());
    assert!(posts(&host).is_empty());
}

#[test]
fn a_call_that_was_declined_leaves_nothing_and_posts_nothing() {
    let host = FakeHost::default();
    start(&host, S, &["ada"]);
    say(&host, S, "ada", (1, 2), "a word before the no");
    stop(&host, S, "declined", 1);
    assert!(posts(&host).is_empty());
    assert!(!host.has_call(S) && host.rows("lines").is_empty() && host.rows("people").is_empty());
}

#[test]
fn a_person_outside_the_agreed_set_is_never_written_down() {
    let host = FakeHost::default();
    start(&host, S, &["ada"]);
    assert_eq!(say(&host, S, "cleo", (1, 2), "I never agreed")["output"], "Ignored: not in the agreed set.");
    event(&host, "call.audio.participant", json!({"session_id": S, "user_id": "bob", "included": true}));
    say(&host, S, "bob", (3, 4), "I agreed late");
    event(&host, "call.audio.participant", json!({"session_id": S, "user_id": "bob", "included": false}));
    say(&host, S, "bob", (5, 6), "I took it back");
    say(&host, S, "ada", (7, 8), "still here");
    stop(&host, S, "stopped", 2);
    let body = &posts(&host)[0];
    assert!(body.contains("I agreed late") && body.contains("still here"), "{body}");
    assert!(!body.contains("never agreed") && !body.contains("took it back"), "{body}");
}

#[test]
fn a_call_nobody_spoke_in_posts_nothing() {
    let host = FakeHost::default();
    start(&host, S, &["ada", "bob"]);
    let ended = stop(&host, S, "stopped", 0);
    assert_eq!(ended["output"], "Nobody was heard, so nothing is posted.");
    assert!(posts(&host).is_empty());
    assert_eq!(host.call(S)["state"], "posted");
}

#[test]
fn a_late_segment_after_the_call_ended_is_ignored() {
    let host = FakeHost::default();
    start(&host, S, &["ada"]);
    say(&host, S, "ada", (1, 2), "before");
    stop(&host, S, "stopped", 1);
    assert_eq!(say(&host, S, "ada", (3, 4), "after")["output"], "Ignored: no call here.");
    assert_eq!(host.rows("lines").len(), 1);
}

#[test]
fn a_repeated_segment_or_start_is_kept_once() {
    let host = FakeHost::default();
    start(&host, S, &["ada"]);
    start(&host, S, &["ada", "bob"]);
    assert_eq!(host.rows("people").len(), 1, "a repeated start does not widen the agreed set");
    say(&host, S, "ada", (1, 2), "same");
    assert_eq!(say(&host, S, "ada", (1, 2), "same")["output"], "Duplicate.");
    assert_eq!(host.call(S)["seen"], 1);
}

#[test]
fn speech_can_never_mention_anyone() {
    let host = FakeHost::default();
    start(&host, S, &["ada"]);
    say(&host, S, "ada", (1, 2), "ping @everyone and @[Mods]\nplease");
    stop(&host, S, "stopped", 1);
    let body = &posts(&host)[0];
    assert!(!body.contains('@') && body.contains("ping \u{FF20}everyone and \u{FF20}[Mods] please"), "{body}");
}

#[test]
fn a_long_call_is_split_across_messages_within_the_host_limit_and_keeps_every_line_in_order() {
    let host = FakeHost::default();
    start(&host, S, &["ada", "bob"]);
    let sentence = "the quick brown fox jumps over the lazy dog and keeps on running";
    let total = 400;
    for n in 0..total {
        let user = if n % 2 == 0 { "ada" } else { "bob" };
        say(&host, S, user, (n * 10, n * 10 + 5), &format!("{n:04} {sentence}"));
    }
    stop(&host, S, "stopped", total);
    let mut at = T0 + 30 * MIN;
    while host.call(S)["state"] != "posted" {
        flush(&host, at);
        at += 5 * MIN;
    }
    let posted = posts(&host);
    assert!(posted.len() > 3, "400 lines cannot fit one run's three posts");
    assert!(posted.iter().all(|p| p.chars().count() <= 4000));
    assert!(posted[0].starts_with("**Call transcript** - ") && posted[1].starts_with("**Call transcript** (part 2)"));
    let all = posted.join("\n");
    let numbers: Vec<usize> = all.match_indices(&sentence[..9]).map(|(i, _)| all[i - 5..i - 1].trim().parse().unwrap_or(usize::MAX)).collect();
    assert_eq!(numbers, (0..total as usize).collect::<Vec<_>>(), "every line once, in order");
    assert_eq!(all.matches("End of transcript.").count(), 1);
    assert!(posted.last().unwrap().ends_with("_End of transcript._"));
}

#[test]
fn a_run_posts_at_most_three_messages_and_the_timer_finishes_the_rest() {
    let host = FakeHost::default();
    start(&host, S, &["ada"]);
    for n in 0..200 {
        say(&host, S, "ada", (n * 100, n * 100 + 5), &format!("{n:04} {}", "word ".repeat(40)));
    }
    stop(&host, S, "stopped", 200);
    assert_eq!(posts(&host).len(), 3);
    assert_eq!(host.call(S)["state"], "done");
    flush(&host, T0 + 30 * MIN);
    assert!(posts(&host).len() > 3);
}

#[test]
fn with_no_posting_channel_the_transcript_waits_and_goes_out_when_one_is_picked() {
    let host = FakeHost::default();
    host.no_channel.set(true);
    start(&host, S, &["ada"]);
    say(&host, S, "ada", (1, 2), "keep me");
    let ended = stop(&host, S, "stopped", 1);
    assert!(ended["output"].as_str().unwrap().contains("no posting channel is set"), "{ended}");
    assert!(posts(&host).is_empty());
    assert_eq!(host.call(S)["posted"], 0, "the claim is given back");
    flush(&host, T0 + 25 * MIN);
    assert!(posts(&host).is_empty());
    host.no_channel.set(false);
    flush(&host, T0 + 30 * MIN);
    assert_eq!(posts(&host).len(), 1);
    assert!(posts(&host)[0].contains("keep me"));
}

#[test]
fn the_footer_says_when_segments_went_missing_and_why_a_call_stopped_early() {
    let host = FakeHost::default();
    start(&host, S, &["ada"]);
    say(&host, S, "ada", (1, 2), "partial");
    stop(&host, S, "time_limit", 4);
    let body = &posts(&host)[0];
    assert!(body.contains("time limit"), "{body}");
    assert!(body.contains("3 segments never reached this transcript."), "{body}");
}

#[test]
fn a_call_past_the_line_limit_stops_taking_lines_and_says_so() {
    let host = FakeHost::default();
    start(&host, S, &["ada"]);
    let mut limited = host.call(S);
    assert_eq!(limited["seen"], 0);
    for n in 0..MAX_LINES + 3 {
        say(&host, S, "ada", (n, n), &format!("w{n}"));
    }
    limited = host.call(S);
    assert_eq!((limited["seen"].as_i64(), limited["dropped"].as_i64()), (Some(MAX_LINES), Some(3)));
    stop(&host, S, "stopped", MAX_LINES + 3);
    let mut at = T0 + 30 * MIN;
    while host.call(S)["state"] != "posted" {
        flush(&host, at);
        at += 5 * MIN;
    }
    assert!(posts(&host).last().unwrap().contains("3 segments never reached this transcript."));
}

#[test]
fn a_call_that_went_quiet_is_closed_by_the_timer_and_posted() {
    let host = FakeHost::default();
    start(&host, S, &["ada"]);
    say(&host, S, "ada", (1, 2), "the server restarted after this");
    assert_eq!(flush(&host, T0 + 10 * MIN)["output"], "Posted 0.", "ten quiet minutes is not yet over");
    assert_eq!(host.call(S)["state"], "on");
    flush(&host, T0 + 40 * MIN);
    assert_eq!(posts(&host).len(), 1);
    assert!(posts(&host)[0].contains("went quiet"));
}

#[test]
fn old_calls_are_deleted_after_the_days_the_admin_set() {
    let host = FakeHost::default();
    start(&host, S, &["ada"]);
    say(&host, S, "ada", (1, 2), "short lived");
    stop(&host, S, "stopped", 1);
    let day = 86_400_000;
    run(&host, "flush", json!(""), json!({"keep_days": 7}), T0 + 6 * day);
    assert!(host.has_call(S));
    run(&host, "flush", json!(""), json!({"keep_days": 7}), T0 + 8 * day);
    assert!(!host.has_call(S) && host.rows("lines").is_empty() && host.rows("people").is_empty());
}

#[test]
fn the_timer_needs_a_clock() {
    let host = FakeHost::default();
    assert_eq!(flush(&host, 0)["ok"], false);
}

#[test]
fn the_command_posts_the_call_so_far_without_moving_the_final_transcript() {
    let host = FakeHost::default();
    start(&host, S, &["ada"]);
    assert_eq!(run(&host, "transcript", json!(""), json!({}), T0)["output"], "Nothing has been said yet.");
    say(&host, S, "ada", (4, 6), "halfway through");
    let reply = run(&host, "transcript", json!(""), json!({}), T0);
    assert_eq!(reply["output"], "");
    assert!(posts(&host)[0].starts_with("**Call transcript so far** - started 2023-11-14 22:13 UTC\n\n[0:04] Speaker 1: halfway through"));
    assert_eq!(host.call(S)["posted"], 0);
    stop(&host, S, "stopped", 1);
    assert_eq!(posts(&host).len(), 2, "the final transcript still posts");
    assert!(posts(&host)[1].starts_with("**Call transcript** - "));
}

#[test]
fn the_command_says_so_when_no_call_is_on_and_when_it_cannot_post() {
    let host = FakeHost::default();
    assert_eq!(run(&host, "transcript", json!(""), json!({}), T0)["output"], "No call is being transcribed right now.");
    start(&host, S, &["ada"]);
    say(&host, S, "ada", (1, 2), "hello");
    host.no_channel.set(true);
    let reply = run(&host, "transcript", json!(""), json!({}), T0);
    assert_eq!(reply["ok"], false);
    host.no_channel.set(false);
    host.refuse_posts.replace(Some("slow mode is active in that channel".to_string()));
    assert!(run(&host, "transcript", json!(""), json!({}), T0)["error"].as_str().unwrap().contains("slow mode"));
}

#[test]
fn the_command_notes_when_a_long_call_does_not_fit_what_it_may_post() {
    let host = FakeHost::default();
    start(&host, S, &["ada"]);
    for n in 0..120 {
        say(&host, S, "ada", (n * 100, n * 100 + 5), &format!("{n:04} {}", "word ".repeat(40)));
    }
    run(&host, "transcript", json!(""), json!({}), T0);
    let posted = posts(&host);
    assert_eq!(posted.len(), 3);
    assert!(posted[2].contains("There is more than fits here.") && !posted[1].contains("There is more than fits here."));
}

#[test]
fn bad_input_answers_an_error_instead_of_trapping() {
    let host = FakeHost::default();
    assert_eq!(serde_json::from_slice::<Value>(&handle(&host, b"nope")).unwrap()["ok"], false);
    assert_eq!(run(&host, "nonsense", json!(""), json!({}), T0)["ok"], false);
    let reply = run(&host, "@event", json!({"event": "call.audio.segment", "data": {}}), json!({}), T0);
    assert_eq!(reply["ok"], false);
    let unknown = run(&host, "@event", json!({"event": "voice.session.closed", "data": {}}), json!({}), T0);
    assert_eq!(unknown["output"], "Ignored.");
}

#[test]
fn two_calls_at_once_stay_apart() {
    let host = FakeHost::default();
    start(&host, "a", &["ada"]);
    start(&host, "b", &["ada"]);
    say(&host, "a", "ada", (1, 2), "in call a");
    say(&host, "b", "ada", (1, 2), "in call b");
    stop(&host, "a", "stopped", 1);
    stop(&host, "b", "stopped", 1);
    let posted = posts(&host);
    assert_eq!(posted.len(), 2);
    assert!(posted[0].contains("in call a") && !posted[0].contains("in call b"));
    assert!(posted[1].contains("in call b") && !posted[1].contains("in call a"));
}

mod manifest {
    use serde_json::Value;

    fn manifest() -> Value {
        serde_json::from_str(include_str!("../manifest.json")).unwrap()
    }

    #[test]
    fn it_asks_for_only_what_it_uses_and_says_why_in_its_own_words() {
        let m = manifest();
        let caps: Vec<&str> = m["capabilities"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert_eq!(caps, ["call.audio", "command.register", "data.store", "message.post", "schedule"]);
        assert!(m["addon_api"].as_i64().unwrap() >= 4, "call.audio is add-on API 4");
        for field in ["purpose", "keeps"] {
            let words = m["call_audio"][field].as_str().unwrap().chars().count();
            assert!((20..=240).contains(&words), "{field} is {words} characters");
        }
        assert!(m["call_audio"]["keeps"].as_str().unwrap().contains("30 days"), "the promise matches the keep_days default");
    }

    #[test]
    fn its_timer_and_commands_are_declared() {
        let m = manifest();
        let commands: Vec<&str> = m["extension_points"].as_array().unwrap().iter().filter(|e| e["kind"] == "command").filter_map(|e| e["name"].as_str()).collect();
        assert_eq!(commands, ["transcript", "flush"]);
        assert_eq!(m["schedules"][0]["command"], "flush");
        let keys: Vec<&str> = m["permissions"].as_array().unwrap().iter().filter_map(|p| p["key"].as_str()).collect();
        for e in m["extension_points"].as_array().unwrap() {
            assert!(keys.contains(&e["permission"].as_str().unwrap()));
        }
    }

    #[test]
    fn the_member_columns_are_marked_so_an_account_deletion_purges_them() {
        let m = manifest();
        for table in m["data"]["tables"].as_array().unwrap() {
            for col in table["columns"].as_array().unwrap() {
                if col["name"] == "user_id" {
                    assert_eq!(col["member"], true, "{} holds a user id", table["name"]);
                }
            }
        }
    }

    #[test]
    fn the_settings_defaults_match_the_code() {
        let m = manifest();
        let defaults: Vec<(String, Value)> = m["settings"].as_array().unwrap().iter().map(|s| (s["key"].as_str().unwrap().to_string(), s["default"].clone())).collect();
        let parsed = crate::settings::Settings::from_value(&serde_json::json!({}));
        assert_eq!(parsed.keep_days, defaults[0].1.as_i64().unwrap());
        assert_eq!(parsed.merge_gap_s, defaults[1].1.as_i64().unwrap());
        assert_eq!(parsed.timestamps, defaults[2].1.as_bool().unwrap());
    }
}
