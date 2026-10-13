//! Rolls a parsed expression and lays the result out so every die is visible.

use crate::notation::{self, DiceTerm, Expr, Keep, Term};
use crate::rng::{Dice, SplitMix64};

pub enum Part {
    Dice { negative: bool, term: DiceTerm, rolls: Vec<u32>, kept: Vec<bool> },
    Flat(i64),
}

pub struct Outcome {
    pub total: i64,
    pub parts: Vec<Part>,
}

fn kept_mask(rolls: &[u32], keep: Keep) -> Vec<bool> {
    let wanted = match keep {
        Keep::All => rolls.len(),
        Keep::Highest(k) | Keep::Lowest(k) => k as usize,
    };
    let mut order: Vec<usize> = (0..rolls.len()).collect();
    match keep {
        Keep::Lowest(_) => order.sort_by_key(|&i| (rolls[i], i)),
        _ => order.sort_by_key(|&i| (std::cmp::Reverse(rolls[i]), i)),
    }
    let mut mask = vec![false; rolls.len()];
    for &i in order.iter().take(wanted) {
        mask[i] = true;
    }
    mask
}

pub fn evaluate(expr: &Expr, rng: &mut impl Dice) -> Outcome {
    let mut total = 0i64;
    let mut parts = Vec::new();
    for &(negative, term) in &expr.terms {
        let sign = if negative { -1 } else { 1 };
        match term {
            Term::Flat(n) => {
                total += sign * n;
                parts.push(Part::Flat(sign * n));
            }
            Term::Dice(term) => {
                let rolls: Vec<u32> = (0..term.count).map(|_| rng.roll(term.sides)).collect();
                let kept = kept_mask(&rolls, term.keep);
                let sum: i64 = rolls.iter().zip(&kept).filter(|(_, &k)| k).map(|(&r, _)| r as i64).sum();
                total += sign * sum;
                parts.push(Part::Dice { negative, term, rolls, kept });
            }
        }
    }
    Outcome { total, parts }
}

fn expression_text(outcome: &Outcome) -> String {
    let mut text = String::new();
    for (i, part) in outcome.parts.iter().enumerate() {
        let (negative, body) = match part {
            Part::Dice { negative, term, .. } => (*negative, term.to_string()),
            Part::Flat(n) => (*n < 0, n.abs().to_string()),
        };
        match (i, negative) {
            (0, false) => {}
            (0, true) | (_, true) => text.push('-'),
            (_, false) => text.push('+'),
        }
        text.push_str(&body);
    }
    text
}

fn natural(outcome: &Outcome) -> Option<&'static str> {
    let mut d20s = outcome.parts.iter().filter_map(|part| match part {
        Part::Dice { negative: false, term, rolls, kept } if term.sides == 20 => Some((rolls, kept)),
        _ => None,
    });
    let (rolls, kept) = d20s.next()?;
    if d20s.next().is_some() || kept.iter().filter(|&&k| k).count() != 1 {
        return None;
    }
    let value = rolls.iter().zip(kept).find(|(_, &k)| k)?.0;
    match value {
        20 => Some(" (natural 20)"),
        1 => Some(" (natural 1)"),
        _ => None,
    }
}

pub fn render(outcome: &Outcome) -> String {
    let mut out = format!("{} = {}{}", expression_text(outcome), outcome.total, natural(outcome).unwrap_or(""));
    let mut modifier = 0i64;
    let mut has_flat = false;
    for part in &outcome.parts {
        match part {
            Part::Flat(n) => {
                modifier += n;
                has_flat = true;
            }
            Part::Dice { negative, term, rolls, kept } => {
                let list = |want: bool| -> String {
                    let picked: Vec<String> =
                        rolls.iter().zip(kept).filter(|(_, &k)| k == want).map(|(r, _)| r.to_string()).collect();
                    picked.join(", ")
                };
                out.push_str(&format!("\n{}{}: {}", if *negative { "-" } else { "" }, term, list(true)));
                if kept.iter().any(|&k| !k) {
                    out.push_str(&format!(" (dropped {})", list(false)));
                }
            }
        }
    }
    if has_flat {
        out.push_str(&format!("\nmodifier {modifier:+}"));
    }
    out
}

