use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant, SystemTime},
};

use crate::{
    AmountBudgetStore, AmountBudgetStoreDecision, Quota, RateLimitDecision, RateLimitStore,
    RateLimitStoreError,
    kernels::bucket::{ScaledQuota, admit, refill},
};

const DEFAULT_MAX_ENTRIES: usize = 10_000;
const PRUNE_INTERVAL: u64 = 256;

/// A bounded, process-local token-bucket adapter.
#[derive(Clone, Debug)]
pub struct InMemoryRateLimitStore {
    inner: std::sync::Arc<Mutex<State>>,
    max_entries: usize,
    epoch: Instant,
}

#[derive(Debug, Default)]
struct State {
    entries: HashMap<String, Entry>,
    amount_entries: HashMap<String, AmountEntry>,
    checks: u64,
    amount_checks: u64,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    tokens: u64,
    last_ms: u64,
    rate: u64,
    capacity: u64,
}

impl Entry {
    fn is_full_at(&self, now_ms: u64) -> bool {
        refill(self.tokens, self.last_ms, now_ms, self.rate, self.capacity) >= self.capacity
    }
}

#[derive(Clone, Copy, Debug)]
struct AmountEntry {
    consumed: u64,
    reset_at: SystemTime,
}

impl InMemoryRateLimitStore {
    /// Creates a store capped at `max_entries` active keys per limiter type.
    ///
    /// # Errors
    ///
    /// [`InMemoryStoreError::ZeroCapacity`] when `max_entries` is zero.
    pub fn new(max_entries: usize) -> Result<Self, InMemoryStoreError> {
        if max_entries == 0 {
            return Err(InMemoryStoreError::ZeroCapacity);
        }
        Ok(Self::with_max_entries(max_entries))
    }

    fn with_max_entries(max_entries: usize) -> Self {
        Self {
            inner: std::sync::Arc::new(Mutex::new(State::default())),
            max_entries,
            epoch: Instant::now(),
        }
    }

