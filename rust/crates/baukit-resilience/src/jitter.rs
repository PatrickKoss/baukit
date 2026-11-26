//! The randomness port for jitter.

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;

const SPLITMIX_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
const SPLITMIX_MUL_1: u64 = 0xBF58_476D_1CE4_E5B9;
const SPLITMIX_MUL_2: u64 = 0x94D0_49BB_1331_11EB;
const FRACTION_SCALE: f64 = 4_294_967_296.0;
const HIGH_HALF_SHIFT: u32 = 32;

/// Source of jitter fractions.
pub trait JitterSource: Send {
    /// Returns a value in `[0, 1)`.
    fn next_fraction(&mut self) -> f64;
}

/// SplitMix64 generator. Not cryptographic, only meant to spread retries.
#[derive(Debug, Clone)]
pub struct SplitMixJitter {
    state: u64,
}

impl SplitMixJitter {
    /// Seeds the generator from the process-random [`RandomState`].
    #[must_use]
    pub fn from_entropy() -> Self {
        Self::with_seed(RandomState::new().hash_one(SPLITMIX_GAMMA))
    }

    /// Seeds the generator with a fixed value.
    #[must_use]
    pub const fn with_seed(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(SPLITMIX_GAMMA);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(SPLITMIX_MUL_1);
        z = (z ^ (z >> 27)).wrapping_mul(SPLITMIX_MUL_2);
        z ^ (z >> 31)
    }
}

impl Default for SplitMixJitter {
    fn default() -> Self {
        Self::from_entropy()
    }
}

impl JitterSource for SplitMixJitter {
    fn next_fraction(&mut self) -> f64 {
        let high = u32::try_from(self.next_u64() >> HIGH_HALF_SHIFT).unwrap_or(0);
        f64::from(high) / FRACTION_SCALE
    }
}

/// Clamps `fraction` into `[0, 1)`; NaN and negatives become `0`.
pub(crate) fn sanitize(fraction: f64) -> f64 {
    if fraction.is_nan() || fraction < 0.0 {
        return 0.0;
    }
    if fraction >= 1.0 {
        return 1.0 - f64::EPSILON;
    }
    fraction
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractions_stay_in_unit_interval() {
        let mut source = SplitMixJitter::with_seed(7);
        for _ in 0..10_000 {
            let fraction = source.next_fraction();
            assert!((0.0..1.0).contains(&fraction));
        }
    }

    #[test]
    fn sanitize_clamps_out_of_range_values() {
        assert!(sanitize(f64::NAN).abs() < f64::EPSILON);
        assert!(sanitize(-1.0).abs() < f64::EPSILON);
        assert!(sanitize(1.0) < 1.0);
        assert!((sanitize(0.25) - 0.25).abs() < f64::EPSILON);
    }
}
