//! A count-based circuit breaker.
//!
//! [`BreakerMachine`] is the pure state machine: each method takes the
//! current instant and, where it opens, a jitter fraction. [`CircuitBreaker`]
//! wraps it with a [`Clock`], a [`JitterSource`] and a mutex.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::clock::{Clock, SystemClock};
use crate::jitter::{JitterSource, SplitMixJitter, sanitize};
use crate::millis::{from_millis, grow, scale_floor, to_millis};

const PERCENT: u32 = 100;
const OPEN_GROWTH: u32 = 2;

/// Thresholds and timings for a [`BreakerMachine`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BreakerPolicy {
    /// Number of recent calls the failure rate is computed over.
    pub window: u32,
    /// Calls the window needs before the failure rate can open the breaker.
    pub min_calls: u32,
    /// Failure rate in percent at which the breaker opens.
    pub failure_rate_percent: u32,
    /// Consecutive timeouts that open the breaker regardless of the window.
    pub consecutive_timeouts: u32,
    /// Open time after the first opening.
    pub open_base: Duration,
    /// Upper bound for any open time, including `HoldFor`.
    pub open_max: Duration,
    /// Up to this percentage is subtracted from each open time as jitter.
    pub jitter_percent: u32,
    /// A half-open probe that reports nothing for this long is abandoned and a new one admitted.
    pub probe_timeout: Duration,
}

impl Default for BreakerPolicy {
    fn default() -> Self {
        Self {
            window: 20,
            min_calls: 10,
            failure_rate_percent: 50,
            consecutive_timeouts: 5,
            open_base: Duration::from_secs(30),
            open_max: Duration::from_secs(300),
            jitter_percent: 20,
            probe_timeout: Duration::from_secs(30),
        }
    }
}

impl BreakerPolicy {
    /// Open time for the `opens`-th consecutive opening (one-based), before
    /// `HoldFor`: `base * 2^(opens - 1)` capped at `open_max`, minus
    /// `floor(open * jitter_percent / 100 * fraction)`.
    #[must_use]
    pub fn open_duration(&self, opens: u32, fraction: f64) -> Duration {
        let cap = to_millis(self.open_max);
        let open = grow(
            to_millis(self.open_base),
            OPEN_GROWTH,
            opens.saturating_sub(1),
        )
        .min(cap);
        let jitter_share = f64::from(self.jitter_percent.min(PERCENT)) / f64::from(PERCENT);
        let jitter = scale_floor(open, sanitize(jitter_share * sanitize(fraction)));
        from_millis(open.saturating_sub(jitter))
    }
}

/// The result of one call, as the breaker sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The destination answered.
    Success,
    /// The destination is unavailable.
    Failure,
    /// The call timed out.
    Timeout,
    /// The call says nothing about the destination's health.
    Ignored,
    /// The destination asked for no calls for this long (`Retry-After`).
    HoldFor(Duration),
}

/// What an admitted call was admitted as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PermitKind {
    /// Admitted while closed.
    Allowed,
    /// Admitted as the half-open probe.
    Probe,
}

/// Proof of admission that [`BreakerMachine::record`] takes back.
///
/// Outcomes only count for the phase that issued the permit: a call admitted
/// while closed that finishes after the breaker opened, or a probe that was
/// replaced, is ignored (except `HoldFor`, which may still extend an open breaker).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Permit {
    kind: PermitKind,
    ticket: u64,
}

impl Permit {
    /// What the call was admitted as.
    #[must_use]
    pub const fn kind(&self) -> PermitKind {
        self.kind
    }

    /// The generation the permit belongs to.
    #[must_use]
    pub const fn ticket(&self) -> u64 {
        self.ticket
    }
}

/// Whether a call may go ahead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// Closed: go ahead and record the outcome with the permit.
    Allowed(Permit),
    /// Half-open: this call is the single probe and must record its outcome.
    Probe(Permit),
    /// Open: do not call before `retry_after` has passed.
    Rejected {
        /// Time until the breaker admits a call again.
        retry_after: Duration,
    },
}

impl Admission {
    /// Whether the call may go ahead.
    #[must_use]
    pub const fn is_admitted(&self) -> bool {
        !matches!(self, Self::Rejected { .. })
    }

    /// The permit to record the outcome with, `None` when rejected.
    #[must_use]
    pub const fn permit(&self) -> Option<Permit> {
        match self {
            Self::Allowed(permit) | Self::Probe(permit) => Some(*permit),
            Self::Rejected { .. } => None,
        }
    }
}

/// The externally visible breaker state, ordered from healthy to unhealthy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BreakerState {
    /// Calls flow.
    Closed,
    /// One probe decides whether to close.
    HalfOpen,
    /// Calls are rejected.
    Open,
}

