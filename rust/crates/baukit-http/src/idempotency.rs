use std::{collections::BTreeMap, error::Error as StdError, fmt};

use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use serde_json::Value;

use crate::ApiError;

/// The `Idempotency-Key` request header.
pub const IDEMPOTENCY_KEY: HeaderName = HeaderName::from_static("idempotency-key");

/// The longest key any [`IdempotencyKeyRule`] may accept, in bytes.
pub const MAX_IDEMPOTENCY_KEY_BYTES: usize = 255;

/// Error code for a route that requires `Idempotency-Key` and received none.
pub const IDEMPOTENCY_KEY_REQUIRED_CODE: &str = "idempotency_key_required";

/// Error code for an `Idempotency-Key` value outside the route's grammar.
pub const INVALID_IDEMPOTENCY_KEY_CODE: &str = "invalid_idempotency_key";

/// Error code for a key already used in the same scope with a different request.
pub const IDEMPOTENCY_KEY_REUSED_CODE: &str = "idempotency_key_reused";

/// Error code for a key whose first request has not finished.
pub const IDEMPOTENCY_KEY_IN_PROGRESS_CODE: &str = "idempotency_key_in_progress";

const REASON_DETAIL: &str = "reason";

/// Length bounds for `Idempotency-Key` values on one route.
///
/// A key is `min_bytes..=max_bytes` bytes of visible ASCII (`0x21..=0x7E`). The key is opaque:
/// quotes are part of the key, and nothing is trimmed beyond what the HTTP stack removes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IdempotencyKeyRule {
    min_bytes: usize,
    max_bytes: usize,
}

impl IdempotencyKeyRule {
    /// Creates a rule for keys of `min_bytes..=max_bytes` bytes.
    ///
    /// # Panics
    ///
    /// Panics unless `1 <= min_bytes <= max_bytes <= MAX_IDEMPOTENCY_KEY_BYTES`. In a `const`
    /// item the panic is a compile error. Use [`IdempotencyKeyRule::try_new`] for bounds read at
    /// runtime.
    #[must_use]
    pub const fn new(min_bytes: usize, max_bytes: usize) -> Self {
        assert!(
            are_valid_bounds(min_bytes, max_bytes),
            "invalid idempotency key bounds"
        );
        Self {
            min_bytes,
            max_bytes,
        }
    }

    /// Creates a rule from bounds read at runtime.
    pub const fn try_new(
        min_bytes: usize,
        max_bytes: usize,
    ) -> Result<Self, InvalidIdempotencyKeyRule> {
        if are_valid_bounds(min_bytes, max_bytes) {
            Ok(Self {
                min_bytes,
                max_bytes,
            })
        } else {
            Err(InvalidIdempotencyKeyRule)
        }
    }

    /// Returns the shortest accepted key, in bytes.
    #[must_use]
    pub const fn min_bytes(&self) -> usize {
        self.min_bytes
    }

    /// Returns the longest accepted key, in bytes.
    #[must_use]
    pub const fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    /// Parses one `Idempotency-Key` field value.
    pub fn parse<'h>(
        &self,
        value: &'h HeaderValue,
    ) -> Result<IdempotencyKey<'h>, InvalidIdempotencyKey> {
        let bytes = value.as_bytes();
        if bytes.is_empty() {
            return Err(InvalidIdempotencyKey::Empty);
        }
        if !bytes.iter().all(u8::is_ascii_graphic) {
            return Err(InvalidIdempotencyKey::InvalidCharacter);
        }
        if bytes.len() < self.min_bytes {
            return Err(InvalidIdempotencyKey::TooShort);
        }
        if bytes.len() > self.max_bytes {
            return Err(InvalidIdempotencyKey::TooLong);
        }
        std::str::from_utf8(bytes)
            .map(IdempotencyKey)
            .map_err(|_| InvalidIdempotencyKey::InvalidCharacter)
    }

    /// Reads the key from a route that requires `Idempotency-Key`.
    ///
    /// A missing header returns [`IdempotencyError::Required`].
    pub fn required<'h>(
        &self,
        headers: &'h HeaderMap,
    ) -> Result<IdempotencyKey<'h>, IdempotencyError> {
        self.optional(headers)?.ok_or(IdempotencyError::Required)
    }

    /// Reads the key from a route where `Idempotency-Key` is optional.
    ///
    /// A missing header returns `Ok(None)`, which runs the mutation without a replay record.
    pub fn optional<'h>(
        &self,
        headers: &'h HeaderMap,
    ) -> Result<Option<IdempotencyKey<'h>>, IdempotencyError> {
        let mut values = headers.get_all(IDEMPOTENCY_KEY).iter();
        let Some(value) = values.next() else {
            return Ok(None);
        };
        if values.next().is_some() {
            return Err(InvalidIdempotencyKey::RepeatedHeader.into());
        }
        Ok(Some(self.parse(value)?))
    }
}

