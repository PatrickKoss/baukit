//! Provider-neutral contract for importing records from external providers.
//!
//! The crate is a port and nothing else. It carries no HTTP client, no
//! database, and no provider adapter, so a product can name the connector shape
//! without inheriting a driver it never calls.
//!
//! [`IntegrationConnector`] is the whole seam. An implementation fetches one
//! cursor-paged [`ConnectorPage`] of records for a leased job, verifies webhook
//! deliveries, and reports failures as a [`ConnectorError`]. Everything a
//! provider knows stays inside the implementation: OAuth, scopes, response
//! models, cursor encoding, and which record duplicates which.
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
//! `baukit_test::FakeConnector` scripts the failure modes worth testing:
//! healthy, rate limited, unavailable, timeout, revoked, and exhausted.

#![deny(missing_docs)]

mod health;
mod port;

pub use baukit_http::RetryClass;
pub use health::{ConnectionHealth, ConnectionStatus};
pub use port::{
    ClaimedConnectorJob, ConnectorError, ConnectorFuture, ConnectorPage, IntegrationConnector,
    VerifiedWebhook, WebhookIngestResult, WebhookVerificationError,
};

// Compiles the README's examples so they cannot drift from the API.
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
struct ReadmeDoctests;
