use super::*;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, Weak},
};
use tokio::sync::{Mutex, RwLock};

pub const FREE_CACHE_TTL_SECONDS: i64 = 60;

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
/// The same read API serves hub and standalone products. Tier meanings remain
/// product policy; this crate only compares tier and feature keys exactly.
#[derive(Clone)]
pub struct Entitlements {
    product_id: String,
    mode: EntitlementMode,
}
#[derive(Clone)]
enum EntitlementMode {
    Hub {
        cache: Arc<dyn EntitlementCache>,
        source: Arc<dyn EntitlementSource>,
        flights: Arc<Mutex<BTreeMap<String, Weak<Mutex<()>>>>>,
    },
    Standalone(StandaloneEntitlements),
}
impl Entitlements {
    pub fn hub(
        product_id: String,
        cache: Arc<dyn EntitlementCache>,
        source: Arc<dyn EntitlementSource>,
    ) -> Self {
        Self {
            product_id,
            mode: EntitlementMode::Hub {
                cache,
                source,
                flights: Arc::default(),
            },
        }
    }
    pub fn standalone(product_id: String, default: StandaloneEntitlements) -> Self {
        Self {
            product_id,
            mode: EntitlementMode::Standalone(default),
        }
    }
    pub async fn current(
        &self,
        subject: &str,
        now: i64,
    ) -> Result<Option<EntitlementSnapshot>, PlatformError> {
        match &self.mode {
            EntitlementMode::Hub {
                cache,
                source,
                flights,
            } => {
                if let Some(snapshot) = cache.get(subject, &self.product_id, now).await? {
                    return Ok(Some(snapshot));
                }
                let flight = {
                    let mut flights = flights.lock().await;
                    flights.retain(|_, lock| lock.strong_count() > 0);
                    match flights.get(subject).and_then(Weak::upgrade) {
                        Some(lock) => lock,
                        None => {
                            let lock = Arc::new(Mutex::new(()));
                            flights.insert(subject.into(), Arc::downgrade(&lock));
                            lock
                        }
                    }
                };
                let _guard = flight.lock().await;
                if let Some(snapshot) = cache.get(subject, &self.product_id, now).await? {
                    return Ok(Some(snapshot));
                }
                let revision = cache.revision(subject, &self.product_id).await?;
                let snapshot =
                    pull_snapshot(source.as_ref(), subject, &self.product_id, revision, now)
                        .await?;
                if snapshot.valid_until <= now {
                    return Err(PlatformError::Unavailable);
                }
                cache.store(&snapshot).await?;
                // A concurrent event may have stored a newer revision during the pull.
                cache
                    .get(subject, &self.product_id, now)
                    .await?
                    .map(Some)
                    .ok_or(PlatformError::Unavailable)
            }
            EntitlementMode::Standalone(default) => Ok(Some(EntitlementSnapshot {
                subject: subject.into(),
                product_id: self.product_id.clone(),
                tier: default.tier.clone(),
                features: default.features.clone(),
                revision: 0,
                valid_until: i64::MAX,
                grace_until: None,
            })),
        }
    }
    pub async fn has_entitlement(
        &self,
        subject: &str,
        feature: &str,
        now: i64,
    ) -> Result<bool, PlatformError> {
        Ok(self
            .current(subject, now)
            .await?
            .is_some_and(|s| s.features.iter().any(|f| f == feature)))
    }
    pub async fn has_tier(
        &self,
        subject: &str,
        tier: &str,
        now: i64,
    ) -> Result<bool, PlatformError> {
        Ok(self
            .current(subject, now)
            .await?
            .is_some_and(|s| s.tier == tier))
    }
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
        Err(error) => return Err(error.into()),
    };
    snapshot
        .validate()
        .map_err(|_| PlatformError::Unavailable)?;
    if snapshot.subject != subject
        || snapshot.product_id != product_id
        || snapshot.revision < revision
    {
        return Err(PlatformError::Unavailable);
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
        now: chrono::DateTime<chrono::Utc>,
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
                now.timestamp(),
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
