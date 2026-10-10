//! A guarded outbound HTTP client for destinations that users supply.
//!
//! Webhook URLs and provider endpoints come from users, so a plain HTTP client
//! can be pointed at `169.254.169.254`, a database on the private network, or
//! a host name whose DNS answer changes after it was checked. [`GuardedClient`]
//! closes those paths:
//!
//! - it resolves host names through the [`Resolver`] port and rejects the
//!   lookup when any answer fails the [`AddressPolicy`];
//! - it connects only to the answers it checked, once per new connection, so a
//!   second lookup cannot swap in another address;
//! - it never follows redirects and ignores proxy settings from the
//!   environment;
//! - it bounds the lookup, the connection, the whole request, and the response
//!   body;
//! - it returns an [`EgressError`] whose [`EgressError::retry_class`] uses
//!   [`baukit_http::classify_http_status_with_options`], and whose messages
//!   never contain the URL.
//!
//! `401` and `403` are revoked by default. Enable
//! [`EgressOptions::with_forbidden_rate_limit`] to recognize rate-limit headers
//! on `403` responses. `401` always stays revoked.
//!
//! The address policy follows the IANA special-purpose address registries and
//! is pinned by the shared vectors in `fixtures/egress/address-policy-v1.json`.
//!
//! # Example
//!
//! ```rust,no_run
//! use baukit_egress::{EgressOptions, EgressRequest, GuardedClient};
//! use url::Url;
//!
//! # async fn deliver(body: Vec<u8>) -> Result<(), Box<dyn std::error::Error>> {
//! let client = GuardedClient::new(EgressOptions::default())?;
//! let url = Url::parse("https://hooks.example.com/deliver")?;
//! match client.execute(EgressRequest::post(url).with_body(body)).await {
//!     Ok(_) => {}
//!     Err(error) if error.is_retryable() => {
//!         // Schedule another attempt, honoring `error.retry_class().retry_after()`.
//!     }
//!     Err(error) => {
//!         // Record `error.code()` and stop.
//!         let _ = error.code();
//!     }
//! }
//! # Ok(())
//! # }
//! ```

#![deny(missing_docs)]

mod client;
mod error;
mod options;
mod policy;
mod resolver;

pub use client::{
    EgressClientError, EgressRequest, EgressResponse, GuardedClient, ResponseBody,
    validate_destination,
};
pub use error::{DestinationRejection, EgressError};
pub use options::{EgressOptions, EgressOptionsError};
pub use policy::{AddressPolicy, is_public_address};
pub use resolver::{
    ResolveError, ResolveFuture, Resolver, StaticResolver, SystemResolver, resolve_destination,
};
