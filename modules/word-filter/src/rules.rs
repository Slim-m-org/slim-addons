//! The admin's word list and patterns, built every run inside the fuel budget: words are a token lookup
//! with no regex at all, and patterns use the regex crate's lazy DFA, which matches in linear time but
//! spends states on bounded repeats, so every size is capped here and every pattern is weighed in weight.rs.

use regex::bytes::{Regex, RegexBuilder};
use serde_json::Value;

use crate::weight;
use crate::words::WordList;

/// At most this many regex patterns are read; the rest are an error, not silently dropped.
pub const MAX_PATTERNS: usize = 5;
pub const MAX_PATTERN_CHARS: usize = 100;
pub const MAX_WORDS: usize = 100;
const COMPILED_SIZE_LIMIT: usize = 8 * 1024;
const NEST_LIMIT: u32 = 16;
/// Past this many pattern matches the rest of the message is masked whole, which keeps a
/// pattern that matches every word inside the fuel budget and errs toward filtering.
const MAX_MASKED_MATCHES: usize = 256;
/// Too small a cache makes the lazy DFA give up and fall back to a matcher many times slower.
const DFA_CACHE: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Mask,
    Block,
}

pub struct Rules {
    pub action: Action,
    pub reason: String,
    exempt_channels: Vec<String>,
    words: WordList,
    patterns: Option<Regex>,
}

pub const DEFAULT_REASON: &str = "That message has a word this server filters.";

impl Rules {
    /// Reads the settings; a pattern that does not compile is an error naming it.
    pub fn from_settings(settings: &Value) -> Result<Rules, String> {
        let action = match settings["action"].as_str() {
            Some("block") => Action::Block,
            _ => Action::Mask,
        };
        let reason = match settings["block_reason"].as_str().map(str::trim) {
            Some(r) if !r.is_empty() => r.chars().take(200).collect(),
            _ => DEFAULT_REASON.to_string(),
        };
        let exempt_channels = split_list(settings["exempt_channels"].as_str().unwrap_or(""));
        let entries = split_list(settings["words"].as_str().unwrap_or(""));
        if entries.len() > MAX_WORDS {
            return Err(format!("the word list has {} entries; the most is {MAX_WORDS}", entries.len()));
        }
        let words = WordList::new(&entries);
        let patterns: Vec<&str> = settings["patterns"]
            .as_str()
            .unwrap_or("")
            .lines()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .collect();
        if patterns.len() > MAX_PATTERNS {
            return Err(format!("there are {} patterns; the most is {MAX_PATTERNS}", patterns.len()));
        }
        if patterns.iter().any(|p| p.chars().count() > MAX_PATTERN_CHARS) {
            return Err(format!("a pattern is longer than {MAX_PATTERN_CHARS} characters"));
        }
        let mut total = 0;
        for pattern in &patterns {
            let w = weight::of(pattern)?;
            if w > weight::MAX_PATTERN_WEIGHT {
                return Err(format!("the pattern {pattern} repeats too much; use + or * instead of large {{n,m}} counts"));
            }
            total += w;
        }
        if total > weight::MAX_TOTAL_WEIGHT {
            return Err("the patterns together repeat too much; remove some or use + and * instead of {n,m}".into());
        }
        let patterns = combined(&patterns)?;
        Ok(Rules { action, reason, exempt_channels, words, patterns })
    }

    pub fn is_exempt(&self, channel_id: Option<&str>) -> bool {
        channel_id.is_some_and(|id| self.exempt_channels.iter().any(|c| c == id))
    }

    pub fn matches(&self, text: &str) -> bool {
        !self.words.spans(text).is_empty() || self.patterns.as_ref().is_some_and(|p| p.is_match(text.as_bytes()))
    }

    /// Every matched span replaced with one asterisk per character, so the length reads the same.
    pub fn mask(&self, text: &str) -> String {
        let mut hidden = vec![false; text.len()];
        for (start, end) in self.words.spans(text) {
            hidden[start..end].iter_mut().for_each(|h| *h = true);
        }
        let mut found = self.patterns.iter().flat_map(|p| p.find_iter(text.as_bytes()));
        for m in found.by_ref().take(MAX_MASKED_MATCHES) {
            hidden[m.start()..m.end()].iter_mut().for_each(|h| *h = true);
        }
        if let Some(m) = found.next() {
            hidden[m.start()..].iter_mut().for_each(|h| *h = true);
        }
        text.char_indices()
            .map(|(i, c)| if hidden[i] && !c.is_whitespace() { '*' } else { c })
            .collect()
    }
}

