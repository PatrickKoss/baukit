use std::{collections::BTreeMap, error::Error as StdError, fmt};

use axum::http::{HeaderMap, HeaderValue, StatusCode, header::IF_MATCH};
use baukit_openapi::{INVALID_IF_MATCH_CODE, PRECONDITION_FAILED_CODE, PRECONDITION_REQUIRED_CODE};
use serde_json::Value;

use crate::ApiError;

/// The longest prefix a [`RevisionEtag`] accepts, in bytes.
pub const MAX_ETAG_PREFIX_BYTES: usize = 64;

const WEAK_INDICATOR: &str = "W/";
const QUOTE: u8 = b'"';
const LIST_SEPARATOR: u8 = b',';
const WILDCARD: &str = "*";
const REASON_DETAIL: &str = "reason";
const CURRENT_REVISION_DETAIL: &str = "currentRevision";

/// A non-negative resource revision carried in a strong ETag.
///
/// The range is `0..=i64::MAX`, so every revision fits a PostgreSQL `BIGINT` and converts to
/// `i64` without loss.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Revision(u64);

impl Revision {
    /// The largest revision, `i64::MAX`.
    pub const MAX: Self = Self(i64::MAX.unsigned_abs());

    /// Returns the revision as an unsigned integer.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Returns the revision as a signed integer for storage.
    #[must_use]
    pub const fn to_i64(self) -> i64 {
        self.0.cast_signed()
    }
}

impl From<u32> for Revision {
    fn from(value: u32) -> Self {
        Self(u64::from(value))
    }
}

impl TryFrom<u64> for Revision {
    type Error = RevisionOutOfRange;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if value > Self::MAX.0 {
            return Err(RevisionOutOfRange);
        }
        Ok(Self(value))
    }
}

impl TryFrom<i64> for Revision {
    type Error = RevisionOutOfRange;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        u64::try_from(value)
            .map(Self)
            .map_err(|_| RevisionOutOfRange)
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// A value outside `0..=i64::MAX` cannot be a [`Revision`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RevisionOutOfRange;

impl fmt::Display for RevisionOutOfRange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("revision must be between 0 and i64::MAX")
    }
}

impl StdError for RevisionOutOfRange {}

impl From<RevisionOutOfRange> for ApiError {
    fn from(error: RevisionOutOfRange) -> Self {
        Self::internal(error)
    }
}

/// An ETag prefix is longer than [`MAX_ETAG_PREFIX_BYTES`] or has a byte outside
/// `[A-Za-z0-9._:-]`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidEtagPrefix;

impl fmt::Display for InvalidEtagPrefix {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "ETag prefixes must be at most {MAX_ETAG_PREFIX_BYTES} bytes of ASCII letters, digits, \
             `.`, `_`, `:`, or `-`"
        )
    }
}

impl StdError for InvalidEtagPrefix {}

/// Formats and parses strong revision ETags of the form `"<prefix><revision>"`.
///
/// The prefix is compared byte for byte, so it is case-sensitive. The revision is canonical
/// decimal: no sign, no leading zeros, and `0` is allowed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RevisionEtag<'a> {
    prefix: &'a str,
}

impl<'a> RevisionEtag<'a> {
    /// Creates a codec for a fixed prefix, such as `"rev-"`.
    ///
    /// # Panics
    ///
    /// Panics when the prefix is invalid. In a `const` item the panic is a compile error. Use
    /// [`RevisionEtag::try_new`] for prefixes built at runtime.
    #[must_use]
    pub const fn new(prefix: &'a str) -> Self {
        assert!(is_valid_prefix(prefix), "invalid revision ETag prefix");
        Self { prefix }
    }

    /// Creates a codec for a prefix built at runtime.
    pub const fn try_new(prefix: &'a str) -> Result<Self, InvalidEtagPrefix> {
        if is_valid_prefix(prefix) {
            Ok(Self { prefix })
        } else {
            Err(InvalidEtagPrefix)
        }
    }

