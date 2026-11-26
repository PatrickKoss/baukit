use super::*;
use async_trait::async_trait;
use baukit_resilience::{Clock, SystemClock};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, PoisonError, Weak},
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, RwLock};

pub const FREE_CACHE_TTL_SECONDS: i64 = 60;
/// After a failed hub pull, reads skip the hub for this long and serve a
/// snapshot in its grace window, or fail with this `Retry-After`.
pub const ENTITLEMENT_FAILURE_COOLDOWN: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntitlementSnapshot {
    pub subject: String,
    pub product_id: String,
    pub tier: String,
    pub features: Vec<String>,
    pub revision: u64,
    /// Exclusive Unix-second deadline. Pull adapters choose their refresh interval.
    pub valid_until: i64,
    pub grace_until: Option<i64>,
}
impl From<TicketClaims> for EntitlementSnapshot {
    fn from(claims: TicketClaims) -> Self {
        Self {
            subject: claims.sub,
            product_id: claims.aud,
            tier: claims.tier,
            features: claims.features,
            revision: claims.rev,
            valid_until: claims.exp,
            grace_until: claims.grace,
        }
    }
}
impl EntitlementSnapshot {
    pub(crate) fn validate(&self) -> Result<(), PlatformError> {
        if self.subject.is_empty()
            || self.product_id.is_empty()
            || self.tier.is_empty()
            || self.revision > MAX_TICKET_REVISION
            || self.valid_until < 0
        {
            return Err(PlatformError::Invalid);
        }
        Ok(())
    }
}
#[async_trait]
pub trait EntitlementCache: Send + Sync {
    /// Keep revision cursors even when a snapshot expires. Reads never return
    /// an expired snapshot. Storage failures must remain errors.
    async fn get(
        &self,
        subject: &str,
        product_id: &str,
        now: i64,
    ) -> Result<Option<EntitlementSnapshot>, PlatformError>;
    /// The stored snapshot even after `valid_until`, so reads can serve it
    /// until `grace_until` while the source fails.
    async fn retained(
        &self,
        subject: &str,
        product_id: &str,
    ) -> Result<Option<EntitlementSnapshot>, PlatformError>;
    /// The retained revision cursor, including expired snapshots.
    async fn revision(&self, subject: &str, product_id: &str) -> Result<u64, PlatformError>;
    /// Remove every product snapshot and revision cursor for this subject.
    /// Late events can recreate rows. Products must filter erased subjects
    /// before entitlement preparation and before read-through calls.
    async fn forget(&self, subject: &str) -> Result<(), PlatformError>;
    /// Ignore older revisions and shorter-lived repeats of the same revision.
    async fn store(&self, snapshot: &EntitlementSnapshot) -> Result<(), PlatformError>;
    /// A PostgreSQL adapter uses the receipt transaction when available. Other
    /// adapters must make repeated writes safe after receipt failure.
    async fn store_in_context(
        &self,
        snapshot: &EntitlementSnapshot,
        _context: &mut EventContext<'_>,
    ) -> Result<(), PlatformError> {
        self.store(snapshot).await
    }
}

