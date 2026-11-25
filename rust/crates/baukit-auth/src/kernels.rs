//! Pure decision kernels shared by the verifier and its proofs.

use std::time::Duration;

/// Why a token's validity window excludes the current time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TimeError {
    Expired,
    NotYetValid,
}

/// Returns whether a JWKS refetch may start, given the last attempt.
///
/// Times are monotonic offsets from one origin chosen by the caller.
pub(crate) fn jwks_refresh_allowed(
    last_attempt: Option<Duration>,
    now: Duration,
    cooldown: Duration,
) -> bool {
    last_attempt.is_none_or(|last| now.saturating_sub(last) >= cooldown)
}

/// Checks `nbf - skew <= now <= exp + skew` without overflow.
pub(crate) fn claims_time_valid(
    exp: i64,
    nbf: Option<i64>,
    now: i64,
    skew: i64,
) -> Result<(), TimeError> {
    let now = i128::from(now);
    let skew = i128::from(skew);
    if now > i128::from(exp) + skew {
        return Err(TimeError::Expired);
    }
    if nbf.is_some_and(|not_before| i128::from(not_before) - skew > now) {
        return Err(TimeError::NotYetValid);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_waits_for_the_full_cooldown() {
        let start = Duration::from_secs(100);
        let cooldown = Duration::from_secs(30);
        assert!(jwks_refresh_allowed(None, start, cooldown));
        assert!(!jwks_refresh_allowed(
            Some(start),
            start + Duration::from_secs(29),
            cooldown
        ));
        assert!(jwks_refresh_allowed(
            Some(start),
            start + cooldown,
            cooldown
        ));
        assert!(!jwks_refresh_allowed(
            Some(start + cooldown),
            start,
            cooldown
        ));
    }

    #[test]
    fn time_window_is_inclusive_and_never_overflows() {
        assert_eq!(claims_time_valid(100, None, 160, 60), Ok(()));
        assert_eq!(
            claims_time_valid(100, None, 161, 60),
            Err(TimeError::Expired)
        );
        assert_eq!(claims_time_valid(1000, Some(200), 140, 60), Ok(()));
        assert_eq!(
            claims_time_valid(1000, Some(200), 139, 60),
            Err(TimeError::NotYetValid)
        );
        assert_eq!(
            claims_time_valid(i64::MAX, Some(i64::MIN), i64::MAX, i64::MAX),
            Ok(())
        );
        assert_eq!(
            claims_time_valid(i64::MIN, None, i64::MAX, 0),
            Err(TimeError::Expired)
        );
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    const STEPS: usize = 8;

    /// K-05: over 8 symbolic, non-decreasing `now` offsets (whole seconds below
    /// 2^20), a caller that records each allowed attempt is allowed at most
    /// `1 + span / cooldown` times. Unwind 9.
    #[kani::proof]
    #[kani::unwind(9)]
    fn refresh_allowed_is_bounded_per_span() {
        let cooldown_secs: u64 = kani::any();
        kani::assume(cooldown_secs > 0 && cooldown_secs < 1 << 20);
        let cooldown = Duration::from_secs(cooldown_secs);
        let mut last = None;
        let mut previous = 0_u64;
        let mut first = None;
        let mut allowed = 0_u64;
        for _ in 0..STEPS {
            let offset: u64 = kani::any();
            kani::assume(offset >= previous && offset < 1 << 20);
            previous = offset;
            let now = Duration::from_secs(offset);
            if jwks_refresh_allowed(last, now, cooldown) {
                allowed += 1;
                last = Some(now);
                first.get_or_insert(offset);
            }
        }
        let span = first.map_or(0, |first| previous - first);
        assert!(allowed <= 1 + span / cooldown_secs);
    }

    /// K-06: for every `i64` input the kernel never overflows, and it accepts
    /// iff `nbf - skew <= now <= exp + skew` in exact integer arithmetic.
    #[kani::proof]
    fn claims_time_valid_matches_exact_window() {
        let exp: i64 = kani::any();
        let nbf: Option<i64> = kani::any();
        let now: i64 = kani::any();
        let skew: i64 = kani::any();
        let result = claims_time_valid(exp, nbf, now, skew);
        let (exp, now, skew) = (i128::from(exp), i128::from(now), i128::from(skew));
        let not_expired = now <= exp + skew;
        let started = nbf.is_none_or(|nbf| i128::from(nbf) - skew <= now);
        assert_eq!(result.is_ok(), not_expired && started);
        if !not_expired {
            assert_eq!(result, Err(TimeError::Expired));
        }
    }
}
