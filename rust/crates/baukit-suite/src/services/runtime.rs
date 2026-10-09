use std::{sync::Arc, time::Duration};

use crate::domain::*;
use crate::ports::*;
use async_trait::async_trait;
use baukit_credential_vault::CredentialCipher;
use baukit_events::IngestOutcome;
use baukit_ratelimit::{Quota, RateLimitStore};
use chrono::{DateTime, NaiveDate, Utc};
use thiserror::Error;
use uuid::Uuid;

use super::types::*;
use crate::secret_cipher::SecretCipher;

#[path = "delivery.rs"]
mod delivery;
#[path = "erasure.rs"]
mod erasure;
#[path = "ingest.rs"]
mod ingest;
#[path = "link.rs"]
mod link;
pub use delivery::SuiteDeliveryService;
pub use erasure::{SuiteErasureNotificationService, erase_with_suite};
pub use ingest::{IngestResult, SignedHeaders, SuiteIngestService};
pub use link::SuiteLinkService;

#[derive(Debug, Error)]
pub enum SuiteServiceError {
    #[error(transparent)]
    Protocol(#[from] LinkProtocolError),
    #[error(transparent)]
    Store(#[from] SuiteStoreError),
    #[error("suite signature is invalid")]
    SignatureInvalid,
    #[error("suite link has been revoked")]
    LinkRevoked,
    #[error("suite payload is invalid")]
    PayloadInvalid,
    #[error("replay date must be between {earliest} and {latest}")]
    ReplayWindow {
        earliest: NaiveDate,
        latest: NaiveDate,
    },
    #[error("suite event applier is unavailable")]
    Unavailable,
    #[error("suite link cannot deliver")]
    LinkInactive,
    #[error("suite event is invalid: {0}")]
    Inbound(#[from] InboundValidationError),
    #[error("suite peer is unreachable")]
    PeerUnreachable,
    #[error("suite request rate exceeded")]
    RateLimited(u64),
    #[error("suite credentials could not be processed")]
    Cipher,
    #[error("suite rate limiter is unavailable")]
    RateLimiter,
}
impl SuiteServiceError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Protocol(e) => e.code(),
            Self::SignatureInvalid => "suite_signature_invalid",
            Self::LinkRevoked => "suite_link_revoked",
            Self::ReplayWindow { .. } => "suite_replay_window",
            Self::PayloadInvalid | Self::Store(SuiteStoreError::PayloadInvalid(_)) => {
                "suite_payload_invalid"
            }
            Self::Inbound(e) => e.code(),
            Self::PeerUnreachable => "suite_peer_unreachable",
            Self::RateLimited(_) => "rate_limited",
            Self::Store(SuiteStoreError::NotFound) => "not_found",
            Self::Store(SuiteStoreError::CodeInvalid) => "suite_code_invalid",
            Self::Store(SuiteStoreError::EventIdConflict) => "suite_event_id_conflict",
            Self::Store(SuiteStoreError::ReplayTooSoon(_)) => "suite_replay_throttled",
            Self::Store(SuiteStoreError::QuotaExceeded { .. }) => "suite_quota_exceeded",
            Self::Store(SuiteStoreError::InvalidData(_)) => "internal_error",
            Self::Unavailable => "suite_unavailable",
            Self::LinkInactive | Self::Store(SuiteStoreError::LinkInactive) => {
                "suite_link_inactive"
            }
            Self::Store(SuiteStoreError::LinkRevoked) => "suite_link_revoked",
            _ => "internal_error",
        }
    }
}
#[derive(Clone)]
pub struct SuiteContext {
    pub registry: Arc<PeerRegistry>,
    pub store: Arc<dyn SuiteLinkStore>,
    pub outbox: Arc<dyn SuiteEventOutbox>,
    pub peer: Arc<dyn SuitePeerClient>,
    pub limiter: Arc<dyn RateLimitStore>,
    pub catalog: Arc<PayloadCatalog>,
    pub replay: Arc<dyn SuiteReplaySource>,
    pub identities: Arc<dyn SuiteIdentitySource>,
    pub metric_prefix: String,
    cipher: SecretCipher,
    pub initial_replay_days: u32,
    pub share_xp: bool,
}
impl SuiteContext {
    pub fn new(
        registry: Arc<PeerRegistry>,
        catalog: Arc<PayloadCatalog>,
        dependencies: SuiteDependencies,
        config: SuiteServiceConfig,
    ) -> Self {
        metrics::describe_counter!(
            format!("{}_suite_deliveries_total", config.metric_prefix),
            "Suite peer delivery attempts by outcome"
        );
        metrics::describe_counter!(
            format!("{}_suite_events_ingested_total", config.metric_prefix),
            "Suite event ingestion by outcome"
        );
        for outcome in [
            "delivered",
            "retry",
            "rejected",
            "unauthorized",
            "revoked",
            "disabled",
        ] {
            metrics::counter!(format!("{}_suite_deliveries_total", config.metric_prefix), "outcome" => outcome).increment(0);
        }
        for outcome in [
            "granted",
            "no_rule",
            "capped",
            "duplicate",
            "rejected",
            "unauthorized",
        ] {
            metrics::counter!(format!("{}_suite_events_ingested_total", config.metric_prefix), "outcome" => outcome).increment(0);
        }
        Self {
            registry,
            catalog,
            store: dependencies.store,
            outbox: dependencies.outbox,
            peer: dependencies.peer,
            limiter: dependencies.limiter,
            replay: dependencies.replay,
            identities: dependencies.identities,
            metric_prefix: config.metric_prefix,
            cipher: SecretCipher::new(config.cipher),
            initial_replay_days: config.initial_replay_days,
            share_xp: config.share_xp,
        }
    }
    async fn limit(&self, key: &str, count: u32) -> Result<(), SuiteServiceError> {
        let quota = Quota::new(
            u64::from(count),
            Duration::from_secs(SUITE_RATE_WINDOW_SECONDS),
            0,
        )
        .map_err(|_| SuiteServiceError::RateLimiter)?;
        let decision = self
            .limiter
            .check_and_consume(key, quota)
            .await
            .map_err(|_| SuiteServiceError::RateLimiter)?;
        if decision.allowed {
            Ok(())
        } else {
            Err(SuiteServiceError::RateLimited(
                decision
                    .retry_after
                    .as_secs()
                    .saturating_add(u64::from(decision.retry_after.subsec_nanos() > 0)),
            ))
        }
    }
    fn active_peer(&self, id: &str) -> Result<&ActivePeer, SuiteServiceError> {
        self.registry.active_peer(id).ok_or_else(|| {
            if self.registry.standalone() {
                LinkProtocolError::Disabled.into()
            } else {
                LinkProtocolError::PeerUnknown.into()
            }
        })
    }
}
pub struct SuiteDependencies {
    pub store: Arc<dyn SuiteLinkStore>,
    pub outbox: Arc<dyn SuiteEventOutbox>,
    pub peer: Arc<dyn SuitePeerClient>,
    pub limiter: Arc<dyn RateLimitStore>,
    pub replay: Arc<dyn SuiteReplaySource>,
    pub identities: Arc<dyn SuiteIdentitySource>,
}

