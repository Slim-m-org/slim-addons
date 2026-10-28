//! The board: which cells are mined, revealed and flagged, and the clock.
//!
//! **The mines are not in the state, only a seed is.** The field is rebuilt from it on every frame,
//! which keeps the state short: a seed and the revealed/flagged masks, rather than a third grid.
//! It is not secrecy: the seed is in the state every viewer receives, so anyone who reads it can
//! compute the mines. That is acceptable for a cooperative game.
//!
//! The first tap is always safe. The seed is chosen at launch, before any tap, so it cannot be
//! steered by where somebody clicked; the opening tap re-rolls the seed until that cell and its
//! neighbours are clear, which is what every implementation of this game does.

use crate::level::{Level, BEGINNER};
use crate::rng::SplitMix64;

/// How many re-rolls the opening tap may spend looking for a seed whose three-by-three
/// neighbourhood is clear. The bound only stops a pathological search on a crowded custom board.
const OPENING_ATTEMPTS: u32 = 256;

pub struct Board {
    pub level: Level,
    pub seed: u64,
    pub revealed: Vec<bool>,
    pub flagged: Vec<bool>,
    /// Whether taps place flags instead of revealing, for a finger with no right button.
    pub flagging: bool,
    pub dead: bool,
    /// The host's clock at the first reveal, and at the move that ended the game.
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
    /// Whether the custom size field is showing.
    pub customizing: bool,
}

impl Board {
    pub fn new(seed: u64, level: Level) -> Self {
        Board {
            level,
            seed,
            revealed: vec![false; level.cells()],
            flagged: vec![false; level.cells()],
            flagging: false,
            dead: false,
            start_ms: None,
            end_ms: None,
            customizing: false,
        }
    }

    /// Rebuilds from the opaque state written last frame:
    /// `2|seed|level|revealed|flagged|flags-mode|dead|start|end|customizing`, the masks as strings
    /// of `0`/`1`. A state from before levels and the clock had five parts and is a beginner board.
    pub fn from_state(state: &str) -> Self {
        let parts: Vec<&str> = state.split('|').collect();
        if parts.first() == Some(&"2") {
            Self::parse_v2(&parts)
        } else {
            Self::parse_v1(&parts)
        }
    }

    fn parse_v1(parts: &[&str]) -> Self {
        let seed = parts.first().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let mut board = Board::new(seed, BEGINNER);
        board.revealed = mask_from(parts.get(1).unwrap_or(&""), BEGINNER.cells());
        board.flagged = mask_from(parts.get(2).unwrap_or(&""), BEGINNER.cells());
        board.flagging = parts.get(3) == Some(&"1");
        board.dead = parts.get(4) == Some(&"1");
        board
    }

    fn parse_v2(parts: &[&str]) -> Self {
        let level = parts.get(2).and_then(|code| Level::from_code(code)).unwrap_or(BEGINNER);
        let seed = parts.get(1).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let mut board = Board::new(seed, level);
        board.revealed = mask_from(parts.get(3).unwrap_or(&""), level.cells());
        board.flagged = mask_from(parts.get(4).unwrap_or(&""), level.cells());
        board.flagging = parts.get(5) == Some(&"1");
        board.dead = parts.get(6) == Some(&"1");
        board.start_ms = parts.get(7).and_then(|s| s.parse().ok());
        board.end_ms = parts.get(8).and_then(|s| s.parse().ok());
        board.customizing = parts.get(9) == Some(&"1");
        board
    }

    pub fn state(&self) -> String {
        let time = |t: Option<i64>| t.map_or_else(String::new, |t| t.to_string());
        format!(
            "2|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            self.seed,
            self.level.code(),
            mask_to(&self.revealed),
            mask_to(&self.flagged),
            u8::from(self.flagging),
            u8::from(self.dead),
            time(self.start_ms),
            time(self.end_ms),
            u8::from(self.customizing),
        )
    }

    /// Before the opening reveal of a game, re-rolls the seed until `index` and everything around
    /// it is clear, so a first tap opens a space rather than ending the game. Does nothing once
    /// anything is revealed, or while taps are placing flags.
    pub fn prepare_opening(&mut self, index: usize) {
        let untouched = self.revealed.iter().all(|r| !r);
        if index >= self.level.cells() || self.flagging || self.dead || !untouched {
            return;
        }
        for _ in 0..OPENING_ATTEMPTS {
            let mines = self.mines();
            if !mines[index] && self.neighbours(index).into_iter().all(|n| !mines[n]) {
                return;
            }
            self.seed = self.seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        }
    }

    pub fn mines(&self) -> Vec<bool> {
        mines_for(self.seed, self.level)
    }

