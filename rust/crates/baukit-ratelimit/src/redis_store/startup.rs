//! Startup connects with backoff and the degraded fallback.

use std::{
    sync::{Arc, Weak},
    time::Duration,
};

use baukit_resilience::{
    Backoff, JitterSource, SplitMixJitter,
    startup::{RetryAttempt, StartupError, retry_within},
};
use redis::{ErrorKind, RedisError, ServerErrorKind};
use tokio::sync::OnceCell;

use super::{RedisRateLimitStore, StoreConnection, open_connection};
use crate::{RateLimitOptions, RateLimitStoreError};

const DEPENDENCY: &str = "redis";
const RECONNECT_INITIAL: Duration = Duration::from_millis(500);
const RECONNECT_MAX: Duration = Duration::from_secs(30);

impl RedisRateLimitStore {
    /// Connects like [`Self::connect`], retrying transient failures within `budget`.
    ///
    /// Retries I/O errors (refused, DNS, timeouts, dropped connections),
    /// `LOADING`, `TRYAGAIN`, `MASTERDOWN`, `CLUSTERDOWN`, and Sentinel answers
    /// without a usable master. Authentication and URL errors fail at once.
    ///
    /// # Errors
    ///
    /// [`StartupError::Permanent`] for a non-transient error and
    /// [`StartupError::BudgetExhausted`] when Redis stays unreachable.
    pub async fn connect_with_backoff(
        redis_url: &str,
        budget: Duration,
    ) -> Result<Self, StartupError<RateLimitStoreError>> {
        retry_within(DEPENDENCY, budget, is_transient, log_retry, || {
            open_connection(redis_url)
        })
        .await
        .map(Self::from_connection)
    }

    /// Like [`Self::connect_with_backoff`], but returns a degraded store when
    /// the budget runs out.
    ///
    /// The degraded store fails every decision with [`RateLimitStoreError`],
    /// so the rate-limit layer applies its fail mode, and a background task
    /// keeps connecting with backoff (500 ms up to 30 s) until Redis answers.
    /// The task stops when the last clone of the store drops. Call it inside a
    /// Tokio runtime.
    ///
    /// # Errors
    ///
    /// [`StartupError::Permanent`] for a non-transient error.
    pub async fn connect_or_degraded(
        redis_url: &str,
        budget: Duration,
    ) -> Result<Self, StartupError<RateLimitStoreError>> {
        match Self::connect_with_backoff(redis_url, budget).await {
            Err(error) if error.is_budget_exhausted() => {
                tracing::error!(
                    dependency = DEPENDENCY,
                    error = %error,
                    "starting with a degraded rate-limit store"
                );
                Ok(Self::degraded(redis_url.to_owned()))
            }
            result => result,
        }
    }

    /// Connects with [`Self::connect_or_degraded`] only when at least one
    /// request-limit scope is enabled.
    ///
    /// Fully disabled request limiting returns `None` without parsing the Redis
    /// URL or opening a connection. The store uses
    /// [`RateLimitOptions::redis_key_namespace`] as its key namespace.
    ///
    /// # Errors
    ///
    /// The errors of [`Self::connect_or_degraded`], and an invalid namespace
    /// as [`StartupError::Permanent`].
    pub async fn connect_if_enabled(
        options: &RateLimitOptions,
        startup_budget: Duration,
    ) -> Result<Option<Self>, StartupError<RateLimitStoreError>> {
        if !options.is_enabled() {
            return Ok(None);
        }
        Self::connect_or_degraded(options.redis_url(), startup_budget)
            .await?
            .with_key_namespace(options.redis_key_namespace())
            .map(Some)
            .map_err(StartupError::Permanent)
    }

    fn degraded(redis_url: String) -> Self {
        let slot = Arc::new(OnceCell::new());
        tokio::spawn(reconnect(Arc::downgrade(&slot), redis_url));
        Self {
            connection: slot,
            namespace: None,
        }
    }
}

async fn reconnect(slot: Weak<OnceCell<StoreConnection>>, redis_url: String) {
    let backoff = Backoff::new(RECONNECT_INITIAL, RECONNECT_MAX);
    let mut jitter = SplitMixJitter::from_entropy();
    let mut attempt = 0_u32;
    loop {
        let delay = backoff
            .next_delay(attempt, &mut jitter as &mut dyn JitterSource)
            .max(RECONNECT_INITIAL);
        tokio::time::sleep(delay).await;
        if slot.strong_count() == 0 {
            return;
        }
        attempt = attempt.saturating_add(1);
        match open_connection(&redis_url).await {
            Ok(connection) => {
                if let Some(slot) = slot.upgrade() {
                    let _ = slot.set(connection);
                    tracing::info!(
                        dependency = DEPENDENCY,
                        attempt,
                        "rate-limit store connected"
                    );
                }
                return;
            }
            Err(error) => tracing::warn!(
                dependency = DEPENDENCY,
                attempt,
                error = %error,
                "rate-limit store still unavailable"
            ),
        }
    }
}

fn log_retry(retry: &RetryAttempt<'_, RateLimitStoreError>) {
    tracing::warn!(
        dependency = retry.dependency,
        attempt = retry.attempt,
        delay_ms = u64::try_from(retry.delay.as_millis()).unwrap_or(u64::MAX),
        error = %retry.error,
        "dependency unavailable at startup, retrying"
    );
}

/// Returns `true` when a Redis connect error may clear up by waiting.
pub(crate) fn is_transient(error: &RateLimitStoreError) -> bool {
    error
        .source
        .downcast_ref::<RedisError>()
        .is_some_and(is_transient_redis)
}

fn is_transient_redis(error: &RedisError) -> bool {
    matches!(
        error.kind(),
        ErrorKind::Io
            | ErrorKind::MasterNameNotFoundBySentinel
            | ErrorKind::NoValidReplicasFoundBySentinel
            | ErrorKind::ClusterConnectionNotFound
            | ErrorKind::Server(
                ServerErrorKind::BusyLoading
                    | ServerErrorKind::TryAgain
                    | ServerErrorKind::MasterDown
                    | ServerErrorKind::ClusterDown
            )
    )
}

#[cfg(test)]
mod tests;