pub struct SuiteServiceConfig {
    pub metric_prefix: String,
    pub cipher: Option<CredentialCipher>,
    pub initial_replay_days: u32,
    pub share_xp: bool,
}

#[async_trait]
pub trait SuiteApi: Send + Sync {
    async fn peers(&self, user: Uuid) -> Result<Vec<SuitePeerView>, SuiteServiceError>;
    async fn start(
        &self,
        user: SuiteUser,
        input: LinkStartRequest,
        now: DateTime<Utc>,
    ) -> Result<LinkStartResponse, SuiteServiceError>;
    async fn callback(
        &self,
        input: LinkCallbackQuery,
        now: DateTime<Utc>,
    ) -> Result<CallbackRedirect, SuiteServiceError>;
    async fn complete(
        &self,
        user: SuiteUser,
        id: Uuid,
        code: String,
        now: DateTime<Utc>,
    ) -> Result<SuiteLinkView, SuiteServiceError>;
    async fn preview(
        &self,
        user: SuiteUser,
        client: String,
        hint: Option<String>,
        hint_domain: Option<String>,
    ) -> Result<AuthorizationPreview, SuiteServiceError>;
    async fn authorize(
        &self,
        user: SuiteUser,
        input: AuthorizationRequest,
        now: DateTime<Utc>,
    ) -> Result<AuthorizationResponse, SuiteServiceError>;
    async fn deny(
        &self,
        input: AuthorizationDeny,
    ) -> Result<AuthorizationResponse, SuiteServiceError>;
    async fn exchange(
        &self,
        input: ExchangeRequest,
        now: DateTime<Utc>,
    ) -> Result<ExchangeResponse, SuiteServiceError>;
    async fn list(&self, user: Uuid) -> Result<Vec<SuiteLinkView>, SuiteServiceError>;
    async fn get(&self, user: Uuid, id: Uuid) -> Result<SuiteLinkView, SuiteServiceError>;
    async fn update(
        &self,
        user: Uuid,
        id: Uuid,
        xp: Option<bool>,
        mode: Option<RewardMode>,
        now: DateTime<Utc>,
    ) -> Result<SuiteLinkView, SuiteServiceError>;
    async fn disconnect(
        &self,
        user: Uuid,
        id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<(), SuiteServiceError>;
    async fn test(&self, user: Uuid, id: Uuid, now: DateTime<Utc>)
    -> Result<(), SuiteServiceError>;
    async fn reenable(
        &self,
        user: Uuid,
        id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<SuiteLinkView, SuiteServiceError>;
    async fn replay(
        &self,
        user: Uuid,
        id: Uuid,
        since: NaiveDate,
        now: DateTime<Utc>,
    ) -> Result<u64, SuiteServiceError>;
    async fn deliveries(
        &self,
        user: Uuid,
        id: Uuid,
    ) -> Result<Vec<SuiteDeliveryRecord>, SuiteServiceError>;
    async fn revoke_from_peer(
        &self,
        id: Uuid,
        headers: SignedHeaders,
        body: Vec<u8>,
        now: DateTime<Utc>,
    ) -> Result<(), SuiteServiceError>;
    async fn ingest(
        &self,
        id: Uuid,
        headers: SignedHeaders,
        body: Vec<u8>,
        now: DateTime<Utc>,
    ) -> Result<IngestResult, SuiteServiceError>;
}

pub struct SuiteModule {
    pub links: SuiteLinkService,
    pub delivery: SuiteDeliveryService,
    pub ingest: SuiteIngestService,
}
#[async_trait]
impl SuiteApi for SuiteModule {
    async fn peers(&self, user: Uuid) -> Result<Vec<SuitePeerView>, SuiteServiceError> {
        self.links.peers(user).await
    }
    async fn start(
        &self,
        u: SuiteUser,
        i: LinkStartRequest,
        n: DateTime<Utc>,
    ) -> Result<LinkStartResponse, SuiteServiceError> {
        self.links.start(u, i, n).await
    }
    async fn callback(
        &self,
        i: LinkCallbackQuery,
        n: DateTime<Utc>,
    ) -> Result<CallbackRedirect, SuiteServiceError> {
        self.links.callback(i, n).await
    }
    async fn complete(
        &self,
        u: SuiteUser,
        id: Uuid,
        c: String,
        n: DateTime<Utc>,
    ) -> Result<SuiteLinkView, SuiteServiceError> {
        self.links
            .complete(u, id, c, n)
            .await
            .map(|link| SuiteLinkView::new(link, n))
    }
    async fn preview(
        &self,
        u: SuiteUser,
        c: String,
        h: Option<String>,
        d: Option<String>,
    ) -> Result<AuthorizationPreview, SuiteServiceError> {
        self.links.preview(u, c, h, d).await
    }
    async fn authorize(
        &self,
        u: SuiteUser,
        i: AuthorizationRequest,
        n: DateTime<Utc>,
    ) -> Result<AuthorizationResponse, SuiteServiceError> {
        self.links.authorize(u, i, n).await
    }
    async fn deny(&self, i: AuthorizationDeny) -> Result<AuthorizationResponse, SuiteServiceError> {
        self.links.deny(i)
    }
    async fn exchange(
        &self,
        i: ExchangeRequest,
        n: DateTime<Utc>,
    ) -> Result<ExchangeResponse, SuiteServiceError> {
        self.links.exchange(i, n).await
    }
    async fn list(&self, u: Uuid) -> Result<Vec<SuiteLinkView>, SuiteServiceError> {
        let now = Utc::now();
        self.links.list(u).await.map(|links| {
            links
                .into_iter()
                .map(|link| SuiteLinkView::new(link, now))
                .collect()
        })
    }
    async fn get(&self, u: Uuid, id: Uuid) -> Result<SuiteLinkView, SuiteServiceError> {
        self.links
            .get(u, id)
            .await
            .map(|link| SuiteLinkView::new(link, Utc::now()))
    }
    async fn update(
        &self,
        u: Uuid,
        id: Uuid,
        xp: Option<bool>,
        m: Option<RewardMode>,
        n: DateTime<Utc>,
    ) -> Result<SuiteLinkView, SuiteServiceError> {
        self.links
            .update(u, id, xp, m, n)
            .await
            .map(|link| SuiteLinkView::new(link, n))
    }
    async fn disconnect(
        &self,
        u: Uuid,
        id: Uuid,
        n: DateTime<Utc>,
    ) -> Result<(), SuiteServiceError> {
        self.links.disconnect(u, id, n).await
    }
    async fn test(&self, u: Uuid, id: Uuid, n: DateTime<Utc>) -> Result<(), SuiteServiceError> {
        self.delivery.test(u, id, n).await
    }
    async fn reenable(
        &self,
        u: Uuid,
        id: Uuid,
        n: DateTime<Utc>,
    ) -> Result<SuiteLinkView, SuiteServiceError> {
        self.links
            .reenable(u, id, n)
            .await
            .map(|link| SuiteLinkView::new(link, n))
    }
    async fn replay(
        &self,
        u: Uuid,
        id: Uuid,
        s: NaiveDate,
        n: DateTime<Utc>,
    ) -> Result<u64, SuiteServiceError> {
        self.delivery.replay(u, id, s, n).await
    }
    async fn deliveries(
        &self,
        u: Uuid,
        id: Uuid,
    ) -> Result<Vec<SuiteDeliveryRecord>, SuiteServiceError> {
        Ok(self
            .links
            .context
            .store
            .deliveries(u, id, SUITE_DELIVERIES_PAGE_SIZE)
            .await?)
    }
    async fn revoke_from_peer(
        &self,
        id: Uuid,
        h: SignedHeaders,
        b: Vec<u8>,
        n: DateTime<Utc>,
    ) -> Result<(), SuiteServiceError> {
        self.ingest.revoke_from_peer(id, h, b, n).await
    }
    async fn ingest(
        &self,
        id: Uuid,
        h: SignedHeaders,
        b: Vec<u8>,
        n: DateTime<Utc>,
    ) -> Result<IngestResult, SuiteServiceError> {
        self.ingest.ingest(id, h, b, n).await
    }
}

#[async_trait]
pub trait SuiteDeliveryRunner: Send + Sync {
    async fn deliver(
        &self,
        job: SuiteDeliverJobV1,
        now: DateTime<Utc>,
        job_id: Uuid,
        worker_id: &str,
    ) -> Result<DeliveryAction, SuiteServiceError>;
    async fn revoke(
        &self,
        job: SuiteRevokeJobV1,
        now: DateTime<Utc>,
    ) -> Result<DeliveryAction, SuiteServiceError>;
    async fn cleanup(&self, now: DateTime<Utc>) -> Result<SuiteCleanupOutcome, SuiteServiceError>;
}
#[async_trait]
impl SuiteDeliveryRunner for SuiteDeliveryService {
    async fn deliver(
        &self,
        j: SuiteDeliverJobV1,
        n: DateTime<Utc>,
        job_id: Uuid,
        worker_id: &str,
    ) -> Result<DeliveryAction, SuiteServiceError> {
        self.deliver_claimed(j, n, Some((job_id, worker_id))).await
    }
    async fn revoke(
        &self,
        j: SuiteRevokeJobV1,
        n: DateTime<Utc>,
    ) -> Result<DeliveryAction, SuiteServiceError> {
        self.revoke(j, n).await
    }
    async fn cleanup(&self, n: DateTime<Utc>) -> Result<SuiteCleanupOutcome, SuiteServiceError> {
        self.cleanup(n).await
    }
}

fn ingest_metric(prefix: &str, outcome: &'static str) {
    metrics::counter!(format!("{prefix}_suite_events_ingested_total"),"outcome"=>outcome)
        .increment(1);
}
fn delivery_metric(prefix: &str, action: DeliveryAction) {
    if let Some(outcome) = action.outcome() {
        metrics::counter!(format!("{prefix}_suite_deliveries_total"),"outcome"=>outcome)
            .increment(1);
    }
}
