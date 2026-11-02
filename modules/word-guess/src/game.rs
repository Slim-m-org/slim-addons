//! One board: an answer, the guesses so far, and what colour each letter of a guess earns.

pub const LENGTH: usize = 5;
pub const TRIES: usize = 6;

/// What a letter turned out to be. Plain text, not the colour: `scene.rs` decides how each draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Right,
    Moved,
    Absent,
}

pub struct Game {
    pub answer: &'static str,
    pub guesses: Vec<String>,
    /// Set when a guess could not be used, so the person is told why rather than watching their typing vanish.
    pub refused: Option<String>,
}

impl Game {
    pub fn new(answer: &'static str) -> Self {
        Game { answer, guesses: Vec::new(), refused: None }
    }

    /// A board rebuilt from stored guesses; anything that is not a usable guess is left out.
    pub fn with_guesses<'a>(answer: &'static str, guesses: impl IntoIterator<Item = &'a str>) -> Self {
        let kept = guesses.into_iter().filter(|g| usable(g)).take(TRIES).map(str::to_owned).collect();
        Game { answer, guesses: kept, refused: None }
    }

    pub fn solved(&self) -> bool {
        self.guesses.last().is_some_and(|g| g == self.answer)
    }

    pub fn over(&self) -> bool {
        self.solved() || self.guesses.len() >= TRIES
    }

    /// Takes a guess; false, with the reason in `refused`, when it cannot be used.
    pub fn guess(&mut self, raw: &str) -> bool {
        if self.over() {
            return false;
        }
        let word = raw.trim().to_lowercase();
        if !usable(&word) {
            self.refused = Some(format!("{LENGTH} letters, a to z"));
            return false;
        }
        if self.guesses.iter().any(|g| *g == word) {
            self.refused = Some("already guessed".to_owned());
            return false;
        }
        self.refused = None;
        self.guesses.push(word);
        true
    }
}

fn usable(word: &str) -> bool {
    word.len() == LENGTH && word.chars().all(|c| c.is_ascii_lowercase())
}

/// Which mark each letter of `guess` earns against `answer`.
///
/// Two passes, because one is wrong in a way that shows: a letter is only "moved" if a copy of it
/// is still unaccounted for after every exact match has claimed its own. Marking greedily left to
/// right reports two ambers for a doubled letter that appears once.
pub fn score(guess: &str, answer: &str) -> Vec<Mark> {
    let guess: Vec<char> = guess.chars().collect();
    let answer: Vec<char> = answer.chars().collect();
    let mut marks = vec![Mark::Absent; guess.len()];
    let mut claimed = vec![false; answer.len()];

    for i in 0..guess.len() {
        if answer.get(i) == Some(&guess[i]) {
            marks[i] = Mark::Right;
            claimed[i] = true;
        }
    }
    for i in 0..guess.len() {
        if marks[i] == Mark::Right {
            continue;
        }
        if let Some(j) = (0..answer.len()).find(|&j| !claimed[j] && answer[j] == guess[i]) {
            marks[i] = Mark::Moved;
            claimed[j] = true;
        }
    }
    marks
}

#[cfg(test)]
mod tests {
    use super::*;
    use Mark::{Absent, Moved, Right};

    #[test]
    fn a_doubled_letter_is_only_amber_when_a_copy_is_unclaimed() {
        assert_eq!(score("speed", "abide"), vec![Absent, Absent, Moved, Absent, Moved]);
        assert_eq!(score("sheep", "melee"), vec![Absent, Absent, Moved, Right, Absent]);
        assert_eq!(score("crane", "crane"), vec![Right; 5]);
    }

    #[test]
    fn a_guess_must_be_five_letters_a_to_z_and_new() {
        let mut game = Game::new("crane");
        assert!(!game.guess("abc"));
        assert!(!game.guess("abcd1"));
        assert!(game.guesses.is_empty() && game.refused.is_some());
        assert!(game.guess("Slate"));
        assert!(!game.guess("slate"));
        assert_eq!(game.guesses, vec!["slate"]);
        assert_eq!(game.refused.as_deref(), Some("already guessed"));
    }

    #[test]
    fn the_game_ends_when_solved_or_out_of_tries_and_takes_no_more() {
        let mut won = Game::new("crane");
        won.guess("crane");
        assert!(won.solved() && won.over());
        assert!(!won.guess("slate"));

        let mut lost = Game::new("crane");
        for w in ["aaaaa", "bbbbb", "ddddd", "eeeee", "fffff", "ggggg", "hhhhh"] {
            lost.guess(w);
        }
        assert_eq!(lost.guesses.len(), TRIES);
        assert!(lost.over() && !lost.solved());
    }

    #[test]
    fn rebuilding_from_stored_guesses_drops_the_unusable_and_caps_the_tries() {
        let game = Game::with_guesses("crane", ["slate", "TOOLONG", "ab", "crate"]);
        assert_eq!(game.guesses, vec!["slate", "crate"]);
        let many = Game::with_guesses("crane", ["aaaaa", "bbbbb", "ccccc", "ddddd", "eeeee", "fffff", "ggggg"]);
        assert_eq!(many.guesses.len(), TRIES);
    }
}
