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
//! Every `5xx` with a valid `Retry-After` reports the capped delay through
//! [`EgressError::retry_class`]. Enable [`EgressOptions::with_error_body_limit`]
//! to capture a bounded error body and read its [`ErrorBody::problem_code`].
//! Bodies may contain personal data; error messages and debug output omit raw bytes.
//!
//! The address policy follows the IANA special-purpose address registries and
//! is pinned by the shared vectors in `fixtures/egress/address-policy-v1.json`.
//! [`AddressPolicy::PublicOnly`] is the default. For in-cluster services such
//! as Prometheus, use [`AddressPolicy::AllowPrivate`] with a non-empty
//! [`PrivateNetworks`] list. [`PrivateNetwork`] accepts private or shared CIDR
//! ranges with no host bits, such as `10.43.0.0/16` or `fd12:3456::/48`.
//! HTTPS can reach public and listed addresses; plain HTTP reaches listed
//! addresses only. Every DNS answer must pass, including [`StaticResolver`]
//! answers. Cloud metadata stays blocked, even inside a listed network.
//! [`AddressPolicy::AllowLoopback`] is for local development and tests.
//!
//! # Example
//!
//! ```rust,no_run
//! use baukit_egress::{EgressOptions, EgressRequest, GuardedClient, ErrorBody};
//! use url::Url;
//!
//! # async fn deliver(body: Vec<u8>) -> Result<(), Box<dyn std::error::Error>> {
//! let options = EgressOptions::default().with_error_body_limit(4096)?;
//! let client = GuardedClient::new(options)?;
//! let url = Url::parse("https://hooks.example.com/deliver")?;
//! match client.execute(EgressRequest::post(url).with_body(body)).await {
//!     Ok(response) => tracing::debug!(status = %response.status(), "request succeeded"),
//!     Err(error) => {
//!         let retry_after = error.retry_class().retry_after();
//!         let problem_code = error.error_body().and_then(ErrorBody::problem_code);
//!         tracing::warn!(code = error.code(), problem_code, ?retry_after, "delivery failed");
//!         return Err(error.into());
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! The delivery scheduler honors `retry_after()` when retrying a `503` or `429`.
//! A `409` is permanent; log its problem code, such as `inventory_sequence_gap`.
//!
//! # In-cluster services
//!
//! ```rust,no_run
//! use baukit_egress::{AddressPolicy, EgressOptions, EgressRequest, GuardedClient, PrivateNetworks};
//! use url::Url;
//!
//! # async fn collect() -> Result<(), Box<dyn std::error::Error>> {
//! let networks = PrivateNetworks::new(["10.43.0.0/16".parse()?])?;
//! let options = EgressOptions::default().with_policy(AddressPolicy::AllowPrivate(networks));
//! let client = GuardedClient::new(options)?;
//! let url = Url::parse("http://prometheus.monitoring.svc:9090/api/v1/query?query=up")?;
//! let response = client.execute(EgressRequest::get(url)).await?;
//! # let _ = response;
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
pub use error::{DestinationRejection, EgressError, ErrorBody};
pub use options::{EgressOptions, EgressOptionsError};
pub use policy::{
    AddressPolicy, PrivateNetwork, PrivateNetworkError, PrivateNetworks, is_public_address,
};
pub use resolver::{
    ResolveError, ResolveFuture, Resolver, StaticResolver, SystemResolver, resolve_destination,
};
