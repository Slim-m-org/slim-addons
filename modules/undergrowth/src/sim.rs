//! The world turning over: hyphae drink, harden, grow, fight, fruit, throw spores and wither
//! when cut off. Every roll comes from the seed and the tick, so the same moves give the same season.

use crate::colony::{ENERGY_CAP, MAX_WAYPOINTS};
use crate::game::{Fx, Game};
use crate::rng::{mix, Rng};
use crate::world::{World, CLAY, MAX_NUTRIENT, VEIN, WATER};

/// How many growth candidates each colony keeps per tick; the best few win.
const TOP_K: usize = 12;
/// A colony grows on one tick in this many, faster in a burst or a warm spell.
const GROW_EVERY: u32 = 3;
/// A heart feeds its colony one energy every this many ticks, so a ruined colony can crawl back.
const HEART_EVERY: u32 = 3;
const SPORE_EVERY: u32 = 90;
const WITHER_EVERY: u32 = 6;
/// Hyphae harden one step every this many ticks, so old territory is hard to take.
const HARDEN_EVERY: u32 = 24;


/// What one call's ticks did, by owner slot: it chooses the presser's sounds and the news.
#[derive(Default, Clone, Debug)]
pub struct News {
    pub grew: [u32; 8],
    pub took: [u32; 8],
    pub lost: [u32; 8],
    pub spores: [u32; 8],
    pub withered: [u32; 8],
    pub ticks: u32,
}

/// Runs up to `ticks` ticks, stopping at the end of the season.
pub fn advance(game: &mut Game, world: &mut World, ticks: u32) -> News {
    let mut news = News::default();
    for _ in 0..ticks {
        if game.over {
            break;
        }
        tick(game, world, &mut news);
        news.ticks += 1;
    }
    recount(game, world);
    news
}

/// Where each owner slot's colony sits in `game.c`.
pub fn slot_index(game: &Game) -> [Option<usize>; 8] {
    let mut idx = [None; 8];
    for (i, c) in game.c.iter().enumerate() {
        idx[c.slot as usize & 7] = Some(i);
    }
    idx
}

/// Refreshes each colony's cell and mushroom counts from the soil.
pub fn recount(game: &mut Game, world: &World) {
    let mut cells = [0u32; 8];
    let mut mushrooms = [0u32; 8];
    for cell in &world.cells {
        let o = cell.owner() as usize;
        cells[o] += 1;
        mushrooms[o] += u32::from(cell.mushroom());
    }
    for c in &mut game.c {
        let o = c.slot as usize & 7;
        c.cells = cells[o];
        c.mushrooms = mushrooms[o];
        c.peak = c.peak.max(c.cells);
    }
}

fn tick(game: &mut Game, world: &mut World, news: &mut News) {
    let t = game.tick;
    let mut rng = Rng::new(mix(&[game.seed, u64::from(t), 0x71c]));
    let idx = slot_index(game);
    game.warm = game.warm.saturating_sub(1);
    for band in &mut game.rain {
        band.1 = band.1.saturating_sub(1);
    }
    game.rain.retain(|b| b.1 > 0);

    let (counts, spores) = feed(game, world, &idx, t);
    if t.is_multiple_of(10) {
        let mut hunger = [0u32; 8];
        for c in &mut game.c {
            let slot = c.slot as usize & 7;
            let owed = upkeep(counts[slot]);
            if c.energy < owed {
                // Odds out of 1024 that any one cell dies back: about two cells per unpaid point.
                hunger[slot] = ((owed - c.energy) as u32 * 2 * 1024 / counts[slot].max(1)).min(1024);
            }
            c.energy = (c.energy - owed).max(0);
        }
        starve(world, &hunger, &mut rng, news);
    }
    if t.is_multiple_of(HEART_EVERY) {
        for c in &mut game.c {
            c.energy = (c.energy + 1).min(ENERGY_CAP);
        }
    }
    wild_mind(game, world, &mut rng, t);
    grow(game, world, &idx, &mut rng, news);
    for (from, owner) in spores {
        throw_spore(game, world, &mut rng, (from, owner), news);
    }
    if t.is_multiple_of(WITHER_EVERY) {
        wither(game, world, news);
    }
    for c in &mut game.c {
        c.burst = c.burst.saturating_sub(1);
    }
    game.tick += 1;
    if game.tick >= game.end {
        game.over = true;
    }
}

