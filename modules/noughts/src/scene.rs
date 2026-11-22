//! The `scene/1` a call's stage paints from the board (slim-m decision 0132).
//! It is drawn from the state alone, so everyone in the call gets the same one.

use serde_json::{json, Value};

use crate::game::Game;

const CELL: f64 = 30.0;
/// The controls under the board; each sends its own name as the next input.
pub const NEW_GAME: &str = "new game";
pub const END_GAME: &str = "end game";

/// The board as a scene: a tap on an empty square sends that square's index.
pub fn render(game: &Game) -> Value {
    let mut ops: Vec<Value> = Vec::new();
    for k in 1..3 {
        let p = f64::from(k) * CELL;
        ops.push(json!({"op":"line","x1":p,"y1":4.0,"x2":p,"y2":86.0,"stroke":"border","sw":1.0}));
        ops.push(json!({"op":"line","x1":4.0,"y1":p,"x2":86.0,"y2":p,"stroke":"border","sw":1.0}));
    }
    let open = game.winner.is_none();
    for (i, mark) in game.board.bytes().enumerate() {
        let (x0, y0) = ((i % 3) as f64 * CELL, (i / 3) as f64 * CELL);
        match mark {
            b'x' => {
                ops.push(json!({"op":"line","x1":x0+8.0,"y1":y0+8.0,"x2":x0+22.0,"y2":y0+22.0,"stroke":"accent","sw":2.5}));
                ops.push(json!({"op":"line","x1":x0+22.0,"y1":y0+8.0,"x2":x0+8.0,"y2":y0+22.0,"stroke":"accent","sw":2.5}));
            }
            b'o' => {
                ops.push(json!({"op":"circle","cx":x0+15.0,"cy":y0+15.0,"r":7.5,"stroke":"text","sw":2.5}));
            }
            _ if open => {
                ops.push(json!({"op":"rect","x":x0+3.0,"y":y0+3.0,"w":24.0,"h":24.0,"fill":"sunken","r":3.0,"tap":i.to_string()}));
            }
            _ => {}
        }
    }
    json!({
        "$slim": "scene/1",
        "width": 90.0,
        "height": 90.0,
        "background": "surface",
        "ops": ops,
        "status": status(game),
        "controls": [NEW_GAME, END_GAME],
    })
}

fn status(game: &Game) -> String {
    match game.winner.as_deref() {
        Some("draw") => "Draw".into(),
        Some(mark) => format!("{} wins", mark.to_uppercase()),
        None if game.x.is_none() => "X to move: tap a square to play".into(),
        None if game.o.is_none() => "O to move: tap a square to join".into(),
        None => format!("{} to move", game.next.to_uppercase()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn taps(scene: &Value) -> Vec<String> {
        scene["ops"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|op| op["tap"].as_str().map(str::to_owned))
            .collect()
    }

    #[test]
    fn a_fresh_board_offers_nine_squares() {
        let scene = render(&Game::default());
        assert_eq!(scene["$slim"], "scene/1");
        assert_eq!(taps(&scene).len(), 9);
        assert_eq!(scene["status"], "X to move: tap a square to play");
        assert_eq!(scene["controls"], json!([NEW_GAME, END_GAME]));
    }

    #[test]
    fn a_taken_square_is_drawn_and_no_longer_tappable() {
        let game = Game {
            board: "x...o....".into(),
            x: Some("aa".into()),
            o: Some("bb".into()),
            ..Game::default()
        };
        let scene = render(&game);
        let open = taps(&scene);
        assert!(!open.contains(&"0".to_owned()) && !open.contains(&"4".to_owned()));
        assert_eq!(open.len(), 7);
        assert_eq!(scene["status"], "X to move");
    }

    #[test]
    fn a_finished_game_offers_no_square() {
        let game = Game {
            board: "xxxoo....".into(),
            winner: Some("x".into()),
            ..Game::default()
        };
        let scene = render(&game);
        assert!(taps(&scene).is_empty());
        assert_eq!(scene["status"], "X wins");
    }
}
