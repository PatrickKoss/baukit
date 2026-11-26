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

    const SMALL: u64 = 15;

    fn small_or_max() -> u64 {
        let value: u64 = kani::any();
        kani::assume(value <= SMALL || value == u64::MAX);
        value
    }

    fn admit_matches_at_rate(rate: u64) {
        let last: u64 = kani::any();
        assert!(admit_matches(
            small_or_max(),
            last,
            last.saturating_add(small_or_max()),
            rate,
            small_or_max(),
            small_or_max(),
        ));
    }

    /// Twin and production `refill` and `admit` agree with a stopped refill.
    /// Bounds: rate 0; tokens, elapsed time, capacity and cost each `<= 15` or `u64::MAX`.
    #[kani::proof]
    fn admit_equivalent_rate_zero() {
        admit_matches_at_rate(0);
    }

    /// The same with the smallest non-zero rate.
    /// Bounds: rate 1; tokens, elapsed time, capacity and cost each `<= 15` or `u64::MAX`.
    #[kani::proof]
    fn admit_equivalent_rate_one() {
        admit_matches_at_rate(1);
    }

    /// The same with a rate that leaves a remainder in `retry_ms`.
    /// Bounds: rate 7; tokens, elapsed time, capacity and cost each `<= 15` or `u64::MAX`.
    #[kani::proof]
    fn admit_equivalent_rate_seven() {
        admit_matches_at_rate(7);
    }

    /// The same with a rate whose refill saturates after one millisecond.
    /// Bounds: rate `u64::MAX`; tokens, elapsed time, capacity and cost each `<= 15` or `u64::MAX`.
    #[kani::proof]
    fn admit_equivalent_rate_max() {
        admit_matches_at_rate(u64::MAX);
    }
}