/// One pass over the soil: veins regrow, hyphae drink and harden, mushrooms score and ripen.
fn feed(game: &mut Game, world: &mut World, idx: &[Option<usize>; 8], t: u32) -> ([u32; 8], Vec<(usize, u8)>) {
    let mut counts = [0u32; 8];
    let mut spores = Vec::new();
    for i in 0..world.cells.len() {
        let mut cell = world.cells[i];
        let phase = (i as u32).wrapping_mul(7).wrapping_add(t);
        if cell.terrain() == VEIN && phase.is_multiple_of(6) && cell.nutrient() < MAX_NUTRIENT {
            cell.set_nutrient(cell.nutrient() + 1);
        }
        let o = cell.owner();
        if o != 0 {
            match idx[o as usize] {
                Some(ci) => {
                    counts[o as usize] += 1;
                    let colony = &mut game.c[ci];
                    if phase.is_multiple_of(4) && cell.nutrient() > 0 {
                        cell.set_nutrient(cell.nutrient() - 1);
                        colony.energy = (colony.energy + 1).min(ENERGY_CAP);
                    }
                    if phase.is_multiple_of(HARDEN_EVERY) {
                        cell.set_strength(cell.strength() + 1);
                    }
                    if cell.mushroom() {
                        if phase.is_multiple_of(10) {
                            colony.score += 1;
                        }
                        if phase.is_multiple_of(SPORE_EVERY) {
                            spores.push((i, o));
                        }
                    }
                }
                None => cell.clear_colony(),
            }
        }
        world.cells[i] = cell;
    }
    (counts, spores)
}

/// What a colony of `cells` owes every ten ticks. It grows faster than the colony does, so one
/// that has swallowed half the soil starts to strain under its own weight.
pub fn upkeep(cells: u32) -> i32 {
    (cells / 40 + cells * cells / 15_000) as i32
}

/// A colony that cannot pay its upkeep dies back in proportion to what it owes, which frees
/// soil for whoever can still afford to grow, and can cut its network so more withers after.
fn starve(world: &mut World, hunger: &[u32; 8], rng: &mut Rng, news: &mut News) {
    if hunger.iter().all(|h| *h == 0) {
        return;
    }
    for i in 0..world.cells.len() {
        let mut cell = world.cells[i];
        let o = cell.owner() as usize;
        if o == 0 || hunger[o] == 0 || cell.anchor() || cell.mushroom() || rng.below(1024) >= hunger[o] {
            continue;
        }
        news.withered[o] += 1;
        cell.clear_colony();
        cell.set_dead(true);
        cell.set_nutrient(cell.nutrient().saturating_add(2));
        world.cells[i] = cell;
    }
}

/// The wild rot plays itself: it drifts toward the richest soil it can see and feeds on nothing.
fn wild_mind(game: &mut Game, world: &World, rng: &mut Rng, t: u32) {
    let Some(wild) = game.c.iter_mut().find(|c| c.is_wild()) else { return };
    if t.is_multiple_of(40) || wild.waypoints.is_empty() {
        let n = world.cells.len() as u32;
        let best = (0..30).map(|_| rng.below(n)).max_by_key(|i| {
            let c = world.cells[*i as usize];
            (if c.passable() { i32::from(c.nutrient()) * 4 } else { -99 }) + i32::from(c.terrain() == VEIN) * 20
        });
        wild.waypoints = best.into_iter().collect();
    }
}

fn manhattan(w: usize, a: usize, b: usize) -> u32 {
    let (ar, ac, br, bc) = ((a / w) as i32, (a % w) as i32, (b / w) as i32, (b % w) as i32);
    ((ar - br).abs() + (ac - bc).abs()) as u32
}

fn wet(game: &Game, world: &World, i: usize) -> bool {
    let col = (i % world.w) as i32;
    game.rain.iter().any(|(c, _)| (col - *c as i32).abs() <= 4)
}

type Candidate = (i32, usize, usize);

/// Every colony scores the soil at its edge and spends energy on the best of it.
fn grow(game: &mut Game, world: &mut World, idx: &[Option<usize>; 8], rng: &mut Rng, news: &mut News) {
    let mut best: [Vec<Candidate>; 8] = Default::default();
    let w = world.w;
    for i in 0..world.cells.len() {
        let o = world.cells[i].owner() as usize;
        let Some(ci) = (o != 0).then(|| idx[o]).flatten() else { continue };
        let lure = game.c[ci].waypoints.first().map(|p| *p as usize);
        for j in world.neighbours(i) {
            let target = world.cells[j];
            if !target.passable() || target.owner() as usize == o {
                continue;
            }
            let mut score = i32::from(target.nutrient()) * 3 + rng.below(10) as i32;
            score += match target.terrain() {
                VEIN => 8,
                WATER => 3,
                CLAY => -4,
                _ => 0,
            };
            if target.dead() {
                score += 4;
            }
            if target.owner() != 0 {
                score += 2 - 3 * i32::from(target.strength());
            }
            if let Some(p) = lure {
                if manhattan(w, j, p) < manhattan(w, i, p) {
                    score += 16;
                }
            }
            if wet(game, world, j) {
                score += 3;
            }
            keep_best(&mut best[o], (score, i, j));
        }
    }
    let n = game.c.len();
    for k in 0..n {
        let ci = (k + game.tick as usize) % n;
        let o = game.c[ci].slot as usize & 7;
        let budget = budget(game, ci);
        let mut list = std::mem::take(&mut best[o]);
        list.sort_by(|a, b| b.0.cmp(&a.0));
        let mut spent = 0;
        for (_, from, to) in list {
            if spent >= budget {
                break;
            }
            if world.cells[from].owner() as usize == o && claim(game, world, (ci, from, to), news) {
                spent += 1;
            }
        }
    }
}

