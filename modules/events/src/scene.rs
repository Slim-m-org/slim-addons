//! The frames: a form to plan an event, and the event with its countdown, answers and who said what.

use serde_json::{json, Value};

use crate::event::{Event, GOING, MAYBE, NO, STATUS_LABELS};
use crate::time::{countdown, format_local, USAGE};

const WIDTH: f64 = 100.0;
const TITLE_LINE_CHARS: usize = 24;
const NAME_LINE_CHARS: usize = 34;
const MAX_NAME_LINES: usize = 4;
const MAX_TITLE_LINES: usize = 3;
const REFRESH_MS: u64 = 30_000;
pub const CANCEL: &str = "cancel event";
pub const FORM_USAGE: &str = "title | when | where (optional)";

/// The form, with `note` saying what was wrong with the last attempt.
pub fn form(note: Option<&str>) -> String {
    let status = note.map_or_else(|| format!("{FORM_USAGE}. {USAGE}"), str::to_owned);
    json!({
        "$slim": "scene/1", "width": WIDTH, "height": 60, "background": "surface",
        "ops": [
            {"op": "text", "x": 50.0, "y": 14.0, "s": "new event", "fill": "text", "align": "center", "size": 10.0},
            {"op": "input", "x": 10.0, "y": 26.0, "w": 80.0, "submit": "plan", "placeholder": "Game night | tomorrow 8pm | Voice 1", "max": 200}
        ],
        "status": status, "controls": [], "state": "",
    })
    .to_string()
}

/// What a frame shows once its event is gone from the store.
pub fn gone() -> String {
    json!({
        "$slim": "scene/1", "width": WIDTH, "height": 40, "background": "surface",
        "ops": [{"op": "text", "x": 50.0, "y": 22.0, "s": "this event has been removed", "fill": "muted", "align": "center", "size": 6.0}],
        "status": "events are kept for a day after they start", "controls": [], "state": "",
    })
    .to_string()
}

pub fn event(e: &Event, id: &str, now_ms: i64, note: Option<&str>) -> String {
    let open = !e.x && !e.started(now_ms);
    let mut page = Page { ops: Vec::new(), y: 8.0 };
    let heading = format!("event #{}", e.n);
    page.line(&heading, 4.5, "muted", "center");
    for title in wrap(&e.t, TITLE_LINE_CHARS, MAX_TITLE_LINES) {
        page.line(&title, 6.5, "text", "center");
    }
    page.line(&format_local(e.at, e.z), 4.5, "muted", "center");
    if !e.w.is_empty() {
        page.line(&format!("at {}", e.w), 4.5, "muted", "center");
    }
    let (state, fill) = if e.x { ("cancelled".to_owned(), "danger") } else { (countdown(e.at - now_ms), "accent") };
    page.line(&state, 6.0, fill, "center");
    page.y += 2.0;
    for status in [GOING, MAYBE, NO] {
        let names = e.names(status);
        if names.is_empty() && status != GOING {
            continue;
        }
        page.line(&format!("{} ({})", STATUS_LABELS[usize::from(status)], names.len()), 5.0, "text", "left");
        let listed = if names.is_empty() { "nobody yet".to_owned() } else { names.join(", ") };
        let lines = wrap(&listed, NAME_LINE_CHARS, MAX_NAME_LINES + 1);
        let more = lines.len() > MAX_NAME_LINES;
        for text in lines.iter().take(MAX_NAME_LINES) {
            page.line(text, 4.5, "muted", "left");
        }
        if more {
            page.line("and more", 4.5, "muted", "left");
        }
        page.y += 1.5;
    }
    let Page { mut ops, mut y } = page;
    if open {
        ops.push(json!({"op": "input", "x": 8.0, "y": y, "w": 84.0, "submit": "edit", "placeholder": "host: edit as title | when | where", "max": 200}));
        y += 13.0;
    }
    let controls: Vec<&str> = if open { vec!["Going", "Maybe", "Can't", CANCEL] } else { vec![] };
    let mut scene = json!({
        "$slim": "scene/1", "width": WIDTH, "height": y + 3.0, "background": "surface", "ops": ops,
        "status": note.map_or_else(|| summary(e), str::to_owned), "controls": controls, "state": id, "live": false,
    });
    if open {
        scene["refresh_ms"] = json!(REFRESH_MS);
    }
    scene.to_string()
}

/// Text ops stacked down the frame, each line below the one before.
struct Page {
    ops: Vec<Value>,
    y: f64,
}

