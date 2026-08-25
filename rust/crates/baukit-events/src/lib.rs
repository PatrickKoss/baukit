//! Versioned event envelopes shared by products in a suite.

#![deny(missing_docs)]

#[doc = include_str!("../README.md")]
#[cfg(doctest)]
struct ReadmeDoctests;

use chrono::{DateTime, TimeDelta, Utc};
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

/// A product-to-product domain event.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EventEnvelope {
    /// Opaque idempotency key chosen by the sender.
    pub event_id: String,
    /// Event name in `<app>.<noun>.<verb>` form.
    #[serde(rename = "type")]
    pub event_type: String,
    /// Identity-service subject that owns the event.
    pub user_id: String,
    /// UTC instant at which the domain action occurred.
    pub occurred_at: DateTime<Utc>,
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

/// Stable response shape for event ingestion.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IngestOutcome {
    /// Ingestion result class.
    pub outcome: IngestOutcomeStatus,
    /// Product-owned ledger row created by a grant, when applicable.
    pub ledger_entry_id: Option<String>,
}

/// Validates context-dependent and stable envelope rules.
pub fn validate_event_envelope(
    envelope: &EventEnvelope,
    expected_user_id: &str,
    now: DateTime<Utc>,
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
    if now.signed_duration_since(envelope.occurred_at) > TimeDelta::seconds(MAX_EVENT_AGE_SECONDS) {
        return Err(EventValidationCode::EventTooOld);
    }
    Ok(())
}

fn valid_event_id(value: &str) -> bool {
    !value.is_empty() && value.trim() == value && value.chars().count() <= MAX_EVENT_ID_CHARACTERS
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