impl BreakerState {
    /// Stable label value.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::HalfOpen => "half_open",
            Self::Open => "open",
        }
    }

    /// Gauge value: `0` closed, `1` half-open, `2` open.
    #[must_use]
    pub const fn gauge_value(&self) -> u8 {
        match self {
            Self::Closed => 0,
            Self::HalfOpen => 1,
            Self::Open => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    Closed {
        failures: VecDeque<bool>,
        consecutive_timeouts: u32,
    },
    Open {
        until: Instant,
        opens: u32,
    },
    HalfOpen {
        probe_since: Option<Instant>,
        opens: u32,
    },
}

impl Phase {
    const fn closed() -> Self {
        Self::Closed {
            failures: VecDeque::new(),
            consecutive_timeouts: 0,
        }
    }
}

/// The pure breaker state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreakerMachine {
    phase: Phase,
    generation: u64,
}

impl Default for BreakerMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl BreakerMachine {
    /// A closed breaker with an empty window.
    #[must_use]
    pub const fn new() -> Self {
        Self::starting_at(0)
    }

    /// A closed breaker whose permits start at `generation`. A [`BreakerSet`]
    /// uses distinct starts so permits from an evicted breaker never match.
    ///
    /// [`BreakerSet`]: crate::BreakerSet
    #[must_use]
    pub const fn starting_at(generation: u64) -> Self {
        Self {
            phase: Phase::closed(),
            generation,
        }
    }

    /// The current state. An open breaker whose time has passed reports
    /// `Open` until the next [`admit`](Self::admit) moves it to half-open.
    #[must_use]
    pub const fn state(&self) -> BreakerState {
        match self.phase {
            Phase::Closed { .. } => BreakerState::Closed,
            Phase::Open { .. } => BreakerState::Open,
            Phase::HalfOpen { .. } => BreakerState::HalfOpen,
        }
    }

    /// Decides whether a call at `now` may go ahead.
    pub fn admit(&mut self, policy: &BreakerPolicy, now: Instant) -> Admission {
        match &mut self.phase {
            Phase::Closed { .. } => Admission::Allowed(self.permit(PermitKind::Allowed)),
            Phase::Open { until, opens } => {
                let (until, opens) = (*until, *opens);
                if now < until {
                    return Admission::Rejected {
                        retry_after: until.saturating_duration_since(now),
                    };
                }
                self.enter(Phase::HalfOpen {
                    probe_since: Some(now),
                    opens,
                });
                Admission::Probe(self.permit(PermitKind::Probe))
            }
            Phase::HalfOpen { probe_since, .. } => {
                let lease_end =
                    probe_since.and_then(|since| since.checked_add(policy.probe_timeout));
                if let Some(end) = lease_end.filter(|end| now < *end) {
                    return Admission::Rejected {
                        retry_after: end.saturating_duration_since(now),
                    };
                }
                *probe_since = Some(now);
                self.bump();
                Admission::Probe(self.permit(PermitKind::Probe))
            }
        }
    }

    /// Records the outcome of the call admitted with `permit`. `fraction` in
    /// `[0, 1)` jitters the open time if this outcome opens the breaker.
    pub fn record(
        &mut self,
        policy: &BreakerPolicy,
        permit: Permit,
        outcome: Outcome,
        now: Instant,
        fraction: f64,
    ) {
        let current = permit.ticket == self.generation;
        let next = match &mut self.phase {
            Phase::Closed {
                failures,
                consecutive_timeouts,
            } if current && permit.kind == PermitKind::Allowed => record_closed(
                policy,
                failures,
                consecutive_timeouts,
                outcome,
                now,
                fraction,
            ),
            Phase::Closed { .. } => None,
            Phase::Open { until, opens } => record_open(policy, *until, *opens, outcome, now),
            Phase::HalfOpen { opens, .. } if current && permit.kind == PermitKind::Probe => {
                record_half_open(policy, *opens, outcome, now, fraction)
            }
            Phase::HalfOpen { .. } => None,
        };
        if let Some(phase) = next {
            self.enter(phase);
        }
    }

    const fn permit(&self, kind: PermitKind) -> Permit {
        Permit {
            kind,
            ticket: self.generation,
        }
    }

    fn enter(&mut self, phase: Phase) {
        self.phase = phase;
        self.bump();
    }

    const fn bump(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }
}

