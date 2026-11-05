//! The transcript as messages: in time order, a speaker named at every change of speaker,
//! split to fit, and honest about segments the module never received.

use crate::store::{Segment, Session, POSTS_PER_RUN};

/// Characters per message, well inside what a message may hold.
pub const CHUNK_CHARS: usize = 1800;

/// Messages built from one page of segments.
pub struct Rendered {
    pub chunks: Vec<Chunk>,
    /// Speakers in order of first speaking, including any new on this page.
    pub speakers: Vec<String>,
}

/// One message, and the last segment it covers.
pub struct Chunk {
    pub text: String,
    pub through_id: i64,
}

/// The transcript's messages for `segments`, a page after the ones already posted. A speaker is
/// "Speaker n" in order of first speaking, because the host sends a module user ids and nothing
/// that names them. The first message of a transcript opens with a heading.
pub fn render(session: &Session, segments: &[Segment], received: i64) -> Rendered {
    let mut speakers: Vec<String> = session.speakers.split(',').filter(|s| !s.is_empty()).map(str::to_string).collect();
    let mut chunks: Vec<Chunk> = Vec::new();
    let mut current = if session.posted == 0 { heading(session, segments.is_empty(), received) } else { String::new() };
    let mut through = session.cursor;
    let mut last: Option<&str> = None;
    for segment in segments {
        let n = match speakers.iter().position(|s| *s == segment.user_id) {
            Some(at) => at + 1,
            None => {
                speakers.push(segment.user_id.clone());
                speakers.len()
            }
        };
        let text = segment.text.trim();
        let continues = last == Some(segment.user_id.as_str()) && !current.is_empty();
        for (i, piece) in split_long(text).into_iter().enumerate() {
            let line = if continues && i == 0 { format!(" {piece}") } else if i == 0 { format!("**Speaker {n}:** {piece}") } else { format!("**Speaker {n}:** {piece}") };
            let joiner = if continues && i == 0 { "" } else if current.is_empty() { "" } else { "\n" };
            if current.chars().count() + joiner.len() + line.chars().count() > CHUNK_CHARS && !current.is_empty() {
                chunks.push(Chunk { text: std::mem::take(&mut current), through_id: through });
                current.push_str(line.trim_start());
            } else {
                current.push_str(joiner);
                current.push_str(&line);
            }
        }
        through = segment.id;
        last = Some(segment.user_id.as_str());
    }
    if !current.is_empty() {
        chunks.push(Chunk { text: current, through_id: through });
    }
    Rendered { chunks, speakers }
}

fn heading(session: &Session, silent: bool, received: i64) -> String {
    let mut head = format!("Transcript of the call, {}.", duration(session));
    if silent {
        head.push_str(" Nobody was heard.");
    }
    let missed = session.segments_sent - received;
    if missed > 0 {
        head.push_str(&format!(" {missed} {} missed because the queue was full.", if missed == 1 { "line was" } else { "lines were" }));
    }
    head.push('\n');
    head
}

/// How long the call was heard, in words.
fn duration(session: &Session) -> String {
    let ms = (session.stopped_ms - session.started_ms).max(0);
    let minutes = (ms + 59_999) / 60_000;
    if minutes <= 1 { "under a minute".to_string() } else { format!("{minutes} minutes") }
}

/// A line longer than a chunk, cut at spaces.
fn split_long(line: &str) -> Vec<String> {
    if line.chars().count() <= CHUNK_CHARS - 40 {
        return vec![line.to_string()];
    }
    let mut pieces = Vec::new();
    let mut current = String::new();
    for word in line.split(' ') {
        if current.chars().count() + word.chars().count() + 1 > CHUNK_CHARS - 40 {
            pieces.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    pieces.push(current);
    pieces
}

/// The chunks one run may still post.
pub fn this_run(chunks: &[Chunk]) -> &[Chunk] {
    &chunks[..chunks.len().min(POSTS_PER_RUN)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(id: i64, user: &str, text: &str) -> Segment {
        Segment { id, user_id: user.into(), start_ms: id, text: text.into() }
    }

    fn session(sent: i64) -> Session {
        Session { started_ms: 0, stopped_ms: 125_000, segments_sent: sent, ..Session::default() }
    }

    #[test]
    fn speakers_are_named_in_order_and_runs_of_one_speaker_join() {
        let segments = [seg(1, "u9", "hello"), seg(2, "u9", "again"), seg(3, "u2", "hi"), seg(4, "u9", "bye")];
        let out = render(&session(4), &segments, 4);
        assert_eq!(out.chunks.len(), 1);
        let text = &out.chunks[0].text;
        assert!(text.starts_with("Transcript of the call, 3 minutes."), "{text}");
        assert!(text.contains("**Speaker 1:** hello again\n**Speaker 2:** hi\n**Speaker 1:** bye"), "{text}");
        assert_eq!(out.speakers, ["u9", "u2"]);
        assert_eq!(out.chunks[0].through_id, 4);
    }

    #[test]
    fn speaker_numbers_carry_over_to_the_next_page() {
        let mut later = session(4);
        later.posted = 1;
        later.cursor = 2;
        later.speakers = "u9,u2".into();
        let out = render(&later, &[seg(3, "u2", "hi"), seg(4, "u7", "new")], 4);
        let text = &out.chunks[0].text;
        assert!(!text.contains("Transcript of the call"), "only the first message has the heading");
        assert!(text.starts_with("**Speaker 2:** hi\n**Speaker 3:** new"), "{text}");
    }

    #[test]
    fn a_gap_is_noted_when_the_host_sent_more_than_arrived() {
        let out = render(&session(5), &[seg(1, "u1", "x")], 3);
        assert!(out.chunks[0].text.contains("2 lines were missed"), "{}", out.chunks[0].text);
        let one = render(&session(4), &[seg(1, "u1", "x")], 3);
        assert!(one.chunks[0].text.contains("1 line was missed"));
        assert!(!render(&session(3), &[seg(1, "u1", "x")], 3).chunks[0].text.contains("missed"));
    }

    #[test]
    fn a_silent_call_says_so() {
        assert!(render(&session(0), &[], 0).chunks[0].text.contains("Nobody was heard."));
    }

    #[test]
    fn long_transcripts_split_under_the_limit_and_keep_every_word() {
        let segments: Vec<Segment> = (1..=200).map(|n| seg(n, if n % 2 == 0 { "a" } else { "b" }, &format!("sentence number {n} of the call"))).collect();
        let out = render(&session(200), &segments, 200);
        assert!(out.chunks.len() > 3, "{}", out.chunks.len());
        assert!(out.chunks.iter().all(|c| c.text.chars().count() <= CHUNK_CHARS), "a chunk was too long");
        let all: String = out.chunks.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join("\n");
        for n in [1, 100, 200] {
            assert!(all.contains(&format!("sentence number {n} of")));
        }
        assert!(out.chunks.windows(2).all(|w| w[0].through_id < w[1].through_id));
    }

    #[test]
    fn one_run_posts_three() {
        let chunks: Vec<Chunk> = (0..7).map(|n| Chunk { text: n.to_string(), through_id: n }).collect();
        assert_eq!(this_run(&chunks).len(), 3);
        assert_eq!(this_run(&chunks[5..]).len(), 2);
    }
}
