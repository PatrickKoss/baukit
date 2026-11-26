//! A signing-key cache that keeps verifying known key ids while the key source is down.
//!
//! [`KeySetCache`] wraps a [`KeySource`] in a [`StaleCache`]:
//!
//! - Within `fresh_for` a known key id is served without any lock held across a fetch.
//! - Within `stale_for` after that, a known key id is still served while one
//!   background task refreshes the set, so no lookup waits for the fetch.
//! - An unknown key id forces a refresh, at most once per `failure_cooldown`,
//!   and never verifies against a stale set.
//! - Every lookup counts once in `auth_key_cache_requests_total{source, result}`.

use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use aws_lc_rs::digest;
use baukit_resilience::{
    CacheRead, Clock, Freshness, StaleCache, StaleCacheError, StalePolicy, SystemClock,
};

const UNKNOWN_KEY_TTL: Duration = Duration::from_secs(30);
const MAX_UNKNOWN_KEYS: usize = 128;
const AUTH_KEY_CACHE_REQUESTS_TOTAL: &str = "auth_key_cache_requests_total";

/// A fetched key set that can look up a key by its id.
pub trait KeySet: Send + Sync + 'static {
    /// The key type handed to the verifier.
    type Key: Clone;

    /// Returns the key with id `key_id`.
    fn key(&self, key_id: &str) -> Option<&Self::Key>;
}

impl<K: Clone + Send + Sync + 'static> KeySet for BTreeMap<String, K> {
    type Key = K;

    fn key(&self, key_id: &str) -> Option<&K> {
        self.get(key_id)
    }
}

/// Boxed future returned by [`KeySource::fetch`].
pub type KeySourceFuture<'a, S, E> = Pin<Box<dyn Future<Output = Result<S, E>> + Send + 'a>>;

/// Port that fetches the current key set, for example over HTTP.
pub trait KeySource: Send + Sync + 'static {
    /// The fetched set.
    type Set: KeySet;
    /// Why a fetch failed.
    type Error: Send + 'static;

    /// Fetches the current key set.
    fn fetch(&self) -> KeySourceFuture<'_, Self::Set, Self::Error>;
}

/// Why [`KeySetCache::key_for`] returned no key.
#[derive(Debug)]
pub enum KeyLookupError<E> {
    /// No current key set has the id. Also returned during a refresh or
    /// cooldown, which keeps unknown ids from hammering the source.
    UnknownKeyId,
    /// The fetch failed and no usable set is cached.
    Fetch(E),
    /// No usable set is cached and a recent failure blocks fetching.
    Unavailable {
        /// Time until the next fetch is allowed.
        retry_after: Duration,
    },
}

/// Label value of `auth_key_cache_requests_total{result}`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LookupResult {
    Fresh,
    Stale,
    Fetched,
    Failed,
}

impl LookupResult {
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

type Read<S> = CacheRead<Arc<<S as KeySource>::Set>>;

/// See the module docs. Cloning shares the cache.
pub struct KeySetCache<S: KeySource> {
    inner: Arc<Inner<S>>,
}

impl<S: KeySource> Clone for KeySetCache<S> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<S: KeySource> std::fmt::Debug for KeySetCache<S> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KeySetCache")
            .field("metric_source", &self.inner.metric_source)
            .finish_non_exhaustive()
    }
}

struct Inner<S: KeySource> {
    source: S,
    metric_source: &'static str,
    clock: Arc<dyn Clock>,
    cache: StaleCache<Arc<S::Set>>,
    unknown_keys: Mutex<UnknownKeys>,
    refreshing: AtomicBool,
}

impl<S: KeySource> KeySetCache<S> {
    /// An empty cache on the system clock.
    ///
    /// `metric_source` is the static `source` label, such as `oidc` or `ticket`.
    #[must_use]
    pub fn new(source: S, policy: StalePolicy, metric_source: &'static str) -> Self {
        Self::with_clock(source, policy, metric_source, Arc::new(SystemClock))
    }

