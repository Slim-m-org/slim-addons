//! Parses dice expressions: `NdM`, sums of terms (`2d6+1d4+3`), keep and drop
//! (`4d6kh3`, `2d20kl1`, `4d6dl1`) and the d20 shorthand `adv` / `dis`
//! (`adv+5`, `d20 adv`, `d20dis-1`). `N` defaults to 1.

use std::fmt;

const USAGE: &str = "expected dice notation like 2d6+3, 4d6kh3, adv+5 or d20 dis";
const MAX_DICE: u32 = 1000;
const MAX_SIDES: u32 = 1_000_000;
const MAX_TERMS: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Keep {
    All,
    Highest(u32),
    Lowest(u32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiceTerm {
    pub count: u32,
    pub sides: u32,
    pub keep: Keep,
}

impl fmt::Display for DiceTerm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}d{}", self.count, self.sides)?;
        match self.keep {
            Keep::All => Ok(()),
            Keep::Highest(k) => write!(f, "kh{k}"),
            Keep::Lowest(k) => write!(f, "kl{k}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Term {
    Dice(DiceTerm),
    Flat(i64),
}

/// The signed terms of one expression, in the order they were written.
#[derive(Debug, PartialEq)]
pub struct Expr {
    pub terms: Vec<(bool, Term)>,
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn eat(&mut self, word: &str) -> bool {
        let hit = self.bytes[self.pos..].starts_with(word.as_bytes());
        if hit {
            self.pos += word.len();
        }
        hit
    }

    fn digits(&mut self) -> Result<Option<u32>, String> {
        let start = self.pos;
        while self.bytes.get(self.pos).is_some_and(u8::is_ascii_digit) {
            self.pos += 1;
        }
        if start == self.pos {
            return Ok(None);
        }
        if self.pos - start > 9 {
            return Err("that number is too large".to_string());
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos]).unwrap_or("0");
        Ok(text.parse().ok())
    }

    fn done(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn term(&mut self) -> Result<Term, String> {
        if self.eat("adv") {
            return Ok(Term::Dice(shorthand(Keep::Highest(1), 20)));
        }
        if self.eat("dis") {
            return Ok(Term::Dice(shorthand(Keep::Lowest(1), 20)));
        }
        let leading = self.digits()?;
        if !self.eat("d") {
            return leading.map(|n| Term::Flat(n as i64)).ok_or_else(|| USAGE.to_string());
        }
        let count = leading.unwrap_or(1);
        let sides = self.digits()?.ok_or_else(|| format!("missing side count: {USAGE}"))?;
        let (count, keep) = self.keep(count)?;
        if count == 0 || count > MAX_DICE {
            return Err(format!("dice count must be between 1 and {MAX_DICE}"));
        }
        if sides == 0 || sides > MAX_SIDES {
            return Err(format!("side count must be between 1 and {MAX_SIDES}"));
        }
        Ok(Term::Dice(DiceTerm { count, sides, keep }))
    }

    /// Reads an optional suffix after the side count; `adv` and `dis` turn one die into two.
    fn keep(&mut self, count: u32) -> Result<(u32, Keep), String> {
        if self.eat("adv") || self.eat("dis") {
            let advantage = self.bytes[self.pos - 1] == b'v';
            if count != 1 {
                return Err("adv and dis go on a single die, like d20 adv".to_string());
            }
            let keep = if advantage { Keep::Highest(1) } else { Keep::Lowest(1) };
            return Ok((2, keep));
        }
        for (word, high, dropping) in [("kh", true, false), ("kl", false, false), ("dh", true, true), ("dl", false, true)] {
            if !self.eat(word) {
                continue;
            }
            let n = self.digits()?.unwrap_or(1);
            return choose(count, n, high, dropping);
        }
        Ok((count, Keep::All))
    }
}

fn shorthand(keep: Keep, sides: u32) -> DiceTerm {
    DiceTerm { count: 2, sides, keep }
}

fn choose(count: u32, n: u32, high: bool, dropping: bool) -> Result<(u32, Keep), String> {
    let kept = if dropping { count.saturating_sub(n) } else { n.min(count) };
    if kept == 0 || (dropping && n == 0) {
        return Err("keep at least one die and drop fewer than you roll".to_string());
    }
    let keep = match (high, dropping) {
        (true, false) | (false, true) => Keep::Highest(kept),
        _ => Keep::Lowest(kept),
    };
    Ok((count, keep))
}

fn normalise(text: &str) -> String {
    text.to_ascii_lowercase()
        .replace("disadvantage", "dis")
        .replace("advantage", "adv")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

pub fn parse(text: &str) -> Result<Expr, String> {
    let cleaned = normalise(text);
    let mut cursor = Cursor { bytes: cleaned.as_bytes(), pos: 0 };
    let mut terms: Vec<(bool, Term)> = Vec::new();
    let mut dice = 0u32;
    loop {
        let negative = match cursor.bytes.get(cursor.pos) {
            Some(b'+') => {
                cursor.pos += 1;
                false
            }
            Some(b'-') => {
                cursor.pos += 1;
                true
            }
            _ if terms.is_empty() => false,
            _ => return Err(USAGE.to_string()),
        };
        let term = cursor.term()?;
        if let Term::Dice(d) = term {
            dice += d.count;
        }
        terms.push((negative, term));
        if cursor.done() {
            break;
        }
    }
    if terms.len() > MAX_TERMS || dice > MAX_DICE {
        return Err(format!("at most {MAX_TERMS} terms and {MAX_DICE} dice per roll"));
    }
    if !terms.iter().any(|(_, t)| matches!(t, Term::Dice(_))) {
        return Err(USAGE.to_string());
    }
    Ok(Expr { terms })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dice(text: &str) -> Vec<(bool, String)> {
        parse(text)
            .unwrap_or_else(|e| panic!("{text:?}: {e}"))
            .terms
            .iter()
            .map(|(neg, t)| match t {
                Term::Dice(d) => (*neg, d.to_string()),
                Term::Flat(n) => (*neg, format!("={n}")),
            })
            .collect()
    }

    fn refused(text: &str) -> String {
        parse(text).expect_err(text)
    }

    #[test]
    fn plain_notation_with_a_modifier_and_loose_spacing() {
        assert_eq!(dice("2d20+3"), [(false, "2d20".into()), (false, "=3".into())]);
        assert_eq!(dice(" D6 - 1 "), [(false, "1d6".into()), (true, "=1".into())]);
        assert_eq!(dice("2d6 + 1d4 + 3").len(), 3);
    }

    #[test]
    fn advantage_and_disadvantage_are_two_d20_keeping_one() {
        for text in ["adv", "advantage", "d20 adv", "1d20advantage", "Advantage"] {
            assert_eq!(dice(text), [(false, "2d20kh1".into())], "{text}");
        }
        for text in ["dis", "disadvantage", "d20dis", "d20 disadvantage"] {
            assert_eq!(dice(text), [(false, "2d20kl1".into())], "{text}");
        }
        assert_eq!(dice("adv+5")[1], (false, "=5".into()));
        assert_eq!(dice("d8 adv")[0].1, "2d8kh1");
    }

    #[test]
    fn keep_and_drop_forms_normalise_to_keep_highest_or_lowest() {
        assert_eq!(dice("4d6kh3")[0].1, "4d6kh3");
        assert_eq!(dice("4d6dl1")[0].1, "4d6kh3");
        assert_eq!(dice("4d6dh1")[0].1, "4d6kl3");
        assert_eq!(dice("2d20kl")[0].1, "2d20kl1");
        assert_eq!(dice("3d6kh9")[0].1, "3d6kh3");
    }

    #[test]
    fn nonsense_is_refused_with_the_usage() {
        for text in ["", "d", "2d", "20", "adb", "2d20+", "+5", "d20 adv adv"] {
            assert!(refused(text).contains("dice notation like"), "{text}: {}", refused(text));
        }
    }

    #[test]
    fn limits_and_bad_keeps_are_refused() {
        assert!(refused("0d6").contains("between 1 and 1000"));
        assert!(refused("1001d6").contains("between 1 and 1000"));
        assert!(refused("1d0").contains("between 1 and 1000000"));
        assert!(refused("3d6kh0").contains("keep at least one"));
        assert!(refused("3d6dl3").contains("keep at least one"));
        assert!(refused("2d20 adv").contains("single die"));
        assert!(refused("1d99999999999").contains("too large"));
        assert!(refused("600d6+600d6").contains("at most"));
    }
}
