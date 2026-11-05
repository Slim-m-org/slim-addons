//! "My colony": a private frame only its player sees, with their numbers, a rendered portrait
//! of their network, a name field and the moves that spend energy.

use serde_json::{json, Value};

use crate::b64;
use crate::colony::{colour, Colony, ENERGY_CAP, MAX_MUSHROOMS};
use crate::game::Game;
use crate::moves::{BURST_COST, FRUIT_COST, SPORE_COST};
use crate::png;
use crate::rng::{hash, mix};
use crate::scene::rgb;
use crate::words::count;
use crate::world::{World, CLAY, ROCK, VEIN, WATER};

/// Pixels across the portrait.
pub const PORTRAIT: usize = 96;
const WIDTH: f32 = 80.0;
const HEIGHT: f32 = 63.0;

pub struct Panel<'a> {
    pub game: &'a Game,
    pub world: &'a World,
    pub pid: &'a str,
    pub status: String,
    pub notes: Vec<Value>,
    /// The player's record across seasons, when the database has one.
    pub record: Option<Value>,
    pub link_code: Option<String>,
    pub state: String,
}

pub fn render(p: &Panel) -> String {
    let mut ops = vec![json!({"op": "rect", "x": 0, "y": 0, "w": WIDTH, "h": HEIGHT, "fill": "surface"})];
    let controls = match p.game.colony(p.pid) {
        Some(c) => {
            colony_ops(p, c, &mut ops);
            vec!["back", "burst", "spore shot", "link profile"]
        }
        None => {
            ops.push(json!({"op": "text", "x": 3, "y": 8, "s": "No colony this season", "fill": "text", "size": 4.2}));
            ops.push(json!({"op": "text", "x": 3, "y": 15, "s": "Press join to take root, then come back here.", "fill": "muted", "size": 2.4}));
            vec!["back", "join"]
        }
    };
    if !p.notes.is_empty() {
        ops.push(json!({"op": "notes", "notes": p.notes}));
    }
    json!({
        "$slim": "scene/1", "width": WIDTH, "height": HEIGHT, "background": "surface", "ops": ops,
        "status": p.status, "controls": controls, "state": p.state, "live": true, "private": true
    })
    .to_string()
}

fn colony_ops(p: &Panel, c: &Colony, ops: &mut Vec<Value>) {
    let game = p.game;
    let ranking = game.ranking();
    let rank = ranking.iter().position(|r| r.pid == c.pid).map_or(0, |k| k + 1);
    let tint = colour(c.slot);
    let text = |ops: &mut Vec<Value>, y: f32, s: String, fill: &str, size: f32| {
        ops.push(json!({"op": "text", "x": 3, "y": y, "s": s, "fill": fill, "size": size}));
    };
    text(ops, 6.0, c.name.clone(), tint, 4.4);
    text(ops, 11.0, format!("Season {} · rank {rank} of {}", game.season, ranking.len()), "muted", 2.4);
    ops.push(json!({"op": "rect", "x": 3, "y": 14, "w": 40, "h": 2.4, "fill": "sunken", "r": 1.2}));
    let filled = 40.0 * c.energy.clamp(0, ENERGY_CAP) as f32 / ENERGY_CAP as f32;
    ops.push(json!({"op": "rect", "x": 3, "y": 14, "w": filled.max(1.2), "h": 2.4, "fill": tint, "r": 1.2}));
    text(ops, 19.5, format!("Energy {} of {ENERGY_CAP}", c.energy), "text", 2.3);
    text(ops, 23.5, format!("{} now, {} at most", count(c.cells, "cell", "cells"), c.peak), "text", 2.3);
    text(ops, 27.5, format!("{} of {MAX_MUSHROOMS} mushrooms", c.mushrooms), "text", 2.3);
    text(ops, 31.5, format!("{} from mushrooms", count(c.score, "point", "points")), "text", 2.3);
    text(ops, 35.5, format!("Burst {BURST_COST} · spore {SPORE_COST} · fruit {FRUIT_COST}"), "muted", 2.1);
    ops.push(json!({"op": "rect", "x": 45.5, "y": 3.5, "w": 31, "h": 31, "fill": "#120d0a", "stroke": "border", "sw": 0.3, "r": 1}));
    if let Some(image) = portrait(p.world, c.slot) {
        ops.push(json!({"op": "image", "x": 46, "y": 4, "w": 30, "h": 30, "b64": image}));
    }
    ops.push(json!({"op": "text", "x": 61, "y": 37, "s": "your network", "fill": "muted", "size": 2.1, "align": "center"}));
    if let Some(r) = p.record.as_ref().filter(|r| r["seasons"].as_i64().unwrap_or(0) > 0) {
        let n = |k: &str| r[k].as_i64().unwrap_or(0);
        let line = format!("All seasons: {} played, {} won, best {} cells", n("seasons"), n("wins"), n("best_cells"));
        text(ops, 41.0, line, "muted", 2.1);
    }
    ops.push(json!({"op": "input", "x": 3, "y": 44, "w": 74, "submit": "name", "value": c.name, "placeholder": "rename your colony", "max": 16}));
    match &p.link_code {
        Some(code) => {
            text(ops, 55.5, format!("Post {code} in any channel within 15 minutes"), "accent", 2.3);
            text(ops, 59.5, "to show your record on your profile. Delete it after.".into(), "muted", 2.1);
        }
        None => text(ops, 55.5, "Fruit by long pressing your own hyphae on the board.".into(), "muted", 2.1),
    }
}

