//! Deterministic randomness: a module has no random source, so every roll is derived from the
//! world seed and the tick, which also makes a season replay the same from the same moves.

/// FNV-1a over bytes, 64 bits. Used to turn ids and seeds into numbers, never for anything secret.
pub fn hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Mixes several numbers into one well-spread seed.
pub fn mix(parts: &[u64]) -> u64 {
    let mut h = 0x9e37_79b9_7f4a_7c15u64;
    for p in parts {
        h ^= splitmix(h ^ p);
        h = h.rotate_left(27).wrapping_mul(0x94d0_49bb_1331_11eb);
    }
    splitmix(h)
}

fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A small, fast generator (splitmix64 stepping).
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        splitmix(self.0)
    }

    /// A number in `0..n`, or 0 when `n` is 0.
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        (((self.next() >> 32) * u64::from(n)) >> 32) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_gives_the_same_rolls() {
        let (mut a, mut b) = (Rng::new(7), Rng::new(7));
        for _ in 0..100 {
            assert_eq!(a.next(), b.next());
        }
    }

    #[test]
    fn below_stays_in_range_and_covers_it() {
        let mut r = Rng::new(mix(&[1, 2, 3]));
        let mut seen = [false; 6];
        for _ in 0..500 {
            let v = r.below(6) as usize;
            assert!(v < 6);
            seen[v] = true;
        }
        assert!(seen.iter().all(|s| *s));
        assert_eq!(r.below(0), 0);
    }

    #[test]
    fn hash_and_mix_spread_nearby_inputs() {
        assert_ne!(hash(b"a"), hash(b"b"));
        assert_ne!(mix(&[1, 0]), mix(&[0, 1]));
    }
}
