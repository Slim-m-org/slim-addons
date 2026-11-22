//! The rules: two players, seated by who moves first, take turns on a 3x3
//! board. Nothing here knows about rooms; `room.rs` carries the board in and out.

use serde::{Deserialize, Serialize};

const EMPTY: u8 = b'.';
const LINES: [[usize; 3]; 8] = [
    [0, 1, 2],
    [3, 4, 5],
    [6, 7, 8],
    [0, 3, 6],
    [1, 4, 7],
    [2, 5, 8],
    [0, 4, 8],
    [2, 4, 6],
];

/// The whole game, as it rides in the room's state. `x` and `o` are the
/// players' opaque caller ids; everyone in the call can read them.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Game {
    pub board: String,
    pub next: String,
    pub x: Option<String>,
    pub o: Option<String>,
    pub winner: Option<String>,
}

impl Default for Game {
    fn default() -> Self {
        Game {
            board: ".........".into(),
            next: "x".into(),
            x: None,
            o: None,
            winner: None,
        }
    }
}

/// What a move asks of the room.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Next(Game),
    End,
}

impl Game {
    /// A board this module did not write is not trusted; a fresh one replaces it.
    pub fn checked(self) -> Game {
        let board_ok = self.board.len() == 9 && self.board.bytes().all(|b| b"xo.".contains(&b));
        let next_ok = self.next == "x" || self.next == "o";
        if board_ok && next_ok {
            self
        } else {
            Game::default()
        }
    }

    fn seated(&self, caller: &str) -> bool {
        self.x.as_deref() == Some(caller) || self.o.as_deref() == Some(caller)
    }

    fn nobody_seated(&self) -> bool {
        self.x.is_none() && self.o.is_none()
    }
}

/// Applies one input from `caller`: a cell `0` to `8`, `new` or `end`.
pub fn apply(game: Game, input: &str, caller: &str) -> Result<Step, &'static str> {
    if caller.is_empty() {
        return Err("this server did not say who you are");
    }
    match input {
        "new" if game.seated(caller) || game.winner.is_some() || game.nobody_seated() => {
            Ok(Step::Next(Game::default()))
        }
        "new" => Err("only a player can start over mid-game"),
        "end" if game.seated(caller) || game.nobody_seated() => Ok(Step::End),
        "end" => Err("only a player can end the game"),
        cell => match cell.parse::<usize>() {
            Ok(index) if index < 9 => play(game, index, caller).map(Step::Next),
            _ => Err("send a cell from 0 to 8, new, or end"),
        },
    }
}

fn play(mut game: Game, index: usize, caller: &str) -> Result<Game, &'static str> {
    if game.winner.is_some() {
        return Err("the game is over; send new to play again");
    }
    let mark = game.next.clone();
    let (mine, theirs) = if mark == "x" {
        (game.x.clone(), game.o.clone())
    } else {
        (game.o.clone(), game.x.clone())
    };
    match mine.as_deref() {
        Some(id) if id == caller => {}
        Some(_) if theirs.as_deref() == Some(caller) => return Err("not your turn"),
        Some(_) => return Err("both sides are taken, you are watching"),
        None if theirs.as_deref() == Some(caller) => return Err("not your turn"),
        None => {}
    }
    let mut cells = game.board.clone().into_bytes();
    if cells[index] != EMPTY {
        return Err("that cell is taken");
    }
    cells[index] = mark.as_bytes()[0];
    if mine.is_none() {
        let seat = if mark == "x" { &mut game.x } else { &mut game.o };
        *seat = Some(caller.to_string());
    }
    game.winner = winner(&cells);
    game.board = String::from_utf8(cells).unwrap_or_default();
    game.next = if mark == "x" { "o".into() } else { "x".into() };
    Ok(game)
}

fn winner(cells: &[u8]) -> Option<String> {
    for line in LINES {
        let [a, b, c] = line.map(|i| cells[i]);
        if a != EMPTY && a == b && b == c {
            return Some((a as char).to_string());
        }
    }
    (!cells.contains(&EMPTY)).then(|| "draw".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moves(steps: &[(&str, &str)]) -> Game {
        steps.iter().fold(Game::default(), |game, (who, cell)| {
            match apply(game, cell, who).expect("a legal move") {
                Step::Next(next) => next,
                Step::End => panic!("the game ended"),
            }
        })
    }

    #[test]
    fn the_first_two_movers_take_the_sides() {
        let game = moves(&[("aa", "0"), ("bb", "4")]);
        assert_eq!(game.board, "x...o....");
        assert_eq!(game.x.as_deref(), Some("aa"));
        assert_eq!(game.o.as_deref(), Some("bb"));
        assert_eq!(game.next, "x");
    }

    #[test]
    fn a_move_out_of_turn_or_from_a_watcher_is_refused() {
        let game = moves(&[("aa", "0")]);
        assert_eq!(apply(game.clone(), "1", "aa"), Err("not your turn"));
        let game = moves(&[("aa", "0"), ("bb", "4")]);
        assert_eq!(apply(game.clone(), "1", "bb"), Err("not your turn"));
        assert_eq!(
            apply(game, "1", "cc"),
            Err("both sides are taken, you are watching")
        );
    }

    #[test]
    fn a_taken_cell_and_nonsense_are_refused() {
        let game = moves(&[("aa", "0")]);
        assert_eq!(apply(game.clone(), "0", "bb"), Err("that cell is taken"));
        assert_eq!(
            apply(game.clone(), "9", "bb"),
            Err("send a cell from 0 to 8, new, or end")
        );
        assert_eq!(apply(game, "0", ""), Err("this server did not say who you are"));
    }

    #[test]
    fn three_in_a_row_wins_and_stops_the_game() {
        let game = moves(&[("aa", "0"), ("bb", "3"), ("aa", "1"), ("bb", "4"), ("aa", "2")]);
        assert_eq!(game.winner.as_deref(), Some("x"));
        assert_eq!(
            apply(game.clone(), "5", "bb"),
            Err("the game is over; send new to play again")
        );
        assert_eq!(apply(game, "new", "cc"), Ok(Step::Next(Game::default())));
    }

    #[test]
    fn a_full_board_without_a_line_is_a_draw() {
        let game = moves(&[
            ("aa", "0"),
            ("bb", "1"),
            ("aa", "2"),
            ("bb", "4"),
            ("aa", "3"),
            ("bb", "5"),
            ("aa", "7"),
            ("bb", "6"),
            ("aa", "8"),
        ]);
        assert_eq!(game.winner.as_deref(), Some("draw"));
    }

    #[test]
    fn only_a_player_may_restart_or_end_a_game_in_progress() {
        let game = moves(&[("aa", "0"), ("bb", "4")]);
        assert_eq!(
            apply(game.clone(), "new", "cc"),
            Err("only a player can start over mid-game")
        );
        assert_eq!(
            apply(game.clone(), "end", "cc"),
            Err("only a player can end the game")
        );
        assert_eq!(apply(game, "end", "bb"), Ok(Step::End));
    }

    #[test]
    fn a_board_this_module_did_not_write_starts_over() {
        let forged = Game {
            board: "xxxxxxxxxx".into(),
            ..Game::default()
        };
        assert_eq!(forged.checked(), Game::default());
    }
}