fn keep_best(list: &mut Vec<Candidate>, cand: Candidate) {
    if list.len() < TOP_K {
        list.push(cand);
        return;
    }
    let (worst, value) = list.iter().enumerate().min_by_key(|(_, c)| c.0).map(|(k, c)| (k, c.0)).unwrap_or((0, i32::MAX));
    if cand.0 > value {
        list[worst] = cand;
    }
}

/// How many cells a colony may try to take this tick.
fn budget(game: &Game, ci: usize) -> u32 {
    let c = &game.c[ci];
    let t = game.tick + u32::from(c.slot);
    let pace = if c.burst > 0 {
        1
    } else if game.warm > 0 {
        2
    } else {
        GROW_EVERY
    };
    if !t.is_multiple_of(pace) {
        return 0;
    }
    1 + u32::from(c.energy >= 30) + u32::from(c.energy >= 90) + u32::from(c.burst > 0)
}

/// Takes `to` for colony `ci` if it can pay: open soil outright, an enemy cell only when the
/// hyphae around it outnumber its strength. Hearts never fall.
fn claim(game: &mut Game, world: &mut World, (ci, _from, to): (usize, usize, usize), news: &mut News) -> bool {
    let o = game.c[ci].slot;
    let mut cell = world.cells[to];
    if !cell.passable() || cell.owner() == o {
        return false;
    }
    let mut cost = if cell.terrain() == CLAY { 3 } else { 1 };
    if wet(game, world, to) {
        cost = (cost - 1).max(1);
    }
    let enemy = cell.owner();
    if enemy != 0 {
        if game.c.iter().any(|c| c.slot == enemy && c.heart as usize == to) {
            return false;
        }
        cost += 2;
    }
    if game.c[ci].energy < cost {
        return false;
    }
    game.c[ci].energy -= cost;
    if enemy != 0 {
        let support = world.neighbours(to).filter(|n| world.cells[*n].owner() == o).count() as u8;
        let defence = cell.strength() + 2 * u8::from(cell.anchor()) + u8::from(cell.mushroom());
        if support <= defence {
            cell.set_strength(cell.strength().saturating_sub(1));
            world.cells[to] = cell;
            return true;
        }
        news.took[o as usize] += 1;
        news.lost[enemy as usize] += 1;
    } else {
        news.grew[o as usize] += 1;
    }
    cell.clear_colony();
    cell.set_dead(false);
    cell.set_owner(o);
    world.cells[to] = cell;
    let w = world.w;
    let colony = &mut game.c[ci];
    if colony.waypoints.first().is_some_and(|p| manhattan(w, to, *p as usize) <= 1) {
        colony.waypoints.remove(0);
    }
    colony.waypoints.truncate(MAX_WAYPOINTS);
    true
}

/// A ripe mushroom throws a spore up to six cells away, which roots as an outpost: an anchor of its own.
fn throw_spore(game: &mut Game, world: &mut World, rng: &mut Rng, (from, owner): (usize, u8), news: &mut News) {
    let (w, h) = (world.w as i32, world.h as i32);
    let (r0, c0) = ((from / world.w) as i32, (from % world.w) as i32);
    for _ in 0..12 {
        let (r, c) = (r0 + rng.below(13) as i32 - 6, c0 + rng.below(13) as i32 - 6);
        if r < 0 || c < 0 || r >= h || c >= w {
            continue;
        }
        let to = (r * w + c) as usize;
        let mut cell = world.cells[to];
        if cell.passable() && cell.owner() == 0 {
            cell.set_dead(false);
            cell.set_owner(owner);
            cell.set_anchor(true);
            cell.set_strength(1);
            world.cells[to] = cell;
            news.spores[owner as usize] += 1;
            if game.fx.len() < 8 {
                game.fx.push(Fx { k: 's', a: from as u32, b: to as u32 });
            }
            return;
        }
    }
}

/// Hyphae with no path home to a heart or an outpost die back, and leave their soil richer.
pub fn wither(game: &mut Game, world: &mut World, news: &mut News) {
    for c in &game.c {
        let mut heart = world.cells[c.heart as usize];
        if heart.passable() {
            heart.set_owner(c.slot);
            heart.set_anchor(true);
            heart.set_strength(3);
            world.cells[c.heart as usize] = heart;
        }
    }
    let n = world.cells.len();
    let mut seen = vec![false; n];
    let mut queue: Vec<usize> = (0..n).filter(|i| world.cells[*i].owner() != 0 && world.cells[*i].anchor()).collect();
    for i in &queue {
        seen[*i] = true;
    }
    while let Some(i) = queue.pop() {
        let o = world.cells[i].owner();
        for j in world.neighbours(i) {
            if !seen[j] && world.cells[j].owner() == o {
                seen[j] = true;
                queue.push(j);
            }
        }
    }
    for (i, reached) in seen.iter().enumerate() {
        let mut cell = world.cells[i];
        if cell.owner() == 0 || *reached {
            continue;
        }
        news.withered[cell.owner() as usize] += 1;
        cell.clear_colony();
        cell.set_dead(true);
        cell.set_nutrient(cell.nutrient().saturating_add(2));
        world.cells[i] = cell;
    }
}

#[cfg(test)]
#[path = "sim_tests.rs"]
mod tests;
