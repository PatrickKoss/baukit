use std::time::Duration;

use chrono::{DateTime, Utc};
use thiserror::Error;

/// A fixed, whole-second UTC interval anchored at the Unix epoch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixedUtcInterval {
    seconds: i64,
}

impl FixedUtcInterval {
    /// Creates an interval.
    ///
    /// The duration must contain at least one whole second and no fractional
    /// second. Whole-second intervals keep slot identifiers stable across
    /// runtimes and database timestamp precision settings.
    pub fn new(duration: Duration) -> Result<Self, FixedUtcIntervalError> {
        if duration.is_zero() {
            return Err(FixedUtcIntervalError::Zero);
        }
        if duration.subsec_nanos() != 0 {
            return Err(FixedUtcIntervalError::Subsecond);
        }
        let seconds =
            i64::try_from(duration.as_secs()).map_err(|_| FixedUtcIntervalError::OutOfRange)?;
        Ok(Self { seconds })
    }

    /// Returns the slot containing `observed_at`.
    pub fn slot_at(
        self,
        observed_at: DateTime<Utc>,
    ) -> Result<FixedUtcSlot, FixedUtcIntervalError> {
        let starts_at_seconds = observed_at
            .timestamp()
            .div_euclid(self.seconds)
            .checked_mul(self.seconds)
            .ok_or(FixedUtcIntervalError::OutOfRange)?;
        let starts_at = DateTime::from_timestamp(starts_at_seconds, 0)
            .ok_or(FixedUtcIntervalError::OutOfRange)?;
        Ok(FixedUtcSlot { starts_at })
    }

    /// Returns the first wall-clock slot after both inputs.
    ///
    /// Pass the slot stored in the current job and the clock time observed by
    /// its handler. A delayed handler skips missed slots. A clock that moves
    /// backward cannot schedule a slot at or before the current one.
    pub fn next_slot(
        self,
        current: FixedUtcSlot,
        observed_at: DateTime<Utc>,
    ) -> Result<FixedUtcSlot, FixedUtcIntervalError> {
        let observed = self.slot_at(observed_at)?;
        let latest_start = current.starts_at.max(observed.starts_at);
        let starts_at = latest_start
            .checked_add_signed(chrono::Duration::seconds(self.seconds))
            .ok_or(FixedUtcIntervalError::OutOfRange)?;
        Ok(FixedUtcSlot { starts_at })
    }

    /// Returns a slot's start plus a deterministic per-key offset.
    ///
    /// The offset is whole seconds in
    /// `0..=min(floor(interval_seconds * fraction), interval_seconds - 1)`.
    /// The bound uses IEEE 754 binary64 multiplication. The run time stays
    /// inside its slot, including when the fraction is one.
    /// Keys are hashed as UTF-8 bytes with 64-bit FNV-1a, using offset basis
    /// `14695981039346656037` and prime `1099511628211`, with wrapping arithmetic.
    /// The hash modulo `bound + 1` is the offset. This algorithm is fixed across
    /// releases. A key keeps its offset across slots and process restarts.
    /// Slot identifiers and [`Self::next_slot`] do not depend on jitter.
    ///
    /// ```
    /// use std::time::Duration;
    /// use baukit_jobs::{FixedUtcInterval, FixedUtcJitter};
    /// use chrono::DateTime;
    ///
    /// let interval = FixedUtcInterval::new(Duration::from_secs(3600))?;
    /// let slot = interval.slot_at(DateTime::UNIX_EPOCH)?;
    /// let jitter = FixedUtcJitter::new(0.1)?;
    /// let run_at = interval.run_at(slot, "connection:1", jitter)?;
    /// assert_eq!(run_at.timestamp(), 345);
    /// assert_eq!(slot.identifier(), "fixed-utc:0");
    /// # Ok::<(), baukit_jobs::FixedUtcIntervalError>(())
    /// ```
    pub fn run_at(
        self,
        slot: FixedUtcSlot,
        key: &str,
        jitter: FixedUtcJitter,
    ) -> Result<DateTime<Utc>, FixedUtcIntervalError> {
        let bound =
            ((self.seconds as f64 * jitter.fraction).floor() as u64).min(self.seconds as u64 - 1);
        let offset = fnv1a(key.as_bytes()) % (bound + 1);
        let offset = chrono::Duration::try_seconds(offset as i64)
            .ok_or(FixedUtcIntervalError::OutOfRange)?;
        slot.starts_at
            .checked_add_signed(offset)
            .ok_or(FixedUtcIntervalError::OutOfRange)
    }
}

