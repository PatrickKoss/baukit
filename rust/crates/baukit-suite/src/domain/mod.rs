use std::{collections::BTreeMap, net::IpAddr, time::Duration};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use baukit_events::{
    EVENT_SCHEMA_VERSION, EventEnvelope, EventValidationCode, validate_event_envelope,
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use url::{Host, Url};
use uuid::Uuid;

mod link_protocol;
pub use link_protocol::*;

mod payloads;
pub mod validation;
pub use payloads::*;

pub const SUITE_EVENT_NAMESPACE: Uuid = Uuid::from_u128(0x5f0e8c1a_3b7d_4c2e_9a61_2d8f4b7e0c95);
pub const SUITE_EVENTS_DELIVER_JOB_TYPE: &str = "suite.events.deliver";
pub const SUITE_LINKS_REVOKE_JOB_TYPE: &str = "suite.links.revoke";
pub const SUITE_QUEUE: &str = "suite-events";
pub const SUITE_CIRCUIT_FAILURES: u32 = 20;
pub const SUITE_MAX_ATTEMPTS: u32 = 10;
pub const SUITE_SIGNATURE_WINDOW_SECONDS: u64 = 300;
pub const SUITE_UNAVAILABLE_RETRY_SECONDS: u64 = 300;
pub const SUITE_RATE_WINDOW_SECONDS: u64 = 60;
pub const SUITE_EXCHANGE_FAILURE_LIMIT: u32 = 20;
pub const SUITE_MAX_PEER_ID_CHARACTERS: usize = 64;
pub const SUITE_DELIVERIES_PAGE_SIZE: u32 = 20;
pub const SUITE_INBOUND_UNKNOWN_LIMIT: u32 = 20;
pub const SUITE_INBOUND_LINK_LIMIT: u32 = 120;
pub const SUITE_ERASURE_REVOKE_TIMEOUT_MILLIS: u64 = 500;
pub const SUITE_INITIAL_REPLAY_DAYS: u32 = 90;
pub const SUITE_MAX_REPLAY_DAYS: u32 = 365;
pub const SUITE_REPLAY_INTERVAL_SECONDS: i64 = 86_400;
pub const SUITE_MAX_TEXT_CHARACTERS: usize = 128;
pub const SUITE_MAX_SUBJECT_BYTES: usize = 512;
pub const SUITE_MAX_BODY_BYTES: usize = 16_384;
pub const SUITE_TERMINAL_JOB_RETENTION_DAYS: i64 = 30;
pub const SUITE_REQUEST_LIFETIME_SECONDS: i64 = 10 * 60;
pub const SUITE_CODE_LIFETIME_SECONDS: i64 = 60;
pub const SUITE_CONNECTION_TEST_TYPE: &str = "suite.connection.tested";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncryptedPayload {
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
    pub key_version: i32,
}

macro_rules! string_enum {
    ($(#[$attr:meta])* $name:ident { $($(#[$variant_attr:meta])* $variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
        $(#[$attr])*
        pub enum $name { $($(#[$variant_attr])* #[serde(rename = $value)] $variant),+ }
        impl $name {
            pub const fn as_str(self) -> &'static str { match self { $(Self::$variant => $value),+ } }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { formatter.write_str(self.as_str()) }
        }
        impl std::str::FromStr for $name {
            type Err = SuiteDataError;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value { $($value => Ok(Self::$variant),)+ _ => Err(SuiteDataError::UnknownEnum { kind: stringify!($name), value: value.to_owned() }) }
            }
        }
    };
}

string_enum!(LinkStatus { Active => "active", NeedsAttention => "needs_attention", Revoked => "revoked" });
string_enum!(LinkRole { Initiator => "initiator", Authorizer => "authorizer" });
string_enum!(DeliveryHealth { Healthy => "healthy", Degraded => "degraded", NeedsAttention => "needs_attention", Disabled => "disabled" });
string_enum!(#[derive(Default)] RewardMode { #[default] Native => "native", SourceXp => "source_xp", Off => "off" });

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum SuiteDataError {
    #[error("unknown {kind} value: {value}")]
    UnknownEnum { kind: &'static str, value: String },
    #[error("invalid embedded peer registry: {0}")]
    InvalidRegistry(String),
    #[error("unknown suite peer: {0}")]
    UnknownPeer(String),
    #[error("invalid suite URL for {0}")]
    InvalidUrl(String),
    #[error("suite return URL is invalid")]
    ReturnUrlInvalid,
    #[error("suite PKCE verifier is invalid")]
    PkceInvalid,
    #[error("suite subject is invalid")]
    SubjectInvalid,
    #[error("suite accounts differ within the same identity domain")]
    AccountMismatch,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PeerMetadata {
    pub id: String,
    pub display_name: String,
    pub emits: Vec<String>,
    pub accepts: Vec<String>,
    pub reward_modes: Vec<RewardMode>,
    pub scheme: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PeerFile {
    schema_version: u32,
    peers: Vec<PeerMetadata>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default)]
pub struct PeerUrls {
    #[serde(deserialize_with = "crate::config::empty_string_as_none")]
    pub api_url: Option<String>,
    #[serde(deserialize_with = "crate::config::empty_string_as_none")]
    pub web_url: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PeerRegistrySettings {
    pub public_api_url: Option<String>,
    pub public_web_url: Option<String>,
    pub peers: BTreeMap<String, PeerUrls>,
    pub allow_loopback: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivePeer {
    pub metadata: PeerMetadata,
    pub api_url: String,
    pub web_url: String,
    pub sends: Vec<String>,
    pub receives: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct PeerRegistry {
    own: PeerMetadata,
    metadata: Vec<PeerMetadata>,
    public_api_url: Option<String>,
    public_web_url: Option<String>,
    active: Vec<ActivePeer>,
}

impl PeerRegistry {
    pub fn new(
        own_app: &str,
        peers_json: &str,
        settings: PeerRegistrySettings,
    ) -> Result<Self, SuiteDataError> {
        let file: PeerFile = serde_json::from_str(peers_json)
            .map_err(|error| SuiteDataError::InvalidRegistry(error.to_string()))?;
        if file.schema_version != 1 {
            return Err(SuiteDataError::InvalidRegistry(
                "unsupported schema".to_owned(),
            ));
        }
        validate_peer_metadata(&file.peers)?;
        let own = file
            .peers
            .iter()
            .find(|peer| peer.id == own_app)
            .cloned()
            .ok_or_else(|| SuiteDataError::UnknownPeer(own_app.to_owned()))?;
        let public_api_url = settings
            .public_api_url
            .as_deref()
            .map(|value| configured_url(value, settings.allow_loopback, false, own_app))
            .transpose()?;
        let public_web_url = settings
            .public_web_url
            .as_deref()
            .map(|value| configured_url(value, settings.allow_loopback, true, own_app))
            .transpose()?;
        let mut active = Vec::new();
        for (id, urls) in settings.peers {
            let peer = file
                .peers
                .iter()
                .find(|peer| peer.id == id && id != own_app)
                .ok_or_else(|| SuiteDataError::UnknownPeer(id.clone()))?;
            let api = urls
                .api_url
                .as_deref()
                .map(|value| configured_url(value, settings.allow_loopback, false, &id))
                .transpose()?;
            let web = urls
                .web_url
                .as_deref()
                .map(|value| configured_url(value, settings.allow_loopback, true, &id))
                .transpose()?;
            let sends = intersect(&own.emits, &peer.accepts);
            let receives = intersect(&peer.emits, &own.accepts);
            if let (Some(api_url), Some(web_url)) = (api, web)
                && (!sends.is_empty() || !receives.is_empty())
            {
                active.push(ActivePeer {
                    metadata: peer.clone(),
                    api_url,
                    web_url,
                    sends,
                    receives,
                });
            }
        }
        if public_api_url.is_none() || public_web_url.is_none() {
            active.clear();
        }
        Ok(Self {
            own,
            metadata: file.peers,
            public_api_url,
            public_web_url,
            active,
        })
    }

    pub fn standalone(&self) -> bool {
        self.active.is_empty()
    }
    pub fn own(&self) -> &PeerMetadata {
        &self.own
    }
    pub fn metadata(&self) -> &[PeerMetadata] {
        &self.metadata
    }
    /// Looks up embedded metadata for the own app or any peer, including inactive peers.
    pub fn peer_metadata(&self, id: &str) -> Option<&PeerMetadata> {
        self.metadata.iter().find(|peer| peer.id == id)
    }
    pub fn active_peers(&self) -> &[ActivePeer] {
        &self.active
    }
    pub fn active_peer(&self, id: &str) -> Option<&ActivePeer> {
        self.active.iter().find(|peer| peer.metadata.id == id)
    }
    pub fn public_api_url(&self) -> Option<&str> {
        self.public_api_url.as_deref()
    }
    pub fn public_web_url(&self) -> Option<&str> {
        self.public_web_url.as_deref()
    }
    pub fn validate_return_url(&self, value: &str) -> Result<(), SuiteDataError> {
        validate_return_url(
            value,
            self.public_web_url
                .as_deref()
                .ok_or(SuiteDataError::ReturnUrlInvalid)?,
            &self.own.scheme,
        )
    }
}

fn intersect(emits: &[String], accepts: &[String]) -> Vec<String> {
    emits
        .iter()
        .filter(|event_type| {
            event_type.as_str() != SUITE_CONNECTION_TEST_TYPE && accepts.contains(event_type)
        })
        .cloned()
        .collect()
}

fn configured_url(
    value: &str,
    allow_loopback: bool,
    origin_only: bool,
    peer: &str,
) -> Result<String, SuiteDataError> {
    let invalid = || SuiteDataError::InvalidUrl(peer.to_owned());
    let url = Url::parse(value).map_err(|_| invalid())?;
    let loopback = match url.host() {
        Some(Host::Domain("localhost")) => true,
        Some(Host::Ipv4(address)) => IpAddr::V4(address).is_loopback(),
        Some(Host::Ipv6(address)) => IpAddr::V6(address).is_loopback(),
        _ => false,
    };
    if url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || (origin_only && url.path() != "/")
        || !(url.scheme() == "https" || (url.scheme() == "http" && allow_loopback && loopback))
    {
        return Err(invalid());
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

pub fn validate_return_url(
    value: &str,
    own_web_origin: &str,
    own_scheme: &str,
) -> Result<(), SuiteDataError> {
    let invalid = || SuiteDataError::ReturnUrlInvalid;
    let origin = Url::parse(own_web_origin).map_err(|_| invalid())?;
    let url = Url::parse(value).map_err(|_| invalid())?;
    if !url.username().is_empty()
        || url.password().is_some()
        || !origin.username().is_empty()
        || origin.password().is_some()
        || origin.path() != "/"
        || origin.query().is_some()
        || origin.fragment().is_some()
    {
        return Err(invalid());
    }
    let web = matches!(url.scheme(), "https" | "http")
        && url.origin() == origin.origin()
        && url.path() == "/suite/linked";
    let native = url.scheme() == own_scheme
        && url.host_str() == Some("suite")
        && url.port().is_none()
        && url.path() == "/linked";
    if web || native {
        Ok(())
    } else {
        Err(invalid())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SuiteLink {
    pub id: Uuid,
    pub user_id: Uuid,
    pub peer_app: String,
    pub role: LinkRole,
    pub remote_link_id: Uuid,
    pub remote_subject: String,
    pub remote_display_name: Option<String>,
    pub suite_subject: Option<String>,
    pub status: LinkStatus,
    pub secret: EncryptedPayload,
    pub sends: Vec<String>,
    pub receives: Vec<String>,
    pub share_xp: bool,
    pub reward_mode: RewardMode,
    pub delivery_health: DeliveryHealth,
    pub consecutive_failures: u32,
    pub last_delivery_at: Option<DateTime<Utc>>,
    pub last_failure_at: Option<DateTime<Utc>>,
    pub last_failure_code: Option<String>,
    pub last_received_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl SuiteLink {
    pub fn can_enqueue(&self) -> bool {
        self.status == LinkStatus::Active
            && self.delivery_health != DeliveryHealth::Disabled
            && self.consecutive_failures < SUITE_CIRCUIT_FAILURES
    }
}

pub fn accepts_inbound_type(link: &SuiteLink, event_type: &str) -> bool {
    if event_type == SUITE_CONNECTION_TEST_TYPE {
        link.status == LinkStatus::Active
    } else {
        link.receives.iter().any(|accepted| accepted == event_type)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ValidatedInbound {
    Activity(ValidatedPayload),
    ConnectionTest,
}

#[derive(Clone, Debug, Error, PartialEq)]
pub enum InboundValidationError {
    #[error("invalid event envelope: {0:?}")]
    Envelope(EventValidationCode),
    #[error("event source app differs from the linked peer")]
    SourceAppMismatch,
    #[error("unsupported inbound event type")]
    UnsupportedType,
    #[error("event occurrence is in the future")]
    FutureOccurrence,
    #[error(transparent)]
    Payload(#[from] PayloadError),
}

impl InboundValidationError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Envelope(code) => code.as_str(),
            Self::SourceAppMismatch | Self::UnsupportedType => "suite_event_type_unsupported",
            Self::Payload(_) | Self::FutureOccurrence => "suite_payload_invalid",
        }
    }
}

pub fn validate_inbound(
    catalog: &PayloadCatalog,
    envelope: &EventEnvelope,
    link: &SuiteLink,
    now: DateTime<Utc>,
    replay: bool,
) -> Result<ValidatedInbound, InboundValidationError> {
    // Baukit's age check is neutralized for replay; every other check still runs.
    validate_event_envelope(
        envelope,
        &link.remote_subject,
        if replay { envelope.occurred_at } else { now },
    )
    .map_err(InboundValidationError::Envelope)?;
    if envelope.occurred_at > now + chrono::Duration::seconds(SUITE_SIGNATURE_WINDOW_SECONDS as i64)
    {
        return Err(InboundValidationError::FutureOccurrence);
    }
    if envelope.source_app != link.peer_app {
        return Err(InboundValidationError::SourceAppMismatch);
    }
    if !accepts_inbound_type(link, &envelope.event_type) {
        return Err(InboundValidationError::UnsupportedType);
    }
    if envelope.event_type == SUITE_CONNECTION_TEST_TYPE {
        validation::validate_fields(&envelope.payload, &[])?;
        return Ok(ValidatedInbound::ConnectionTest);
    }
    Ok(ValidatedInbound::Activity(catalog.validate(
        &envelope.event_type,
        envelope.payload.clone(),
    )?))
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum ReplayWindowError {
    #[error("replay date is in the future")]
    Future,
    #[error("replay date is more than 365 days old")]
    TooOld,
}

pub fn validate_replay_since(since: NaiveDate, today: NaiveDate) -> Result<(), ReplayWindowError> {
    let age = today.signed_duration_since(since).num_days();
    if age < 0 {
        Err(ReplayWindowError::Future)
    } else if age > i64::from(SUITE_MAX_REPLAY_DAYS) {
        Err(ReplayWindowError::TooOld)
    } else {
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SuiteEvent {
    pub event_type: String,
    pub natural_key: String,
    pub occurred_at: DateTime<Utc>,
    pub payload: Map<String, Value>,
}

impl SuiteEvent {
    pub fn event_id(&self) -> Uuid {
        Uuid::new_v5(
            &SUITE_EVENT_NAMESPACE,
            format!("{}:{}", self.event_type, self.natural_key).as_bytes(),
        )
    }
    pub fn envelope(&self, subject: &str, source_app: &str) -> EventEnvelope {
        EventEnvelope {
            event_id: self.event_id().to_string(),
            event_type: self.event_type.clone(),
            user_id: subject.to_owned(),
            occurred_at: self.occurred_at,
            source_app: source_app.to_owned(),
            schema_version: EVENT_SCHEMA_VERSION,
            payload: self.payload.clone(),
        }
    }
    pub fn envelope_for_link(
        &self,
        subject: &str,
        source_app: &str,
        share_xp: bool,
        link_share_xp: bool,
    ) -> EventEnvelope {
        let mut envelope = self.envelope(subject, source_app);
        if !share_xp || !link_share_xp {
            envelope.payload.remove("xp");
        }
        envelope
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SuiteDeliverJobV1 {
    #[serde(deserialize_with = "validation::integer::<_, u32, 1, 1>")]
    pub schema_version: u32,
    pub link_id: Uuid,
    pub envelope: EventEnvelope,
    pub replay: bool,
}

impl SuiteDeliverJobV1 {
    pub fn new(link_id: Uuid, envelope: EventEnvelope, replay: bool) -> Self {
        Self {
            schema_version: 1,
            link_id,
            envelope,
            replay,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SuiteRevokeJobV1 {
    #[serde(deserialize_with = "validation::integer::<_, u32, 1, 1>")]
    pub schema_version: u32,
    pub link_id: Uuid,
}

impl SuiteRevokeJobV1 {
    pub const fn new(link_id: Uuid) -> Self {
        Self {
            schema_version: 1,
            link_id,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedExchange {
    pub link_id: Uuid,
    pub secret: EncryptedPayload,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkRequest {
    pub id: Uuid,
    pub user_id: Uuid,
    pub peer_app: String,
    pub state_hash: [u8; 32],
    pub verifier: EncryptedPayload,
    pub exchange: Option<PreparedExchange>,
    pub client_state_nonce: String,
    pub return_url: String,
    pub link_id: Option<Uuid>,
    pub expires_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkCode {
    pub code_hash: [u8; 32],
    pub user_id: Uuid,
    pub peer_app: String,
    pub code_challenge: String,
    pub suite_subject: Option<String>,
    pub auto_approved: bool,
    pub link_id: Option<Uuid>,
    pub expires_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExchangeRequest {
    pub client: String,
    pub code: String,
    pub code_verifier: String,
    pub initiator_link_id: Uuid,
    pub link_secret: String,
    pub initiator_subject: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initiator_suite_subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initiator_display_name: Option<String>,
}

impl std::fmt::Debug for ExchangeRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExchangeRequest")
            .field("client", &self.client)
            .field("initiator_link_id", &self.initiator_link_id)
            .field("code", &"[redacted]")
            .field("code_verifier", &"[redacted]")
            .field("link_secret", &"[redacted]")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExchangeResponse {
    pub link_id: Uuid,
    pub subject: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suite_subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub authorizer_sends: Vec<String>,
    pub initiator_sends: Vec<String>,
}

pub fn signature_timestamp_valid(timestamp: i64, now: i64) -> bool {
    timestamp.abs_diff(now) <= SUITE_SIGNATURE_WINDOW_SECONDS
}

pub fn pkce_challenge(verifier: &str) -> Result<String, SuiteDataError> {
    if !(43..=SUITE_MAX_TEXT_CHARACTERS).contains(&verifier.len())
        || !verifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-._~".contains(&byte))
    {
        return Err(SuiteDataError::PkceInvalid);
    }
    Ok(URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())))
}

pub fn hint_for(suite_subject: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(
        format!("suite-link-hint-v1\n{suite_subject}").as_bytes(),
    ))
}

pub fn suite_subject(
    identity_domain: Option<&str>,
    identity_claim: Option<&str>,
    claims: &Map<String, Value>,
) -> Result<Option<String>, SuiteDataError> {
    let (Some(domain), Some(claim)) = (identity_domain, identity_claim) else {
        return Ok(None);
    };
    if domain.is_empty()
        || domain.trim() != domain
        || domain.contains('|')
        || domain.chars().any(char::is_control)
        || claim.is_empty()
    {
        return Err(SuiteDataError::SubjectInvalid);
    }
    let Some(value) = claims.get(claim) else {
        return Ok(None);
    };
    let value = value
        .as_str()
        .filter(|value| !value.is_empty() && !value.chars().any(char::is_control))
        .ok_or(SuiteDataError::SubjectInvalid)?;
    Ok(Some(format!("{domain}|{value}")))
}

pub fn shared_suite_subject(
    initiator: Option<&str>,
    authorizer: Option<&str>,
) -> Result<Option<String>, SuiteDataError> {
    let (Some(initiator), Some(authorizer)) = (initiator, authorizer) else {
        return Ok(None);
    };
    let (initiator_domain, initiator_id) = initiator
        .split_once('|')
        .ok_or(SuiteDataError::SubjectInvalid)?;
    let (authorizer_domain, authorizer_id) = authorizer
        .split_once('|')
        .ok_or(SuiteDataError::SubjectInvalid)?;
    if initiator_domain.is_empty()
        || authorizer_domain.is_empty()
        || initiator_id.is_empty()
        || authorizer_id.is_empty()
        || initiator.chars().any(char::is_control)
        || authorizer.chars().any(char::is_control)
        || initiator_domain.trim() != initiator_domain
        || authorizer_domain.trim() != authorizer_domain
    {
        return Err(SuiteDataError::SubjectInvalid);
    }
    if initiator_domain != authorizer_domain {
        return Ok(None);
    }
    if initiator != authorizer {
        return Err(SuiteDataError::AccountMismatch);
    }
    Ok(Some(initiator.to_owned()))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RewardSelection {
    Native,
    SourceXp(u32),
    Off,
}

pub fn select_reward(
    mode: RewardMode,
    xp: Option<u32>,
    event_type: &str,
    replay: bool,
) -> RewardSelection {
    if replay || event_type == SUITE_CONNECTION_TEST_TYPE {
        return RewardSelection::Off;
    }
    match (mode, xp) {
        (RewardMode::Off, _) => RewardSelection::Off,
        (RewardMode::SourceXp, Some(xp)) => RewardSelection::SourceXp(xp),
        _ => RewardSelection::Native,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryResult {
    Http {
        status: u16,
        retry_after: Option<Duration>,
    },
    Timeout,
    Transport,
    Dns,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryAction {
    LinkMissing,
    LinkRevoked,
    Disabled,
    Delivered,
    Retry { after: Option<Duration> },
    Unauthorized,
    Revoked,
    Rejected,
}

impl DeliveryAction {
    pub const fn outcome(self) -> Option<&'static str> {
        match self {
            Self::LinkMissing | Self::LinkRevoked => None,
            Self::Disabled => Some("disabled"),
            Self::Delivered => Some("delivered"),
            Self::Retry { .. } => Some("retry"),
            Self::Unauthorized => Some("unauthorized"),
            Self::Revoked => Some("revoked"),
            Self::Rejected => Some("rejected"),
        }
    }
    pub const fn permanent_code(self) -> Option<&'static str> {
        match self {
            Self::Unauthorized => Some("suite_unauthorized"),
            Self::Revoked => Some("suite_link_revoked"),
            Self::Rejected => Some("suite_rejected"),
            Self::Disabled => Some("suite_link_disabled"),
            _ => None,
        }
    }
}

pub fn delivery_preflight(link: Option<&SuiteLink>) -> Option<DeliveryAction> {
    match link {
        None => Some(DeliveryAction::LinkMissing),
        Some(link) if link.status == LinkStatus::Revoked => Some(DeliveryAction::LinkRevoked),
        Some(link)
            if link.delivery_health == DeliveryHealth::Disabled
                || link.consecutive_failures >= SUITE_CIRCUIT_FAILURES =>
        {
            Some(DeliveryAction::Disabled)
        }
        Some(_) => None,
    }
}

pub fn map_delivery_result(result: DeliveryResult) -> DeliveryAction {
    match result {
        DeliveryResult::Http {
            status: 200..=299, ..
        } => DeliveryAction::Delivered,
        DeliveryResult::Http {
            status: 401 | 403, ..
        } => DeliveryAction::Unauthorized,
        DeliveryResult::Http { status: 410, .. } => DeliveryAction::Revoked,
        DeliveryResult::Http {
            status: 408 | 425 | 429 | 500..=599,
            retry_after,
        } => DeliveryAction::Retry {
            after: retry_after.map(|delay| delay.min(Duration::from_secs(300))),
        },
        DeliveryResult::Timeout | DeliveryResult::Transport | DeliveryResult::Dns => {
            DeliveryAction::Retry { after: None }
        }
        _ => DeliveryAction::Rejected,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeliveryState {
    pub status: LinkStatus,
    pub health: DeliveryHealth,
    pub consecutive_failures: u32,
}

pub fn after_delivery(state: DeliveryState, action: DeliveryAction) -> DeliveryState {
    match action {
        DeliveryAction::Delivered if state.health == DeliveryHealth::Disabled => state,
        DeliveryAction::Delivered => DeliveryState {
            status: state.status,
            health: DeliveryHealth::Healthy,
            consecutive_failures: 0,
        },
        DeliveryAction::Unauthorized => DeliveryState {
            status: LinkStatus::NeedsAttention,
            health: DeliveryHealth::NeedsAttention,
            ..state
        },
        DeliveryAction::Revoked => DeliveryState {
            status: LinkStatus::Revoked,
            ..state
        },
        _ => state,
    }
}

fn validate_peer_metadata(peers: &[PeerMetadata]) -> Result<(), SuiteDataError> {
    let mut ids = std::collections::BTreeSet::new();
    for peer in peers {
        let mut bytes = peer.id.bytes();
        if peer.id.len() > SUITE_MAX_PEER_ID_CHARACTERS
            || !bytes.next().is_some_and(|b| b.is_ascii_lowercase())
            || !bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            || !ids.insert(&peer.id)
            || peer.scheme.is_empty()
            || !peer.scheme.bytes().enumerate().all(|(i, b)| {
                if i == 0 {
                    b.is_ascii_lowercase()
                } else {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || b"+-.".contains(&b)
                }
            })
        {
            return Err(SuiteDataError::InvalidRegistry(
                "invalid or duplicate peer metadata".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
