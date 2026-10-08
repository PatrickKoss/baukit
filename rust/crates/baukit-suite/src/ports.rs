use std::time::Duration;

use crate::domain::{
    ActivePeer, ExchangeRequest, ExchangeResponse, RewardMode, SuiteLink, ValidatedPayload,
};
#[cfg(feature = "postgres")]
use crate::domain::{
    DeliveryAction, DeliveryState, LinkCode, LinkRequest, PreparedExchange, SuiteEvent,
};
use async_trait::async_trait;
use baukit_events::{EventEnvelope, IngestOutcome, IngestOutcomeStatus};
#[cfg(feature = "postgres")]
use chrono::NaiveDate;
use chrono::{DateTime, Utc};
use thiserror::Error;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppliedOutcomeStatus {
    Granted,
    NoRule,
    Capped,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppliedOutcome {
    pub outcome: AppliedOutcomeStatus,
    pub ledger_entry_id: Option<String>,
}

impl From<AppliedOutcome> for IngestOutcome {
    fn from(value: AppliedOutcome) -> Self {
        Self {
            outcome: match value.outcome {
                AppliedOutcomeStatus::Granted => IngestOutcomeStatus::Granted,
                AppliedOutcomeStatus::NoRule => IngestOutcomeStatus::NoRule,
                AppliedOutcomeStatus::Capped => IngestOutcomeStatus::Capped,
            },
            ledger_entry_id: value.ledger_entry_id,
        }
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum SuiteStoreError {
    #[error("suite row is missing")]
    NotFound,
    #[error("suite code is invalid, expired or consumed")]
    CodeInvalid,
    #[error("suite event id conflicts with the stored payload")]
    EventIdConflict,
    #[error("suite replay was requested within the last 24 hours")]
    ReplayTooSoon(u64),
    #[error("suite link is revoked")]
    LinkRevoked,
    #[error("suite link cannot deliver")]
    LinkInactive,
    #[error("invalid suite persistence data: {0}")]
    InvalidData(String),
    #[error("suite persistence failed: {0}")]
    Storage(String),
    #[error("suite persistence timed out")]
    Timeout,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SuiteInboundEvent {
    pub link_id: Uuid,
    pub event_id: String,
    pub user_id: Uuid,
    pub event_type: String,
    pub occurred_at: DateTime<Utc>,
    pub payload_hash: [u8; 32],
    pub replay: bool,
    pub outcome: AppliedOutcome,
    pub received_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InboxLookup {
    New,
    Duplicate(AppliedOutcome),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SuiteDeliveryRecord {
    pub job_id: Uuid,
    pub link_id: Uuid,
    pub event_type: Option<String>,
    pub status: String,
    pub attempt_count: u32,
    pub last_error_code: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SuiteCleanupOutcome {
    pub requests: u64,
    pub codes: u64,
    pub revoked_links: u64,
    pub unused_links: u64,
    pub terminal_jobs: u64,
}

// One transaction crosses link replacement, inbox, applier and outbox operations.
#[async_trait]
#[cfg(feature = "postgres")]
pub trait SuiteLinkStore: Send + Sync {
    async fn begin_transaction(
        &self,
    ) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, SuiteStoreError>;
    async fn lock_ingest_user(
        &self,
        tx: &mut sqlx::PgConnection,
        user_id: Uuid,
    ) -> Result<(), SuiteStoreError>;
    async fn commit_transaction(
        &self,
        tx: sqlx::Transaction<'static, sqlx::Postgres>,
    ) -> Result<(), SuiteStoreError>;
    /// Serializes starts per owner/peer, consumes their open requests, then inserts the new one.
    async fn create_request_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        request: &LinkRequest,
        now: DateTime<Utc>,
    ) -> Result<(), SuiteStoreError>;
    /// Includes expired/consumed requests; callback lookup never consumes them.
    async fn find_request_by_state_hash(
        &self,
        state_hash: [u8; 32],
    ) -> Result<Option<LinkRequest>, SuiteStoreError>;
    /// Locks the row, including completed requests; another owner's row returns None.
    async fn request_for_owner_for_update(
        &self,
        tx: &mut sqlx::PgConnection,
        request_id: Uuid,
        owner_id: Uuid,
    ) -> Result<Option<LinkRequest>, SuiteStoreError>;
    async fn prepare_exchange_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        request_id: Uuid,
        owner_id: Uuid,
        exchange: &PreparedExchange,
    ) -> Result<(), SuiteStoreError>;
    async fn replay_retry_after(
        &self,
        link_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<u64, SuiteStoreError>;
    /// Consumes an unexpired open request after exchange; a non-open row is CodeInvalid.
    async fn consume_request_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        request_id: Uuid,
        owner_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<(), SuiteStoreError>;
    /// Records the link with insertion/replay; an open or already linked request is CodeInvalid.
    async fn record_request_link_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        request_id: Uuid,
        owner_id: Uuid,
        link_id: Uuid,
    ) -> Result<(), SuiteStoreError>;
    async fn create_code_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        code: &LinkCode,
    ) -> Result<(), SuiteStoreError>;
    /// Includes expired/consumed codes so domain validation can distinguish an exchange retry.
    async fn code_for_update(
        &self,
        tx: &mut sqlx::PgConnection,
        code_hash: [u8; 32],
    ) -> Result<Option<LinkCode>, SuiteStoreError>;
    /// Reads the link recorded on the locked code, including revoked links.
    async fn link_for_code_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        code_hash: [u8; 32],
    ) -> Result<Option<SuiteLink>, SuiteStoreError>;
    /// Consumes an open code and records its link atomically; a non-open row is CodeInvalid.
    async fn consume_code_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        code_hash: [u8; 32],
        link_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<(), SuiteStoreError>;
    async fn link_for_update(
        &self,
        tx: &mut sqlx::PgConnection,
        link_id: Uuid,
    ) -> Result<SuiteLink, SuiteStoreError>;
    async fn active_link_for_update(
        &self,
        tx: &mut sqlx::PgConnection,
        owner_id: Uuid,
        peer_app: &str,
    ) -> Result<Option<SuiteLink>, SuiteStoreError>;
    async fn insert_link_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        link: &SuiteLink,
    ) -> Result<(), SuiteStoreError>;
    async fn revoke_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        link_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<(), SuiteStoreError>;
    async fn find_link(&self, link_id: Uuid) -> Result<Option<SuiteLink>, SuiteStoreError>;
    async fn find_link_for_owner(
        &self,
        link_id: Uuid,
        owner_id: Uuid,
    ) -> Result<Option<SuiteLink>, SuiteStoreError>;
    async fn list_links(&self, owner_id: Uuid) -> Result<Vec<SuiteLink>, SuiteStoreError>;
    async fn update_preferences(
        &self,
        owner_id: Uuid,
        link_id: Uuid,
        share_xp: Option<bool>,
        reward_mode: Option<RewardMode>,
        now: DateTime<Utc>,
    ) -> Result<SuiteLink, SuiteStoreError>;
    async fn reenable(
        &self,
        owner_id: Uuid,
        link_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<SuiteLink, SuiteStoreError>;
    async fn record_delivery(
        &self,
        link_id: Uuid,
        action: DeliveryAction,
        now: DateTime<Utc>,
    ) -> Result<DeliveryState, SuiteStoreError>;
    async fn complete_delivery(
        &self,
        link_id: Uuid,
        job_id: Uuid,
        worker_id: &str,
        now: DateTime<Utc>,
    ) -> Result<(), SuiteStoreError>;
    async fn inbox_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        link_id: Uuid,
        event_id: &str,
        payload_hash: [u8; 32],
    ) -> Result<InboxLookup, SuiteStoreError>;
    /// Stores the outcome and updates the link's last_received_at in this transaction.
    async fn record_inbound_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        event: &SuiteInboundEvent,
    ) -> Result<(), SuiteStoreError>;
    /// Updates last_received_at for a protocol test without writing an inbox row.
    async fn record_received_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        link_id: Uuid,
        received_at: DateTime<Utc>,
    ) -> Result<(), SuiteStoreError>;
    async fn deliveries(
        &self,
        owner_id: Uuid,
        link_id: Uuid,
        limit: u32,
    ) -> Result<Vec<SuiteDeliveryRecord>, SuiteStoreError>;
    async fn cleanup(&self, now: DateTime<Utc>) -> Result<SuiteCleanupOutcome, SuiteStoreError>;
}