#[derive(Default)]
pub struct InMemoryEntitlementCache {
    entries: RwLock<BTreeMap<(String, String), EntitlementSnapshot>>,
}
#[async_trait]
impl EntitlementCache for InMemoryEntitlementCache {
    async fn get(
        &self,
        subject: &str,
        product_id: &str,
        now: i64,
    ) -> Result<Option<EntitlementSnapshot>, PlatformError> {
        Ok(self
            .entries
            .read()
            .await
            .get(&(subject.into(), product_id.into()))
            .filter(|s| now < s.valid_until)
            .cloned())
    }
    async fn retained(
        &self,
        subject: &str,
        product_id: &str,
    ) -> Result<Option<EntitlementSnapshot>, PlatformError> {
        Ok(self
            .entries
            .read()
            .await
            .get(&(subject.into(), product_id.into()))
            .cloned())
    }
    async fn revision(&self, subject: &str, product_id: &str) -> Result<u64, PlatformError> {
        Ok(self
            .entries
            .read()
            .await
            .get(&(subject.into(), product_id.into()))
            .map_or(0, |s| s.revision))
    }
    async fn forget(&self, subject: &str) -> Result<(), PlatformError> {
        self.entries
            .write()
            .await
            .retain(|(entry_subject, _), _| entry_subject != subject);
        Ok(())
    }
    async fn store(&self, snapshot: &EntitlementSnapshot) -> Result<(), PlatformError> {
        snapshot.validate()?;
        let mut entries = self.entries.write().await;
        let key = (snapshot.subject.clone(), snapshot.product_id.clone());
        if entries.get(&key).is_none_or(|old| {
            snapshot.revision > old.revision
                || snapshot.revision == old.revision && snapshot.valid_until > old.valid_until
        }) {
            entries.insert(key, snapshot.clone());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct StandaloneEntitlements {
    pub tier: String,
    pub features: Vec<String>,
}
impl Default for StandaloneEntitlements {
    fn default() -> Self {
        Self {
            tier: "free".into(),
            features: Vec::new(),
        }
    }
}
/// A snapshot and whether it is past `valid_until`, served within its grace
/// window because the hub failed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntitlementRead {
    pub snapshot: EntitlementSnapshot,
    pub stale: bool,
}

/// Why [`Entitlements`] returned no snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum EntitlementError {
    /// The hub failed and no snapshot is within its grace window. Answer
    /// `503` with this `Retry-After`; `From<EntitlementError> for ApiError` does.
    #[error("entitlements_unavailable")]
    Unavailable { retry_after: Duration },
    /// The entitlement cache failed.
    #[error(transparent)]
    Platform(#[from] PlatformError),
}
impl From<EntitlementError> for baukit_http::ApiError {
    fn from(error: EntitlementError) -> Self {
        match error {
            EntitlementError::Unavailable { retry_after } => Self::unavailable(retry_after),
            EntitlementError::Platform(PlatformError::Unavailable) => {
                Self::unavailable(ENTITLEMENT_FAILURE_COOLDOWN)
            }
            EntitlementError::Platform(error) => Self::internal(error),
        }
    }
}

/// The same read API serves hub and standalone products. Tier meanings remain
/// product policy; this crate only compares tier and feature keys exactly.
#[derive(Clone)]
pub struct Entitlements {
    product_id: String,
    mode: EntitlementMode,
}
#[derive(Clone)]
enum EntitlementMode {
    Hub(Arc<HubEntitlements>),
    Standalone(StandaloneEntitlements),
}
struct HubEntitlements {
    cache: Arc<dyn EntitlementCache>,
    source: Arc<dyn EntitlementSource>,
    flights: Mutex<BTreeMap<String, Weak<Mutex<()>>>>,
    clock: Arc<dyn Clock>,
    cooldown: std::sync::Mutex<Option<Instant>>,
}
impl Entitlements {
    pub fn hub(
        product_id: String,
        cache: Arc<dyn EntitlementCache>,
        source: Arc<dyn EntitlementSource>,
    ) -> Self {
        Self::hub_with_clock(product_id, cache, source, Arc::new(SystemClock))
    }
    /// [`Self::hub`] with the clock that drives the failure cooldown.
    pub fn hub_with_clock(
        product_id: String,
        cache: Arc<dyn EntitlementCache>,
        source: Arc<dyn EntitlementSource>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            product_id,
            mode: EntitlementMode::Hub(Arc::new(HubEntitlements {
                cache,
                source,
                flights: Mutex::default(),
                clock,
                cooldown: std::sync::Mutex::new(None),
            })),
        }
    }
    pub fn standalone(product_id: String, default: StandaloneEntitlements) -> Self {
        Self {
            product_id,
            mode: EntitlementMode::Standalone(default),
        }
    }
    /// The current snapshot. While the hub fails, a snapshot past `valid_until`
    /// but before `grace_until` is returned with `stale: true`.
    ///
    /// # Errors
    ///
    /// `EntitlementError::Unavailable` in hub mode when the pull fails or is cooling down after a
    /// failure, and no snapshot is within its grace window. A pull fails when the source fails
    /// with anything but `NoEntitlements`, or returns a snapshot that is invalid, already expired,
    /// for another subject or product, or older than the cached revision. `retry_after` is the
    /// source's own delay (an open breaker) or the remaining [`ENTITLEMENT_FAILURE_COOLDOWN`].
    /// `Platform` for cache read and write errors and when the snapshot is gone after the store.
    /// Standalone mode never fails.
    pub async fn read(&self, subject: &str, now: i64) -> Result<EntitlementRead, EntitlementError> {
        match &self.mode {
            EntitlementMode::Hub(hub) => hub.read(subject, &self.product_id, now).await,
            EntitlementMode::Standalone(default) => Ok(EntitlementRead {
                snapshot: EntitlementSnapshot {
                    subject: subject.into(),
                    product_id: self.product_id.clone(),
                    tier: default.tier.clone(),
                    features: default.features.clone(),
                    revision: 0,
                    valid_until: i64::MAX,
                    grace_until: None,
                },
                stale: false,
            }),
        }
    }
    /// [`Self::read`] without the staleness flag.
    ///
    /// # Errors
    ///
    /// The errors of [`Self::read`].
    pub async fn current(
        &self,
        subject: &str,
        now: i64,
    ) -> Result<Option<EntitlementSnapshot>, EntitlementError> {
        Ok(Some(self.read(subject, now).await?.snapshot))
    }
    /// # Errors
    ///
    /// The errors of [`Self::read`].
    pub async fn has_entitlement(
        &self,
        subject: &str,
        feature: &str,
        now: i64,
    ) -> Result<bool, EntitlementError> {
        Ok(self
            .current(subject, now)
            .await?
            .is_some_and(|s| s.features.iter().any(|f| f == feature)))
    }
    /// # Errors
    ///
    /// The errors of [`Self::read`].
    pub async fn has_tier(
        &self,
        subject: &str,
        tier: &str,
        now: i64,
    ) -> Result<bool, EntitlementError> {
        Ok(self
            .current(subject, now)
            .await?
            .is_some_and(|s| s.tier == tier))
    }
}
impl HubEntitlements {
    async fn read(
        &self,
        subject: &str,
        product_id: &str,
        now: i64,
    ) -> Result<EntitlementRead, EntitlementError> {
        if let Some(read) = self.fresh(subject, product_id, now).await? {
            return Ok(read);
        }
        if let Some(retry_after) = self.cooling_down() {
            return self.grace(subject, product_id, now, retry_after).await;
        }
        let flight = self.flight(subject).await;
        let _guard = flight.lock().await;
        if let Some(read) = self.fresh(subject, product_id, now).await? {
            return Ok(read);
        }
        if let Some(retry_after) = self.cooling_down() {
            return self.grace(subject, product_id, now, retry_after).await;
        }
        let revision = self.cache.revision(subject, product_id).await?;
        let snapshot = match pull_checked(self.source.as_ref(), subject, product_id, revision, now)
            .await
            .and_then(|snapshot| unexpired(snapshot, now))
        {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let retry_after = self.failed(error);
                return self.grace(subject, product_id, now, retry_after).await;
            }
        };
        self.cache.store(&snapshot).await?;
        // A concurrent event may have stored a newer revision during the pull.
        self.fresh(subject, product_id, now)
            .await?
            .ok_or(EntitlementError::Platform(PlatformError::Unavailable))
    }
    async fn fresh(
        &self,
        subject: &str,
        product_id: &str,
        now: i64,
    ) -> Result<Option<EntitlementRead>, PlatformError> {
        Ok(self
            .cache
            .get(subject, product_id, now)
            .await?
            .map(|snapshot| EntitlementRead {
                snapshot,
                stale: false,
            }))
    }
    async fn grace(
        &self,
        subject: &str,
        product_id: &str,
        now: i64,
        retry_after: Duration,
    ) -> Result<EntitlementRead, EntitlementError> {
        self.cache
            .retained(subject, product_id)
            .await?
            .filter(|snapshot| snapshot.grace_until.is_some_and(|grace| now < grace))
            .map(|snapshot| EntitlementRead {
                snapshot,
                stale: true,
            })
            .ok_or(EntitlementError::Unavailable { retry_after })
    }
    async fn flight(&self, subject: &str) -> Arc<Mutex<()>> {
        let mut flights = self.flights.lock().await;
        flights.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = flights.get(subject).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(Mutex::new(()));
        flights.insert(subject.into(), Arc::downgrade(&lock));
        lock
    }
    fn cooling_down(&self) -> Option<Duration> {
        let until = (*self.lock_cooldown())?;
        until
            .checked_duration_since(self.clock.now())
            .filter(|remaining| !remaining.is_zero())
    }
    /// Starts the cooldown when the hub itself failed and returns the delay
    /// callers should wait.
    fn failed(&self, error: EntitlementSourceError) -> Duration {
        let retry_after = match error {
            EntitlementSourceError::CoolingDown { retry_after } => retry_after,
            _ => ENTITLEMENT_FAILURE_COOLDOWN,
        };
        if matches!(
            error,
            EntitlementSourceError::Unavailable | EntitlementSourceError::CoolingDown { .. }
        ) {
            *self.lock_cooldown() = self.clock.now().checked_add(retry_after);
        }
        retry_after
    }
    fn lock_cooldown(&self) -> std::sync::MutexGuard<'_, Option<Instant>> {
        self.cooldown.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
fn unexpired(
    snapshot: EntitlementSnapshot,
    now: i64,
) -> Result<EntitlementSnapshot, EntitlementSourceError> {
    if snapshot.valid_until <= now {
        return Err(EntitlementSourceError::Invalid);
    }
    Ok(snapshot)
}
#[async_trait]
pub trait EntitlementSource: Send + Sync {
    /// Pull current state with the product's service credentials, or obtain and
    /// verify a ticket. Bind both subject and product before returning it.
    async fn pull(
        &self,
        subject: &str,
        product_id: &str,
    ) -> Result<EntitlementSnapshot, EntitlementSourceError>;
}

/// Only these two hub responses mean no entitlements. Other HTTP failures
/// remain errors, including product availability and service authorization errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum EntitlementSourceError {
    #[error("no_entitlements")]
    NoEntitlements,
    #[error("entitlement_source_unavailable")]
    Unavailable,
    /// The source refused to call the hub, for example while its breaker is
    /// open. Retry after `retry_after`.
    #[error("entitlement_source_unavailable")]
    CoolingDown { retry_after: std::time::Duration },
    #[error("entitlement_source_invalid")]
    Invalid,
}
impl EntitlementSourceError {
    pub fn from_hub_response(status: u16, code: &str) -> Self {
        match (status, code) {
            (404, "platform_subject_not_found") | (403, "account_inactive") => Self::NoEntitlements,
            _ => Self::Unavailable,
        }
    }
}
impl From<EntitlementSourceError> for PlatformError {
    fn from(_: EntitlementSourceError) -> Self {
        // A bad pull must not permanently reject an otherwise valid notification.
        Self::Unavailable
    }
}
async fn pull_snapshot(
    source: &dyn EntitlementSource,
    subject: &str,
    product_id: &str,
    revision: u64,
    now: i64,
) -> Result<EntitlementSnapshot, PlatformError> {
    Ok(pull_checked(source, subject, product_id, revision, now).await?)
}
async fn pull_checked(
    source: &dyn EntitlementSource,
    subject: &str,
    product_id: &str,
    revision: u64,
    now: i64,
) -> Result<EntitlementSnapshot, EntitlementSourceError> {
    let snapshot = match source.pull(subject, product_id).await {
        Ok(snapshot) => snapshot,
        Err(EntitlementSourceError::NoEntitlements) => EntitlementSnapshot {
            subject: subject.into(),
            product_id: product_id.into(),
            tier: "free".into(),
            features: Vec::new(),
            revision,
            valid_until: now.saturating_add(FREE_CACHE_TTL_SECONDS),
            grace_until: None,
        },
        Err(error) => return Err(error),
    };
    snapshot
        .validate()
        .map_err(|_| EntitlementSourceError::Invalid)?;
    if snapshot.subject != subject
        || snapshot.product_id != product_id
        || snapshot.revision < revision
    {
        return Err(EntitlementSourceError::Invalid);
    }
    Ok(snapshot)
}

