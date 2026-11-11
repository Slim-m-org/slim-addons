//! Turning stored lines into the text of a message: speaker labels, clock offsets, merged runs and the length budget.
//!
//! Everything here is pure, so the unit tests need no host.

use std::collections::HashMap;

use crate::store::{Call, Line, Person};

/// What a posted message may hold; the host refuses more.
pub const MESSAGE_CHARS: usize = 4000;
/// Room kept for the part's header and footer inside a message.
const FRAME_CHARS: usize = 500;
/// Characters of speech a message carries, leaving room for the frame.
pub const BODY_CHARS: usize = MESSAGE_CHARS - FRAME_CHARS;
const DAY_MS: i64 = 86_400_000;

/// How a block of speech is laid out; settings and the call decide it.
pub struct Layout<'a> {
    pub started_ms: i64,
    pub merge_gap_ms: i64,
    pub timestamps: bool,
    pub speakers: &'a HashMap<String, Person>,
}

struct Block {
    user: String,
    prefix: String,
    text: String,
    end_ms: i64,
}

/// Lines folded into blocks of one speaker, filled until the next line would not fit a message.
pub struct Chunk<'a> {
    layout: &'a Layout<'a>,
    blocks: Vec<Block>,
    chars: usize,
    pub lines: i64,
}

impl<'a> Chunk<'a> {
    pub fn new(layout: &'a Layout<'a>) -> Self {
        Chunk { layout, blocks: Vec::new(), chars: 0, lines: 0 }
    }

    /// Adds a line; false, and nothing added, when it would push the chunk past a message.
    pub fn push(&mut self, line: &Line) -> bool {
        let text = tidy(&line.body);
        if text.is_empty() {
            self.lines += 1;
            return true;
        }
        let merges = self.blocks.last().is_some_and(|b| b.user == line.user_id && line.start_ms - b.end_ms <= self.layout.merge_gap_ms);
        let cost = if merges {
            1 + text.chars().count()
        } else {
            let separator = usize::from(!self.blocks.is_empty());
            separator + self.prefix(line).chars().count() + text.chars().count()
        };
        if self.chars + cost > BODY_CHARS && !self.blocks.is_empty() {
            return false;
        }
        self.chars += cost;
        self.lines += 1;
        match self.blocks.last_mut().filter(|_| merges) {
            Some(block) => {
                block.text.push(' ');
                block.text.push_str(&text);
                block.end_ms = line.end_ms.max(block.end_ms);
            }
            None => {
                let prefix = self.prefix(line);
                self.blocks.push(Block { user: line.user_id.clone(), prefix, text, end_ms: line.end_ms });
            }
        }
        true
    }

    fn prefix(&self, line: &Line) -> String {
        let who = label(self.layout.speakers.get(&line.user_id).unwrap_or(&Person::default()));
        if self.layout.timestamps {
            format!("[{}] {who}: ", clock(line.start_ms - self.layout.started_ms))
        } else {
            format!("{who}: ")
        }
    }

    pub fn text(&self) -> String {
        self.blocks.iter().map(|b| format!("{}{}", b.prefix, b.text)).collect::<Vec<_>>().join("\n")
    }
}

/// The name the host sent, made safe to read as speech; `Speaker N` when there is none.
pub fn label(person: &Person) -> String {
    let name = tidy(&person.name);
    if !name.is_empty() {
        name
    } else if person.ord > 0 {
        format!("Speaker {}", person.ord)
    } else {
        "Speaker".to_string()
    }
}

/// `m:ss`, or `h:mm:ss` from the first hour; a negative offset is a clock skew and reads as zero.
pub fn clock(offset_ms: i64) -> String {
    let secs = offset_ms.max(0) / 1000;
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// One line of plain text: whitespace collapsed, and `@` swapped for a look-alike so speech can never mention anyone.
pub fn tidy(speech: &str) -> String {
    let flat = speech.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.replace('@', "\u{FF20}")
}

/// `2026-11-10 14:03 UTC` for a UTC epoch in milliseconds.
pub fn stamp(ms: i64) -> String {
    let days = ms.div_euclid(DAY_MS);
    let in_day = ms.rem_euclid(DAY_MS) / 60_000;
    let (y, m, d) = civil(days);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02} UTC", in_day / 60, in_day % 60)
}

/// Days since 1970-01-01 as a calendar date.
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// How long a call ran, for a header: `under a minute`, `12 min` or `1 h 05 min`.
pub fn duration(ms: i64) -> String {
    let minutes = ms.max(0) / 60_000;
    match minutes {
        0 => "under a minute".to_string(),
        1..=59 => format!("{minutes} min"),
        _ => format!("{} h {:02} min", minutes / 60, minutes % 60),
    }
}

/// Why a call stopped, for a footer; ordinary ends say nothing.
pub fn reason_note(reason: &str) -> Option<&'static str> {
    match reason {
        "time_limit" => Some("The call reached the time limit for transcripts, so the rest is not here."),
        "overload" => Some("The server could not keep up, so some of the call may be missing."),
        "listener_unavailable" => Some("The listener went away, so the rest of the call is not here."),
        "module_disabled" | "channel_removed" => Some("Transcribing was switched off during the call."),
        "lapsed" => Some("The call went quiet for too long and was closed here."),
        _ => None,
    }
}

