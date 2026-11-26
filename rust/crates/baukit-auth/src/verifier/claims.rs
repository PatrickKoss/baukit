use std::collections::{BTreeMap, BTreeSet};

use jiff::Timestamp;
use serde::Deserialize;
use serde_json::Value;

use super::{JwtClaims, Principal, ProfileClaim, TokenVerification, VerificationError};
use crate::OidcConfig;
use crate::authorized_party::{validate_clerk_authorized_party, validate_client_id};
use crate::config::{ClaimPath, TokenProfile};
use crate::kernels::{TimeError, claims_time_valid};

impl JwtClaims {
    pub(super) fn validate(
        mut self,
        config: &OidcConfig,
        verification: TokenVerification<'_>,
        now: u64,
    ) -> Result<Principal, VerificationError> {
        let subject = self
            .sub
            .as_deref()
            .filter(|subject| !subject.is_empty())
            .map(str::to_owned)
            .ok_or(VerificationError::MissingSubject)?;
        if self.iss.as_deref() != Some(config.issuer.as_str()) {
            return Err(VerificationError::WrongIssuer);
        }
        validate_token_profile(&self, config, verification)?;
        let skew = config.clock_skew.as_secs();
        let expiry = self.exp.ok_or(VerificationError::MissingExpiry)?;
        claims_time_valid(
            saturating_i64(expiry),
            self.nbf.map(saturating_i64),
            saturating_i64(now),
            saturating_i64(skew),
        )
        .map_err(|error| match error {
            TimeError::Expired => VerificationError::Expired,
            TimeError::NotYetValid => VerificationError::NotYetValid,
        })?;
        let mapped_time = self
            .extra
            .remove(config.claim_mapping.authenticated_at.as_ref());
        let auth_time = self.extra.remove("auth_time");
        let authenticated_at = authentication_time(mapped_time.or(auth_time).as_ref(), now, skew)?;
        let organization = mapped_claim(&self.extra, config.claim_mapping.organization.as_ref())?;
        let tenant = mapped_claim(&self.extra, config.claim_mapping.tenant.as_ref())?;
        let client_id = mapped_claim(&self.extra, config.claim_mapping.client_id.as_ref())?;
        let scopes = scope_claim(self.extra.get(config.claim_mapping.scope.as_ref()))?;
        let profile_claims = selected_profile_claims(&self.extra, &config.claim_mapping.profile);
        Ok(Principal {
            subject,
            issuer: Some(config.issuer().to_owned()),
            authenticated_at,
            organization,
            tenant,
            client_id,
            scopes,
            profile_claims,
            api_token: None,
        })
    }
}

fn saturating_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

pub(super) fn authentication_time(
    value: Option<&Value>,
    now: u64,
    skew: u64,
) -> Result<Option<Timestamp>, VerificationError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let seconds = value.as_u64().ok_or(VerificationError::MalformedToken)?;
    let timestamp = i64::try_from(seconds)
        .ok()
        .and_then(|seconds| jiff::Timestamp::from_second(seconds).ok())
        .ok_or(VerificationError::MalformedToken)?;
    if seconds > now.saturating_add(skew) {
        return Err(VerificationError::NotYetValid);
    }
    Ok(Some(timestamp))
}

pub(super) fn validate_token_profile(
    claims: &JwtClaims,
    config: &OidcConfig,
    verification: TokenVerification<'_>,
) -> Result<(), VerificationError> {
    if let TokenVerification::Id { client_id, .. } = verification {
        return validate_id_token_client(claims, client_id);
    }
    if claims.extra.contains_key("nonce") {
        return Err(VerificationError::WrongTokenType);
    }
    if !config.audiences.is_empty() {
        validate_audience(claims, config)?;
    }
    match &config.token_profile {
        TokenProfile::Oidc => validate_oidc_authorized_party(claims, config),
        TokenProfile::Clerk {
            authorized_parties,
            allow_missing_azp,
        } => validate_clerk_authorized_party(
            claims.extra.get("azp"),
            authorized_parties,
            *allow_missing_azp,
        ),
        TokenProfile::ClientBound { client_id } => {
            validate_client_id(claims.extra.get("client_id"), Some(client_id.as_str()))
        }
        TokenProfile::ScopeBound { required_scopes } => {
            validate_client_id(claims.extra.get("client_id"), None)?;
            let granted = scope_claim(claims.extra.get(config.claim_mapping.scope.as_ref()))?;
            if required_scopes.is_subset(&granted) {
                Ok(())
            } else {
                Err(VerificationError::MissingScope)
            }
        }
    }
}

