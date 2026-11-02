//! The `@slot` run and a check of its answer against the `slot/1` contract in
//! `docs/modules/reactive-modules.md`, which the client skips silently on a miss.

use serde_json::{json, Value};

const MAX_USERS: usize = 100;
const MAX_NODES: usize = 40;
const MAX_DEPTH: usize = 4;
const MAX_TEXT: usize = 120;
const MAX_BADGE: usize = 24;
const TONES: [&str; 3] = ["normal", "muted", "accent"];

/// The `input` of an `@slot` run, for a slot the manifest declares.
pub fn input(slot: &[String; 3], viewer: &str, manifest: Option<&Value>) -> Result<String, String> {
    let [kind, name, ids] = slot;
    if let Some(manifest) = manifest {
        let declared = manifest["slots"].as_array().is_some_and(|s| s.iter().any(|s| s["kind"] == kind.as_str() && s["name"] == name.as_str()));
        if !declared {
            return Err(format!("the manifest declares no {kind} slot named {name}"));
        }
    }
    let ids = user_ids(ids);
    if ids.is_empty() || ids.len() > MAX_USERS {
        return Err(format!("--slot needs 1 to {MAX_USERS} comma-separated user ids"));
    }
    Ok(json!({"slot": kind, "name": name, "user_ids": ids, "viewer_id": viewer}).to_string())
}

pub fn user_ids(list: &str) -> Vec<&str> {
    list.split(',').map(str::trim).filter(|id| !id.is_empty()).collect()
}

/// Every way the run's reply breaks the contract; empty when it is fine.
pub fn check(reply: &str, ids: &[&str]) -> Vec<String> {
    let Ok(reply) = serde_json::from_str::<Value>(reply) else { return vec!["the reply is not JSON".to_owned()] };
    if reply["ok"] != true {
        return vec![format!("the run failed: {}", reply["error"].as_str().unwrap_or("no reason given"))];
    }
    let Some(answer) = reply["output"].as_str().and_then(|o| serde_json::from_str::<Value>(o).ok()) else {
        return vec!["the output is not JSON, so slim shows nothing and counts a failed run".to_owned()];
    };
    if answer["$slim"] != "slot/1" {
        return vec!["the output is not a slot/1 answer ($slim must be \"slot/1\")".to_owned()];
    }
    let Some(items) = answer["items"].as_object() else { return vec!["items must be an object keyed by user id".to_owned()] };
    let mut problems = Vec::new();
    for (user, item) in items {
        if !ids.contains(&user.as_str()) {
            problems.push(format!("{user}: answered for a member that was not asked about"));
        }
        if item["fallback"].as_str().is_none_or(str::is_empty) {
            problems.push(format!("{user}: no fallback, so a client that skips every node shows nothing"));
        }
        let nodes = item["nodes"].as_array().cloned().unwrap_or_default();
        let mut count = 0;
        nodes.iter().for_each(|n| node(n, 1, &mut count, &mut problems, user));
        if count > MAX_NODES {
            problems.push(format!("{user}: {count} nodes is over {MAX_NODES}"));
        }
    }
    problems
}

fn node(n: &Value, depth: usize, count: &mut usize, problems: &mut Vec<String>, user: &str) {
    *count += 1;
    if depth > MAX_DEPTH {
        problems.push(format!("{user}: nested deeper than {MAX_DEPTH}"));
        return;
    }
    let kind = n["type"].as_str().unwrap_or_default();
    let limit = match kind {
        "text" => Some(MAX_TEXT),
        "badge" => Some(MAX_BADGE),
        "icon" | "progress" => None,
        "row" | "stack" => {
            n["children"].as_array().into_iter().flatten().for_each(|c| node(c, depth + 1, count, problems, user));
            None
        }
        other => {
            problems.push(format!("{user}: unknown node type {other:?}"));
            None
        }
    };
    if let Some(limit) = limit.filter(|l| n["text"].as_str().is_some_and(|t| t.chars().count() > *l)) {
        problems.push(format!("{user}: a {kind} over {limit} characters"));
    }
    if n["tone"].as_str().is_some_and(|t| !TONES.contains(&t)) {
        problems.push(format!("{user}: unknown tone {}", n["tone"]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(answer: &Value) -> String {
        json!({"ok": true, "output": answer.to_string()}).to_string()
    }

    #[test]
    fn a_good_answer_passes_and_a_bad_one_names_what_slim_would_drop() {
        let good = json!({"$slim": "slot/1", "items": {"u": {"nodes": [{"type": "text", "text": "Lv 2", "tone": "accent"}], "fallback": "Lv 2"}}});
        assert!(check(&reply(&good), &["u"]).is_empty());
        let bad = json!({"$slim": "slot/1", "items": {"x": {"nodes": [{"type": "blink"}]}}});
        let problems = check(&reply(&bad), &["u"]).join(";");
        assert!(problems.contains("not asked") && problems.contains("no fallback") && problems.contains("blink"), "{problems}");
        assert!(check(&reply(&json!({"items": {}})), &["u"])[0].contains("slot/1"));
    }

    #[test]
    fn nesting_and_badge_length_are_bounded() {
        let deep = json!({"type": "row", "children": [{"type": "row", "children": [{"type": "row", "children": [{"type": "row", "children": [{"type": "icon", "name": "x"}]}]}]}]});
        let long = json!({"type": "badge", "text": "x".repeat(25)});
        let answer = json!({"$slim": "slot/1", "items": {"u": {"nodes": [deep, long], "fallback": "f"}}});
        let problems = check(&reply(&answer), &["u"]).join(";");
        assert!(problems.contains("deeper") && problems.contains("over 24"), "{problems}");
    }

    #[test]
    fn an_undeclared_slot_is_refused() {
        let manifest = json!({"slots": [{"kind": "member.badge", "name": "level"}]});
        let ask = |name: &str| input(&["member.badge".into(), name.into(), "u".into()], "v", Some(&manifest));
        assert!(ask("level").is_ok() && ask("rank").is_err());
    }
}