#[async_trait]
#[cfg(feature = "postgres")]
pub trait SuiteEventOutbox: Send + Sync {
    /// Call before locking product rows to serialize domain writes with ingest and erasure.
    async fn lock_owner_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        owner_id: Uuid,
    ) -> Result<(), SuiteStoreError>;
    async fn enqueue_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        owner_id: Uuid,
        events: &[SuiteEvent],
    ) -> Result<u64, SuiteStoreError>;
    async fn enqueue_revoke_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        link_id: Uuid,
    ) -> Result<(), SuiteStoreError>;
    async fn enqueue_replay_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        owner_id: Uuid,
        link_id: Uuid,
        since: NaiveDate,
        requested_at: DateTime<Utc>,
        events: &[SuiteEvent],
    ) -> Result<u64, SuiteStoreError>;
    async fn enqueue_test_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        owner_id: Uuid,
        link_id: Uuid,
        event: &SuiteEvent,
    ) -> Result<(), SuiteStoreError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SuitePeerResponse {
    pub status: u16,
    pub retry_after: Option<Duration>,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum SuitePeerError {
    #[error("suite peer timed out")]
    Timeout,
    #[error("suite peer transport failed")]
    Transport,
    #[error("suite peer DNS lookup failed")]
    Dns,
    #[error("suite peer rejected the request with status {status}: {code}")]
    Rejected { status: u16, code: String },
    #[error("suite peer returned an invalid response")]
    InvalidResponse,
}

pub struct SuiteSignedCall<'a> {
    pub remote_link_id: Uuid,
    pub delivery_id: &'a str,
    pub timestamp: i64,
    pub source_app: &'a str,
    pub secret: &'a [u8],
    pub body: &'a [u8],
    pub replay: bool,
}

