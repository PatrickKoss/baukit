//! Hybrid logical timestamps compatible with JavaScript safe integers.
//!
//! An encoded timestamp is `wall_time_ms * 1_000 + counter + 1`. The added one
//! reserves zero as invalid. [`HybridLogicalClock`] combines an injected
//! physical clock with a logical counter, so local timestamps keep increasing
//! when physical time stalls or moves backward.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Number of logical counter values available in one physical millisecond.
pub const COUNTERS_PER_MILLISECOND: i64 = 1_000;

/// Largest encoded timestamp that every JavaScript runtime can represent exactly.
pub const MAX_ENCODED_TIMESTAMP: i64 = 9_007_199_254_740_991;

/// Serializable state needed to restore a clock.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HybridLogicalClockState {
    /// Physical millisecond component retained by the clock.
    pub wall_time_ms: i64,
    /// Logical counter within `wall_time_ms`.
    pub counter: i64,
    /// Caller-supplied identifier used to reject state from another clock.
    pub device_id: String,
}

/// Invalid input or exhausted JavaScript-safe timestamp space.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum HlcError {
    /// A wall-time or counter component is outside its accepted range.
    #[error("invalid hybrid logical timestamp component")]
    InvalidComponent,
    /// An encoded timestamp is not a positive JavaScript safe integer.
    #[error("invalid encoded hybrid logical timestamp")]
    InvalidTimestamp,
    /// Encoding or logical rollover would exceed the JavaScript safe-integer limit.
    #[error("hybrid logical timestamp exceeds safe integer range")]
    ExceedsSafeInteger,
    /// The injected physical clock returned a negative or unsafe value.
    #[error("physical clock must return a non-negative safe integer")]
    InvalidPhysicalClock,
    /// The caller supplied a blank clock identifier.
    #[error("HLC device id must not be empty")]
    EmptyDeviceId,
}

/// Encodes physical milliseconds and a logical counter into one timestamp.
///
/// # Errors
///
/// Returns [`HlcError::InvalidComponent`] for a negative wall time or a counter
/// outside `0..1000`. Returns [`HlcError::ExceedsSafeInteger`] when the result
/// is above [`MAX_ENCODED_TIMESTAMP`].
pub fn encode(wall_time_ms: i64, counter: i64) -> Result<i64, HlcError> {
    if wall_time_ms < 0 || !(0..COUNTERS_PER_MILLISECOND).contains(&counter) {
        return Err(HlcError::InvalidComponent);
    }

    let encoded = wall_time_ms
        .checked_mul(COUNTERS_PER_MILLISECOND)
        .and_then(|value| value.checked_add(counter))
        .and_then(|value| value.checked_add(1))
        .ok_or(HlcError::ExceedsSafeInteger)?;
    if encoded > MAX_ENCODED_TIMESTAMP {
        return Err(HlcError::ExceedsSafeInteger);
    }
    Ok(encoded)
}

/// Decodes a positive JavaScript-safe timestamp into physical and logical parts.
///
/// # Errors
///
/// Returns [`HlcError::InvalidTimestamp`] for zero, negative values, or values
/// above [`MAX_ENCODED_TIMESTAMP`].
pub fn decode(timestamp: i64) -> Result<(i64, i64), HlcError> {
    if !(1..=MAX_ENCODED_TIMESTAMP).contains(&timestamp) {
        return Err(HlcError::InvalidTimestamp);
    }

    let zero_based = timestamp - 1;
    Ok((
        zero_based / COUNTERS_PER_MILLISECOND,
        zero_based % COUNTERS_PER_MILLISECOND,
    ))
}

/// Compares two valid encoded timestamps.
///
/// Returns `None` when either input is not a positive JavaScript-safe encoded
/// timestamp.
#[must_use]
pub fn compare(left: i64, right: i64) -> Option<Ordering> {
    (decode(left).is_ok() && decode(right).is_ok()).then(|| left.cmp(&right))
}

/// Stateful hybrid logical clock backed by a caller-supplied physical clock.
pub struct HybridLogicalClock<C> {
    device_id: String,
    physical_clock: C,
    state: HybridLogicalClockState,
}

