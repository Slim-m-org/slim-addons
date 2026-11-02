//! The `remind` command, run on a timer: posts one reminder per settled night as its start nears.

use serde_json::json;

use crate::host::Host;
use crate::night::{Night, Store};
use crate::settings::Settings;

/// A run may post at most three times, so it stops there and the next run takes the rest.
const PER_RUN: i64 = 3;
const KEEP_MS: i64 = 30 * 86_400_000;
const PRUNE_PER_RUN: i64 = 2;

pub fn post<H: Host>(host: &H, content: &str) -> Result<(), String> {
    host.call(&json!({"capability": "message.post", "content": content})).map(|_| ())
}

/// A refused post is reported, not failed: a run that errors counts toward disabling the module.
pub fn run<H: Host>(host: &H, settings: &Settings, now_ms: i64) -> Result<String, String> {
    let store = Store::new(host);
    let mut sent = 0;
    let mut problem = None;
    for night in store.due(now_ms, PER_RUN)? {
        if !store.claim_reminder(&night.id)? {
            continue;
        }
        let Some(slot) = usize::try_from(night.chosen).ok().and_then(|i| night.slots.get(i)) else { continue };
        if slot.start_ms <= now_ms {
            continue;
        }
        let text = reminder(&night, &store, slot.start_ms - now_ms, settings)?;
        match post(host, &text) {
            Ok(()) => sent += 1,
            Err(why) => problem = Some(why),
        }
    }
    store.prune(now_ms - KEEP_MS, PRUNE_PER_RUN)?;
    Ok(match problem {
        Some(why) => format!("Reminded {sent}; a post was refused ({why})."),
        None => format!("Reminded {sent}."),
    })
}

fn reminder<H: Host>(night: &Night, store: &Store<H>, away_ms: i64, _settings: &Settings) -> Result<String, String> {
    let tally = store.tally(night, "")?;
    let mention = if night.role.is_empty() { String::new() } else { format!("@[{}] ", night.role) };
    let minutes = (away_ms / 60_000).max(1);
    Ok(format!("{mention}{} starts in {minutes} min - {} in, {} maybe.", night.game, tally.answers[0], tally.answers[1]))
}