/// The colony's own patch of soil, painted at `PORTRAIT` pixels square: dark earth, glowing
/// hyphae threaded with noise, bright mushrooms and a white heart. Base64 PNG.
pub fn portrait(world: &World, slot: u8) -> Option<String> {
    let mine: Vec<usize> = (0..world.cells.len()).filter(|i| world.cells[*i].owner() == slot).collect();
    let rows = mine.iter().map(|i| i / world.w);
    let cols = mine.iter().map(|i| i % world.w);
    let (r0, r1) = (rows.clone().min()?, rows.max()?);
    let (c0, c1) = (cols.clone().min()?, cols.max()?);
    let side = (r1 - r0).max(c1 - c0) + 5;
    let (cr, cc) = ((r0 + r1) as f32 / 2.0, (c0 + c1) as f32 / 2.0);
    let (top, left) = (cr - side as f32 / 2.0, cc - side as f32 / 2.0);
    let near = distance_field(world, slot);
    let base = rgb(colour(slot));
    let mut pixels = Vec::with_capacity(PORTRAIT * PORTRAIT * 3);
    for py in 0..PORTRAIT {
        for px in 0..PORTRAIT {
            let y = top + (py as f32 + 0.5) / PORTRAIT as f32 * side as f32;
            let x = left + (px as f32 + 0.5) / PORTRAIT as f32 * side as f32;
            pixels.extend_from_slice(&shade(world, &near, (y, x), base));
        }
    }
    Some(b64::encode(&png::encode_rgb(PORTRAIT as u32, PORTRAIT as u32, &pixels)))
}

/// How far each cell is from the colony, in tenths of a cell (a two-pass chamfer transform).
fn distance_field(world: &World, slot: u8) -> Vec<u16> {
    let (w, h) = (world.w, world.h);
    let mut d: Vec<u16> = world.cells.iter().map(|c| if c.owner() == slot { 0 } else { 9999 }).collect();
    for i in 0..w * h {
        let (r, c) = (i / w, i % w);
        let mut best = d[i];
        if c > 0 {
            best = best.min(d[i - 1] + 10);
        }
        if r > 0 {
            best = best.min(d[i - w] + 10);
            if c > 0 {
                best = best.min(d[i - w - 1] + 14);
            }
            if c + 1 < w {
                best = best.min(d[i - w + 1] + 14);
            }
        }
        d[i] = best;
    }
    for i in (0..w * h).rev() {
        let (r, c) = (i / w, i % w);
        let mut best = d[i];
        if c + 1 < w {
            best = best.min(d[i + 1] + 10);
        }
        if r + 1 < h {
            best = best.min(d[i + w] + 10);
            if c + 1 < w {
                best = best.min(d[i + w + 1] + 14);
            }
            if c > 0 {
                best = best.min(d[i + w - 1] + 14);
            }
        }
        d[i] = best;
    }
    d
}

