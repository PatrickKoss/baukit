//! JWKS cache for [`OidcVerifier`](super::OidcVerifier).
//!
//! Known key ids are served from a fresh set without any lock held across a
//! fetch. A stale set keeps serving known key ids while one background task
//! refreshes it. Unknown key ids force a refresh, subject to the cooldown, and
//! never verify against a stale set.

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use aws_lc_rs::digest;
use baukit_egress::GuardedClient;
use baukit_resilience::{
    CacheRead, Clock, Freshness, StaleCache, StaleCacheError, StalePolicy, SystemClock,
};
use reqwest::Url;

use super::{Jwk, JwkSet, VerificationError};
use crate::oidc_http::{self, Document};

pub(super) const UNKNOWN_KEY_TTL: Duration = Duration::from_secs(30);
pub(super) const MAX_UNKNOWN_KEYS: usize = 128;
const FAILURE_COOLDOWN: Duration = Duration::from_secs(10);
const AUTH_KEY_CACHE_REQUESTS_TOTAL: &str = "auth_key_cache_requests_total";
const OIDC_SOURCE: &str = "oidc";

/// Label value of `auth_key_cache_requests_total{result}`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum KeyCacheResult {
    Fresh,
    Stale,
    Fetched,
    Failed,
}

impl KeyCacheResult {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Stale => "stale",
            Self::Fetched => "fetched",
            Self::Failed => "failed",
        }
    }

    fn of<T>(read: &CacheRead<T>) -> Self {
        if read.refresh_failed {
            return Self::Failed;
        }
        match read.freshness {
            Freshness::Fresh => Self::Fresh,
            Freshness::Stale => Self::Stale,
            Freshness::Fetched => Self::Fetched,
        }
    }
}

pub(super) fn record(result: KeyCacheResult) {
    metrics::counter!(
        AUTH_KEY_CACHE_REQUESTS_TOTAL,
        "source" => OIDC_SOURCE,
        "result" => result.as_str()
    )
    .increment(1);
}

/// Shared JWKS state. Cloning shares the cache.
#[derive(Clone)]
pub(super) struct KeyCache {
    source: Arc<KeySource>,
}

struct KeySource {
    client: GuardedClient,
    jwks_uri: Url,
    clock: Arc<dyn Clock>,
    cache: StaleCache<Arc<JwkSet>>,
    unknown_keys: Mutex<UnknownKeys>,
    refreshing: AtomicBool,
}

impl KeyCache {
    pub(super) fn new(
        client: GuardedClient,
        jwks_uri: Url,
        fresh_for: Duration,
        stale_for: Duration,
    ) -> Self {
        Self::with_clock(
            client,
            jwks_uri,
            fresh_for,
            stale_for,
            Arc::new(SystemClock),
        )
    }

    pub(super) fn with_clock(
        client: GuardedClient,
        jwks_uri: Url,
        fresh_for: Duration,
        stale_for: Duration,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let policy = StalePolicy::new(fresh_for, stale_for, FAILURE_COOLDOWN);
        Self {
            source: Arc::new(KeySource {
                client,
                jwks_uri,
                cache: StaleCache::with_clock(policy, Arc::clone(&clock)),
                clock,
                unknown_keys: Mutex::new(UnknownKeys::default()),
                refreshing: AtomicBool::new(false),
            }),
        }
    }

    pub(super) fn jwks_uri(&self) -> &Url {
        &self.source.jwks_uri
    }

    /// A cache with the same endpoint and clock but no cached keys.
    pub(super) fn emptied(&self, fresh_for: Duration, stale_for: Duration) -> Self {
        Self::with_clock(
            self.source.client.clone(),
            self.source.jwks_uri.clone(),
            fresh_for,
            stale_for,
            Arc::clone(&self.source.clock),
        )
    }

    pub(super) async fn key_for(&self, key_id: &str) -> Result<Jwk, VerificationError> {
        let read = match self.source.cache.current() {
            Some(read) => read,
            None => self.load(key_id).await?,
        };
        if let Some(key) = read.value.find(key_id).cloned() {
            record(KeyCacheResult::of(&read));
            self.source.forget_unknown(key_id);
            if read.freshness == Freshness::Stale {
                self.refresh_in_background();
            }
            return Ok(key);
        }
        self.unknown_key(key_id).await
    }

    #[cfg(test)]
    pub(super) async fn fetch(&self) -> Result<Arc<JwkSet>, VerificationError> {
        self.source.fetch().await
    }

    async fn load(&self, key_id: &str) -> Result<CacheRead<Arc<JwkSet>>, VerificationError> {
        let source = &self.source;
        let read = source
            .cache
            .get_or_refresh(|| source.fetch_noting_unknown(key_id))
            .await;
        observed(read)
    }

