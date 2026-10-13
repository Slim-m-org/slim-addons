//! The initiative tracker as an interactive scene: draws the order, and turns a
//! tap, control or typed line into the next frame.

use serde_json::{json, Value};

use crate::initiative::Tracker;
use crate::rng::{Dice, SplitMix64};

const WIDTH: i64 = 100;
const ROW_H: i64 = 9;
const TOP: i64 = 12;
const INPUT_H: i64 = 14;

#[derive(serde::Deserialize, Default)]
struct Frame {
    #[serde(default)]
    action: String,
    #[serde(default)]
    state: String,
}

/// The `initiative` command: an empty input launches a fresh tracker, a JSON frame advances one.
pub fn apply(input: &str, entropy: &str) -> Result<String, String> {
    let frame: Frame = serde_json::from_str(input).unwrap_or_default();
    let mut tracker = Tracker::from_state(&frame.state);
    let note = act(&mut tracker, &frame.action, entropy);
    Ok(scene(&tracker, note).to_string())
}

fn act(tracker: &mut Tracker, action: &str, entropy: &str) -> Option<String> {
    match action {
        "" => None,
        "next" => {
            tracker.next();
            None
        }
        "sort" => {
            tracker.sort();
            None
        }
        "clear" => {
            tracker.clear();
            Some("cleared".to_string())
        }
        _ => {
            if let Some(line) = action.strip_prefix("add:") {
                return add(tracker, line, entropy, action);
            }
            let index = |prefix: &str| action.strip_prefix(prefix)?.parse::<usize>().ok();
            if let Some(i) = index("set") {
                tracker.set_turn(i);
            } else if let Some(i) = index("rm") {
                return tracker.remove(i).map(|gone| format!("removed {}", gone.name));
            }
            None
        }
    }
}

/// `Goblin 14` adds with that initiative; `Goblin`, `Goblin r` and `Goblin r+2` roll a d20 for it.
fn add(tracker: &mut Tracker, line: &str, entropy: &str, seed: &str) -> Option<String> {
    let mut words: Vec<&str> = line.split_whitespace().collect();
    let (init, rolled) = match words.last().and_then(|w| initiative_word(w)) {
        Some(Initiative::Fixed(n)) => (n, None),
        Some(Initiative::Roll(modifier)) => (roll_for(entropy, seed, modifier), Some(modifier)),
        None => (roll_for(entropy, seed, 0), Some(0)),
    };
    if words.last().is_some_and(|w| initiative_word(w).is_some()) {
        words.pop();
    }
    match tracker.add(&words.join(" "), init) {
        Ok(()) if rolled.is_some() => Some(format!("{} rolled {init} for initiative", tracker.list.last()?.name)),
        Ok(()) => None,
        Err(refusal) => Some(refusal),
    }
}

enum Initiative {
    Fixed(i32),
    Roll(i32),
}

fn initiative_word(word: &str) -> Option<Initiative> {
    if let Ok(n) = word.parse::<i32>() {
        return (-99..=999).contains(&n).then_some(Initiative::Fixed(n));
    }
    let modifier = word.to_ascii_lowercase().strip_prefix('r')?.to_string();
    if modifier.is_empty() {
        return Some(Initiative::Roll(0));
    }
    let n: i32 = modifier.parse().ok().filter(|_| modifier.starts_with(['+', '-']))?;
    (-99..=99).contains(&n).then_some(Initiative::Roll(n))
}

fn roll_for(entropy: &str, seed: &str, modifier: i32) -> i32 {
    let mut rng = SplitMix64::seeded(entropy.as_bytes(), seed.as_bytes());
    rng.roll(20) as i32 + modifier
}

