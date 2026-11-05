//! The shared frame everyone in the channel sees: the soil, the colonies, their mushrooms and
//! plans, the weather in motion, and a scoreboard.

use serde_json::{json, Value};

use crate::colony::colour;
use crate::game::{Fx, Game};
use crate::words::count;
use crate::world::{World, CLAY, LOAM, ROCK, SKY, VEIN, WATER};

/// The palette the soil grid indexes: ten soils, then three shades for each of the seven owners.
fn palette() -> Vec<String> {
    let mut p: Vec<String> = ["sunken", "#2a1f18", "#3a2a1e", "#4d3523", "#6a4934", "#5a5a60", "#264660", "#c9a646", "#1c1611", "#57402a"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    for slot in 1..=7u8 {
        let base = colour(slot);
        p.push(blend(base, "#2a1f18", 0.5));
        p.push(base.to_owned());
        p.push(blend(base, "#ffffff", 0.45));
    }
    p
}

/// `a` moved `t` of the way to `b`, both `#rrggbb`.
pub fn blend(a: &str, b: &str, t: f32) -> String {
    let (a, b) = (rgb(a), rgb(b));
    let mix = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    format!("#{:02x}{:02x}{:02x}", mix(a[0], b[0]), mix(a[1], b[1]), mix(a[2], b[2]))
}

pub fn rgb(hex: &str) -> [u8; 3] {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0);
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

/// The palette index of one cell, as the character a `cells` op reads.
fn glyph(world: &World, i: usize) -> char {
    let cell = world.cells[i];
    let index = if cell.owner() != 0 {
        let shade = if cell.anchor() || cell.mushroom() {
            2
        } else if cell.strength() >= 2 {
            1
        } else {
            0
        };
        10 + (cell.owner() as usize - 1) * 3 + shade
    } else if cell.dead() {
        8
    } else {
        match cell.terrain() {
            LOAM if i / world.w < SKY + 2 && cell.nutrient() >= 9 => 9,
            LOAM => 1 + (cell.nutrient() as usize * 3 / 16).min(2),
            CLAY => 4,
            ROCK => 5,
            WATER => 6,
            VEIN => 7,
            _ => 0,
        }
    };
    char::from(b'0' + index as u8)
}

/// Everything one frame needs beyond the world.
pub struct Frame<'a> {
    pub game: &'a Game,
    pub world: &'a World,
    pub status: String,
    pub notes: Vec<Value>,
    /// Seconds of song to sweep a playhead across, when the song was asked for.
    pub song_secs: Option<f32>,
    pub redraw_s: u32,
    pub tick_ms: i64,
}

pub struct Layout {
    pub u: f32,
    pub hud: f32,
    pub width: f32,
    pub height: f32,
}

/// The scoreboard sits under the soil, not beside it: a module cannot know how wide the viewer's
/// screen is, and a side panel's text is unreadable once a phone scales the whole scene down.
pub fn layout(world: &World) -> Layout {
    let u = world.w as f32 / 64.0;
    let hud = 5.0 * u;
    Layout { u, hud, width: world.w as f32, height: world.h as f32 + hud + BOARD_ROWS as f32 * ROW_H * u + u }
}

/// Rows of two colonies each: room for all six players, so the board never changes size as they join.
const BOARD_ROWS: usize = 3;
const ROW_H: f32 = 5.0;

pub fn render(f: &Frame) -> String {
    let (game, world) = (f.game, f.world);
    let l = layout(world);
    let mut ops = vec![json!({"op": "rect", "x": 0, "y": 0, "w": l.width, "h": l.height, "fill": "surface"})];
    hud(f, &l, &mut ops);
    let data: String = (0..world.cells.len()).map(|i| glyph(world, i)).collect();
    ops.push(json!({
        "op": "cells", "cols": world.w, "rows": world.h, "data": data, "palette": palette(),
        "x": 0, "y": l.hud, "w": world.w, "h": world.h, "tap": "lure", "alt": "fruit", "tap_batch": true
    }));
    overlays(game, world, &l, &mut ops);
    let mut sweeps = 1;
    if let Some(secs) = f.song_secs {
        let y = l.hud;
        ops.push(json!({"op": "line", "x1": 0, "y1": y, "x2": 0, "y2": y + world.h as f32, "stroke": "text", "sw": 0.4 * l.u,
            "sweep": {"dx": world.w, "secs": secs}}));
        sweeps += 1;
    }
    for fx in game.fx.iter().take(8 - sweeps) {
        effect(fx, world, &l, &mut ops);
    }
    scoreboard(game, &l, &mut ops);
    if game.over {
        verdict(game, world, &l, &mut ops);
    }
    if !f.notes.is_empty() {
        ops.push(json!({"op": "notes", "notes": f.notes}));
    }
    let mut stored = game.clone();
    stored.fx.clear();
    let controls: Vec<&str> = if game.over { vec!["new season", "song", "my colony"] } else { vec!["play", "join", "song", "my colony"] };
    let mut scene = json!({
        "$slim": "scene/1", "width": l.width, "height": l.height, "background": "surface",
        "ops": ops, "status": f.status, "controls": controls, "state": stored.encode(world), "live": true
    });
    if !game.over {
        scene["refresh_ms"] = json!(f.redraw_s * 1000);
    }
    scene.to_string()
}