    /// An empty cache on `clock`.
    #[must_use]
    pub fn with_clock(
        source: S,
        policy: StalePolicy,
        metric_source: &'static str,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                source,
                metric_source,
                cache: StaleCache::with_clock(policy, Arc::clone(&clock)),
                clock,
                unknown_keys: Mutex::new(UnknownKeys::default()),
                refreshing: AtomicBool::new(false),
            }),
        }
    }

    /// The key source.
    #[must_use]
    pub fn source(&self) -> &S {
        &self.inner.source
    }

    /// The clock this cache reads.
    #[must_use]
    pub fn clock(&self) -> Arc<dyn Clock> {
        Arc::clone(&self.inner.clock)
    }

    /// Whether a background refresh is running.
    #[must_use]
    pub fn is_refreshing(&self) -> bool {
        self.inner.refreshing.load(Ordering::Acquire)
    }

    /// Returns the key with id `key_id`.
    ///
    /// # Errors
    ///
    /// [`KeyLookupError::UnknownKeyId`] when no current set has the id,
    /// [`KeyLookupError::Fetch`] or [`KeyLookupError::Unavailable`] when no
    /// usable set is cached and none could be fetched.
    pub async fn key_for(
        &self,
        key_id: &str,
    ) -> Result<<S::Set as KeySet>::Key, KeyLookupError<S::Error>> {
        let read = match self.inner.cache.current() {
            Some(read) => read,
            None => self.load(key_id).await?,
        };
        if let Some(key) = read.value.key(key_id).cloned() {
            self.inner.record(LookupResult::of(&read));
            self.inner.lock_unknown().forget(key_id);
            if read.freshness == Freshness::Stale {
                self.refresh_in_background();
            }
            return Ok(key);
        }
        self.unknown_key(key_id).await
    }

    async fn load(&self, key_id: &str) -> Result<Read<S>, KeyLookupError<S::Error>> {
        let inner = &self.inner;
        let read = inner
            .cache
            .get_or_refresh(|| inner.fetch_noting_unknown(key_id))
            .await;
        inner.observed(read)
    }

    async fn unknown_key(
        &self,
        key_id: &str,
    ) -> Result<<S::Set as KeySet>::Key, KeyLookupError<S::Error>> {
        let inner = &self.inner;
        if inner.is_known_unknown(key_id) || self.is_refreshing() {
            return Err(KeyLookupError::UnknownKeyId);
        }
        let read = inner.observed(
            inner
                .cache
                .force_refresh(|| inner.fetch_noting_unknown(key_id))
                .await,
        )?;
        let fetched = read.freshness == Freshness::Fetched && !read.refresh_failed;
        match read.value.key(key_id) {
            Some(key) if fetched => Ok(key.clone()),
            _ => Err(KeyLookupError::UnknownKeyId),
        }
    }

    fn refresh_in_background(&self) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        if self.inner.refreshing.swap(true, Ordering::AcqRel) {
            return;
        }
        let inner = Arc::clone(&self.inner);
        runtime.spawn(async move {
            let read = inner.cache.get_or_refresh(|| inner.fetch()).await;
            inner.refreshing.store(false, Ordering::Release);
            match read {
                Ok(read) if read.freshness == Freshness::Fetched => {
                    inner.record(LookupResult::of(&read));
                }
                Ok(_) => {}
                Err(_) => inner.record(LookupResult::Failed),
            }
        });
    }
}

impl<S: KeySource> Inner<S> {
    async fn fetch(&self) -> Result<Arc<S::Set>, S::Error> {
        self.source.fetch().await.map(Arc::new)
    }

    /// Remembers `key_id` as unknown before the new set is published, so
    /// callers waiting on the same fetch do not refetch for it.
    async fn fetch_noting_unknown(&self, key_id: &str) -> Result<Arc<S::Set>, S::Error> {
        let set = self.fetch().await?;
        if set.key(key_id).is_none() {
            self.lock_unknown().remember(key_id, self.clock.now());
        }
        Ok(set)
    }

    fn observed(
        &self,
        read: Result<Read<S>, StaleCacheError<S::Error>>,
    ) -> Result<Read<S>, KeyLookupError<S::Error>> {
        read.map_err(|error| {
            self.record(LookupResult::Failed);
            match error {
                StaleCacheError::Fetch(error) => KeyLookupError::Fetch(error),
                StaleCacheError::CoolingDown { retry_after } => {
                    KeyLookupError::Unavailable { retry_after }
                }
            }
        })
    }

    fn is_known_unknown(&self, key_id: &str) -> bool {
        self.lock_unknown().contains(key_id, self.clock.now())
    }

    fn record(&self, result: LookupResult) {
        metrics::counter!(
            AUTH_KEY_CACHE_REQUESTS_TOTAL,
            "source" => self.metric_source,
            "result" => result.as_str()
        )
        .increment(1);
    }

    fn lock_unknown(&self) -> MutexGuard<'_, UnknownKeys> {
        self.unknown_keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Key ids recently missing from a fetched set, stored as SHA-256 digests.
#[derive(Default)]
struct UnknownKeys {
    entries: BTreeMap<[u8; 32], Instant>,
}

impl UnknownKeys {
    fn remember(&mut self, key_id: &str, now: Instant) {
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
}

fn unknown_key_hash(key_id: &str) -> [u8; 32] {
    let hash = digest::digest(&digest::SHA256, key_id.as_bytes());
    let mut bytes = [0; 32];
    bytes.copy_from_slice(hash.as_ref());
    bytes
}

#[cfg(test)]
#[path = "key_set_cache_tests.rs"]
mod tests;