fn record_closed(
    policy: &BreakerPolicy,
    failures: &mut VecDeque<bool>,
    consecutive_timeouts: &mut u32,
    outcome: Outcome,
    now: Instant,
    fraction: f64,
) -> Option<Phase> {
    let failed = match outcome {
        Outcome::Ignored => return None,
        Outcome::HoldFor(delay) => return Some(hold(policy, now, delay, 0)),
        Outcome::Success => {
            *consecutive_timeouts = 0;
            false
        }
        Outcome::Failure => {
            *consecutive_timeouts = 0;
            true
        }
        Outcome::Timeout => {
            *consecutive_timeouts = consecutive_timeouts.saturating_add(1);
            true
        }
    };
    failures.push_back(failed);
    let window = usize::try_from(policy.window.max(1)).unwrap_or(usize::MAX);
    while failures.len() > window {
        failures.pop_front();
    }
    should_trip(policy, failures, *consecutive_timeouts).then(|| open(policy, now, 1, fraction))
}

fn should_trip(
    policy: &BreakerPolicy,
    failures: &VecDeque<bool>,
    consecutive_timeouts: u32,
) -> bool {
    if consecutive_timeouts >= policy.consecutive_timeouts.max(1) {
        return true;
    }
    let calls = u32::try_from(failures.len()).unwrap_or(u32::MAX);
    if calls < policy.min_calls.max(1) {
        return false;
    }
    let failed =
        u32::try_from(failures.iter().filter(|failed| **failed).count()).unwrap_or(u32::MAX);
    u64::from(failed) * u64::from(PERCENT)
        >= u64::from(policy.failure_rate_percent) * u64::from(calls)
}

fn record_open(
    policy: &BreakerPolicy,
    until: Instant,
    opens: u32,
    outcome: Outcome,
    now: Instant,
) -> Option<Phase> {
    let Outcome::HoldFor(delay) = outcome else {
        return None;
    };
    let held = hold(policy, now, delay, opens);
    match held {
        Phase::Open {
            until: held_until, ..
        } if held_until > until => Some(held),
        _ => None,
    }
}

fn record_half_open(
    policy: &BreakerPolicy,
    opens: u32,
    outcome: Outcome,
    now: Instant,
    fraction: f64,
) -> Option<Phase> {
    match outcome {
        Outcome::Success => Some(Phase::closed()),
        Outcome::Failure | Outcome::Timeout => {
            Some(open(policy, now, opens.saturating_add(1), fraction))
        }
        Outcome::HoldFor(delay) => Some(hold(policy, now, delay, opens)),
        Outcome::Ignored => Some(Phase::HalfOpen {
            probe_since: None,
            opens,
        }),
    }
}

fn open(policy: &BreakerPolicy, now: Instant, opens: u32, fraction: f64) -> Phase {
    open_for(now, policy.open_duration(opens, fraction), opens)
}

fn hold(policy: &BreakerPolicy, now: Instant, delay: Duration, opens: u32) -> Phase {
    open_for(now, delay.min(policy.open_max), opens)
}

fn open_for(now: Instant, duration: Duration, opens: u32) -> Phase {
    let until = now.checked_add(duration).unwrap_or(now);
    Phase::Open { until, opens }
}

struct Guarded {
    machine: BreakerMachine,
    jitter: Box<dyn JitterSource>,
}

/// A thread-safe [`BreakerMachine`] on a clock. Clones share state.
#[derive(Clone)]
pub struct CircuitBreaker {
    policy: BreakerPolicy,
    clock: Arc<dyn Clock>,
    inner: Arc<Mutex<Guarded>>,
}

impl std::fmt::Debug for CircuitBreaker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CircuitBreaker")
            .field("policy", &self.policy)
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}

impl CircuitBreaker {
    /// A breaker on the system clock with entropy-seeded jitter.
    #[must_use]
    pub fn new(policy: BreakerPolicy) -> Self {
        Self::with_parts(
            policy,
            Arc::new(SystemClock),
            Box::new(SplitMixJitter::from_entropy()),
        )
    }

    /// A breaker on the given clock and jitter source.
    #[must_use]
    pub fn with_parts(
        policy: BreakerPolicy,
        clock: Arc<dyn Clock>,
        jitter: Box<dyn JitterSource>,
    ) -> Self {
        Self {
            policy,
            clock,
            inner: Arc::new(Mutex::new(Guarded {
                machine: BreakerMachine::new(),
                jitter,
            })),
        }
    }

    /// The policy.
    #[must_use]
    pub const fn policy(&self) -> &BreakerPolicy {
        &self.policy
    }

    /// Decides whether a call may go ahead now.
    #[must_use]
    pub fn admit(&self) -> Admission {
        let now = self.clock.now();
        self.lock().machine.admit(&self.policy, now)
    }

    /// Records the outcome of the call admitted with `permit`.
    pub fn record(&self, permit: Permit, outcome: Outcome) {
        let now = self.clock.now();
        let mut guarded = self.lock();
        let fraction = guarded.jitter.next_fraction();
        guarded
            .machine
            .record(&self.policy, permit, outcome, now, fraction);
    }

    /// The current state.
    #[must_use]
    pub fn state(&self) -> BreakerState {
        self.lock().machine.state()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Guarded> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
