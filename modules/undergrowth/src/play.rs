//! One press on the shared surface, start to finish.
//!
//! 1. The press becomes a move in the log (`data.store`), never a direct edit of the board;
//!    starting a new season is a move too.
//! 2. Every move since the board's last one is replayed, the presser's included.
//! 3. Weather the space's events left in the inbox is applied.
//! 4. The clock runs the world forward by the real time that passed, so a redraw, a tap and
//!    someone's play loop all agree on how fast the season goes.
//! 5. A season that just ended is recorded once, and a timer is set to announce it.
//!
//! Two presses at once both reach the log, so whichever frame slim stores last still holds
//! both: the losing frame's moves are replayed by the next one.

use serde_json::{json, Value};

use crate::colony::{pid_of, Colony};
use crate::game::Game;
use crate::host::Host;
use crate::moves::{self, Move, Said};
use crate::panel::{self, Panel};
use crate::rng::{hash, mix};
use crate::scene::{self, Frame};
use crate::settings::Settings;
use crate::sim::{self, News};
use crate::song;
use crate::store::Store;
use crate::world::{size_for, World};

/// The most ticks one frame runs, so a board nobody watched for an hour wakes in one bounded step.
pub const MAX_CATCH_UP: i64 = 40;
const SAVE_EVERY_MS: i64 = 15_000;

pub struct Run<'a> {
    pub caller: &'a str,
    pub now_ms: i64,
    pub entropy: &'a str,
    pub settings: &'a Settings,
}

/// The scene, and the one-shot timer to ask slim for, if any.
pub struct Answer {
    pub output: String,
    pub run_after: Option<Value>,
}

pub fn surface<H: Host>(store: &Store<H>, input: &str, run: &Run) -> Answer {
    let press: Value = serde_json::from_str(input.trim()).unwrap_or(Value::Null);
    let state = press["state"].as_str();
    let Some((game, world)) = state.and_then(Game::decode) else {
        let why = state.map(|_| "that board could not be read, so fresh soil was dealt");
        return launch(store, run, why);
    };
    // A malformed action on a real board redraws it; only a press with no board deals fresh soil.
    let action = press["action"].as_str().unwrap_or("");
    let pid = pid_of(run.caller);
    let private = matches!(action, "my colony" | "burst" | "spore shot" | "link profile") || action.starts_with("name:");
    if private {
        return Answer { output: colony_panel(store, (game, world), run, (&pid, action), state.unwrap_or_default().to_owned()), run_after: None };
    }
    shared(store, (game, world), run, (&pid, action))
}

/// The move a shared press makes, if it makes one.
fn move_for(pid: &str, action: &str) -> Option<Move> {
    let mv = |act: &str, arg: &str| Some(Move { pid: pid.into(), act: act.into(), arg: arg.into() });
    match action.split_once(':') {
        Some(("lure", cells)) => mv("lure", cells),
        Some(("fruit", cells)) => mv("fruit", cells),
        Some(("name", name)) => mv("name", name),
        _ => match action {
            "join" => mv("join", ""),
            "burst" => mv("burst", ""),
            "spore shot" => mv("spore", ""),
            _ => None,
        },
    }
}

/// Logs the press, then replays the log; returns what the presser's own move said, and why the log could not be used.
fn take_moves<H: Host>(store: &Store<H>, (game, world): (&mut Game, &mut World), run: &Run, mv: Option<Move>) -> (Option<(String, Said)>, Option<String>) {
    let mut own = None;
    let mut trouble = None;
    let logged = match &mv {
        Some(mv) => match store.log_move(&game.g, mv, run.now_ms) {
            Ok(id) => Some(id),
            Err(why) => {
                // Without the log the press still counts, it just can be lost to a simultaneous one.
                own = Some((mv.act.clone(), replay((game, world), run.settings, (hash(run.entropy.as_bytes()), run.now_ms), mv)));
                trouble = Some(why);
                None
            }
        },
        None => None,
    };
    match store.moves_after(&game.g, game.mv) {
        Ok(list) => {
            for (id, at_ms, m) in list {
                let said = replay((game, world), run.settings, (id as u64, at_ms), &m);
                if Some(id) == logged {
                    own = Some((m.act.clone(), said));
                }
                game.mv = game.mv.max(id);
            }
        }
        Err(why) => trouble = trouble.or(Some(why)),
    }
    (own, trouble)
}

/// Applies one logged move; a new season is a move too, so every frame that replays it deals the same soil.
fn replay((game, world): (&mut Game, &mut World), settings: &Settings, (salt, at_ms): (u64, i64), m: &Move) -> Said {
    if m.act != "season" {
        return moves::apply(game, world, m);
    }
    if !game.over || m.arg.parse::<u32>().ok() != Some(game.season + 1) {
        return Err("a new season has already begun".into());
    }
    let seed = mix(&[game.seed, salt]);
    new_season((game, world), settings, seed, at_ms);
    Ok(format!("season {} begins", game.season))
}

