//! Reading "30m take the bread out" into a delay and a text.

/// Shortest and longest delay a reminder may have, in milliseconds.
pub const MIN_DELAY_MS: i64 = 60_000;
pub const MAX_DELAY_MS: i64 = 30 * 24 * 60 * 60_000;
/// Longest reminder text, in characters.
pub const MAX_TEXT_CHARS: usize = 200;

#[derive(Debug)]
pub struct Parsed {
    pub delay_ms: i64,
    pub text: String,
}

const USAGE: &str = "Say when and what: /remind 30m take the bread out (m, h or d)";

pub fn parse(input: &str) -> Result<Parsed, String> {
    let input = input.trim();
    let input = input.strip_prefix("in ").unwrap_or(input);
    let (when, text) = input.split_once(char::is_whitespace).unwrap_or((input, ""));
    let delay_ms = delay(when).ok_or_else(|| USAGE.to_string())?;
    let text = text.trim();
    if text.is_empty() {
        return Err(USAGE.to_string());
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(format!("Keep the reminder under {MAX_TEXT_CHARS} characters."));
    }
    if !(MIN_DELAY_MS..=MAX_DELAY_MS).contains(&delay_ms) {
        return Err("Pick a time between 1 minute and 30 days from now.".to_string());
    }
    Ok(Parsed { delay_ms, text: text.to_string() })
}

/// `90s`-less on purpose: the timer ticks every five minutes, so seconds mean nothing.
fn delay(word: &str) -> Option<i64> {
    let split = word.find(|c: char| !c.is_ascii_digit())?;
    let (digits, unit) = word.split_at(split);
    let count: i64 = digits.parse().ok()?;
    let unit_ms = match unit.to_ascii_lowercase().as_str() {
        "m" | "min" | "mins" => 60_000,
        "h" | "hr" | "hrs" => 3_600_000,
        "d" | "day" | "days" => 86_400_000,
        _ => return None,
    };
    count.checked_mul(unit_ms)
}

/// "12 minutes", "3 hours", "2 days": the coarsest unit that is still true.
pub fn words(ms: i64) -> String {
    let ms = ms.max(0);
    let (n, unit) = if ms < 3_600_000 {
        (ms / 60_000, "minute")
    } else if ms < 2 * 86_400_000 {
        (ms / 3_600_000, "hour")
    } else {
        (ms / 86_400_000, "day")
    };
    let n = n.max(1);
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_delay_and_a_text_are_read_in_any_of_the_spellings() {
        for input in ["30m take the bread out", "in 30min take the bread out", "30MIN  take the bread out"] {
            assert_eq!(parse(input).unwrap().delay_ms, 30 * 60_000, "{input}");
        }
        assert_eq!(parse("2h call mum").unwrap().delay_ms, 2 * 3_600_000);
        assert_eq!(parse("1d stand-up").unwrap().text, "stand-up");
    }

    #[test]
    fn malformed_input_says_how_to_write_it() {
        for input in ["", "30m", "soon bread", "30x bread", "m bread", "-5m bread"] {
            assert!(parse(input).unwrap_err().starts_with("Say when and what"), "{input}");
        }
    }

    #[test]
    fn the_range_and_the_length_are_held() {
        assert!(parse("0m x").unwrap_err().contains("between 1 minute and 30 days"));
        assert!(parse("31d x").unwrap_err().contains("between"));
        assert!(parse("30d x").is_ok());
        assert!(parse(&format!("5m {}", "x".repeat(MAX_TEXT_CHARS + 1))).unwrap_err().contains("under 200"));
        assert!(parse("99999999999999999999d x").is_err());
    }

    #[test]
    fn a_wait_reads_in_the_coarsest_true_unit() {
        assert_eq!(words(60_000), "1 minute");
        assert_eq!(words(29 * 60_000 + 59_000), "29 minutes");
        assert_eq!(words(3 * 3_600_000), "3 hours");
        assert_eq!(words(5 * 86_400_000), "5 days");
        assert_eq!(words(-5), "1 minute");
    }
}
