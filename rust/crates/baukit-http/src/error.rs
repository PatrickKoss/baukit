use std::{collections::BTreeMap, error::Error as StdError, fmt};

use axum::{
    Json,
    extract::rejection::{JsonRejection, PathRejection, QueryRejection},
    http::StatusCode,
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
            is_snake_case(&code),
            "API error codes must be non-empty snake_case identifiers"
        );
        Self {
            status,
            code,
            message: message.into(),
            details: BTreeMap::new(),
            cause: None,
        }
    }

    /// Replaces the structured details object.
    #[must_use]
    pub fn with_details(mut self, details: BTreeMap<String, Value>) -> Self {
        self.details = details;
        self
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

    /// Returns a `401 unauthorized` error with a safe message.
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }

    /// Returns a `403 forbidden` error with a safe message.
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", message)
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
        Self::validation(BTreeMap::from([(
            "body".to_owned(),
            Value::String("must be valid JSON matching the request schema".to_owned()),
        )]))
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
        Self::validation(BTreeMap::from([(
            "query".to_owned(),
            Value::String("must contain valid query parameters".to_owned()),
        )]))
    }
}

fn is_snake_case(code: &str) -> bool {
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
        response.headers_mut().insert(
            crate::X_REQUEST_ID,
            request_id
                .header_value()
                .expect("request IDs originate from a valid header or UUID"),
        );
        response
    }
}
