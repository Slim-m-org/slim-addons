//! The board app: tag names as rows, tap one to read it.
//!
//! An app frame runs through the shared code-run route, which hands a module no
//! `message.post`, so the board can show a tag but not post it; the status line says so.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::host::Host;
use crate::name;
use crate::store::Tags;

const WIDTH: f64 = 120.0;
const MARGIN: f64 = 6.0;
const ROW: f64 = 9.0;
const TEXT_COLS: usize = 54;
const TEXT_LEADING: f64 = 5.0;

#[derive(Deserialize)]
struct Action {
    #[serde(default)]
    action: String,
    #[serde(default)]
    state: String,
}

/// The next frame for the input the client sent: empty on launch, else `{action, state}`.
pub fn apply<H: Host>(host: &H, input: &str) -> Result<String, String> {
    let sent: Action = if input.trim().is_empty() {
        Action { action: String::new(), state: String::new() }
    } else {
        serde_json::from_str(input).map_err(|_| "The board did not understand that.".to_string())?
    };
    let wanted = sent.action.strip_prefix("show:").unwrap_or(&sent.state);
    let tags = Tags::new(host);
    let frame = match load(&tags, wanted) {
        Ok(frame) => frame,
        Err(problem) => Frame { names: Vec::new(), open: None, status: problem },
    };
    Ok(render(&frame))
}

struct Frame {
    names: Vec<String>,
    /// The tag being read, with its text.
    open: Option<(String, String)>,
    status: String,
}

fn load<H: Host>(tags: &Tags<H>, wanted: &str) -> Result<Frame, String> {
    let names = tags.names()?;
    let mut status = if names.is_empty() { "No tags yet.".to_string() } else { "Tap a tag to read it.".to_string() };
    let mut open = None;
    if !wanted.is_empty() {
        match name::normalize(wanted).ok().map(|n| tags.get(&n)).transpose()?.flatten() {
            Some(text) => {
                open = Some((wanted.to_string(), text));
                status = "The board only shows a tag. To post it, type /tag and its name.".to_string();
            }
            None => status = format!("{wanted} is gone."),
        }
    }
    Ok(Frame { names, open, status })
}

fn render(frame: &Frame) -> String {
    let mut ops: Vec<Value> = vec![text(MARGIN, 10.0, "Tags", "text", 7.0, "left")];
    if !frame.names.is_empty() {
        ops.push(text(WIDTH - MARGIN, 10.0, &format!("{} saved", frame.names.len()), "muted", 5.0, "right"));
    }
    let mut y = 16.0;

    if let Some((name, body)) = &frame.open {
        ops.push(text(MARGIN, y + 4.0, name, "accent", 5.0, "left"));
        y += 7.0;
        for line in wrap(body, TEXT_COLS) {
            ops.push(text(MARGIN, y + 3.0, &line, "text", 3.6, "left"));
            y += TEXT_LEADING;
        }
        y += 4.0;
    }

    if frame.names.is_empty() {
        ops.push(text(MARGIN + 3.0, y + 6.0, "nothing saved - a manager adds one with /tagadd", "muted", 4.0, "left"));
        y += ROW;
    }
    for name in &frame.names {
        let selected = frame.open.as_ref().is_some_and(|(open, _)| open == name);
        ops.push(row(name, y, selected));
        ops.push(text(MARGIN + 3.0, y + 5.8, name, "text", 4.0, "left"));
        y += ROW;
    }

    let state = frame.open.as_ref().map_or("", |(name, _)| name.as_str());
    json!({
        "$slim": "scene/1",
        "width": WIDTH,
        "height": y + 4.0,
        "background": "surface",
        "ops": ops,
        "status": frame.status,
        "controls": ["refresh"],
        "state": state,
        "live": true
    })
    .to_string()
}

