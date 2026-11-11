//! Posting a call's transcript: cutting it into messages, and keeping count of how many lines already went out.
//!
//! The count lives on the call row and moves with one conditional update before each post,
//! so two runs that reach the same call (the end of the call and the timer) never post the same lines twice.

use crate::host::Host;
use crate::render::{self, Chunk, Layout};
use crate::settings::Settings;
use crate::store::{Call, PostError, Store, PAGE};

/// Past this many lines a call stops taking more; the host reads a query at most 10,000 rows in.
pub const MAX_LINES: i64 = 9_000;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub messages: u32,
    pub finished: bool,
    /// Set when the host would not take a post, so the rest waits for the next timer.
    pub waiting: Option<String>,
}

/// Which message of a call a chunk becomes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The whole call, after it ended.
    Final,
    /// What was said up to now in a call that is still going.
    SoFar,
}

struct Cut {
    text: String,
    lines: i64,
    /// The chunk reached the last line the call has.
    end: bool,
}

/// Reads lines from `offset` until a message is full or the call has no more.
fn cut<H: Host>(store: &Store<H>, call: &Call, offset: i64, layout: &Layout) -> Result<Cut, String> {
    let mut chunk = Chunk::new(layout);
    let mut at = offset;
    loop {
        let page = store.lines(&call.session_id, at)?;
        for line in &page {
            if !chunk.push(line) {
                return Ok(Cut { text: chunk.text(), lines: chunk.lines, end: false });
            }
        }
        at += page.len() as i64;
        if (page.len() as i64) < PAGE {
            return Ok(Cut { text: chunk.text(), lines: chunk.lines, end: true });
        }
    }
}

/// The message for one cut: a title, the speech, and a closing note on the last part of a transcript.
fn frame(call: &Call, kind: Kind, part: i64, cut: &Cut, truncated: bool) -> String {
    let title = match (kind, part) {
        (Kind::SoFar, 1) => format!("**Call transcript so far** - started {}", render::stamp(call.started_ms)),
        (Kind::SoFar, n) => format!("**Call transcript so far** (part {n})"),
        (Kind::Final, 1) => format!(
            "**Call transcript** - {}, {}",
            render::stamp(call.started_ms),
            render::duration(call.stopped_ms - call.started_ms)
        ),
        (Kind::Final, n) => format!("**Call transcript** (part {n})"),
    };
    let mut notes: Vec<String> = Vec::new();
    if cut.end && kind == Kind::Final {
        notes.extend(render::reason_note(&call.reason).map(str::to_string));
        notes.extend(render::missing_note(call));
        notes.push("End of transcript.".to_string());
    }
    if truncated {
        notes.push("There is more than fits here. The full transcript is posted when the call ends.".to_string());
    }
    let footer = if notes.is_empty() { String::new() } else { format!("\n\n_{}_", notes.join(" ")) };
    format!("{title}\n\n{}{footer}", cut.text)
}

/// Posts what is left of a call that ended, as far as this run's three posts go.
pub fn post_pending<H: Host>(store: &Store<H>, settings: &Settings, call: &Call) -> Result<Outcome, String> {
    let speakers = store.speakers(&call.session_id)?;
    let layout = Layout { started_ms: call.started_ms, merge_gap_ms: settings.merge_gap_s * 1000, timestamps: settings.timestamps, speakers: &speakers };
    let mut call = call.clone();
    let mut outcome = Outcome::default();
    while store.posts_left() > 0 && store.data_left() > 8 {
        let part = cut(store, &call, call.posted, &layout)?;
        if part.lines == 0 {
            outcome.finished = true;
            break;
        }
        if !store.claim_lines(&call.session_id, (call.posted, part.lines))? {
            break;
        }
        match store.post(&frame(&call, Kind::Final, call.parts + 1, &part, false)) {
            Ok(()) => {
                outcome.messages += 1;
                call.posted += part.lines;
                call.parts += 1;
            }
            Err(failure) => {
                store.release_lines(&call.session_id, (call.posted, part.lines))?;
                outcome.waiting = Some(match failure {
                    PostError::Unavailable => "no posting channel is set".to_string(),
                    PostError::Refused(why) => why,
                });
                break;
            }
        }
        if part.end {
            outcome.finished = true;
            break;
        }
    }
    if outcome.finished {
        store.move_call(&call.session_id, ("done", "posted"), serde_json::json!({}))?;
    }
    Ok(outcome)
}

/// Posts the start of a call that is still going, in the channel the command ran in; the call's own count does not move.
pub fn post_so_far<H: Host>(store: &Store<H>, settings: &Settings, call: &Call) -> Result<Result<u32, PostError>, String> {
    let speakers = store.speakers(&call.session_id)?;
    let layout = Layout { started_ms: call.started_ms, merge_gap_ms: settings.merge_gap_s * 1000, timestamps: settings.timestamps, speakers: &speakers };
    let (mut offset, mut messages) = (0, 0);
    while store.posts_left() > 0 && store.data_left() > 8 {
        let part = cut(store, call, offset, &layout)?;
        if part.lines == 0 {
            break;
        }
        let truncated = !part.end && store.posts_left() == 1;
        if let Err(failure) = store.post(&frame(call, Kind::SoFar, messages + 1, &part, truncated)) {
            return Ok(Err(failure));
        }
        messages += 1;
        offset += part.lines;
        if part.end {
            break;
        }
    }
    Ok(Ok(messages as u32))
}