#[async_trait]
pub trait SuitePeerClient: Send + Sync {
    async fn exchange(
        &self,
        peer: &ActivePeer,
        request: &ExchangeRequest,
    ) -> Result<ExchangeResponse, SuitePeerError>;
    async fn revoke(
        &self,
        peer: &ActivePeer,
        call: SuiteSignedCall<'_>,
    ) -> Result<SuitePeerResponse, SuitePeerError>;
    async fn deliver(
        &self,
        peer: &ActivePeer,
        call: SuiteSignedCall<'_>,
    ) -> Result<SuitePeerResponse, SuitePeerError>;
}

#[async_trait]
/// The caller holds a suite owner advisory lock. Lock product rows here before applying.
#[cfg(feature = "postgres")]
pub trait SuiteEventApplier: Send + Sync {
    async fn apply(
        &self,
        tx: &mut sqlx::PgConnection,
        event: SuiteApplyEvent<'_>,
    ) -> Result<AppliedOutcome, SuiteStoreError>;
}

pub struct SuiteApplyEvent<'a> {
    pub owner_id: Uuid,
    pub link: &'a SuiteLink,
    pub envelope: &'a EventEnvelope,
    pub payload: &'a ValidatedPayload,
    pub reward_mode: RewardMode,
    /// SHA-256 of canonical payload JSON with sorted keys and compact encoding.
    /// Provider history hashes the same bytes.
    pub payload_hash: [u8; 32],
    pub replay: bool,
    pub received_at: DateTime<Utc>,
}

#[async_trait]
pub trait SuiteProfileRevoker: Send + Sync {
    async fn revoke_before_erasure(&self, links: &[SuiteLink]);
}

#[async_trait]
pub trait SuiteErasureLinks: Send + Sync {
    async fn links_for_erasure(&self, subject: &str) -> Result<Vec<SuiteLink>, SuiteStoreError>;
}

#[async_trait]
pub trait SuiteErasureNotifier: Send + Sync {
    async fn notify_before_erasure(&self, subject: &str);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_applied_outcomes_can_be_converted_to_ingest_responses() {
        for (outcome, ledger_entry_id, expected) in [
            (
                AppliedOutcomeStatus::Granted,
                Some("ledger-1".to_owned()),
                IngestOutcomeStatus::Granted,
            ),
            (
                AppliedOutcomeStatus::NoRule,
                None,
                IngestOutcomeStatus::NoRule,
            ),
            (
                AppliedOutcomeStatus::Capped,
                None,
                IngestOutcomeStatus::Capped,
            ),
        ] {
            let response: IngestOutcome = AppliedOutcome {
                outcome,
                ledger_entry_id: ledger_entry_id.clone(),
            }
            .into();
            assert_eq!(response.outcome, expected);
            assert_eq!(response.ledger_entry_id, ledger_entry_id);
        }
    }
}

/// Reads historical product events. Reuse the product's live event builders.
#[async_trait]
#[cfg(feature = "postgres")]
pub trait SuiteReplaySource: Send + Sync {
    async fn replay_since(
        &self,
        connection: &mut sqlx::PgConnection,
        owner_id: Uuid,
        since: NaiveDate,
    ) -> Result<Vec<SuiteEvent>, SuiteStoreError>;
}

/// Looks up the product's stable outbound subject and current display name.
#[async_trait]
#[cfg(feature = "postgres")]
pub trait SuiteIdentitySource: Send + Sync {
    async fn identity(&self, owner_id: Uuid) -> Result<SuiteIdentity, SuiteStoreError>;
    /// Uses the caller's connection during emission, without acquiring another pool connection.
    async fn identity_in_transaction(
        &self,
        connection: &mut sqlx::PgConnection,
        owner_id: Uuid,
    ) -> Result<SuiteIdentity, SuiteStoreError>;
}

#[derive(Clone, Debug)]
pub struct SuiteIdentity {
    pub subject: String,
    pub display_name: Option<String>,
}