impl Page {
    fn line(&mut self, text: &str, size: f64, fill: &str, align: &str) {
        let x = if align == "center" { 50.0 } else { 8.0 };
        self.ops.push(json!({"op": "text", "x": x, "y": self.y, "s": text, "fill": fill, "align": align, "size": size}));
        self.y += size + 2.5;
    }
}

fn summary(e: &Event) -> String {
    let [going, maybe, no] = e.counts();
    format!("{going} going, {maybe} maybe, {no} can't")
}

/// Breaks `text` into lines of at most `width` characters, at spaces where it can, keeping at most `max_lines`.
fn wrap(text: &str, width: usize, max_lines: usize) -> Vec<String> {
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
    lines.truncate(max_lines);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::who;

    fn sample() -> Event {
        Event { n: 3, t: "Game night".into(), at: 10_000_000_000, z: 120, w: "Voice 1".into(), by: who("h"), ch: String::new(), x: false, r: false, rem: 0, rs: vec![] }
    }

    fn parsed(raw: &str) -> Value {
        serde_json::from_str(raw).unwrap()
    }

    fn texts(scene: &Value) -> Vec<String> {
        scene["ops"].as_array().unwrap().iter().filter_map(|o| o["s"].as_str().map(str::to_owned)).collect()
    }

    #[test]
    fn an_upcoming_event_counts_down_in_the_hosts_zone_and_offers_the_answers() {
        let scene = parsed(&event(&sample(), "ab12", 10_000_000_000 - 90 * 60_000, None));
        let shown = texts(&scene);
        assert!(shown.contains(&"Sun 26 Apr 1970, 19:46 UTC+02:00".to_owned()), "{shown:?}");
        assert!(shown.contains(&"starts in 1h 30m".to_owned()) && shown.contains(&"at Voice 1".to_owned()));
        assert_eq!(scene["controls"], json!(["Going", "Maybe", "Can't", CANCEL]));
        assert_eq!((scene["state"].clone(), scene["refresh_ms"].clone()), (json!("ab12"), json!(30_000)));
    }

    #[test]
    fn the_frame_lists_who_is_going_and_says_so_when_nobody_is() {
        let mut e = sample();
        assert!(texts(&parsed(&event(&e, "i", 0, None))).contains(&"nobody yet".to_owned()));
        e.answer("alice", "Alice", GOING).ok();
        e.answer("bob", "Bob", NO).ok();
        let scene = parsed(&event(&e, "i", 0, None));
        let shown = texts(&scene);
        assert!(shown.contains(&"Going (1)".to_owned()) && shown.contains(&"Alice".to_owned()) && shown.contains(&"Can't (1)".to_owned()));
        assert_eq!(scene["status"], "1 going, 0 maybe, 1 can't");
    }

    #[test]
    fn a_long_list_is_cut_with_and_more_instead_of_growing_forever() {
        let mut e = sample();
        for i in 0..crate::event::MAX_ANSWERS {
            e.answer(&format!("caller-{i:04}"), &format!("Guest number {i}"), GOING).ok();
        }
        let shown = texts(&parsed(&event(&e, "i", 0, None)));
        assert!(shown.contains(&"and more".to_owned()));
        assert!(shown.len() < 30, "{}", shown.len());
    }

    #[test]
    fn a_started_or_cancelled_event_has_nothing_to_press_and_stops_refreshing() {
        for (cancelled, now) in [(false, 10_000_000_000 + 5 * 60_000), (true, 0)] {
            let mut e = sample();
            e.x = cancelled;
            let scene = parsed(&event(&e, "i", now, None));
            assert_eq!(scene["controls"], json!([]));
            assert!(scene.get("refresh_ms").is_none());
            assert!(scene["ops"].as_array().unwrap().iter().all(|o| o["op"] != "input"));
        }
        assert!(texts(&parsed(&event(&sample(), "i", 10_000_000_000 + 5 * 60_000, None))).contains(&"started 5m ago".to_owned()));
        assert!(texts(&parsed(&event(&Event { x: true, ..sample() }, "i", 0, None))).contains(&"cancelled".to_owned()));
    }

    #[test]
    fn a_long_title_wraps_and_the_form_and_gone_frames_are_scenes() {
        let mut e = sample();
        e.t = "word ".repeat(40);
        let shown = texts(&parsed(&event(&e, "i", 0, None)));
        assert_eq!(shown.iter().filter(|s| s.starts_with("word")).count(), MAX_TITLE_LINES);
        assert_eq!(parsed(&form(Some("nope")))["status"], "nope");
        assert!(parsed(&form(None))["status"].as_str().unwrap().contains("title | when"));
        assert_eq!(parsed(&gone())["controls"], json!([]));
    }
}
