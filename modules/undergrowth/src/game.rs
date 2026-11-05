//! One running season: the header that rides beside the soil in the scene's `state`.
//!
//! The state is `<header json>|<soil base64>`. On a shared surface slim hands back the state
//! of the last frame it stored, so this is trusted as far as this module wrote it, and still
//! parsed defensively because a test, an older frame or a broken build can send anything.

use serde::{Deserialize, Serialize};

use crate::colony::Colony;
use crate::world::{valid_size, World};

/// The most a state string may be before it is not read at all.
const MAX_STATE: usize = 60_000;

/// Something worth animating in the next frame: a leaf falling, rain, a spore flying, a fruiting.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Fx {
    /// `l` leaf (a = column), `r` rain (a = column), `s` spore (a -> b, cell indices),
    /// `f` fruiting (a = cell), `w` the wild rot stirring (a = cell).
    pub k: char,
    pub a: u32,
    #[serde(default)]
    pub b: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Game {
    /// This surface's own id, minted at launch so two games in one space keep their moves apart.
    pub g: String,
    pub seed: u64,
    pub season: u32,
    pub tick: u32,
    /// The tick the season ends on.
    pub end: u32,
    /// The clock the world has been advanced to, in host milliseconds.
    pub ms: i64,
    /// The last move and inbox rows this world has taken in.
    pub mv: i64,
    pub ib: i64,
    pub w: usize,
    pub h: usize,
    /// Ticks of warm spell left (from time people spent in calls).
    #[serde(default)]
    pub warm: u32,
    /// Rain bands as (column, ticks left).
    #[serde(default)]
    pub rain: Vec<(u32, u32)>,
    #[serde(default)]
    pub fx: Vec<Fx>,
    #[serde(default)]
    pub c: Vec<Colony>,
    #[serde(default)]
    pub over: bool,
    /// Leaves fallen this season, for the status line.
    #[serde(default)]
    pub leaves: u32,
    /// The last few things that happened, newest first.
    #[serde(default)]
    pub news: Vec<String>,
    /// When the standings were last written for the almanac.
    #[serde(default)]
    pub saved_ms: i64,
}

impl Game {
    pub fn encode(&self, world: &World) -> String {
        let header = serde_json::to_string(self).unwrap_or_default();
        format!("{header}|{}", world.encode())
    }

    /// The game and soil a state carries, or `None` for anything this module would not have written.
    pub fn decode(state: &str) -> Option<(Game, World)> {
        if state.len() > MAX_STATE {
            return None;
        }
        let (header, soil) = state.rsplit_once('|')?;
        let game: Game = serde_json::from_str(header).ok()?;
        if !valid_size(game.w, game.h) || game.c.len() > 7 || game.g.is_empty() {
            return None;
        }
        let world = World::decode(game.w, game.h, soil)?;
        let cells = world.cells.len() as u32;
        let sane = game.c.iter().all(|c| {
            c.heart < cells && (1..=7).contains(&c.slot) && c.waypoints.len() <= 8 && c.waypoints.iter().all(|w| *w < cells)
        });
        sane.then_some((game, world))
    }

    pub fn colony(&self, pid: &str) -> Option<&Colony> {
        self.c.iter().find(|c| c.pid == pid)
    }

    pub fn colony_mut(&mut self, pid: &str) -> Option<&mut Colony> {
        self.c.iter_mut().find(|c| c.pid == pid)
    }

    /// Colonies ranked best first, players only.
    pub fn ranking(&self) -> Vec<&Colony> {
        let mut players: Vec<&Colony> = self.c.iter().filter(|c| !c.is_wild()).collect();
        players.sort_by(|a, b| b.total().cmp(&a.total()).then(a.slot.cmp(&b.slot)));
        players
    }

    pub fn note(&mut self, line: String) {
        self.news.insert(0, line);
        self.news.truncate(3);
    }

    pub fn ticks_left(&self) -> u32 {
        self.end.saturating_sub(self.tick)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> (Game, World) {
        let world = World::generate(48, 30, 5);
        let game = Game {
            g: "abc".into(),
            seed: 5,
            season: 1,
            w: 48,
            h: 30,
            end: 100,
            c: vec![Colony { pid: "p1".into(), name: "Moss | Cap".into(), slot: 1, heart: 200, ..Colony::default() }],
            ..Game::default()
        };
        (game, world)
    }

    #[test]
    fn a_state_round_trips_even_with_a_bar_in_a_name() {
        let (game, world) = sample();
        let text = game.encode(&world);
        assert_eq!(Game::decode(&text), Some((game, world)));
    }

    #[test]
    fn hostile_states_are_refused_not_trusted() {
        let (mut game, world) = sample();
        assert!(Game::decode("").is_none());
        assert!(Game::decode("|||").is_none());
        assert!(Game::decode(&"x".repeat(70_000)).is_none());
        game.c[0].heart = 999_999;
        assert!(Game::decode(&game.encode(&world)).is_none());
        let (mut game, world) = sample();
        game.w = 4000;
        assert!(Game::decode(&game.encode(&world)).is_none());
        let (mut game, world) = sample();
        game.c[0].slot = 0;
        assert!(Game::decode(&game.encode(&world)).is_none());
    }

    #[test]
    fn news_keeps_the_three_newest() {
        let (mut game, _) = sample();
        for n in 0..5 {
            game.note(format!("n{n}"));
        }
        assert_eq!(game.news, vec!["n4", "n3", "n2"]);
    }
}
