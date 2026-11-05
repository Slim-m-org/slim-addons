//! What players and the space do to the world: moves from the log, and weather from the inbox.
//!
//! A move is applied when a frame replays the log, never straight from a tap, so two people
//! pressing at once cannot overwrite each other: the second frame simply replays both.

use crate::colony::{clean_name, default_name, Colony, MAX_MUSHROOMS, MAX_PLAYERS, MAX_WAYPOINTS, WILD_ID};
use crate::game::{Fx, Game};
use crate::rng::{hash, mix, Rng};
use crate::world::{World, SKY, WILD};

pub const FRUIT_COST: i32 = 40;
pub const BURST_COST: i32 = 30;
pub const SPORE_COST: i32 = 60;
const START_ENERGY: i32 = 30;

/// One logged move: who, what, and its argument.
#[derive(Clone, Debug, PartialEq)]
pub struct Move {
    pub pid: String,
    pub act: String,
    pub arg: String,
}

/// What a move told the player who made it: `Ok` when it changed the world.
pub type Said = Result<String, String>;

/// Applies `mv`.
pub fn apply(game: &mut Game, world: &mut World, mv: &Move) -> Said {
    if game.over && mv.act != "name" {
        return Err("the season is over".into());
    }
    match mv.act.as_str() {
        "join" => join(game, world, &mv.pid),
        "lure" => lure(game, world, &mv.pid, &mv.arg),
        "fruit" => fruit(game, world, &mv.pid, &mv.arg),
        "burst" => burst(game, &mv.pid),
        "spore" => spore(game, world, &mv.pid),
        "name" => rename(game, &mv.pid, &mv.arg),
        _ => Err("nothing happened".into()),
    }
}

fn join(game: &mut Game, world: &mut World, pid: &str) -> Said {
    if let Some(c) = game.colony(pid) {
        return Err(format!("you are already growing as {}", c.name));
    }
    if pid.is_empty() {
        return Err("slim did not say who you are".into());
    }
    if game.c.iter().filter(|c| !c.is_wild()).count() >= MAX_PLAYERS {
        return Err("six colonies already share this soil; wait for next season".into());
    }
    let slot = (1..=MAX_PLAYERS as u8).find(|s| game.c.iter().all(|c| c.slot != *s)).unwrap_or(1);
    let mut rng = Rng::new(mix(&[game.seed, hash(pid.as_bytes()), u64::from(game.tick)]));
    let Some(heart) = find_home(game, world, &mut rng, (SKY + 1, (SKY + 7).min(world.h - 1))) else {
        return Err("there is no open soil left to root in".into());
    };
    let name = default_name(pid, game.season);
    root(world, heart, slot);
    game.c.push(Colony { pid: pid.into(), name: name.clone(), slot, energy: START_ENERGY, heart: heart as u32, ..Colony::default() });
    game.note(format!("{name} took root"));
    Ok(format!("you are {name}; tap the soil to lure your hyphae, long press to fruit"))
}

/// The open cell in `rows` farthest from every other heart.
fn find_home(game: &Game, world: &World, rng: &mut Rng, rows: (usize, usize)) -> Option<usize> {
    let hearts: Vec<usize> = game.c.iter().map(|c| c.heart as usize).collect();
    let span = (rows.1.saturating_sub(rows.0) + 1) as u32;
    (0..60)
        .map(|_| world.idx(rows.0 + rng.below(span) as usize, 1 + rng.below(world.w as u32 - 2) as usize))
        .filter(|i| world.cells[*i].passable() && world.cells[*i].owner() == 0)
        .max_by_key(|i| {
            let (r, c) = ((*i / world.w) as i32, (*i % world.w) as i32);
            hearts
                .iter()
                .map(|h| ((h / world.w) as i32 - r).abs() + ((h % world.w) as i32 - c).abs())
                .min()
                .unwrap_or(999)
        })
}

