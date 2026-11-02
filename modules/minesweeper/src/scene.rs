//! Drawing a board as a scene. The grid is one `cells` op, so an expert board stays well under
//! the 64 KB a shared scene may be; the numbers and flags are drawn over it.

use serde_json::{json, Value};

use crate::board::{clock, Board};
use crate::level::Level;

pub const CELL: f64 = 22.0;
/// The gutter between cells as a share of a cell, which is how a `cells` op reads `gap`.
const GAP: f64 = 0.08;
const HIDDEN: char = '0';
const REVEALED: char = '1';
const HIDDEN_WHILE_FLAGGING: char = '2';
/// Only the names the renderer resolves exist; an unknown one silently falls back.
const PALETTE: [&str; 3] = ["border", "bg", "accent-soft"];

/// The count colours. Low counts read calm, high counts read loud.
fn count_colour(count: u8) -> &'static str {
    match count {
        1 => "accent",
        2 => "muted",
        3 => "danger",
        _ => "text",
    }
}

pub fn render(board: &Board, now_ms: i64, note: Option<&str>) -> String {
    let (cols, rows) = (board.level.cols, board.level.rows);
    let (width, mut height) = (cols as f64 * CELL, rows as f64 * CELL);
    let mines = board.mines();
    let over = board.over();
    let mut grid = String::with_capacity(cols * rows);
    let mut marks: Vec<Value> = Vec::new();

    for index in 0..cols * rows {
        let (x, y) = ((index % cols) as f64 * CELL, (index / cols) as f64 * CELL);
        let centre = (x + CELL / 2.0, y + CELL / 2.0);
        let shown = board.revealed[index] || (over && mines[index]);
        grid.push(match (shown, board.flagging) {
            (true, _) => REVEALED,
            (false, true) => HIDDEN_WHILE_FLAGGING,
            (false, false) => HIDDEN,
        });
        if shown && mines[index] {
            let fill = if board.revealed[index] && board.dead { "danger" } else { "muted" };
            marks.push(json!({"op": "circle", "cx": centre.0, "cy": centre.1, "r": CELL / 2.0 - 6.0, "fill": fill}));
        } else if shown {
            let count = board.adjacent(&mines, index);
            if count > 0 {
                marks.push(json!({
                    "op": "text", "x": centre.0, "y": centre.1, "s": count.to_string(),
                    "fill": count_colour(count), "align": "center",
                }));
            }
        } else if board.flagged[index] {
            marks.push(json!({"op": "circle", "cx": centre.0, "cy": centre.1, "r": CELL / 2.0 - 7.0, "fill": "danger"}));
        }
    }

    let mut cells = json!({
        "op": "cells", "cols": cols, "rows": rows, "data": grid, "palette": PALETTE, "gap": GAP,
    });
    if !over {
        cells["tap"] = json!("c");
        cells["alt"] = json!("f");
    }
    let mut ops = vec![cells];
    ops.extend(marks);

    if board.won() {
        ops.extend(win_card(board, now_ms, width, height));
    }
    if board.customizing && !over {
        ops.push(json!({
            "op": "input", "submit": "custom", "x": 0.0, "y": height + 4.0, "w": width.min(260.0),
            "placeholder": "columns rows mines, like 20x12 30", "max": 16,
        }));
        height += 32.0;
    }

    json!({
        "$slim": "scene/1",
        "width": width,
        "height": height,
        "background": "surface",
        "ops": ops,
        "status": note.map_or_else(|| status(board, now_ms), str::to_owned),
        "controls": controls(board),
        "state": board.state(),
        "live": true,
    })
    .to_string()
}

