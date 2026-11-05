//! The four call-audio events, the `post` continuation, and the scheduled tidy.

use serde_json::{json, Value};

use crate::host::Host;
use crate::post;
use crate::settings::Settings;
use crate::store::{Session, Store};

/// A session with nothing heard for this long counts as ended; the host sends no `stopped` after a restart.
pub const SILENT_MS: i64 = 10 * 60 * 1000;
/// The prune runs at most this often.
const PRUNE_EVERY_MS: i64 = 24 * 60 * 60 * 1000;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// Segments read for one run's worth of messages.
const PAGE: i64 = 60;

/// What a run answers: text, and optionally a follow-up run.
pub struct Reply {
    pub output: String,
    pub run_after: Option<Value>,
}

impl Reply {
    fn text(output: impl Into<String>) -> Reply {
        Reply { output: output.into(), run_after: None }
    }
}

pub fn on_event<H: Host>(store: &Store<H>, settings: &Settings, input: &str, now_ms: i64) -> Result<Reply, String> {
    let event: Value = serde_json::from_str(input).map_err(|_| "the event was not JSON".to_string())?;
    let data = &event["data"];
    let (Some(session), Some(channel)) = (data["session_id"].as_str(), data["channel_id"].as_str()) else {
        return Ok(Reply::text("Ignored."));
    };
    let at = event["at_ms"].as_i64().unwrap_or(now_ms);
    match event["event"].as_str() {
        Some("call.audio.started") => {
            store.touch_session(session, channel, at)?;
            store.set_started(session, data["started_at_ms"].as_i64().unwrap_or(at))?;
            Ok(Reply::text("Started."))
        }
        Some("call.audio.participant") => {
            store.touch_session(session, channel, at)?;
            Ok(Reply::text("Noted."))
        }
        Some("call.audio.segment") => {
            let (Some(id), Some(user), Some(text)) = (data["segment_id"].as_str(), data["user_id"].as_str(), data["text"].as_str()) else {
                return Err("the segment event was missing its speaker or text".to_string());
            };
            store.touch_session(session, channel, at)?;
            let (start, end) = (data["start_ms"].as_i64().unwrap_or(0), data["end_ms"].as_i64().unwrap_or(0));
            store.add_segment(session, id, user, start, end, text)?;
            Ok(Reply::text("Kept."))
        }
        Some("call.audio.stopped") => {
            store.touch_session(session, channel, at)?;
            let stopped = data["stopped_at_ms"].as_i64().unwrap_or(at);
            let reason = data["reason"].as_str().unwrap_or("stopped");
            store.end_session(session, stopped, reason, data["segments_sent"].as_i64().unwrap_or(0))?;
            finish(store, settings, session)
        }
        _ => Ok(Reply::text("Ignored.")),
    }
}

/// Posts the next messages of a finished session's transcript, and asks to be run again for the rest.
pub fn finish<H: Host>(store: &Store<H>, settings: &Settings, session: &str) -> Result<Reply, String> {
    if !settings.post_when_done {
        return Ok(Reply::text("Kept, not posted."));
    }
    let Some(row) = store.session(session)? else {
        return Ok(Reply::text("Unknown session."));
    };
    let segments = store.segments_after(session, row.cursor, PAGE)?;
    if segments.is_empty() && row.posted > 0 {
        return Ok(Reply::text("Transcript already posted."));
    }
    let received = store.count_segments(session)?;
    let rendered = post::render(&row, &segments, received);
    let batch = post::this_run(&rendered.chunks);
    for chunk in batch {
        store.post(&chunk.text)?;
    }
    let cursor = batch.last().map_or(row.cursor, |c| c.through_id);
    let posted = row.posted + batch.len() as i64;
    store.set_progress(session, posted, cursor, &rendered.speakers.join(","))?;
    let more = rendered.chunks.len() > batch.len() || segments.len() as i64 >= PAGE;
    Ok(Reply {
        output: format!("Posted {} messages.", batch.len()),
        run_after: more.then(|| json!({"command": "post", "after_ms": 10_000, "key": session, "input": session})),
    })
}

/// The `post` command, run by the host a little after a transcript that needed more than one run.
pub fn continue_post<H: Host>(store: &Store<H>, settings: &Settings, session: &str) -> Result<Reply, String> {
    finish(store, settings, session)
}

/// The scheduled command: closes calls that went silent without a `stopped`, and prunes old transcripts daily.
pub fn tidy<H: Host>(store: &Store<H>, settings: &Settings, now_ms: i64) -> Result<Reply, String> {
    let mut lines = Vec::new();
    let mut run_after = None;
    // One at a time: a long transcript asks to be continued, and a run can ask for only one follow-up.
    if let Some(stale) = store.stale_sessions(now_ms - SILENT_MS)?.first() {
        let received = store.count_segments(&stale.session_id)?;
        store.end_session(&stale.session_id, stale.last_event_ms, "lost", received)?;
        lines.push(close_note(stale));
        run_after = finish(store, settings, &stale.session_id)?.run_after;
    }
    if now_ms - store.last_prune()? >= PRUNE_EVERY_MS {
        let removed = store.prune(now_ms - settings.keep_days * DAY_MS)?;
        store.set_last_prune(now_ms)?;
        lines.push(format!("Pruned {removed} old transcripts."));
    }
    let output = if lines.is_empty() { "Nothing to do.".to_string() } else { lines.join(" ") };
    Ok(Reply { output, run_after })
}

fn close_note(session: &Session) -> String {
    format!("Closed the silent call {}.", session.session_id)
}
