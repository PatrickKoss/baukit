//! Erase product data atomically, then delete the identity account durably.
#![deny(missing_docs)]

#[cfg(feature = "api-providers")]
mod api;
#[cfg(feature = "keycloak")]
mod keycloak;
mod store;
mod worker;

use std::{future::Future, pin::Pin};

#[cfg(feature = "api-providers")]
pub use api::{ApiDeletionConfig, ClerkAccountDeleter, WorkOsAccountDeleter};
#[cfg(feature = "keycloak")]
pub use keycloak::{KeycloakAccountDeleter, KeycloakDeletionConfig};
pub use store::{
    ErasureError, ErasureOutcome, ErasureService, ErasureState, PostgresErasureStore,
    ProductErasure, reject_fenced_subject,
};
pub use worker::IdentityDeletionHandler;

/// Reference migration to copy after the baukit-jobs migrations.
pub const POSTGRES_MIGRATION_SQL: &str = include_str!("../migrations/0001_erasure.sql");
/// Durable job identifier.
pub const IDENTITY_DELETE_JOB_TYPE: &str = "identity.account.delete";
/// Boxed asynchronous result used by erasure ports.
pub type ErasureFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Safe failure classes. Errors never include provider bodies, subjects, or credentials.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum IdentityDeletionError {
    /// Network, token acquisition, rate limit, or server failure.
    #[error("identity deletion temporarily unavailable")]
    Retryable,
    /// Invalid request or missing provider permission.
    #[error("identity deletion rejected permanently")]
    Permanent,
}

/// Provider-neutral account deletion. An absent account is successful deletion.
pub trait IdentityAccountDeleter: Send + Sync + 'static {
    /// Deletes the account and its provider sessions.
    fn delete_account<'a>(
        &'a self,
        subject: &'a str,
    ) -> ErasureFuture<'a, Result<(), IdentityDeletionError>>;
}