/// A validated fraction of a fixed UTC interval available for per-key jitter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FixedUtcJitter {
    fraction: f64,
}

impl FixedUtcJitter {
    /// Creates a jitter bound with a finite fraction in `(0, 1]`.
    pub fn new(fraction: f64) -> Result<Self, FixedUtcIntervalError> {
        if !fraction.is_finite() || fraction <= 0.0 || fraction > 1.0 {
            return Err(FixedUtcIntervalError::InvalidJitterFraction);
        }
        Ok(Self { fraction })
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(14_695_981_039_346_656_037, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(1_099_511_628_211)
    })
}

/// One fixed recurring UTC slot.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct FixedUtcSlot {
    starts_at: DateTime<Utc>,
}

impl FixedUtcSlot {
    /// Returns the UTC boundary at which the slot starts.
    #[must_use]
    pub const fn starts_at(self) -> DateTime<Utc> {
        self.starts_at
    }

    /// Returns the canonical idempotency identifier for this UTC slot.
    #[must_use]
    pub fn identifier(self) -> String {
        format!("fixed-utc:{}", self.starts_at.timestamp())
    }
}

/// Invalid fixed UTC interval, jitter fraction, or timestamp outside Chrono's range.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum FixedUtcIntervalError {
    /// The interval is zero.
    #[error("fixed UTC interval must be non-zero")]
    Zero,
    /// The interval contains a fractional second.
    #[error("fixed UTC interval must use whole seconds")]
    Subsecond,
    /// The jitter fraction is non-finite or outside `(0, 1]`.
    #[error("fixed UTC jitter fraction must be finite and greater than zero up to one")]
    InvalidJitterFraction,
    /// The interval or calculated slot is outside the supported timestamp range.
    #[error("fixed UTC interval or slot is out of range")]
    OutOfRange,
}

#[cfg(test)]
mod tests {
    use chrono::{TimeDelta, TimeZone as _};