    fn check_at(
        &self,
        key: &str,
        quota: Quota,
        now: Instant,
    ) -> Result<RateLimitDecision, RateLimitStoreError> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| RateLimitStoreError::unavailable("in-memory store lock poisoned"))?;
        let now_ms = self.millis_since_epoch(now);
        state.checks = state.checks.wrapping_add(1);
        if state.checks % PRUNE_INTERVAL == 0 || state.entries.len() >= self.max_entries {
            state.entries.retain(|_, entry| !entry.is_full_at(now_ms));
        }
        if !state.entries.contains_key(key) && state.entries.len() >= self.max_entries {
            return Err(RateLimitStoreError::unavailable(
                "in-memory store is full of buckets that are still refilling",
            ));
        }

        let scaled = ScaledQuota::from_quota(quota);
        let entry = state.entries.entry(key.to_owned()).or_insert(Entry {
            tokens: scaled.capacity,
            last_ms: now_ms,
            rate: scaled.rate,
            capacity: scaled.capacity,
        });
        let (allowed, tokens, retry_ms) = admit(
            entry.tokens,
            entry.last_ms,
            now_ms,
            scaled.rate,
            scaled.capacity,
            scaled.cost,
        );
        *entry = Entry {
            tokens,
            last_ms: now_ms.max(entry.last_ms),
            rate: scaled.rate,
            capacity: scaled.capacity,
        };
        let remaining = tokens / scaled.cost;
        if allowed {
            Ok(RateLimitDecision::allowed(remaining))
        } else {
            Ok(RateLimitDecision::limited(
                remaining,
                Duration::from_millis(retry_ms),
            ))
        }
    }

    fn millis_since_epoch(&self, now: Instant) -> u64 {
        u64::try_from(now.saturating_duration_since(self.epoch).as_millis()).unwrap_or(u64::MAX)
    }

    fn check_amount_at(
        &self,
        key: &str,
        amount: u64,
        limit: u64,
        now: SystemTime,
        reset_at: SystemTime,
    ) -> Result<AmountBudgetStoreDecision, RateLimitStoreError> {
        if reset_at <= now {
            return Err(RateLimitStoreError::unavailable(
                "amount-budget reset must be after the current time",
            ));
        }
        let mut state = self
            .inner
            .lock()
            .map_err(|_| RateLimitStoreError::unavailable("in-memory store lock poisoned"))?;
        state.amount_checks = state.amount_checks.wrapping_add(1);
        if state.amount_checks % PRUNE_INTERVAL == 0
            || state.amount_entries.len() >= self.max_entries
        {
            state.amount_entries.retain(|_, entry| entry.reset_at > now);
        }
        if !state.amount_entries.contains_key(key) && state.amount_entries.len() >= self.max_entries
        {
            remove_earliest_amount_entry(&mut state);
        }

        let entry = state
            .amount_entries
            .entry(key.to_owned())
            .or_insert(AmountEntry {
                consumed: 0,
                reset_at,
            });
        entry.reset_at = reset_at;
        let allowed = entry.consumed <= limit && amount <= limit - entry.consumed;
        if allowed {
            entry.consumed += amount;
        }
        Ok(AmountBudgetStoreDecision {
            allowed,
            remaining: limit.saturating_sub(entry.consumed),
        })
    }

    fn release_amount_at(
        &self,
        key: &str,
        amount: u64,
        limit: u64,
        now: SystemTime,
        reset_at: SystemTime,
    ) -> Result<AmountBudgetStoreDecision, RateLimitStoreError> {
        if reset_at <= now {
            return Err(RateLimitStoreError::unavailable(
                "amount-budget reset must be after the current time",
            ));
        }
        let mut state = self
            .inner
            .lock()
            .map_err(|_| RateLimitStoreError::unavailable("in-memory store lock poisoned"))?;
        state.amount_checks = state.amount_checks.wrapping_add(1);
        if state.amount_checks % PRUNE_INTERVAL == 0
            || state.amount_entries.len() >= self.max_entries
        {
            state.amount_entries.retain(|_, entry| entry.reset_at > now);
        }
        if state
            .amount_entries
            .get(key)
            .is_some_and(|entry| entry.reset_at <= now)
        {
            state.amount_entries.remove(key);
        }
        let consumed = state.amount_entries.get_mut(key).map_or(0, |entry| {
            entry.consumed = entry.consumed.saturating_sub(amount);
            entry.reset_at = reset_at;
            entry.consumed
        });
        Ok(AmountBudgetStoreDecision {
            allowed: true,
            remaining: limit.saturating_sub(consumed),
        })
    }
}

fn remove_earliest_amount_entry(state: &mut State) {
    let earliest = state
        .amount_entries
        .iter()
        .min_by_key(|(_, entry)| entry.reset_at)
        .map(|(key, _)| key.clone());
    if let Some(earliest) = earliest {
        state.amount_entries.remove(&earliest);
    }
}

impl Default for InMemoryRateLimitStore {
    fn default() -> Self {
        Self::with_max_entries(DEFAULT_MAX_ENTRIES)
    }
}

impl RateLimitStore for InMemoryRateLimitStore {
    fn check_and_consume<'a>(
        &'a self,
        key: &'a str,
        quota: Quota,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<RateLimitDecision, RateLimitStoreError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move { self.check_at(key, quota, Instant::now()) })
    }
}

impl AmountBudgetStore for InMemoryRateLimitStore {
    fn check_and_consume_amount<'a>(
        &'a self,
        key: &'a str,
        amount: u64,
        limit: u64,
        now: SystemTime,
        reset_at: SystemTime,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<AmountBudgetStoreDecision, RateLimitStoreError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move { self.check_amount_at(key, amount, limit, now, reset_at) })
    }

    fn release_amount<'a>(
        &'a self,
        key: &'a str,
        amount: u64,
        limit: u64,
        now: SystemTime,
        reset_at: SystemTime,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<AmountBudgetStoreDecision, RateLimitStoreError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move { self.release_amount_at(key, amount, limit, now, reset_at) })
    }
}