fn scene(tracker: &Tracker, note: Option<String>) -> Value {
    let mut ops: Vec<Value> = vec![json!({
        "op": "text", "x": 3, "y": 6, "s": format!("Round {}", tracker.round),
        "fill": "muted", "size": 4.5, "align": "left"
    })];
    for (i, who) in tracker.list.iter().enumerate() {
        let top = TOP + i as i64 * ROW_H;
        let mid = top as f64 + 4.0;
        let current = i == tracker.turn;
        let ink = if current { "bg" } else { "text" };
        ops.push(json!({
            "op": "rect", "x": 2, "y": top, "w": 96, "h": 8, "r": 2,
            "fill": if current { "accent" } else { "sunken" }, "tap": format!("set{i}")
        }));
        ops.push(json!({"op": "text", "x": 5, "y": mid, "s": who.name, "fill": ink, "size": 4.5, "align": "left"}));
        ops.push(json!({"op": "text", "x": 82, "y": mid, "s": who.init.to_string(), "fill": ink, "size": 4.5, "align": "right"}));
        ops.push(json!({"op": "rect", "x": 86, "y": top + 1, "w": 10, "h": 6, "r": 2, "fill": "border", "tap": format!("rm{i}")}));
        ops.push(json!({"op": "text", "x": 91, "y": mid, "s": "x", "fill": "text", "size": 4, "align": "center"}));
    }
    let input_y = TOP + tracker.list.len() as i64 * ROW_H + 3;
    ops.push(json!({
        "op": "input", "submit": "add", "x": 2, "y": input_y, "w": 96, "value": "", "max": 40,
        "placeholder": "name and initiative, e.g. Goblin 14"
    }));
    let status = note.unwrap_or_else(|| match tracker.current() {
        Some(who) => format!("{}'s turn", who.name),
        None => "no combatants yet - add a name and initiative below".to_string(),
    });
    json!({
        "$slim": "scene/1", "width": WIDTH, "height": input_y + INPUT_H + 2, "background": "surface",
        "ops": ops, "status": status, "controls": ["next", "sort", "clear"],
        "state": tracker.state(), "live": true
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(action: &str, state: &str) -> Value {
        let input = json!({"action": action, "state": state}).to_string();
        serde_json::from_str(&apply(&input, "entropy").unwrap()).unwrap()
    }

    fn state_of(scene: &Value) -> String {
        scene["state"].as_str().unwrap().to_string()
    }

    fn after(actions: &[&str]) -> Value {
        let mut state = state_of(&frame("", ""));
        let mut last = frame("", "");
        for action in actions {
            last = frame(action, &state);
            state = state_of(&last);
        }
        last
    }

    fn texts(scene: &Value) -> Vec<String> {
        scene["ops"].as_array().unwrap().iter().filter_map(|o| o["s"].as_str().map(str::to_string)).collect()
    }

    #[test]
    fn an_empty_launch_draws_an_empty_tracker_with_its_controls() {
        let scene = frame("", "");
        assert_eq!(scene["$slim"], "scene/1");
        assert_eq!(scene["controls"], json!(["next", "sort", "clear"]));
        assert!(scene["status"].as_str().unwrap().contains("no combatants"));
        assert_eq!(scene["ops"].as_array().unwrap().last().unwrap()["op"], "input");
        assert_eq!(apply("not a frame", "").unwrap(), apply("", "").unwrap());
    }

    #[test]
    fn adding_sorting_and_advancing_highlights_the_current_combatant() {
        let scene = after(&["add:Goblin 12", "add:Aria 18", "add:Bo 7", "sort"]);
        assert_eq!(texts(&scene)[..7], ["Round 1", "Aria", "18", "x", "Goblin", "12", "x"]);
        assert_eq!(scene["status"], "Aria's turn");
        let scene = after(&["add:Goblin 12", "add:Aria 18", "sort", "next"]);
        assert_eq!(scene["status"], "Goblin's turn");
        let fills: Vec<&str> =
            scene["ops"].as_array().unwrap().iter().filter(|o| o["op"] == "rect" && o["tap"].as_str().is_some_and(|t| t.starts_with("set"))).map(|o| o["fill"].as_str().unwrap()).collect();
        assert_eq!(fills, ["sunken", "accent"]);
    }

    #[test]
    fn the_round_counts_up_when_the_order_wraps() {
        let scene = after(&["add:A 3", "add:B 2", "next", "next"]);
        assert_eq!(texts(&scene)[0], "Round 2");
        assert_eq!(scene["status"], "A's turn");
    }

    #[test]
    fn tapping_a_row_sets_the_turn_and_its_x_removes_it() {
        let scene = after(&["add:A 3", "add:B 2", "add:C 1", "set2"]);
        assert_eq!(scene["status"], "C's turn");
        let scene = after(&["add:A 3", "add:B 2", "add:C 1", "set2", "rm0"]);
        assert_eq!(scene["status"], "removed A");
        assert_eq!(Tracker::from_state(scene["state"].as_str().unwrap()).current().unwrap().name, "C");
        assert_eq!(after(&["add:A 3", "rm0"])["status"], "removed A");
        assert!(after(&["add:A 3", "rm9"])["status"].as_str().unwrap().contains("A's turn"));
    }

    #[test]
    fn clear_empties_the_fight() {
        let scene = after(&["add:A 3", "add:B 2", "next", "clear"]);
        assert_eq!(scene["status"], "cleared");
        assert_eq!(texts(&scene), ["Round 1"]);
    }

    #[test]
    fn a_name_with_spaces_and_a_negative_initiative_are_kept() {
        let scene = after(&["add:Dire Wolf -2"]);
        assert_eq!(texts(&scene)[1..3], ["Dire Wolf", "-2"]);
    }

    #[test]
    fn leaving_initiative_off_rolls_a_d20_for_it() {
        let scene = after(&["add:Goblin"]);
        let rolled: i32 = texts(&scene)[2].parse().unwrap();
        assert!((1..=20).contains(&rolled), "{rolled}");
        assert!(scene["status"].as_str().unwrap().starts_with("Goblin rolled"));
        let boosted: i32 = texts(&after(&["add:Goblin r+5"]))[2].parse().unwrap();
        assert!((6..=25).contains(&boosted), "{boosted}");
        assert_eq!(texts(&after(&["add:r 4"]))[1], "r");
    }

    #[test]
    fn a_blank_name_and_a_full_roster_are_explained_not_dropped() {
        assert_eq!(after(&["add:  "])["status"], "give the combatant a name");
        assert_eq!(after(&["add:14"])["status"], "give the combatant a name");
        let many: Vec<String> = (0..25).map(|i| format!("add:g{i} 1")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        assert!(after(&refs)["status"].as_str().unwrap().contains("at most"));
    }

    #[test]
    fn the_scene_height_grows_with_the_roster_and_keeps_a_wide_ratio() {
        let small = frame("", "")["height"].as_i64().unwrap();
        let big = after(&["add:A 1", "add:B 2"])["height"].as_i64().unwrap();
        assert_eq!(big - small, 2 * ROW_H);
        assert!(small * 4 >= WIDTH, "{small} is too short for a 4:1 scene");
    }
}
