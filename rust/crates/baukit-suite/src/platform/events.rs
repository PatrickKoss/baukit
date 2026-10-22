use baukit_events::{EventEnvelope, validate_event_envelope};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use uuid::Uuid;

use super::PlatformError;
use crate::domain::{PayloadCatalog, PayloadError};

/// Payload rules use a neutral prefix; the decoder supplies the configured prefix.
pub const PLATFORM_CATALOG_JSON: &str = include_str!("catalog.json");
pub const MAX_EVENT_FUTURE_SECONDS: i64 = 300;
const MAX_HUB_IDENTIFIER_BYTES: usize = 64;
static CATALOG: LazyLock<Result<PayloadCatalog, PlatformError>> = LazyLock::new(|| {
    PayloadCatalog::from_json(PLATFORM_CATALOG_JSON).map_err(|_| PlatformError::Invalid)
});

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubjectRevision {
    pub suite_subject: String,
    pub revision: u64,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntitlementsChanged {
    pub suite_subject: String,
    pub product: String,
    pub revision: u64,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ErasureRequested {
    pub suite_subject: String,
    pub erasure_id: Uuid,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OnboardingShared {
    pub suite_subject: String,
    pub profile_version: u64,
    pub revision: u64,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FamilyStatus {
    Active,
    Removed,
    Left,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FamilyRole {
    Organizer,
    Member,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MembershipStatus {
    Active,
    Suspended,
    Removed,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountRole {
    Owner,
    Admin,
    Billing,
    Member,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FamilyMembershipChanged {
    pub family_id: Uuid,
    pub suite_subject: String,
    pub status: FamilyStatus,
    pub family_role: FamilyRole,
    pub revision: u64,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrganizationMembershipChanged {
    pub organization_id: Uuid,
    pub suite_subject: String,
    pub status: MembershipStatus,
    pub account_role: AccountRole,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SeatChanged {
    pub organization_id: Uuid,
    pub product: String,
    pub suite_subject: String,
    pub assigned: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", content = "payload")]
pub enum PlatformEventKind {
    #[serde(rename = "entitlements.changed")]
    EntitlementsChanged(EntitlementsChanged),
    #[serde(rename = "account.updated")]
    AccountUpdated(SubjectRevision),
    #[serde(rename = "account.erasure_requested")]
    ErasureRequested(ErasureRequested),
    #[serde(rename = "preferences.changed")]
    PreferencesChanged(SubjectRevision),
    #[serde(rename = "onboarding.shared")]
    OnboardingShared(OnboardingShared),
    #[serde(rename = "family.membership_changed")]
    FamilyMembershipChanged(FamilyMembershipChanged),
    #[serde(rename = "organization.membership_changed")]
    OrganizationMembershipChanged(OrganizationMembershipChanged),
    #[serde(rename = "seat.changed")]
    SeatChanged(SeatChanged),
    #[serde(other)]
    Unknown,
}

/// Validated envelope and typed payload. Unknown types retain their raw envelope.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(into = "EventEnvelope")]
pub struct PlatformEvent {
    envelope: EventEnvelope,
    kind: PlatformEventKind,
}
impl PlatformEvent {
    pub fn envelope(&self) -> &EventEnvelope {
        &self.envelope
    }
    pub fn kind(&self) -> &PlatformEventKind {
        &self.kind
    }
}
/// A hub's source and event prefix are deployment settings, not payload rules.
#[derive(Clone, Debug)]
pub struct PlatformEventDecoder {
    source_app: String,
    event_type_prefix: String,
}
impl PlatformEventDecoder {
    pub fn new(
        source_app: String,
        event_type_prefix: Option<String>,
    ) -> Result<Self, PlatformError> {
        let event_type_prefix = event_type_prefix.unwrap_or_else(|| source_app.clone());
        if source_app.is_empty()
            || source_app.trim() != source_app
            || source_app.chars().any(char::is_control)
            || source_app.len() > MAX_HUB_IDENTIFIER_BYTES
            || event_type_prefix.is_empty()
            || event_type_prefix.len() > MAX_HUB_IDENTIFIER_BYTES
            || !event_type_prefix.as_bytes()[0].is_ascii_lowercase()
            || !event_type_prefix
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(PlatformError::Invalid);
        }
        Ok(Self {
            source_app,
            event_type_prefix,
        })
    }
    pub fn source_app(&self) -> &str {
        &self.source_app
    }
    pub fn decode(&self, body: &[u8], now: DateTime<Utc>) -> Result<PlatformEvent, PlatformError> {
        let envelope: EventEnvelope =
            serde_json::from_slice(body).map_err(|_| PlatformError::Invalid)?;
        // Deliveries may retry beyond the suite's seven-day event age.
        validate_event_envelope(&envelope, &envelope.user_id, envelope.occurred_at)
            .map_err(|_| PlatformError::Invalid)?;
        if envelope.source_app != self.source_app
            || envelope.occurred_at.signed_duration_since(now)
                > chrono::TimeDelta::seconds(MAX_EVENT_FUTURE_SECONDS)
        {
            return Err(PlatformError::Invalid);
        }
        let suffix = envelope
            .event_type
            .strip_prefix(&self.event_type_prefix)
            .and_then(|suffix| suffix.strip_prefix('.'));
        let catalog = CATALOG.as_ref().map_err(|error| *error)?;
        let kind = match suffix {
            None => PlatformEventKind::Unknown,
            Some(suffix) => {
                match catalog.validate(&format!("platform.{suffix}"), envelope.payload.clone()) {
                    Err(PayloadError::UnsupportedType(_)) => PlatformEventKind::Unknown,
                    Err(_) => return Err(PlatformError::Invalid),
                    Ok(_) => {
                        if envelope
                            .payload
                            .get("suiteSubject")
                            .and_then(|v| v.as_str())
                            != Some(envelope.user_id.as_str())
                        {
                            return Err(PlatformError::Invalid);
                        }
                        serde_json::from_value(
                            serde_json::json!({"type": suffix, "payload": envelope.payload}),
                        )
                        .map_err(|_| PlatformError::Invalid)?
                    }
                }
            }
        };
        Ok(PlatformEvent { envelope, kind })
    }
}
impl From<PlatformEvent> for EventEnvelope {
    fn from(event: PlatformEvent) -> Self {
        event.envelope
    }
}

/// A clock adapter avoids process-global test clocks.
pub trait PlatformClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemPlatformClock;
impl PlatformClock for SystemPlatformClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}