/// Invalid in-memory adapter construction.
#[derive(Clone, Copy, Debug, thiserror::Error, Eq, PartialEq)]
pub enum InMemoryStoreError {
    /// At least one key slot is required.
    #[error("in-memory store capacity must be non-zero")]
    ZeroCapacity,
}

#[cfg(test)]
mod tests {
    use std::time::UNIX_EPOCH;

    use super::*;

    #[test]
    fn burst_exhaustion_and_refill_have_token_bucket_semantics() {
        let store = InMemoryRateLimitStore::new(8).expect("store");
        let quota = Quota::new(2, Duration::from_secs(2), 1).expect("quota");
        let started = Instant::now();

        for expected_remaining in [2, 1, 0] {
            let decision = store.check_at("id:a", quota, started).expect("decision");
            assert!(decision.allowed);
            assert_eq!(decision.remaining, expected_remaining);
        }
        let exhausted = store.check_at("id:a", quota, started).expect("decision");
        assert!(!exhausted.allowed);
        assert_eq!(exhausted.retry_after, Duration::from_secs(1));

        let one_token = store
            .check_at("id:a", quota, started + Duration::from_secs(1))
            .expect("decision");
        assert!(one_token.allowed);
        assert_eq!(one_token.remaining, 0);
        let full_again = store
            .check_at("id:a", quota, started + Duration::from_secs(4))
            .expect("decision");
        assert!(full_again.allowed);
        assert_eq!(full_again.remaining, 2);
    }

    #[test]
    fn never_refilling_quota_admits_its_full_capacity() {
        let store = InMemoryRateLimitStore::new(8).expect("store");
        let quota = Quota::new(100, Duration::MAX, 0).expect("quota");
        let started = Instant::now();

        for _ in 0..100 {
            assert!(
                store
                    .check_at("id:a", quota, started)
                    .expect("decision")
                    .allowed
            );
        }
        assert!(
            !store
                .check_at("id:a", quota, started)
                .expect("decision")
                .allowed
        );
    }

    #[test]
    fn capacity_is_bounded_and_expired_entries_are_pruned() {
        let store = InMemoryRateLimitStore::new(1).expect("store");
        let quota = Quota::new(1, Duration::from_secs(1), 0).expect("quota");
        let started = Instant::now();
        store.check_at("first", quota, started).expect("decision");
        store
            .check_at("second", quota, started + Duration::from_secs(2))
            .expect("decision");
        let state = store.inner.lock().expect("state");
        assert_eq!(state.entries.len(), 1);
        assert!(state.entries.contains_key("second"));
    }

    #[test]
    fn a_drained_bucket_is_never_evicted_to_come_back_full() {
        let store = InMemoryRateLimitStore::new(1).expect("store");
        let quota = Quota::new(1, Duration::from_secs(60), 0).expect("quota");
        let started = Instant::now();
        assert!(
            store
                .check_at("victim", quota, started)
                .expect("decision")
                .allowed
        );

        assert!(store.check_at("attacker", quota, started).is_err());
        let again = store.check_at("victim", quota, started).expect("decision");
        assert!(!again.allowed);
    }

    #[test]
    fn amount_budget_allows_limit_minus_one_and_limit_but_denies_limit_plus_one() {
        let store = InMemoryRateLimitStore::new(8).expect("store");
        let now = UNIX_EPOCH + Duration::from_secs(1_000);
        let reset_at = now + Duration::from_secs(60);

        let below = store
            .check_amount_at("below", 9, 10, now, reset_at)
            .expect("decision");
        assert!(below.allowed);
        assert_eq!(below.remaining, 1);

        let at = store
            .check_amount_at("at", 10, 10, now, reset_at)
            .expect("decision");
        assert!(at.allowed);
        assert_eq!(at.remaining, 0);

        let above = store
            .check_amount_at("above", 11, 10, now, reset_at)
            .expect("decision");
        assert!(!above.allowed);
        assert_eq!(above.remaining, 10);
    }

