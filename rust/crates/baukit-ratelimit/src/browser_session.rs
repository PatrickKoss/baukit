use std::{fmt, sync::Arc, time::Duration};

use baukit_auth::browser_session::{
    BrowserSessionRateLimit, BrowserSessionRateLimiter, RateLimitFuture, RateLimitOutcome,
    RateLimitSubject,
};
use baukit_config::RateLimitFailMode;

use crate::{Quota, RateLimitOptions, RateLimitStore, resolve_client_ip};

const BUCKET_PERIOD: Duration = Duration::from_secs(60);

/// [`BrowserSessionRateLimiter`] over a [`RateLimitStore`].
///
/// Client IPs resolve through [`resolve_client_ip`] with the options'
/// trusted proxy hops. Keys use the options' prefix and the bucket group. A
/// store failure follows the options' fail mode.
#[derive(Clone)]
pub struct StoreBrowserSessionRateLimiter {
    store: Arc<dyn RateLimitStore>,
    options: RateLimitOptions,
}

impl StoreBrowserSessionRateLimiter {
    /// Builds the limiter from a store and the product's rate-limit options.
    pub fn new(store: impl RateLimitStore + 'static, options: RateLimitOptions) -> Self {
        Self {
            store: Arc::new(store),
            options,
        }
    }

    fn key(&self, group: &str, subject: RateLimitSubject<'_>) -> Option<String> {
        let prefix = self.options.key_prefix();
        match subject {
            RateLimitSubject::ClientIp { headers, peer } => {
                let address = resolve_client_ip(headers, peer, self.options.trusted_proxy_hops)
                    .map_or_else(|| "unknown".to_owned(), |address| address.to_string());
                Some(format!("{prefix}{group}:ip:{address}"))
            }
            RateLimitSubject::SessionId(id) => Some(format!("{prefix}{group}:session:{id}")),
            _ => None,
        }
    }

    fn on_failure(&self) -> RateLimitOutcome {
        match self.options.fail_mode {
            RateLimitFailMode::Open => RateLimitOutcome::Allowed,
            RateLimitFailMode::Closed => RateLimitOutcome::Limited {
                retry_after: BUCKET_PERIOD,
            },
        }
    }
}

impl fmt::Debug for StoreBrowserSessionRateLimiter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoreBrowserSessionRateLimiter")
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

