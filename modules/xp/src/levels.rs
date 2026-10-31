//! Levels: a pure function of total XP and the chosen curve.

/// The last level; keeps every number small and the badge short.
pub const MAX_LEVEL: i64 = 999;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Curve {
    Gentle,
    Standard,
    Steep,
}

impl Curve {
    pub fn parse(name: &str) -> Curve {
        match name {
            "gentle" => Curve::Gentle,
            "steep" => Curve::Steep,
            _ => Curve::Standard,
        }
    }
}

/// XP to climb from level `n` to `n + 1`.
pub fn cost_of_level(n: i64, curve: Curve) -> i64 {
    let n = n.clamp(1, MAX_LEVEL);
    match curve {
        Curve::Gentle => 50 * n,
        Curve::Standard => 100 * n,
        Curve::Steep => 100 * n * n,
    }
}

/// Total XP at which level `n` starts; level 1 starts at zero.
pub fn xp_for_level(n: i64, curve: Curve) -> i64 {
    let below = n.clamp(1, MAX_LEVEL) - 1;
    match curve {
        Curve::Gentle => 50 * below * (below + 1) / 2,
        Curve::Standard => 100 * below * (below + 1) / 2,
        Curve::Steep => 100 * below * (below + 1) * (2 * below + 1) / 6,
    }
}

/// Where a total sits: its level, the XP into it, and the XP the level asks (zero at the last level).
#[derive(Debug, PartialEq, Eq)]
pub struct Standing {
    pub level: i64,
    pub into: i64,
    pub needed: i64,
}

pub fn standing(xp: i64, curve: Curve) -> Standing {
    let xp = xp.max(0);
    let (mut lo, mut hi) = (1, MAX_LEVEL);
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        if xp_for_level(mid, curve) <= xp {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    if lo == MAX_LEVEL {
        return Standing { level: lo, into: 0, needed: 0 };
    }
    Standing { level: lo, into: xp - xp_for_level(lo, curve), needed: cost_of_level(lo, curve) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CURVES: [Curve; 3] = [Curve::Gentle, Curve::Standard, Curve::Steep];

    #[test]
    fn level_one_starts_at_zero_on_every_curve() {
        for curve in CURVES {
            assert_eq!(xp_for_level(1, curve), 0);
            assert_eq!(standing(0, curve), Standing { level: 1, into: 0, needed: cost_of_level(1, curve) });
        }
    }

    #[test]
    fn the_standard_curve_asks_a_hundred_times_the_level() {
        assert_eq!(xp_for_level(2, Curve::Standard), 100);
        assert_eq!(xp_for_level(3, Curve::Standard), 300);
        assert_eq!(xp_for_level(4, Curve::Standard), 600);
        assert_eq!(standing(780, Curve::Standard), Standing { level: 4, into: 180, needed: 400 });
    }

    #[test]
    fn gentle_is_easier_and_steep_is_harder_than_standard() {
        for level in 3..60 {
            assert!(xp_for_level(level, Curve::Gentle) < xp_for_level(level, Curve::Standard));
            assert!(xp_for_level(level, Curve::Standard) < xp_for_level(level, Curve::Steep));
        }
        assert_eq!(xp_for_level(3, Curve::Steep), 500);
    }

    #[test]
    fn the_closed_forms_match_adding_up_the_costs() {
        for curve in CURVES {
            let mut total = 0;
            for level in 1..=MAX_LEVEL {
                assert_eq!(xp_for_level(level, curve), total, "{curve:?} level {level}");
                total += cost_of_level(level, curve);
            }
        }
    }

    #[test]
    fn a_total_lands_on_the_level_whose_start_it_has_passed() {
        for curve in CURVES {
            for level in 1..MAX_LEVEL {
                let start = xp_for_level(level, curve);
                assert_eq!(standing(start, curve).level, level);
                assert_eq!(standing(xp_for_level(level + 1, curve) - 1, curve).level, level);
            }
        }
    }

    #[test]
    fn the_last_level_is_a_ceiling_and_negative_xp_is_zero() {
        for curve in CURVES {
            let top = xp_for_level(MAX_LEVEL, curve);
            assert_eq!(standing(top, curve), Standing { level: MAX_LEVEL, into: 0, needed: 0 });
            assert_eq!(standing(i64::MAX, curve).level, MAX_LEVEL);
            assert_eq!(standing(-5, curve).level, 1);
        }
    }

    #[test]
    fn an_unknown_curve_name_is_standard() {
        assert_eq!(Curve::parse("gentle"), Curve::Gentle);
        assert_eq!(Curve::parse("steep"), Curve::Steep);
        assert_eq!(Curve::parse("wild"), Curve::Standard);
    }
}
