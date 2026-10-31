//! What the profile section and the member badge show, as `slot/1`.

use serde_json::{json, Map, Value};

use crate::host::Host;
use crate::levels::{standing, Curve};
use crate::store::Store;

/// Hosts ask for at most 100 members at a time.
const MAX_MEMBERS: usize = 100;

/// Answers an `@slot` run: an item for each asked member that has XP, none for the rest.
pub fn answer<H: Host>(store: &Store<H>, curve: Curve, input: &Value) -> Result<String, String> {
    let name = input["name"].as_str().unwrap_or("");
    let users: Vec<String> = input["user_ids"]
        .as_array()
        .map(|ids| ids.iter().filter_map(|id| id.as_str().map(str::to_string)).take(MAX_MEMBERS).collect())
        .unwrap_or_default();
    let mut items = Map::new();
    if !users.is_empty() {
        for (user, xp) in store.totals(&users)? {
            if xp > 0 {
                items.insert(user, item(name, xp, curve));
            }
        }
    }
    Ok(json!({"$slim": "slot/1", "items": items}).to_string())
}

fn item(slot: &str, xp: i64, curve: Curve) -> Value {
    let level = standing(xp, curve);
    let fallback = format!("Level {}", level.level);
    if slot == "level" {
        return json!({"nodes": [{"type": "badge", "text": format!("Lv {}", level.level), "tone": "accent"}], "fallback": format!("Lv {}", level.level)});
    }
    let bar = if level.needed == 0 {
        json!({"type": "progress", "value": 1, "max": 1, "label": "Max level"})
    } else {
        json!({"type": "progress", "value": level.into, "max": level.needed, "label": format!("{} / {} XP", level.into, level.needed)})
    };
    json!({"nodes": [{"type": "stack", "children": [{"type": "text", "text": fallback, "tone": "accent"}, bar]}], "fallback": fallback})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeHost;

    fn ask(host: &FakeHost, name: &str, users: &[&str]) -> Value {
        let input = json!({"slot": "x", "name": name, "user_ids": users, "viewer_id": "v"});
        serde_json::from_str(&answer(&Store::new(host), Curve::Standard, &input).unwrap()).unwrap()
    }

    #[test]
    fn the_profile_section_is_a_level_and_a_progress_bar() {
        let host = FakeHost::with_xp(&[("ada", 780)]);
        let reply = ask(&host, "xp", &["ada"]);
        assert_eq!(reply["$slim"], "slot/1");
        let children = &reply["items"]["ada"]["nodes"][0]["children"];
        assert_eq!(children[0], json!({"type": "text", "text": "Level 4", "tone": "accent"}));
        assert_eq!(children[1], json!({"type": "progress", "value": 180, "max": 400, "label": "180 / 400 XP"}));
        assert_eq!(reply["items"]["ada"]["fallback"], "Level 4");
    }

    #[test]
    fn the_badge_is_a_short_level_chip() {
        let host = FakeHost::with_xp(&[("ada", 100)]);
        let reply = ask(&host, "level", &["ada"]);
        assert_eq!(reply["items"]["ada"]["nodes"][0]["text"], "Lv 2");
    }

    #[test]
    fn a_member_with_no_xp_or_no_row_gets_no_item() {
        let host = FakeHost::with_xp(&[("zero", 0)]);
        let reply = ask(&host, "xp", &["zero", "nobody"]);
        assert_eq!(reply["items"], json!({}));
    }

    #[test]
    fn asking_for_nobody_makes_no_host_call() {
        let host = FakeHost::default();
        let reply = ask(&host, "xp", &[]);
        assert_eq!(reply["items"], json!({}));
        assert_eq!(host.calls.get(), 0);
    }

    #[test]
    fn the_last_level_shows_a_full_bar() {
        let host = FakeHost::with_xp(&[("ada", i64::MAX / 4)]);
        let reply = ask(&host, "xp", &["ada"]);
        let bar = &reply["items"]["ada"]["nodes"][0]["children"][1];
        assert_eq!(bar["label"], "Max level");
    }
}
