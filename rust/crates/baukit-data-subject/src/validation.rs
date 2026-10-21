use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::{DataSubjectRequest, Limits, codes};

/// A safe validation failure. No variant retains the rejected input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ValidationError {
    /// Reference is not 16 to 128 visible ASCII bytes.
    #[error("invalid data subject reference")]
    SubjectReference,
    /// Request contains no identifiers or exceeds its advertised count.
    #[error("invalid data subject identifier count")]
    IdentifierCount,
    /// Raw or normalized identifier exceeds the advertised byte limit.
    #[error("data subject identifier is too long")]
    IdentifierLength,
    /// Identifier is empty after normalization.
    #[error("data subject identifier is empty")]
    IdentifierEmpty,
    /// Phone is not a plus followed by 8 to 15 ASCII digits.
    #[error("invalid data subject phone identifier")]
    Phone,
    /// Type is neither a built-in nor a valid custom type.
    #[error("invalid data subject identifier type")]
    IdentifierType,
    /// Type is valid but is absent from discovery.
    #[error("unsupported data subject identifier type")]
    UnsupportedIdentifierType,
    /// Idempotency key is missing or is not 16 to 128 visible ASCII bytes.
    #[error("invalid data subject idempotency key")]
    IdempotencyKey,
}

impl ValidationError {
    /// Stable HTTP error code for an adapter's error envelope.
    pub const fn code(self) -> &'static str {
        match self {
            Self::IdempotencyKey => codes::IDEMPOTENCY_KEY_INVALID,
            Self::UnsupportedIdentifierType => codes::IDENTIFIER_TYPE_UNSUPPORTED,
            _ => codes::REQUEST_INVALID,
        }
    }

    /// HTTP status as a number, without an HTTP framework dependency.
    pub const fn status_code(self) -> u16 {
        match self {
            Self::IdempotencyKey => 400,
            _ => 422,
        }
    }
}

/// Validated custom type, matching `x-[a-z0-9]+(-[a-z0-9]+)*`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CustomIdentifierType(String);

impl CustomIdentifierType {
    /// Parses a complete custom type, including its `x-` prefix.
    pub fn new(value: &str) -> Result<Self, ValidationError> {
        let suffix = value
            .strip_prefix("x-")
            .ok_or(ValidationError::IdentifierType)?;
        if suffix.split('-').any(|segment| {
            segment.is_empty()
                || !segment
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        }) {
            return Err(ValidationError::IdentifierType);
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the complete wire name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Built-in identifier type or a validated product-defined `x-` type.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(try_from = "String", into = "String")]
pub enum IdentifierType {
    /// Email, trimmed and lowercased before matching.
    Email,
    /// Exact product account identifier after trimming.
    AccountId,
    /// Exact product customer identifier after trimming.
    CustomerId,
    /// E.164 phone, a plus and 8 to 15 ASCII digits.
    Phone,
    /// Product-defined exact identifier after trimming.
    Custom(CustomIdentifierType),
}

impl IdentifierType {
    /// Returns the camelCase built-in name or the custom wire name.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Email => "email",
            Self::AccountId => "accountId",
            Self::CustomerId => "customerId",
            Self::Phone => "phone",
            Self::Custom(custom) => custom.as_str(),
        }
    }
}

impl FromStr for IdentifierType {
    type Err = ValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "email" => Ok(Self::Email),
            "accountId" => Ok(Self::AccountId),
            "customerId" => Ok(Self::CustomerId),
            "phone" => Ok(Self::Phone),
            _ => CustomIdentifierType::new(value).map(Self::Custom),
        }
    }
}

impl TryFrom<String> for IdentifierType {
    type Error = ValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<IdentifierType> for String {
    fn from(value: IdentifierType) -> Self {
        value.as_str().to_owned()
    }
}

/// Normalized transient lookup input. Constructed only by [`normalize_request`].
#[derive(Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedIdentifier {
    #[serde(rename = "type")]
    identifier_type: IdentifierType,
    value: String,
}

impl NormalizedIdentifier {
    /// Type that passed the advertised allowlist.
    pub fn identifier_type(&self) -> &IdentifierType {
        &self.identifier_type
    }

    /// Normalized value. Do not persist or log it.
    pub fn value(&self) -> &str {
        &self.value
    }
}

impl fmt::Debug for NormalizedIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NormalizedIdentifier")
            .field("identifier_type", &self.identifier_type)
            .finish_non_exhaustive()
    }
}

/// Validated request, serialized in stable field and identifier-array order for HMAC input.
/// Never persist its serialized bytes; persist only their keyed hash.
#[derive(Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedRequest {
    subject_reference: String,
    identifiers: Vec<NormalizedIdentifier>,
}

impl NormalizedRequest {
    /// Client's validated opaque reference.
    pub fn subject_reference(&self) -> &str {
        &self.subject_reference
    }

    /// Transient normalized inputs for product matching.
    pub fn identifiers(&self) -> &[NormalizedIdentifier] {
        &self.identifiers
    }
}

impl fmt::Debug for NormalizedRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NormalizedRequest { .. }")
    }
}

fn visible_reference(value: &str) -> bool {
    (16..=128).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_graphic())
}

/// Checks the opaque reference without trimming or changing it.
pub fn validate_subject_reference(value: &str) -> Result<(), ValidationError> {
    if visible_reference(value) {
        Ok(())
    } else {
        Err(ValidationError::SubjectReference)
    }
}

/// Validates one required header value without trimming or changing it.
/// HTTP adapters must also reject repeated `Idempotency-Key` headers.
pub fn validate_idempotency_key(value: Option<&str>) -> Result<(), ValidationError> {
    if value.is_some_and(visible_reference) {
        Ok(())
    } else {
        Err(ValidationError::IdempotencyKey)
    }
}

/// Validates bounds and advertised types, then normalizes transient lookup values.
/// Array order and duplicates are preserved when comparing normalized request bodies.
pub fn normalize_request(
    request: &DataSubjectRequest,
    limits: &Limits,
    supported_types: &[IdentifierType],
) -> Result<NormalizedRequest, ValidationError> {
    validate_subject_reference(&request.subject_reference)?;
    if request.identifiers.is_empty() || request.identifiers.len() > limits.max_identifiers {
        return Err(ValidationError::IdentifierCount);
    }
    let identifiers = request
        .identifiers
        .iter()
        .map(|identifier| {
            if !supported_types.contains(&identifier.identifier_type) {
                return Err(ValidationError::UnsupportedIdentifierType);
            }
            if identifier.value.len() > limits.max_identifier_length {
                return Err(ValidationError::IdentifierLength);
            }
            let value = match identifier.identifier_type {
                IdentifierType::Email => identifier.value.trim().to_lowercase(),
                IdentifierType::Phone => {
                    let digits = identifier
                        .value
                        .strip_prefix('+')
                        .ok_or(ValidationError::Phone)?;
                    if !(8..=15).contains(&digits.len())
                        || !digits.bytes().all(|b| b.is_ascii_digit())
                    {
                        return Err(ValidationError::Phone);
                    }
                    identifier.value.clone()
                }
                _ => identifier.value.trim().to_owned(),
            };
            if value.is_empty() {
                return Err(ValidationError::IdentifierEmpty);
            }
            if value.len() > limits.max_identifier_length {
                return Err(ValidationError::IdentifierLength);
            }
            Ok(NormalizedIdentifier {
                identifier_type: identifier.identifier_type.clone(),
                value,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(NormalizedRequest {
        subject_reference: request.subject_reference.clone(),
        identifiers,
    })
}
