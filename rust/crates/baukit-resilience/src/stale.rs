//! A single-flight cache that serves the last good value when the source fails.
//!
//! A value is fresh for [`StalePolicy::fresh_for`], then stale for
//! [`StalePolicy::stale_for`], then gone. Callers holding a fresh or stale
//! value never wait for another caller's fetch.

use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync::Mutex as FlightLock;

use crate::clock::{Clock, SystemClock};

/// Lifetimes for a [`StaleCache`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StalePolicy {
    /// How long a fetched value is served without a refresh.
    pub fresh_for: Duration,
    /// How long after `fresh_for` the value is served when refreshing fails.
    pub stale_for: Duration,
    /// Minimum gap after a failed fetch, and between forced refreshes.
    pub failure_cooldown: Duration,
}

impl StalePolicy {
    /// A policy from its three durations.
    #[must_use]
    pub const fn new(fresh_for: Duration, stale_for: Duration, failure_cooldown: Duration) -> Self {
        Self {
            fresh_for,
            stale_for,
            failure_cooldown,
        }
    }
}

/// Where a returned value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// Cached and within `fresh_for`.
    Fresh,
    /// Cached, past `fresh_for`, within `stale_for`; refreshing failed or is cooling down or in flight.
    Stale,
    /// Fetched by this call.
    Fetched,
}

impl Freshness {
    /// Stable label value.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Stale => "stale",
            Self::Fetched => "fetched",
        }
    }
}

/// A value and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheRead<T> {
    /// The value.
    pub value: T,
    /// Where it came from.
    pub freshness: Freshness,
    /// This call fetched, the fetch failed, and the stale value was served instead.
    pub refresh_failed: bool,
}

/// Why no value could be returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StaleCacheError<E> {
    /// The fetch failed and no usable value is cached.
    Fetch(E),
    /// A recent failure blocks fetching and no usable value is cached.
    CoolingDown {
        /// Time until the next fetch is allowed.
        retry_after: Duration,
    },
}

impl<E: std::fmt::Display> std::fmt::Display for StaleCacheError<E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fetch(error) => write!(formatter, "fetch failed: {error}"),
            Self::CoolingDown { retry_after } => {
                write!(
                    formatter,
                    "fetch cooling down for {} ms",
                    retry_after.as_millis()
                )
            }
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for StaleCacheError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Fetch(error) => Some(error),
            Self::CoolingDown { .. } => None,
        }
    }
}

type ReadResult<T, E> = Result<CacheRead<T>, StaleCacheError<E>>;

struct Slot<T> {
    value: Option<(T, Instant)>,
    last_failure: Option<Instant>,
    last_forced: Option<Instant>,
    generation: u64,
}

/// See the module docs.
pub struct StaleCache<T> {
    policy: StalePolicy,
    clock: Arc<dyn Clock>,
    slot: Mutex<Slot<T>>,
    flight: FlightLock<()>,
}

impl<T> std::fmt::Debug for StaleCache<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StaleCache")
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

impl<T: Clone> StaleCache<T> {
    /// An empty cache on the system clock.
    #[must_use]
    pub fn new(policy: StalePolicy) -> Self {
        Self::with_clock(policy, Arc::new(SystemClock))
    }

    /// An empty cache on `clock`.
    #[must_use]
    pub fn with_clock(policy: StalePolicy, clock: Arc<dyn Clock>) -> Self {
        Self {
            policy,
            clock,
            slot: Mutex::new(Slot {
                value: None,
                last_failure: None,
                last_forced: None,
                generation: 0,
            }),
            flight: FlightLock::new(()),
        }
    }

    /// The cached value if it is fresh or stale, without fetching.
    #[must_use]
    pub fn current(&self) -> Option<CacheRead<T>> {
        let now = self.clock.now();
        self.cached(&self.lock(), now)
    }

