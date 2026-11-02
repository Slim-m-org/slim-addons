//! The scenes: the landing everyone shares, a player's own board (private), and practice.

use serde_json::{json, Value};

use crate::game::{score, Game, Mark, LENGTH, TRIES};

/// Green and amber are hex rather than theme tokens because the palette has no pair for "right"
/// and "nearly", and a guessing game where those two are not instantly distinguishable is not
/// playable. Everything else in the scene stays on tokens.
const RIGHT: &str = "#4f9d69";
const MOVED: &str = "#e3b341";
const ABSENT: &str = "sunken";
const EMPTY: &str = "surface";
/// Letter colours chosen for contrast on the hex tiles; a theme text token would flip with the theme and wash out.
const ON_RIGHT: &str = "#ffffff";
const ON_MOVED: &str = "#1f1f1f";

/// The epoch day Word Guess first shipped on (2026-11-01); it is day 1.
const LAUNCH_DAY: i64 = 20_758;

const SCENE_HEIGHT: f64 = 110.0;
const BOARD_TOP: f64 = 13.0;
const INPUT_Y: f64 = 89.0;

/// The number a day goes by on screen and in a share, counted from the launch.
pub fn game_day(epoch_day: i64) -> i64 {
    (epoch_day - LAUNCH_DAY + 1).max(1)
}

fn tile(mark: Option<Mark>) -> (&'static str, &'static str) {
    match mark {
        Some(Mark::Right) => (RIGHT, ON_RIGHT),
        Some(Mark::Moved) => (MOVED, ON_MOVED),
        Some(Mark::Absent) => (ABSENT, "text"),
        None => (EMPTY, "text"),
    }
}

/// One row of the board: a tile and its letter per column.
fn row(ops: &mut Vec<Value>, index: usize, guess: Option<&str>, marks: &[Mark]) {
    let y = BOARD_TOP + (index as f64) * 12.0;
    for col in 0..LENGTH {
        let x = 21.0 + (col as f64) * 12.0;
        let (fill, ink) = tile(marks.get(col).copied());
        let stroke = if marks.is_empty() { "muted" } else { fill };
        ops.push(json!({"op": "rect", "x": x, "y": y, "w": 10.0, "h": 10.0, "r": 1.5, "fill": fill, "stroke": stroke, "sw": 0.4}));
        if let Some(letter) = guess.and_then(|g| g.chars().nth(col)) {
            ops.push(json!({
                "op": "text", "x": x + 5.0, "y": y + 5.5, "s": letter.to_uppercase().to_string(),
                "fill": ink, "size": 6.0, "align": "center",
            }));
        }
    }
}

fn header(ops: &mut Vec<Value>, title: &str) {
    ops.push(json!({"op": "text", "x": 50.0, "y": 7.5, "s": title, "fill": "accent", "size": 5.0, "align": "center"}));
}

fn input(ops: &mut Vec<Value>, submit: &str) {
    ops.push(json!({
        "op": "input", "x": 21.0, "y": INPUT_Y, "w": 58.0, "submit": submit,
        "placeholder": "type a 5-letter word", "max": LENGTH,
    }));
}

/// Where a board's guesses are typed, and what to call the person's own state.
pub struct Board<'a> {
    pub game: &'a Game,
    /// The line above the tiles: "Word Guess - day 2", or "Word Guess - practice".
    pub title: String,
    /// The action name the input submits under: `guess` for the day's puzzle, `pguess` for practice.
    pub submit: &'a str,
    pub controls: Vec<&'static str>,
    pub status: String,
    /// What the module hands itself back; empty for the daily board, whose truth is stored.
    pub state: String,
}

/// A player's own board, marked private so the host shows it only to them and never stores or broadcasts it.
pub fn render_board(board: &Board) -> String {
    let game = board.game;
    let mut ops = Vec::new();
    header(&mut ops, &board.title);
    for i in 0..TRIES {
        match game.guesses.get(i) {
            Some(guess) => row(&mut ops, i, Some(guess), &score(guess, game.answer)),
            None => row(&mut ops, i, None, &[]),
        }
    }
    if !game.over() {
        input(&mut ops, board.submit);
    }
    json!({
        "$slim": "scene/1", "private": true, "width": 100, "height": SCENE_HEIGHT, "background": "surface",
        "ops": ops, "state": board.state, "controls": board.controls, "status": board.status,
    })
    .to_string()
}

/// What everyone sees on the message: an empty board to type into, nothing that depends on who is looking.
pub fn render_landing(day: i64) -> String {
    let mut ops = Vec::new();
    header(&mut ops, &format!("Word Guess - day {}", game_day(day)));
    for i in 0..TRIES {
        row(&mut ops, i, None, &[]);
    }
    input(&mut ops, "guess");
    json!({
        "$slim": "scene/1", "width": 100, "height": SCENE_HEIGHT, "background": "surface",
        "ops": ops,
        "controls": ["practice"],
        "status": "everyone plays their own board",
        "live": true,
    })
    .to_string()
}

