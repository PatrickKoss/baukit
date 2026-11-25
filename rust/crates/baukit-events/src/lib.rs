//! Versioned event envelopes shared by products in a suite.

#![deny(missing_docs)]

#[doc = include_str!("../README.md")]
#[cfg(doctest)]
struct ReadmeDoctests;

use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The event-envelope schema understood by this package.
pub const EVENT_SCHEMA_VERSION: u32 = 1;
/// Maximum number of Unicode scalar values in an event ID.
pub const MAX_EVENT_ID_CHARACTERS: usize = 64;
/// Maximum number of characters in each event-type segment.
pub const MAX_EVENT_TYPE_SEGMENT_CHARACTERS: usize = 32;
/// Maximum age of an event that may affect current product state.
pub const MAX_EVENT_AGE_SECONDS: i64 = 7 * 24 * 60 * 60;
/// Maximum number of top-level payload keys.
pub const MAX_EVENT_PAYLOAD_KEYS: usize = 32;
/// Maximum number of characters in a payload key.
pub const MAX_EVENT_PAYLOAD_KEY_CHARACTERS: usize = 64;
/// Maximum number of characters in `sourceApp`.
pub const MAX_SOURCE_APP_CHARACTERS: usize = 32;
/// Maximum number of Unicode scalar values in `userId`.
pub const MAX_USER_ID_CHARACTERS: usize = 255;
/// How far `occurredAt` may run ahead of the ingesting clock.
pub const MAX_EVENT_CLOCK_SKEW_SECONDS: i64 = 5 * 60;
const MAX_FRACTION_DIGITS: usize = 3;

/// A product-to-product domain event.
///
/// Field names serialize as camelCase: `eventId`, `type`, `userId`, `occurredAt`,
/// `sourceApp`, `schemaVersion`, and `payload`.
///
/// Deserialization enforces the wire shape and fails on unknown fields, a
/// `userId` that is empty, padded or over [`MAX_USER_ID_CHARACTERS`], a
/// `sourceApp` outside `^[a-z][a-z0-9_-]{0,31}$`, an `occurredAt` that is not
/// `YYYY-MM-DDTHH:MM:SS[.fff]Z` naming a real instant, and payload keys outside
/// `^[a-z][a-zA-Z0-9]{0,63}$` or more than [`MAX_EVENT_PAYLOAD_KEYS`] of them.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", try_from = "RawEventEnvelope")]
pub struct EventEnvelope {
    /// Opaque idempotency key chosen by the sender.
    pub event_id: String,
    /// Event name in `<app>.<noun>.<verb>` form.
    #[serde(rename = "type")]
    pub event_type: String,
    /// Identity-service subject that owns the event.
    pub user_id: String,
    /// UTC instant at which the domain action occurred.
    pub occurred_at: Timestamp,
    /// Stable sender identifier.
    pub source_app: String,
    /// Envelope schema version.
    pub schema_version: u32,
    /// Event-specific JSON object.
    pub payload: Map<String, Value>,
}

/// Stable validation failures returned at event-ingestion boundaries.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventValidationCode {
    /// The idempotency key is empty, padded, or longer than the contract permits.
    EventIdInvalid,
    /// The event type does not have three bounded lower-snake-case segments.
    EventTypeInvalid,
    /// The envelope owner differs from the authenticated connection owner.
    EventUserMismatch,
    /// The event occurred more than seven days before ingestion.
    EventTooOld,
    /// The event claims to occur more than five minutes after ingestion.
    EventInFuture,
    /// The envelope schema is not supported.
    EventSchemaUnsupported,
}

impl EventValidationCode {
    /// Returns the wire-safe validation code.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EventIdInvalid => "event_id_invalid",
            Self::EventTypeInvalid => "event_type_invalid",
            Self::EventUserMismatch => "event_user_mismatch",
            Self::EventTooOld => "event_too_old",
            Self::EventInFuture => "event_in_future",
            Self::EventSchemaUnsupported => "event_schema_unsupported",
        }
    }
}

/// Result class persisted for one accepted or rejected event ID.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IngestOutcomeStatus {
    /// At least one grant was written.
    Granted,
    /// The event matched no enabled grant rule.
    NoRule,
    /// A configured cap removed the available grant.
    Capped,
    /// The event ID had already been ingested.
    Duplicate,
    /// Validation or authorization rejected the event.
    Rejected,
}

/// Stable response shape for event ingestion: `{ "outcome", "ledgerEntryId" }`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestOutcome {
    /// Ingestion result class.
    pub outcome: IngestOutcomeStatus,
    /// Product-owned ledger row created by a grant, when applicable.
    pub ledger_entry_id: Option<String>,
}