fn take_weather<H: Host>(store: &Store<H>, (game, world): (&mut Game, &mut World), settings: &Settings) {
    let Ok(list) = store.weather_after(game.ib) else { return };
    for (id, mut w) in list {
        game.ib = game.ib.max(id);
        let skip = (w.kind == "wild" && !settings.wild_rot) || (w.kind == "leaf" && settings.leaf_power == 0);
        if skip || game.over {
            continue;
        }
        if w.kind == "leaf" {
            w.amount = (w.amount * settings.leaf_power).div_ceil(3);
        }
        moves::weather(game, world, &w);
    }
}

/// Ticks owed for the time since the world last moved, and the clock to keep.
pub fn ticks_owed(game_ms: i64, now_ms: i64, tick_ms: i64) -> (u32, i64) {
    let elapsed = now_ms - game_ms;
    if elapsed <= 0 {
        return (0, game_ms);
    }
    let owed = elapsed / tick_ms.max(1);
    if owed > MAX_CATCH_UP {
        return (MAX_CATCH_UP as u32, now_ms);
    }
    (owed as u32, game_ms + owed * tick_ms)
}

fn shared<H: Host>(store: &Store<H>, (mut game, mut world): (Game, World), run: &Run, (pid, action): (&str, &str)) -> Answer {
    let s = run.settings;
    let mv = match action {
        "new season" => game.over.then(|| Move { pid: pid.into(), act: "season".into(), arg: (game.season + 1).to_string() }),
        _ => move_for(pid, action),
    };
    let (own, trouble) = take_moves(store, (&mut game, &mut world), run, mv);
    take_weather(store, (&mut game, &mut world), s);
    let was_over = game.over;
    let (ticks, clock) = ticks_owed(game.ms, run.now_ms, s.tick_ms);
    game.ms = clock;
    let news = sim::advance(&mut game, &mut world, ticks);
    let mut run_after = None;
    if game.over && !was_over {
        run_after = end_season(store, &game, run.now_ms);
    } else if !game.over && run.now_ms - game.saved_ms > SAVE_EVERY_MS {
        let _ = store.save_standings(&game.g, (game.season, game.tick, game.end), &standings(&game), run.now_ms);
        game.saved_ms = run.now_ms;
    }
    let mine = game.colony(pid).map(|c| c.slot as usize);
    let mut notes = Vec::new();
    let mut song_secs = None;
    let status = match (&own, action) {
        (Some((act, Ok(text))), _) => {
            notes = song::cue(act);
            text.clone()
        }
        (Some((_, Err(text))), _) => text.clone(),
        (None, "song") => {
            let (n, secs) = song::forest(&world);
            notes = n;
            song_secs = Some(secs);
            "the undergrowth sings its mushrooms, left to right".into()
        }
        _ => {
            notes = news_cue(&news, mine);
            trouble.map(|t| format!("moves could not be saved ({t}), so a press may be lost")).unwrap_or_else(|| headline(&game, mine))
        }
    };
    let frame = Frame { game: &game, world: &world, status, notes, song_secs, redraw_s: s.redraw_s, tick_ms: s.tick_ms };
    Answer { output: scene::render(&frame), run_after }
}

/// A sound for what happened to the presser's colony while they watched.
fn news_cue(news: &News, mine: Option<usize>) -> Vec<Value> {
    let Some(slot) = mine else { return Vec::new() };
    if news.lost[slot] > 0 {
        song::cue("lost")
    } else if news.took[slot] > 0 || news.spores[slot] > 0 {
        song::cue("took")
    } else {
        Vec::new()
    }
}

fn headline(game: &Game, mine: Option<usize>) -> String {
    if game.over {
        return match game.ranking().first() {
            Some(c) => format!("{} won season {} with {} points", c.name, game.season, c.total()),
            None => format!("season {} is over", game.season),
        };
    }
    if let Some(line) = game.news.first() {
        return line.clone();
    }
    match (mine, game.ranking().first()) {
        (None, _) => "press join to root a colony; tap to lure, long press to fruit".into(),
        (Some(_), Some(lead)) => format!("{} leads with {} points", lead.name, lead.total()),
        (Some(_), None) => "your colony is growing".into(),
    }
}

/// What the almanac and the herald read: each colony's numbers, best first.
pub fn standings(game: &Game) -> String {
    let rows: Vec<Value> = game
        .ranking()
        .iter()
        .take(6)
        .map(|c| json!({"name": c.name, "cells": c.cells, "peak": c.peak, "mushrooms": c.mushrooms, "total": c.total()}))
        .collect();
    json!({"season": game.season, "over": game.over, "leaves": game.leaves, "colonies": rows}).to_string()
}

