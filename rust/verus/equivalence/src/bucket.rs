use baukit_ratelimit::kernels::bucket as production;
use baukit_verus::bucket as twin;

#[must_use]
pub fn admit_matches(
    tokens: u64,
    last_ms: u64,
    now_ms: u64,
    rate: u64,
    capacity: u64,
    cost: u64,
) -> bool {
    production::refill(tokens, last_ms, now_ms, rate, capacity)
        == twin::refill(tokens, last_ms, now_ms, rate, capacity)
        && production::admit(tokens, last_ms, now_ms, rate, capacity, cost)
            == twin::admit(tokens, last_ms, now_ms, rate, capacity, cost)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn unit() -> impl Strategy<Value = u64> {
        prop_oneof![0_u64..4_096, u64::MAX - 4..=u64::MAX, any::<u64>()]
    }

    proptest! {
        #[test]
        fn admit_matches_production(
            tokens in unit(),
            last in unit(),
            now in unit(),
            rate in unit(),
            capacity in unit(),
            cost in unit(),
        ) {
            prop_assert!(admit_matches(tokens, last, now, rate, capacity, cost));
        }
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// Twin and production `refill` and `admit` agree on every input.
    /// Bounds: none, straight-line code over unconstrained `u64`.
    #[kani::proof]
    fn admit_equivalent() {
        assert!(admit_matches(
            kani::any(),
            kani::any(),
            kani::any(),
            kani::any(),
            kani::any(),
            kani::any(),
        ));
    }
}
