//! The frames: a form to plan a night, and the night itself with its times, answers and countdown.

use serde_json::{json, Value};

use crate::night::{Night, Tally, CANCELLED, OPEN};
use crate::parse::USAGE;

const WIDTH: f64 = 100.0;
const ROW_H: f64 = 10.0;
const ROW_GAP: f64 = 3.0;
const TITLE_LINE_CHARS: usize = 26;
const TITLE_LEADING: f64 = 6.5;
pub const CLOSE: &str = "close poll";
pub const CANCEL: &str = "cancel";
pub const REFRESH: &str = "refresh";

/// The form, with `note` saying what was wrong with the last attempt.
pub fn form(note: Option<&str>) -> String {
    json!({
        "$slim": "scene/1", "width": WIDTH, "height": 60, "background": "surface",
        "ops": [
            {"op": "text", "x": 50.0, "y": 14.0, "s": "game night", "fill": "text", "align": "center", "size": 10.0},
            {"op": "input", "x": 10.0, "y": 26.0, "w": 80.0, "submit": "plan", "placeholder": "game | fri 8pm, sat 9pm | role", "max": 160}
        ],
        "status": note.unwrap_or(USAGE), "controls": [], "state": "",
    })
    .to_string()
}

pub fn night(night: &Night, tally: &Tally, now_ms: i64, note: Option<&str>) -> String {
    let title = wrap(&night.game, TITLE_LINE_CHARS);
    let mut ops: Vec<Value> = title
        .iter()
        .enumerate()
        .map(|(i, line)| json!({"op": "text", "x": 50.0, "y": 7.0 + i as f64 * TITLE_LEADING, "s": line, "fill": "text", "align": "center", "size": 6.0}))
        .collect();
    let mut y = 12.0 + title.len() as f64 * TITLE_LEADING;
    let voting = night.chosen == OPEN;
    let top = tally.per_slot.iter().copied().max().unwrap_or(0).max(1);
    for (i, slot) in night.slots.iter().enumerate() {
        let picked = night.chosen == i as i64;
        let mine = tally.mine_slots.contains(&i);
        let mut track = json!({"op": "rect", "x": 10.0, "y": y, "w": 80.0, "h": ROW_H, "r": 2.0, "fill": "sunken"});
        if voting {
            track["tap"] = json!(format!("vote:{i}"));
        }
        if mine || picked {
            track["stroke"] = json!("accent");
            track["sw"] = json!(0.8);
        }
        ops.push(track);
        let votes = tally.per_slot.get(i).copied().unwrap_or(0);
        if votes > 0 {
            let fill = 80.0 * votes as f64 / top as f64;
            ops.push(json!({"op": "rect", "x": 10.0, "y": y, "w": fill, "h": ROW_H, "r": 2.0, "grad": {"from": "accent", "to": "accent-soft", "dir": "h"}}));
        }
        ops.push(json!({"op": "text", "x": 13.0, "y": y + 7.0, "s": slot.label, "fill": "text", "align": "left", "size": 5.5}));
        ops.push(json!({"op": "text", "x": 87.0, "y": y + 7.0, "s": votes.to_string(), "fill": "text", "align": "right", "size": 5.5}));
        y += ROW_H + ROW_GAP;
    }
    y += 2.0;
    if night.chosen != CANCELLED {
        for (i, (name, tap)) in [("In", "in"), ("Maybe", "maybe"), ("Out", "out")].into_iter().enumerate() {
            let x = 10.0 + i as f64 * 28.0;
            let mut button = json!({"op": "rect", "x": x, "y": y, "w": 24.0, "h": ROW_H, "r": 2.0, "fill": "sunken", "tap": tap});
            if tally.mine_answer == Some(i as i64) {
                button["stroke"] = json!("accent");
                button["sw"] = json!(0.8);
            }
            ops.push(button);
            ops.push(json!({"op": "text", "x": x + 12.0, "y": y + 7.0, "s": format!("{name} {}", tally.answers[i]), "fill": "text", "align": "center", "size": 5.0}));
        }
        y += ROW_H + ROW_GAP;
    }
    let controls: Vec<&str> = match night.chosen {
        OPEN => vec![CLOSE, CANCEL, REFRESH],
        CANCELLED => vec![],
        _ => vec![CANCEL, REFRESH],
    };
    json!({
        "$slim": "scene/1", "width": WIDTH, "height": y + 3.0, "background": "surface", "ops": ops,
        "status": note.map_or_else(|| status(night, tally, now_ms), str::to_owned),
        "controls": controls, "state": night.id, "live": false,
    })
    .to_string()
}

