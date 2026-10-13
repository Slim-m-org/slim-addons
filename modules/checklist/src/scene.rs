//! Draws a [`Frame`] as a scene: the title, one tappable row per item, the add field, the notes and the rename field.

use serde_json::{json, Value};

use crate::list::{Frame, List, MAX_ITEM, MAX_NOTES, MAX_TITLE};

const WIDTH: f64 = 120.0;
const MARGIN: f64 = 6.0;
const ROW: f64 = 9.0;
const FIELD: f64 = 12.0;
/// Characters that fit on one line of notes at the notes text size.
const NOTES_COLS: usize = 54;
const NOTES_LEADING: f64 = 5.0;
const CHAR_WIDTH: f64 = 2.0;

pub fn render(frame: &Frame) -> String {
    let list = &frame.list;
    let mut ops: Vec<Value> = Vec::new();
    let mut y = header(&mut ops, list);

    if list.items.is_empty() {
        ops.push(text(MARGIN + 3.0, y + 6.0, "nothing here yet - add the first item", "muted", 4.0, "left"));
        y += ROW;
    }
    for (i, item) in list.items.iter().enumerate() {
        item_row(&mut ops, i, &item.text, item.done, y);
        y += ROW;
    }

    y += 3.0;
    field(&mut ops, y, "add", "add an item", "", MAX_ITEM);
    y += FIELD + 5.0;

    ops.push(text(MARGIN, y + 3.0, "Notes", "muted", 4.0, "left"));
    y += 6.0;
    for line in wrap(&list.notes, NOTES_COLS) {
        ops.push(text(MARGIN, y + 3.0, &line, "text", 3.6, "left"));
        y += NOTES_LEADING;
    }
    y += 2.0;
    field(&mut ops, y, "notes", "shared notes", &list.notes, MAX_NOTES);
    y += FIELD + 5.0;

    field(&mut ops, y, "title", "rename the list", &list.title, MAX_TITLE);
    y += FIELD + 4.0;

    let status = frame.status.clone().unwrap_or_else(|| summary(list));
    json!({
        "$slim": "scene/1",
        "width": WIDTH,
        "height": y,
        "background": "surface",
        "ops": ops,
        "status": status,
        "controls": ["clear done"],
        "state": list.state(),
        "live": true
    })
    .to_string()
}

fn header(ops: &mut Vec<Value>, list: &List) -> f64 {
    ops.push(text(MARGIN, 10.0, &list.title, "text", 7.0, "left"));
    if !list.items.is_empty() {
        let progress = format!("{}/{}", list.done_count(), list.items.len());
        ops.push(text(WIDTH - MARGIN, 10.0, &progress, "muted", 5.0, "right"));
    }
    16.0
}

fn summary(list: &List) -> String {
    match list.items.len() {
        0 => "tap an item to tick it".to_string(),
        n => format!("{} of {n} done", list.done_count()),
    }
}

fn item_row(ops: &mut Vec<Value>, index: usize, label: &str, done: bool, y: f64) {
    let tone = if done { "muted" } else { "text" };
    ops.push(json!({
        "op": "rect", "x": MARGIN, "y": y, "w": WIDTH - 2.0 * MARGIN - 12.0, "h": ROW - 1.0,
        "fill": "sunken", "r": 2, "tap": format!("t:{index}")
    }));
    let box_fill = if done { "accent" } else { "surface" };
    ops.push(json!({
        "op": "rect", "x": MARGIN + 3.0, "y": y + 1.75, "w": 5, "h": 5,
        "fill": box_fill, "stroke": "border", "sw": 0.5, "r": 1
    }));
    ops.push(text(MARGIN + 11.0, y + 5.8, label, tone, 4.0, "left"));
    if done {
        let x1 = MARGIN + 11.0;
        let x2 = x1 + label.chars().count() as f64 * CHAR_WIDTH;
        ops.push(json!({"op": "line", "x1": x1, "y1": y + 4.2, "x2": x2, "y2": y + 4.2, "stroke": "muted", "sw": 0.5}));
    }
    remove_button(ops, index, y);
}

fn remove_button(ops: &mut Vec<Value>, index: usize, y: f64) {
    let x = WIDTH - MARGIN - 10.0;
    ops.push(json!({
        "op": "rect", "x": x, "y": y, "w": 10, "h": ROW - 1.0,
        "fill": "sunken", "r": 2, "tap": format!("x:{index}")
    }));
    let (cx, cy) = (x + 5.0, y + (ROW - 1.0) / 2.0);
    for (dx, dy) in [(1.0, 1.0), (1.0, -1.0)] {
        ops.push(json!({
            "op": "line", "x1": cx - 1.8 * dx, "y1": cy - 1.8 * dy, "x2": cx + 1.8 * dx, "y2": cy + 1.8 * dy,
            "stroke": "muted", "sw": 0.7
        }));
    }
}