/// Records a finished season once, however many frames see it end, and asks for the herald.
/// The standings the herald posts are written by the same claim, so they match the records.
fn end_season<H: Host>(store: &Store<H>, game: &Game, now_ms: i64) -> Option<Value> {
    if !store.claim_season_end(&game.g, (game.season, game.tick, game.end), &standings(game), now_ms).ok()? {
        return None;
    }
    let ranking = game.ranking();
    for (rank, c) in ranking.iter().enumerate() {
        let _ = store.record_season(&c.pid, &c.name, (rank == 0 && c.total() > 0, c.peak, c.mushrooms), now_ms);
    }
    if let Some(winner) = ranking.first() {
        let _ = store.set_fact("champion", &winner.pid, i64::from(game.season));
    }
    Some(json!({"command": "herald", "after_ms": 10_000, "key": game.g, "input": game.g}))
}

fn launch<H: Host>(store: &Store<H>, run: &Run, why: Option<&str>) -> Answer {
    let s = run.settings;
    let seed = hash(run.entropy.as_bytes());
    let (w, h) = size_for(&s.world);
    let world = World::generate(w, h, seed);
    let id: String = run.entropy.chars().filter(char::is_ascii_alphanumeric).take(12).collect();
    let game = Game {
        g: if id.is_empty() { format!("{seed:x}") } else { id },
        seed,
        season: 1,
        end: s.season_ticks,
        ms: run.now_ms,
        ib: store.weather_head().unwrap_or(0),
        w,
        h,
        ..Game::default()
    };
    let status = why.unwrap_or("fresh soil; press join to root a colony, then tap to lure it and long press to fruit").to_owned();
    let frame = Frame { game: &game, world: &world, status, notes: Vec::new(), song_secs: None, redraw_s: s.redraw_s, tick_ms: s.tick_ms };
    Answer { output: scene::render(&frame), run_after: None }
}

/// Deals fresh soil for the next season and roots every player of the last one again.
fn new_season((game, world): (&mut Game, &mut World), settings: &Settings, seed: u64, at_ms: i64) {
    let players: Vec<Colony> = game.c.iter().filter(|c| !c.is_wild()).cloned().collect();
    let (w, h) = size_for(&settings.world);
    *world = World::generate(w, h, seed);
    *game = Game {
        g: game.g.clone(),
        seed,
        season: game.season + 1,
        end: settings.season_ticks,
        ms: at_ms,
        mv: game.mv,
        ib: game.ib,
        w,
        h,
        ..Game::default()
    };
    for p in players {
        let rejoin = Move { pid: p.pid.clone(), act: "join".into(), arg: String::new() };
        if moves::apply(game, world, &rejoin).is_ok() {
            let _ = moves::apply(game, world, &Move { pid: p.pid, act: "name".into(), arg: p.name });
        }
    }
    game.note(format!("season {} begins", game.season));
}

/// The private panel, after taking the press into a copy of the board that is never stored.
fn colony_panel<H: Host>(store: &Store<H>, (mut game, mut world): (Game, World), run: &Run, (pid, action): (&str, &str), state: String) -> String {
    let mut link_code = None;
    let mut status = String::new();
    let mut notes = Vec::new();
    if action == "link profile" {
        if game.colony(pid).is_none() {
            status = "join first; a record starts with your first season".into();
        } else {
            let code = format!("ug-{}", base36(mix(&[hash(run.entropy.as_bytes()), hash(pid.as_bytes())]), 6));
            match store.add_link(&code, pid, run.now_ms) {
                Ok(()) => link_code = Some(code),
                Err(why) => status = format!("could not make a link code: {why}"),
            }
        }
    }
    let (own, trouble) = take_moves(store, (&mut game, &mut world), run, move_for(pid, action));
    sim::recount(&mut game, &world);
    if let Some((act, said)) = &own {
        match said {
            Ok(text) => {
                notes = song::cue(act);
                status = text.clone();
            }
            Err(text) => status = text.clone(),
        }
    }
    if status.is_empty() {
        status = trouble.unwrap_or_else(|| "only you can see this panel; back returns to the board".into());
    }
    let record = store.player(pid).ok().flatten();
    panel::render(&Panel { game: &game, world: &world, pid, status, notes, record, link_code, state })
}

fn base36(mut n: u64, len: usize) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    (0..len)
        .map(|_| {
            let d = DIGITS[(n % 36) as usize] as char;
            n /= 36;
            d
        })
        .collect()
}

#[cfg(test)]
#[path = "play_tests.rs"]
mod tests;