    /// Returns the prefix.
    #[must_use]
    pub const fn prefix(&self) -> &'a str {
        self.prefix
    }

    /// Returns the quoted ETag, for example `"rev-42"` including the quotes.
    #[must_use]
    pub fn format(&self, revision: Revision) -> String {
        format!("\"{}{revision}\"", self.prefix)
    }

    /// Returns the quoted ETag as an `ETag` response header value.
    #[must_use]
    pub fn header_value(&self, revision: Revision) -> HeaderValue {
        HeaderValue::from_str(&self.format(revision))
            .expect("a validated prefix and a decimal revision form a valid header value")
    }

    /// Parses one `If-Match` field value.
    pub fn parse(&self, value: &HeaderValue) -> Result<Revision, InvalidIfMatch> {
        let text = visible_ascii(value.as_bytes())?;
        let tag = text.trim_matches([' ', '\t']);
        if tag.is_empty() {
            return Err(InvalidIfMatch::Empty);
        }
        if tag.starts_with(WILDCARD) {
            return Err(InvalidIfMatch::Wildcard);
        }
        let (weak, opaque) = single_entity_tag(tag)?;
        if weak {
            return Err(InvalidIfMatch::WeakValidator);
        }
        let digits = opaque
            .strip_prefix(self.prefix)
            .ok_or(InvalidIfMatch::PrefixMismatch)?;
        parse_revision(digits)
    }

    /// Reads the revision from a route that requires `If-Match`.
    ///
    /// A missing header returns [`PreconditionError::Required`].
    pub fn required_if_match(&self, headers: &HeaderMap) -> Result<Revision, PreconditionError> {
        self.optional_if_match(headers)?
            .ok_or(PreconditionError::Required)
    }

    /// Reads the revision from a route where `If-Match` is optional.
    ///
    /// A missing header returns `Ok(None)`, which permits an unconditional write.
    pub fn optional_if_match(
        &self,
        headers: &HeaderMap,
    ) -> Result<Option<Revision>, PreconditionError> {
        let mut values = headers.get_all(IF_MATCH).iter();
        let Some(value) = values.next() else {
            return Ok(None);
        };
        if values.next().is_some() {
            return Err(InvalidIfMatch::RepeatedHeader.into());
        }
        Ok(Some(self.parse(value)?))
    }
}

/// Why an `If-Match` value is not one strong ETag for the resource.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidIfMatch {
    /// The request carries more than one `If-Match` header.
    RepeatedHeader,
    /// The header value is empty or only whitespace.
    Empty,
    /// The header value contains bytes outside visible ASCII, space, and tab.
    NonAscii,
    /// The header value is `*`.
    Wildcard,
    /// The ETag is weak (`W/"..."`).
    WeakValidator,
    /// The header value lists more than one ETag.
    List,
    /// The header value is not a quoted entity tag.
    Malformed,
    /// The ETag does not start with the route's prefix.
    PrefixMismatch,
    /// The revision is not canonical decimal digits.
    InvalidRevision,
    /// The revision is larger than [`Revision::MAX`].
    RevisionOutOfRange,
}

impl InvalidIfMatch {
    /// Returns the stable snake_case reason sent in the error `details.reason`.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::RepeatedHeader => "repeated_header",
            Self::Empty => "empty",
            Self::NonAscii => "non_ascii",
            Self::Wildcard => "wildcard",
            Self::WeakValidator => "weak_validator",
            Self::List => "list",
            Self::Malformed => "malformed",
            Self::PrefixMismatch => "prefix_mismatch",
            Self::InvalidRevision => "invalid_revision",
            Self::RevisionOutOfRange => "revision_out_of_range",
        }
    }
}

impl fmt::Display for InvalidIfMatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid If-Match: {}", self.reason())
    }
}

impl StdError for InvalidIfMatch {}

/// A failed revision precondition.
///
/// Converts into [`ApiError`] as 428 `precondition_required`, 400 `invalid_if_match`, or 412
/// `precondition_failed`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreconditionError {
    /// A route that requires `If-Match` received none.
    Required,
    /// The `If-Match` value is not one strong ETag for the resource.
    Invalid(InvalidIfMatch),
    /// The expected revision is not the stored one.
    Stale {
        /// The stored revision, when the product knows it.
        current: Option<Revision>,
    },
}