impl<C> HybridLogicalClock<C>
where
    C: FnMut() -> i64,
{
    /// Opens a clock from optional persisted state.
    ///
    /// State with invalid components, an unencodable value, or a different
    /// `device_id` is treated as corrupt and replaced with zero state. The
    /// caller owns loading and saving the snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`HlcError::EmptyDeviceId`] when `device_id` is blank.
    pub fn open(
        device_id: impl Into<String>,
        physical_clock: C,
        persisted: Option<HybridLogicalClockState>,
    ) -> Result<Self, HlcError> {
        let device_id = device_id.into();
        if device_id.trim().is_empty() {
            return Err(HlcError::EmptyDeviceId);
        }

        let state = match persisted {
            Some(state)
                if state.device_id == device_id
                    && encode(state.wall_time_ms, state.counter).is_ok() =>
            {
                state
            }
            _ => HybridLogicalClockState {
                wall_time_ms: 0,
                counter: 0,
                device_id: device_id.clone(),
            },
        };

        Ok(Self {
            device_id,
            physical_clock,
            state,
        })
    }

    /// Returns the next local timestamp and advances the clock.
    ///
    /// # Errors
    ///
    /// Returns [`HlcError::InvalidPhysicalClock`] for an invalid clock reading.
    /// Returns [`HlcError::ExceedsSafeInteger`] when no later safe timestamp is
    /// available. State remains unchanged on error.
    pub fn now(&mut self) -> Result<i64, HlcError> {
        let physical = self.read_physical_time()?;
        let wall_time_ms = physical.max(self.state.wall_time_ms);
        let counter = if physical > self.state.wall_time_ms {
            0
        } else {
            self.state.counter + 1
        };
        self.advance(wall_time_ms, counter)
    }

    /// Observes a remote timestamp and returns a timestamp ordered after it.
    ///
    /// # Errors
    ///
    /// Returns [`HlcError::InvalidTimestamp`] for invalid remote input,
    /// [`HlcError::InvalidPhysicalClock`] for an invalid local clock reading,
    /// or [`HlcError::ExceedsSafeInteger`] when no later timestamp is available.
    /// State remains unchanged on error.
    pub fn observe(&mut self, remote_timestamp: i64) -> Result<i64, HlcError> {
        let (remote_wall_time_ms, remote_counter) = decode(remote_timestamp)?;
        let physical = self.read_physical_time()?;
        let wall_time_ms = physical
            .max(self.state.wall_time_ms)
            .max(remote_wall_time_ms);
        let counter =
            if wall_time_ms == self.state.wall_time_ms && wall_time_ms == remote_wall_time_ms {
                self.state.counter.max(remote_counter) + 1
            } else if wall_time_ms == self.state.wall_time_ms {
                self.state.counter + 1
            } else if wall_time_ms == remote_wall_time_ms {
                remote_counter + 1
            } else {
                0
            };
        self.advance(wall_time_ms, counter)
    }

    /// Returns an owned snapshot suitable for caller-controlled persistence.
    #[must_use]
    pub fn snapshot(&self) -> HybridLogicalClockState {
        self.state.clone()
    }

    fn read_physical_time(&mut self) -> Result<i64, HlcError> {
        let value = (self.physical_clock)();
        if !(0..=MAX_ENCODED_TIMESTAMP).contains(&value) {
            return Err(HlcError::InvalidPhysicalClock);
        }
        Ok(value)
    }

    fn advance(&mut self, mut wall_time_ms: i64, mut counter: i64) -> Result<i64, HlcError> {
        if counter >= COUNTERS_PER_MILLISECOND {
            wall_time_ms = wall_time_ms
                .checked_add(1)
                .ok_or(HlcError::ExceedsSafeInteger)?;
            counter = 0;
        }

        let timestamp = encode(wall_time_ms, counter)?;
        self.state = HybridLogicalClockState {
            wall_time_ms,
            counter,
            device_id: self.device_id.clone(),
        };
        Ok(timestamp)
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, cmp::Ordering};

    use serde::Deserialize;

    use super::{
        HlcError, HybridLogicalClock, HybridLogicalClockState, MAX_ENCODED_TIMESTAMP, compare,
        decode, encode,
    };

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Vectors {
        encode: Vec<EncodeVector>,
        compare: Vec<CompareVector>,
        merge: Vec<MergeVector>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct EncodeVector {
        wall_time_ms: i64,
        counter: i64,
        timestamp: i64,
    }

    #[derive(Deserialize)]
    struct CompareVector {
        left: i64,
        right: i64,
        ordering: i8,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct MergeVector {
        device_id: String,
        initial: HybridLogicalClockState,
        physical_time_ms: i64,
        remote_timestamp: i64,
        expected_timestamp: i64,
        expected_state: HybridLogicalClockState,
    }

    #[test]
    fn advances_when_physical_time_stalls_or_moves_backward() {
        let physical = Cell::new(1_000);
        let mut clock = HybridLogicalClock::open("device-a", || physical.get(), None)
            .expect("clock should open");

        let first = clock.now().expect("first timestamp");
        let stalled = clock.now().expect("stalled timestamp");
        physical.set(900);
        let backward = clock.now().expect("backward timestamp");

        assert!(first < stalled && stalled < backward);
        assert_eq!(decode(backward), Ok((1_000, 2)));
    }

    #[test]
    fn rolls_the_logical_counter_into_the_next_millisecond() {
        let state = HybridLogicalClockState {
            wall_time_ms: 1_000,
            counter: 999,
            device_id: "device-a".to_owned(),
        };
        let mut clock =
            HybridLogicalClock::open("device-a", || 1_000, Some(state)).expect("clock should open");

        assert_eq!(clock.now(), Ok(1_001_001));
        assert_eq!(
            clock.snapshot(),
            HybridLogicalClockState {
                wall_time_ms: 1_001,
                counter: 0,
                device_id: "device-a".to_owned(),
            }
        );
    }

    #[test]
    fn restores_matching_state_and_rejects_corrupt_state() {
        let restored = HybridLogicalClockState {
            wall_time_ms: 2_000,
            counter: 7,
            device_id: "device-a".to_owned(),
        };
        let mut clock = HybridLogicalClock::open("device-a", || 1_000, Some(restored))
            .expect("clock should open");
        assert_eq!(clock.now(), Ok(2_000_009));

        let corrupt = HybridLogicalClockState {
            wall_time_ms: MAX_ENCODED_TIMESTAMP,
            counter: 0,
            device_id: "device-a".to_owned(),
        };
        let mut reset = HybridLogicalClock::open("device-a", || 10, Some(corrupt))
            .expect("corrupt state should reset");
        assert_eq!(reset.now(), Ok(10_001));
    }

    #[test]
    fn observes_remote_time_and_preserves_state_on_invalid_input() {
        let mut clock =
            HybridLogicalClock::open("device-a", || 900, None).expect("clock should open");

        assert_eq!(clock.observe(1_000_006), Ok(1_000_007));
        let snapshot = clock.snapshot();
        assert_eq!(clock.observe(0), Err(HlcError::InvalidTimestamp));
        assert_eq!(clock.snapshot(), snapshot);
        assert_eq!(compare(0, 1), None);
    }

    #[test]
    fn rejects_blank_device_ids_and_invalid_physical_time() {
        assert!(matches!(
            HybridLogicalClock::open("   ", || 0, None),
            Err(HlcError::EmptyDeviceId)
        ));

        let mut clock =
            HybridLogicalClock::open("device-a", || -1, None).expect("clock should open");
        assert_eq!(clock.now(), Err(HlcError::InvalidPhysicalClock));
        assert_eq!(
            clock.snapshot(),
            HybridLogicalClockState {
                wall_time_ms: 0,
                counter: 0,
                device_id: "device-a".to_owned(),
            }
        );
    }

    #[test]
    fn supports_the_maximum_encoded_value_and_reports_exhaustion() {
        assert_eq!(encode(9_007_199_254_740, 990), Ok(MAX_ENCODED_TIMESTAMP));
        assert_eq!(decode(MAX_ENCODED_TIMESTAMP), Ok((9_007_199_254_740, 990)));
        assert_eq!(
            encode(9_007_199_254_740, 991),
            Err(HlcError::ExceedsSafeInteger)
        );

        let state = HybridLogicalClockState {
            wall_time_ms: 9_007_199_254_740,
            counter: 990,
            device_id: "device-a".to_owned(),
        };
        let mut clock = HybridLogicalClock::open("device-a", || 0, Some(state))
            .expect("maximum state should open");
        assert_eq!(clock.now(), Err(HlcError::ExceedsSafeInteger));
        assert_eq!(
            encode(clock.snapshot().wall_time_ms, clock.snapshot().counter),
            Ok(MAX_ENCODED_TIMESTAMP)
        );
    }

    #[test]
    fn matches_shared_cross_runtime_vectors() {
        let vectors: Vectors =
            serde_json::from_str(include_str!("../../../../fixtures/hlc/vectors-v1.json"))
                .expect("shared HLC vectors must be valid JSON");

        for vector in vectors.encode {
            assert_eq!(
                encode(vector.wall_time_ms, vector.counter),
                Ok(vector.timestamp)
            );
            assert_eq!(
                decode(vector.timestamp),
                Ok((vector.wall_time_ms, vector.counter))
            );
        }
        for vector in vectors.compare {
            let expected = match vector.ordering {
                -1 => Ordering::Less,
                0 => Ordering::Equal,
                1 => Ordering::Greater,
                value => panic!("invalid vector ordering {value}"),
            };
            assert_eq!(compare(vector.left, vector.right), Some(expected));
        }
        for vector in vectors.merge {
            let physical_time_ms = vector.physical_time_ms;
            let mut clock = HybridLogicalClock::open(
                vector.device_id,
                move || physical_time_ms,
                Some(vector.initial),
            )
            .expect("vector clock state should open");
            assert_eq!(
                clock.observe(vector.remote_timestamp),
                Ok(vector.expected_timestamp)
            );
            assert_eq!(clock.snapshot(), vector.expected_state);
        }
    }
}
