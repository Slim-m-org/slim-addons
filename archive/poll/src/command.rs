//! Poll: create a poll, tap a bar to vote, tap "close poll" to lock it.
//!
//! Every frame is a fresh sandboxed call. The poll and its voter rows ride in
//! the scene's opaque `state` (see `ledger`), and nothing is remembered on the
//! host between calls. The caller id the host sends is what makes a vote
//! belong to a person; the state is client-echoed, so it is not authoritative.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::ledger::{Cast, Poll};

const MIN_OPTIONS: usize = 2;
const MAX_OPTIONS: usize = 6;
const MAX_QUESTION: usize = 60;
const MAX_OPTION: usize = 14;
const QUESTION_SIZE: f64 = 6.0;
const QUESTION_LINE_CHARS: usize = 26;
const QUESTION_LEADING: f64 = 6.5;

pub fn apply(command: &str, input: &str, caller: &str) -> Result<String, String> {
    match command {
        "poll" => Ok(handle(input, caller)),
        other => Err(format!("unknown command: {other}")),
    }
}

fn handle(input: &str, caller: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return render_setup(None);
    }
    match serde_json::from_str::<Action>(trimmed) {
        Ok(action) => dispatch(action, caller),
        // A slash-command launch hands the typed text straight through, so treat it as a poll definition.
        Err(_) => match parse_setup(trimmed) {
            Ok(poll) => render_poll(&poll, None),
            Err(msg) => render_setup(Some(&msg)),
        },
    }
}

/// The client sends a control's label back verbatim as the action.
const CLOSE_CONTROL: &str = "close poll";

fn dispatch(action: Action, caller: &str) -> String {
    let existing = Poll::parse(&action.state);

    if let Some(rest) = action.action.strip_prefix("setup:") {
        return match parse_setup(rest) {
            Ok(poll) => render_poll(&poll, None),
            Err(msg) => render_setup(Some(&msg)),
        };
    }
    if action.action == "reset" {
        return render_setup(None);
    }
    if let Some(rest) = action.action.strip_prefix("vote:") {
        let Some(mut poll) = existing else {
            return render_setup(Some("start a poll first"));
        };
        let outcome = match rest.parse::<usize>() {
            Ok(option) => poll.cast(caller, option),
            Err(_) => Err(crate::ledger::Refusal::NoSuchOption),
        };
        let note = match outcome {
            Ok(Cast::Moved) => Some("a vote moved"),
            Ok(Cast::New | Cast::Unchanged) => None,
            Err(refusal) => Some(refusal.note()),
        };
        return render_poll(&poll, note);
    }
    if action.action == "close" || action.action == CLOSE_CONTROL {
        return match existing {
            Some(mut poll) => {
                poll.open = false;
                render_poll(&poll, None)
            }
            None => render_setup(None),
        };
    }
    // Unknown action: redraw whatever is currently there rather than guessing.
    match existing {
        Some(poll) => render_poll(&poll, None),
        None => render_setup(None),
    }
}

#[derive(Deserialize)]
struct Action {
    action: String,
    #[serde(default)]
    state: String,
}

/// Strips control characters, which also keeps text from colliding with the
/// state's own delimiters, and bounds it so a poll stays a small drawing.
fn sanitize(text: &str, max_chars: usize) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(max_chars)
        .collect()
}

/// Parses `"<question> | <opt a>, <opt b>, ..."` into a fresh, open poll.
fn parse_setup(text: &str) -> Result<Poll, String> {
    let (question_part, options_part) = text
        .split_once('|')
        .ok_or_else(|| "use: question? | option a, option b".to_string())?;

    let question = sanitize(question_part.trim(), MAX_QUESTION);
    if question.is_empty() {
        return Err("need a question before the |".to_string());
    }

    let options: Vec<String> = options_part
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| sanitize(s, MAX_OPTION))
        .filter(|label| !label.is_empty())
        .collect();

    if options.len() < MIN_OPTIONS {
        return Err(format!("need at least {MIN_OPTIONS} options after the |"));
    }
    if options.len() > MAX_OPTIONS {
        return Err(format!("use at most {MAX_OPTIONS} options"));
    }

    Ok(Poll::new(question, options))
}

