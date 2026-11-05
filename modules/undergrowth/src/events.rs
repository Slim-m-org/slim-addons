//! The space feeding the soil. Each pushed event only drops a row in the inbox; the next frame
//! of every running game reads the rows it has not seen. Calls are read from the durable log,
//! so a session missed while the server was down is still counted exactly once.

use serde_json::Value;

use crate::host::Host;
use crate::moves::Weather;
use crate::rng::hash;
use crate::settings::Settings;
use crate::store::Store;

pub fn on_event<H: Host>(store: &Store<H>, settings: &Settings, input: &str, now_ms: i64) -> Result<String, String> {
    let event: Value = serde_json::from_str(input).map_err(|_| "the event was not JSON".to_string())?;
    let data = &event["data"];
    match event["event"].as_str() {
        Some("message.created") => message(store, data, now_ms),
        Some("reaction.changed") => {
            if data["count"].as_i64().unwrap_or(0) <= 0 {
                return Ok("a reaction went away; no rain".into());
            }
            let at = spot(data["message_id"].as_str().unwrap_or(""));
            store.add_weather(&[Weather { kind: "rain".into(), at, amount: 1 }], now_ms)?;
            Ok("rain".into())
        }
        Some("member.joined") => {
            if !settings.wild_rot {
                return Ok("the wild rot is switched off".into());
            }
            let at = spot(data["user_id"].as_str().unwrap_or(""));
            store.add_weather(&[Weather { kind: "wild".into(), at, amount: at }], now_ms)?;
            Ok("the wild rot stirs".into())
        }
        Some("voice.session.closed") => catch_up_calls(store, now_ms).map(|n| format!("{n} calls warmed the soil")),
        _ => Ok("ignored".into()),
    }
}

/// Where on any board something lands: a stable spread from an id, folded to the width later.
fn spot(id: &str) -> u32 {
    (hash(id.as_bytes()) % 4096) as u32
}

fn message<H: Host>(store: &Store<H>, data: &Value, now_ms: i64) -> Result<String, String> {
    if data["is_bot"].as_bool().unwrap_or(false) {
        return Ok("bots do not drop leaves".into());
    }
    let content = data["content"].as_str().unwrap_or("");
    let author = data["author_id"].as_str();
    let mut said = Vec::new();
    if let (Some(code), Some(user)) = (link_code(content), author) {
        if let Some(pid) = store.take_link(&code, now_ms)? {
            store.link_user(&pid, user)?;
            said.push("linked a profile");
        }
    }
    let amount = 1 + (content.chars().count() as u32 / 60).min(3);
    let at = spot(data["message_id"].as_str().unwrap_or(content));
    store.add_weather(&[Weather { kind: "leaf".into(), at, amount }], now_ms)?;
    if let Some(user) = author {
        store.feed(user, (1, 0))?;
    }
    said.push("a leaf fell");
    Ok(said.join(", "))
}

/// The first `ug-xxxxxx` in a message, lowercased.
pub fn link_code(content: &str) -> Option<String> {
    let lower = content.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find("ug-") {
        let start = from + at;
        let tail: String = lower[start + 3..].chars().take_while(char::is_ascii_alphanumeric).collect();
        if tail.len() == 6 {
            return Some(format!("ug-{tail}"));
        }
        from = start + 3;
    }
    None
}

/// Reads closed calls after the stored cursor, claims the batch by moving the cursor, then
/// warms the soil and credits each member. A crash after the claim loses that warmth rather
/// than counting it twice, which is the right side to fail on.
pub fn catch_up_calls<H: Host>(store: &Store<H>, now_ms: i64) -> Result<usize, String> {
    let cursor = store.fact("voice_cursor")?.and_then(|r| r["num"].as_i64()).unwrap_or(0);
    let (rows, next) = store.voice_sessions(cursor, 40)?;
    if rows.is_empty() || !store.advance_fact("voice_cursor", cursor, next)? {
        return Ok(0);
    }
    let mut warm = Vec::new();
    let mut minutes: Vec<(String, i64)> = Vec::new();
    for row in &rows {
        let n = |k: &str| row[k].as_i64().unwrap_or(0);
        let together = (n("left_at_ms") - n("joined_at_ms") - n("alone_ms")).max(0) / 60_000;
        if together == 0 {
            continue;
        }
        warm.push(Weather { kind: "warm".into(), at: 0, amount: together.min(600) as u32 });
        let user = row["user_id"].as_str().unwrap_or("").to_owned();
        match minutes.iter_mut().find(|(u, _)| *u == user) {
            Some(entry) => entry.1 += together,
            None => minutes.push((user, together)),
        }
    }
    store.add_weather(&warm, now_ms)?;
    for (user, m) in minutes.iter().filter(|(u, _)| !u.is_empty()) {
        store.feed(user, (0, *m))?;
    }
    Ok(warm.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_code_is_found_anywhere_in_a_message() {
        assert_eq!(link_code("here: UG-4KF9ZA thanks"), Some("ug-4kf9za".into()));
        assert_eq!(link_code("ug-12 then ug-abcdef"), Some("ug-abcdef".into()));
        assert_eq!(link_code("ug-abcdefg is too long"), None);
        assert_eq!(link_code("nothing here"), None);
    }

    #[test]
    fn spots_are_stable_and_bounded() {
        assert_eq!(spot("m1"), spot("m1"));
        assert!(spot("anything") < 4096);
    }
}
