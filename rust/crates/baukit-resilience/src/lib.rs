//! Framework-free resilience building blocks.
//!
//! - [`clock`]: the [`Clock`] port and [`SystemClock`].
//! - [`backoff`]: [`Backoff`] with full or decorrelated jitter.
//! - [`breaker`]: the [`CircuitBreaker`] state machine and its policy.
//! - [`breaker_set`]: [`BreakerSet`], one breaker per key under a shared policy.
//! - [`deadline`]: [`Deadline`], a fixed point in time that caps timeouts.
//! - [`stale`]: [`StaleCache`], a single-flight cache that serves stale values on error.
//!
//! Every time-dependent decision takes `now` from a [`Clock`] and every random
//! decision takes a fraction in `[0, 1)` from a [`JitterSource`], so the logic
//! can be replayed from the vectors in `fixtures/resilience`.

pub mod backoff;
pub mod breaker;
pub mod breaker_set;
pub mod clock;
pub mod deadline;
pub mod jitter;
mod millis;
pub mod stale;

pub use backoff::{Backoff, BackoffStrategy};
pub use breaker::{
    Admission, BreakerMachine, BreakerPolicy, BreakerState, CircuitBreaker, Outcome, Permit,
    PermitKind,
};
pub use breaker_set::{BreakerSet, DEFAULT_MAX_KEYS};
#[cfg(feature = "test-support")]
pub use clock::ManualClock;
pub use clock::{Clock, SystemClock};
pub use deadline::Deadline;
pub use jitter::{JitterSource, SplitMixJitter};
pub use stale::{CacheRead, Freshness, StaleCache, StaleCacheError, StalePolicy};