    /// The cells around `index` that are on the board.
    pub fn neighbours(&self, index: usize) -> Vec<usize> {
        let (cols, rows) = (self.level.cols as isize, self.level.rows as isize);
        let (col, row) = ((index % self.level.cols) as isize, (index / self.level.cols) as isize);
        let mut out = Vec::with_capacity(8);
        for dr in -1isize..=1 {
            for dc in -1isize..=1 {
                let (c, r) = (col + dc, row + dr);
                if (dr, dc) != (0, 0) && c >= 0 && r >= 0 && c < cols && r < rows {
                    out.push(r as usize * self.level.cols + c as usize);
                }
            }
        }
        out
    }

    pub fn adjacent(&self, mines: &[bool], index: usize) -> u8 {
        self.neighbours(index).into_iter().filter(|&n| mines[n]).count() as u8
    }

    pub fn over(&self) -> bool {
        self.dead || self.won()
    }

    /// Reveals `index`; the first reveal starts the clock and the one that ends the game stops it.
    pub fn tap(&mut self, index: usize, now_ms: i64) {
        if index >= self.level.cells() || self.over() {
            return;
        }
        if self.flagging {
            self.toggle_flag(index);
            return;
        }
        // A flag is a deliberate "do not touch": honour it rather than letting a stray tap detonate it.
        if self.flagged[index] || self.revealed[index] {
            return;
        }
        self.start_ms.get_or_insert(now_ms);
        let mines = self.mines();
        if mines[index] {
            self.dead = true;
            self.revealed[index] = true;
        } else {
            self.flood(index, &mines);
        }
        if self.over() {
            self.end_ms = Some(now_ms);
        }
    }

    /// Flags or unflags a hidden cell, whatever the mode: this is what a right click does.
    pub fn toggle_flag(&mut self, index: usize) {
        if index < self.level.cells() && !self.over() && !self.revealed[index] {
            self.flagged[index] = !self.flagged[index];
        }
    }

    /// Reveals `index` and keeps going through any neighbour that also touches no mine.
    fn flood(&mut self, index: usize, mines: &[bool]) {
        let mut stack = vec![index];
        while let Some(current) = stack.pop() {
            if self.revealed[current] || self.flagged[current] || mines[current] {
                continue;
            }
            self.revealed[current] = true;
            if self.adjacent(mines, current) == 0 {
                stack.extend(self.neighbours(current).into_iter().filter(|&n| !self.revealed[n]));
            }
        }
    }

    /// Won when every cell that is not a mine has been revealed. Flags are deliberately not part
    /// of it: a player who cleared the board without marking a mine has still cleared the board.
    pub fn won(&self) -> bool {
        let mines = self.mines();
        !self.dead && (0..self.level.cells()).all(|i| mines[i] || self.revealed[i])
    }

    pub fn flags_left(&self) -> i64 {
        self.level.mines as i64 - self.flagged.iter().filter(|f| **f).count() as i64
    }

    /// How long the game has run: to the last move once it is over, to `now_ms` while it is on.
    pub fn elapsed_ms(&self, now_ms: i64) -> Option<i64> {
        let start = self.start_ms?;
        Some((self.end_ms.unwrap_or(now_ms) - start).max(0))
    }
}

/// The mine field for a seed. Deterministic, so every frame rebuilds exactly the same board
/// without carrying it in the state.
pub fn mines_for(seed: u64, level: Level) -> Vec<bool> {
    let mut rng = SplitMix64::from_seed(seed);
    let mut mines = vec![false; level.cells()];
    let mut placed = 0;
    while placed < level.mines {
        let at = (rng.next_u64() % level.cells() as u64) as usize;
        if !mines[at] {
            mines[at] = true;
            placed += 1;
        }
    }
    mines
}

fn mask_to(mask: &[bool]) -> String {
    mask.iter().map(|b| if *b { '1' } else { '0' }).collect()
}

fn mask_from(s: &str, cells: usize) -> Vec<bool> {
    let mut mask = vec![false; cells];
    for (i, c) in s.chars().take(cells).enumerate() {
        mask[i] = c == '1';
    }
    mask
}

