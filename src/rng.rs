//! Seeded, serializable randomness. The whole game is deterministic given a
//! seed and an action log.

use rand_chacha::{
    ChaCha8Rng,
    rand_core::{Rng as _, SeedableRng},
};
use serde::{Deserialize, Serialize};

/// Deterministic random number generator that lives inside the game state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rng(ChaCha8Rng);

impl Rng {
    /// Creates a generator from a seed.
    pub fn from_seed(seed: u64) -> Self {
        Self(ChaCha8Rng::seed_from_u64(seed))
    }

    /// Uniform integer in `0..n`, free of modulo bias.
    ///
    /// # Panics
    /// Panics when `n == 0`.
    pub fn below(&mut self, n: usize) -> usize {
        assert!(n > 0, "Rng::below(0)");
        let n = u64::try_from(n).expect("usize fits in u64");
        // Reject the incomplete last bucket so every residue is equally likely.
        let limit = u64::MAX - u64::MAX % n;
        loop {
            let x = self.0.next_u64();
            if x < limit {
                return usize::try_from(x % n).expect("value below n fits in usize");
            }
        }
    }

    /// Fisher-Yates shuffle.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i + 1);
            items.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sequence() {
        let (mut a, mut b) = (Rng::from_seed(7), Rng::from_seed(7));
        let xs: Vec<_> = (0..20).map(|_| a.below(100)).collect();
        let ys: Vec<_> = (0..20).map(|_| b.below(100)).collect();
        assert_eq!(xs, ys);
    }

    #[test]
    fn below_stays_in_range() {
        let mut r = Rng::from_seed(1);
        assert!((0..1000).all(|_| r.below(3) < 3));
    }

    #[test]
    fn shuffle_is_a_deterministic_permutation() {
        let mut a: Vec<u32> = (0..50).collect();
        let mut b = a.clone();
        Rng::from_seed(3).shuffle(&mut a);
        Rng::from_seed(3).shuffle(&mut b);
        assert_eq!(a, b);
        assert_ne!(a, (0..50).collect::<Vec<_>>());
        a.sort_unstable();
        assert_eq!(a, (0..50).collect::<Vec<_>>());
    }

    #[test]
    fn serde_round_trip_continues_the_stream() {
        let mut a = Rng::from_seed(9);
        a.below(10);
        let mut b: Rng = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(a.below(1000), b.below(1000));
    }
}
