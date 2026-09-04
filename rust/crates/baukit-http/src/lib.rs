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
//!     .with_allowed_origins(["https://app.example.com"])?
//!     .with_additional_allowed_headers(["accept", "x-webhook-secret"])?
//!     .with_json_rejection_code("invalid_json")?;
//! let app = finalize(Router::new().route("/hello", get(hello)), options);
//! # let _: Router = app;
//! # Ok::<(), baukit_http::HttpOptionsError>(())
//! ```
//!
//! [`pagination`] adds keyset pagination whose opaque cursors are bound to the
//! request filters, and [`classify_http_status`] turns an upstream response into
//! a [`RetryClass`] so outbound clients share one retry policy.
//!
//! # Paginated handler
//!
//! ```rust
//! use axum::extract::Query;
//! use baukit_http::{ApiError, Page, PageKey, PageParams, ResponseEnvelope};
//! use serde::{Deserialize, Serialize};
//! use uuid::Uuid;
//!
//! #[derive(Deserialize)]
//! struct ListQuery {
//!     limit: Option<i64>,
//!     cursor: Option<String>,
//!     category: Option<String>,
//! }
//!
//! #[derive(Serialize)]
//! struct Filters {
//!     category: Option<String>,
//! }
//!
//! #[derive(Clone, Serialize)]
//! struct Item {
//!     id: Uuid,
//!     name: String,
//! }
//!
//! #[derive(Serialize)]
//! struct PageMeta {
//!     next_cursor: Option<String>,
//! }
//!
//! async fn list(
//!     Query(query): Query<ListQuery>,
//! ) -> Result<ResponseEnvelope<Vec<Item>, PageMeta>, ApiError> {
//!     let params = PageParams::new(query.limit, query.cursor)?;
//!     let filters = Filters { category: query.category };
//!     let after = params.decode_cursor(&filters)?;
//!
//!     // Fetch `params.fetch_limit()?` rows ordered by (name, id), starting
//!     // after `after.page_key::<String>()?` when it is present.
//!     let _ = after;
//!     let rows: Vec<Item> = Vec::new();
//!
//!     let page = Page::from_rows(rows, &params, &filters, |item| {
//!         PageKey::new(item.name.clone(), item.id)
//!     })?;
//!     Ok(ResponseEnvelope::new(
//!         page.items,
//!         PageMeta { next_cursor: page.next_cursor },
//!     ))
//! }
//! # let _ = list;
//! ```
//!
//! This crate records HTTP metrics through the [`metrics`] facade and never
//! installs a recorder. The recorder owner must configure
//! `http_request_duration_seconds` with [`DURATION_BUCKETS`].

#![deny(missing_docs)]

mod error;
mod extract;
mod locale;
mod middleware;
mod options;
pub mod pagination;
pub mod retry;
mod routing;

pub use baukit_openapi::{ErrorBody, ErrorEnvelope, ResponseEnvelope, Rfc3339DateTime};
pub use error::ApiError;
pub use extract::{ApiJson, ApiPath, ApiQuery, Path, Query};
pub use locale::{
    LocaleQueryOverride, MAX_ACCEPT_LANGUAGE_BYTES, MAX_LOCALE_QUERY_BYTES, MAX_SUPPORTED_LOCALES,
    RequestLocale, RequestLocaleConfig, RequestLocaleConfigError, RequestLocaleRejection,
};
pub use middleware::{
    DURATION_BUCKETS, HTTP_REQUEST_DURATION_SECONDS, HTTP_REQUESTS_IN_FLIGHT, HTTP_REQUESTS_TOTAL,
    RequestId, X_REQUEST_ID, extract_trace_context, inject_current_trace_context,
    inject_trace_context, layers,
};
pub use options::{HttpOptions, HttpOptionsError, JsonRejectionCodes};
pub use pagination::{
    Cursor, DEFAULT_PAGE_LIMIT, MAX_PAGE_LIMIT, Page, PageKey, PageParams, PaginationError,
};
pub use retry::{
    RetryClass, RetryHeaderOptions, classify_http_status, classify_http_status_with_options,
    classify_transport_error, retry_after_from_headers, retry_after_from_headers_at,
    retry_after_from_headers_with_options, retry_after_from_headers_with_options_at,
};
pub use routing::finalize;

/// The metrics facade used by the HTTP lifecycle middleware.
///
/// Applications normally initialize a recorder through `baukit-telemetry`.
pub use metrics;

#[cfg(test)]
mod tests;

// Compiles the README's examples so they cannot drift from the API.
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
struct ReadmeDoctests;