fn status(night: &Night, tally: &Tally, now_ms: i64) -> String {
    match night.chosen {
        OPEN => {
            let voters: i64 = tally.answers.iter().sum();
            format!("{voters} answered - tap the times that work, then say if you are in")
        }
        CANCELLED => "cancelled".to_owned(),
        chosen => {
            let slot = &night.slots[(chosen as usize).min(night.slots.len().saturating_sub(1))];
            format!("{} - {} - {} in, {} maybe", slot.label, countdown(slot.start_ms - now_ms), tally.answers[0], tally.answers[1])
        }
    }
}

/// How far off a start is, or how long ago it was.
pub fn countdown(ms: i64) -> String {
    let (minutes, past) = (ms.abs() / 60_000, ms < 0);
    let span = match minutes {
        0 => return "starting now".to_owned(),
        1..=59 => format!("{minutes}m"),
        60..=1439 => format!("{}h {}m", minutes / 60, minutes % 60),
        _ => format!("{}d {}h", minutes / 1440, minutes % 1440 / 60),
    };
    if past { format!("started {span} ago") } else { format!("starts in {span}") }
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if line.chars().count() + 1 + word.chars().count() <= width => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.chars().take(width).collect()),
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::Slot;

    fn sample(chosen: i64) -> Night {
        let slots = vec![Slot { label: "Fri 20:00".into(), start_ms: 10_000_000 }, Slot { label: "Sat 20:00".into(), start_ms: 20_000_000 }];
        Night { id: "n1".into(), game: "Valorant".into(), role: "Gamers".into(), host: "h".into(), slots, chosen, remind_at: 0, created: 0 }
    }

    fn parsed(raw: &str) -> Value {
        serde_json::from_str(raw).unwrap()
    }

    #[test]
    fn an_open_night_offers_a_tap_per_time_and_an_answer_row() {
        let tally = Tally { per_slot: vec![3, 1], answers: [2, 1, 0], mine_slots: vec![1], mine_answer: Some(0) };
        let scene = parsed(&night(&sample(OPEN), &tally, 0, None));
        let taps: Vec<&str> = scene["ops"].as_array().unwrap().iter().filter_map(|o| o["tap"].as_str()).collect();
        assert_eq!(taps, ["vote:0", "vote:1", "in", "maybe", "out"]);
        assert_eq!(scene["controls"], json!([CLOSE, CANCEL, REFRESH]));
        assert_eq!(scene["state"], "n1");
        assert!(scene["status"].as_str().unwrap().starts_with("3 answered"));
    }

    #[test]
    fn a_closed_night_stops_taking_times_and_counts_down() {
        let tally = Tally { per_slot: vec![3, 1], answers: [2, 1, 0], ..Tally::default() };
        let scene = parsed(&night(&sample(0), &tally, 10_000_000 - 130 * 60_000, None));
        let taps: Vec<&str> = scene["ops"].as_array().unwrap().iter().filter_map(|o| o["tap"].as_str()).collect();
        assert_eq!(taps, ["in", "maybe", "out"]);
        assert_eq!(scene["status"], "Fri 20:00 - starts in 2h 10m - 2 in, 1 maybe");
        assert_eq!(scene["controls"], json!([CANCEL, REFRESH]));
    }

    #[test]
    fn a_cancelled_night_has_nothing_to_tap() {
        let scene = parsed(&night(&sample(CANCELLED), &Tally::default(), 0, None));
        assert_eq!((scene["status"].clone(), scene["controls"].clone()), (json!("cancelled"), json!([])));
        assert!(scene["ops"].as_array().unwrap().iter().all(|o| o.get("tap").is_none()));
    }

    #[test]
    fn countdowns_read_naturally() {
        assert_eq!(countdown(30_000), "starting now");
        assert_eq!(countdown(14 * 60_000), "starts in 14m");
        assert_eq!(countdown(-5 * 60_000), "started 5m ago");
        assert_eq!(countdown(50 * 3_600_000), "starts in 2d 2h");
    }
}
