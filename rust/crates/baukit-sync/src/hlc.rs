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

/// Default bound on how far a remote wall time may lead the local physical clock.
pub const DEFAULT_MAX_DRIFT_MS: i64 = 24 * 60 * 60 * 1_000;

/// Wall time and logical counter of a clock.
pub type HlcStep = (i64, i64);

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
    /// A remote wall time leads the local physical clock by more than the drift bound.
    #[error("remote hybrid logical timestamp is too far ahead of the local clock")]
    RemoteTooFarAhead,
    /// The configured drift bound is negative.
    #[error("HLC max drift must not be negative")]
    InvalidMaxDrift,
}

/// Computes the state after a local event, without side effects.
///
/// # Errors
///
/// [`HlcError::InvalidPhysicalClock`] for a physical time outside
/// `0..=MAX_ENCODED_TIMESTAMP`, [`HlcError::InvalidComponent`] for an invalid
/// state, and [`HlcError::ExceedsSafeInteger`] when no later timestamp exists.
pub fn now_step(state: HlcStep, physical: i64) -> Result<HlcStep, HlcError> {
    check_physical(physical)?;
    encode(state.0, state.1)?;
    let wall = physical.max(state.0);
    let counter = if physical > state.0 { 0 } else { state.1 + 1 };
    roll_over(wall, counter)
}

/// Computes the state after receiving `remote`, without side effects.
///
/// # Errors
///
/// [`HlcError::InvalidTimestamp`] for an invalid remote,
/// [`HlcError::RemoteTooFarAhead`] when the remote wall time exceeds
/// `physical + max_drift_ms`, [`HlcError::InvalidMaxDrift`] for a negative
/// bound, and the errors of [`now_step`].
pub fn observe_step(
    state: HlcStep,
    physical: i64,
    remote: i64,
    max_drift_ms: i64,
) -> Result<HlcStep, HlcError> {
    let (remote_wall, remote_counter) = decode(remote)?;
    check_physical(physical)?;
    encode(state.0, state.1)?;
    check_drift(remote_wall, physical, max_drift_ms)?;
    let wall = physical.max(state.0).max(remote_wall);
    let counter = if wall == state.0 && wall == remote_wall {
        state.1.max(remote_counter) + 1
    } else if wall == state.0 {
        state.1 + 1
    } else if wall == remote_wall {
        remote_counter + 1
    } else {
        0
    };
    roll_over(wall, counter)
}

/// Rejects a client timestamp whose wall time leads the server clock by more
/// than `max_drift_ms`. Push handlers call this before accepting a change.
///
/// # Errors
///
/// [`HlcError::InvalidTimestamp`] for an invalid timestamp,
/// [`HlcError::InvalidPhysicalClock`] for an invalid server time,
/// [`HlcError::InvalidMaxDrift`] for a negative bound, and
/// [`HlcError::RemoteTooFarAhead`] when the timestamp is too far ahead.
pub fn validate_client_timestamp(
    timestamp: i64,
    server_now_ms: i64,
    max_drift_ms: i64,
) -> Result<(), HlcError> {
    let (wall, _) = decode(timestamp)?;
    check_physical(server_now_ms)?;
    check_drift(wall, server_now_ms, max_drift_ms)
}

fn check_physical(physical: i64) -> Result<(), HlcError> {
    if (0..=MAX_ENCODED_TIMESTAMP).contains(&physical) {
        Ok(())
    } else {
        Err(HlcError::InvalidPhysicalClock)
    }
}

fn check_drift(remote_wall: i64, physical: i64, max_drift_ms: i64) -> Result<(), HlcError> {
    if max_drift_ms < 0 {
        return Err(HlcError::InvalidMaxDrift);
    }
    if remote_wall > physical.saturating_add(max_drift_ms) {
        return Err(HlcError::RemoteTooFarAhead);
    }
    Ok(())
}

fn roll_over(wall: i64, counter: i64) -> Result<HlcStep, HlcError> {
    let step = if counter >= COUNTERS_PER_MILLISECOND {
        (wall.checked_add(1).ok_or(HlcError::ExceedsSafeInteger)?, 0)
    } else {
        (wall, counter)
    };
    encode(step.0, step.1)?;
    Ok(step)
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
    max_drift_ms: i64,
}

