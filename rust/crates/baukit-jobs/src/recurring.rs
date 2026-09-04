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

/// Invalid fixed UTC interval or a slot outside Chrono's timestamp range.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum FixedUtcIntervalError {
    /// The interval is zero.
    #[error("fixed UTC interval must be non-zero")]
    Zero,
    /// The interval contains a fractional second.
    #[error("fixed UTC interval must use whole seconds")]
    Subsecond,
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
}