/// A heart and the open soil around it.
fn root(world: &mut World, heart: usize, slot: u8) {
    let around: Vec<usize> = world.neighbours(heart).collect();
    for i in std::iter::once(heart).chain(around) {
        let mut cell = world.cells[i];
        if !cell.passable() || (cell.owner() != 0 && i != heart) {
            continue;
        }
        cell.clear_colony();
        cell.set_dead(false);
        cell.set_owner(slot);
        cell.set_strength(if i == heart { 3 } else { 1 });
        cell.set_anchor(i == heart);
        world.cells[i] = cell;
    }
}

/// `r,c;r,c;...` as cell indices on the board, in order, at most eight.
pub fn cells_of(world: &World, arg: &str) -> Vec<u32> {
    let all: Vec<u32> = arg
        .split(';')
        .filter_map(|p| {
            let (r, c) = p.split_once(',')?;
            let (r, c) = (r.trim().parse::<usize>().ok()?, c.trim().parse::<usize>().ok()?);
            (r < world.h && c < world.w).then(|| world.idx(r, c) as u32)
        })
        .collect();
    if all.len() <= MAX_WAYPOINTS {
        return all;
    }
    // A long drag keeps its shape: evenly spaced points, always ending where the finger stopped.
    let step = (all.len() - 1) as f32 / (MAX_WAYPOINTS - 1) as f32;
    (0..MAX_WAYPOINTS).map(|k| all[((k as f32 * step).round() as usize).min(all.len() - 1)]).collect()
}

fn lure(game: &mut Game, world: &World, pid: &str, arg: &str) -> Said {
    let points = cells_of(world, arg);
    let Some(colony) = game.colony_mut(pid) else { return Err("join first, then lure".into()) };
    if points.is_empty() {
        return Err("that is not a place on the board".into());
    }
    colony.waypoints = points;
    let n = colony.waypoints.len();
    if n == 1 {
        Ok("your hyphae turn toward it".into())
    } else {
        Ok(format!("your hyphae will follow {n} waypoints"))
    }
}

fn fruit(game: &mut Game, world: &mut World, pid: &str, arg: &str) -> Said {
    let Some(&at) = cells_of(world, arg).first() else { return Err("that is not a place on the board".into()) };
    let Some(colony) = game.colony(pid) else { return Err("join first, then fruit".into()) };
    let (slot, energy, name) = (colony.slot, colony.energy, colony.name.clone());
    let mut cell = world.cells[at as usize];
    if cell.owner() != slot {
        return Err("you can only fruit on your own hyphae".into());
    }
    if cell.mushroom() {
        return Err("a mushroom already stands there".into());
    }
    let standing = world.cells.iter().filter(|c| c.owner() == slot && c.mushroom()).count() as u32;
    if standing >= MAX_MUSHROOMS {
        return Err("eight mushrooms is all one colony can hold up".into());
    }
    if energy < FRUIT_COST {
        return Err(format!("fruiting takes {FRUIT_COST} energy; you have {energy}"));
    }
    cell.set_mushroom(true);
    cell.set_strength(3);
    world.cells[at as usize] = cell;
    if let Some(c) = game.colony_mut(pid) {
        c.energy -= FRUIT_COST;
    }
    if game.fx.len() < 8 {
        game.fx.push(Fx { k: 'f', a: at, b: 0 });
    }
    game.note(format!("{name} fruited"));
    Ok("a mushroom pushes up through the soil".into())
}

fn burst(game: &mut Game, pid: &str) -> Said {
    let Some(c) = game.colony_mut(pid) else { return Err("join first".into()) };
    if c.energy < BURST_COST {
        return Err(format!("a burst takes {BURST_COST} energy; you have {}", c.energy));
    }
    c.energy -= BURST_COST;
    c.burst = 30;
    Ok("your colony surges for thirty ticks".into())
}