/// `m:ss`, or `h:mm:ss` past an hour.
pub fn clock(ms: i64) -> String {
    let total = (ms / 1000).max(0);
    let (h, m, s) = (total / 3600, (total / 60) % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{}:{s:02}", total / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::{custom, EXPERT, INTERMEDIATE};

    const LEVELS: [Level; 3] = [BEGINNER, INTERMEDIATE, EXPERT];

    fn known() -> (Board, Vec<bool>) {
        let board = Board::new(12_345, BEGINNER);
        let mines = board.mines();
        (board, mines)
    }

    fn first_mine(mines: &[bool]) -> usize {
        mines.iter().position(|m| *m).expect("a field has mines")
    }

    fn first_safe(mines: &[bool]) -> usize {
        mines.iter().position(|m| !*m).expect("a field has blanks")
    }

    #[test]
    fn a_field_has_exactly_its_levels_mines() {
        for level in LEVELS.into_iter().chain(custom(20, 12, 30)) {
            for seed in [0u64, 1, 12_345, u64::MAX] {
                let count = mines_for(seed, level).iter().filter(|m| **m).count();
                assert_eq!(count, level.mines, "seed {seed} on {level:?}");
            }
        }
    }

    #[test]
    fn the_beginner_field_for_a_seed_is_the_one_every_earlier_release_dealt() {
        let mines = mines_for(12_345, BEGINNER);
        let held: Vec<usize> = (0..81).filter(|&i| mines[i]).collect();
        let mut rng = SplitMix64::from_seed(12_345);
        let mut expect = Vec::new();
        while expect.len() < 10 {
            let at = (rng.next_u64() % 81) as usize;
            if !expect.contains(&at) {
                expect.push(at);
            }
        }
        expect.sort_unstable();
        assert_eq!(held, expect, "a board in flight must not change under a player");
    }

    #[test]
    fn the_same_seed_gives_the_same_field_and_another_gives_another() {
        assert_eq!(mines_for(999, EXPERT), mines_for(999, EXPERT));
        assert_ne!(mines_for(1, BEGINNER), mines_for(2, BEGINNER));
    }

    #[test]
    fn the_state_is_a_short_seed_and_never_the_mine_grid() {
        for level in LEVELS {
            let mut board = Board::new(12_345, level);
            let mines = board.mines();
            board.tap(first_safe(&mines), 1_000);
            let state = board.state();
            assert!(!state.contains(&mask_to(&mines)), "the mine field leaked into the state");
            let parts: Vec<&str> = state.split('|').collect();
            assert_eq!(parts.len(), 10, "state shape changed: {state}");
            assert_eq!(parts[3].len(), level.cells(), "revealed mask is one bit per cell");
            assert_eq!(parts[4].len(), level.cells(), "flagged mask is one bit per cell");
        }
    }

    #[test]
    fn state_round_trips_with_its_level_and_clock() {
        let (_, mines) = known();
        for level in [BEGINNER, EXPERT, custom(20, 12, 30).unwrap()] {
            let mut board = Board::new(77, level);
            let mines_here = board.mines();
            board.tap(first_safe(&mines_here), 5_000);
            board.flagging = true;
            board.tap(first_mine(&mines_here), 6_000);
            board.customizing = true;
            let again = Board::from_state(&board.state());
            assert_eq!(again.level, board.level);
            assert_eq!(again.seed, board.seed);
            assert_eq!(again.revealed, board.revealed);
            assert_eq!(again.flagged, board.flagged);
            assert_eq!((again.flagging, again.customizing), (true, true));
            assert_eq!((again.start_ms, again.end_ms), (Some(5_000), None));
        }
        let _ = mines;
    }

    #[test]
    fn a_state_from_before_levels_and_the_clock_is_a_beginner_board_with_no_clock() {
        let old = format!("{}|{}|{}|1|0", 4242, "0".repeat(81), "0".repeat(81));
        let board = Board::from_state(&old);
        assert_eq!(board.level, BEGINNER);
        assert!(board.flagging && !board.dead);
        assert_eq!((board.start_ms, board.end_ms), (None, None));
        assert_eq!(board.seed, 4242);
    }

    #[test]
    fn a_garbled_state_still_makes_a_board() {
        for junk in ["", "2", "2|x|nope|1|1", "||||", "2|1|9x9x10|11"] {
            let board = Board::from_state(junk);
            assert_eq!(board.revealed.len(), board.level.cells(), "{junk:?}");
        }
    }

    #[test]
    fn tapping_a_mine_ends_the_game_and_a_dead_board_takes_no_more_taps() {
        let (mut board, mines) = known();
        board.tap(first_mine(&mines), 1_000);
        assert!(board.dead);
        let before = board.revealed.clone();
        board.tap(first_safe(&mines), 2_000);
        assert_eq!(board.revealed, before);
    }

    #[test]
    fn a_flagged_cell_cannot_be_detonated_by_a_stray_tap() {
        let (mut board, mines) = known();
        let mine = first_mine(&mines);
        board.toggle_flag(mine);
        board.tap(mine, 1_000);
        assert!(!board.dead, "a flag is a do-not-touch, not a decoration");
    }

    #[test]
    fn flag_mode_flags_on_a_tap_and_a_revealed_cell_cannot_be_flagged() {
        let (mut board, mines) = known();
        let safe = first_safe(&mines);
        board.tap(safe, 1_000);
        board.toggle_flag(safe);
        assert!(!board.flagged[safe]);
        board.flagging = true;
        let other = (0..81).find(|&i| !board.revealed[i]).unwrap();
        board.tap(other, 2_000);
        assert!(board.flagged[other]);
        board.tap(other, 3_000);
        assert!(!board.flagged[other], "the same tap takes it back off");
    }

    #[test]
    fn revealing_a_blank_cascades_and_never_onto_a_mine() {
        for level in LEVELS {
            for seed in 0..100u64 {
                let mut board = Board::new(seed, level);
                let mines = board.mines();
                if let Some(blank) = (0..level.cells()).find(|&i| !mines[i] && board.adjacent(&mines, i) == 0) {
                    board.tap(blank, 1);
                    assert!(board.revealed.iter().filter(|r| **r).count() > 1, "an empty cell opens its neighbourhood");
                    assert!((0..level.cells()).all(|i| !(mines[i] && board.revealed[i])), "seed {seed} cascaded onto a mine");
                }
            }
        }
    }

    #[test]
    fn an_opening_tap_is_always_safe_and_opens_a_space() {
        for level in LEVELS.into_iter().chain(custom(8, 8, 22)) {
            for seed in 0..40u64 {
                for index in [0, level.cols - 1, level.cells() / 2, level.cells() - 1] {
                    let mut board = Board::new(seed, level);
                    board.prepare_opening(index);
                    board.tap(index, 1);
                    assert!(!board.dead, "seed {seed} killed an opening tap on cell {index} of {level:?}");
                    assert!(board.revealed.iter().filter(|r| **r).count() > 1 || board.won());
                }
            }
        }
    }

    #[test]
    fn the_opening_is_only_rerolled_before_anything_is_revealed() {
        let mut board = Board::new(5, BEGINNER);
        board.prepare_opening(40);
        board.tap(40, 1);
        let seed = board.seed;
        board.prepare_opening(0);
        assert_eq!(board.seed, seed, "a seed changed under a game in progress");
    }

    #[test]
    fn clearing_every_safe_cell_wins_and_a_won_board_takes_no_more_taps() {
        let mut board = Board::new(4_242, BEGINNER);
        let mines = board.mines();
        for i in 0..81 {
            board.revealed[i] = !mines[i];
        }
        assert!(board.won() && !board.dead);
        board.tap(first_mine(&mines), 1);
        assert!(!board.dead, "the game was already over");
    }

    #[test]
    fn the_clock_starts_on_the_first_reveal_not_on_a_flag() {
        let (mut board, mines) = known();
        board.toggle_flag(first_mine(&mines));
        assert_eq!(board.elapsed_ms(9_000), None);
        board.tap(first_safe(&mines), 10_000);
        assert_eq!(board.start_ms, Some(10_000));
        assert_eq!(board.elapsed_ms(14_500), Some(4_500));
        board.tap(first_safe(&mines), 20_000);
        assert_eq!(board.start_ms, Some(10_000), "a later reveal does not restart it");
    }

    #[test]
    fn the_clock_stops_at_the_move_that_ends_the_game() {
        let (mut board, mines) = known();
        board.tap(first_safe(&mines), 1_000);
        board.tap(first_mine(&mines), 8_000);
        assert!(board.dead);
        assert_eq!(board.end_ms, Some(8_000));
        assert_eq!(board.elapsed_ms(999_999), Some(7_000));
    }

    #[test]
    fn the_clock_stops_when_the_last_safe_cell_opens() {
        let mut board = Board::new(4_242, BEGINNER);
        let mines = board.mines();
        let last = (0..81).rev().find(|&i| !mines[i]).unwrap();
        for i in 0..81 {
            board.revealed[i] = !mines[i] && i != last;
        }
        board.start_ms = Some(1_000);
        board.tap(last, 61_000);
        assert!(board.won());
        assert_eq!(board.end_ms, Some(61_000));
        assert_eq!(board.elapsed_ms(1_000_000), Some(60_000));
    }

    #[test]
    fn a_clock_reads_minutes_and_seconds_and_hours_when_it_must() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(59_999), "0:59");
        assert_eq!(clock(61_000), "1:01");
        assert_eq!(clock(3_599_000), "59:59");
        assert_eq!(clock(3_661_000), "1:01:01");
        assert_eq!(clock(-5), "0:00");
    }
}
