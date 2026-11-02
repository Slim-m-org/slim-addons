//! The daily puzzle: the word for a UTC day, and each player's own board for it, kept in the
//! module's database because the module only ever sees an opaque id for whoever is playing.

use std::cell::Cell;

use serde_json::{json, Value};

use crate::host::Host;
use crate::words::WORDS;

pub const DAY_MS: i64 = 86_400_000;
/// Host calls one run may spend on data.store; the host allows 64 with kv.store.
const BUDGET: u32 = 64;
/// How many past wins a streak looks back over.
const STREAK_LOOKBACK: i64 = 60;
/// The `day` a practice row carries; a streak never reaches back this far.
const PRACTICE_DAY: i64 = -1;

/// The UTC day number a clock reading falls on.
pub fn day_of(now_ms: i64) -> i64 {
    now_ms.div_euclid(DAY_MS)
}

/// The word for `day`, the same for everyone. A multiply-xor-shift, so neighbouring days do not
/// give neighbouring words; a plain modulo would walk the list one entry at a time.
pub fn answer_for(day: i64) -> &'static str {
    let mixed = (day as u64).wrapping_mul(6_364_136_223_846_793_005) ^ 0x9E37_79B9_7F4A_7C15;
    WORDS[(mixed >> 17) as usize % WORDS.len()]
}

/// One player's stored board for one day.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stored {
    pub guesses: Vec<String>,
    pub done: bool,
    pub won: bool,
    /// The practice board's seed; 0 for a daily board, whose word comes from the date.
    pub seed: u64,
}

pub struct Plays<'a, H: Host> {
    host: &'a H,
    used: Cell<u32>,
}

impl<'a, H: Host> Plays<'a, H> {
    pub fn new(host: &'a H) -> Self {
        Plays { host, used: Cell::new(0) }
    }

    fn data(&self, op: &str, rest: Value) -> Result<Value, String> {
        if self.used.get() >= BUDGET {
            return Err("this run used all its data calls".to_owned());
        }
        self.used.set(self.used.get() + 1);
        let mut request = json!({"capability": "data.store", "op": op, "table": "plays"});
        if let (Some(base), Some(extra)) = (request.as_object_mut(), rest.as_object()) {
            base.extend(extra.clone());
        }
        self.host.call(&request).map_err(|e| explain(&e))
    }

    pub fn load(&self, day: i64, player: &str) -> Result<Stored, String> {
        self.load_key(&format!("{day}|{player}"))
    }

    pub fn save(&self, day: i64, player: &str, stored: &Stored) -> Result<(), String> {
        self.save_key(&format!("{day}|{player}"), day, player, stored)
    }

    /// The player's one practice board, kept beside the daily ones under a day that no date can be.
    pub fn load_practice(&self, player: &str) -> Result<Stored, String> {
        self.load_key(&format!("p|{player}"))
    }

    pub fn save_practice(&self, player: &str, stored: &Stored) -> Result<(), String> {
        self.save_key(&format!("p|{player}"), PRACTICE_DAY, player, stored)
    }

    fn load_key(&self, key: &str) -> Result<Stored, String> {
        let reply = self.data("get", json!({"where": [{"col": "k", "op": "eq", "val": key}]}))?;
        let row = &reply["rows"][0];
        Ok(Stored {
            guesses: row["guesses"].as_str().unwrap_or("").split(',').filter(|g| !g.is_empty()).map(str::to_owned).collect(),
            done: row["done"].as_i64() == Some(1),
            won: row["won"].as_i64() == Some(1),
            seed: row["seed"].as_i64().unwrap_or(0) as u64,
        })
    }

    fn save_key(&self, key: &str, day: i64, player: &str, stored: &Stored) -> Result<(), String> {
        let values = json!({
            "k": key, "day": day, "player": player, "guesses": stored.guesses.join(","),
            "done": i64::from(stored.done), "won": i64::from(stored.won), "seed": stored.seed as i64,
        });
        self.data("upsert", json!({"values": values, "on": "k"})).map(|_| ())
    }

    /// Days in a row the player won, counting back from today, or from yesterday while today is still open.
    pub fn streak(&self, player: &str, today: i64) -> Result<i64, String> {
        let reply = self.data("query", json!({
            "where": [{"col": "player", "op": "eq", "val": player}, {"col": "won", "op": "eq", "val": 1}],
            "order": [{"col": "day", "dir": "desc"}], "limit": STREAK_LOOKBACK,
        }))?;
        let days: Vec<i64> = reply["rows"].as_array().map(Vec::as_slice).unwrap_or(&[]).iter().filter_map(|r| r["day"].as_i64()).collect();
        Ok(streak_from(&days, today))
    }
}

