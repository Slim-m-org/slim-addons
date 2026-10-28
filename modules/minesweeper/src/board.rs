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
//!
//! A board is shared by whoever sees the message. It remembers who started it, whether anyone may
//! play or only they may, who has played, and every move in order, so a win can be replayed from
//! the seed before it is written to the scoreboard.

use crate::level::{Level, BEGINNER};
use crate::rng::SplitMix64;

/// How many re-rolls the opening tap may spend looking for a seed whose three-by-three
/// neighbourhood is clear. The bound only stops a pathological search on a crowded custom board.
const OPENING_ATTEMPTS: u32 = 256;

/// The most moves a board keeps; a game that long is not scored, which no honest game reaches.
pub const MAX_MOVES: usize = 2000;
/// The most distinct players one board credits with a win.
pub const MAX_PLAYERS: usize = 8;

/// One effective move, as a cell index: a reveal or a flag toggled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    Dig(usize),
    Flag(usize),
}

/// What the scene shows: the board, or the scoreboard for a level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    Board,
    Scores(Level),
}

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
    /// The caller who started the board, and whether others may dig and flag on it.
    pub host: String,
    pub open: bool,
    /// Callers who have made a move, in order, up to [`MAX_PLAYERS`].
    pub players: Vec<String>,
    pub moves: Vec<Move>,
    /// Set when the log passed [`MAX_MOVES`] and was dropped, so the game cannot be replayed.
    pub unverifiable: bool,
    /// Set once the win has been written to the scoreboard, so a repeated frame writes nothing.
    pub scored: bool,
    pub view: View,
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
            host: String::new(),
            open: true,
            players: Vec::new(),
            moves: Vec::new(),
            unverifiable: false,
            scored: false,
            view: View::Board,
        }
    }

    /// A board started by `host`, who may later close it to everyone else.
    pub fn started_by(seed: u64, level: Level, host: &str) -> Self {
        Board { host: host.to_owned(), ..Board::new(seed, level) }
    }

    /// Rebuilds from the opaque state written last frame:
    /// `3|seed|level|revealed|flagged|flags-mode|dead|start|end|customizing|host|open|players|moves|flags|view`,
    /// the masks as strings of `0`/`1`. A state from before the clock had five parts and is a
    /// beginner board; a 2 state has the first ten and no host, so anyone may play it.
    pub fn from_state(state: &str) -> Self {
        let parts: Vec<&str> = state.split('|').collect();
        if matches!(parts.first(), Some(&"2") | Some(&"3")) {
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
        board.host = parts.get(10).unwrap_or(&"").to_string();
        board.open = parts.get(11) != Some(&"0");
        board.players = split_list(parts.get(12).unwrap_or(&"")).take(MAX_PLAYERS).map(str::to_owned).collect();
        board.moves = split_list(parts.get(13).unwrap_or(&"")).filter_map(Move::read).take(MAX_MOVES).collect();
        let flags = parts.get(14).unwrap_or(&"");
        board.unverifiable = flags.contains('u');
        board.scored = flags.contains('s');
        board.view = parts.get(15).and_then(|v| View::read(v)).unwrap_or(View::Board);
        board
    }

    pub fn state(&self) -> String {
        let time = |t: Option<i64>| t.map_or_else(String::new, |t| t.to_string());
        let moves: Vec<String> = self.moves.iter().map(Move::write).collect();
        let flags = format!("{}{}", if self.unverifiable { "u" } else { "" }, if self.scored { "s" } else { "" });
        format!(
            "3|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            self.seed,
            self.level.code(),
            mask_to(&self.revealed),
            mask_to(&self.flagged),
            u8::from(self.flagging),
            u8::from(self.dead),
            time(self.start_ms),
            time(self.end_ms),
            u8::from(self.customizing),
            self.host,
            u8::from(self.open),
            self.players.join(","),
            moves.join(","),
            flags,
            self.view.write(),
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
        self.log(Move::Dig(index));
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
            self.log(Move::Flag(index));
        }
    }

    fn log(&mut self, mv: Move) {
        if self.moves.len() >= MAX_MOVES {
            self.unverifiable = true;
            self.moves.clear();
        } else if !self.unverifiable {
            self.moves.push(mv);
        }
    }

    /// Notes that `caller` played, so a win credits everyone who took part.
    pub fn note_player(&mut self, caller: &str) {
        if !caller.is_empty() && self.players.len() < MAX_PLAYERS && !self.players.iter().any(|p| p == caller) {
            self.players.push(caller.to_owned());
        }
    }

    /// Whether `caller` may dig and flag: always the host, anyone while the board is open.
    pub fn may_play(&self, caller: &str) -> bool {
        self.open || self.host.is_empty() || self.host == caller
    }

    pub fn is_host(&self, caller: &str) -> bool {
        self.host.is_empty() || self.host == caller
    }

    /// Plays the logged moves again on a fresh board from the same seed and says whether it ends
    /// won, so a time is only written for a game that can be reproduced. A board whose log was
    /// dropped, or has none, cannot be.
    pub fn replays_to_a_win(&self) -> bool {
        if self.unverifiable || self.moves.is_empty() || !self.won() {
            return false;
        }
        let mut again = Board::new(self.seed, self.level);
        for mv in &self.moves {
            match *mv {
                Move::Dig(i) => again.tap(i, 0),
                Move::Flag(i) => again.toggle_flag(i),
            }
        }
        again.won() && again.revealed == self.revealed
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

fn split_list(field: &str) -> impl Iterator<Item = &str> {
    field.split(',').filter(|part| !part.is_empty())
}

impl Move {
    fn write(&self) -> String {
        match self {
            Move::Dig(i) => format!("d{i}"),
            Move::Flag(i) => format!("f{i}"),
        }
    }

    fn read(text: &str) -> Option<Move> {
        let (kind, rest) = text.split_at_checked(1)?;
        let index = rest.parse().ok()?;
        match kind {
            "d" => Some(Move::Dig(index)),
            "f" => Some(Move::Flag(index)),
            _ => None,
        }
    }
}

impl View {
    fn write(&self) -> String {
        match self {
            View::Board => "b".to_owned(),
            View::Scores(level) => format!("s:{}", level.code()),
        }
    }

    fn read(text: &str) -> Option<View> {
        match text.strip_prefix("s:") {
            Some(code) => Level::from_code(code).map(View::Scores),
            None => Some(View::Board),
        }
    }
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
/// A board one move from won, as the state before that move, and the cell that wins it.
pub fn near_win(level: Level, host: &str) -> (Board, usize) {
    let mut board = Board::started_by(4_242, level, host);
    board.prepare_opening(level.cells() / 2);
    board.note_player(host);
    board.tap(level.cells() / 2, 1_000);
    let mines = board.mines();
    for i in (0..level.cells()).filter(|&i| !mines[i]) {
        if board.revealed[i] {
            continue;
        }
        let before = Board::from_state(&board.state());
        board.tap(i, 2_000);
        if board.won() {
            return (before, i);
        }
    }
    panic!("the board was won before its last move");
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
            assert_eq!(parts.len(), 16, "state shape changed: {state}");
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
    fn only_effective_moves_are_logged() {
        let (mut board, mines) = known();
        let safe = first_safe(&mines);
        board.tap(safe, 1);
        let digs = board.moves.len();
        board.tap(safe, 2);
        assert_eq!(board.moves.len(), digs, "a tap on an open cell is not a move");
        let mine = first_mine(&mines);
        board.toggle_flag(mine);
        board.tap(mine, 3);
        assert_eq!(board.moves.len(), digs + 1, "a flag counts and the tap it blocked does not");
        board.toggle_flag(safe);
        assert_eq!(board.moves.len(), digs + 1, "a flag on an open cell is not a move");
    }

    #[test]
    fn a_won_game_replays_to_a_win_and_a_forged_one_does_not() {
        let (mut board, last) = near_win(BEGINNER, "ann");
        board.tap(last, 61_000);
        assert!(board.won() && board.replays_to_a_win());

        let mut forged = Board::from_state(&board.state());
        forged.moves.clear();
        assert!(!forged.replays_to_a_win(), "a win with no moves behind it");
        let mut short = Board::from_state(&board.state());
        short.moves.pop();
        assert!(!short.replays_to_a_win(), "a log that stops one move short of the win");
        let mut other = Board::from_state(&board.state());
        other.seed ^= 1;
        assert!(!other.replays_to_a_win(), "a log played against another field");
    }

    #[test]
    fn a_log_past_its_ceiling_is_dropped_and_the_game_is_never_scored() {
        let mut board = Board::new(1, EXPERT);
        for i in 0..MAX_MOVES + 5 {
            board.toggle_flag(i % 100);
        }
        assert!(board.unverifiable && board.moves.is_empty());
        let again = Board::from_state(&board.state());
        assert!(again.unverifiable);
        assert!(!again.replays_to_a_win());
    }

    #[test]
    fn host_access_players_log_and_view_round_trip() {
        let mut board = Board::started_by(9, EXPERT, "ann");
        board.open = false;
        board.note_player("ann");
        board.note_player("bob");
        board.note_player("bob");
        board.toggle_flag(3);
        board.view = View::Scores(INTERMEDIATE);
        board.scored = true;
        let again = Board::from_state(&board.state());
        assert_eq!((again.host.as_str(), again.open), ("ann", false));
        assert_eq!(again.players, vec!["ann".to_owned(), "bob".to_owned()]);
        assert_eq!(again.moves, vec![Move::Flag(3)]);
        assert_eq!(again.view, View::Scores(INTERMEDIATE));
        assert!(again.scored && !again.unverifiable);
    }

    #[test]
    fn a_state_from_0_4_0_is_open_to_everyone_with_no_host() {
        let old = format!("2|77|beginner|{}|{}|0|0|||0", "0".repeat(81), "0".repeat(81));
        let board = Board::from_state(&old);
        assert!(board.open && board.host.is_empty() && board.players.is_empty());
        assert!(board.may_play("anyone") && board.is_host("anyone"));
    }

    #[test]
    fn a_closed_board_is_the_hosts_alone_and_players_are_capped() {
        let mut board = Board::started_by(1, BEGINNER, "ann");
        board.open = false;
        assert!(board.may_play("ann") && !board.may_play("bob"));
        assert!(board.is_host("ann") && !board.is_host("bob"));
        for n in 0..20 {
            board.note_player(&format!("p{n}"));
        }
        assert_eq!(board.players.len(), MAX_PLAYERS);
        board.note_player("");
        assert_eq!(board.players.len(), MAX_PLAYERS);
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