    use super::*;

    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 4, hour, minute, 0)
            .single()
            .expect("valid timestamp")
    }

    #[test]
    fn exact_boundary_is_the_current_slot_and_next_is_strictly_later() {
        let interval = FixedUtcInterval::new(Duration::from_secs(60 * 60)).expect("interval");
        let current = interval.slot_at(at(12, 0)).expect("current slot");

        assert_eq!(current.starts_at(), at(12, 0));
        assert_eq!(
            interval
                .next_slot(current, at(12, 0))
                .expect("next slot")
                .starts_at(),
            at(13, 0)
        );
    }

    #[test]
    fn delayed_execution_uses_the_next_wall_clock_boundary() {
        let interval = FixedUtcInterval::new(Duration::from_secs(60 * 60)).expect("interval");
        let current = interval.slot_at(at(12, 0)).expect("current slot");

        assert_eq!(
            interval
                .next_slot(current, at(12, 47))
                .expect("next slot")
                .starts_at(),
            at(13, 0)
        );
    }

    #[test]
    fn missed_slots_are_skipped_without_losing_monotonic_progress() {
        let interval = FixedUtcInterval::new(Duration::from_secs(60 * 60)).expect("interval");
        let current = interval.slot_at(at(8, 0)).expect("current slot");

        assert_eq!(
            interval
                .next_slot(current, at(13, 29))
                .expect("next slot")
                .starts_at(),
            at(14, 0)
        );
    }

    #[test]
    fn clock_movement_keeps_slots_aligned_and_forward_only() {
        let interval = FixedUtcInterval::new(Duration::from_secs(60 * 60)).expect("interval");
        let current = interval.slot_at(at(12, 0)).expect("current slot");

        assert_eq!(
            interval
                .next_slot(current, at(10, 30))
                .expect("backward clock")
                .starts_at(),
            at(13, 0)
        );
        assert_eq!(
            interval
                .next_slot(current, at(16, 30))
                .expect("forward clock")
                .starts_at(),
            at(17, 0)
        );
    }

    #[test]
    fn duplicate_delivery_and_restart_produce_the_same_identifier() {
        let interval = FixedUtcInterval::new(Duration::from_secs(60 * 60)).expect("interval");
        let current = interval.slot_at(at(12, 0)).expect("current slot");
        let before_restart = interval.next_slot(current, at(12, 21)).expect("next slot");
        let after_restart = interval.next_slot(current, at(12, 21)).expect("next slot");

        assert_eq!(before_restart, after_restart);
        assert_eq!(before_restart.identifier(), "fixed-utc:1788526800");
    }

    #[test]
    fn rejects_zero_subsecond_and_excessive_intervals() {
        assert_eq!(
            FixedUtcInterval::new(Duration::ZERO),
            Err(FixedUtcIntervalError::Zero)
        );
        assert_eq!(
            FixedUtcInterval::new(Duration::from_millis(1500)),
            Err(FixedUtcIntervalError::Subsecond)
        );
        assert_eq!(
            FixedUtcInterval::new(Duration::from_secs(i64::MAX as u64 + 1)),
            Err(FixedUtcIntervalError::OutOfRange)
        );
    }

    #[test]
    fn slots_before_the_unix_epoch_use_floor_division() {
        let interval = FixedUtcInterval::new(Duration::from_secs(60)).expect("interval");
        let observed = DateTime::UNIX_EPOCH - TimeDelta::milliseconds(1);

        assert_eq!(
            interval.slot_at(observed).expect("slot").starts_at(),
            DateTime::UNIX_EPOCH - TimeDelta::seconds(60)
        );
    }

    #[test]
    fn jitter_vectors_pin_the_hash_and_offset_algorithm() {
        for (key, seconds, fraction, expected) in [
            ("", 3600, 0.1, 291),
            ("connection:1", 3600, 0.1, 345),
            ("connection:2", 3600, 0.1, 311),
            ("poll:%_é", 60, 0.5, 23),
            ("alpha", 10, 1.0, 5),
            ("tiny", 1, 0.1, 0),
        ] {
            let interval = FixedUtcInterval::new(Duration::from_secs(seconds)).expect("interval");
            let slot = interval.slot_at(DateTime::UNIX_EPOCH).expect("slot");
            let jitter = FixedUtcJitter::new(fraction).expect("jitter");
            assert_eq!(
                interval
                    .run_at(slot, key, jitter)
                    .expect("run time")
                    .timestamp(),
                expected,
                "key={key}, seconds={seconds}, fraction={fraction}"
            );
        }
    }

    #[test]
    fn rejects_invalid_jitter_fractions() {
        for fraction in [
            0.0,
            -0.0,
            -0.1,
            1.01,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ] {
            assert_eq!(
                FixedUtcJitter::new(fraction),
                Err(FixedUtcIntervalError::InvalidJitterFraction)
            );
        }
        assert!(FixedUtcJitter::new(f64::MIN_POSITIVE).is_ok());
        assert!(FixedUtcJitter::new(1.0).is_ok());
    }

    #[test]
    fn jitter_is_bounded_and_spread_over_many_keys() {
        let interval = FixedUtcInterval::new(Duration::from_secs(600)).expect("interval");
        let slot = interval.slot_at(at(12, 0)).expect("slot");
        let jitter = FixedUtcJitter::new(0.999).expect("jitter");
        let mut buckets = [0; 10];
        let mut endpoints = [false; 2];
        for sequence in 0..10_000 {
            let key = format!("connection:{sequence}");
            let offset = (interval.run_at(slot, &key, jitter).expect("run time")
                - slot.starts_at())
            .num_seconds();
            assert!((0..=599).contains(&offset));
            buckets[offset as usize / 60] += 1;
            endpoints[0] |= offset == 0;
            endpoints[1] |= offset == 599;
        }
        assert_eq!(
            endpoints,
            [true, true],
            "both inclusive bounds are reachable"
        );
        assert!(
            buckets.iter().all(|count| (700..=1300).contains(count)),
            "{buckets:?}"
        );
    }

    #[test]
    fn subsecond_jitter_bound_has_zero_offset() {
        let interval = FixedUtcInterval::new(Duration::from_secs(1)).expect("interval");
        let slot = interval.slot_at(at(12, 0)).expect("slot");
        let jitter = FixedUtcJitter::new(0.5).expect("jitter");
        for key in ["connection:1", "connection:2", ""] {
            assert_eq!(interval.run_at(slot, key, jitter), Ok(slot.starts_at()));
        }
    }

    #[test]
    fn full_fraction_jitter_keeps_every_key_inside_its_slot() {
        let jitter = FixedUtcJitter::new(1.0).expect("jitter");
        for seconds in [1, 2, 10, 60, 3600] {
            let interval = FixedUtcInterval::new(Duration::from_secs(seconds)).expect("interval");
            for observed in [
                DateTime::UNIX_EPOCH - TimeDelta::seconds(1),
                DateTime::UNIX_EPOCH,
                at(12, 0),
            ] {
                let slot = interval.slot_at(observed).expect("slot");
                for sequence in 0..10_000 {
                    let key = format!("connection:{sequence}");
                    let run_at = interval.run_at(slot, &key, jitter).expect("run time");
                    assert_eq!(interval.slot_at(run_at).expect("run slot"), slot);
                }
            }
        }
    }

    #[test]
    fn keys_and_jitter_leave_slot_identifiers_and_progress_unchanged() {
        let interval = FixedUtcInterval::new(Duration::from_secs(3600)).expect("interval");
        let slot = interval.slot_at(at(12, 0)).expect("slot");
        let identifier = slot.identifier();
        let jitter = FixedUtcJitter::new(0.1).expect("jitter");
        let first = interval
            .run_at(slot, "connection:1", jitter)
            .expect("first key");
        let second = interval
            .run_at(slot, "connection:2", jitter)
            .expect("second key");
        assert_ne!(first, second);
        for run_at in [first, second] {
            assert_eq!(
                interval.slot_at(run_at).expect("same slot").identifier(),
                identifier
            );
            assert_eq!(
                interval
                    .next_slot(slot, run_at)
                    .expect("next slot")
                    .starts_at(),
                at(13, 0)
            );
        }
        let next = interval.next_slot(slot, first).expect("next slot");
        assert_eq!(
            interval
                .run_at(next, "connection:1", jitter)
                .expect("next run")
                - next.starts_at(),
            first - slot.starts_at()
        );
    }

    #[test]
    fn jitter_handles_timestamp_limits_without_panicking() {
        let interval = FixedUtcInterval::new(Duration::from_secs(1)).expect("interval");
        let slot = interval
            .slot_at(DateTime::<Utc>::MAX_UTC)
            .expect("last slot");
        assert_eq!(
            interval.run_at(slot, "", FixedUtcJitter::new(1.0).expect("jitter")),
            Ok(slot.starts_at())
        );
        let interval =
            FixedUtcInterval::new(Duration::from_secs(i64::MAX as u64)).expect("interval");
        let slot = interval.slot_at(DateTime::UNIX_EPOCH).expect("slot");
        assert_eq!(
            interval.run_at(slot, "", FixedUtcJitter::new(1.0).expect("jitter")),
            Err(FixedUtcIntervalError::OutOfRange)
        );
    }
}