impl BrowserSessionRateLimiter for StoreBrowserSessionRateLimiter {
    fn check<'a>(
        &'a self,
        limit: BrowserSessionRateLimit,
        subject: RateLimitSubject<'a>,
    ) -> RateLimitFuture<'a> {
        Box::pin(async move {
            let Some(key) = self.key(limit.group, subject) else {
                tracing::warn!(
                    group = limit.group,
                    "unsupported browser session rate-limit subject"
                );
                return self.on_failure();
            };
            let quota = match Quota::new(limit.requests_per_minute, BUCKET_PERIOD, limit.burst) {
                Ok(quota) => quota,
                Err(error) => {
                    tracing::warn!(group = limit.group, %error, "invalid browser session quota");
                    return self.on_failure();
                }
            };
            match self.store.check_and_consume(&key, quota).await {
                Ok(decision) if decision.allowed => RateLimitOutcome::Allowed,
                Ok(decision) => RateLimitOutcome::Limited {
                    retry_after: decision.retry_after,
                },
                Err(error) => {
                    tracing::warn!(group = limit.group, %error, "browser session rate-limit store failed");
                    self.on_failure()
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        net::{IpAddr, Ipv4Addr, SocketAddr},
        pin::Pin,
    };

    use axum::http::{HeaderMap, HeaderValue};
    use baukit_auth::browser_session::{AUTH_SESSION_ID_LIMIT, AUTH_SESSION_IP_LIMIT};
    use baukit_config::RateLimitConfig;

    use super::*;
    use crate::{InMemoryRateLimitStore, RateLimitDecision, RateLimitStoreError};

    type TestResult = Result<(), Box<dyn Error>>;

    const STORE_CAPACITY: usize = 64;
    const PEER: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 4000);

    fn limiter() -> Result<StoreBrowserSessionRateLimiter, Box<dyn Error>> {
        Ok(StoreBrowserSessionRateLimiter::new(
            InMemoryRateLimitStore::new(STORE_CAPACITY)?,
            RateLimitOptions::default(),
        ))
    }

    fn admitted(limit: BrowserSessionRateLimit) -> u64 {
        limit.requests_per_minute + limit.burst
    }

    #[tokio::test]
    async fn ip_bucket_admits_rate_plus_burst_per_forwarded_client() -> TestResult {
        let limiter = limiter()?;
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.7"));
        let client = RateLimitSubject::ClientIp {
            headers: &headers,
            peer: Some(PEER),
        };
        for _ in 0..admitted(AUTH_SESSION_IP_LIMIT) {
            assert_eq!(
                limiter.check(AUTH_SESSION_IP_LIMIT, client).await,
                RateLimitOutcome::Allowed
            );
        }
        assert!(matches!(
            limiter.check(AUTH_SESSION_IP_LIMIT, client).await,
            RateLimitOutcome::Limited { .. }
        ));
        let empty = HeaderMap::new();
        let peer_only = RateLimitSubject::ClientIp {
            headers: &empty,
            peer: Some(PEER),
        };
        assert_eq!(
            limiter.check(AUTH_SESSION_IP_LIMIT, peer_only).await,
            RateLimitOutcome::Allowed
        );
        Ok(())
    }

    #[tokio::test]
    async fn custom_limit_is_enforced_at_its_own_count() -> TestResult {
        let limiter = limiter()?;
        let custom = BrowserSessionRateLimit {
            requests_per_minute: 100,
            burst: 20,
            ..AUTH_SESSION_IP_LIMIT
        };
        let headers = HeaderMap::new();
        let client = RateLimitSubject::ClientIp {
            headers: &headers,
            peer: Some(PEER),
        };
        for _ in 0..admitted(custom) {
            assert_eq!(
                limiter.check(custom, client).await,
                RateLimitOutcome::Allowed
            );
        }
        assert!(matches!(
            limiter.check(custom, client).await,
            RateLimitOutcome::Limited { .. }
        ));
        Ok(())
    }

    #[tokio::test]
    async fn session_buckets_are_independent() -> TestResult {
        let limiter = limiter()?;
        for _ in 0..admitted(AUTH_SESSION_ID_LIMIT) {
            limiter
                .check(AUTH_SESSION_ID_LIMIT, RateLimitSubject::SessionId("a"))
                .await;
        }
        assert!(matches!(
            limiter
                .check(AUTH_SESSION_ID_LIMIT, RateLimitSubject::SessionId("a"))
                .await,
            RateLimitOutcome::Limited { .. }
        ));
        assert_eq!(
            limiter
                .check(AUTH_SESSION_ID_LIMIT, RateLimitSubject::SessionId("b"))
                .await,
            RateLimitOutcome::Allowed
        );
        Ok(())
    }

    struct FailingStore;

    impl RateLimitStore for FailingStore {
        fn check_and_consume<'a>(
            &'a self,
            _key: &'a str,
            _quota: Quota,
        ) -> Pin<Box<dyn Future<Output = Result<RateLimitDecision, RateLimitStoreError>> + Send + 'a>>
        {
            Box::pin(std::future::ready(Err(RateLimitStoreError::unavailable(
                "down",
            ))))
        }
    }

    #[tokio::test]
    async fn store_failure_follows_the_fail_mode() -> TestResult {
        for (mode, expected) in [
            (RateLimitFailMode::Open, RateLimitOutcome::Allowed),
            (
                RateLimitFailMode::Closed,
                RateLimitOutcome::Limited {
                    retry_after: BUCKET_PERIOD,
                },
            ),
        ] {
            let config = RateLimitConfig {
                fail_mode: mode,
                ..RateLimitConfig::default()
            };
            let limiter = StoreBrowserSessionRateLimiter::new(
                FailingStore,
                RateLimitOptions::from_config(&config)?,
            );
            assert_eq!(
                limiter
                    .check(AUTH_SESSION_ID_LIMIT, RateLimitSubject::SessionId("a"))
                    .await,
                expected
            );
        }
        Ok(())
    }
}