/// Bounds outside `1 <= min_bytes <= max_bytes <= MAX_IDEMPOTENCY_KEY_BYTES`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidIdempotencyKeyRule;

impl fmt::Display for InvalidIdempotencyKeyRule {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "idempotency key bounds must satisfy 1 <= min <= max <= {MAX_IDEMPOTENCY_KEY_BYTES}"
        )
    }
}

impl StdError for InvalidIdempotencyKeyRule {}

/// A validated `Idempotency-Key` value.
///
/// `Debug` hides the value so the key does not reach logs through a derived `Debug`.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct IdempotencyKey<'h>(&'h str);

impl<'h> IdempotencyKey<'h> {
    /// Returns the key as sent.
    #[must_use]
    pub const fn as_str(&self) -> &'h str {
        self.0
    }
}

impl fmt::Debug for IdempotencyKey<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("IdempotencyKey(..)")
    }
}

/// Why an `Idempotency-Key` value is outside the route's grammar.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidIdempotencyKey {
    /// The request carries more than one `Idempotency-Key` header.
    RepeatedHeader,
    /// The header value is empty.
    Empty,
    /// The value contains a byte outside visible ASCII.
    InvalidCharacter,
    /// The value is shorter than the rule's minimum.
    TooShort,
    /// The value is longer than the rule's maximum.
    TooLong,
}

impl InvalidIdempotencyKey {
    /// Returns the stable snake_case reason sent in the error `details.reason`.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::RepeatedHeader => "repeated_header",
            Self::Empty => "empty",
            Self::InvalidCharacter => "invalid_character",
            Self::TooShort => "too_short",
            Self::TooLong => "too_long",
        }
    }
}

impl fmt::Display for InvalidIdempotencyKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid Idempotency-Key: {}", self.reason())
    }
}

impl StdError for InvalidIdempotencyKey {}

/// A keyed mutation that cannot run or replay.
///
/// Converts into [`ApiError`] as 400 `idempotency_key_required`, 400 `invalid_idempotency_key`,
/// 409 `idempotency_key_reused`, or 409 `idempotency_key_in_progress`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdempotencyError {
    /// A route that requires `Idempotency-Key` received none.
    Required,
    /// The `Idempotency-Key` value is outside the route's grammar.
    Invalid(InvalidIdempotencyKey),
    /// The key was used in the same scope with a different request fingerprint.
    Reused,
    /// The first request with this key has not finished, and the product chose not to wait.
    InProgress,
}

impl IdempotencyError {
    /// Returns the HTTP status for this error.
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::Required | Self::Invalid(_) => StatusCode::BAD_REQUEST,
            Self::Reused | Self::InProgress => StatusCode::CONFLICT,
        }
    }

    /// Returns the stable error code for this error.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Required => IDEMPOTENCY_KEY_REQUIRED_CODE,
            Self::Invalid(_) => INVALID_IDEMPOTENCY_KEY_CODE,
            Self::Reused => IDEMPOTENCY_KEY_REUSED_CODE,
            Self::InProgress => IDEMPOTENCY_KEY_IN_PROGRESS_CODE,
        }
    }

    const fn message(&self) -> &'static str {
        match self {
            Self::Required => "Idempotency-Key is required",
            Self::Invalid(_) => "Idempotency-Key is not a valid key for this route",
            Self::Reused => "Idempotency-Key was already used with a different request",
            Self::InProgress => "A request with this Idempotency-Key is still running",
        }
    }

    fn details(&self) -> BTreeMap<String, Value> {
        match self {
            Self::Invalid(reason) => {
                BTreeMap::from([(REASON_DETAIL.to_owned(), Value::from(reason.reason()))])
            }
            Self::Required | Self::Reused | Self::InProgress => BTreeMap::new(),
        }
    }
}

impl From<InvalidIdempotencyKey> for IdempotencyError {
    fn from(error: InvalidIdempotencyKey) -> Self {
        Self::Invalid(error)
    }
}

impl fmt::Display for IdempotencyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} ({})", self.message(), self.code())
    }
}

impl StdError for IdempotencyError {}

impl From<IdempotencyError> for ApiError {
    fn from(error: IdempotencyError) -> Self {
        Self::new(error.status(), error.code(), error.message()).with_details(error.details())
    }
}

const fn are_valid_bounds(min_bytes: usize, max_bytes: usize) -> bool {
    min_bytes >= 1 && min_bytes <= max_bytes && max_bytes <= MAX_IDEMPOTENCY_KEY_BYTES
}

#[cfg(test)]
mod tests;
