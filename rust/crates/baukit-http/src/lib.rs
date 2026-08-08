//! Shared Axum middleware, HTTP telemetry, and public errors for Baukit services.
//!
//! [`finalize`] applies one consistent request lifecycle: standard extractor and
//! routing errors, request identity, W3C trace extraction and response
//! propagation, route-template spans, HTTP RED metrics, panic and timeout
//! envelopes, body and concurrency limits, and explicit CORS.
//!
//! # Example
//!
//! ```rust
//! use axum::{Router, routing::get};
//! use baukit_http::{HttpOptions, RequestId, finalize};
//!
//! async fn hello(request_id: RequestId) -> String {
//!     format!("hello from request {}", request_id.as_str())
//! }
//!
//! let options = HttpOptions::default()
//!     .with_allowed_origins(["https://app.example.com"])?;
//! let app = finalize(Router::new().route("/hello", get(hello)), options);
//! # let _: Router = app;
//! # Ok::<(), baukit_http::HttpOptionsError>(())
//! ```
//!
//! This crate records HTTP metrics through the [`metrics`] facade and never
//! installs a recorder. The recorder owner must configure
//! `http_request_duration_seconds` with [`DURATION_BUCKETS`].

#![deny(missing_docs)]

mod error;
mod extract;
mod middleware;
mod options;
mod routing;

pub use baukit_openapi::{ErrorBody, ErrorEnvelope};
pub use error::ApiError;
pub use extract::{ApiJson, ApiPath, ApiQuery};
pub use middleware::{
    DURATION_BUCKETS, HTTP_REQUEST_DURATION_SECONDS, HTTP_REQUESTS_IN_FLIGHT, HTTP_REQUESTS_TOTAL,
    RequestId, X_REQUEST_ID, extract_trace_context, inject_current_trace_context,
    inject_trace_context, layers,
};
pub use options::{HttpOptions, HttpOptionsError};
pub use routing::finalize;

/// The metrics facade used by the HTTP lifecycle middleware.
///
/// Applications normally initialize a recorder through `baukit-telemetry`.
pub use metrics;

#[cfg(test)]
mod tests;
