//! Minesweeper as a scene loop. Every frame is a fresh sandboxed call: the board rides in the
//! scene's opaque `state`, a tap arrives as an action, and the module returns the next frame.
//!
//! The board is shared by everyone who sees the message. Its host (whoever launched it) decides
//! whether anyone may dig and flag, and only the host reshapes a game in progress. A win is
//! replayed from the move log before it is written to the scoreboard.

use serde::Deserialize;

use crate::board::{Board, View};
use crate::host::Host;
use crate::level::{self, Level, BEGINNER};
use crate::rng::SplitMix64;
use crate::scene::{render, render_scores};
use crate::scores::{self, Scores};

pub fn apply<H: Host>(host: &H, command: &str, input: &str, caller: &str, now_ms: i64) -> Result<String, String> {
    match command {
        "play" => Ok(play(host, input, caller, now_ms)),
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

/// The host's id is kept in the state every viewer receives; it is an opaque per-module id, but it
/// still must not be able to break the state's own delimiters.
fn safe_id(caller: &str) -> String {
    caller.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(64).collect()
}

fn play<H: Host>(host: &H, input: &str, caller: &str, now_ms: i64) -> String {
    let caller = safe_id(caller);
    let trimmed = input.trim();
    // The launch runs with empty input: start a fresh board.
    if trimmed.is_empty() {
        return render(&Board::started_by(seed_from(&caller), BEGINNER, &caller), now_ms, None);
    }
    let Ok(act) = serde_json::from_str::<Action>(trimmed) else {
        return render(&Board::started_by(seed_from(&format!("{caller}|{trimmed}")), BEGINNER, &caller), now_ms, None);
    };
    let mut board = Board::from_state(&act.state);
    if let View::Scores(level) = board.view.clone() {
        return scores_view(host, &mut board, level, (&act.action, &caller), now_ms);
    }
    let mut note: Option<String> = None;
    let act_name = act.action.as_str();
    let can_reshape = board.is_host(&caller) || board.over();
    let seed = || seed_from(&format!("{}|{caller}", board.seed));

    match act_name {
        "scores" => {
            let level = board.level;
            return scores_view(host, &mut board, level, ("", &caller), now_ms);
        }
        "new game" | "new" if can_reshape => board = restart(seed(), board.level, &board, &caller),
        name if can_reshape && Level::from_preset(name).is_some() => {
            board = restart(seed(), Level::from_preset(name).unwrap_or(BEGINNER), &board, &caller);
        }
        "custom" if can_reshape => board.customizing = !board.customizing,
        typed if typed.starts_with("custom:") && can_reshape => match level::parse_custom(&typed["custom:".len()..]) {
            Ok(level) => board = restart(seed(), level, &board, &caller),
            Err(_) => board.customizing = true,
        },
        "host only" | "everyone plays" if board.is_host(&caller) => board.open = !board.open,
        name if name.starts_with("flag") || name.starts_with("dig") => board.flagging = !board.flagging,
        "new game" | "new" | "custom" | "host only" | "everyone plays" => note = Some(HOST_ONLY.to_owned()),
        name if Level::from_preset(name).is_some() || name.starts_with("custom:") => note = Some(HOST_ONLY.to_owned()),
        tap => match Press::read(tap, board.level) {
            Some(_) if !board.may_play(&caller) => note = Some("only the host can play this board".to_owned()),
            Some(Press::Dig(index)) => dig(host, &mut board, index, &caller, now_ms, &mut note),
            Some(Press::Flag(index)) => {
                board.note_player(&caller);
                board.toggle_flag(index);
            }
            None => {}
        },
    }
    render(&board, now_ms, note.as_deref())
}

const HOST_ONLY: &str = "only the host can change the game while it is on";

/// A new board for `level`, started by whoever pressed, keeping the access the host chose when the host did.
fn restart(seed: u64, level: Level, old: &Board, caller: &str) -> Board {
    let mut board = Board::started_by(seed, level, caller);
    if old.is_host(caller) {
        board.open = old.open;
    }
    board
}

fn dig<H: Host>(host: &H, board: &mut Board, index: usize, caller: &str, now_ms: i64, note: &mut Option<String>) {
    let was_over = board.over();
    board.prepare_opening(index);
    board.note_player(caller);
    board.tap(index, now_ms);
    if !was_over && board.won() && !board.scored {
        board.scored = true;
        *note = Some(record_win(host, board, now_ms));
    }
}

/// Writes the winning time for everyone who played, if the game replays to a win on a preset board.
fn record_win<H: Host>(host: &H, board: &Board, now_ms: i64) -> String {
    let (Some(level), Some(ms)) = (board.level.preset_name(), board.elapsed_ms(now_ms)) else {
        return format!("cleared in {}", board.elapsed_ms(now_ms).map_or_else(String::new, crate::board::clock));
    };
    let time = crate::board::clock(ms);
    if !board.replays_to_a_win() {
        return format!("cleared in {time} - not scored, the game could not be replayed");
    }
    let scores = Scores::new(host);
    let mut better = 0;
    for player in &board.players {
        match scores.record(level, player, ms, board.seed, now_ms) {
            Ok(true) => better += 1,
            Ok(false) => {}
            Err(why) => return format!("cleared in {time} - {why}"),
        }
    }
    match better {
        0 => format!("cleared in {time} - no one beat their best"),
        1 => format!("cleared in {time} - a new best for 1 player"),
        n => format!("cleared in {time} - new bests for {n} players"),
    }
}

/// The scoreboard view: pick a level, set a name, or go back to the board.
fn scores_view<H: Host>(host: &H, board: &mut Board, level: Level, (action, caller): (&str, &str), now_ms: i64) -> String {
    let scores = Scores::new(host);
    let mut note: Option<String> = None;
    let mut level = level;
    match action {
        "back" => {
            board.view = View::Board;
            return render(board, now_ms, None);
        }
        name if Level::from_preset(name).is_some() => level = Level::from_preset(name).unwrap_or(BEGINNER),
        typed if typed.starts_with("name:") => {
            note = Some(match scores::clean_name(&typed["name:".len()..]) {
                Some(name) => scores.set_name(caller, &name).map_or_else(|why| why, |()| format!("you are {name} on the board")),
                None => "type a name first".to_owned(),
            });
        }
        _ => {}
    }
    board.view = View::Scores(level);
    let code = level.preset_name().unwrap_or("beginner");
    let lines = match scores.top(code, scores::SHOWN) {
        Ok(entries) => {
            let ids: Vec<String> = entries.iter().map(|e| e.player.clone()).collect();
            let names = scores.names(&ids).unwrap_or_default();
            entries.into_iter().map(|e| (names.get(&e.player).cloned().unwrap_or_else(|| anonymous(&e.player)), e.ms)).collect()
        }
        Err(why) => {
            note = Some(why);
            Vec::new()
        }
    };
    render_scores(board, level, &lines, note.as_deref())
}

/// What a player who never chose a name is called: a few characters of their opaque id.
fn anonymous(player: &str) -> String {
    format!("player {}", player.chars().take(4).collect::<String>())
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
    use crate::fake::FakeHost;
    use serde_json::{json, Value};

    fn launch(caller: &str) -> Value {
        serde_json::from_str(&apply(&FakeHost::default(), "play", "", caller, 1_000).unwrap()).unwrap()
    }

    fn press(scene: &Value, action: &str, caller: &str, now_ms: i64) -> Value {
        let input = json!({"action": action, "state": scene["state"]}).to_string();
        serde_json::from_str(&apply(&FakeHost::default(), "play", &input, caller, now_ms).unwrap()).unwrap()
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
        let next: Value = serde_json::from_str(&apply(&FakeHost::default(), "play", &input, "alice", 1).unwrap()).unwrap();
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
        assert!(apply(&FakeHost::default(), "nope", "", "", 0).is_err());
    }
}

#[cfg(test)]
mod shared_tests {
    use super::*;
    use crate::board::near_win;
    use crate::fake::FakeHost;
    use crate::level::{custom, EXPERT};
    use serde_json::{json, Value};

    fn press(host: &FakeHost, scene_state: &str, action: &str, caller: &str, now_ms: i64) -> Value {
        let input = json!({"action": action, "state": scene_state}).to_string();
        serde_json::from_str(&apply(host, "play", &input, caller, now_ms).unwrap()).unwrap()
    }

    fn state(scene: &Value) -> String {
        scene["state"].as_str().unwrap().to_owned()
    }

    fn texts(scene: &Value) -> Vec<String> {
        scene["ops"].as_array().unwrap().iter().filter_map(|o| o["s"].as_str().map(str::to_owned)).collect()
    }

    fn launched_by(host: &FakeHost, caller: &str) -> Value {
        serde_json::from_str(&apply(host, "play", "", caller, 1_000).unwrap()).unwrap()
    }

    #[test]
    fn the_launcher_is_the_host_and_the_board_starts_open_to_everyone() {
        let host = FakeHost::default();
        let scene = launched_by(&host, "ann");
        let board = Board::from_state(&state(&scene));
        assert_eq!((board.host.as_str(), board.open), ("ann", true));
        let dug = press(&host, &state(&scene), "c:4,4", "bob", 5_000);
        assert!(dug["ops"][0]["data"].as_str().unwrap().contains('1'), "a second player digs on an open board");
        assert_eq!(Board::from_state(&state(&dug)).players, vec!["bob".to_owned()]);
    }

    #[test]
    fn a_host_only_board_refuses_everyone_else_and_says_so() {
        let host = FakeHost::default();
        let scene = launched_by(&host, "ann");
        let closed = press(&host, &state(&scene), "host only", "ann", 2_000);
        assert!(closed["controls"].as_array().unwrap().iter().any(|c| c == "everyone plays"));
        let refused = press(&host, &state(&closed), "c:4,4", "bob", 3_000);
        assert_eq!(refused["status"], "only the host can play this board");
        assert_eq!(refused["ops"][0]["data"], closed["ops"][0]["data"], "nothing opened");
        let flagged = press(&host, &state(&closed), "f:0,0", "bob", 3_000);
        assert_eq!(flagged["status"], "only the host can play this board");
        let hosted = press(&host, &state(&closed), "c:4,4", "ann", 3_000);
        assert!(hosted["ops"][0]["data"].as_str().unwrap().contains('1'));
        let reopened = press(&host, &state(&hosted), "everyone plays", "ann", 4_000);
        assert!(reopened["controls"].as_array().unwrap().iter().any(|c| c == "host only"));
    }

    #[test]
    fn only_the_host_changes_the_access_or_reshapes_a_game_in_progress() {
        let host = FakeHost::default();
        let scene = launched_by(&host, "ann");
        let started = press(&host, &state(&scene), "c:4,4", "ann", 2_000);
        for action in ["host only", "new game", "expert", "custom", "custom:20x12 30"] {
            let refused = press(&host, &state(&started), action, "bob", 3_000);
            assert_eq!(refused["status"], HOST_ONLY, "{action}");
            assert_eq!(state(&refused), state(&started), "{action} changed the game");
        }
        let fresh = press(&host, &state(&started), "expert", "ann", 3_000);
        assert_eq!(fresh["ops"][0]["data"].as_str().unwrap().len(), 480);
    }

    #[test]
    fn once_the_game_is_over_anyone_may_start_the_next() {
        let host = FakeHost::default();
        let scene = launched_by(&host, "ann");
        let mut board = Board::from_state(&state(&scene));
        let mine = board.mines().iter().position(|m| *m).unwrap();
        board.tap(mine, 2_000);
        assert!(board.dead);
        let next = press(&host, &board.state(), "new game", "bob", 3_000);
        let fresh = Board::from_state(&state(&next));
        assert_eq!(fresh.host, "bob", "whoever starts the next game hosts it");
        assert!(!fresh.over());
    }

    #[test]
    fn a_new_game_by_the_host_keeps_the_access_they_chose() {
        let host = FakeHost::default();
        let scene = launched_by(&host, "ann");
        let closed = press(&host, &state(&scene), "host only", "ann", 2_000);
        let again = press(&host, &state(&closed), "new game", "ann", 3_000);
        assert!(!Board::from_state(&state(&again)).open);
    }

    #[test]
    fn a_win_is_scored_for_everyone_who_played_and_only_once() {
        let host = FakeHost::default();
        let (mut before, last) = near_win(BEGINNER, "ann");
        before.note_player("bob");
        let (row, col) = (last / 9, last % 9);
        let won = press(&host, &before.state(), &format!("c:{row},{col}"), "bob", 91_000);
        assert!(won["status"].as_str().unwrap().starts_with("cleared in 1:30"), "{}", won["status"]);
        assert!(won["status"].as_str().unwrap().contains("new bests for 2 players"), "{}", won["status"]);
        assert_eq!(host.best_ms("beginner", "ann"), Some(90_000));
        assert_eq!(host.best_ms("beginner", "bob"), Some(90_000));

        let calls = host.calls.get();
        let again = press(&host, &state(&won), &format!("c:{row},{col}"), "bob", 99_000);
        assert_eq!(host.calls.get(), calls, "a frame after the win writes nothing");
        assert_eq!(again["status"], "cleared in 1:30");
    }

    #[test]
    fn a_slower_win_does_not_replace_a_better_time() {
        let host = FakeHost::default();
        Scores::new(&host).record("beginner", "ann", 20_000, 1, 1).unwrap();
        let (before, last) = near_win(BEGINNER, "ann");
        let won = press(&host, &before.state(), &format!("c:{},{}", last / 9, last % 9), "ann", 91_000);
        assert!(won["status"].as_str().unwrap().contains("no one beat their best"));
        assert_eq!(host.best_ms("beginner", "ann"), Some(20_000));
    }

    #[test]
    fn a_win_the_log_does_not_reproduce_is_not_scored() {
        let host = FakeHost::default();
        let (mut before, last) = near_win(BEGINNER, "ann");
        before.moves.clear();
        let won = press(&host, &before.state(), &format!("c:{},{}", last / 9, last % 9), "ann", 91_000);
        assert!(won["status"].as_str().unwrap().contains("not scored"), "{}", won["status"]);
        assert!(host.best.borrow().is_empty());
    }

    #[test]
    fn a_custom_board_is_never_scored() {
        let host = FakeHost::default();
        let (before, last) = near_win(custom(10, 8, 12).unwrap(), "ann");
        let won = press(&host, &before.state(), &format!("c:{},{}", last / 10, last % 10), "ann", 91_000);
        assert!(won["status"].as_str().unwrap().starts_with("cleared in"));
        assert!(host.best.borrow().is_empty());
        assert_eq!(host.calls.get(), 0);
    }

    #[test]
    fn a_host_that_refuses_the_scoreboard_still_shows_the_win() {
        let host = FakeHost::default();
        host.refuse.set(true);
        let (before, last) = near_win(BEGINNER, "ann");
        let won = press(&host, &before.state(), &format!("c:{},{}", last / 9, last % 9), "ann", 91_000);
        assert!(won["status"].as_str().unwrap().contains("An admin needs to approve"), "{}", won["status"]);
        assert!(won["ops"].as_array().unwrap().iter().any(|o| o["s"] == "You win!"));
    }

    #[test]
    fn a_win_with_the_most_players_stays_inside_the_call_budget() {
        let host = FakeHost::default();
        let (mut before, last) = near_win(EXPERT, "p0");
        for n in 1..20 {
            before.note_player(&format!("p{n}"));
        }
        let won = press(&host, &before.state(), &format!("c:{},{}", last / 30, last % 30), "p0", 600_000);
        assert!(won["status"].as_str().unwrap().contains("new bests for 8 players"), "{}", won["status"]);
        assert!(host.calls.get() <= 64);
    }

    #[test]
    fn the_scoreboard_lists_times_with_names_and_a_back_button() {
        let host = FakeHost::default();
        let scores = Scores::new(&host);
        scores.record("beginner", "ann0000", 70_000, 1, 1).unwrap();
        scores.record("beginner", "bob1111", 50_000, 1, 1).unwrap();
        scores.set_name("bob1111", "Bob").unwrap();
        let scene = launched_by(&host, "ann0000");
        let view = press(&host, &state(&scene), "scores", "ann0000", 2_000);
        let shown = texts(&view);
        let expected = ["Best times - beginner", "1", "Bob", "0:50", "2", "player ann0", "1:10"];
        for want in expected {
            assert!(shown.iter().any(|t| t == want), "{want} in {shown:?}");
        }
        assert_eq!(view["controls"], json!(["back", "beginner", "intermediate", "expert"]));
        let back = press(&host, &state(&view), "back", "ann0000", 3_000);
        assert_eq!(back["ops"][0]["op"], "cells");
        assert_eq!(Board::from_state(&state(&back)).view, View::Board);
    }

    #[test]
    fn the_scoreboard_switches_level_and_says_when_it_is_empty() {
        let host = FakeHost::default();
        Scores::new(&host).record("expert", "zed", 300_000, 1, 1).unwrap();
        let scene = launched_by(&host, "ann");
        let view = press(&host, &state(&scene), "scores", "ann", 2_000);
        assert!(texts(&view).contains(&"no times yet".to_owned()));
        let expert = press(&host, &state(&view), "expert", "ann", 3_000);
        assert!(texts(&expert).contains(&"Best times - expert".to_owned()));
        assert!(texts(&expert).contains(&"5:00".to_owned()));
        assert_eq!(Board::from_state(&state(&expert)).view, View::Scores(EXPERT));
    }

    #[test]
    fn a_player_sets_their_own_name_and_a_junk_one_is_refused() {
        let host = FakeHost::default();
        let scene = launched_by(&host, "ann");
        let view = press(&host, &state(&scene), "scores", "ann", 2_000);
        let named = press(&host, &state(&view), "name:  Ann\u{202E}ie ", "ann", 3_000);
        assert_eq!(named["status"], "you are Annie on the board");
        assert_eq!(host.name_of("ann").as_deref(), Some("Annie"));
        let blank = press(&host, &state(&named), "name:   ", "ann", 4_000);
        assert_eq!(blank["status"], "type a name first");
        assert_eq!(host.name_of("ann").as_deref(), Some("Annie"));
    }

    #[test]
    fn a_failing_scoreboard_reads_as_a_sentence_not_a_broken_view() {
        let host = FakeHost::default();
        host.refuse.set(true);
        let scene = launched_by(&host, "ann");
        let view = press(&host, &state(&scene), "scores", "ann", 2_000);
        assert!(view["status"].as_str().unwrap().contains("An admin needs to approve"));
        assert_eq!(view["$slim"], "scene/1");
    }

    #[test]
    fn a_caller_id_cannot_break_the_state_it_is_stored_in() {
        let host = FakeHost::default();
        let scene = launched_by(&host, "a|b,c\"d\u{0}");
        let board = Board::from_state(&state(&scene));
        assert_eq!(board.host, "abcd");
        assert_eq!(Board::from_state(&board.state()).host, "abcd");
    }
}
