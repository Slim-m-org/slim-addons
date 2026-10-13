//! A deterministic splitmix64 counter, seeded from the run's `entropy` and the
//! request's input bytes.
//!
//! A module has no clock or random source, so the host hands each run a fresh
//! `entropy` string and the seed is an FNV-1a hash of it and the input. It is
//! the same construction the dice module uses.

/// Anything that can roll one die; tests inject a scripted one.
pub trait Dice {
    /// Returns a value in `1..=sides`.
    fn roll(&mut self, sides: u32) -> u32;
}

pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// Folds `entropy`, a separator and `input` through FNV-1a into a seed.
    pub fn seeded(entropy: &[u8], input: &[u8]) -> Self {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut fold = |bytes: &[u8]| {
            for &byte in bytes {
                hash ^= byte as u64;
                hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
            }
        };
        if !entropy.is_empty() {
            fold(entropy);
            fold(&[0]);
        }
        fold(input);
        Self { state: hash }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

impl Dice for SplitMix64 {
    fn roll(&mut self, sides: u32) -> u32 {
        1 + (self.next_u64() % sides as u64) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first(entropy: &str, input: &str) -> u64 {
        SplitMix64::seeded(entropy.as_bytes(), input.as_bytes()).next_u64()
    }

    #[test]
    fn the_same_seed_rolls_the_same_and_entropy_changes_it() {
        assert_eq!(first("abc", "d20"), first("abc", "d20"));
        let seen: std::collections::BTreeSet<u64> =
            (0..50).map(|i| first(&format!("run-{i}"), "d20")).collect();
        assert_eq!(seen.len(), 50);
    }

    #[test]
    fn entropy_and_input_cannot_trade_bytes() {
        assert_ne!(first("ab", "c"), first("a", "bc"));
    }

    #[test]
    fn rolls_stay_within_the_sides() {
        let mut rng = SplitMix64::seeded(b"x", b"y");
        assert!((0..500).all(|_| (1..=6).contains(&rng.roll(6))));
        assert!((0..50).all(|_| rng.roll(1) == 1));
    }
}
