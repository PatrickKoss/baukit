use std::{collections::BTreeMap, error::Error as StdError, fmt};

use axum::{
    Json,
    extract::rejection::{JsonRejection, PathRejection, QueryRejection},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header::RETRY_AFTER},
    response::{IntoResponse, Response},
};
use serde_json::Value;

use crate::{ErrorBody, ErrorEnvelope, middleware::current_request_id};

/// A safe public API error rendered in Baukit's standard JSON envelope.
///
/// The public code and message must be stable and safe for clients. Attach
/// internal diagnostic errors with [`ApiError::internal`]; causes are logged at
/// the request span and are never serialized.
pub struct ApiError {
    status: StatusCode,
    code: String,
    message: String,
    details: BTreeMap<String, Value>,
    headers: Option<Box<HeaderMap>>,
    cause: Option<Box<dyn StdError + Send + Sync>>,
}

impl ApiError {
    /// Creates a public API error.
    ///
    /// `code` should be a stable snake_case identifier and `message` must not
    /// contain internal or sensitive information.
    pub fn new(status: StatusCode, code: impl Into<String>, message: impl Into<String>) -> Self {
        let code = code.into();
        assert!(
            is_valid_error_code(&code),
            "API error codes must be non-empty snake_case identifiers"
        );
        Self {
            status,
            code,
            message: message.into(),
            details: BTreeMap::new(),
            headers: None,
            cause: None,
        }
    }

    /// Replaces the structured details object.
    #[must_use]
    pub fn with_details(mut self, details: BTreeMap<String, Value>) -> Self {
        self.details = details;
        self
    }

    /// Adds a header to the error response.
    ///
    /// A later call with the same name replaces the previous value. The
    /// request middleware always supplies the final `X-Request-Id` value.
    #[must_use]
    pub fn with_header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.headers
            .get_or_insert_with(Default::default)
            .insert(name, value);
        self
    }

    /// Adds a `Retry-After` response header using delta seconds.
    #[must_use]
    pub fn with_retry_after(self, seconds: u64) -> Self {
        self.with_header(
            RETRY_AFTER,
            HeaderValue::from_str(&seconds.to_string())
                .expect("unsigned integers are valid header values"),
        )
    }

    /// Returns a `400 bad_request` error with a safe message.
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    /// Returns a `400 validation_failed` error with structured field details.
    pub fn validation(details: BTreeMap<String, Value>) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "validation_failed",
            "The request is invalid",
        )
        .with_details(details)
    }

    /// Returns a validation error for one DTO field.
    ///
    /// This is the concise form of [`ApiError::validation`] for the common case
    /// where a product has one field-level semantic or bounds error.
    pub fn validation_field(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self::validation(BTreeMap::from([(
            field.into(),
            Value::String(message.into()),
        )]))
    }

    /// Returns a validation error containing multiple DTO field messages.
    ///
    /// Field names are sorted in the serialized details object. If an iterator
    /// contains the same field more than once, the last message wins.
    pub fn validation_fields<K, M, I>(fields: I) -> Self
    where
        K: Into<String>,
        M: Into<String>,
        I: IntoIterator<Item = (K, M)>,
    {
        Self::validation(
            fields
                .into_iter()
                .map(|(field, message)| (field.into(), Value::String(message.into())))
                .collect(),
        )
    }

    /// Returns a `401 unauthorized` error with a safe message.
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }

    /// Returns the standard `401 unauthenticated` authentication error.
    pub fn unauthenticated() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "Authentication is required",
        )
    }

    /// Returns a `403 forbidden` error with a safe message.
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", message)
    }

    /// Returns the standard `403 permission_denied` authorization error.
    pub fn permission_denied() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "permission_denied",
            "Permission denied",
        )
    }

    /// Returns a `404 not_found` error with a safe message.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }

    /// Returns a `409 conflict` error with a safe message.
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", message)
    }

    /// Returns a `405 method_not_allowed` error with a stable safe message.
    pub fn method_not_allowed() -> Self {
        Self::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
            "Method not allowed",
        )
    }

    /// Returns the standard `429 rate_limited` error.
    pub fn rate_limited() -> Self {
        Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Too many requests",
        )
    }

    /// Returns an opaque `500 internal` error and retains its cause for logging.
    pub fn internal(error: impl StdError + Send + Sync + 'static) -> Self {
        let mut api_error = Self::internal_without_cause();
        api_error.cause = Some(Box::new(error));
        api_error
    }

    pub(crate) fn timeout() -> Self {
        Self::new(
            StatusCode::GATEWAY_TIMEOUT,
            "request_timeout",
            "The request timed out",
        )
    }

    pub(crate) fn payload_too_large() -> Self {
        Self::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload_too_large",
            "The request body is too large",
        )
    }

    pub(crate) fn json_rejection(code: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, "The request is invalid").with_details(
            BTreeMap::from([(
                "body".to_owned(),
                Value::String("must be valid JSON matching the request schema".to_owned()),
            )]),
        )
    }

    pub(crate) fn query_rejection() -> Self {
        Self::validation_field("query", "must contain valid query parameters")
    }

    pub(crate) fn internal_without_cause() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "An internal error occurred",
        )
    }

    /// Returns this error's HTTP status.
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        self.status
    }

    /// Returns this error's stable machine-readable code.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }
}

impl From<JsonRejection> for ApiError {
    fn from(_rejection: JsonRejection) -> Self {
        Self::json_rejection("validation_failed")
    }
}

impl From<PathRejection> for ApiError {
    fn from(_rejection: PathRejection) -> Self {
        Self::validation(BTreeMap::from([(
            "path".to_owned(),
            Value::String("must contain valid route parameters".to_owned()),
        )]))
    }
}

impl From<QueryRejection> for ApiError {
    fn from(_rejection: QueryRejection) -> Self {
        Self::query_rejection()
    }
}

pub(crate) fn is_valid_error_code(code: &str) -> bool {
    !code.is_empty()
        && !code.starts_with('_')
        && !code.ends_with('_')
        && !code.contains("__")
        && code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

impl fmt::Debug for ApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApiError")
            .field("status", &self.status)
            .field("code", &self.code)
            .field("message", &self.message)
            .field("details", &self.details)
            .field("headers", &self.headers)
            .field("has_cause", &self.cause.is_some())
            .finish()
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} ({})", self.message, self.code)
    }
}

impl StdError for ApiError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.cause
            .as_deref()
            .map(|cause| cause as &(dyn StdError + 'static))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let request_id = current_request_id();
        if let Some(cause) = self.cause.as_deref() {
            tracing::error!(error = %cause, "request failed with an internal error");
        }

        let envelope = ErrorEnvelope {
            error: ErrorBody {
                code: self.code,
                message: self.message,
                request_id: request_id.to_string(),
                details: self.details,
            },
        };
        let mut response = (self.status, Json(envelope)).into_response();
        if let Some(headers) = self.headers {
            response.headers_mut().extend(*headers);
        }
        response.headers_mut().insert(
            crate::X_REQUEST_ID,
            request_id
                .header_value()
                .expect("request IDs originate from a valid header or UUID"),
        );
        response
    }
}
