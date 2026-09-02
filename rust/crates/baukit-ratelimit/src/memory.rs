use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Instant, SystemTime},
};

use crate::{
    AmountBudgetStore, AmountBudgetStoreDecision, Quota, RateLimitDecision, RateLimitStore,
    RateLimitStoreError,
};

const DEFAULT_MAX_ENTRIES: usize = 10_000;
const PRUNE_INTERVAL: u64 = 256;

/// A bounded, process-local token-bucket adapter.
#[derive(Clone, Debug)]
pub struct InMemoryRateLimitStore {
    inner: std::sync::Arc<Mutex<State>>,
    max_entries: usize,
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
    tokens: f64,
    last_refill: Instant,
    expires_at: Instant,
}

#[derive(Clone, Copy, Debug)]
struct AmountEntry {
    consumed: u64,
    reset_at: SystemTime,
}

impl InMemoryRateLimitStore {
    /// Creates a store capped at `max_entries` active keys per limiter type.
    pub fn new(max_entries: usize) -> Result<Self, InMemoryStoreError> {
        if max_entries == 0 {
            return Err(InMemoryStoreError::ZeroCapacity);
        }
        Ok(Self {
            inner: std::sync::Arc::new(Mutex::new(State::default())),
            max_entries,
        })
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
        state.checks = state.checks.wrapping_add(1);
        if state.checks % PRUNE_INTERVAL == 0 || state.entries.len() >= self.max_entries {
            state.entries.retain(|_, entry| entry.expires_at > now);
        }
        if !state.entries.contains_key(key) && state.entries.len() >= self.max_entries {
            let oldest = state
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.expires_at)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                state.entries.remove(&oldest);
            }
        }

        let capacity = quota.capacity() as f64;
        let entry = state.entries.entry(key.to_owned()).or_insert(Entry {
            tokens: capacity,
            last_refill: now,
            expires_at: expires_at(now, quota),
        });
        let elapsed = now.saturating_duration_since(entry.last_refill);
        let refill = elapsed.as_secs_f64() * quota.requests_per_period() as f64
            / quota.period().as_secs_f64();
        entry.tokens = (entry.tokens + refill).min(capacity);
        entry.last_refill = now;
        entry.expires_at = expires_at(now, quota);

        if entry.tokens >= 1.0 {
            entry.tokens -= 1.0;
            Ok(RateLimitDecision::allowed(entry.tokens.floor() as u64))
        } else {
            let seconds = (1.0 - entry.tokens) * quota.period().as_secs_f64()
                / quota.requests_per_period() as f64;
            Ok(RateLimitDecision::limited(
                entry.tokens.floor() as u64,
                std::time::Duration::from_secs_f64(seconds.max(0.000_001)),
            ))
        }
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

fn expires_at(now: Instant, quota: Quota) -> Instant {
    now.checked_add(quota.idle_ttl())
        .unwrap_or_else(|| now + std::time::Duration::from_secs(365 * 24 * 60 * 60))
}

impl Default for InMemoryRateLimitStore {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_ENTRIES).expect("the default capacity is non-zero")
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
    use std::time::{Duration, UNIX_EPOCH};

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
}
