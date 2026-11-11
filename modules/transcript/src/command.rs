//! The commands a member or the timer runs: the transcript so far, and the timer's sweep.

use crate::host::Host;
use crate::post;
use crate::settings::Settings;
use crate::store::{PostError, Store};

const MINUTE_MS: i64 = 60_000;
const DAY_MS: i64 = 86_400_000;
/// A call that has heard nothing for this long is treated as over, as slim's own docs advise after a restart.
pub const LAPSE_MS: i64 = 30 * MINUTE_MS;
/// Calls looked at in one sweep, so a backlog drains over a few runs.
const SWEEP: i64 = 10;

/// Posts what has been said in the call that is on, as the member who asked, in the channel they asked from.
pub fn transcript<H: Host>(store: &Store<H>, settings: &Settings) -> Result<String, String> {
    let Some(call) = store.calls_in_state("on", "desc", 1)?.into_iter().next() else {
        return Ok("No call is being transcribed right now.".to_string());
    };
    match post::post_so_far(store, settings, &call)? {
        Ok(0) => Ok("Nothing has been said yet.".to_string()),
        Ok(_) => Ok(String::new()),
        Err(PostError::Unavailable) => Err("Transcript cannot post here yet. An admin needs to allow it to post.".to_string()),
        Err(PostError::Refused(why)) => Err(format!("The transcript was not posted: {why}")),
    }
}

/// The timer: closes calls that went quiet, posts what is waiting, and forgets old calls.
pub fn flush<H: Host>(store: &Store<H>, settings: &Settings, now_ms: i64) -> Result<String, String> {
    if now_ms <= 0 {
        return Err("slim gave no clock, so the timer cannot tell what is due".to_string());
    }
    let mut posted = 0;
    let mut waiting = None;
    for call in store.calls_idle_since("on", now_ms - LAPSE_MS, SWEEP)? {
        let close = serde_json::json!({"stopped_ms": call.last_ms, "reason": "lapsed"});
        if store.move_call(&call.session_id, ("on", "done"), close)? && call.seen == 0 {
            store.move_call(&call.session_id, ("done", "posted"), serde_json::json!({}))?;
        }
    }
    for call in store.calls_in_state("done", "asc", SWEEP)? {
        if store.posts_left() == 0 || store.data_left() < 12 {
            break;
        }
        let outcome = post::post_pending(store, settings, &call)?;
        posted += outcome.messages;
        waiting = waiting.or(outcome.waiting);
    }
    let cutoff = now_ms - settings.keep_days * DAY_MS;
    for call in store.finished_calls_idle_since(cutoff, SWEEP)? {
        if store.data_left() < 6 {
            break;
        }
        store.delete_call(&call.session_id)?;
    }
    Ok(match waiting {
        Some(why) => format!("Posted {posted}; waiting: {why}."),
        None => format!("Posted {posted}."),
    })
}