    #[test]
    fn amount_budget_does_not_consume_a_denied_amount() {
        let store = InMemoryRateLimitStore::new(8).expect("store");
        let now = UNIX_EPOCH + Duration::from_secs(1_000);
        let reset_at = now + Duration::from_secs(60);
        assert!(
            store
                .check_amount_at("budget", 9, 10, now, reset_at)
                .expect("decision")
                .allowed
        );
        assert!(
            !store
                .check_amount_at("budget", 2, 10, now, reset_at)
                .expect("decision")
                .allowed
        );
        let final_unit = store
            .check_amount_at("budget", 1, 10, now, reset_at)
            .expect("decision");
        assert!(final_unit.allowed);
        assert_eq!(final_unit.remaining, 0);
    }

    #[test]
    fn amount_budget_release_is_floored_at_zero() {
        let store = InMemoryRateLimitStore::new(8).expect("store");
        let now = UNIX_EPOCH + Duration::from_secs(1_000);
        let reset_at = now + Duration::from_secs(60);
        store
            .check_amount_at("budget", 7, 10, now, reset_at)
            .expect("consume");

        let partial = store
            .release_amount_at("budget", 3, 10, now, reset_at)
            .expect("release");
        assert!(partial.allowed);
        assert_eq!(partial.remaining, 6);

        let floored = store
            .release_amount_at("budget", 20, 10, now, reset_at)
            .expect("release");
        assert!(floored.allowed);
        assert_eq!(floored.remaining, 10);
    }

    #[test]
    fn amount_entries_expire_against_the_injected_time() {
        let store = InMemoryRateLimitStore::new(1).expect("store");
        let first_now = UNIX_EPOCH + Duration::from_secs(1_000);
        let boundary = first_now + Duration::from_secs(10);
        store
            .check_amount_at("old-window", 10, 10, first_now, boundary)
            .expect("decision");

        store
            .check_amount_at(
                "new-window",
                1,
                10,
                boundary,
                boundary + Duration::from_secs(10),
            )
            .expect("decision");

        let state = store.inner.lock().expect("state");
        assert_eq!(state.amount_entries.len(), 1);
        assert!(!state.amount_entries.contains_key("old-window"));
        assert!(state.amount_entries.contains_key("new-window"));
    }

    mod properties {
        use proptest::prelude::*;

        use super::*;

        const KEYS: usize = 4;
        const MAX_ENTRIES: usize = 2;

        proptest! {
            /// P-01: admitted requests per key in any interval stay within
            /// `capacity + rate * length`, with eviction pressure from more keys than slots.
            #[test]
            fn admitted_cost_never_exceeds_capacity_plus_refill(
                requests in 1_u64..5,
                burst in 0_u64..3,
                period_ms in 1_u64..2_000,
                trace in prop::collection::vec((0..KEYS, 0_u64..1_500), 1..200),
            ) {
                let store = InMemoryRateLimitStore::new(MAX_ENTRIES).expect("store");
                let quota = Quota::new(requests, Duration::from_millis(period_ms), burst)
                    .expect("quota");
                let mut now_ms = 0_u64;
                let mut admitted: Vec<Vec<u64>> = vec![Vec::new(); KEYS];
                for (key, step) in trace {
                    now_ms += step;
                    let now = store.epoch + Duration::from_millis(now_ms);
                    let name = format!("k{key}");
                    if store.check_at(&name, quota, now).is_ok_and(|decision| decision.allowed) {
                        admitted[key].push(now_ms);
                    }
                }
                for times in &admitted {
                    for (first, start) in times.iter().enumerate() {
                        for (last, end) in times.iter().enumerate().skip(first) {
                            let count = u128::try_from(last - first + 1).expect("count");
                            let budget = u128::from(quota.capacity()) * u128::from(period_ms)
                                + u128::from(requests) * u128::from(end - start);
                            prop_assert!(count * u128::from(period_ms) <= budget);
                        }
                    }
                }
            }
        }
    }
}
