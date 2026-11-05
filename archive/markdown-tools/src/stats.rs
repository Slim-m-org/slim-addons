//! Word count, character count and estimated reading time for markdown text.
//!
//! Reading time assumes 200 words per minute, rounded up to the nearest
//! minute (a partial minute still shows as 1, unless there are no words).

const WORDS_PER_MINUTE: usize = 200;

/// Words are tokens with at least one letter or digit, so heading hashes, bullets, rules and table pipes are not counted.
///
/// Summarizes `input` as `Words: N`, `Characters: N`, `Reading time: N min`.
pub fn summarize(input: &str) -> String {
    let words = input.split_whitespace().filter(|token| token.chars().any(char::is_alphanumeric)).count();
    let characters = input.chars().count();
    let minutes = reading_minutes(words);

    format!("Words: {words}\nCharacters: {characters}\nReading time: {minutes} min")
}

fn reading_minutes(words: usize) -> usize {
    if words == 0 {
        return 0;
    }
    words.div_ceil(WORDS_PER_MINUTE).max(1)
}

#[cfg(test)]
mod tests {
    use super::summarize;

    fn words(input: &str) -> String {
        summarize(input).lines().next().unwrap().to_owned()
    }

    #[test]
    fn markdown_markers_are_not_words() {
        assert_eq!(words("# Title\n\n- a\n- b"), "Words: 3");
    }

    #[test]
    fn table_pipes_rules_and_fences_are_not_words() {
        assert_eq!(words("| a | b |\n|---|---|\n| c | d |\n\n---\n```rust\n```"), "Words: 5");
    }

    #[test]
    fn plain_prose_counts_every_word() {
        assert_eq!(words("one two  three\nfour"), "Words: 4");
        assert_eq!(words("it's 3.5 well-known e-mail"), "Words: 4");
    }

    #[test]
    fn characters_still_count_everything() {
        assert_eq!(summarize("# a").lines().nth(1), Some("Characters: 3"));
    }

    #[test]
    fn reading_time_rounds_up_to_a_started_minute_and_is_zero_for_nothing() {
        let time = |n: usize| summarize(&"w ".repeat(n)).lines().nth(2).unwrap().to_owned();
        assert_eq!(time(0), "Reading time: 0 min");
        assert_eq!(time(1), "Reading time: 1 min");
        assert_eq!(time(200), "Reading time: 1 min");
        assert_eq!(time(201), "Reading time: 2 min");
    }
}
