use baukit_sync::hlc as production;
use baukit_verus::hlc as twin;

#[must_use]
pub fn same_error(production: production::HlcError, twin: twin::HlcError) -> bool {
    matches!(
        (production, twin),
        (
            production::HlcError::InvalidComponent,
            twin::HlcError::InvalidComponent
        ) | (
            production::HlcError::InvalidTimestamp,
            twin::HlcError::InvalidTimestamp
        ) | (
            production::HlcError::ExceedsSafeInteger,
            twin::HlcError::ExceedsSafeInteger
        ) | (
            production::HlcError::InvalidPhysicalClock,
            twin::HlcError::InvalidPhysicalClock
        ) | (
            production::HlcError::RemoteTooFarAhead,
            twin::HlcError::RemoteTooFarAhead
        ) | (
            production::HlcError::InvalidMaxDrift,
            twin::HlcError::InvalidMaxDrift
        )
    )
}

#[must_use]
pub fn same<T: PartialEq>(
    production: Result<T, production::HlcError>,
    twin: Result<T, twin::HlcError>,
) -> bool {
    match (production, twin) {
        (Ok(left), Ok(right)) => left == right,
        (Err(left), Err(right)) => same_error(left, right),
        _ => false,
    }
}

#[must_use]
pub fn now_matches(state: (i64, i64), physical: i64) -> bool {
    same(
        production::now_step(state, physical),
        twin::now_step(state, physical),
    )
}

#[must_use]
pub fn observe_matches(state: (i64, i64), physical: i64, remote: i64, max_drift_ms: i64) -> bool {
    same(
        production::observe_step(state, physical, remote, max_drift_ms),
        twin::observe_step(state, physical, remote, max_drift_ms),
    )
}

#[must_use]
pub fn codec_matches(wall: i64, counter: i64, timestamp: i64) -> bool {
    same(
        production::encode(wall, counter),
        twin::encode(wall, counter),
    ) && same(production::decode(timestamp), twin::decode(timestamp))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const MAX: i64 = production::MAX_ENCODED_TIMESTAMP;

    fn component() -> impl Strategy<Value = i64> {
        prop_oneof![
            -2_i64..2_000,
            MAX / 1_000 - 2..MAX / 1_000 + 2,
            MAX - 2..=MAX + 2,
            any::<i64>(),
        ]
    }

    fn counter() -> impl Strategy<Value = i64> {
        prop_oneof![-1_i64..=1_000, any::<i64>()]
    }

    proptest! {
        #[test]
        fn now_step_matches(wall in component(), count in counter(), physical in component()) {
            prop_assert!(now_matches((wall, count), physical));
        }

        #[test]
        fn observe_step_matches(
            wall in component(),
            count in counter(),
            physical in component(),
            remote in component(),
            drift in component(),
        ) {
            prop_assert!(observe_matches((wall, count), physical, remote, drift));
        }

        #[test]
        fn codec_matches_production(wall in component(), count in counter(), timestamp in component()) {
            prop_assert!(codec_matches(wall, count, timestamp));
        }
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// V-01: twin and production `now_step` agree on every input.
    /// Bounds: none, straight-line code over unconstrained `i64`.
    #[kani::proof]
    fn now_step_equivalent() {
        assert!(now_matches((kani::any(), kani::any()), kani::any()));
    }

    /// V-01: twin and production `observe_step` agree on every input.
    /// Bounds: none, straight-line code over unconstrained `i64`.
    #[kani::proof]
    fn observe_step_equivalent() {
        assert!(observe_matches(
            (kani::any(), kani::any()),
            kani::any(),
            kani::any(),
            kani::any(),
        ));
    }

    /// V-01: twin and production `encode` and `decode` agree on every input.
    /// Bounds: none, straight-line code over unconstrained `i64`.
    #[kani::proof]
    fn codec_equivalent() {
        assert!(codec_matches(kani::any(), kani::any(), kani::any()));
    }
}
