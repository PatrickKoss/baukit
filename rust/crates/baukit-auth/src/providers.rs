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
/// Clerk session tokens do not require an `aud` claim, so this adapter checks
/// none unless [`ClerkVerifier::with_audiences`] configures one. When `azp` is
/// present, this adapter requires an exact match in the configured allowlist.
/// Clerk's version 2 active organization claim is mapped from `o.id`.
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

    /// Also requires the token's `aud` claim to contain one of `audiences`.
    ///
    /// Use this when Clerk JWT templates add an audience the product must check.
    pub fn with_audiences<I, T>(self, audiences: I) -> Result<Self, ProviderVerifierError>
    where
        I: IntoIterator<Item = T>,
        T: Into<String>,
    {
        let config = self.inner.config().clone().with_audiences(audiences)?;
        Ok(Self {
            inner: self.inner.with_config(config),
        })
    }

    /// Copies the named top-level string or boolean claims into [`Principal::profile_claims`].
    #[must_use]
    pub fn with_profile_claims<I, T>(self, claims: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<String>,
    {
        let config = self.inner.config().clone().with_profile_claims(claims);
        Self {
            inner: self.inner.with_config(config),
        }
    }

    /// Returns the Clerk issuer this verifier accepts.
    #[must_use]
    pub fn issuer(&self) -> &str {
        self.inner.issuer()
    }

    /// Verifies one Clerk session token.
    pub async fn verify(&self, token: &str) -> Result<Principal, VerificationError> {
        self.inner.verify(token).await
    }

    pub(crate) fn into_oidc(self) -> OidcVerifier {
        self.inner
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

/// Clerk OAuth JWT verification restricted to one pre-registered OAuth client.
///
/// Clerk does not document resource-indicator audiences for OAuth access tokens.
/// This verifier requires the signed `client_id` instead. Dedicate that OAuth
/// client to one MCP resource and enforce resource-specific scopes separately.
#[derive(Clone, Debug)]
pub struct ClerkOAuthVerifier {
    inner: OidcVerifier,
}

impl ClerkOAuthVerifier {
    /// Uses the Clerk instance's Frontend API URL and signing keys.
    pub fn new(
        issuer: impl AsRef<str>,
        client_id: impl Into<String>,
    ) -> Result<Self, ProviderVerifierError> {
        let client_id = client_id.into();
        let config = OidcConfig::client_bound(issuer, &client_id)?;
        let jwks = clerk_jwks_uri(&config)?;
        Self::from_jwks_uri(config.issuer.as_str(), client_id, jwks.as_str())
    }

    /// Uses an explicit signing-key endpoint with the same OAuth client binding.
    pub fn from_jwks_uri(
        issuer: impl AsRef<str>,
        client_id: impl Into<String>,
        jwks_uri: impl AsRef<str>,
    ) -> Result<Self, ProviderVerifierError> {
        let config =
            OidcConfig::client_bound(issuer, client_id)?.with_clock_skew(std::time::Duration::ZERO);
        Ok(Self {
            inner: OidcVerifier::from_jwks_uri(config, jwks_uri)?,
        })
    }
}

impl IdentityVerifier for ClerkOAuthVerifier {
    fn verify<'a>(
        &'a self,
        token: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Principal, VerificationError>> + Send + 'a>> {
        Box::pin(self.inner.verify(token))
    }
}

/// WorkOS AuthKit session-token verification bound to one application client.
///
/// AuthKit session tokens do not require an `aud` claim, so this adapter checks
/// none unless [`WorkOsVerifier::with_audiences`] configures one. It requires
/// the token's `client_id` claim to match the configured application and maps
/// `org_id` into [`Principal::organization`].
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
        let config = OidcConfig::client_bound(issuer, client_id)?;
        Ok(Self {
            inner: OidcVerifier::from_jwks_uri(config, jwks_uri)?,
        })
    }

    /// Also requires the token's `aud` claim to contain one of `audiences`.
    pub fn with_audiences<I, T>(self, audiences: I) -> Result<Self, ProviderVerifierError>
    where
        I: IntoIterator<Item = T>,
        T: Into<String>,
    {
        let config = self.inner.config().clone().with_audiences(audiences)?;
        Ok(Self {
            inner: self.inner.with_config(config),
        })
    }

    /// Copies the named top-level string or boolean claims into [`Principal::profile_claims`].
    #[must_use]
    pub fn with_profile_claims<I, T>(self, claims: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<String>,
    {
        let config = self.inner.config().clone().with_profile_claims(claims);
        Self {
            inner: self.inner.with_config(config),
        }
    }

    /// Returns the AuthKit issuer this verifier accepts.
    #[must_use]
    pub fn issuer(&self) -> &str {
        self.inner.issuer()
    }

    /// Verifies one WorkOS AuthKit session token.
    pub async fn verify(&self, token: &str) -> Result<Principal, VerificationError> {
        self.inner.verify(token).await
    }

    pub(crate) fn into_oidc(self) -> OidcVerifier {
        self.inner
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
