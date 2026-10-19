//! A poll and who has voted in it. A module keeps nothing between frames, so
//! the question, the options and one row per voter all ride in the scene state.
//!
//! Tallies are never stored, only counted from the voter rows, so one person
//! can hold at most one vote by construction. The state is echoed by the
//! client, so this stops accidents and casual padding but not a member who
//! forges the state string.

pub const FIELD: char = '\u{1e}';
pub const ITEM: char = '\u{1f}';
pub const ENTRY: char = '\u{1d}';

pub const MAX_VOTERS: usize = 200;

/// Longest caller id accepted; the host sends a 64-character hex digest.
const MAX_ID: usize = 64;

#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    Anonymous,
    Closed,
    NoSuchOption,
    Full,
}

impl Refusal {
    pub fn note(&self) -> &'static str {
        match self {
            Refusal::Anonymous => "this server did not say who you are",
            Refusal::Closed => "poll is closed",
            Refusal::NoSuchOption => "no such option",
            Refusal::Full => "this poll has all the voters it can hold",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Cast {
    New,
    Moved,
    Unchanged,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Poll {
    pub open: bool,
    pub question: String,
    pub options: Vec<String>,
    votes: Vec<(String, usize)>,
}

/// An id is lowercase hex, which also keeps it from breaking the state's framing.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID && id.bytes().all(|b| b.is_ascii_hexdigit())
}

impl Poll {
    pub fn new(question: String, options: Vec<String>) -> Self {
        Poll {
            open: true,
            question,
            options,
            votes: Vec::new(),
        }
    }

    pub fn parse(state: &str) -> Option<Self> {
        let mut fields = state.split(FIELD);
        let open = fields.next()? == "1";
        let question = fields.next()?.to_string();
        let options: Vec<String> = fields
            .next()
            .unwrap_or("")
            .split(ENTRY)
            .filter(|entry| !entry.is_empty())
            .map(|entry| entry.split(ITEM).next().unwrap_or("").to_string())
            .collect();
        if question.is_empty() || options.len() < 2 {
            return None;
        }
        let mut poll = Poll {
            open,
            question,
            options,
            votes: Vec::new(),
        };
        poll.votes = poll.read_voters(fields.next().unwrap_or(""));
        Some(poll)
    }

    /// Rows that name a bad id or a missing option are dropped, and a repeated id keeps its first row.
    fn read_voters(&self, blob: &str) -> Vec<(String, usize)> {
        let mut rows: Vec<(String, usize)> = Vec::new();
        for entry in blob.split(ENTRY).take(MAX_VOTERS) {
            let Some((id, option)) = entry.split_once(ITEM) else {
                continue;
            };
            let Ok(option) = option.parse::<usize>() else {
                continue;
            };
            if valid_id(id)
                && option < self.options.len()
                && rows.iter().all(|(seen, _)| seen != id)
            {
                rows.push((id.to_string(), option));
            }
        }
        rows
    }

    pub fn state(&self) -> String {
        let join = |parts: Vec<String>| parts.join(&ENTRY.to_string());
        let options = join(self.options.clone());
        let voters = join(
            self.votes
                .iter()
                .map(|(id, option)| format!("{id}{ITEM}{option}"))
                .collect(),
        );
        let open = if self.open { "1" } else { "0" };
        format!(
            "{open}{FIELD}{}{FIELD}{options}{FIELD}{voters}",
            self.question
        )
    }

    pub fn tally(&self, option: usize) -> usize {
        self.votes.iter().filter(|(_, o)| *o == option).count()
    }

    pub fn total(&self) -> usize {
        self.votes.len()
    }

    /// Records `caller`'s vote for `option`, moving it if they had voted before.
    pub fn cast(&mut self, caller: &str, option: usize) -> Result<Cast, Refusal> {
        if !valid_id(caller) {
            return Err(Refusal::Anonymous);
        }
        if !self.open {
            return Err(Refusal::Closed);
        }
        if option >= self.options.len() {
            return Err(Refusal::NoSuchOption);
        }
        if let Some(row) = self.votes.iter_mut().find(|(id, _)| id == caller) {
            if row.1 == option {
                return Ok(Cast::Unchanged);
            }
            row.1 = option;
            return Ok(Cast::Moved);
        }
        if self.votes.len() >= MAX_VOTERS {
            return Err(Refusal::Full);
        }
        self.votes.push((caller.to_string(), option));
        Ok(Cast::New)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "aa";
    const B: &str = "bb";

    fn poll() -> Poll {
        Poll::new("q?".into(), vec!["x".into(), "y".into(), "z".into()])
    }

    #[test]
    fn a_vote_is_counted() {
        let mut p = poll();
        assert_eq!(p.cast(A, 1), Ok(Cast::New));
        assert_eq!((p.tally(0), p.tally(1), p.total()), (0, 1, 1));
    }

    #[test]
    fn voting_twice_for_the_same_option_adds_nothing() {
        let mut p = poll();
        p.cast(A, 1).unwrap();
        assert_eq!(p.cast(A, 1), Ok(Cast::Unchanged));
        assert_eq!(p.total(), 1);
    }

    #[test]
    fn a_changed_vote_moves_rather_than_adds() {
        let mut p = poll();
        p.cast(A, 0).unwrap();
        assert_eq!(p.cast(A, 2), Ok(Cast::Moved));
        assert_eq!((p.tally(0), p.tally(2), p.total()), (0, 1, 1));
    }

    #[test]
    fn a_second_person_adds_their_own_vote() {
        let mut p = poll();
        p.cast(A, 0).unwrap();
        assert_eq!(p.cast(B, 0), Ok(Cast::New));
        assert_eq!((p.tally(0), p.total()), (2, 2));
    }

    #[test]
    fn a_third_option_takes_a_vote_and_leaves_the_others() {
        let mut p = poll();
        p.cast(A, 0).unwrap();
        p.cast(B, 1).unwrap();
        p.cast("cc", 2).unwrap();
        assert_eq!((p.tally(0), p.tally(1), p.tally(2)), (1, 1, 1));
    }

    #[test]
    fn a_closed_poll_takes_no_vote_and_keeps_its_count() {
        let mut p = poll();
        p.cast(A, 0).unwrap();
        p.open = false;
        assert_eq!(p.cast(B, 0), Err(Refusal::Closed));
        assert_eq!(p.cast(A, 1), Err(Refusal::Closed));
        assert_eq!((p.tally(0), p.tally(1)), (1, 0));
    }

    #[test]
    fn a_caller_with_no_usable_id_cannot_vote() {
        let mut p = poll();
        for id in ["", "not hex", "a\u{1f}1", &"a".repeat(65)] {
            assert_eq!(p.cast(id, 0), Err(Refusal::Anonymous));
        }
        assert_eq!(p.total(), 0);
    }

    #[test]
    fn an_option_that_is_not_there_is_refused() {
        assert_eq!(poll().cast(A, 3), Err(Refusal::NoSuchOption));
    }

    #[test]
    fn the_state_round_trips_with_the_voters() {
        let mut p = poll();
        p.cast(A, 0).unwrap();
        p.cast(B, 2).unwrap();
        assert_eq!(Poll::parse(&p.state()), Some(p));
    }

    #[test]
    fn a_repeated_or_forged_voter_row_is_dropped_on_read() {
        let state = format!(
            "1{FIELD}q{FIELD}x{ENTRY}y{FIELD}aa{ITEM}0{ENTRY}aa{ITEM}1{ENTRY}a b{ITEM}0{ENTRY}bb{ITEM}9"
        );
        let p = Poll::parse(&state).unwrap();
        assert_eq!((p.tally(0), p.tally(1), p.total()), (1, 0, 1));
    }

    #[test]
    fn the_poll_holds_a_bounded_number_of_voters() {
        let mut p = poll();
        for n in 0..MAX_VOTERS {
            p.cast(&format!("{n:x}"), 0).unwrap();
        }
        assert_eq!(p.cast("ffff", 0), Err(Refusal::Full));
        assert_eq!(p.cast("0", 1), Ok(Cast::Moved));
    }
}