/// One alternation, so a message is scanned once however many patterns there are; only when
/// it fails are the patterns compiled one by one, to name the bad one.
fn combined(patterns: &[&str]) -> Result<Option<Regex>, String> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let joined: Vec<String> = patterns.iter().map(|p| format!("(?:{p})")).collect();
    if let Ok(regex) = build(&joined.join("|"), "") {
        return Ok(Some(regex));
    }
    for pattern in patterns {
        build(pattern, pattern)?;
    }
    Err("the patterns together are too large; remove some".into())
}

fn build(pattern: &str, name: &str) -> Result<Regex, String> {
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .size_limit(COMPILED_SIZE_LIMIT)
        .nest_limit(NEST_LIMIT)
        .unicode(false)
        .dfa_size_limit(DFA_CACHE)
        .build()
        .map_err(|_| format!("the pattern {name} is not a valid or small enough regex"))
}

/// Commas and line breaks both separate entries, so either way of typing a list works.
fn split_list(raw: &str) -> Vec<String> {
    raw.split([',', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn rules(settings: Value) -> Rules {
        Rules::from_settings(&settings).unwrap()
    }

    #[test]
    fn a_listed_word_is_masked_whole_and_case_blind() {
        let r = rules(json!({"words": "darn, heck"}));
        assert_eq!(r.mask("Darn it, what the HECK"), "**** it, what the ****");
    }

    #[test]
    fn a_word_inside_another_word_is_left_alone() {
        let r = rules(json!({"words": "ass"}));
        assert!(!r.matches("a classic pass"));
        assert!(r.matches("you ass"));
    }

    #[test]
    fn a_pattern_masks_what_it_matches_and_keeps_spaces() {
        let r = rules(json!({"patterns": r"\d{3}-\d{4}"}));
        assert_eq!(r.mask("call 555-1234 now"), "call ******** now");
    }

    #[test]
    fn a_mask_keeps_multibyte_text_intact() {
        let r = rules(json!({"words": "café"}));
        assert_eq!(r.mask("un café crème"), "un **** crème");
    }

    #[test]
    fn past_the_match_cap_the_rest_is_masked_whole() {
        let r = rules(json!({"patterns": r"\bx\b"}));
        let text = "x ".repeat(MAX_MASKED_MATCHES + 1) + "tail";
        let masked = r.mask(&text);
        assert!(masked.ends_with("****"));
        assert!(!masked.contains('x'));
    }

    #[test]
    fn dot_and_negated_classes_are_accepted_and_mask_whole_characters() {
        let r = rules(json!({"patterns": "https?://\\S+\\.ru\\b\nc.f."}));
        assert_eq!(r.mask("see http://x.ru now"), "see *********** now");
        assert_eq!(r.mask("un café"), "un ****");
    }

    #[test]
    fn a_nested_bounded_repeat_is_refused_with_a_way_out() {
        let err = Rules::from_settings(&json!({"patterns": "(?:.{0,10}){3}x"})).err().unwrap();
        assert!(err.contains("instead of large"));
        assert!(Rules::from_settings(&json!({"patterns": ".{1,18}x"})).is_ok());
    }

    #[test]
    fn a_bad_pattern_is_an_error_that_names_it() {
        let err = Rules::from_settings(&json!({"patterns": "(unclosed"})).err().unwrap();
        assert!(err.contains("(unclosed"));
    }

    #[test]
    fn too_many_patterns_is_an_error_not_a_silent_drop() {
        let many: Vec<String> = (0..=MAX_PATTERNS).map(|i| format!("x{i}")).collect();
        assert!(Rules::from_settings(&json!({"patterns": many.join("\n")})).is_err());
    }

    #[test]
    fn a_huge_repetition_is_refused_at_compile() {
        assert!(Rules::from_settings(&json!({"patterns": r"(\w{100}){100}"})).is_err());
    }

    #[test]
    fn a_phrase_in_the_word_list_matches_across_spacing() {
        let r = rules(json!({"words": "free crypto"}));
        assert_eq!(r.mask("get FREE   crypto now"), "get ****   ****** now");
        assert!(!r.matches("free stuff, crypto later"));
    }

    #[test]
    fn a_backtracking_shaped_pattern_runs_in_linear_time() {
        let r = rules(json!({"patterns": "(a+)+$"}));
        let long = format!("{}b", "a".repeat(4000));
        assert!(!r.matches(&long));
    }

    #[test]
    fn exempt_channels_and_defaults() {
        let r = rules(json!({"exempt_channels": "c1,\nc2"}));
        assert!(r.is_exempt(Some("c2")));
        assert!(!r.is_exempt(Some("c3")));
        assert!(!r.is_exempt(None));
        assert_eq!(r.action, Action::Mask);
        assert_eq!(r.reason, DEFAULT_REASON);
        assert!(!r.matches("anything at all"));
    }
}