fn validate_oidc_authorized_party(
    claims: &JwtClaims,
    config: &OidcConfig,
) -> Result<(), VerificationError> {
    if let Some(allowed_clients) = &config.allowed_clients {
        return match claims.extra.get("azp") {
            Some(Value::String(client)) if allowed_clients.contains(client) => Ok(()),
            _ => Err(VerificationError::WrongAuthorizedParty),
        };
    }
    Ok(())
}

fn validate_id_token_client(claims: &JwtClaims, client_id: &str) -> Result<(), VerificationError> {
    if !claims
        .aud
        .as_ref()
        .is_some_and(|audience| audience.values().any(|value| value == client_id))
    {
        return Err(VerificationError::WrongAudience);
    }
    match claims.extra.get("azp") {
        None if !has_several_audiences(claims) => Ok(()),
        Some(Value::String(client)) if client == client_id => Ok(()),
        _ => Err(VerificationError::WrongAuthorizedParty),
    }
}

pub(super) fn has_several_audiences(claims: &JwtClaims) -> bool {
    claims
        .aud
        .as_ref()
        .is_some_and(|audience| audience.values().count() > 1)
}

pub(super) fn validate_audience(
    claims: &JwtClaims,
    config: &OidcConfig,
) -> Result<(), VerificationError> {
    let audience = claims
        .aud
        .as_ref()
        .ok_or(VerificationError::WrongAudience)?;
    if audience
        .values()
        .any(|audience| config.audiences.contains(audience))
    {
        Ok(())
    } else {
        Err(VerificationError::WrongAudience)
    }
}

pub(super) fn scope_claim(value: Option<&Value>) -> Result<BTreeSet<String>, VerificationError> {
    match value {
        None | Some(Value::Null) => Ok(BTreeSet::new()),
        Some(Value::String(scopes)) => {
            Ok(scopes.split_ascii_whitespace().map(str::to_owned).collect())
        }
        Some(Value::Array(scopes)) => scopes
            .iter()
            .map(|scope| match scope {
                Value::String(scope) if !scope.is_empty() => Ok(scope.clone()),
                _ => Err(VerificationError::InvalidPrincipalContext),
            })
            .collect(),
        Some(_) => Err(VerificationError::InvalidPrincipalContext),
    }
}

pub(super) fn selected_profile_claims(
    claims: &BTreeMap<String, Value>,
    selected: &BTreeSet<String>,
) -> BTreeMap<String, ProfileClaim> {
    selected
        .iter()
        .filter_map(|name| {
            let claim = match claims.get(name)? {
                Value::String(value) => ProfileClaim::String(value.clone()),
                Value::Bool(value) => ProfileClaim::Bool(*value),
                _ => return None,
            };
            Some((name.clone(), claim))
        })
        .collect()
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum Audience {
    One(String),
    Many(Vec<String>),
}

impl Audience {
    pub(super) fn values(&self) -> Box<dyn Iterator<Item = &String> + '_> {
        match self {
            Self::One(value) => Box::new(std::iter::once(value)),
            Self::Many(values) => Box::new(values.iter()),
        }
    }
}

fn mapped_claim(
    claims: &BTreeMap<String, Value>,
    claim_path: Option<&ClaimPath>,
) -> Result<Option<String>, VerificationError> {
    let Some(claim_path) = claim_path else {
        return Ok(None);
    };
    let mut segments = claim_path.segments().iter();
    let Some(first) = segments.next() else {
        return Ok(None);
    };
    let mut value = claims.get(first);
    for segment in segments {
        value = match value {
            None | Some(Value::Null) => return Ok(None),
            Some(Value::Object(object)) => object.get(segment),
            Some(_) => return Err(VerificationError::InvalidPrincipalContext),
        };
    }
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value.clone())),
        Some(_) => Err(VerificationError::InvalidPrincipalContext),
    }
}

#[cfg(test)]
#[path = "claims_tests.rs"]
mod tests;