/// What the footer says about what the module did not get; `None` when nothing is missing.
pub fn missing_note(call: &Call) -> Option<String> {
    let lost = if call.sent >= 0 { call.sent - call.seen } else { call.dropped };
    (lost > 0).then(|| format!("{lost} segments never reached this transcript."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(user: &str, start_ms: i64, end_ms: i64, body: &str) -> Line {
        Line { user_id: user.to_string(), start_ms, end_ms, body: body.to_string() }
    }

    fn person(ord: i64, name: &str) -> Person {
        Person { ord, name: name.to_string() }
    }

    fn layout(speakers: &HashMap<String, Person>) -> Layout<'_> {
        Layout { started_ms: 1_000, merge_gap_ms: 30_000, timestamps: true, speakers }
    }

    #[test]
    fn clock_offsets_read_as_minutes_and_hours() {
        assert_eq!((clock(0).as_str(), clock(65_000).as_str(), clock(3_725_000).as_str()), ("0:00", "1:05", "1:02:05"));
        assert_eq!(clock(-5), "0:00");
    }

    #[test]
    fn a_stamp_is_utc_and_survives_leap_days_and_the_epoch() {
        assert_eq!(stamp(0), "1970-01-01 00:00 UTC");
        assert_eq!(stamp(1_709_164_800_000 + 59 * 60_000 + 23 * 3_600_000), "2024-02-29 23:59 UTC");
        assert_eq!(stamp(1_700_000_000_000), "2023-11-14 22:13 UTC");
    }

    #[test]
    fn speech_loses_its_mentions_and_its_line_breaks() {
        assert_eq!(tidy("hi\n  @everyone  and @[Mods]"), "hi \u{FF20}everyone and \u{FF20}[Mods]");
    }

    #[test]
    fn durations_round_down_to_minutes() {
        assert_eq!((duration(30_000).as_str(), duration(12 * 60_000 + 59_000).as_str(), duration(65 * 60_000).as_str()), ("under a minute", "12 min", "1 h 05 min"));
    }

    #[test]
    fn a_speaker_who_talks_on_is_one_block_and_a_pause_or_another_voice_starts_a_new_one() {
        let people = HashMap::from([("a".to_string(), person(1, "")), ("b".to_string(), person(2, ""))]);
        let layout = layout(&people);
        let mut chunk = Chunk::new(&layout);
        for l in [
            line("a", 2_000, 3_000, "one"),
            line("a", 4_000, 5_000, "two"),
            line("b", 6_000, 7_000, "three"),
            line("a", 8_000, 9_000, "four"),
            line("a", 60_000, 61_000, "five"),
        ] {
            assert!(chunk.push(&l));
        }
        assert_eq!(chunk.text(), "[0:01] Speaker 1: one two\n[0:05] Speaker 2: three\n[0:07] Speaker 1: four\n[0:59] Speaker 1: five");
        assert_eq!(chunk.lines, 5);
    }

    #[test]
    fn a_name_replaces_the_number_and_a_missing_one_falls_back_to_it() {
        let people = HashMap::from([("a".to_string(), person(1, "Ada @everyone")), ("b".to_string(), person(2, ""))]);
        let layout = layout(&people);
        let mut chunk = Chunk::new(&layout);
        assert!(chunk.push(&line("a", 2_000, 3_000, "hello")) && chunk.push(&line("b", 6_000, 7_000, "hi")));
        assert_eq!(chunk.text(), "[0:01] Ada \u{FF20}everyone: hello\n[0:05] Speaker 2: hi");
    }

    #[test]
    fn timestamps_can_be_left_out() {
        let people = HashMap::from([("a".to_string(), person(1, ""))]);
        let layout = Layout { timestamps: false, ..layout(&people) };
        let mut chunk = Chunk::new(&layout);
        assert!(chunk.push(&line("a", 2_000, 3_000, "hello")));
        assert_eq!(chunk.text(), "Speaker 1: hello");
    }

    #[test]
    fn a_chunk_refuses_the_line_that_would_overflow_a_message_and_never_an_empty_one() {
        let people = HashMap::from([("a".to_string(), person(1, "")), ("b".to_string(), person(2, ""))]);
        let layout = layout(&people);
        let mut chunk = Chunk::new(&layout);
        let long = "word ".repeat(180);
        let mut taken = 0;
        while chunk.push(&line(if taken % 2 == 0 { "a" } else { "b" }, 2_000 + taken * 1000, 2_500 + taken * 1000, &long)) {
            taken += 1;
        }
        assert!(taken >= 3 && chunk.lines == taken);
        assert!(chunk.text().chars().count() <= BODY_CHARS);
        let mut alone = Chunk::new(&layout);
        assert!(alone.push(&line("a", 2_000, 3_000, &"x".repeat(1_000))));
    }
}
