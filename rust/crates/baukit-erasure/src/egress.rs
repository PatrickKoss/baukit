use std::time::Duration;

use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use baukit_egress::{
    AddressPolicy, AllowedEndpoint, EgressError, EgressOptions, EgressRequest, GuardedClient,
};
use url::Url;

use crate::IdentityDeletionError;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Builds a client that reaches only the scheme, host and port of `base`.
pub(crate) fn endpoint_client(base: &Url) -> Result<GuardedClient, IdentityDeletionError> {
    let endpoint = AllowedEndpoint::from_url(base).map_err(|_| IdentityDeletionError::Permanent)?;
    let options = EgressOptions::default()
        .with_policy(AddressPolicy::OnlyEndpoint(endpoint))
        .with_connect_timeout(CONNECT_TIMEOUT)
        .and_then(|options| options.with_request_timeout(REQUEST_TIMEOUT))
        .map_err(|_| IdentityDeletionError::Permanent)?;
    GuardedClient::new(options).map_err(|_| IdentityDeletionError::Permanent)
}

pub(crate) fn bearer(token: &str) -> Result<HeaderMap, IdentityDeletionError> {
    let mut value = HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|_| IdentityDeletionError::Permanent)?;
    value.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(header::AUTHORIZATION, value);
    Ok(headers)
}

/// Sends `request` and returns the response status, including non-`2xx` ones.
pub(crate) async fn status(
    client: &GuardedClient,
    request: EgressRequest,
) -> Result<StatusCode, IdentityDeletionError> {
    match client.execute(request).await {
        Ok(response) => Ok(response.status()),
        Err(EgressError::Status { status, .. }) => Ok(status),
        Err(error) => Err(transport_error(&error)),
    }
}

/// Sends `request` and returns the successful response.
#[cfg(feature = "keycloak")]
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
        _ => IdentityDeletionError::Retryable,
    }
}