/// The result as plain text a player can post: tries and tiles, never the word or the letters.
pub fn share_text(day: i64, game: &Game) -> String {
    let tries = if game.solved() { game.guesses.len().to_string() } else { "X".to_owned() };
    let rows: Vec<String> = game
        .guesses
        .iter()
        .map(|guess| {
            score(guess, game.answer)
                .into_iter()
                .map(|m| match m {
                    Mark::Right => '#',
                    Mark::Moved => '+',
                    Mark::Absent => '.',
                })
                .collect()
        })
        .collect();
    format!("Word Guess day {} {tries}/{TRIES}\n```\n{}\n```", game_day(day), rows.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(scene: &str) -> Value {
        serde_json::from_str(scene).unwrap()
    }

    #[test]
    fn a_board_is_private_draws_every_guess_and_offers_the_input_while_open() {
        let mut game = Game::new("crane");
        game.guess("slate");
        let board = Board { game: &game, title: String::new(), submit: "guess", controls: vec!["close"], status: "1 of 6".into(), state: String::new() };
        let scene = parsed(&render_board(&board));
        assert_eq!(scene["private"], true);
        let letters: Vec<&str> = scene["ops"].as_array().unwrap().iter().filter_map(|o| o["s"].as_str()).filter(|s| s.len() == 1).collect();
        assert_eq!(letters, ["S", "L", "A", "T", "E"]);
        let input = scene["ops"].as_array().unwrap().iter().find(|o| o["op"] == "input").unwrap();
        assert_eq!(input["submit"], "guess");
    }

    #[test]
    fn a_finished_board_has_no_input() {
        let mut game = Game::new("crane");
        game.guess("crane");
        let board = Board { game: &game, title: String::new(), submit: "guess", controls: vec![], status: String::new(), state: String::new() };
        assert!(!render_board(&board).contains(r#""op":"input""#));
    }

    #[test]
    fn the_landing_is_shared_and_carries_no_letters_of_anyones_board() {
        let scene = parsed(&render_landing(20_000));
        assert!(scene.get("private").is_none(), "the landing is what everyone sees");
        let texts: Vec<&str> = scene["ops"].as_array().unwrap().iter().filter_map(|o| o["s"].as_str()).collect();
        assert!(texts.contains(&"Word Guess - day 1"));
        assert_eq!(scene["controls"], json!(["practice"]));
        let ops = scene["ops"].as_array().unwrap();
        assert_eq!(ops.iter().filter(|o| o["op"] == "rect").count(), TRIES * LENGTH, "an empty board to type into");
        assert!(ops.iter().any(|o| o["op"] == "input" && o["submit"] == "guess"));
        assert!(texts.iter().all(|t| t.len() > 1), "no letters on the shared frame");
    }

    #[test]
    fn days_count_from_the_launch_day_and_never_below_one() {
        assert_eq!(game_day(LAUNCH_DAY), 1);
        assert_eq!(game_day(LAUNCH_DAY + 1), 2);
        assert_eq!(game_day(0), 1);
    }

    #[test]
    fn the_share_text_shows_tries_and_tiles_and_never_the_word() {
        let mut game = Game::new("crane");
        game.guess("slate");
        game.guess("crate");
        game.guess("crane");
        let text = share_text(LAUNCH_DAY + 2, &game);
        assert!(text.starts_with("Word Guess day 3 3/6\n```\n"), "{text}");
        assert!(text.contains("..#.#\n###.#\n#####"), "{text}");
        assert!(!text.to_lowercase().contains("crane") && !text.to_lowercase().contains("slate"));
    }

    #[test]
    fn a_lost_game_shares_as_x() {
        let mut game = Game::new("crane");
        for w in ["aaaaa", "bbbbb", "ddddd", "eeeee", "fffff", "ggggg"] {
            game.guess(w);
        }
        assert!(share_text(LAUNCH_DAY + 4, &game).starts_with("Word Guess day 5 X/6"));
    }

    #[test]
    fn every_colour_is_one_the_renderer_knows_or_a_hex_literal() {
        const KNOWN: [&str; 9] = ["bg", "surface", "sunken", "accent", "accent-soft", "muted", "text", "border", "danger"];
        let mut game = Game::new("crane");
        game.guess("slate");
        let board = Board { game: &game, title: String::new(), submit: "guess", controls: vec![], status: String::new(), state: String::new() };
        for scene in [render_board(&board), render_landing(1)] {
            for op in parsed(&scene)["ops"].as_array().unwrap() {
                for key in ["fill", "stroke"] {
                    if let Some(name) = op[key].as_str() {
                        assert!(name.starts_with('#') || KNOWN.contains(&name), "{name}");
                    }
                }
            }
        }
    }
}