/// A card over the middle of the cleared board, with the time it took.
fn win_card(board: &Board, now_ms: i64, width: f64, height: f64) -> Vec<Value> {
    let time = board.elapsed_ms(now_ms).map_or_else(String::new, clock);
    let (w, h) = ((width * 0.7).min(220.0), 64.0);
    let (x, y) = ((width - w) / 2.0, (height - h) / 2.0);
    let name = board.level.preset_name().map_or_else(|| "custom board".to_owned(), str::to_owned);
    vec![
        json!({"op": "rect", "x": x, "y": y, "w": w, "h": h, "r": 6.0, "fill": "surface", "stroke": "accent", "sw": 2.0}),
        json!({"op": "text", "x": width / 2.0, "y": y + 20.0, "s": "You win!", "fill": "accent", "size": 16.0, "align": "center"}),
        json!({"op": "text", "x": width / 2.0, "y": y + 44.0, "s": format!("{time} on {name}"), "fill": "text", "align": "center"}),
    ]
}

fn status(board: &Board, now_ms: i64) -> String {
    let time = board.elapsed_ms(now_ms).map(clock);
    let with_time = |text: String| time.clone().map_or_else(|| text.clone(), |t| format!("{text} - {t}"));
    if board.dead {
        with_time("boom - new game to try again".to_owned())
    } else if board.won() {
        format!("cleared in {}", time.unwrap_or_default())
    } else if board.flagging {
        with_time(format!("flagging - {} to place", board.flags_left()))
    } else {
        with_time(format!("{} mines left", board.flags_left()))
    }
}

/// The flag control names the mode it will put you in, so the board does not depend on the
/// status line to say what a tap does.
fn controls(board: &Board) -> Vec<&'static str> {
    let mode = if board.flagging { "dig mode" } else { "flag mode" };
    let access = if board.open { "host only" } else { "everyone plays" };
    if board.over() {
        vec!["new game", "beginner", "intermediate", "expert", "custom", "scores"]
    } else {
        vec![mode, "new game", "beginner", "intermediate", "expert", "custom", "scores", access]
    }
}

