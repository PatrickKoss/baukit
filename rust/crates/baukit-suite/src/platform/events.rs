use baukit_events::{EventEnvelope, validate_event_envelope};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::PlatformError;
use crate::domain::{PayloadCatalog, PayloadError};

/// Required by the immutable v1 hub wire contract.
pub const PLATFORM_SOURCE_APP: &str = "schluesselbund";
pub const PLATFORM_CATALOG_JSON: &str = include_str!("catalog.json");

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
    #[serde(rename = "schluesselbund.entitlements.changed")]
    EntitlementsChanged(EntitlementsChanged),
    #[serde(rename = "schluesselbund.account.updated")]
    AccountUpdated(SubjectRevision),
    #[serde(rename = "schluesselbund.account.erasure_requested")]
    ErasureRequested(ErasureRequested),
    #[serde(rename = "schluesselbund.preferences.changed")]
    PreferencesChanged(SubjectRevision),
    #[serde(rename = "schluesselbund.onboarding.shared")]
    OnboardingShared(OnboardingShared),
    #[serde(rename = "schluesselbund.family.membership_changed")]
    FamilyMembershipChanged(FamilyMembershipChanged),
    #[serde(rename = "schluesselbund.organization.membership_changed")]
    OrganizationMembershipChanged(OrganizationMembershipChanged),
    #[serde(rename = "schluesselbund.seat.changed")]
    SeatChanged(SeatChanged),
    #[serde(other)]
    Unknown,
}

/// Validated envelope and typed payload. Unknown types retain their raw envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "EventEnvelope", into = "EventEnvelope")]
pub struct PlatformEvent {
    envelope: EventEnvelope,
    kind: PlatformEventKind,
}
impl PlatformEvent {
    pub fn decode(body: &[u8]) -> Result<Self, PlatformError> {
        let envelope: EventEnvelope =
            serde_json::from_slice(body).map_err(|_| PlatformError::Invalid)?;
        Self::try_from(envelope)
    }
    pub fn envelope(&self) -> &EventEnvelope {
        &self.envelope
    }
    pub fn kind(&self) -> &PlatformEventKind {
        &self.kind
    }
}
impl TryFrom<EventEnvelope> for PlatformEvent {
    type Error = PlatformError;
    fn try_from(envelope: EventEnvelope) -> Result<Self, Self::Error> {
        // Hub deliveries can retry for longer than the suite's seven-day event age.
        validate_event_envelope(&envelope, &envelope.user_id, envelope.occurred_at)
            .map_err(|_| PlatformError::Invalid)?;
        if envelope.source_app != PLATFORM_SOURCE_APP {
            return Err(PlatformError::Invalid);
        }
        let catalog =
            PayloadCatalog::from_json(PLATFORM_CATALOG_JSON).map_err(|_| PlatformError::Invalid)?;
        let kind = match catalog.validate(&envelope.event_type, envelope.payload.clone()) {
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
                serde_json::from_value(serde_json::json!({
                    "type": envelope.event_type, "payload": envelope.payload
                }))
                .map_err(|_| PlatformError::Invalid)?
            }
        };
        Ok(Self { envelope, kind })
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
