use baukit_egress::{
    AddressPolicy, AllowedEndpoint, EgressError, EgressOptions, EgressRequest, GuardedClient,
    validate_destination,
};
use reqwest::Url;
use serde::de::DeserializeOwned;

use crate::{OidcConfig, VerificationError};

/// Which OIDC document a request fetches, for error mapping.
#[derive(Clone, Copy)]
pub(crate) enum Document {
    Discovery,
    Jwks,
}

/// Builds a client for an operator-configured URL.
///
/// Without an explicit [`OidcConfig::with_egress_policy`], the client reaches
/// only the URL's exact scheme, host and port.
pub(crate) fn configured_client(
    config: &OidcConfig,
    url: &Url,
) -> Result<GuardedClient, VerificationError> {
    let policy = match &config.egress_policy {
        Some(policy) => policy.clone(),
        None => AddressPolicy::OnlyEndpoint(
            AllowedEndpoint::from_url(url).map_err(|_| VerificationError::EgressEndpoint)?,
        ),
    };
    build(config, policy)
}

/// Returns the client for a JWKS URL named by the discovery document.
///
/// The discovery client is reused when its policy allows the URL. Otherwise
/// the client reaches only the JWKS URL's exact scheme, host and port: the
/// configured issuer is trusted to name its own key endpoint, and metadata
/// and link-local addresses stay blocked.
pub(crate) fn discovered_jwks_client(
    config: &OidcConfig,
    discovery: GuardedClient,
    jwks_uri: &Url,
) -> Result<GuardedClient, VerificationError> {
    if config.egress_policy.is_some()
        || validate_destination(jwks_uri, discovery.options().policy()).is_ok()
    {
        return Ok(discovery);
    }
    configured_client(config, jwks_uri)
}

fn build(config: &OidcConfig, policy: AddressPolicy) -> Result<GuardedClient, VerificationError> {
    let options = EgressOptions::default()
        .with_policy(policy)
        .with_request_timeout(config.request_timeout)
        .map_err(VerificationError::EgressOptions)?;
    GuardedClient::new(options).map_err(VerificationError::Client)
}

pub(crate) async fn get_json<T: DeserializeOwned>(
    client: &GuardedClient,
    url: Url,
    document: Document,
) -> Result<T, VerificationError> {
    let response = client
        .execute(EgressRequest::get(url))
        .await
        .map_err(|error| document.request_error(error))?;
    serde_json::from_slice(response.body()).map_err(|error| document.invalid(error))
}

impl Document {
    fn request_error(self, error: EgressError) -> VerificationError {
        match (self, error) {
            (Self::Discovery, EgressError::Status { status, .. }) => {
                VerificationError::DiscoveryStatus(status)
            }
            (Self::Discovery, EgressError::Timeout) => VerificationError::DiscoveryTimeout,
            (Self::Discovery, error) => VerificationError::DiscoveryRequest(Box::new(error)),
            (Self::Jwks, EgressError::Status { status, .. }) => {
                VerificationError::JwksStatus(status)
            }
            (Self::Jwks, EgressError::Timeout) => VerificationError::JwksTimeout,
            (Self::Jwks, error) => VerificationError::JwksRequest(Box::new(error)),
        }
    }

    fn invalid(self, error: serde_json::Error) -> VerificationError {
        match self {
            Self::Discovery => VerificationError::InvalidDiscoveryDocument(error),
            Self::Jwks => VerificationError::InvalidJwksDocument(error),
        }
    }
}