fn hud(f: &Frame, l: &Layout, ops: &mut Vec<Value>) {
    let game = f.game;
    let u = l.u;
    let bar_w = game.w as f32;
    let done = game.tick as f32 / game.end.max(1) as f32;
    let per_redraw = (f.redraw_s as f32 * 1000.0 / f.tick_ms.max(1) as f32) / game.end.max(1) as f32;
    let ahead = if game.over { 0.0 } else { (bar_w * per_redraw).min(bar_w * (1.0 - done)) };
    ops.push(json!({"op": "rect", "x": 0, "y": 3.6 * u, "w": bar_w, "h": 0.7 * u, "fill": "sunken", "r": 0.35 * u}));
    ops.push(json!({"op": "rect", "x": 0, "y": 3.6 * u, "w": (bar_w * done).max(0.5 * u), "h": 0.7 * u, "fill": "accent", "r": 0.35 * u,
        "sweep": {"dw": ahead, "secs": f.redraw_s.max(1)}}));
    let left_ms = i64::from(game.ticks_left()) * f.tick_ms;
    let clock = format!("{}:{:02} left", left_ms / 60_000, left_ms / 1000 % 60);
    let title = if game.over { format!("Season {} is over", game.season) } else { format!("Season {} · {clock}", game.season) };
    ops.push(json!({"op": "text", "x": 0.2 * u, "y": 1.6 * u, "s": title, "fill": "text", "size": 2.2 * u}));
    let mut sky = Vec::new();
    if !game.rain.is_empty() {
        sky.push("raining".to_string());
    }
    if game.warm > 0 {
        sky.push("warm spell".to_string());
    }
    if let Some(wild) = game.c.iter().find(|c| c.is_wild()) {
        sky.push(format!("wild rot {}", count(wild.cells, "cell", "cells")));
    }
    sky.push(format!("{} fell", count(game.leaves, "leaf", "leaves")));
    ops.push(json!({"op": "text", "x": bar_w, "y": 1.6 * u, "s": sky.join(" · "), "fill": "muted", "size": 1.8 * u, "align": "right"}));
}

fn centre(world: &World, l: &Layout, i: usize) -> (f32, f32) {
    ((i % world.w) as f32 + 0.5, l.hud + (i / world.w) as f32 + 0.5)
}

fn overlays(game: &Game, world: &World, l: &Layout, ops: &mut Vec<Value>) {
    for c in game.c.iter().filter(|c| !c.is_wild()) {
        if c.waypoints.len() > 1 {
            let d: Vec<String> = c
                .waypoints
                .iter()
                .enumerate()
                .map(|(k, p)| {
                    let (x, y) = centre(world, l, *p as usize);
                    format!("{} {x:.1} {y:.1}", if k == 0 { "M" } else { "L" })
                })
                .collect();
            ops.push(json!({"op": "path", "d": d.join(" "), "stroke": colour(c.slot), "sw": 0.2}));
        }
        if let Some(last) = c.waypoints.last() {
            let (x, y) = centre(world, l, *last as usize);
            ops.push(json!({"op": "circle", "cx": x, "cy": y, "r": 0.6, "fill": "#120d0a", "stroke": colour(c.slot), "sw": 0.25}));
        }
    }
    for c in &game.c {
        let (x, y) = centre(world, l, c.heart as usize);
        ops.push(json!({"op": "circle", "cx": x, "cy": y, "r": 0.8, "fill": colour(c.slot), "stroke": "#ffffff", "sw": 0.25}));
    }
    for (i, cell) in world.cells.iter().enumerate() {
        if !cell.mushroom() {
            continue;
        }
        let (x, y) = centre(world, l, i);
        let tint = colour(cell.owner());
        ops.push(json!({"op": "rect", "x": x - 0.25, "y": y - 0.6, "w": 0.5, "h": 1.1, "fill": "#efe4cf"}));
        let d = format!("M {:.2} {:.2} Q {x:.2} {:.2} {:.2} {:.2} Z", x - 1.2, y - 0.4, y - 2.6, x + 1.2, y - 0.4);
        ops.push(json!({"op": "path", "d": d, "fill": tint, "stroke": "#fff6e0", "sw": 0.15}));
    }
}