/// Breaks `text` into lines of at most `width` characters, at spaces where it can.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        let word: Vec<char> = word.chars().collect();
        for piece in word.chunks(width) {
            let piece: String = piece.iter().collect();
            match lines.last_mut() {
                Some(last) if last.chars().count() + 1 + piece.chars().count() <= width => {
                    last.push(' ');
                    last.push_str(&piece);
                }
                _ => lines.push(piece),
            }
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn render_setup(refused: Option<&str>) -> String {
    let status = refused
        .map(str::to_string)
        .unwrap_or_else(|| "type a question and 2 to 6 options, separated by |".to_string());
    json!({
        "$slim": "scene/1",
        "width": 100,
        "height": 60,
        "background": "surface",
        "ops": [
            { "op": "text", "x": 50.0, "y": 14.0, "s": "new poll", "fill": "text", "align": "center", "size": 10.0 },
            { "op": "input", "x": 10.0, "y": 26.0, "w": 80.0, "submit": "setup", "placeholder": "question? | opt a, opt b", "max": 200 }
        ],
        "status": status,
        "controls": [],
        "state": "",
    })
    .to_string()
}

fn render_poll(poll: &Poll, note: Option<&str>) -> String {
    let total = poll.total();
    let lines = wrap(&poll.question, QUESTION_LINE_CHARS);
    let mut ops: Vec<Value> = lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            json!({
                "op": "text", "x": 50.0, "y": 7.0 + (i as f64) * QUESTION_LEADING, "s": line,
                "fill": "text", "align": "center", "size": QUESTION_SIZE
            })
        })
        .collect();
    let rows_top = 16.0 + (lines.len() - 1) as f64 * QUESTION_LEADING;

    for (i, label) in poll.options.iter().enumerate() {
        let y = rows_top + (i as f64) * 13.0;
        let votes = poll.tally(i);
        let pct = if total > 0 {
            votes as f64 / total as f64
        } else {
            0.0
        };
        let bar_w = 80.0 * pct;

        let mut track = json!({
            "op": "rect", "x": 10.0, "y": y, "w": 80.0, "h": 10.0, "r": 2.0, "fill": "sunken"
        });
        if poll.open {
            track["tap"] = json!(format!("vote:{i}"));
        }
        ops.push(track);

        if bar_w > 0.5 {
            ops.push(json!({
                "op": "rect", "x": 10.0, "y": y, "w": bar_w, "h": 10.0, "r": 2.0,
                "grad": { "from": "accent", "to": "accent-soft", "dir": "h" }
            }));
        }

        ops.push(json!({
            "op": "text", "x": 13.0, "y": y + 7.0, "s": label, "fill": "text", "align": "left", "size": 5.5
        }));
        let pct_label = format!("{} ({}%)", votes, (pct * 100.0).round() as i64);
        ops.push(json!({
            "op": "text", "x": 87.0, "y": y + 7.0, "s": pct_label, "fill": "text", "align": "right", "size": 5.5
        }));
    }

    let status = match note {
        Some(msg) => msg.to_string(),
        None if poll.open => {
            format!("{total} vote(s) - tap a bar to vote, tap another to change it")
        }
        None => format!("final: {total} vote(s)"),
    };

    let controls: Vec<&str> = if poll.open {
        vec![CLOSE_CONTROL, "reset"]
    } else {
        vec!["reset"]
    };
    let height = rows_top + (poll.options.len() as f64) * 13.0 + 6.0;

    json!({
        "$slim": "scene/1",
        "width": 100,
        "height": height,
        "background": "surface",
        "ops": ops,
        "status": status,
        "controls": controls,
        "state": poll.state(),
        "live": false,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "aa";
    const B: &str = "bb";

    fn scene(out: &str) -> Value {
        serde_json::from_str(out).unwrap()
    }

    fn act(action: &str, state: &str, caller: &str) -> Value {
        let input = json!({ "action": action, "state": state }).to_string();
        scene(&apply("poll", &input, caller).unwrap())
    }

    fn started() -> String {
        let out = scene(&apply("poll", "q? | x, y, z", A).unwrap());
        out["state"].as_str().unwrap().to_string()
    }

    fn counts(state: &str) -> Vec<usize> {
        let poll = Poll::parse(state).unwrap();
        (0..poll.options.len()).map(|i| poll.tally(i)).collect()
    }

    fn next(out: &Value) -> String {
        out["state"].as_str().unwrap().to_string()
    }

    #[test]
    fn empty_input_is_the_setup_screen() {
        let out = apply("poll", "", "").unwrap();
        assert!(out.contains(r#""$slim":"scene/1""#));
        assert!(out.contains("new poll"));
    }

    #[test]
    fn a_slash_command_style_string_creates_a_poll_directly() {
        let out = apply("poll", "tabs or spaces? | tabs, spaces", A).unwrap();
        assert!(out.contains("tabs or spaces?"));
        assert!(out.contains(r#""controls":["close poll","reset"]"#));
    }

    #[test]
    fn setup_needs_a_pipe_and_between_two_and_six_options() {
        assert!(parse_setup("no pipe here").is_err());
        assert!(parse_setup("q? | onlyone").is_err());
        assert!(parse_setup("q? | a, b").is_ok());
        assert!(parse_setup("q? | a, b, c, d, e, f, g").is_err());
    }

    #[test]
    fn a_vote_shows_in_the_bar_and_the_state() {
        let out = act("vote:1", &started(), A);
        assert_eq!(counts(&next(&out)), vec![0, 1, 0]);
        assert!(out["status"].as_str().unwrap().starts_with("1 vote(s)"));
    }

    #[test]
    fn a_person_tapping_the_same_bar_repeatedly_counts_once() {
        let mut state = started();
        for _ in 0..5 {
            state = next(&act("vote:0", &state, A));
        }
        assert_eq!(counts(&state), vec![1, 0, 0]);
    }

    #[test]
    fn changing_a_vote_moves_it_and_says_so() {
        let state = next(&act("vote:0", &started(), A));
        let out = act("vote:2", &state, A);
        assert_eq!(counts(&next(&out)), vec![0, 0, 1]);
        assert_eq!(out["status"], "a vote moved");
    }

    #[test]
    fn a_second_person_adds_a_vote_and_a_third_option_is_counted() {
        let state = next(&act("vote:0", &started(), A));
        let state = next(&act("vote:0", &state, B));
        assert_eq!(counts(&state), vec![2, 0, 0]);
        let state = next(&act("vote:2", &state, "cc"));
        assert_eq!(counts(&state), vec![2, 0, 1]);
    }

    #[test]
    fn a_closed_poll_refuses_votes_and_keeps_the_count() {
        let state = next(&act("vote:0", &started(), A));
        let closed = next(&act("close", &state, A));
        let out = act("vote:1", &closed, B);
        assert_eq!(counts(&next(&out)), vec![1, 0, 0]);
        assert_eq!(out["status"], "poll is closed");
        assert!(!out.to_string().contains("vote:"));
    }

    #[test]
    fn a_caller_with_no_id_cannot_vote_and_the_poll_is_unchanged() {
        let start = started();
        let out = act("vote:0", &start, "");
        assert_eq!(counts(&next(&out)), vec![0, 0, 0]);
        assert_eq!(out["status"], "this server did not say who you are");
    }

    #[test]
    fn a_vote_with_no_poll_behind_it_asks_to_start_one() {
        let out = act("vote:0", "", A);
        assert_eq!(out["status"], "start a poll first");
    }

    #[test]
    fn reset_discards_the_poll_and_returns_to_setup() {
        let out = apply("poll", r#"{"action":"reset","state":""}"#, A).unwrap();
        assert!(out.contains("new poll"));
    }

    #[test]
    fn a_question_or_option_cannot_smuggle_the_state_delimiters() {
        let evil = format!("evil{}question{}{}?", '\u{1e}', '\u{1d}', '\u{1f}');
        let poll = parse_setup(&format!("{evil} | a, b")).unwrap();
        let reparsed = Poll::parse(&poll.state()).expect("state parses after a hostile question");
        assert_eq!(reparsed.options.len(), 2);
    }

    #[test]
    fn pressing_the_close_poll_control_closes_the_poll() {
        let first = act("setup:q? | a, b", "", A);
        let label = first["controls"][0].as_str().unwrap();
        let out = act(label, first["state"].as_str().unwrap(), A);
        assert!(
            out["status"].as_str().unwrap().starts_with("final:"),
            "poll still open after pressing {label:?}: {}",
            out["status"]
        );
    }

    #[test]
    fn every_control_a_scene_offers_changes_the_scene() {
        let open = act("setup:q? | a, b", "", A);
        let closed = act("close", open["state"].as_str().unwrap(), A);
        for scene in [open, closed] {
            for control in scene["controls"].as_array().unwrap() {
                let out = act(control.as_str().unwrap(), scene["state"].as_str().unwrap(), A);
                assert_ne!(out, scene, "control {control} was treated as an unknown action");
            }
        }
    }

    /// A generous per-character width in scene units for Plex-like text.
    fn extent(op: &Value) -> (f64, f64) {
        let width = op["s"].as_str().unwrap().chars().count() as f64 * 0.55 * op["size"].as_f64().unwrap();
        let x = op["x"].as_f64().unwrap();
        match op["align"].as_str().unwrap() {
            "center" => (x - width / 2.0, x + width / 2.0),
            "right" => (x - width, x),
            _ => (x, x + width),
        }
    }

    #[test]
    fn a_long_question_and_label_stay_inside_the_scene_and_clear_of_the_count() {
        let long_q = "Should we migrate the whole backend to the new framework now?";
        let setup = format!("setup:{long_q} | twenty chars long ab, twenty chars long cd");
        let voted = act(&setup, "", A)["state"].as_str().unwrap().to_owned();
        let out = act("vote:0", &voted, A);
        let texts: Vec<&Value> =
            out["ops"].as_array().unwrap().iter().filter(|op| op["op"] == "text").collect();
        for op in &texts {
            let (left, right) = extent(op);
            assert!(left >= 0.0 && right <= 100.0, "{} spans {left}..{right}", op["s"]);
        }
        for pair in texts[texts.len() - 4..].chunks(2) {
            let (_, label_end) = extent(pair[0]);
            let (count_start, _) = extent(pair[1]);
            assert!(label_end < count_start, "{} runs into {}", pair[0]["s"], pair[1]["s"]);
        }
    }

    #[test]
    fn the_whole_question_is_still_drawn_when_it_wraps() {
        let q = "Should we migrate the whole backend to the new framework now";
        let out = act(&format!("setup:{q} | a, b"), "", A);
        let drawn: Vec<&str> = out["ops"].as_array().unwrap().iter()
            .filter(|op| op["op"] == "text" && op["size"].as_f64() == Some(QUESTION_SIZE))
            .map(|op| op["s"].as_str().unwrap())
            .collect();
        assert!(drawn.len() > 1);
        assert_eq!(drawn.join(" "), q);
    }
}