/// The scoreboard for `level`: the fastest times with the names players gave, and a field to give one.
pub fn render_scores(board: &Board, level: Level, lines: &[(String, i64)], note: Option<&str>) -> String {
    const ROW: f64 = 20.0;
    let rows = lines.len().max(1) as f64;
    let (width, top) = (240.0, 30.0);
    let height = top + rows * ROW + 40.0;
    let title = format!("Best times - {}", level.preset_name().unwrap_or("beginner"));
    let mut ops = vec![json!({"op": "text", "x": width / 2.0, "y": 14.0, "s": title, "fill": "accent", "size": 13.0, "align": "center"})];
    if lines.is_empty() {
        ops.push(json!({"op": "text", "x": width / 2.0, "y": top + 10.0, "s": "no times yet", "fill": "muted", "align": "center"}));
    }
    for (i, (name, ms)) in lines.iter().enumerate() {
        let y = top + i as f64 * ROW + 10.0;
        ops.push(json!({"op": "text", "x": 12.0, "y": y, "s": format!("{}", i + 1), "fill": "muted", "align": "left"}));
        ops.push(json!({"op": "text", "x": 36.0, "y": y, "s": name, "fill": "text", "align": "left"}));
        ops.push(json!({"op": "text", "x": width - 12.0, "y": y, "s": clock(*ms), "fill": "text", "align": "right"}));
    }
    ops.push(json!({
        "op": "input", "submit": "name", "x": 12.0, "y": top + rows * ROW + 8.0, "w": width - 24.0,
        "placeholder": "your name for this board", "max": crate::scores::MAX_NAME,
    }));
    json!({
        "$slim": "scene/1", "width": width, "height": height, "background": "surface", "ops": ops,
        "status": note.unwrap_or("fastest times"),
        "controls": ["back", "beginner", "intermediate", "expert"],
        "state": board.state(),
        "live": true,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::{custom, BEGINNER, EXPERT, INTERMEDIATE};

    fn parsed(scene: &str) -> Value {
        serde_json::from_str(scene).unwrap()
    }

    fn op<'a>(scene: &'a Value, kind: &str) -> Option<&'a Value> {
        scene["ops"].as_array().unwrap().iter().find(|o| o["op"] == kind)
    }

    /// The bug that shipped in 0.1.0: a text op written under `text` instead of `s` is dropped by
    /// the renderer, and the board rendered blank.
    #[test]
    fn revealed_counts_are_emitted_where_the_renderer_reads_them() {
        let mut board = Board::new(2_024, BEGINNER);
        board.prepare_opening(40);
        board.tap(40, 1);
        let scene = parsed(&render(&board, 2, None));
        let texts: Vec<&Value> = scene["ops"].as_array().unwrap().iter().filter(|o| o["op"] == "text").collect();
        assert!(!texts.is_empty());
        assert!(texts.iter().all(|t| t["s"].is_string() && t.get("text").is_none()));
    }

    /// Only a few names resolve in the renderer; an unknown one silently falls back.
    #[test]
    fn every_colour_named_is_one_the_renderer_knows() {
        const KNOWN: [&str; 9] = ["bg", "surface", "sunken", "accent", "accent-soft", "muted", "text", "border", "danger"];
        let mut flagging = Board::new(31, BEGINNER);
        flagging.flagging = true;
        flagging.toggle_flag(0);
        let mut won = Board::new(4_242, BEGINNER);
        let mines = won.mines();
        for i in 0..81 {
            won.revealed[i] = !mines[i];
        }
        won.start_ms = Some(0);
        won.end_ms = Some(61_000);
        let mut dead = Board::new(5, INTERMEDIATE);
        dead.tap(dead.mines().iter().position(|m| *m).unwrap(), 9);
        for scene in [render(&flagging, 1, None), render(&won, 1, None), render(&dead, 1, None), render(&Board::new(7, EXPERT), 1, None)] {
            let scene = parsed(&scene);
            let palette = op(&scene, "cells").unwrap()["palette"].as_array().unwrap().clone();
            for name in palette.iter().filter_map(Value::as_str) {
                assert!(KNOWN.contains(&name), "palette colour {name:?} falls back silently");
            }
            for o in scene["ops"].as_array().unwrap() {
                for key in ["fill", "stroke"] {
                    if let Some(name) = o[key].as_str() {
                        assert!(name.starts_with('#') || KNOWN.contains(&name), "{key} {name:?} falls back silently");
                    }
                }
            }
        }
    }

    #[test]
    fn a_fresh_board_is_one_hidden_grid_that_answers_a_tap_and_a_right_click() {
        let scene = parsed(&render(&Board::new(3, BEGINNER), 0, None));
        let grid = op(&scene, "cells").unwrap();
        assert_eq!((grid["cols"].as_u64(), grid["rows"].as_u64()), (Some(9), Some(9)));
        assert_eq!(grid["data"].as_str().unwrap(), "0".repeat(81));
        assert_eq!((grid["tap"].as_str(), grid["alt"].as_str()), (Some("c"), Some("f")));
        assert_eq!(scene["status"], "10 mines left");
        assert_eq!(scene["controls"][0], "flag mode");
    }

    #[test]
    fn each_level_is_drawn_at_its_own_size() {
        for (level, w, h) in [(BEGINNER, 198.0, 198.0), (INTERMEDIATE, 352.0, 352.0), (EXPERT, 660.0, 352.0)] {
            let scene = parsed(&render(&Board::new(3, level), 0, None));
            assert_eq!((scene["width"].as_f64(), scene["height"].as_f64()), (Some(w), Some(h)), "{level:?}");
        }
    }

    #[test]
    fn the_worst_expert_frame_fits_a_shared_scene() {
        // Every safe cell open and numbered, a flag on every mine: as many ops as a frame can have.
        let mut board = Board::new(9, EXPERT);
        let mines = board.mines();
        for i in 0..board.level.cells() {
            board.revealed[i] = !mines[i];
            board.flagged[i] = mines[i];
        }
        let size = render(&board, 0, None).len();
        assert!(size < 48 * 1024, "{size} bytes leaves too little room under the 64 KB ceiling");
    }

    #[test]
    fn flag_mode_tints_the_hidden_cells_and_the_button_names_the_other_mode() {
        let mut board = Board::new(3, BEGINNER);
        board.flagging = true;
        let scene = parsed(&render(&board, 0, None));
        assert_eq!(op(&scene, "cells").unwrap()["data"].as_str().unwrap(), "2".repeat(81));
        assert_eq!(scene["controls"][0], "dig mode");
        assert!(scene["status"].as_str().unwrap().starts_with("flagging - 10 to place"));
    }

    #[test]
    fn flags_are_drawn_and_counted() {
        let mut board = Board::new(3, BEGINNER);
        board.toggle_flag(4);
        board.toggle_flag(5);
        let scene = parsed(&render(&board, 0, None));
        let circles = scene["ops"].as_array().unwrap().iter().filter(|o| o["op"] == "circle").count();
        assert_eq!(circles, 2);
        assert_eq!(scene["status"], "8 mines left");
    }

    #[test]
    fn the_status_shows_the_clock_once_it_runs_and_freezes_it_when_over() {
        let mut board = Board::new(3, BEGINNER);
        board.prepare_opening(40);
        board.tap(40, 100_000);
        assert_eq!(parsed(&render(&board, 165_000, None))["status"], "10 mines left - 1:05");
        let mine = board.mines().iter().position(|m| *m).unwrap();
        board.tap(mine, 130_000);
        let status = parsed(&render(&board, 900_000, None))["status"].as_str().unwrap().to_owned();
        assert_eq!(status, "boom - new game to try again - 0:30");
    }

    #[test]
    fn a_win_shows_a_card_with_the_final_time_and_offers_no_taps() {
        let mut board = Board::new(4_242, BEGINNER);
        let mines = board.mines();
        for i in 0..81 {
            board.revealed[i] = !mines[i];
        }
        board.start_ms = Some(1_000);
        board.end_ms = Some(84_000);
        let scene = parsed(&render(&board, 5_000_000, None));
        let words: Vec<&str> = scene["ops"].as_array().unwrap().iter().filter_map(|o| o["s"].as_str()).collect();
        assert!(words.contains(&"You win!"), "{words:?}");
        assert!(words.contains(&"1:23 on beginner"), "{words:?}");
        assert_eq!(scene["status"], "cleared in 1:23");
        assert!(op(&scene, "cells").unwrap().get("tap").is_none());
        assert!(!scene["controls"].as_array().unwrap().iter().any(|c| c == "flag mode"));
    }

    #[test]
    fn a_dead_board_invites_no_more_taps_and_shows_the_mines() {
        let mut board = Board::new(5, BEGINNER);
        board.tap(board.mines().iter().position(|m| *m).unwrap(), 1);
        let scene = parsed(&render(&board, 2, None));
        assert!(op(&scene, "cells").unwrap().get("tap").is_none());
        let circles = scene["ops"].as_array().unwrap().iter().filter(|o| o["op"] == "circle").count();
        assert_eq!(circles, 10);
    }

    #[test]
    fn the_custom_field_appears_under_the_board_only_when_asked_for() {
        let mut board = Board::new(3, custom(12, 8, 12).unwrap());
        assert!(op(&parsed(&render(&board, 0, None)), "input").is_none());
        board.customizing = true;
        let scene = parsed(&render(&board, 0, None));
        let input = op(&scene, "input").unwrap();
        assert_eq!(input["submit"], "custom");
        assert!(input["y"].as_f64().unwrap() >= scene["height"].as_f64().unwrap() - 32.0);
        assert!(scene["height"].as_f64().unwrap() > 8.0 * CELL);
    }
}
