//! One event and the answers to it, in the compact shape kept in one `kv.store` value.

use serde::{Deserialize, Serialize};

pub const GOING: u8 = 0;
pub const MAYBE: u8 = 1;
pub const NO: u8 = 2;
pub const STATUS_LABELS: [&str; 3] = ["Going", "Maybe", "Can't"];

/// Answers kept per event, so one event always fits the 4 KiB a stored value may hold.
pub const MAX_ANSWERS: usize = 50;
pub const MAX_TITLE: usize = 60;
pub const MAX_WHERE: usize = 40;
pub const MAX_NAME: usize = 14;
const MINUTE: i64 = 60_000;

/// Who answered (the first 16 characters of their opaque id), what, and the name they gave.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Answer(pub String, pub u8, pub String);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Event {
    pub n: u32,
    pub t: String,
    /// Start, UTC milliseconds.
    pub at: i64,
    /// The zone offset in minutes the host typed the start in, used only to show it.
    pub z: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub w: String,
    pub by: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ch: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub x: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub r: bool,
    /// When the reminder is due, UTC milliseconds; zero for none.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rem: i64,
    #[serde(default)]
    pub rs: Vec<Answer>,
}

fn is_false(b: &bool) -> bool {
    !b
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

/// Plain text only: no control characters and nothing a message would read as a mention or markup.
pub fn clean(text: &str, max: usize) -> String {
    let kept: String = text.chars().filter(|c| !c.is_control() && !"@<>`".contains(*c)).collect();
    let words: Vec<&str> = kept.split_whitespace().collect();
    words.join(" ").chars().take(max).collect()
}

/// The stable short form of a caller id that an answer is kept under.
pub fn who(caller: &str) -> String {
    caller.chars().take(16).collect()
}

pub enum Refusal {
    Full,
}

impl Event {
    pub fn started(&self, now_ms: i64) -> bool {
        now_ms >= self.at
    }

    pub fn is_host(&self, caller: &str) -> bool {
        !caller.is_empty() && self.by == who(caller)
    }

    pub fn answer_of(&self, caller: &str) -> Option<&Answer> {
        let key = who(caller);
        self.rs.iter().find(|a| a.0 == key)
    }

    pub fn answer(&mut self, caller: &str, status: u8) -> Result<(), Refusal> {
        let key = who(caller);
        if let Some(existing) = self.rs.iter_mut().find(|a| a.0 == key) {
            existing.1 = status;
            return Ok(());
        }
        if self.rs.len() >= MAX_ANSWERS {
            return Err(Refusal::Full);
        }
        self.rs.push(Answer(key, status, String::new()));
        Ok(())
    }

    /// Sets the name shown for `caller`; false when they have not answered yet.
    pub fn set_name(&mut self, caller: &str, name: &str) -> bool {
        let key = who(caller);
        match self.rs.iter_mut().find(|a| a.0 == key) {
            Some(existing) => {
                existing.2 = clean(name, MAX_NAME);
                true
            }
            None => false,
        }
    }

    pub fn counts(&self) -> [usize; 3] {
        let mut counts = [0; 3];
        for a in &self.rs {
            counts[usize::from(a.1).min(2)] += 1;
        }
        counts
    }

    /// The names of everyone who gave `status`, oldest answer first.
    pub fn names(&self, status: u8) -> Vec<String> {
        self.rs.iter().filter(|a| a.1 == status).map(label).collect()
    }

    /// When to remind: an hour ahead, or ten minutes ahead for an event that is soon; `None` when it is too close to bother.
    pub fn remind_at(&self, now_ms: i64) -> Option<i64> {
        let lead = if self.at - now_ms >= 75 * MINUTE { 60 * MINUTE } else { 10 * MINUTE };
        (self.at - now_ms >= lead + 2 * MINUTE).then_some(self.at - lead)
    }
}

pub fn label(answer: &Answer) -> String {
    if answer.2.is_empty() {
        format!("member {}", answer.0.chars().take(4).collect::<String>())
    } else {
        answer.2.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn sample() -> Event {
        Event { n: 1, t: "Game night".into(), at: 10_000_000_000, z: 0, w: String::new(), by: who("host-id"), ch: String::new(), x: false, r: false, rem: 0, rs: vec![] }
    }

    #[test]
    fn answering_again_moves_the_answer_instead_of_adding_one() {
        let mut e = sample();
        assert!(e.answer("alice", GOING).is_ok() && e.answer("bob", MAYBE).is_ok() && e.answer("alice", NO).is_ok());
        assert_eq!(e.counts(), [0, 1, 1]);
        assert_eq!(e.answer_of("alice").map(|a| a.1), Some(NO));
    }

    #[test]
    fn the_answer_list_stops_at_its_limit_but_still_lets_people_change_theirs() {
        let mut e = sample();
        for i in 0..MAX_ANSWERS {
            assert!(e.answer(&format!("caller-{i:04}"), GOING).is_ok());
        }
        assert!(e.answer("one-too-many", GOING).is_err());
        assert!(e.answer("caller-0007", NO).is_ok());
        assert_eq!(e.counts(), [MAX_ANSWERS - 1, 0, 1]);
    }

    #[test]
    fn a_full_event_with_the_longest_text_still_fits_one_kv_value() {
        let mut e = sample();
        e.t = "t".repeat(MAX_TITLE);
        e.w = "w".repeat(MAX_WHERE);
        e.ch = "0190a3c2-7e11-7aaa-b0f0-123456789abc".into();
        e.x = true;
        e.r = true;
        e.rem = 1_796_493_600_000;
        for i in 0..MAX_ANSWERS {
            let caller = format!("{i:016x}ffffffffffffffffffffffffffffffffffffffffffffffff");
            e.answer(&caller, GOING).ok();
            e.set_name(&caller, &"n".repeat(MAX_NAME));
        }
        assert!(serde_json::to_string(&e).unwrap().len() < 4096 - 256);
    }

    #[test]
    fn names_are_plain_text_with_no_mentions_or_markup() {
        assert_eq!(clean("  @everyone  <@123> `x`\n", 40), "everyone 123 x");
        assert_eq!(clean(&"a".repeat(100), 14).len(), 14);
        let mut e = sample();
        e.answer("alice", GOING).ok();
        assert!(!e.set_name("nobody", "x"));
        e.set_name("alice", "@here");
        assert_eq!(e.names(GOING), ["here"]);
        e.set_name("alice", "");
        assert_eq!(e.names(GOING), ["member alic"]);
    }

    #[test]
    fn the_reminder_leads_by_an_hour_or_ten_minutes_and_skips_what_is_too_close() {
        let mut e = sample();
        e.at = 100 * MINUTE;
        assert_eq!(e.remind_at(0), Some(40 * MINUTE));
        e.at = 30 * MINUTE;
        assert_eq!(e.remind_at(0), Some(20 * MINUTE));
        e.at = 11 * MINUTE;
        assert_eq!(e.remind_at(0), None);
    }

    #[test]
    fn only_the_host_is_the_host() {
        let e = sample();
        assert!(e.is_host("host-id") && !e.is_host("alice") && !e.is_host(""));
    }
}
