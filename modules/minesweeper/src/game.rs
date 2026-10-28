//! Minesweeper as a scene loop. Every frame is a fresh sandboxed call: the board rides in the
//! scene's opaque `state`, a tap arrives as an action, and the module returns the next frame.

use serde::Deserialize;

use crate::board::Board;
use crate::level::{self, Level, BEGINNER};
use crate::rng::SplitMix64;
use crate::scene::render;

pub fn apply(command: &str, input: &str, caller: &str, now_ms: i64) -> Result<String, String> {
    match command {
        "play" => Ok(play(input, caller, now_ms)),
        other => Err(format!("unknown command: {other}")),
    }
}

#[derive(Deserialize)]
struct Action {
    action: String,
    #[serde(default)]
    state: String,
}

/// A seed for a board nobody has played yet, folded from the caller's id (and, for a new game,
/// the previous seed) so two people launching do not share a board.
fn seed_from(input: &str) -> u64 {
    SplitMix64::from_bytes(input.as_bytes()).next_u64()
}

fn play(input: &str, caller: &str, now_ms: i64) -> String {
    let trimmed = input.trim();
    // The launch runs with empty input: start a fresh board.
    if trimmed.is_empty() {
        return render(&Board::new(seed_from(caller), BEGINNER), now_ms);
    }
    let Ok(act) = serde_json::from_str::<Action>(trimmed) else {
        return render(&Board::new(seed_from(&format!("{caller}|{trimmed}")), BEGINNER), now_ms);
    };
    let mut board = Board::from_state(&act.state);
    let seed = || seed_from(&format!("{}|{caller}", board.seed));

    match act.action.as_str() {
        "new game" | "new" => board = Board::new(seed(), board.level),
        name if Level::from_preset(name).is_some() => {
            board = Board::new(seed(), Level::from_preset(name).unwrap_or(BEGINNER));
        }
        "custom" => board.customizing = !board.customizing,
        typed if typed.starts_with("custom:") => match level::parse_custom(&typed["custom:".len()..]) {
            Ok(level) => board = Board::new(seed(), level),
            Err(_) => board.customizing = true,
        },
        name if name.starts_with("flag") || name.starts_with("dig") => board.flagging = !board.flagging,
        tap => match Press::read(tap, board.level) {
            Some(Press::Dig(index)) => {
                board.prepare_opening(index);
                board.tap(index, now_ms);
            }
            Some(Press::Flag(index)) => board.toggle_flag(index),
            None => {}
        },
    }
    render(&board, now_ms)
}

/// A press on a cell: a tap digs, a right click or long press flags.
enum Press {
    Dig(usize),
    Flag(usize),
}

