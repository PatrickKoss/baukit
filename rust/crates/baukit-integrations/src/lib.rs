//! Provider-neutral contracts for external providers.
//!
//! The crate is a port and nothing else. It carries no HTTP client, no
//! database, and no provider adapter, so a product can name the connector shape
//! without inheriting a driver it never calls.
//!
//! [`IntegrationConnector`] is the paged-import seam. An implementation fetches one
//! cursor-paged [`ConnectorPage`] of records for a leased job, verifies webhook
//! deliveries, and reports failures as a [`ConnectorError`]. Everything a
//! provider knows stays inside the implementation: OAuth, scopes, response
//! models, cursor encoding, and which record duplicates which.
//!
//! [`CredentialProbe`] is separate from import jobs. It checks a credential,
//! returns a redacted [`ExternalAccountId`], and maps failures to six fixed
//! outcomes with [`CredentialProbeError`]. The product still owns its endpoint,
//! headers, required scopes, and response parser.
//!
//! # One retry vocabulary
//!
//! [`ConnectorError`] carries a [`RetryClass`] re-exported from `baukit-http`,
//! so the classification made at the HTTP boundary survives to the runner's
//! requeue decision without translation:
//!
//! ```rust
//! use baukit_http::classify_transport_error;
//! use baukit_integrations::{ConnectorError, RetryClass};
//!
//! let class = classify_transport_error(true);
//! assert_eq!(class, RetryClass::Timeout);
//!
//! let error = ConnectorError::new("timeout", class);
//! assert!(error.is_retryable());
//! assert!(!ConnectorError::revoked("revoked").is_retryable());
//! ```
//!
//! # Composing with `baukit-jobs`
//!
//! The crate does not depend on `baukit-jobs`. A product's job handler calls
//! [`IntegrationConnector::fetch_page`] and turns the error class into a
//! `JobError`. The README walks through that mapping.
//!
//! # Testing
//!
//! `baukit_test::FakeConnector` scripts paged-import failures.
//! `baukit_test::check_credential_probe_conformance` runs a product credential
//! adapter against raw responses from a loopback HTTP server.

#![deny(missing_docs)]

mod credential_probe;
mod health;
mod port;

pub use baukit_http::RetryClass;
pub use credential_probe::{
    CredentialProbe, CredentialProbeError, CredentialProbeFuture, CredentialProbeResult,
    CredentialProbeSuccess, ExternalAccountId, InvalidExternalAccountId,
    MAX_CREDENTIAL_PROBE_RESPONSE_BYTES, MAX_EXTERNAL_ACCOUNT_ID_BYTES,
};
pub use health::{ConnectionHealth, ConnectionStatus};
pub use port::{
    ClaimedConnectorJob, ConnectorError, ConnectorFuture, ConnectorPage, IntegrationConnector,
    VerifiedWebhook, WebhookIngestResult, WebhookVerificationError,
};

// Compiles the README's examples so they cannot drift from the API.
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
struct ReadmeDoctests;