    async fn unknown_key(&self, key_id: &str) -> Result<Jwk, VerificationError> {
        let source = &self.source;
        if source.is_known_unknown(key_id) || source.refreshing.load(Ordering::Acquire) {
            return Err(VerificationError::UnknownKeyId);
        }
        let read = observed(
            source
                .cache
                .force_refresh(|| source.fetch_noting_unknown(key_id))
                .await,
        )?;
        let fetched = read.freshness == Freshness::Fetched && !read.refresh_failed;
        match read.value.find(key_id) {
            Some(key) if fetched => Ok(key.clone()),
            _ => Err(VerificationError::UnknownKeyId),
        }
    }

    /// Refreshes a stale set on the runtime so no request waits for the fetch.
    fn refresh_in_background(&self) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        if self.source.refreshing.swap(true, Ordering::AcqRel) {
            return;
        }
        let source = Arc::clone(&self.source);
        runtime.spawn(async move {
            let read = source.cache.get_or_refresh(|| source.fetch()).await;
            source.refreshing.store(false, Ordering::Release);
            match read {
                Ok(read) if read.freshness == Freshness::Fetched => {
                    record(KeyCacheResult::of(&read))
                }
                Ok(_) => {}
                Err(_) => record(KeyCacheResult::Failed),
            }
        });
    }

    #[cfg(test)]
    pub(super) fn is_refreshing(&self) -> bool {
        self.source.refreshing.load(Ordering::Acquire)
    }
}

impl KeySource {
    async fn fetch(&self) -> Result<Arc<JwkSet>, VerificationError> {
        let set: JwkSet =
            oidc_http::get_json(&self.client, self.jwks_uri.clone(), Document::Jwks).await?;
        Ok(Arc::new(set))
    }

    /// Fetches and, before other callers see the new set, remembers `key_id`
    /// when the set lacks it, so concurrent callers do not refetch for it.
    async fn fetch_noting_unknown(&self, key_id: &str) -> Result<Arc<JwkSet>, VerificationError> {
        let set = self.fetch().await?;
        if set.find(key_id).is_none() {
            self.lock_unknown().remember(key_id, self.clock.now());
        }
        Ok(set)
    }

    fn is_known_unknown(&self, key_id: &str) -> bool {
        self.lock_unknown().contains(key_id, self.clock.now())
    }

    fn forget_unknown(&self, key_id: &str) {
        self.lock_unknown().forget(key_id);
    }

    fn lock_unknown(&self) -> std::sync::MutexGuard<'_, UnknownKeys> {
        self.unknown_keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

fn observed(
    read: Result<CacheRead<Arc<JwkSet>>, StaleCacheError<VerificationError>>,
) -> Result<CacheRead<Arc<JwkSet>>, VerificationError> {
    match read {
        Ok(read) => Ok(read),
        Err(error) => {
            record(KeyCacheResult::Failed);
            Err(match error {
                StaleCacheError::Fetch(error) => error,
                StaleCacheError::CoolingDown { .. } => VerificationError::JwksUnavailable,
            })
        }
    }
}

/// Key ids recently missing from a fresh JWKS, stored as SHA-256 digests.
#[derive(Default)]
pub(super) struct UnknownKeys {
    entries: BTreeMap<[u8; 32], Instant>,
}

impl UnknownKeys {
    pub(super) fn remember(&mut self, key_id: &str, now: Instant) {
        self.entries
            .retain(|_, cached_at| now.saturating_duration_since(*cached_at) < UNKNOWN_KEY_TTL);
        if self.entries.len() >= MAX_UNKNOWN_KEYS
            && let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, cached_at)| **cached_at)
                .map(|(hash, _)| *hash)
        {
            self.entries.remove(&oldest);
        }
        self.entries.insert(unknown_key_hash(key_id), now);
    }

    fn contains(&self, key_id: &str, now: Instant) -> bool {
        self.entries
            .get(&unknown_key_hash(key_id))
            .is_some_and(|cached_at| now.saturating_duration_since(*cached_at) < UNKNOWN_KEY_TTL)
    }

    fn forget(&mut self, key_id: &str) {
        self.entries.remove(&unknown_key_hash(key_id));
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub(super) fn has(&self, key_id: &str) -> bool {
        self.entries.contains_key(&unknown_key_hash(key_id))
    }
}

fn unknown_key_hash(key_id: &str) -> [u8; 32] {
    let hash = digest::digest(&digest::SHA256, key_id.as_bytes());
    let mut bytes = [0; 32];
    bytes.copy_from_slice(hash.as_ref());
    bytes
}