/// `days` are won days, newest first.
pub fn streak_from(days: &[i64], today: i64) -> i64 {
    let mut expect = match days.first() {
        Some(&first) if first == today || first == today - 1 => first,
        _ => return 0,
    };
    let mut run = 0;
    for &day in days {
        if day != expect {
            break;
        }
        run += 1;
        expect -= 1;
    }
    run
}

/// Host refusals in words the admin reading the Dock can act on.
pub fn explain(host_error: &str) -> String {
    if host_error.contains("not approved") {
        "An admin needs to approve Word Guess to keep each player's board (stored data) in the Dock.".to_owned()
    } else {
        format!("Word Guess could not reach slim ({host_error})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeHost;

    #[test]
    fn a_day_is_a_utc_day_and_the_word_follows_it() {
        assert_eq!(day_of(0), 0);
        assert_eq!(day_of(DAY_MS - 1), 0);
        assert_eq!(day_of(DAY_MS), 1);
        assert_eq!(day_of(-1), -1);
        assert_eq!(answer_for(20_000), answer_for(20_000));
        assert_ne!(answer_for(20_000), answer_for(20_001));
    }

    #[test]
    fn the_word_for_a_day_is_always_one_of_the_list() {
        for day in [i64::MIN, -5, 0, 1, 20_000, i64::MAX] {
            assert!(WORDS.contains(&answer_for(day)));
        }
    }

    #[test]
    fn a_board_is_stored_per_day_and_per_player_and_read_back() {
        let host = FakeHost::default();
        let plays = Plays::new(&host);
        assert_eq!(plays.load(5, "ann").unwrap(), Stored::default());
        let stored = Stored { guesses: vec!["crane".into(), "slate".into()], done: false, won: false, seed: 0 };
        plays.save(5, "ann", &stored).unwrap();
        assert_eq!(plays.load(5, "ann").unwrap(), stored);
        assert_eq!(plays.load(6, "ann").unwrap(), Stored::default(), "tomorrow is a new board");
        assert_eq!(plays.load(5, "bob").unwrap(), Stored::default(), "and another player's");
        let done = Stored { guesses: vec!["crane".into()], done: true, won: true, seed: 0 };
        plays.save(5, "ann", &done).unwrap();
        assert_eq!(plays.load(5, "ann").unwrap(), done);
        assert_eq!(host.plays.borrow().len(), 1, "saving again replaces the row");
    }

    #[test]
    fn a_practice_board_is_kept_apart_from_the_days_and_never_feeds_a_streak() {
        let host = FakeHost::default();
        let plays = Plays::new(&host);
        let practice = Stored { guesses: vec!["crane".into()], done: true, won: true, seed: 99 };
        plays.save_practice("ann", &practice).unwrap();
        assert_eq!(plays.load_practice("ann").unwrap(), practice);
        assert_eq!(plays.load(5, "ann").unwrap(), Stored::default());
        assert_eq!(plays.streak("ann", 5).unwrap(), 0);
        assert_eq!(plays.load_practice("bob").unwrap(), Stored::default());
    }

    #[test]
    fn a_streak_counts_consecutive_won_days_and_survives_an_open_today() {
        assert_eq!(streak_from(&[], 10), 0);
        assert_eq!(streak_from(&[10, 9, 8], 10), 3);
        assert_eq!(streak_from(&[9, 8], 10), 2, "today is still open");
        assert_eq!(streak_from(&[10, 8, 7], 10), 1, "a missed day ends it");
        assert_eq!(streak_from(&[7, 6], 10), 0, "nothing for two days");
    }

    #[test]
    fn the_streak_reads_the_players_wins_newest_first() {
        let host = FakeHost::default();
        let plays = Plays::new(&host);
        for (day, won) in [(8, true), (9, true), (10, false), (11, true)] {
            plays.save(day, "ann", &Stored { guesses: vec!["crane".into()], done: true, won, seed: 0 }).unwrap();
        }
        plays.save(11, "bob", &Stored { guesses: vec![], done: true, won: true, seed: 0 }).unwrap();
        assert_eq!(plays.streak("ann", 11).unwrap(), 1);
        assert_eq!(plays.streak("zed", 11).unwrap(), 0);
        let early = FakeHost::default();
        let plays = Plays::new(&early);
        for day in [7, 8, 9] {
            plays.save(day, "ann", &Stored { guesses: vec!["crane".into()], done: true, won: true, seed: 0 }).unwrap();
        }
        assert_eq!(plays.streak("ann", 9).unwrap(), 3);
        assert_eq!(plays.streak("ann", 10).unwrap(), 3, "today is still open");
        assert_eq!(plays.streak("ann", 11).unwrap(), 0);
    }

    #[test]
    fn a_refused_capability_is_explained_for_the_admin() {
        let host = FakeHost::default();
        host.refuse.set(true);
        let why = Plays::new(&host).load(1, "ann").unwrap_err();
        assert!(why.contains("An admin needs to approve"), "{why}");
    }
}
