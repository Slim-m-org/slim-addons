//! Whole-word and phrase matching by tokens, so a long word list costs a lookup per token
//! instead of a regex compile per run. A message can be 4000 characters and the hook has
//! 10M fuel, so the scan allocates nothing per token and skips tokens no entry could be.

/// Entries as their lowercased tokens, sorted so a token is found by binary search.
pub struct WordList {
    entries: Vec<Vec<String>>,
    longest_token: usize,
}

impl WordList {
    pub fn new(entries: &[String]) -> WordList {
        let mut sorted: Vec<Vec<String>> = Vec::with_capacity(entries.len());
        let mut longest_token = 0;
        let mut buf = String::new();
        for entry in entries {
            let tokens: Vec<String> = bounds(entry)
                .into_iter()
                .map(|(s, e)| {
                    lower_into(&entry[s..e], &mut buf);
                    buf.clone()
                })
                .collect();
            longest_token = tokens.iter().map(String::len).fold(longest_token, usize::max);
            if !tokens.is_empty() {
                sorted.push(tokens);
            }
        }
        sorted.sort_unstable_by(|a, b| a[0].cmp(&b[0]));
        WordList { entries: sorted, longest_token }
    }

    /// Byte ranges of every listed word or phrase in `text`.
    pub fn spans(&self, text: &str) -> Vec<(usize, usize)> {
        if self.entries.is_empty() {
            return Vec::new();
        }
        let found = bounds(text);
        let mut spans = Vec::new();
        let mut buf = String::new();
        for (i, &(start, end)) in found.iter().enumerate() {
            if !self.could_be(&text[start..end]) {
                continue;
            }
            lower_into(&text[start..end], &mut buf);
            let from = self.entries.partition_point(|e| e[0].as_str() < buf.as_str());
            let to = from + self.entries[from..].iter().take_while(|e| e[0] == buf).count();
            for entry in &self.entries[from..to] {
                if let Some(last) = self.rest_matches(text, &found[i..], entry, &mut buf) {
                    spans.push((start, last));
                }
            }
        }
        spans
    }

    /// An ASCII token longer than every listed token can never match, so it is not lowercased.
    fn could_be(&self, token: &str) -> bool {
        !token.is_ascii() || token.len() <= self.longest_token
    }

    /// The end of the phrase when the tokens after the first one follow it in order.
    fn rest_matches(&self, text: &str, from: &[(usize, usize)], entry: &[String], buf: &mut String) -> Option<usize> {
        let window = from.get(..entry.len())?;
        for (&(s, e), want) in window.iter().zip(entry).skip(1) {
            lower_into(&text[s..e], buf);
            if buf != want {
                return None;
            }
        }
        Some(window[window.len() - 1].1)
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
    fn a_long_token_is_skipped_but_a_listed_one_of_the_same_start_is_found() {
        let l = list(&["spam"]);
        assert!(l.spans("spamspamspam").is_empty());
        assert_eq!(l.spans("SPAM spamspam"), vec![(0, 4)]);
    }
}
