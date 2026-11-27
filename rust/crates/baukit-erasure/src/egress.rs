use std::time::Duration;

use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use baukit_egress::{
    AddressPolicy, AllowedEndpoint, EgressError, EgressOptions, EgressRequest, GuardedClient,
};
use url::Url;

use crate::IdentityDeletionError;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const ERROR_BODY_LIMIT: usize = 4096;
const CLERK_MISSING_CODE: &str = "resource_not_found";
const WORKOS_MISSING_CODE: &str = "entity_not_found";
const KEYCLOAK_MISSING_ERROR: &str = "user not found";

/// The error body dialect of an identity provider's "user missing" answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissingUserDialect {
    /// `{"errors":[{"code":"resource_not_found"}]}`.
    Clerk,
    /// `{"code":"entity_not_found"}`.
    WorkOs,
    /// `{"error":"User not found"}`.
    Keycloak,
}

/// Returns whether a provider `404` body says the requested user does not exist.
pub fn body_says_missing(dialect: MissingUserDialect, body: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return false;
    };
    match dialect {
        MissingUserDialect::Clerk => value
            .get("errors")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|errors| {
                errors.iter().any(|error| {
                    error.get("code").and_then(serde_json::Value::as_str)
                        == Some(CLERK_MISSING_CODE)
                })
            }),
        MissingUserDialect::WorkOs => {
            value.get("code").and_then(serde_json::Value::as_str) == Some(WORKOS_MISSING_CODE)
        }
        MissingUserDialect::Keycloak => value
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|error| error.trim().eq_ignore_ascii_case(KEYCLOAK_MISSING_ERROR)),
    }
}

/// Appends `parts` and then `subject` as single path segments of `base`.
///
/// # Errors
///
/// [`IdentityDeletionError::Permanent`] for a subject that is not one well-formed segment or a
/// base URL that cannot have path segments.
pub fn user_url(base: &Url, parts: &[&str], subject: &str) -> Result<Url, IdentityDeletionError> {
    if !crate::kernel::is_well_formed_subject(subject) {
        return Err(IdentityDeletionError::Permanent);
    }
    let mut url = base.clone();
    url.path_segments_mut()
        .map_err(|_| IdentityDeletionError::Permanent)?
        .pop_if_empty()
        .extend(parts)
        .push(subject);
    Ok(url)
}

/// Whether `host` is a loopback name or address.
pub(crate) fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    }
}

/// Classifies a deletion response with the provider's missing-user dialect.
pub(crate) fn deletion_outcome(
    dialect: MissingUserDialect,
    (status, body): &(StatusCode, Vec<u8>),
) -> Result<(), IdentityDeletionError> {
    let missing = *status == StatusCode::NOT_FOUND && body_says_missing(dialect, body);
    crate::kernel::deletion_status_outcome(status.as_u16(), missing)
}

/// Builds a client that reaches only the scheme, host and port of `base`.
pub(crate) fn endpoint_client(base: &Url) -> Result<GuardedClient, IdentityDeletionError> {
    let endpoint = AllowedEndpoint::from_url(base).map_err(|_| IdentityDeletionError::Permanent)?;
    let options = EgressOptions::default()
        .with_policy(AddressPolicy::OnlyEndpoint(endpoint))
        .with_connect_timeout(CONNECT_TIMEOUT)
        .and_then(|options| options.with_request_timeout(REQUEST_TIMEOUT))
        .and_then(|options| options.with_error_body_limit(ERROR_BODY_LIMIT))
        .map_err(|_| IdentityDeletionError::Permanent)?;
    GuardedClient::new("keycloak-admin", options).map_err(|_| IdentityDeletionError::Permanent)
}

pub(crate) fn bearer(token: &str) -> Result<HeaderMap, IdentityDeletionError> {
    let mut value = HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|_| IdentityDeletionError::Permanent)?;
    value.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(header::AUTHORIZATION, value);
    Ok(headers)
}

/// Sends `request` and returns the status and any captured error body, including non-`2xx` ones.
pub(crate) async fn status(
    client: &GuardedClient,
    request: EgressRequest,
) -> Result<(StatusCode, Vec<u8>), IdentityDeletionError> {
    match client.execute(request).await {
        Ok(response) => Ok((response.status(), Vec::new())),
        Err(EgressError::Status { status, body, .. }) => Ok((
            status,
            body.map(|body| body.bytes().to_vec()).unwrap_or_default(),
        )),
        Err(error) => Err(transport_error(&error)),
    }
}

/// Sends `request` and returns the successful response.
pub(crate) async fn success(
    client: &GuardedClient,
    request: EgressRequest,
) -> Result<baukit_egress::EgressResponse, IdentityDeletionError> {
    client
        .execute(request)
        .await
        .map_err(|error| transport_error(&error))
}

fn transport_error(error: &EgressError) -> IdentityDeletionError {
    match error {
        EgressError::Destination(_) | EgressError::BlockedAddress => {
            IdentityDeletionError::Permanent
        }
        EgressError::CircuitOpen { .. } | EgressError::Saturated => error
            .retry_class()
            .retry_after()
            .map_or(IdentityDeletionError::Retryable, |retry_after| {
                IdentityDeletionError::Unavailable { retry_after }
            }),
        _ => IdentityDeletionError::Retryable,
    }
}
