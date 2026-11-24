//! The `remind` command, run on a timer: posts one reminder per settled night as its start nears.

use serde_json::json;

use crate::host::Host;
use crate::night::{Night, Store};
use crate::settings::Settings;

/// A run may post at most three times, so it stops there and the next run takes the rest.
const PER_RUN: i64 = 3;
const KEEP_MS: i64 = 30 * 86_400_000;
const PRUNE_PER_RUN: i64 = 10;

pub fn post<H: Host>(host: &H, content: &str) -> Result<(), String> {
    host.call(&json!({"capability": "message.post", "content": content})).map(|_| ())
}

/// A refused post is reported and the night stays claimed, not retried: a run that errors counts toward disabling the module.
/// A night is claimed before its post so two runs cannot both send it, and given back only when its tally could not be read.
pub fn run<H: Host>(host: &H, settings: &Settings, now_ms: i64) -> Result<String, String> {
    let store = Store::new(host);
    let mut sent = 0;
    let mut problem = None;
    let mut unreadable = None;
    let mut skip: Vec<String> = Vec::new();
    for _ in 0..PER_RUN {
        let limit = PER_RUN + skip.len() as i64;
        let Some(night) = store.due(now_ms, limit)?.into_iter().find(|n| !skip.contains(&n.id)) else { break };
        skip.push(night.id.clone());
        if !store.claim_reminder(&night.id)? {
            continue;
        }
        let Some(slot) = usize::try_from(night.chosen).ok().and_then(|i| night.slots.get(i)) else { continue };
        if slot.start_ms <= now_ms {
            continue;
        }
        let text = match reminder(&night, &store, slot.start_ms - now_ms, settings) {
            Ok(text) => text,
            Err(why) => {
                store.release_reminder(&night.id)?;
                unreadable = Some(why);
                continue;
            }
        };
        match post(host, &text) {
            Ok(()) => sent += 1,
            Err(why) => problem = Some(why),
        }
    }
    store.sweep_over(&[], now_ms)?;
    store.prune(now_ms - KEEP_MS, PRUNE_PER_RUN)?;
    if let Some(why) = unreadable {
        return Err(why);
    }
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

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use serde_json::{json, Value};

    use super::*;

    const NOW: i64 = 1_700_000_000_000;

    /// One chosen night, starting 30 minutes after `NOW`, with the claim flag the real table keeps.
    #[derive(Default)]
    struct FakeHost {
        reminded: Cell<bool>,
        refuse_post: RefCell<Option<String>>,
        votes_unreadable: Cell<bool>,
        posted: RefCell<Vec<String>>,
    }

    impl FakeHost {
        fn night_row() -> Value {
            let slots = json!([{"l": "fri 8pm", "t": NOW + 30 * 60_000}]).to_string();
            json!({"night_id": "n1", "game": "Valorant", "role": "", "host": "a1", "slots": slots, "chosen": 0, "remind_at": NOW, "created": NOW})
        }

        fn update(&self, request: &Value) -> Value {
            let expects = request["where"][1]["val"].as_bool() == Some(self.reminded.get());
            if expects {
                self.reminded.set(request["values"]["reminded"].as_bool().unwrap_or(false));
            }
            json!({"ok": true, "changed": i64::from(expects)})
        }
    }

    impl Host for FakeHost {
        fn call(&self, request: &Value) -> Result<Value, String> {
            if request["capability"] == "message.post" {
                if let Some(why) = self.refuse_post.borrow().clone() {
                    return Err(why);
                }
                self.posted.borrow_mut().push(request["content"].as_str().unwrap_or_default().to_string());
                return Ok(json!({"ok": true}));
            }
            match request["op"].as_str() {
                Some("update") => Ok(self.update(request)),
                Some("count") if self.votes_unreadable.get() => Err("data.store is unavailable".to_string()),
                Some("count") => Ok(json!({"ok": true, "count": 2})),
                Some("query") if request["table"] == "nights" && request["where"][1]["col"] == "reminded" && !self.reminded.get() => {
                    Ok(json!({"ok": true, "rows": [Self::night_row()]}))
                }
                _ => Ok(json!({"ok": true, "rows": []})),
            }
        }
    }

    fn tick(host: &FakeHost) -> Result<String, String> {
        run(host, &Settings::from_value(&Value::Null), NOW)
    }

    #[test]
    fn a_refused_post_is_reported_and_not_retried() {
        let host = FakeHost::default();
        *host.refuse_post.borrow_mut() = Some("message.post rate limit reached".to_string());
        let first = tick(&host).unwrap();
        assert!(first.starts_with("Reminded 0; a post was refused"), "{first}");
        assert!(host.reminded.get(), "a refused night is handled, so it cannot starve the ones behind it");

        *host.refuse_post.borrow_mut() = None;
        assert_eq!(tick(&host).unwrap(), "Reminded 0.");
        assert!(host.posted.borrow().is_empty());
    }

    /// Several chosen nights in due order; a night whose game is "BAD" has its post refused.
    struct ManyHost {
        reminded: RefCell<Vec<bool>>,
        games: Vec<&'static str>,
        posted: RefCell<Vec<String>>,
    }

    impl Host for ManyHost {
        fn call(&self, request: &Value) -> Result<Value, String> {
            if request["capability"] == "message.post" {
                let content = request["content"].as_str().unwrap_or_default().to_string();
                if content.contains("BAD") {
                    return Err("content contains hidden characters".to_string());
                }
                self.posted.borrow_mut().push(content);
                return Ok(json!({"ok": true}));
            }
            let slots = json!([{"l": "fri 8pm", "t": NOW + 30 * 60_000}]).to_string();
            match request["op"].as_str() {
                Some("query") if request["table"] == "nights" && request["where"][1]["col"] == "reminded" => {
                    let limit = request["limit"].as_i64().unwrap_or(0) as usize;
                    let rows: Vec<Value> = (0..self.games.len()).filter(|i| !self.reminded.borrow()[*i]).take(limit).map(|i| {
                        json!({"night_id": format!("n{i}"), "game": self.games[i], "role": "", "host": "a1", "slots": slots, "chosen": 0, "remind_at": NOW, "created": NOW})
                    }).collect();
                    Ok(json!({"ok": true, "rows": rows}))
                }
                Some("update") => {
                    let id = request["where"][0]["val"].as_str().unwrap_or_default();
                    let index: usize = id.trim_start_matches('n').parse().unwrap();
                    let want = request["where"][1]["val"].as_bool().unwrap();
                    let changed = self.reminded.borrow()[index] == want;
                    if changed {
                        self.reminded.borrow_mut()[index] = request["values"]["reminded"].as_bool().unwrap();
                    }
                    Ok(json!({"ok": true, "changed": i64::from(changed)}))
                }
                Some("count") => Ok(json!({"ok": true, "count": 0})),
                _ => Ok(json!({"ok": true, "rows": []})),
            }
        }
    }

    #[test]
    fn refused_nights_do_not_block_the_ones_behind_them() {
        let host = ManyHost { reminded: RefCell::new(vec![false; 5]), games: vec!["BAD1", "BAD2", "BAD3", "Chess", "Go"], posted: RefCell::new(vec![]) };
        let first = run(&host, &Settings::from_value(&Value::Null), NOW).unwrap();
        assert!(first.starts_with("Reminded 0; a post was refused"), "{first}");
        assert_eq!(run(&host, &Settings::from_value(&Value::Null), NOW).unwrap(), "Reminded 2.");
        assert_eq!(host.posted.borrow().len(), 2);
    }

    #[test]
    fn a_reminder_that_went_out_is_claimed_for_good() {
        let host = FakeHost::default();
        assert_eq!(tick(&host).unwrap(), "Reminded 1.");
        assert!(host.reminded.get());
        assert_eq!(tick(&host).unwrap(), "Reminded 0.");
        assert_eq!(host.posted.borrow().len(), 1);
    }

    #[test]
    fn a_reminder_whose_tally_could_not_be_read_is_given_back_and_the_run_fails() {
        let host = FakeHost::default();
        host.votes_unreadable.set(true);
        assert!(tick(&host).unwrap_err().contains("unavailable"));
        assert!(!host.reminded.get());

        host.votes_unreadable.set(false);
        assert_eq!(tick(&host).unwrap(), "Reminded 1.");
    }
}
