//! Whole-word and phrase matching by tokens, so a long word list costs a lookup per token
//! instead of a regex compile per run. A message can be 4000 characters and the hook has
//! 10M fuel, so the scan allocates nothing per token and skips tokens no entry could be.

use std::collections::BTreeMap;

/// Past this many token steps in one message the scan stops and the rest counts as a match.
const MAX_STEPS: usize = 8_000;

#[derive(Default)]
struct Node {
    ends_phrase: bool,
    next: BTreeMap<String, Node>,
}

/// What a scan found, and where it gave up when the message cost more than `MAX_STEPS`.
pub struct Scan {
    pub spans: Vec<(usize, usize)>,
    pub gave_up_at: Option<usize>,
}

/// Entries as a trie of lowercased tokens, so phrases sharing a first word are walked once per token.
pub struct WordList {
    root: Node,
    longest_token: usize,
}

impl WordList {
    pub fn new(entries: &[String]) -> WordList {
        let mut root = Node::default();
        let mut longest_token = 0;
        let mut buf = String::new();
        for entry in entries {
            let mut node = &mut root;
            let mut any = false;
            for (s, e) in bounds(entry) {
                lower_into(&entry[s..e], &mut buf);
                longest_token = longest_token.max(buf.len());
                node = node.next.entry(buf.clone()).or_default();
                any = true;
            }
            node.ends_phrase |= any;
        }
        WordList { root, longest_token }
    }

    /// Byte ranges of every listed word or phrase in `text`.
    pub fn scan(&self, text: &str) -> Scan {
        let mut scan = Scan { spans: Vec::new(), gave_up_at: None };
        if self.root.next.is_empty() {
            return scan;
        }
        let found = bounds(text);
        let mut buf = String::new();
        let mut steps = 0;
        for (i, &(start, _)) in found.iter().enumerate() {
            let mut node = &self.root;
            for &(s, e) in &found[i..] {
                steps += 1;
                if steps > MAX_STEPS {
                    scan.gave_up_at = Some(start);
                    return scan;
                }
                if !self.could_be(&text[s..e]) {
                    break;
                }
                lower_into(&text[s..e], &mut buf);
                let Some(child) = node.next.get(buf.as_str()) else { break };
                if child.ends_phrase {
                    scan.spans.push((start, e));
                }
                node = child;
            }
        }
        scan
    }

    /// Byte ranges of every listed word or phrase in `text`.
    #[cfg(test)]
    pub fn spans(&self, text: &str) -> Vec<(usize, usize)> {
        self.scan(text).spans
    }

    /// An ASCII token longer than every listed token can never match, so it is not lowercased.
    fn could_be(&self, token: &str) -> bool {
        !token.is_ascii() || token.len() <= self.longest_token
    }
}

/// Byte ranges of the runs of letters and digits.
fn bounds(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        let word_char = if c.is_ascii() { c.is_ascii_alphanumeric() } else { c.is_alphanumeric() };
        match (word_char, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, text.len()));
    }
    out
}

fn lower_into(token: &str, buf: &mut String) {
    buf.clear();
    if token.is_ascii() {
        buf.push_str(token);
        buf.make_ascii_lowercase();
    } else {
        buf.extend(token.chars().flat_map(char::to_lowercase));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(entries: &[&str]) -> WordList {
        WordList::new(&entries.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn finds_whole_words_only_and_ignores_case() {
        let l = list(&["darn"]);
        assert_eq!(l.spans("Darn, darned DARN"), vec![(0, 4), (13, 17)]);
    }

    #[test]
    fn a_phrase_needs_its_tokens_in_order() {
        let l = list(&["free crypto"]);
        assert_eq!(l.spans("free, crypto!"), vec![(0, 12)]);
        assert!(l.spans("crypto free").is_empty());
        assert!(l.spans("free").is_empty());
    }

    #[test]
    fn non_ascii_words_match_by_their_lowercase() {
        assert_eq!(list(&["ÉCOLE"]).spans("une école"), vec![(4, 10)]);
    }

    #[test]
    fn phrases_sharing_a_first_word_all_match() {
        let l = list(&["free money", "free crypto", "free"]);
        assert_eq!(l.spans("free crypto"), vec![(0, 4), (0, 11)]);
        assert_eq!(l.spans("free money"), vec![(0, 4), (0, 10)]);
    }

    #[test]
    fn a_long_message_against_a_shared_first_word_list_gives_up_instead_of_running_on() {
        let entries: Vec<String> = (0..100).map(|i| format!("free w{i}")).collect();
        let l = WordList::new(&entries);
        let scan = l.scan(&"free ".repeat(20_000));
        assert!(scan.gave_up_at.is_some());
        assert!(l.scan("free free free").gave_up_at.is_none());
    }

    #[test]
    fn a_long_token_is_skipped_but_a_listed_one_of_the_same_start_is_found() {
        let l = list(&["spam"]);
        assert!(l.spans("spamspamspam").is_empty());
        assert_eq!(l.spans("SPAM spamspam"), vec![(0, 4)]);
    }
}
