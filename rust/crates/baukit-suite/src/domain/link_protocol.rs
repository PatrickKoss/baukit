use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

use super::{
    ActivePeer, ExchangeRequest, LinkCode, LinkRequest, PeerRegistry, SuiteDataError, SuiteLink,
    hint_for, pkce_challenge, shared_suite_subject,
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LinkStartRequest {
    pub peer_app: String,
    pub return_url: String,
    pub state_nonce: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LinkStartResponse {
    pub request_id: Uuid,
    pub authorize_url: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LinkCompleteRequest {
    pub code: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthorizationPreview {
    pub peer: String,
    pub authorizer_sends: Vec<String>,
    pub initiator_sends: Vec<String>,
    pub existing_link: bool,
    pub auto_approve: bool,
    pub hint_mismatch: bool,
}

impl AuthorizationPreview {
    pub fn new(
        peer: &ActivePeer,
        existing_link: bool,
        hint: Option<&str>,
        suite_subject: Option<&str>,
        hint_domain: Option<&str>,
    ) -> Self {
        let auto_approve = hint
            .zip(suite_subject)
            .is_some_and(|(hint, subject)| hint == hint_for(subject));
        Self {
            peer: peer.metadata.display_name.clone(),
            authorizer_sends: peer.sends.clone(),
            initiator_sends: peer.receives.clone(),
            existing_link,
            auto_approve,
            hint_mismatch: hint.is_some()
                && !auto_approve
                && suite_subject
                    .and_then(|subject| subject.split_once('|').map(|(domain, _)| domain))
                    .zip(hint_domain)
                    .is_some_and(|(domain, hint_domain)| domain == hint_domain),
        }
    }
}

pub fn preview_authorization(
    registry: &PeerRegistry,
    client: &str,
    existing_link: bool,
    hint: Option<&str>,
    suite_subject: Option<&str>,
    hint_domain: Option<&str>,
) -> Result<AuthorizationPreview, LinkProtocolError> {
    let peer = registry
        .active_peer(client)
        .ok_or(LinkProtocolError::PeerUnknown)?;
    Ok(AuthorizationPreview::new(
        peer,
        existing_link,
        hint,
        suite_subject,
        hint_domain,
    ))
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum LinkProtocolError {
    #[error("suite request is not found")]
    NotFound,
    #[error("suite code or request is invalid")]
    CodeInvalid,
    #[error("suite accounts differ")]
    AccountMismatch,
    #[error("suite peer is unknown")]
    PeerUnknown,
    #[error("suite is disabled")]
    Disabled,
    #[error("suite return URL is invalid")]
    ReturnUrlInvalid,
    #[error("suite link payload is invalid")]
    PayloadInvalid,
}

impl LinkProtocolError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::CodeInvalid => "suite_code_invalid",
            Self::AccountMismatch => "suite_link_account_mismatch",
            Self::PeerUnknown => "suite_peer_unknown",
            Self::Disabled => "suite_disabled",
            Self::ReturnUrlInvalid => "suite_return_url_invalid",
            Self::PayloadInvalid => "suite_payload_invalid",
        }
    }
}

pub fn validate_link_start<'a>(
    registry: &'a PeerRegistry,
    request: &LinkStartRequest,
) -> Result<&'a ActivePeer, LinkProtocolError> {
    if !registry
        .metadata()
        .iter()
        .any(|peer| peer.id == request.peer_app && peer.id != registry.own().id)
    {
        return Err(LinkProtocolError::PeerUnknown);
    }
    let peer = registry
        .active_peer(&request.peer_app)
        .ok_or(LinkProtocolError::Disabled)?;
    registry
        .validate_return_url(&request.return_url)
        .map_err(|_| LinkProtocolError::ReturnUrlInvalid)?;
    if request.state_nonce.is_empty()
        || request.state_nonce.chars().count() > super::SUITE_MAX_TEXT_CHARACTERS
        || request.state_nonce.chars().any(char::is_control)
    {
        return Err(LinkProtocolError::PayloadInvalid);
    }
    Ok(peer)
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LinkCallbackQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CallbackRedirect {
    pub location: String,
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum CallbackError {
    #[error("suite callback request is unknown or has an invalid stored URL")]
    CodeInvalid,
}

impl CallbackError {
    pub const fn code(self) -> &'static str {
        "suite_code_invalid"
    }
}

pub fn relay_callback(
    request: Option<&LinkRequest>,
    query: &LinkCallbackQuery,
    now: DateTime<Utc>,
) -> Result<CallbackRedirect, CallbackError> {
    let request = request.ok_or(CallbackError::CodeInvalid)?;
    let valid = query.state.as_ref().is_some_and(|state| {
        !state.is_empty()
            && <[u8; 32]>::from(Sha256::digest(state.as_bytes())) == request.state_hash
    }) && request.expires_at > now
        && request.consumed_at.is_none()
        && query.from.as_deref() == Some(request.peer_app.as_str());
    let mut url = Url::parse(&request.return_url).map_err(|_| CallbackError::CodeInvalid)?;
    if url.cannot_be_a_base()
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(CallbackError::CodeInvalid);
    }
    let retained: Vec<_> = url
        .query_pairs()
        .filter(|(key, _)| !matches!(key.as_ref(), "state" | "request" | "code" | "status"))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    url.set_query(None);
    {
        let mut pairs = url.query_pairs_mut();
        pairs.extend_pairs(retained);
        pairs.append_pair("state", &request.client_state_nonce);
        match (valid, query.code.as_deref(), query.error.as_deref()) {
            (true, Some(code), None) if !code.is_empty() => {
                pairs.append_pair("request", &request.id.to_string());
                pairs.append_pair("code", code);
            }
            (true, None, Some("access_denied")) => {
                pairs.append_pair("request", &request.id.to_string());
                pairs.append_pair("status", "denied");
            }
            _ => {
                pairs.append_pair("status", "failed");
                pairs.append_pair("code", "suite_code_invalid");
            }
        }
    }
    Ok(CallbackRedirect {
        location: url.into(),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompleteDecision {
    CreateLink,
    ExistingLink(Uuid),
}

pub fn validate_complete(
    request: Option<&LinkRequest>,
    owner_id: Uuid,
    now: DateTime<Utc>,
) -> Result<CompleteDecision, LinkProtocolError> {
    let request = request
        .filter(|request| request.user_id == owner_id)
        .ok_or(LinkProtocolError::NotFound)?;
    if request.consumed_at.is_some() {
        return request
            .link_id
            .map(CompleteDecision::ExistingLink)
            .ok_or(LinkProtocolError::CodeInvalid);
    }
    if request.expires_at <= now || request.link_id.is_some() {
        return Err(LinkProtocolError::CodeInvalid);
    }
    Ok(CompleteDecision::CreateLink)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExchangeDecision {
    CreateLink { suite_subject: Option<String> },
    ExistingLink(Uuid),
}

pub fn validate_exchange(
    code: Option<&LinkCode>,
    request: &ExchangeRequest,
    existing_link: Option<&SuiteLink>,
    now: DateTime<Utc>,
) -> Result<ExchangeDecision, LinkProtocolError> {
    let code = code.ok_or(LinkProtocolError::CodeInvalid)?;
    if code.expires_at <= now
        || code.peer_app != request.client
        || <[u8; 32]>::from(Sha256::digest(request.code.as_bytes())) != code.code_hash
        || pkce_challenge(&request.code_verifier).ok().as_deref()
            != Some(code.code_challenge.as_str())
    {
        return Err(LinkProtocolError::CodeInvalid);
    }
    if code.auto_approved
        && (code.suite_subject.is_none() || request.initiator_suite_subject != code.suite_subject)
    {
        return Err(LinkProtocolError::AccountMismatch);
    }
    let suite_subject = shared_suite_subject(
        request.initiator_suite_subject.as_deref(),
        code.suite_subject.as_deref(),
    )
    .map_err(|error| match error {
        SuiteDataError::AccountMismatch => LinkProtocolError::AccountMismatch,
        _ => LinkProtocolError::CodeInvalid,
    })?;
    if code.consumed_at.is_some() {
        let link = existing_link
            .filter(|link| {
                Some(link.id) == code.link_id
                    && link.remote_link_id == request.initiator_link_id
                    && link.user_id == code.user_id
                    && link.peer_app == code.peer_app
            })
            .ok_or(LinkProtocolError::CodeInvalid)?;
        return Ok(ExchangeDecision::ExistingLink(link.id));
    }
    if code.link_id.is_some() {
        return Err(LinkProtocolError::CodeInvalid);
    }
    Ok(ExchangeDecision::CreateLink { suite_subject })
}