impl<C> HybridLogicalClock<C>
where
    C: FnMut() -> i64,
{
    /// Opens a clock from optional persisted state.
    ///
    /// Without persisted state the clock starts at wall time zero. State with
    /// invalid components, an unencodable value, or a different `device_id`
    /// is corrupt; the clock then starts at the current physical time instead
    /// of zero, so it does not reissue timestamps below those it handed out
    /// before. The caller owns loading and saving the snapshot. The drift
    /// bound defaults to [`DEFAULT_MAX_DRIFT_MS`].
    ///
    /// # Errors
    ///
    /// Returns [`HlcError::EmptyDeviceId`] when `device_id` is blank and
    /// [`HlcError::InvalidPhysicalClock`] when corrupt state meets an invalid
    /// physical reading.
    pub fn open(
        device_id: impl Into<String>,
        mut physical_clock: C,
        persisted: Option<HybridLogicalClockState>,
    ) -> Result<Self, HlcError> {
        let device_id = device_id.into();
        if device_id.trim().is_empty() {
            return Err(HlcError::EmptyDeviceId);
        }

        let wall_time_ms = match persisted {
            Some(state)
                if state.device_id == device_id
                    && encode(state.wall_time_ms, state.counter).is_ok() =>
            {
                return Ok(Self::with_state(device_id, physical_clock, state));
            }
            Some(_) => {
                let physical = physical_clock();
                check_physical(physical)?;
                physical
            }
            None => 0,
        };
        let state = HybridLogicalClockState {
            wall_time_ms,
            counter: 0,
            device_id: device_id.clone(),
        };
        Ok(Self::with_state(device_id, physical_clock, state))
    }

    /// Sets how far a remote wall time may lead the physical clock in [`Self::observe`].
    ///
    /// # Errors
    ///
    /// Returns [`HlcError::InvalidMaxDrift`] for a negative bound.
    pub fn with_max_drift_ms(mut self, max_drift_ms: i64) -> Result<Self, HlcError> {
        if max_drift_ms < 0 {
            return Err(HlcError::InvalidMaxDrift);
        }
        self.max_drift_ms = max_drift_ms;
        Ok(self)
    }

    const fn with_state(
        device_id: String,
        physical_clock: C,
        state: HybridLogicalClockState,
    ) -> Self {
        Self {
            device_id,
            physical_clock,
            state,
            max_drift_ms: DEFAULT_MAX_DRIFT_MS,
        }
    }

    /// Returns the next local timestamp and advances the clock.
    ///
    /// # Errors
    ///
    /// Returns [`HlcError::InvalidPhysicalClock`] for an invalid clock reading.
    /// Returns [`HlcError::ExceedsSafeInteger`] when no later safe timestamp is
    /// available. State remains unchanged on error.
    pub fn now(&mut self) -> Result<i64, HlcError> {
        let physical = (self.physical_clock)();
        let step = now_step(self.step(), physical)?;
        self.commit(step)
    }

    /// Observes a remote timestamp and returns a timestamp ordered after it.
    ///
    /// # Errors
    ///
    /// Returns [`HlcError::InvalidTimestamp`] for invalid remote input,
    /// [`HlcError::RemoteTooFarAhead`] when the remote leads the physical clock
    /// by more than the drift bound, [`HlcError::InvalidPhysicalClock`] for an
    /// invalid local clock reading, or [`HlcError::ExceedsSafeInteger`] when
    /// no later timestamp is available. State remains unchanged on error.
    pub fn observe(&mut self, remote_timestamp: i64) -> Result<i64, HlcError> {
        let physical = (self.physical_clock)();
        let step = observe_step(self.step(), physical, remote_timestamp, self.max_drift_ms)?;
        self.commit(step)
    }

    /// Returns an owned snapshot suitable for caller-controlled persistence.
    #[must_use]
    pub fn snapshot(&self) -> HybridLogicalClockState {
        self.state.clone()
    }

    const fn step(&self) -> HlcStep {
        (self.state.wall_time_ms, self.state.counter)
    }

    fn commit(&mut self, (wall_time_ms, counter): HlcStep) -> Result<i64, HlcError> {
        let timestamp = encode(wall_time_ms, counter)?;
        self.state = HybridLogicalClockState {
            wall_time_ms,
            counter,
            device_id: self.device_id.clone(),
        };
        Ok(timestamp)
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    fn any_valid_step() -> HlcStep {
        let step: HlcStep = (kani::any(), kani::any());
        kani::assume(encode(step.0, step.1).is_ok());
        step
    }

    fn encoded(step: HlcStep) -> i64 {
        encode(step.0, step.1).unwrap_or(0)
    }

    /// K-09 for `now_step`: Ok advances the state; the wall time is at most
    /// `max(physical, state.wall) + 1` (the +1 is counter rollover); Err never
    /// changes a value (the function is pure) and nothing panics.
    /// Bounds: any `i64` inputs, no loops, no unwind bound needed.
    #[kani::proof]
    fn now_step_advances_within_bound() {
        let state = any_valid_step();
        let physical: i64 = kani::any();
        if let Ok(next) = now_step(state, physical) {
            assert!(encoded(next) > encoded(state));
            assert!(next.0 <= physical.max(state.0) + 1);
        }
    }

    /// K-09 for `observe_step`: Ok is above both the state and the remote,
    /// with wall time at most `max(physical + max_drift, state.wall) + 1`
    /// (the +1 is counter rollover). A remote beyond the drift bound is
    /// rejected. No panic. Bounds: any `i64` inputs, no loops.
    #[kani::proof]
    fn observe_step_respects_drift_bound() {
        let state = any_valid_step();
        let physical: i64 = kani::any();
        let remote: i64 = kani::any();
        let max_drift: i64 = kani::any();
        let result = observe_step(state, physical, remote, max_drift);
        if let Ok((remote_wall, _)) = decode(remote)
            && max_drift >= 0
            && check_physical(physical).is_ok()
            && remote_wall > physical.saturating_add(max_drift)
        {
            assert!(result == Err(HlcError::RemoteTooFarAhead));
        }
        if let Ok(next) = result {
            assert!(encoded(next) > encoded(state));
            assert!(encoded(next) > remote);
            assert!(next.0 <= physical.saturating_add(max_drift).max(state.0) + 1);
        }
    }

    /// K-10: encode and decode round trip over all valid components, and
    /// `compare` orders valid timestamps exactly like their components.
    /// Bounds: any `i64` inputs, no loops.
    #[kani::proof]
    fn encode_round_trips_and_compare_is_lexicographic() {
        let left = any_valid_step();
        let right = any_valid_step();
        let (left_encoded, right_encoded) = (encoded(left), encoded(right));
        assert!(decode(left_encoded) == Ok(left));
        assert!(compare(left_encoded, right_encoded) == Some(left.cmp(&right)));
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
        assert_eq!(
            reset.snapshot().wall_time_ms,
            10,
            "corrupt state starts at physical time, not zero"
        );
        assert_eq!(reset.now(), Ok(10_002));
    }

    #[test]
    fn observe_rejects_remotes_beyond_the_drift_bound_without_changing_state() {
        let mut clock = HybridLogicalClock::open("device-a", || 1_000, None)
            .expect("clock")
            .with_max_drift_ms(500)
            .expect("drift");
        let before = clock.snapshot();
        let too_far = encode(1_501, 0).expect("encode");
        assert_eq!(clock.observe(too_far), Err(HlcError::RemoteTooFarAhead));
        assert_eq!(clock.snapshot(), before);
        assert_eq!(
            clock.observe(encode(1_500, 3).expect("encode")),
            Ok(1_500_005)
        );
        assert_eq!(
            clock.observe(MAX_ENCODED_TIMESTAMP),
            Err(HlcError::RemoteTooFarAhead)
        );
        assert!(
            clock.now().is_ok(),
            "a far-future remote must not wedge the clock"
        );
    }

    #[test]
    fn default_drift_bound_is_one_day_and_negative_bounds_are_rejected() {
        let day = super::DEFAULT_MAX_DRIFT_MS;
        let mut clock = HybridLogicalClock::open("device-a", || 0, None).expect("clock");
        assert!(clock.observe(encode(day, 0).expect("encode")).is_ok());
        assert_eq!(
            clock.observe(encode(day + 1, 0).expect("encode")),
            Err(HlcError::RemoteTooFarAhead)
        );
        assert!(matches!(
            HybridLogicalClock::open("device-a", || 0, None)
                .expect("clock")
                .with_max_drift_ms(-1),
            Err(HlcError::InvalidMaxDrift)
        ));
    }

    #[test]
    fn server_validates_client_timestamps_against_its_clock() {
        let ok = encode(2_000, 0).expect("encode");
        assert_eq!(super::validate_client_timestamp(ok, 1_000, 1_000), Ok(()));
        assert_eq!(
            super::validate_client_timestamp(ok, 999, 1_000),
            Err(HlcError::RemoteTooFarAhead)
        );
        assert_eq!(
            super::validate_client_timestamp(0, 1_000, 1_000),
            Err(HlcError::InvalidTimestamp)
        );
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
