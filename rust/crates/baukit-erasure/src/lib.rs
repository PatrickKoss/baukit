//! Erase product data atomically and choose whether to delete the identity account.
#![deny(missing_docs)]

#[cfg(feature = "api-providers")]
mod api;
#[cfg(any(feature = "api-providers", feature = "keycloak"))]
mod egress;
pub mod kernel;
#[cfg(feature = "keycloak")]
mod keycloak;
mod store;
mod worker;
mod write_fence;

use std::{future::Future, pin::Pin};

#[cfg(feature = "api-providers")]
pub use api::{ApiDeletionConfig, ClerkAccountDeleter, WorkOsAccountDeleter};
#[cfg(feature = "keycloak")]
pub use keycloak::{KeycloakAccountDeleter, KeycloakDeletionConfig};
pub use store::{
    ErasureError, ErasureOutcome, ErasurePreflight, ErasureService, ErasureState,
    IdentityRetention, PostgresErasureStore, ProductErasure, reject_fenced_subject,
};
pub use worker::IdentityDeletionHandler;
pub use write_fence::{WriteFence, begin_fenced_write, with_write_fence};

/// Reference migration to copy after the baukit-jobs migrations.
pub const POSTGRES_MIGRATION_SQL: &str = include_str!("../migrations/0001_erasure.sql");
/// Reference migration to copy after 0001 erasure.
pub const POSTGRES_REONBOARDING_MIGRATION_SQL: &str =
    include_str!("../migrations/0002_erasure_reonboarding.sql");
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
    /// The provider's circuit is open or its client is saturated; no request was sent.
    ///
    /// The job runner defers the job by `retry_after` without consuming an attempt.
    #[error("identity provider unavailable")]
    Unavailable {
        /// Time until the provider's client admits a call again.
        retry_after: std::time::Duration,
    },
}

/// Provider-neutral account deletion. An account the provider reports as missing is
/// successful deletion; an unexplained `404` is not.
pub trait IdentityAccountDeleter: Send + Sync + 'static {
    /// Deletes the account and its provider sessions.
    fn delete_account<'a>(
        &'a self,
        subject: &'a str,
    ) -> ErasureFuture<'a, Result<(), IdentityDeletionError>>;

    /// Checks that the configured endpoint answers like the provider, for readiness.
    /// A misconfigured base URL must fail here instead of turning every deletion into a 404.
    fn probe(&self) -> ErasureFuture<'_, Result<(), IdentityDeletionError>> {
        Box::pin(async { Ok(()) })
    }
}

/// Pure helpers and SQL exposed for fuzz targets and agreement tests. Not a stable API.
#[doc(hidden)]
pub mod internals {
    #[cfg(any(feature = "api-providers", feature = "keycloak"))]
    pub use crate::egress::{MissingUserDialect, body_says_missing, user_url};
    pub use crate::store::REJECTED_SESSION_SQL;
    pub use crate::worker::parse_payload;
}
