//! A small seeded generator for the operation mix and kill times.
//!
//! The campaign must be reproducible from its recorded seeds, and needs no
//! cryptographic quality, so it uses `SplitMix64` rather than a dependency.

/// Steele, Lea and Flood's `SplitMix64`.
#[derive(Clone, Debug)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// A generator seeded with `seed`.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// The next 64 random bits.
    pub const fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    /// A value in `0..bound` (0 when `bound` is 0). The modulo bias is
    /// irrelevant for choosing operations.
    pub const fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 { 0 } else { self.next() % bound }
    }
}

#[cfg(test)]
mod tests {
    use super::SplitMix64;

    #[test]
    fn the_sequence_is_the_published_one_and_stays_in_bounds() {
        // The first outputs for seed 0 in the reference implementation.
        let mut rng = SplitMix64::new(0);
        assert_eq!(rng.next(), 0xe220_a839_7b1d_cdaf);
        assert_eq!(rng.next(), 0x6e78_9e6a_a1b9_65f4);
        let mut rng = SplitMix64::new(7);
        assert!((0..1_000).all(|_| rng.below(10) < 10));
        assert_eq!(rng.below(0), 0);
    }
}
