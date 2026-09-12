use std::{future::Future, pin::Pin};

use reqwest::Url;
use thiserror::Error;

use crate::{
    IdentityVerifier, OidcConfig, OidcConfigError, OidcVerifier, Principal, VerificationError,
};

const WORKOS_ISSUER: &str = "https://api.workos.com/";
const WORKOS_JWKS_BASE: &str = "https://api.workos.com/sso/jwks/";

/// Clerk session-token verification with authorized-party checks.
///
/// Clerk session tokens do not require an `aud` claim. When `azp` is present,
/// this adapter requires an exact match in the configured allowlist. Clerk's
/// version 2 active organization claim is mapped from `o.id`.
#[derive(Clone, Debug)]
pub struct ClerkVerifier {
    inner: OidcVerifier,
}

impl ClerkVerifier {
    /// Uses the Clerk instance's Frontend API URL as issuer and JWKS host.
    pub fn new<I, T>(
        frontend_api_url: impl AsRef<str>,
        authorized_parties: I,
    ) -> Result<Self, ProviderVerifierError>
    where
        I: IntoIterator<Item = T>,
        T: Into<String>,
    {
        let config = OidcConfig::clerk(frontend_api_url, authorized_parties)?;
        let jwks_uri = clerk_jwks_uri(&config)?;
        Self::from_config(config, jwks_uri.as_str())
    }

    /// Uses an explicit JWKS endpoint while retaining Clerk claim validation.
    ///
    /// This is useful for a private proxy, an emulator, or an integration test.
    pub fn from_jwks_uri<I, T>(
        frontend_api_url: impl AsRef<str>,
        authorized_parties: I,
        jwks_uri: impl AsRef<str>,
    ) -> Result<Self, ProviderVerifierError>
    where
        I: IntoIterator<Item = T>,
        T: Into<String>,
    {
        let config = OidcConfig::clerk(frontend_api_url, authorized_parties)?;
        Self::from_config(config, jwks_uri.as_ref())
    }

    fn from_config(
        config: OidcConfig,
        jwks_uri: impl AsRef<str>,
    ) -> Result<Self, ProviderVerifierError> {
        Ok(Self {
            inner: OidcVerifier::from_jwks_uri(config, jwks_uri)?,
        })
    }

    /// Verifies one Clerk session token.
    pub async fn verify(&self, token: &str) -> Result<Principal, VerificationError> {
        self.inner.verify(token).await
    }
}

impl IdentityVerifier for ClerkVerifier {
    fn verify<'a>(
        &'a self,
        token: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Principal, VerificationError>> + Send + 'a>> {
        Box::pin(ClerkVerifier::verify(self, token))
    }
}

/// WorkOS AuthKit session-token verification bound to one application client.
///
/// AuthKit session tokens do not require an `aud` claim. This adapter instead
/// requires the token's `client_id` claim to match the configured application
/// and maps `org_id` into [`Principal::organization`].
#[derive(Clone, Debug)]
pub struct WorkOsVerifier {
    inner: OidcVerifier,
}

impl WorkOsVerifier {
    /// Uses WorkOS's default issuer and client-specific signing-key endpoint.
    pub fn new(client_id: impl Into<String>) -> Result<Self, ProviderVerifierError> {
        Self::with_issuer(WORKOS_ISSUER, client_id)
    }

    /// Uses a custom AuthKit issuer and WorkOS's client-specific signing-key endpoint.
    pub fn with_issuer(
        issuer: impl AsRef<str>,
        client_id: impl Into<String>,
    ) -> Result<Self, ProviderVerifierError> {
        let client_id = client_id.into();
        let jwks_uri = workos_jwks_uri(&client_id)?;
        Self::from_jwks_uri(issuer, client_id, jwks_uri.as_str())
    }

    /// Uses explicit issuer and JWKS endpoints while retaining WorkOS claim validation.
    ///
    /// This supports WorkOS Emulate, a private proxy, and integration tests.
    pub fn from_jwks_uri(
        issuer: impl AsRef<str>,
        client_id: impl Into<String>,
        jwks_uri: impl AsRef<str>,
    ) -> Result<Self, ProviderVerifierError> {
        let config = OidcConfig::workos(issuer, client_id)?;
        Ok(Self {
            inner: OidcVerifier::from_jwks_uri(config, jwks_uri)?,
        })
    }

    /// Verifies one WorkOS AuthKit session token.
    pub async fn verify(&self, token: &str) -> Result<Principal, VerificationError> {
        self.inner.verify(token).await
    }
}

impl IdentityVerifier for WorkOsVerifier {
    fn verify<'a>(
        &'a self,
        token: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Principal, VerificationError>> + Send + 'a>> {
        Box::pin(WorkOsVerifier::verify(self, token))
    }
}

fn clerk_jwks_uri(config: &OidcConfig) -> Result<Url, OidcConfigError> {
    let mut jwks_uri = config.issuer.clone();
    let mut segments = jwks_uri
        .path_segments_mut()
        .map_err(|_| OidcConfigError::IssuerCannotBeBase)?;
    segments
        .pop_if_empty()
        .push(".well-known")
        .push("jwks.json");
    drop(segments);
    Ok(jwks_uri)
}

fn workos_jwks_uri(client_id: &str) -> Result<Url, OidcConfigError> {
    if client_id.is_empty() {
        return Err(OidcConfigError::EmptyValue("client ID"));
    }
    let mut jwks_uri = Url::parse(WORKOS_JWKS_BASE)
        .map_err(|error| OidcConfigError::InvalidIssuer(error.to_string()))?;
    jwks_uri
        .path_segments_mut()
        .map_err(|_| OidcConfigError::IssuerCannotBeBase)?
        .pop_if_empty()
        .push(client_id);
    Ok(jwks_uri)
}

/// Invalid configuration for a Clerk or WorkOS verifier adapter.
#[derive(Debug, Error)]
pub enum ProviderVerifierError {
    /// The provider issuer, client, or allowlist was invalid.
    #[error(transparent)]
    Configuration(#[from] OidcConfigError),
    /// The provider's JWKS-backed verifier could not be constructed.
    #[error(transparent)]
    Verification(#[from] VerificationError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_urls_follow_documented_endpoints() -> Result<(), ProviderVerifierError> {
        let clerk_config = OidcConfig::clerk(
            "https://example.clerk.accounts.dev/",
            ["https://app.example.com"],
        )?;
        assert_eq!(
            clerk_jwks_uri(&clerk_config)?.as_str(),
            "https://example.clerk.accounts.dev/.well-known/jwks.json"
        );
        assert_eq!(
            workos_jwks_uri("client with spaces")?.as_str(),
            "https://api.workos.com/sso/jwks/client%20with%20spaces"
        );
        Ok(())
    }

    #[test]
    fn security_identifiers_cannot_be_empty() {
        assert!(matches!(
            ClerkVerifier::new("https://example.clerk.accounts.dev", Vec::<String>::new()),
            Err(ProviderVerifierError::Configuration(
                OidcConfigError::EmptyValue("authorized party")
            ))
        ));
        assert!(matches!(
            WorkOsVerifier::new(""),
            Err(ProviderVerifierError::Configuration(
                OidcConfigError::EmptyValue("client ID")
            ))
        ));
    }
}