impl PreconditionError {
    /// Returns the HTTP status for this error.
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::Required => StatusCode::PRECONDITION_REQUIRED,
            Self::Invalid(_) => StatusCode::BAD_REQUEST,
            Self::Stale { .. } => StatusCode::PRECONDITION_FAILED,
        }
    }

    /// Returns the stable error code for this error.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Required => PRECONDITION_REQUIRED_CODE,
            Self::Invalid(_) => INVALID_IF_MATCH_CODE,
            Self::Stale { .. } => PRECONDITION_FAILED_CODE,
        }
    }

    const fn message(&self) -> &'static str {
        match self {
            Self::Required => "If-Match is required",
            Self::Invalid(_) => "If-Match must contain one strong ETag for this resource",
            Self::Stale { .. } => "The resource changed since it was read",
        }
    }

    fn details(&self) -> BTreeMap<String, Value> {
        match self {
            Self::Required | Self::Stale { current: None } => BTreeMap::new(),
            Self::Invalid(reason) => {
                BTreeMap::from([(REASON_DETAIL.to_owned(), Value::from(reason.reason()))])
            }
            Self::Stale {
                current: Some(current),
            } => BTreeMap::from([(
                CURRENT_REVISION_DETAIL.to_owned(),
                Value::from(current.get()),
            )]),
        }
    }
}

impl From<InvalidIfMatch> for PreconditionError {
    fn from(error: InvalidIfMatch) -> Self {
        Self::Invalid(error)
    }
}

impl fmt::Display for PreconditionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} ({})", self.message(), self.code())
    }
}

impl StdError for PreconditionError {}

impl From<PreconditionError> for ApiError {
    fn from(error: PreconditionError) -> Self {
        Self::new(error.status(), error.code(), error.message()).with_details(error.details())
    }
}

/// Compares the revision a client sent with the stored one.
///
/// A mismatch returns [`PreconditionError::Stale`] carrying `current`, which becomes 412
/// `precondition_failed` with `details.currentRevision`.
pub fn ensure_current_revision(
    expected: Revision,
    current: Revision,
) -> Result<(), PreconditionError> {
    if expected == current {
        return Ok(());
    }
    Err(PreconditionError::Stale {
        current: Some(current),
    })
}

const fn is_valid_prefix(prefix: &str) -> bool {
    let bytes = prefix.as_bytes();
    if bytes.len() > MAX_ETAG_PREFIX_BYTES {
        return false;
    }
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if !(byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')) {
            return false;
        }
        index += 1;
    }
    true
}

fn visible_ascii(bytes: &[u8]) -> Result<&str, InvalidIfMatch> {
    if !bytes
        .iter()
        .all(|byte| byte.is_ascii_graphic() || matches!(byte, b' ' | b'\t'))
    {
        return Err(InvalidIfMatch::NonAscii);
    }
    std::str::from_utf8(bytes).map_err(|_| InvalidIfMatch::NonAscii)
}

fn single_entity_tag(tag: &str) -> Result<(bool, &str), InvalidIfMatch> {
    let (weak, quoted) = tag
        .strip_prefix(WEAK_INDICATOR)
        .map_or((false, tag), |rest| (true, rest));
    let body = quoted
        .strip_prefix(char::from(QUOTE))
        .ok_or(InvalidIfMatch::Malformed)?;
    let end = body
        .bytes()
        .position(|byte| byte == QUOTE)
        .ok_or(InvalidIfMatch::Malformed)?;
    let (opaque, rest) = (&body[..end], &body[end + 1..]);
    let rest = rest.trim_start_matches([' ', '\t']);
    if rest.bytes().next() == Some(LIST_SEPARATOR) {
        return Err(InvalidIfMatch::List);
    }
    if !rest.is_empty() || opaque.contains([' ', '\t']) {
        return Err(InvalidIfMatch::Malformed);
    }
    Ok((weak, opaque))
}

fn parse_revision(digits: &str) -> Result<Revision, InvalidIfMatch> {
    let canonical = !digits.is_empty()
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'));
    if !canonical {
        return Err(InvalidIfMatch::InvalidRevision);
    }
    digits
        .parse::<u64>()
        .ok()
        .and_then(|value| Revision::try_from(value).ok())
        .ok_or(InvalidIfMatch::RevisionOutOfRange)
}

#[cfg(test)]
mod tests;
