use super::*;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::RwLock;

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
            || self.revision > i64::MAX as u64
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
    Hub(Arc<dyn EntitlementCache>),
    Standalone(StandaloneEntitlements),
}
impl Entitlements {
    pub fn hub(product_id: String, cache: Arc<dyn EntitlementCache>) -> Self {
        Self {
            product_id,
            mode: EntitlementMode::Hub(cache),
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
            EntitlementMode::Hub(cache) => cache.get(subject, &self.product_id, now).await,
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
    ) -> Result<EntitlementSnapshot, PlatformError>;
}

/// Composes entitlement refresh with a product handler for all other events.
/// Pull failure leaves the receipt uncommitted, so the hub retries the event.
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
#[async_trait]
impl PlatformEventHandler for EntitlementEventHandler {
    async fn handle(
        &self,
        event: &PlatformEvent,
        context: &mut EventContext<'_>,
    ) -> Result<(), PlatformError> {
        if let PlatformEventKind::EntitlementsChanged(changed) = event.kind() {
            if changed.product != self.product_id {
                return Err(PlatformError::Invalid);
            }
            let snapshot = self
                .source
                .pull(&changed.suite_subject, &self.product_id)
                .await?;
            if snapshot.subject != changed.suite_subject
                || snapshot.product_id != self.product_id
                || snapshot.revision < changed.revision
            {
                return Err(PlatformError::Unavailable);
            }
            return self.cache.store_in_context(&snapshot, context).await;
        }
        self.next.handle(event, context).await
    }
}