    /// Returns the fresh value, or fetches once across concurrent callers.
    ///
    /// While a fetch is in flight or cooling down, callers get the stale
    /// value if there is one. Callers without any usable value wait for the
    /// in-flight fetch and share its result.
    ///
    /// # Errors
    ///
    /// [`StaleCacheError::Fetch`] when the fetch fails and nothing usable is
    /// cached, [`StaleCacheError::CoolingDown`] when a recent failure blocks
    /// fetching and nothing usable is cached.
    pub async fn get_or_refresh<F, Fut, E>(&self, fetch: F) -> ReadResult<T, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        if let Some(served) = self.serve_without_fetch(self.clock.now()) {
            return served;
        }
        let _flight = match self.flight.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                if let Some(stale) = self.current() {
                    return Ok(stale);
                }
                let guard = self.flight.lock().await;
                if let Some(served) = self.serve_without_fetch(self.clock.now()) {
                    return served;
                }
                guard
            }
        };
        self.fetch_into_slot(fetch, false).await
    }

    /// Fetches even if the value is fresh, for example for an unknown key id.
    ///
    /// Within the cooldown after the last failed or forced fetch this returns
    /// the cached value instead. Concurrent forced refreshes share one fetch.
    ///
    /// # Errors
    ///
    /// As [`get_or_refresh`](Self::get_or_refresh).
    pub async fn force_refresh<F, Fut, E>(&self, fetch: F) -> ReadResult<T, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let generation = {
            let slot = self.lock();
            if let Some(retry_after) = self.force_cooldown(&slot, self.clock.now()) {
                return self.cached_or_cooling(&slot, retry_after);
            }
            slot.generation
        };
        let _flight = self.flight.lock().await;
        {
            let slot = self.lock();
            if slot.generation != generation {
                let now = self.clock.now();
                let retry_after = self.force_cooldown(&slot, now).unwrap_or_default();
                return self.cached_or_cooling(&slot, retry_after);
            }
        }
        self.fetch_into_slot(fetch, true).await
    }

    fn serve_without_fetch<E>(&self, now: Instant) -> Option<ReadResult<T, E>> {
        let slot = self.lock();
        let cached = self.cached(&slot, now);
        if let Some(read) = cached
            .as_ref()
            .filter(|read| read.freshness == Freshness::Fresh)
        {
            return Some(Ok(read.clone()));
        }
        let retry_after = remaining(slot.last_failure, self.policy.failure_cooldown, now)?;
        Some(cached.ok_or(StaleCacheError::CoolingDown { retry_after }))
    }

    async fn fetch_into_slot<F, Fut, E>(&self, fetch: F, forced: bool) -> ReadResult<T, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let result = fetch().await;
        let now = self.clock.now();
        let mut slot = self.lock();
        slot.generation = slot.generation.wrapping_add(1);
        if forced {
            slot.last_forced = Some(now);
        }
        match result {
            Ok(value) => {
                slot.value = Some((value.clone(), now));
                slot.last_failure = None;
                Ok(CacheRead {
                    value,
                    freshness: Freshness::Fetched,
                    refresh_failed: false,
                })
            }
            Err(error) => {
                slot.last_failure = Some(now);
                self.cached(&slot, now)
                    .map(|read| CacheRead {
                        refresh_failed: true,
                        ..read
                    })
                    .ok_or(StaleCacheError::Fetch(error))
            }
        }
    }

    fn cached(&self, slot: &Slot<T>, now: Instant) -> Option<CacheRead<T>> {
        let (value, fetched_at) = slot.value.as_ref()?;
        let age = now.saturating_duration_since(*fetched_at);
        let freshness = if age < self.policy.fresh_for {
            Freshness::Fresh
        } else if age < self.policy.fresh_for.saturating_add(self.policy.stale_for) {
            Freshness::Stale
        } else {
            return None;
        };
        Some(CacheRead {
            value: value.clone(),
            freshness,
            refresh_failed: false,
        })
    }

    fn cached_or_cooling<E>(&self, slot: &Slot<T>, retry_after: Duration) -> ReadResult<T, E> {
        self.cached(slot, self.clock.now())
            .ok_or(StaleCacheError::CoolingDown { retry_after })
    }

    fn force_cooldown(&self, slot: &Slot<T>, now: Instant) -> Option<Duration> {
        let last = slot.last_failure.max(slot.last_forced);
        remaining(last, self.policy.failure_cooldown, now)
    }

    fn lock(&self) -> MutexGuard<'_, Slot<T>> {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn remaining(since: Option<Instant>, window: Duration, now: Instant) -> Option<Duration> {
    let end = since?.checked_add(window)?;
    (now < end).then(|| end.saturating_duration_since(now))
}
