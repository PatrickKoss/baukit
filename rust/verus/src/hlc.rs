//! Twin of `baukit_sync::hlc` (`now_step`, `observe_step`, `encode`, `decode`)
//! and of `HybridLogicalClock::{now, observe, commit}`.
//!
//! V-01. For every sequence of `now` and `observe` calls, outputs strictly
//! increase, and the clock never runs ahead of the largest physical time seen
//! plus `max_drift` by more than one millisecond per 1000 ticks.
use vstd::prelude::*;

verus! {

pub const COUNTERS_PER_MILLISECOND: i64 = 1_000;

pub const MAX_ENCODED_TIMESTAMP: i64 = 9_007_199_254_740_991;

pub type HlcStep = (i64, i64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HlcError {
    InvalidComponent,
    InvalidTimestamp,
    ExceedsSafeInteger,
    InvalidPhysicalClock,
    RemoteTooFarAhead,
    InvalidMaxDrift,
}

pub open spec fn valid(step: HlcStep) -> bool {
    0 <= step.0 && 0 <= step.1 < COUNTERS_PER_MILLISECOND
        && step.0 * COUNTERS_PER_MILLISECOND + step.1 + 1 <= MAX_ENCODED_TIMESTAMP
}

/// The step as one logical number; the encoded timestamp is this plus one.
pub open spec fn logical(step: HlcStep) -> int {
    step.0 * COUNTERS_PER_MILLISECOND + step.1
}

pub open spec fn max_int(a: int, b: int) -> int {
    if a >= b { a } else { b }
}

pub fn now_step(state: HlcStep, physical: i64) -> (out: Result<HlcStep, HlcError>)
    ensures
        out matches Ok(next) ==> valid(state) && valid(next) && 0 <= physical
            && logical(next) == max_int(physical * COUNTERS_PER_MILLISECOND, logical(state) + 1),
{
    check_physical(physical)?;
    encode(state.0, state.1)?;
    let wall = if physical >= state.0 { physical } else { state.0 };
    let counter = if physical > state.0 { 0 } else { state.1 + 1 };
    roll_over(wall, counter)
}

pub fn observe_step(state: HlcStep, physical: i64, remote: i64, max_drift_ms: i64) -> (out: Result<HlcStep, HlcError>)
    ensures
        out matches Ok(next) ==> valid(state) && valid(next) && 0 <= physical && 0 <= max_drift_ms
            && 1 <= remote <= (physical + max_drift_ms + 1) * COUNTERS_PER_MILLISECOND
            && logical(next) == max_int(
                max_int(physical * COUNTERS_PER_MILLISECOND, logical(state) + 1),
                remote as int,
            ),
{
    let (remote_wall, remote_counter) = decode(remote)?;
    check_physical(physical)?;
    encode(state.0, state.1)?;
    check_drift(remote_wall, physical, max_drift_ms)?;
    let wall = max(max(physical, state.0), remote_wall);
    let counter = if wall == state.0 && wall == remote_wall {
        max(state.1, remote_counter) + 1
    } else if wall == state.0 {
        state.1 + 1
    } else if wall == remote_wall {
        remote_counter + 1
    } else {
        0
    };
    roll_over(wall, counter)
}

fn max(a: i64, b: i64) -> (out: i64)
    ensures out as int == max_int(a as int, b as int),
{
    if a >= b { a } else { b }
}

fn check_physical(physical: i64) -> (out: Result<(), HlcError>)
    ensures out is Ok <==> 0 <= physical <= MAX_ENCODED_TIMESTAMP,
{
    if 0 <= physical && physical <= MAX_ENCODED_TIMESTAMP {
        Ok(())
    } else {
        Err(HlcError::InvalidPhysicalClock)
    }
}

fn check_drift(remote_wall: i64, physical: i64, max_drift_ms: i64) -> (out: Result<(), HlcError>)
    requires 0 <= physical,
    ensures out is Ok <==> 0 <= max_drift_ms && remote_wall <= physical + max_drift_ms,
{
    if max_drift_ms < 0 {
        return Err(HlcError::InvalidMaxDrift);
    }
    let bound = if max_drift_ms > i64::MAX - physical { i64::MAX } else { physical + max_drift_ms };
    if remote_wall > bound {
        return Err(HlcError::RemoteTooFarAhead);
    }
    Ok(())
}

fn roll_over(wall: i64, counter: i64) -> (out: Result<HlcStep, HlcError>)
    requires 0 <= wall, 0 <= counter <= COUNTERS_PER_MILLISECOND,
    ensures out matches Ok(next) ==> valid(next) && logical(next) == wall * COUNTERS_PER_MILLISECOND + counter,
{
    let step = if counter >= COUNTERS_PER_MILLISECOND {
        if wall == i64::MAX {
            return Err(HlcError::ExceedsSafeInteger);
        }
        (wall + 1, 0)
    } else {
        (wall, counter)
    };
    encode(step.0, step.1)?;
    Ok(step)
}

pub fn encode(wall_time_ms: i64, counter: i64) -> (out: Result<i64, HlcError>)
    ensures
        out is Ok <==> valid((wall_time_ms, counter)),
        out matches Ok(t) ==> t == logical((wall_time_ms, counter)) + 1,
{
    if wall_time_ms < 0 || !(0 <= counter && counter < COUNTERS_PER_MILLISECOND) {
        return Err(HlcError::InvalidComponent);
    }
    if wall_time_ms > MAX_ENCODED_TIMESTAMP / COUNTERS_PER_MILLISECOND {
        return Err(HlcError::ExceedsSafeInteger);
    }
    let encoded = wall_time_ms * COUNTERS_PER_MILLISECOND + counter + 1;
    if encoded > MAX_ENCODED_TIMESTAMP {
        return Err(HlcError::ExceedsSafeInteger);
    }
    Ok(encoded)
}

pub fn decode(timestamp: i64) -> (out: Result<HlcStep, HlcError>)
    ensures
        out is Ok <==> 1 <= timestamp <= MAX_ENCODED_TIMESTAMP,
        out matches Ok(step) ==> valid(step) && logical(step) + 1 == timestamp,
{
    if !(1 <= timestamp && timestamp <= MAX_ENCODED_TIMESTAMP) {
        return Err(HlcError::InvalidTimestamp);
    }
    let zero_based = timestamp - 1;
    let wall = zero_based / COUNTERS_PER_MILLISECOND;
    let counter = zero_based % COUNTERS_PER_MILLISECOND;
    assert(zero_based == wall * COUNTERS_PER_MILLISECOND + counter) by (nonlinear_arith)
        requires wall == zero_based / 1000, counter == zero_based % 1000, zero_based >= 0;
    Ok((wall, counter))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockCall {
    Now { physical: i64 },
    Observe { physical: i64, remote: i64 },
}

pub open spec fn call_physical(call: ClockCall) -> int {
    match call {
        ClockCall::Now { physical } => physical as int,
        ClockCall::Observe { physical, .. } => physical as int,
    }
}

/// Largest physical time among the first `n` calls, floored at `floor`.
pub open spec fn max_physical(calls: Seq<ClockCall>, n: int, floor: int) -> int
    decreases n,
{
    if n <= 0 { floor } else { max_int(max_physical(calls, n - 1, floor), call_physical(calls[n - 1])) }
}

proof fn max_physical_grows(calls: Seq<ClockCall>, n: int, floor: int)
    requires 0 <= n,
    ensures
        max_physical(calls, n, floor) >= floor,
        forall|i: int| 0 <= i < n ==> max_physical(calls, n, floor) >= #[trigger] call_physical(calls[i]),
    decreases n,
{
    if n > 0 {
        max_physical_grows(calls, n - 1, floor);
    }
}

/// Twin of `HybridLogicalClock::now` and `observe`: each call reads the
/// physical clock, steps, and commits only on success. Returns every emitted
/// timestamp and the final state.
///
/// V-01: outputs strictly increase, every output exceeds the starting state,
/// and the final logical time `wall * 1000 + counter` is at most the last
/// logical time of millisecond `max_physical + max_drift` (or the starting
/// state, if later) plus the number of calls. The wall clock passes
/// `max_physical + max_drift` only by counter roll-over, one millisecond per
/// 1000 calls.
pub fn replay(initial: HlcStep, calls: &Vec<ClockCall>, max_drift_ms: i64) -> (out: (Vec<i64>, HlcStep))
    requires valid(initial), 0 <= max_drift_ms,
    ensures
        valid(out.1),
        forall|i: int, j: int| 0 <= i < j < out.0.len() ==> out.0[i] < out.0[j],
        forall|i: int| 0 <= i < out.0.len() ==> out.0[i] > logical(initial) + 1,
        out.0.len() > 0 ==> out.0[out.0.len() - 1] == logical(out.1) + 1,
        logical(out.1) <= max_int(
            logical(initial),
            (max_physical(calls@, calls.len() as int, 0) + max_drift_ms) * COUNTERS_PER_MILLISECOND + COUNTERS_PER_MILLISECOND - 1,
        ) + calls.len(),
{
    let mut state = initial;
    let mut outputs: Vec<i64> = Vec::new();
    let mut k: usize = 0;
    while k < calls.len()
        invariant
            0 <= k <= calls.len(),
            valid(state),
            0 <= max_drift_ms,
            forall|i: int, j: int| 0 <= i < j < outputs.len() ==> outputs[i] < outputs[j],
            forall|i: int| 0 <= i < outputs.len() ==> outputs[i] > logical(initial) + 1,
            outputs.len() > 0 ==> outputs[outputs.len() - 1] == logical(state) + 1,
            logical(state) >= logical(initial),
            logical(state) <= max_int(
                logical(initial),
                (max_physical(calls@, k as int, 0) + max_drift_ms) * COUNTERS_PER_MILLISECOND + COUNTERS_PER_MILLISECOND - 1,
            ) + k,
        decreases calls.len() - k,
    {
        let call = calls[k];
        let result = match call {
            ClockCall::Now { physical } => now_step(state, physical),
            ClockCall::Observe { physical, remote } => observe_step(state, physical, remote, max_drift_ms),
        };
        let ghost before = logical(state);
        let ghost bound_before = max_int(
            logical(initial),
            (max_physical(calls@, k as int, 0) + max_drift_ms) * COUNTERS_PER_MILLISECOND + COUNTERS_PER_MILLISECOND - 1,
        );
        proof {
            max_physical_grows(calls@, k as int + 1, 0);
            assert(max_physical(calls@, k as int + 1, 0) >= max_physical(calls@, k as int, 0));
            assert(max_physical(calls@, k as int + 1, 0) >= call_physical(calls@[k as int]));
            assert((max_physical(calls@, k as int + 1, 0) + max_drift_ms) * COUNTERS_PER_MILLISECOND
                >= (max_physical(calls@, k as int, 0) + max_drift_ms) * COUNTERS_PER_MILLISECOND) by (nonlinear_arith)
                requires max_physical(calls@, k as int + 1, 0) >= max_physical(calls@, k as int, 0);
            assert((max_physical(calls@, k as int + 1, 0) + max_drift_ms) * COUNTERS_PER_MILLISECOND
                >= (call_physical(calls@[k as int]) + max_drift_ms) * COUNTERS_PER_MILLISECOND) by (nonlinear_arith)
                requires max_physical(calls@, k as int + 1, 0) >= call_physical(calls@[k as int]);
            assert(call_physical(calls@[k as int]) * COUNTERS_PER_MILLISECOND
                <= (call_physical(calls@[k as int]) + max_drift_ms) * COUNTERS_PER_MILLISECOND) by (nonlinear_arith)
                requires 0 <= max_drift_ms;
        }
        if let Ok(next) = result {
            let timestamp = encode(next.0, next.1);
            if let Ok(t) = timestamp {
                assert(before < logical(next));
                outputs.push(t);
                state = next;
            }
        }
        proof {
            assert(bound_before <= max_int(
                logical(initial),
                (max_physical(calls@, k as int + 1, 0) + max_drift_ms) * COUNTERS_PER_MILLISECOND + COUNTERS_PER_MILLISECOND - 1,
            ));
        }
        k = k + 1;
    }
    (outputs, state)
}

} // verus!