fn row(name: &str, y: f64, selected: bool) -> Value {
    let mut op = json!({
        "op": "rect", "x": MARGIN, "y": y, "w": WIDTH - 2.0 * MARGIN, "h": ROW - 1.0,
        "fill": "sunken", "r": 2, "tap": format!("show:{name}")
    });
    if selected {
        op["stroke"] = json!("accent");
        op["sw"] = json!(0.6);
    }
    op
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
    use crate::fake::FakeHost;
    use crate::store::MAX_TAGS;

    fn seeded(pairs: &[(&str, &str)]) -> FakeHost {
        let host = FakeHost::default();
        for (name, body) in pairs {
            Tags::new(&host).add(name, body).unwrap();
            host.next_run();
        }
        host
    }

    fn frame(host: &FakeHost, input: &str) -> Value {
        serde_json::from_str(&apply(host, input).unwrap()).unwrap()
    }

    fn taps(scene: &Value) -> Vec<&str> {
        scene["ops"].as_array().unwrap().iter().filter_map(|o| o["tap"].as_str()).collect()
    }

    #[test]
    fn the_launch_frame_has_a_tappable_row_per_tag() {
        let s = frame(&seeded(&[("rules", "r"), ("faq", "f")]), "");
        assert_eq!(s["$slim"], "scene/1");
        assert_eq!(s["live"], true);
        assert_eq!(taps(&s), ["show:faq", "show:rules"]);
        assert_eq!(s["status"], "Tap a tag to read it.");
    }

    #[test]
    fn tapping_a_row_shows_its_text_and_remembers_it_in_state() {
        let host = seeded(&[("rules", "Be kind to each other")]);
        let s = frame(&host, r#"{"action":"show:rules","state":""}"#);
        assert_eq!(s["state"], "rules");
        let drawn: Vec<&str> = s["ops"].as_array().unwrap().iter().filter_map(|o| o["s"].as_str()).collect();
        assert!(drawn.contains(&"Be kind to each other"), "{drawn:?}");
        assert!(s["status"].as_str().unwrap().contains("only shows a tag"));
    }

    #[test]
    fn refresh_keeps_the_open_tag_and_picks_up_changes() {
        let host = seeded(&[("rules", "v1")]);
        Tags::new(&host).edit("rules", "v2").unwrap();
        let s = frame(&host, r#"{"action":"refresh","state":"rules"}"#);
        let drawn: Vec<&str> = s["ops"].as_array().unwrap().iter().filter_map(|o| o["s"].as_str()).collect();
        assert!(drawn.contains(&"v2"));
    }

    #[test]
    fn a_tag_removed_while_open_is_reported_gone() {
        let s = frame(&seeded(&[("a", "1")]), r#"{"action":"refresh","state":"b"}"#);
        assert_eq!(s["status"], "b is gone.");
        assert_eq!(s["state"], "");
    }

    #[test]
    fn an_empty_space_says_how_to_add_one() {
        let s = frame(&FakeHost::default(), "");
        assert_eq!(s["status"], "No tags yet.");
        assert!(taps(&s).is_empty());
    }

    #[test]
    fn a_denied_store_becomes_a_frame_with_the_reason() {
        let host = FakeHost::default();
        host.kv_off.set(true);
        let s = frame(&host, "");
        assert_eq!(s["status"], "An admin needs to allow Tags to store data in the Dock.");
    }

    #[test]
    fn garbage_input_is_an_error_not_a_trap() {
        assert!(apply(&FakeHost::default(), "{nope").is_err());
    }

    #[test]
    fn the_board_reads_the_index_and_one_tag_only() {
        let host = seeded(&[("a", "1")]);
        host.next_run();
        frame(&host, r#"{"action":"show:a","state":""}"#);
        assert_eq!(host.calls.get(), 2);
    }

    #[test]
    fn a_full_board_with_the_longest_text_keeps_a_sane_shape() {
        let host = FakeHost::default();
        for i in 0..MAX_TAGS {
            Tags::new(&host).add(&format!("tag-{i:02}"), &"word ".repeat(crate::store::MAX_CHARS / 5)).unwrap();
            host.next_run();
        }
        let raw = apply(&host, r#"{"action":"show:tag-07","state":""}"#).unwrap();
        assert!(raw.len() < 32 * 1024, "{}", raw.len());
        let s: Value = serde_json::from_str(&raw).unwrap();
        assert!(s["height"].as_f64().unwrap() / WIDTH < 5.0);
    }

    #[test]
    fn wrap_breaks_on_words_and_splits_overlong_words() {
        assert_eq!(wrap("aa bb cc", 5), ["aa bb", "cc"]);
        assert_eq!(wrap("abcdefg hi", 3), ["abc", "def", "g", "hi"]);
        assert!(wrap("   ", 5).is_empty());
    }
}
