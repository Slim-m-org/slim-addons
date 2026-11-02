//! How much matching work a pattern can cause. A bounded repeat multiplies the states the
//! lazy DFA tracks, and nesting multiplies again, so `.{0,20}` inside `{3}` costs like
//! sixty; measured against the 10M fuel a hook gets, the caps below keep a 4000-character
//! message well inside it.

use regex_syntax::hir::{Hir, HirKind};
use regex_syntax::ParserBuilder;

pub const MAX_PATTERN_WEIGHT: u32 = 20;
pub const MAX_TOTAL_WEIGHT: u32 = 40;

/// The pattern's weight, or why it cannot be used.
pub fn of(pattern: &str) -> Result<u32, String> {
    let hir = ParserBuilder::new()
        .unicode(false)
        .utf8(false)
        .nest_limit(16)
        .build()
        .parse(pattern)
        .map_err(|_| format!("the pattern {pattern} is not a valid regex"))?;
    Ok(weigh(&hir))
}

fn weigh(hir: &Hir) -> u32 {
    match hir.kind() {
        HirKind::Empty | HirKind::Look(_) => 0,
        HirKind::Literal(_) | HirKind::Class(_) => 1,
        HirKind::Repetition(rep) => {
            let times = rep.max.unwrap_or(rep.min.saturating_add(1)).max(1);
            times.saturating_mul(weigh(&rep.sub))
        }
        HirKind::Capture(cap) => weigh(&cap.sub),
        HirKind::Concat(parts) | HirKind::Alternation(parts) => {
            parts.iter().fold(0u32, |sum, part| sum.saturating_add(weigh(part)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_and_unbounded_loops_are_cheap() {
        assert_eq!(of("free crypto").unwrap(), 1);
        assert_eq!(of(r"https?://\S+\.ru").unwrap(), 6);
        assert_eq!(of("(a+)+$").unwrap(), 4);
    }

    #[test]
    fn bounded_repeats_multiply_and_nesting_multiplies_again() {
        assert_eq!(of(".{1,20}x").unwrap(), 21);
        assert_eq!(of("(?:.{0,20}){3}x").unwrap(), 61);
        assert_eq!(of(r"\d{3}-\d{4}").unwrap(), 8);
    }

    #[test]
    fn a_huge_count_saturates_instead_of_overflowing() {
        assert!(of("(?:(?:a{1000}){1000}){1000}").unwrap() > MAX_PATTERN_WEIGHT);
    }
}
