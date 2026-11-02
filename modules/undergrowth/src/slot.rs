//! What the undergrowth shows on profiles: how much a member's messages and calls have fed it,
//! and, for members who linked their profile, their record and the champion badge.

use serde_json::{json, Map, Value};

use crate::host::Host;
use crate::store::Store;
use crate::words::count;

/// Leaves it takes to reach each rank.
const RANKS: [(i64, &str); 5] = [(0, "Sprout"), (25, "Rootlet"), (100, "Mycelium"), (400, "Elder"), (1500, "Ancient")];

pub fn answer<H: Host>(store: &Store<H>, input: &str) -> Result<String, String> {
    let ask: Value = serde_json::from_str(input).map_err(|_| "the slot request was not JSON".to_string())?;
    let users: Vec<String> = ask["user_ids"].as_array().into_iter().flatten().filter_map(|u| u.as_str().map(str::to_owned)).take(100).collect();
    let champion = store.fact("champion")?.and_then(|r| r["val"].as_str().map(str::to_owned));
    let players = if users.is_empty() { Vec::new() } else { store.players_of_users(&users)? };
    let crowned = |user: &str| {
        champion.as_deref().is_some_and(|c| players.iter().any(|p| p["user_id"] == user && p["pid"] == c))
    };
    let mut items = Map::new();
    match ask["name"].as_str() {
        Some("grove") => {
            let fed = if users.is_empty() { Vec::new() } else { store.feeders(&users)? };
            for user in &users {
                let feeder = fed.iter().find(|f| f["user_id"] == user.as_str());
                let record = players.iter().find(|p| p["user_id"] == user.as_str());
                if let Some(item) = section(feeder, record, crowned(user)) {
                    items.insert(user.clone(), item);
                }
            }
        }
        Some("champion") => {
            for user in users.iter().filter(|u| crowned(u)) {
                let badge = json!({"type": "badge", "text": "Spore champion", "tone": "accent"});
                items.insert(user.clone(), json!({"nodes": [badge], "fallback": "Spore champion"}));
            }
        }
        _ => {}
    }
    Ok(json!({"$slim": "slot/1", "items": items}).to_string())
}

pub fn rank(leaves: i64) -> (&'static str, Option<i64>) {
    let at = RANKS.iter().rposition(|(min, _)| leaves >= *min).unwrap_or(0);
    (RANKS[at].1, RANKS.get(at + 1).map(|r| r.0))
}

fn section(feeder: Option<&Value>, record: Option<&Value>, crowned: bool) -> Option<Value> {
    let n = |v: Option<&Value>, k: &str| v.and_then(|v| v[k].as_i64()).unwrap_or(0);
    let leaves = n(feeder, "leaves");
    let warm = n(feeder, "warm_min");
    if leaves == 0 && warm == 0 && n(record, "seasons") == 0 && !crowned {
        return None;
    }
    let (title, next) = rank(leaves);
    let fallback = format!("{title}: fed the undergrowth {}", count(leaves, "leaf", "leaves"));
    let mut nodes = vec![json!({"type": "text", "text": fallback, "tone": "accent"})];
    if let Some(next) = next {
        let (upcoming, _) = rank(next);
        nodes.push(json!({"type": "progress", "value": leaves, "max": next, "label": format!("{upcoming} at {next} leaves")}));
    }
    if warm > 0 {
        nodes.push(json!({"type": "text", "text": format!("Kept it warm for {} h {} min in calls", warm / 60, warm % 60), "tone": "muted"}));
    }
    if n(record, "seasons") > 0 {
        let line = format!(
            "{} played, {} won, best colony {}",
            count(n(record, "seasons"), "season", "seasons"),
            n(record, "wins"),
            count(n(record, "best_cells"), "cell", "cells")
        );
        nodes.push(json!({"type": "text", "text": line, "tone": "normal"}));
    }
    if crowned {
        nodes.push(json!({"type": "badge", "text": "Spore champion", "tone": "accent"}));
    }
    Some(json!({"nodes": [{"type": "stack", "children": nodes}], "fallback": fallback}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_climb_with_leaves() {
        assert_eq!(rank(0), ("Sprout", Some(25)));
        assert_eq!(rank(99), ("Rootlet", Some(100)));
        assert_eq!(rank(5000), ("Ancient", None));
    }

    #[test]
    fn a_member_who_fed_nothing_and_never_linked_shows_nothing() {
        assert!(section(None, None, false).is_none());
        let fed = json!({"leaves": 30, "warm_min": 75});
        let item = section(Some(&fed), None, false).unwrap();
        let texts = item.to_string();
        assert!(texts.contains("Rootlet") && texts.contains("1 h 15 min"), "{texts}");
    }
}