/// Pulls before the receipt transaction; applies the revision-checked snapshot
/// inside it. Pull failure leaves the receipt uncommitted for a hub retry.
pub struct EntitlementEventHandler {
    product_id: String,
    cache: Arc<dyn EntitlementCache>,
    source: Arc<dyn EntitlementSource>,
    next: Arc<dyn PlatformEventHandler>,
}
impl EntitlementEventHandler {
    pub fn new(
        product_id: String,
        cache: Arc<dyn EntitlementCache>,
        source: Arc<dyn EntitlementSource>,
        next: Arc<dyn PlatformEventHandler>,
    ) -> Self {
        Self {
            product_id,
            cache,
            source,
            next,
        }
    }
}
struct PreparedEntitlements {
    cache: Arc<dyn EntitlementCache>,
    snapshot: EntitlementSnapshot,
}
#[async_trait]
impl PlatformEventHandler for PreparedEntitlements {
    async fn handle(
        &self,
        _: &PlatformEvent,
        context: &mut EventContext<'_>,
    ) -> Result<(), PlatformError> {
        self.cache.store_in_context(&self.snapshot, context).await
    }
}
#[async_trait]
impl PlatformEventHandler for EntitlementEventHandler {
    async fn prepare(
        &self,
        event: &PlatformEvent,
    ) -> Result<Option<Arc<dyn PlatformEventHandler>>, PlatformError> {
        self.prepare_at(event, SystemPlatformClock.now()).await
    }
    async fn prepare_at(
        &self,
        event: &PlatformEvent,
        now: jiff::Timestamp,
    ) -> Result<Option<Arc<dyn PlatformEventHandler>>, PlatformError> {
        if let PlatformEventKind::EntitlementsChanged(changed) = event.kind() {
            if changed.product != self.product_id {
                return Err(PlatformError::Invalid);
            }
            let revision = self
                .cache
                .revision(&changed.suite_subject, &self.product_id)
                .await?
                .max(changed.revision);
            let snapshot = pull_snapshot(
                self.source.as_ref(),
                &changed.suite_subject,
                &self.product_id,
                revision,
                now.as_second(),
            )
            .await?;
            return Ok(Some(Arc::new(PreparedEntitlements {
                cache: self.cache.clone(),
                snapshot,
            })));
        }
        self.next.prepare_at(event, now).await
    }
    async fn handle(
        &self,
        event: &PlatformEvent,
        context: &mut EventContext<'_>,
    ) -> Result<(), PlatformError> {
        if matches!(event.kind(), PlatformEventKind::EntitlementsChanged(_)) {
            // A custom inbox that skipped preparation must leave the event retryable.
            return Err(PlatformError::Unavailable);
        }
        self.next.handle(event, context).await
    }
}
