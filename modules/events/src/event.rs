//! One event and the answers to it, in the compact shape kept in one `kv.store` value.

use serde::{Deserialize, Serialize};

pub const GOING: u8 = 0;
pub const MAYBE: u8 = 1;
pub const NO: u8 = 2;
pub const STATUS_LABELS: [&str; 3] = ["Going", "Maybe", "Can't"];

/// Answers kept per event: each holds a full 64-character caller id, so 25 keep one event under 3 KiB and 20 events under the 64 KiB store.
pub const MAX_ANSWERS: usize = 25;
pub const MAX_TITLE: usize = 60;
pub const MAX_WHERE: usize = 40;
pub const MAX_NAME: usize = 24;
/// Serialized bytes an event may reach: the store keeps 4096 per value, and multi-byte names count by byte.
const MAX_BYTES: usize = 3800;
const MAX_ID: usize = 64;
const MINUTE: i64 = 60_000;

/// Who answered (their full caller id, which a reminder needs to mention them), what, and the name the host last gave for them.
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

/// The characters the host refuses in a post because they change what a reader sees without drawing anything.
fn is_hidden_char(c: char) -> bool {
    c.is_control()
        || matches!(c,
            '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{115F}'..='\u{1160}' | '\u{17B4}'..='\u{17B5}' | '\u{180E}'
            | '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{2029}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{206F}'
            | '\u{2800}' | '\u{3164}' | '\u{FEFF}' | '\u{FFA0}' | '\u{FFF9}'..='\u{FFFC}' | '\u{E0000}'..='\u{E007F}')
}

/// Plain text only: no control or hidden characters and nothing a message would read as a mention or markup.
pub fn clean(text: &str, max: usize) -> String {
    let kept: String = text.chars().filter(|c| !is_hidden_char(*c) && !"@<>`".contains(*c)).collect();
    let words: Vec<&str> = kept.split_whitespace().collect();
    words.join(" ").chars().take(max).collect()
}

/// The caller id an answer is kept under: the whole id, at most 64 characters.
pub fn who(caller: &str) -> String {
    caller.chars().take(MAX_ID).collect()
}

pub enum Refusal {
    Full,
    TooLarge,
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

    /// Records `status` for `caller`, and the `name` slim gave for them this time when there is one.
    pub fn answer(&mut self, caller: &str, name: &str, status: u8) -> Result<(), Refusal> {
        let key = who(caller);
        let name = clean(name, MAX_NAME);
        if let Some(index) = self.rs.iter().position(|a| a.0 == key) {
            let before = self.rs[index].clone();
            self.rs[index].1 = status;
            if !name.is_empty() {
                self.rs[index].2 = name;
            }
            if self.too_large() {
                self.rs[index] = before;
                return Err(Refusal::TooLarge);
            }
            return Ok(());
        }
        if self.rs.len() >= MAX_ANSWERS {
            return Err(Refusal::Full);
        }
        self.rs.push(Answer(key, status, name));
        if self.too_large() {
            self.rs.pop();
            return Err(Refusal::TooLarge);
        }
        Ok(())
    }

    fn too_large(&self) -> bool {
        serde_json::to_string(self).map_or(true, |json| json.len() > MAX_BYTES)
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

    /// The caller ids of everyone who gave `status`, oldest answer first.
    pub fn ids(&self, status: u8) -> Vec<&str> {
        self.rs.iter().filter(|a| a.1 == status).map(|a| a.0.as_str()).collect()
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
        assert!(e.answer("alice", "", GOING).is_ok() && e.answer("bob", "", MAYBE).is_ok() && e.answer("alice", "", NO).is_ok());
        assert_eq!(e.counts(), [0, 1, 1]);
        assert_eq!(e.answer_of("alice").map(|a| a.1), Some(NO));
    }

    #[test]
    fn the_answer_list_stops_at_its_limit_but_still_lets_people_change_theirs() {
        let mut e = sample();
        for i in 0..MAX_ANSWERS {
            assert!(e.answer(&format!("caller-{i:04}"), "", GOING).is_ok());
        }
        assert!(e.answer("one-too-many", "", GOING).is_err());
        assert!(e.answer("caller-0007", "", NO).is_ok());
        assert_eq!(e.counts(), [MAX_ANSWERS - 1, 0, 1]);
    }

    #[test]
    fn a_full_event_with_the_longest_text_still_fits_one_kv_value_and_twenty_fit_the_store() {
        let mut e = sample();
        e.t = "t".repeat(MAX_TITLE);
        e.w = "w".repeat(MAX_WHERE);
        e.by = who(&"f".repeat(64));
        e.ch = "0190a3c2-7e11-7aaa-b0f0-123456789abc".into();
        e.x = true;
        e.r = true;
        e.rem = 1_796_493_600_000;
        for i in 0..MAX_ANSWERS {
            let caller = format!("{i:016x}ffffffffffffffffffffffffffffffffffffffffffffffff");
            e.answer(&caller, &"n".repeat(MAX_NAME), GOING).ok();
        }
        let size = serde_json::to_string(&e).unwrap().len();
        assert!(size < 4096 - 256, "{size}");
        assert!(20 * size < 64 * 1024, "{size}");
    }

    #[test]
    fn hidden_characters_are_dropped_from_text() {
        assert_eq!(clean("Game\u{200B} ni\u{202E}ght\u{FEFF}", 40), "Game night");
    }

    #[test]
    fn multi_byte_names_stop_at_the_byte_budget_not_the_answer_count() {
        for glyph in ["\u{4e2d}", "\u{1F600}"] {
            let mut e = sample();
            e.t = glyph.repeat(MAX_TITLE);
            e.w = glyph.repeat(MAX_WHERE);
            let mut refused = false;
            for i in 0..MAX_ANSWERS {
                let caller = format!("{i:016x}ffffffffffffffffffffffffffffffffffffffffffffffff");
                if e.answer(&caller, &glyph.repeat(MAX_NAME), GOING).is_err() {
                    refused = true;
                    break;
                }
            }
            assert!(refused, "the budget should bite before the answer count");
            assert!(serde_json::to_string(&e).unwrap().len() <= 4096 - 256);
            let id = e.rs[0].0.clone();
            assert!(e.answer(&id, "", NO).is_ok(), "changing an answer still fits");
        }
    }

    #[test]
    fn names_are_plain_text_with_no_mentions_or_markup_and_the_latest_one_wins() {
        assert_eq!(clean("  @everyone  <@123> `x`\n", 40), "everyone 123 x");
        assert_eq!(clean(&"a".repeat(100), 14).len(), 14);
        let mut e = sample();
        e.answer("alice-id", "@here", GOING).ok();
        assert_eq!(e.names(GOING), ["here"]);
        e.answer("alice-id", "Alice", MAYBE).ok();
        assert_eq!(e.names(MAYBE), ["Alice"]);
        e.answer("alice-id", "", GOING).ok();
        assert_eq!(e.names(GOING), ["Alice"], "no name this time keeps the last one");
        e.answer("bob-id", "", GOING).ok();
        assert_eq!(e.names(GOING), ["Alice", "member bob-"]);
    }

    #[test]
    fn the_ids_of_those_going_are_whole_caller_ids() {
        let mut e = sample();
        let id = "a".repeat(64);
        e.answer(&id, "Ann", GOING).ok();
        e.answer("b", "Bo", MAYBE).ok();
        assert_eq!(e.ids(GOING), [id.as_str()]);
        assert_eq!(who(&"c".repeat(80)).len(), 64);
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
