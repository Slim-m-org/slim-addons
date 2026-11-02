//! The scenes: the landing everyone shares, a player's own board (private), and practice.

use serde_json::{json, Value};

use crate::game::{score, Game, Mark, LENGTH, TRIES};

/// Green and amber are hex rather than theme tokens because the palette has no pair for "right"
/// and "nearly", and a guessing game where those two are not instantly distinguishable is not
/// playable. Everything else in the scene stays on tokens.
const RIGHT: &str = "#4f9d69";
const MOVED: &str = "#e3b341";
const ABSENT: &str = "sunken";

fn colour(mark: Mark) -> &'static str {
    match mark {
        Mark::Right => RIGHT,
        Mark::Moved => MOVED,
        Mark::Absent => ABSENT,
    }
}

/// One row of the board: a tile and its letter per column.
fn row(ops: &mut Vec<Value>, index: usize, guess: Option<&str>, marks: &[Mark]) {
    let y = 4.0 + (index as f64) * 12.0;
    for col in 0..LENGTH {
        let x = 21.0 + (col as f64) * 12.0;
        let fill = marks.get(col).map_or(ABSENT, |m| colour(*m));
        ops.push(json!({"op": "rect", "x": x, "y": y, "w": 10.0, "h": 10.0, "r": 1.5, "fill": fill, "stroke": "border", "sw": 0.3}));
        if let Some(letter) = guess.and_then(|g| g.chars().nth(col)) {
            ops.push(json!({
                "op": "text", "x": x + 5.0, "y": y + 5.5, "s": letter.to_uppercase().to_string(),
                "fill": "text", "size": 6.0, "align": "center",
            }));
        }
    }
}

/// Where a board's guesses are typed, and what to call the person's own state.
pub struct Board<'a> {
    pub game: &'a Game,
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
    for i in 0..TRIES {
        match game.guesses.get(i) {
            Some(guess) => row(&mut ops, i, Some(guess), &score(guess, game.answer)),
            None => row(&mut ops, i, None, &[]),
        }
    }
    if !game.over() {
        ops.push(json!({
            "op": "input", "x": 21.0, "y": 80.0, "w": 58.0, "submit": board.submit,
            "placeholder": "type a word", "max": LENGTH,
        }));
    }
    json!({
        "$slim": "scene/1", "private": true, "width": 100, "height": 100, "background": "surface",
        "ops": ops, "state": board.state, "controls": board.controls, "status": board.status,
    })
    .to_string()
}

/// What everyone sees on the message: no letters, nothing that depends on who is looking.
pub fn render_landing(day: i64) -> String {
    json!({
        "$slim": "scene/1", "width": 100, "height": 100, "background": "surface",
        "ops": [
            {"op": "text", "x": 50.0, "y": 22.0, "s": "Word Guess", "fill": "accent", "size": 9.0, "align": "center"},
            {"op": "text", "x": 50.0, "y": 36.0, "s": format!("day {day}"), "fill": "muted", "size": 5.0, "align": "center"},
            {"op": "text", "x": 50.0, "y": 50.0, "s": "one word a day, the same for everyone", "fill": "text", "size": 4.0, "align": "center"},
            {"op": "text", "x": 50.0, "y": 58.0, "s": "your board is private to you", "fill": "text", "size": 4.0, "align": "center"},
            {"op": "input", "x": 21.0, "y": 80.0, "w": 58.0, "submit": "guess", "placeholder": "type today's guess", "max": LENGTH},
        ],
        "controls": ["play today", "practice"],
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
    format!("Word Guess day {day} {tries}/{TRIES}\n```\n{}\n```", rows.join("\n"))
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
        let board = Board { game: &game, submit: "guess", controls: vec!["close"], status: "1 of 6".into(), state: String::new() };
        let scene = parsed(&render_board(&board));
        assert_eq!(scene["private"], true);
        let letters: Vec<&str> = scene["ops"].as_array().unwrap().iter().filter_map(|o| o["s"].as_str()).collect();
        assert_eq!(letters, ["S", "L", "A", "T", "E"]);
        let input = scene["ops"].as_array().unwrap().iter().find(|o| o["op"] == "input").unwrap();
        assert_eq!(input["submit"], "guess");
    }

    #[test]
    fn a_finished_board_has_no_input() {
        let mut game = Game::new("crane");
        game.guess("crane");
        let board = Board { game: &game, submit: "guess", controls: vec![], status: String::new(), state: String::new() };
        assert!(!render_board(&board).contains(r#""op":"input""#));
    }

    #[test]
    fn the_landing_is_shared_and_carries_no_letters_of_anyones_board() {
        let scene = parsed(&render_landing(20_000));
        assert!(scene.get("private").is_none(), "the landing is what everyone sees");
        let texts: Vec<&str> = scene["ops"].as_array().unwrap().iter().filter_map(|o| o["s"].as_str()).collect();
        assert!(texts.contains(&"day 20000"));
        assert_eq!(scene["controls"], json!(["play today", "practice"]));
        assert_eq!(scene["ops"].as_array().unwrap().iter().filter(|o| o["op"] == "rect").count(), 0);
    }

    #[test]
    fn the_share_text_shows_tries_and_tiles_and_never_the_word() {
        let mut game = Game::new("crane");
        game.guess("slate");
        game.guess("crate");
        game.guess("crane");
        let text = share_text(20_000, &game);
        assert!(text.starts_with("Word Guess day 20000 3/6\n```\n"), "{text}");
        assert!(text.contains("..#.#\n###.#\n#####"), "{text}");
        assert!(!text.to_lowercase().contains("crane") && !text.to_lowercase().contains("slate"));
    }

    #[test]
    fn a_lost_game_shares_as_x() {
        let mut game = Game::new("crane");
        for w in ["aaaaa", "bbbbb", "ddddd", "eeeee", "fffff", "ggggg"] {
            game.guess(w);
        }
        assert!(share_text(5, &game).starts_with("Word Guess day 5 X/6"));
    }

    #[test]
    fn every_colour_is_one_the_renderer_knows_or_a_hex_literal() {
        const KNOWN: [&str; 9] = ["bg", "surface", "sunken", "accent", "accent-soft", "muted", "text", "border", "danger"];
        let mut game = Game::new("crane");
        game.guess("slate");
        let board = Board { game: &game, submit: "guess", controls: vec![], status: String::new(), state: String::new() };
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