fn spore(game: &mut Game, world: &mut World, pid: &str) -> Said {
    let Some(c) = game.colony(pid) else { return Err("join first".into()) };
    let (slot, heart, energy) = (c.slot, c.heart, c.energy);
    let Some(&target) = c.waypoints.last() else { return Err("lure somewhere first; the spore flies to your last waypoint".into()) };
    if energy < SPORE_COST {
        return Err(format!("a spore shot takes {SPORE_COST} energy; you have {energy}"));
    }
    let landing = std::iter::once(target as usize)
        .chain(world.neighbours(target as usize))
        .find(|i| world.cells[*i].passable() && world.cells[*i].owner() == 0);
    let Some(at) = landing else { return Err("the spore needs open soil at your last waypoint".into()) };
    let mut cell = world.cells[at];
    cell.set_dead(false);
    cell.set_owner(slot);
    cell.set_anchor(true);
    cell.set_strength(2);
    world.cells[at] = cell;
    if let Some(c) = game.colony_mut(pid) {
        c.energy -= SPORE_COST;
    }
    push_fx(game, Fx { k: 's', a: heart, b: at as u32 });
    Ok("your spore lands and roots an outpost".into())
}

fn rename(game: &mut Game, pid: &str, arg: &str) -> Said {
    let Some(name) = clean_name(arg) else { return Err("type a name first".into()) };
    let Some(c) = game.colony_mut(pid) else { return Err("join first, then name your colony".into()) };
    c.name = name.clone();
    Ok(format!("your colony is now {name}"))
}

/// One thing the space did, from the inbox the events fill.
#[derive(Clone, Debug, PartialEq)]
pub struct Weather {
    pub kind: String,
    pub at: u32,
    pub amount: u32,
}

/// A message is a leaf that rots into the topsoil; a reaction is rain; time in calls is warmth;
/// someone joining the space stirs the wild rot.
pub fn weather(game: &mut Game, world: &mut World, w: &Weather) {
    let col = (w.at as usize) % world.w;
    match w.kind.as_str() {
        "leaf" => {
            let amount = w.amount.clamp(1, 8) as u8;
            for r in SKY..(SKY + 6).min(world.h) {
                for c in col.saturating_sub(1)..=(col + 1).min(world.w - 1) {
                    let i = world.idx(r, c);
                    let mut cell = world.cells[i];
                    if cell.passable() {
                        cell.set_nutrient(cell.nutrient().saturating_add(amount));
                        world.cells[i] = cell;
                    }
                }
            }
            game.leaves += 1;
            push_fx(game, Fx { k: 'l', a: col as u32, b: 0 });
        }
        "rain" => {
            game.rain.retain(|b| b.0 != col as u32);
            game.rain.push((col as u32, 40));
            game.rain.truncate(6);
            push_fx(game, Fx { k: 'r', a: col as u32, b: 0 });
        }
        "warm" => {
            game.warm = (game.warm + w.amount.min(60) * 6).min(400);
            game.note("a warm spell from the calls".into());
        }
        "wild" => wild(game, world, w.amount),
        _ => {}
    }
}

fn push_fx(game: &mut Game, fx: Fx) {
    if game.fx.len() < 8 {
        game.fx.push(fx);
    }
}

fn wild(game: &mut Game, world: &mut World, salt: u32) {
    if let Some(c) = game.c.iter_mut().find(|c| c.is_wild()) {
        c.energy += 40;
        return;
    }
    let mut rng = Rng::new(mix(&[game.seed, u64::from(salt), 0x3117]));
    let deep = (world.h / 2, world.h - 1);
    let Some(heart) = find_home(game, world, &mut rng, deep) else { return };
    root(world, heart, WILD);
    game.c.push(Colony { pid: WILD_ID.into(), name: "the wild rot".into(), slot: WILD, energy: 60, heart: heart as u32, ..Colony::default() });
    push_fx(game, Fx { k: 'w', a: heart as u32, b: 0 });
    game.note("the wild rot stirs".into());
}

#[cfg(test)]
#[path = "moves_tests.rs"]
mod tests;