/// Validates context-dependent and stable envelope rules.
///
/// # Errors
///
/// Returns [`EventValidationCode::EventSchemaUnsupported`] for another schema
/// version, `EventIdInvalid` or `EventTypeInvalid` for a malformed id or type,
/// `EventUserMismatch` when the envelope names another user, `EventTooOld`
/// when `occurred_at` is older than the allowed age at `now`, and
/// `EventInFuture` when it is more than [`MAX_EVENT_CLOCK_SKEW_SECONDS`] ahead.
pub fn validate_event_envelope(
    envelope: &EventEnvelope,
    expected_user_id: &str,
    now: Timestamp,
) -> Result<(), EventValidationCode> {
    if envelope.schema_version != EVENT_SCHEMA_VERSION {
        return Err(EventValidationCode::EventSchemaUnsupported);
    }
    if !valid_event_id(&envelope.event_id) {
        return Err(EventValidationCode::EventIdInvalid);
    }
    if !valid_event_type(&envelope.event_type) {
        return Err(EventValidationCode::EventTypeInvalid);
    }
    if envelope.user_id != expected_user_id {
        return Err(EventValidationCode::EventUserMismatch);
    }
    if now.duration_since(envelope.occurred_at) > SignedDuration::from_secs(MAX_EVENT_AGE_SECONDS) {
        return Err(EventValidationCode::EventTooOld);
    }
    if envelope.occurred_at.duration_since(now)
        > SignedDuration::from_secs(MAX_EVENT_CLOCK_SKEW_SECONDS)
    {
        return Err(EventValidationCode::EventInFuture);
    }
    Ok(())
}

/// A wire-shape failure, reported by deserialization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventShapeError {
    /// `userId` is empty, padded or too long.
    UserId,
    /// `sourceApp` is outside its pattern.
    SourceApp,
    /// `occurredAt` is not a canonical UTC `Z` instant.
    OccurredAt,
    /// A payload key is outside its pattern, or there are too many.
    Payload,
}

impl std::fmt::Display for EventShapeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::UserId => "invalid userId",
            Self::SourceApp => "invalid sourceApp",
            Self::OccurredAt => "occurredAt must be YYYY-MM-DDTHH:MM:SS[.fff]Z",
            Self::Payload => "invalid payload keys",
        })
    }
}

impl std::error::Error for EventShapeError {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawEventEnvelope {
    event_id: String,
    #[serde(rename = "type")]
    event_type: String,
    user_id: String,
    occurred_at: String,
    source_app: String,
    schema_version: u32,
    payload: Map<String, Value>,
}

impl TryFrom<RawEventEnvelope> for EventEnvelope {
    type Error = EventShapeError;

    fn try_from(raw: RawEventEnvelope) -> Result<Self, Self::Error> {
        if !unpadded(&raw.user_id) || raw.user_id.chars().count() > MAX_USER_ID_CHARACTERS {
            return Err(EventShapeError::UserId);
        }
        if !valid_source_app(&raw.source_app) {
            return Err(EventShapeError::SourceApp);
        }
        if raw.payload.len() > MAX_EVENT_PAYLOAD_KEYS
            || !raw.payload.keys().all(|key| valid_payload_key(key))
        {
            return Err(EventShapeError::Payload);
        }
        Ok(Self {
            occurred_at: parse_utc_instant(&raw.occurred_at).ok_or(EventShapeError::OccurredAt)?,
            event_id: raw.event_id,
            event_type: raw.event_type,
            user_id: raw.user_id,
            source_app: raw.source_app,
            schema_version: raw.schema_version,
            payload: raw.payload,
        })
    }
}

/// JavaScript's `trim` also strips U+FEFF; NEL is stripped only by Rust's.
fn is_edge_space(character: char) -> bool {
    character.is_whitespace() || character == '\u{feff}'
}

fn unpadded(value: &str) -> bool {
    !value.is_empty() && value.trim_matches(is_edge_space) == value
}

fn valid_source_app(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.first().is_some_and(u8::is_ascii_lowercase)
        && bytes.len() <= MAX_SOURCE_APP_CHARACTERS
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

fn valid_payload_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    bytes.first().is_some_and(u8::is_ascii_lowercase)
        && bytes.len() <= MAX_EVENT_PAYLOAD_KEY_CHARACTERS
        && bytes.iter().all(u8::is_ascii_alphanumeric)
}

/// Accepts `YYYY-MM-DDTHH:MM:SS` with up to three fraction digits and `Z`,
/// which is the form both runtimes parse to the same millisecond.
fn parse_utc_instant(text: &str) -> Option<Timestamp> {
    const SHAPE: &[u8; 19] = b"dddd-dd-ddTdd:dd:dd";
    let bytes = text.as_bytes();
    let (head, tail) = (bytes.get(..SHAPE.len())?, bytes.get(SHAPE.len()..)?);
    let head_ok = head.iter().zip(SHAPE).all(|(byte, shape)| match shape {
        b'd' => byte.is_ascii_digit(),
        literal => byte == literal,
    });
    let tail_ok = match tail {
        [b'Z'] => true,
        [b'.', fraction @ .., b'Z'] => {
            (1..=MAX_FRACTION_DIGITS).contains(&fraction.len())
                && fraction.iter().all(u8::is_ascii_digit)
        }
        _ => false,
    };
    let second_ok = head.get(17).is_some_and(|tens| *tens <= b'5');
    if !head_ok || !tail_ok || !second_ok {
        return None;
    }
    text.parse().ok()
}

fn valid_event_id(value: &str) -> bool {
    unpadded(value) && value.chars().count() <= MAX_EVENT_ID_CHARACTERS
}

fn valid_event_type(value: &str) -> bool {
    let segments = value.split('.').collect::<Vec<_>>();
    segments.len() == 3 && segments.into_iter().all(valid_event_type_segment)
}

fn valid_event_type_segment(segment: &str) -> bool {
    let mut characters = segment.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    segment.chars().count() <= MAX_EVENT_TYPE_SEGMENT_CHARACTERS
        && first.is_ascii_lowercase()
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
}