/// The `roll` command: parses `input`, rolls it seeded from the run's entropy, and renders the result.
pub fn run(input: &str, entropy: &str) -> Result<String, String> {
    let expr = notation::parse(input)?;
    let mut rng = SplitMix64::seeded(entropy.as_bytes(), input.as_bytes());
    Ok(render(&evaluate(&expr, &mut rng)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hands out scripted faces in order, so a roll is exact.
    struct Scripted(std::vec::IntoIter<u32>);

    impl Dice for Scripted {
        fn roll(&mut self, sides: u32) -> u32 {
            let face = self.0.next().expect("script ran out");
            assert!(face >= 1 && face <= sides, "{face} is not on a d{sides}");
            face
        }
    }

    fn rolled(text: &str, faces: &[u32]) -> String {
        let expr = notation::parse(text).unwrap();
        render(&evaluate(&expr, &mut Scripted(faces.to_vec().into_iter())))
    }

    #[test]
    fn a_plain_roll_sums_the_dice_and_the_modifier() {
        assert_eq!(rolled("2d6+3", &[4, 5]), "2d6+3 = 12\n2d6: 4, 5\nmodifier +3");
        assert_eq!(rolled("d20-2", &[11]), "1d20-2 = 9\n1d20: 11\nmodifier -2");
    }

    #[test]
    fn advantage_keeps_the_higher_and_shows_the_dropped_die() {
        assert_eq!(rolled("adv+5", &[17, 3]), "2d20kh1+5 = 22\n2d20kh1: 17 (dropped 3)\nmodifier +5");
        assert_eq!(rolled("d20 adv", &[3, 17]), "2d20kh1 = 17\n2d20kh1: 17 (dropped 3)");
    }

    #[test]
    fn disadvantage_keeps_the_lower() {
        assert_eq!(rolled("dis", &[17, 1]), "2d20kl1 = 1 (natural 1)\n2d20kl1: 1 (dropped 17)");
    }

    #[test]
    fn keep_highest_of_four_drops_the_lowest() {
        let out = rolled("4d6kh3", &[2, 6, 3, 5]);
        assert_eq!(out, "4d6kh3 = 14\n4d6kh3: 6, 3, 5 (dropped 2)");
        assert_eq!(rolled("4d6dl1", &[2, 6, 3, 5]), "4d6kh3 = 14\n4d6kh3: 6, 3, 5 (dropped 2)");
    }

    #[test]
    fn ties_keep_the_earlier_die_and_dropped_dice_never_count() {
        assert_eq!(rolled("3d6kh2", &[4, 4, 4]), "3d6kh2 = 8\n3d6kh2: 4, 4 (dropped 4)");
    }

    #[test]
    fn subtracted_dice_reduce_the_total() {
        assert_eq!(rolled("d20-1d4", &[10, 3]), "1d20-1d4 = 7\n1d20: 10\n-1d4: 3");
    }

    #[test]
    fn natural_twenty_only_counts_for_a_single_kept_d20() {
        assert!(rolled("d20+4", &[20]).starts_with("1d20+4 = 24 (natural 20)"));
        assert!(!rolled("2d20", &[20, 3]).contains("natural"));
        assert!(!rolled("d20+d20", &[20, 3]).contains("natural"));
        assert!(!rolled("d12", &[12]).contains("natural"));
    }

    #[test]
    fn the_seeded_run_is_stable_and_varies_with_entropy() {
        assert_eq!(run("4d10kh2+1", "seed").unwrap(), run("4d10kh2+1", "seed").unwrap());
        let seen: std::collections::BTreeSet<String> =
            (0..40).map(|i| run("d20", &format!("run-{i}")).unwrap()).collect();
        assert!(seen.len() > 5, "{} distinct", seen.len());
        assert!(run("banana", "x").is_err());
    }

    #[test]
    fn a_thousand_dice_fit_in_one_roll() {
        let out = run("1000d6", "seed").unwrap();
        assert!(out.starts_with("1000d6 = "));
    }
}