fn field(ops: &mut Vec<Value>, y: f64, submit: &str, placeholder: &str, value: &str, max: usize) {
    ops.push(json!({
        "op": "input", "x": MARGIN, "y": y, "w": WIDTH - 2.0 * MARGIN,
        "submit": submit, "placeholder": placeholder, "value": value, "max": max
    }));
}

fn text(x: f64, y: f64, s: &str, fill: &str, size: f64, align: &str) -> Value {
    json!({"op": "text", "x": x, "y": y, "s": s, "fill": fill, "size": size, "align": align})
}

/// Greedy word wrap to `cols` characters; a word longer than a line is broken where the line ends.
fn wrap(text: &str, cols: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let mut word: Vec<char> = word.chars().collect();
        while word.len() > cols {
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            lines.push(word.drain(..cols).collect());
        }
        let word: String = word.into_iter().collect();
        let needed = if line.is_empty() { word.chars().count() } else { line.chars().count() + 1 + word.chars().count() };
        if needed > cols {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(&word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::list::{apply, Item, MAX_ITEMS};

    fn scene(list: List, status: Option<&str>) -> Value {
        serde_json::from_str(&render(&Frame { list, status: status.map(String::from) })).unwrap()
    }

    fn list(items: &[(&str, bool)]) -> List {
        List {
            title: "Trip".into(),
            items: items.iter().map(|(s, d)| Item { text: s.to_string(), done: *d }).collect(),
            notes: String::new(),
        }
    }

    fn ops_of<'a>(scene: &'a Value, kind: &str) -> Vec<&'a Value> {
        scene["ops"].as_array().unwrap().iter().filter(|o| o["op"] == kind).collect()
    }

    #[test]
    fn launch_scene_is_interactive_and_names_its_inputs() {
        let s: Value = serde_json::from_str(&apply("checklist", "").unwrap()).unwrap();
        assert_eq!(s["$slim"], "scene/1");
        assert_eq!(s["live"], true);
        assert_eq!(s["controls"], json!(["clear done"]));
        let submits: Vec<&str> = ops_of(&s, "input").iter().map(|o| o["submit"].as_str().unwrap()).collect();
        assert_eq!(submits, ["add", "notes", "title"]);
    }

    #[test]
    fn every_item_has_a_tick_and_a_remove_tap() {
        let s = scene(list(&[("a", false), ("b", true)]), None);
        let taps: Vec<&str> = ops_of(&s, "rect").iter().filter_map(|o| o["tap"].as_str()).collect();
        assert_eq!(taps, ["t:0", "x:0", "t:1", "x:1"]);
    }

    #[test]
    fn a_done_item_is_struck_through_and_an_open_one_is_not() {
        assert_eq!(ops_of(&scene(list(&[("a", true)]), None), "line").len(), 3);
        assert_eq!(ops_of(&scene(list(&[("a", false)]), None), "line").len(), 2);
    }

    #[test]
    fn status_shows_progress_or_the_refusal() {
        assert_eq!(scene(list(&[("a", true), ("b", false)]), None)["status"], "1 of 2 done");
        assert_eq!(scene(list(&[]), Some("too long"))["status"], "too long");
    }

    #[test]
    fn state_in_the_scene_is_the_list() {
        let l = list(&[("a", true)]);
        assert_eq!(scene(l.clone(), None)["state"], l.state());
    }

    #[test]
    fn a_full_list_with_full_notes_stays_a_small_scene_with_a_sane_shape() {
        let mut l = list(&[]);
        l.items = (0..MAX_ITEMS).map(|_| Item { text: "w".repeat(MAX_ITEM), done: true }).collect();
        l.notes = "word ".repeat(MAX_NOTES / 5);
        let raw = render(&Frame { list: l, status: None });
        assert!(raw.len() < 32 * 1024, "{}", raw.len());
        let s: Value = serde_json::from_str(&raw).unwrap();
        assert!(s["height"].as_f64().unwrap() / WIDTH < 4.0);
    }

    #[test]
    fn wrap_breaks_on_words_and_splits_overlong_words() {
        assert_eq!(wrap("aa bb cc", 5), ["aa bb", "cc"]);
        assert_eq!(wrap("abcdefg hi", 3), ["abc", "def", "g", "hi"]);
        assert!(wrap("   ", 5).is_empty());
    }

    #[test]
    fn notes_over_several_lines_are_all_drawn() {
        let mut l = list(&[]);
        l.notes = "word ".repeat(40);
        let s = scene(l, None);
        let drawn = ops_of(&s, "text").iter().filter(|o| o["size"] == 3.6).count();
        assert!(drawn >= 3, "{drawn}");
    }
}
