use std::{collections::BTreeSet, sync::Arc};

use baukit_config::Secret;
use baukit_erasure::ErasureService;
use chrono::TimeDelta;

use crate::{
    CONTRACT_VERSION, DiscoveryDocument, IdentifierType, Limits, OperationKind, ReceiptKey,
    ReceiptKeyStatus, ReceiptSigner, ReceiptVerifier, Scopes, Service,
};

/// Explicit product configuration. Secrets have no debug representation here.
pub struct DataSubjectConfig {
    /// Service identity signed into every receipt.
    pub service: Service,
    /// OAuth client IDs allowed to use authenticated routes.
    pub allowed_client_ids: BTreeSet<String>,
    /// Accepted identifier types, also advertised in discovery.
    pub identifier_types: Vec<IdentifierType>,
    /// Advertised validation bounds.
    pub limits: Limits,
    /// Current product signing key.
    pub signer: ReceiptSigner,
    /// Previous public keys needed by retained receipts, all marked previous.
    pub previous_receipt_keys: Vec<ReceiptKey>,
    /// Stable secret with at least 32 bytes for domain-separated request hashes.
    pub hash_key: Secret<String>,
    /// Operation and receipt retention, default 400 days.
    pub operation_retention: TimeDelta,
    /// Maximum erasure job attempts, default five.
    pub max_attempts: u32,
    /// Wait for identity deletion after local erasure, default 24 hours, at most 30 days.
    pub identity_deletion_timeout: TimeDelta,
    /// Product fence store and explicit identity retention policy, using the same database.
    pub erasure: ErasureService,
}

impl DataSubjectConfig {
    /// Starts with default limits, 400-day retention and five job attempts.
    pub fn new(
        service: Service,
        allowed_client_ids: BTreeSet<String>,
        identifier_types: Vec<IdentifierType>,
        signer: ReceiptSigner,
        hash_key: Secret<String>,
        erasure: ErasureService,
    ) -> Self {
        Self {
            service,
            allowed_client_ids,
            identifier_types,
            limits: Limits::default(),
            signer,
            previous_receipt_keys: Vec::new(),
            hash_key,
            operation_retention: TimeDelta::days(400),
            max_attempts: 5,
            identity_deletion_timeout: TimeDelta::days(1),
            erasure,
        }
    }

    pub(super) fn validate(&self) -> Result<(), DataSubjectError> {
        if self.service.name.trim().is_empty()
            || self.service.version.trim().is_empty()
            || self.allowed_client_ids.is_empty()
            || self
                .allowed_client_ids
                .iter()
                .any(|id| id.trim().is_empty())
            || self.identifier_types.is_empty()
            || self
                .identifier_types
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                != self.identifier_types.len()
            || self.limits.max_identifiers == 0
            || self.limits.max_identifier_length == 0
            || self.limits.max_export_part_bytes == 0
            || self.hash_key.expose().len() < 32
            || self.operation_retention <= TimeDelta::zero()
            || chrono::Utc::now()
                .checked_add_signed(self.operation_retention)
                .is_none()
            || self.max_attempts == 0
            || self.max_attempts > i32::MAX as u32
            || self.identity_deletion_timeout <= TimeDelta::zero()
            || self.identity_deletion_timeout > TimeDelta::days(30)
            || self
                .previous_receipt_keys
                .iter()
                .any(|key| key.status != ReceiptKeyStatus::Previous)
        {
            return Err(DataSubjectError::Configuration);
        }
        ReceiptVerifier::from_discovery(&self.discovery())
            .map_err(|_| DataSubjectError::Configuration)?;
        Ok(())
    }

    pub(super) fn discovery(&self) -> DiscoveryDocument {
        let mut receipt_keys = vec![self.signer.receipt_key().clone()];
        receipt_keys.extend(self.previous_receipt_keys.clone());
        DiscoveryDocument {
            contract_version: CONTRACT_VERSION.into(),
            service: self.service.clone(),
            operations: vec![OperationKind::Erasure],
            identifier_types: self.identifier_types.clone(),
            scopes: Scopes::default(),
            limits: self.limits,
            receipt_keys,
        }
    }
}

/// Safe server failure classes. Database causes and product errors are discarded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DataSubjectError {
    /// Missing or unsafe product configuration.
    #[error("invalid data subject configuration")]
    Configuration,
    /// A database operation failed, without exposing its diagnostic content.
    #[error("data subject database unavailable")]
    Database,
    /// A durable enqueue or lease operation failed.
    #[error("data subject job store unavailable")]
    Jobs,
    /// Stored data or receipt signing failed.
    #[error("invalid data subject stored result")]
    Result,
    /// Same client, kind and idempotency key with a different normalized request.
    #[error("data subject idempotency conflict")]
    Conflict,
}

impl From<sqlx::Error> for DataSubjectError {
    fn from(_: sqlx::Error) -> Self {
        Self::Database
    }
}
impl From<baukit_jobs::StoreError> for DataSubjectError {
    fn from(_: baukit_jobs::StoreError) -> Self {
        Self::Jobs
    }
}
impl From<serde_json::Error> for DataSubjectError {
    fn from(_: serde_json::Error) -> Self {
        Self::Result
    }
}

pub(super) type SharedConfig = Arc<DataSubjectConfig>;