impl Press {
    /// Reads `c:row,col` and `f:row,col`, and the `c12` form earlier boards sent for a dig.
    fn read(action: &str, level: Level) -> Option<Press> {
        let (kind, rest) = action.split_at_checked(1)?;
        let index = match rest.strip_prefix(':') {
            Some(pair) => {
                let (row, col) = pair.split_once(',')?;
                let (row, col) = (row.parse::<usize>().ok()?, col.parse::<usize>().ok()?);
                (row < level.rows && col < level.cols).then(|| row * level.cols + col)?
            }
            None => rest.parse::<usize>().ok().filter(|i| *i < level.cells())?,
        };
        match kind {
            "c" => Some(Press::Dig(index)),
            "f" if rest.starts_with(':') => Some(Press::Flag(index)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn launch(caller: &str) -> Value {
        serde_json::from_str(&apply("play", "", caller, 1_000).unwrap()).unwrap()
    }

    fn press(scene: &Value, action: &str, caller: &str, now_ms: i64) -> Value {
        let input = json!({"action": action, "state": scene["state"]}).to_string();
        serde_json::from_str(&apply("play", &input, caller, now_ms).unwrap()).unwrap()
    }

    fn cells_data(scene: &Value) -> String {
        scene["ops"][0]["data"].as_str().unwrap().to_owned()
    }

    #[test]
    fn play_launches_a_beginner_board_with_every_cell_hidden() {
        let scene = launch("alice");
        assert_eq!(scene["$slim"], "scene/1");
        assert_eq!(scene["status"], "10 mines left");
        assert_eq!(cells_data(&scene), "0".repeat(81));
        assert_eq!(scene["ops"][0]["tap"], "c");
    }

    #[test]
    fn the_levels_switch_the_board_in_place() {
        let scene = launch("alice");
        for (name, cells, mines) in [("intermediate", 256, 40), ("expert", 480, 99), ("beginner", 81, 10)] {
            let next = press(&scene, name, "alice", 2_000);
            assert_eq!(cells_data(&next).len(), cells, "{name}");
            assert_eq!(next["status"], format!("{mines} mines left"), "{name}");
        }
    }

    #[test]
    fn a_new_game_keeps_the_level_and_deals_a_different_field() {
        let expert = press(&launch("alice"), "expert", "alice", 1);
        let again = press(&expert, "new game", "alice", 2);
        assert_eq!(cells_data(&again).len(), 480);
        assert_ne!(again["state"], expert["state"]);
    }

    #[test]
    fn custom_asks_for_a_size_and_a_typed_size_deals_that_board() {
        let asked = press(&launch("alice"), "custom", "alice", 1);
        assert!(asked["ops"].as_array().unwrap().iter().any(|o| o["op"] == "input"));
        let dealt = press(&asked, "custom:20x12 30", "alice", 2);
        assert_eq!(cells_data(&dealt).len(), 240);
        assert_eq!(dealt["status"], "30 mines left");
        assert!(!dealt["ops"].as_array().unwrap().iter().any(|o| o["op"] == "input"));
    }

    #[test]
    fn a_size_that_cannot_be_dealt_keeps_the_board_and_the_field() {
        let asked = press(&launch("alice"), "custom", "alice", 1);
        let refused = press(&asked, "custom:99x99 5", "alice", 2);
        assert_eq!(cells_data(&refused).len(), 81);
        assert!(refused["ops"].as_array().unwrap().iter().any(|o| o["op"] == "input"));
    }

    #[test]
    fn a_right_click_flags_without_leaving_dig_mode() {
        let scene = launch("alice");
        let flagged = press(&scene, "f:2,3", "alice", 5);
        assert_eq!(flagged["status"], "9 mines left", "flagging alone does not start the clock");
        assert_eq!(flagged["controls"][0], "flag mode");
        let again = press(&flagged, "f:2,3", "alice", 6);
        assert_eq!(again["status"], "10 mines left");
    }

    #[test]
    fn a_first_tap_is_safe_starts_the_clock_and_a_later_one_reads_it() {
        let scene = launch("alice");
        let opened = press(&scene, "c:4,4", "alice", 10_000);
        assert!(!opened["status"].as_str().unwrap().contains("boom"));
        assert_eq!(opened["status"], "10 mines left - 0:00");
        let hidden = cells_data(&opened).find('0').unwrap();
        let later = press(&opened, &format!("f:{},{}", hidden / 9, hidden % 9), "alice", 75_000);
        assert_eq!(later["status"], "9 mines left - 1:05");
    }

    #[test]
    fn the_old_cell_action_still_reveals() {
        let opened = press(&launch("alice"), "c40", "alice", 1);
        assert!(cells_data(&opened).contains('1'));
    }

    #[test]
    fn nonsense_actions_change_nothing() {
        let scene = launch("alice");
        for action in ["c:99,99", "f:9,0", "c:x,y", "c", "f", "f12", "zzz", ""] {
            let next = press(&scene, action, "alice", 3);
            assert_eq!(cells_data(&next), cells_data(&scene), "{action:?}");
        }
    }

    #[test]
    fn a_state_from_the_old_release_keeps_playing() {
        let old = format!("{}|{}|{}|0|0", 4242, "0".repeat(81), "0".repeat(81));
        let input = json!({"action": "c:4,4", "state": old}).to_string();
        let next: Value = serde_json::from_str(&apply("play", &input, "alice", 1).unwrap()).unwrap();
        assert!(cells_data(&next).contains('1'));
    }

    #[test]
    fn a_win_through_the_actions_shows_the_card() {
        let scene = launch("alice");
        let mut state = Board::from_state(scene["state"].as_str().unwrap());
        state.prepare_opening(40);
        let mines = state.mines();
        let mut scene = scene;
        scene["state"] = json!(state.state());
        let mut frame = press(&scene, "c:4,4", "alice", 1_000);
        for index in (0..81).filter(|i| !mines[*i]) {
            frame = press(&frame, &format!("c:{},{}", index / 9, index % 9), "alice", 91_000);
        }
        assert_eq!(frame["status"], "cleared in 1:30");
        assert!(frame["ops"].as_array().unwrap().iter().any(|o| o["s"] == "You win!"));
    }

    #[test]
    fn two_callers_launching_get_different_boards_and_unknown_commands_fail() {
        assert_ne!(launch("alice")["state"], launch("bob")["state"]);
        assert!(apply("nope", "", "", 0).is_err());
    }
}