/// One moving flourish: the thing that just happened, played once between frames.
fn effect(fx: &Fx, world: &World, l: &Layout, ops: &mut Vec<Value>) {
    match fx.k {
        'l' => {
            let x = fx.a as f32 + 0.5;
            ops.push(json!({"op": "circle", "cx": x, "cy": l.hud - 1.0, "r": 0.8, "fill": "#c98a2e",
                "sweep": {"dy": SKY as f32 + 2.0, "secs": 1.2}}));
        }
        'r' => {
            let x = fx.a as f32;
            ops.push(json!({"op": "line", "x1": x - 3.0, "y1": l.hud, "x2": x - 2.0, "y2": l.hud + 2.0, "stroke": "#7fb4e6", "sw": 0.3,
                "sweep": {"dy": 8.0, "dx": 1.0, "secs": 1.5}}));
        }
        's' => {
            let (fx0, fy0) = centre(world, l, fx.a as usize);
            let (tx, ty) = centre(world, l, fx.b as usize);
            ops.push(json!({"op": "circle", "cx": fx0, "cy": fy0, "r": 0.6, "fill": "#fff6e0",
                "sweep": {"dx": tx - fx0, "dy": ty - fy0, "secs": 1.0}}));
        }
        'f' | 'w' => {
            let (x, y) = centre(world, l, fx.a as usize);
            let stroke = if fx.k == 'w' { colour(7) } else { "#fff6e0" };
            ops.push(json!({"op": "circle", "cx": x, "cy": y, "r": 0.5, "stroke": stroke, "sw": 0.3,
                "sweep": {"dw": 3.5, "secs": 1.2}}));
        }
        _ => {}
    }
}

fn scoreboard(game: &Game, l: &Layout, ops: &mut Vec<Value>) {
    let u = l.u;
    let top = l.hud + game.h as f32 + 1.0 * u;
    let ranked = game.ranking();
    if ranked.is_empty() {
        ops.push(json!({"op": "text", "x": 0.4 * u, "y": top + 2.0 * u, "s": "No colonies yet. Press join to take root.", "fill": "muted", "size": 2.2 * u}));
        return;
    }
    let half = game.w as f32 / 2.0;
    for (rank, c) in ranked.into_iter().enumerate().take(BOARD_ROWS * 2) {
        let x = (rank % 2) as f32 * half + 0.4 * u;
        let y = top + (rank / 2) as f32 * ROW_H * u + 1.4 * u;
        ops.push(json!({"op": "rect", "x": x, "y": y - 1.0 * u, "w": 2.0 * u, "h": 2.0 * u, "fill": colour(c.slot), "r": 0.5 * u}));
        ops.push(json!({"op": "text", "x": x + 2.8 * u, "y": y, "s": format!("{}. {}", rank + 1, c.name), "fill": "text", "size": 2.2 * u}));
        let line = format!("{} · {}", count(c.cells, "cell", "cells"), count(c.mushrooms, "mushroom", "mushrooms"));
        ops.push(json!({"op": "text", "x": x + 2.8 * u, "y": y + 2.3 * u, "s": line, "fill": "muted", "size": 1.8 * u}));
        let full = half - 4.0 * u;
        let energy = full * c.energy.clamp(0, crate::colony::ENERGY_CAP) as f32 / crate::colony::ENERGY_CAP as f32;
        ops.push(json!({"op": "rect", "x": x + 2.8 * u, "y": y + 3.5 * u, "w": full, "h": 0.4 * u, "fill": "sunken"}));
        ops.push(json!({"op": "rect", "x": x + 2.8 * u, "y": y + 3.5 * u, "w": energy.max(0.2 * u), "h": 0.4 * u, "fill": colour(c.slot)}));
    }
}

fn verdict(game: &Game, world: &World, l: &Layout, ops: &mut Vec<Value>) {
    let (w, h) = (world.w as f32, world.h as f32);
    ops.push(json!({"op": "rect", "x": w * 0.15, "y": l.hud + h * 0.3, "w": w * 0.7, "h": h * 0.4, "fill": "surface", "stroke": "border", "sw": 0.3, "r": 1.5}));
    let headline = match game.ranking().first() {
        Some(c) => format!("{} wins season {}", c.name, game.season),
        None => format!("Season {} ends with no colonies", game.season),
    };
    ops.push(json!({"op": "text", "x": w / 2.0, "y": l.hud + h * 0.43, "s": headline, "fill": "text", "size": 2.6 * l.u, "align": "center"}));
    ops.push(json!({"op": "text", "x": w / 2.0, "y": l.hud + h * 0.57, "s": "Press new season to deal fresh soil.", "fill": "muted",
        "size": 1.8 * l.u, "align": "center"}));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_palette_has_a_shade_for_every_glyph() {
        assert_eq!(palette().len(), 10 + 7 * 3);
        assert_eq!(char::from(b'0' + 30), 'N');
    }

    #[test]
    fn blending_moves_between_colours() {
        assert_eq!(blend("#000000", "#ffffff", 0.5), "#808080");
        assert_eq!(blend("#102030", "#102030", 0.7), "#102030");
        assert_eq!(rgb("#e0a040"), [0xe0, 0xa0, 0x40]);
    }
}
