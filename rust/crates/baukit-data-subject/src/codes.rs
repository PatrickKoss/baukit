//! Stable HTTP error codes. Messages and diagnostics must not contain identifiers.

/// Missing or malformed `Idempotency-Key`, HTTP 400.
pub const IDEMPOTENCY_KEY_INVALID: &str = "data_subject_idempotency_key_invalid";
/// Invalid request fields or limits, HTTP 422.
pub const REQUEST_INVALID: &str = "data_subject_request_invalid";
/// An identifier type was not advertised, HTTP 422.
pub const IDENTIFIER_TYPE_UNSUPPORTED: &str = "data_subject_identifier_type_unsupported";
/// An idempotency key was reused for a different normalized body, HTTP 409.
pub const IDEMPOTENCY_CONFLICT: &str = "data_subject_idempotency_conflict";
/// Operation unknown, another kind, or owned by another client, HTTP 404.
pub const OPERATION_NOT_FOUND: &str = "data_subject_operation_not_found";
/// Export part requested before completion, HTTP 409.
pub const EXPORT_NOT_READY: &str = "data_subject_export_not_ready";
/// Export part past its retention, HTTP 410.
pub const EXPORT_EXPIRED: &str = "data_subject_export_expired";
/// Principal is not an allowed machine client, HTTP 403.
pub const CLIENT_NOT_ALLOWED: &str = "data_subject_client_not_allowed";
/// Principal lacks the operation scope, HTTP 403.
pub const INSUFFICIENT_SCOPE: &str = "insufficient_scope";
/// Accepted work exhausted its attempts, recorded in a failed operation resource.
pub const OPERATION_FAILED: &str = "data_subject_operation_failed";
