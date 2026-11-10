//! The four call-audio events: they open a call, change who is in the agreed set, bring the speech, and close the call.
//!
//! Nothing is kept for a session the module was never told had started, or for a person who is not in the agreed set.

use serde_json::Value;

use crate::host::Host;
use crate::post::{self, MAX_LINES};
use crate::settings::Settings;
use crate::store::{Line, Store};

/// Longest a stored utterance may be; the host cuts at 1,000 already, so this only guards a bad sender.
const MAX_LINE_CHARS: usize = 1_000;

pub fn on_event<H: Host>(store: &Store<H>, settings: &Settings, input: &str) -> Result<String, String> {
    let event: Value = serde_json::from_str(input).map_err(|_| "the event was not JSON".to_string())?;
    let data = &event["data"];
    match event["event"].as_str() {
        Some("call.audio.started") => started(store, data),
        Some("call.audio.participant") => participant(store, data),
        Some("call.audio.segment") => segment(store, data, event["id"].as_str().unwrap_or("")),
        Some("call.audio.stopped") => stopped(store, settings, data),
        _ => Ok("Ignored.".to_string()),
    }
}

fn started<H: Host>(store: &Store<H>, data: &Value) -> Result<String, String> {
    let (Some(session), Some(at)) = (data["session_id"].as_str(), data["started_at_ms"].as_i64()) else {
        return Err("the started event had no session or time".to_string());
    };
    if !store.insert_call(session, at)? {
        return Ok("Already started.".to_string());
    }
    let users: Vec<String> = data["included"].as_array().into_iter().flatten().filter_map(|u| u.as_str().map(str::to_string)).collect();
    store.insert_people(session, &users)?;
    Ok(format!("Listening with {} people.", users.len()))
}

fn participant<H: Host>(store: &Store<H>, data: &Value) -> Result<String, String> {
    let (Some(session), Some(user), Some(included)) = (data["session_id"].as_str(), data["user_id"].as_str(), data["included"].as_bool()) else {
        return Err("the participant event had no session, person or answer".to_string());
    };
    match store.call(session)? {
        Some(call) if call.state == "on" => store.set_included(session, user, included).map(|()| "Noted.".to_string()),
        _ => Ok("Ignored: no call here.".to_string()),
    }
}

fn segment<H: Host>(store: &Store<H>, data: &Value, event_id: &str) -> Result<String, String> {
    let (Some(session), Some(user)) = (data["session_id"].as_str(), data["user_id"].as_str()) else {
        return Err("the segment had no session or speaker".to_string());
    };
    let Some(call) = store.call(session)?.filter(|c| c.state == "on") else {
        return Ok("Ignored: no call here.".to_string());
    };
    let Some((true, ord)) = store.speaker(session, user)? else {
        return Ok("Ignored: not in the agreed set.".to_string());
    };
    let body: String = data["text"].as_str().unwrap_or("").trim().chars().take(MAX_LINE_CHARS).collect();
    if body.is_empty() {
        return Ok("Ignored: nothing said.".to_string());
    }
    if call.seen >= MAX_LINES {
        store.bump_call(session, serde_json::json!({}), serde_json::json!({"dropped": 1}))?;
        return Ok("Dropped: the call is at its line limit.".to_string());
    }
    let (start_ms, end_ms) = (data["start_ms"].as_i64().unwrap_or(0), data["end_ms"].as_i64().unwrap_or(0));
    let line = Line { user_id: user.to_string(), start_ms, end_ms: end_ms.max(start_ms), body };
    let id = data["segment_id"].as_str().unwrap_or(event_id);
    if !store.insert_line((session, id), &line)? {
        return Ok("Duplicate.".to_string());
    }
    let first_words = ord == 0;
    if first_words {
        store.set_speaker(session, user, call.speakers + 1)?;
    }
    let add = serde_json::json!({"seen": 1, "speakers": i64::from(first_words)});
    store.bump_call(session, serde_json::json!({"last_ms": end_ms.max(call.last_ms)}), add)?;
    Ok("Kept.".to_string())
}

fn stopped<H: Host>(store: &Store<H>, settings: &Settings, data: &Value) -> Result<String, String> {
    let (Some(session), Some(at)) = (data["session_id"].as_str(), data["stopped_at_ms"].as_i64()) else {
        return Err("the stopped event had no session or time".to_string());
    };
    let reason = data["reason"].as_str().unwrap_or("stopped");
    let sent = data["segments_sent"].as_i64().unwrap_or(-1);
    let Some(call) = store.call(session)? else {
        return Ok("Ignored: no call here.".to_string());
    };
    let close = serde_json::json!({"stopped_ms": at, "last_ms": at.max(call.last_ms), "reason": reason, "sent": sent});
    if !store.move_call(session, ("on", "done"), close)? {
        return Ok("Already ended.".to_string());
    }
    if reason == "declined" {
        store.delete_call(session)?;
        return Ok("Declined, so nothing is kept.".to_string());
    }
    let call = store.call(session)?.unwrap_or(call);
    end_call(store, settings, &call)
}

/// Posts a call that just ended, or drops it quietly when nobody was heard.
pub fn end_call<H: Host>(store: &Store<H>, settings: &Settings, call: &crate::store::Call) -> Result<String, String> {
    if call.seen == 0 {
        store.move_call(&call.session_id, ("done", "posted"), serde_json::json!({}))?;
        return Ok("Nobody was heard, so nothing is posted.".to_string());
    }
    let outcome = post::post_pending(store, settings, call)?;
    Ok(match outcome.waiting {
        Some(why) => format!("Posted {}; waiting: {why}.", outcome.messages),
        None => format!("Posted {}.", outcome.messages),
    })
}