fn shade(world: &World, near: &[u16], (y, x): (f32, f32), base: [u8; 3]) -> [u8; 3] {
    if y < 0.0 || x < 0.0 || y >= world.h as f32 || x >= world.w as f32 {
        return [10, 8, 7];
    }
    let (r, c) = (y as usize, x as usize);
    let i = r * world.w + c;
    let cell = world.cells[i];
    let grain = (hash(&mix(&[(x * 7.0) as u64, (y * 7.0) as u64]).to_le_bytes()) % 24) as f32;
    let mut out = match cell.terrain() {
        ROCK => [62.0, 60.0, 64.0],
        WATER => [26.0, 46.0, 70.0],
        VEIN => [120.0, 98.0, 40.0],
        CLAY => [70.0, 48.0, 34.0],
        _ => [34.0 + f32::from(cell.nutrient()) * 2.0, 25.0 + f32::from(cell.nutrient()), 19.0],
    };
    for ch in &mut out {
        *ch += grain * 0.4;
    }
    let glow = (-(f32::from(near[i]) / 10.0) / 1.4).exp();
    let tint = [f32::from(base[0]), f32::from(base[1]), f32::from(base[2])];
    if near[i] == 0 {
        let (fy, fx) = (y.fract() - 0.5, x.fract() - 0.5);
        let thread = 0.55 + 0.45 * (1.0 - (fx * fy * 8.0 + grain / 30.0).abs().min(1.0));
        let lift = 0.55 + 0.15 * f32::from(cell.strength());
        for k in 0..3 {
            out[k] = out[k] * 0.2 + tint[k] * thread * lift;
        }
        if cell.mushroom() || cell.anchor() {
            let spot = 1.0 - ((fx * fx + fy * fy).sqrt() * 2.2).min(1.0);
            let cap = if cell.anchor() && !cell.mushroom() { [255.0, 255.0, 255.0] } else { [255.0, 236.0, 200.0] };
            for k in 0..3 {
                out[k] += (cap[k] - out[k]) * spot;
            }
        }
    } else {
        for k in 0..3 {
            out[k] += tint[k] * glow * 0.45;
        }
    }
    out.map(|v| v.clamp(0.0, 255.0) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;

    #[test]
    fn a_colony_with_no_cells_has_no_portrait() {
        let world = World::generate(48, 30, 4);
        assert!(portrait(&world, 3).is_none());
    }

    #[test]
    fn a_portrait_is_a_png_inside_the_image_ceiling() {
        let mut world = World::generate(128, 80, 4);
        for i in (0..world.cells.len()).step_by(3) {
            if world.cells[i].passable() {
                world.cells[i].set_owner(2);
            }
        }
        let image = portrait(&world, 2).expect("drawn");
        assert!(image.len() < 88 * 1024, "{}", image.len());
        let bytes = b64::decode(&image).unwrap();
        assert_eq!(&bytes[1..4], b"PNG");
    }

    #[test]
    fn the_distance_field_is_zero_on_the_colony_and_grows_away_from_it() {
        let mut world = World::generate(48, 30, 4);
        let at = world.idx(10, 10);
        world.cells[at].set_owner(1);
        let d = distance_field(&world, 1);
        assert_eq!(d[at], 0);
        assert_eq!(d[world.idx(10, 13)], 30);
        assert_eq!(d[world.idx(11, 11)], 14);
    }
}
