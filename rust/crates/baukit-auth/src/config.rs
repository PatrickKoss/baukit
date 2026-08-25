use std::{collections::BTreeSet, time::Duration};

use reqwest::Url;
use thiserror::Error;

/// JWT signing algorithms that can be explicitly allowed by a verifier.
///
/// Symmetric algorithms are intentionally unsupported because OIDC verification
/// should not distribute a provider's signing secret through JWKS.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SigningAlgorithm {
    /// RSASSA-PKCS1-v1_5 using SHA-256.
    Rs256,
    /// RSASSA-PKCS1-v1_5 using SHA-384.
    Rs384,
    /// RSASSA-PKCS1-v1_5 using SHA-512.
    Rs512,
    /// RSASSA-PSS using SHA-256.
    Ps256,
    /// RSASSA-PSS using SHA-384.
    Ps384,
    /// RSASSA-PSS using SHA-512.
    Ps512,
    /// ECDSA P-256 using SHA-256.
    Es256,
    /// ECDSA P-384 using SHA-384.
    Es384,
    /// Ed25519.
    EdDsa,
}

impl SigningAlgorithm {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Rs256 => "RS256",
            Self::Rs384 => "RS384",
            Self::Rs512 => "RS512",
            Self::Ps256 => "PS256",
            Self::Ps384 => "PS384",
            Self::Ps512 => "PS512",
            Self::Es256 => "ES256",
            Self::Es384 => "ES384",
            Self::EdDsa => "EdDSA",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "RS256" => Some(Self::Rs256),
            "RS384" => Some(Self::Rs384),
            "RS512" => Some(Self::Rs512),
            "PS256" => Some(Self::Ps256),
            "PS384" => Some(Self::Ps384),
            "PS512" => Some(Self::Ps512),
            "ES256" => Some(Self::Es256),
            "ES384" => Some(Self::Es384),
            "EdDSA" => Some(Self::EdDsa),
            _ => None,
        }
    }
}

/// Configuration that maps provider claims onto Baukit's stable principal fields.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PrincipalClaimMapping {
    pub(crate) organization: Option<String>,
    pub(crate) tenant: Option<String>,
}

impl PrincipalClaimMapping {
    /// Creates a mapping with only the standard `sub` identity claim.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            organization: None,
            tenant: None,
        }
    }

    /// Maps a top-level provider claim into [`Principal::organization`](crate::Principal::organization).
    #[must_use]
    pub fn organization_claim(mut self, claim: impl Into<String>) -> Self {
        self.organization = Some(claim.into());
        self
    }

    /// Maps a top-level provider claim into [`Principal::tenant`](crate::Principal::tenant).
    #[must_use]
    pub fn tenant_claim(mut self, claim: impl Into<String>) -> Self {
        self.tenant = Some(claim.into());
        self
    }
}

/// Provider-neutral OIDC verification configuration.
#[derive(Clone, Debug)]
pub struct OidcConfig {
    pub(crate) issuer: Url,
    pub(crate) audiences: BTreeSet<String>,
    pub(crate) algorithms: BTreeSet<SigningAlgorithm>,
    pub(crate) cache_ttl: Duration,
    pub(crate) request_timeout: Duration,
    pub(crate) clock_skew: Duration,
    pub(crate) claim_mapping: PrincipalClaimMapping,
}

impl OidcConfig {
    /// Creates configuration for an issuer and one required audience.
    pub fn new(
        issuer: impl AsRef<str>,
        audience: impl Into<String>,
    ) -> Result<Self, OidcConfigError> {
        let issuer = normalized_issuer(issuer.as_ref())?;
        let audience = audience.into();
        validate_nonempty("audience", &audience)?;
        Ok(Self {
            issuer,
            audiences: BTreeSet::from([audience]),
            algorithms: BTreeSet::from([SigningAlgorithm::Rs256]),
            cache_ttl: Duration::from_secs(300),
            request_timeout: Duration::from_secs(5),
            clock_skew: Duration::from_secs(60),
            claim_mapping: PrincipalClaimMapping::new(),
        })
    }

    /// Creates Keycloak-shaped configuration using `/realms/{realm}` as the issuer.
    ///
    /// This is only a URL convention. Discovery and verification still use
    /// standard OIDC metadata and no Keycloak SDK or API.
    pub fn keycloak(
        base_url: impl AsRef<str>,
        realm: impl AsRef<str>,
        audience: impl Into<String>,
    ) -> Result<Self, OidcConfigError> {
        let realm = realm.as_ref();
        validate_nonempty("realm", realm)?;
        let mut issuer = Url::parse(base_url.as_ref())
            .map_err(|error| OidcConfigError::InvalidIssuer(error.to_string()))?;
        issuer.set_query(None);
        issuer.set_fragment(None);
        {
            let mut segments = issuer
                .path_segments_mut()
                .map_err(|_| OidcConfigError::IssuerCannotBeBase)?;
            segments.pop_if_empty().push("realms").push(realm);
        }
        Self::new(issuer.as_str(), audience)
    }

    /// Replaces the acceptable audiences. At least one non-empty value is required.
    pub fn with_audiences<I, T>(mut self, audiences: I) -> Result<Self, OidcConfigError>
    where
        I: IntoIterator<Item = T>,
        T: Into<String>,
    {
        self.audiences = audiences.into_iter().map(Into::into).collect();
        if self.audiences.is_empty() || self.audiences.iter().any(String::is_empty) {
            return Err(OidcConfigError::EmptyValue("audience"));
        }
        Ok(self)
    }

    /// Replaces the signing-algorithm allowlist. At least one algorithm is required.
    pub fn with_allowed_algorithms<I>(mut self, algorithms: I) -> Result<Self, OidcConfigError>
    where
        I: IntoIterator<Item = SigningAlgorithm>,
    {
        self.algorithms = algorithms.into_iter().collect();
        if self.algorithms.is_empty() {
            return Err(OidcConfigError::EmptyAlgorithmAllowlist);
        }
        Ok(self)
    }

    /// Sets how long a successfully fetched JWKS remains fresh.
    pub fn with_jwks_cache_ttl(mut self, cache_ttl: Duration) -> Result<Self, OidcConfigError> {
        validate_duration("JWKS cache TTL", cache_ttl)?;
        self.cache_ttl = cache_ttl;
        Ok(self)
    }

    /// Sets the timeout applied independently to discovery and JWKS requests.
    pub fn with_request_timeout(
        mut self,
        request_timeout: Duration,
    ) -> Result<Self, OidcConfigError> {
        validate_duration("request timeout", request_timeout)?;
        self.request_timeout = request_timeout;
        Ok(self)
    }

    /// Sets allowed clock skew for `exp` and `nbf` checks.
    #[must_use]
    pub const fn with_clock_skew(mut self, clock_skew: Duration) -> Self {
        self.clock_skew = clock_skew;
        self
    }

    /// Configures optional organization and tenant claim mappings.
    #[must_use]
    pub fn with_principal_claims(mut self, mapping: PrincipalClaimMapping) -> Self {
        self.claim_mapping = mapping;
        self
    }

    /// Returns the exact issuer expected in discovery metadata and tokens.
    #[must_use]
    pub fn issuer(&self) -> &str {
        self.issuer.as_str()
    }

    pub(crate) fn discovery_url(&self) -> Url {
        let mut discovery = self.issuer.clone();
        discovery.set_path(&format!(
            "{}/.well-known/openid-configuration",
            self.issuer.path().trim_end_matches('/')
        ));
        discovery
    }
}

fn normalized_issuer(value: &str) -> Result<Url, OidcConfigError> {
    validate_nonempty("issuer", value)?;
    let mut issuer =
        Url::parse(value).map_err(|error| OidcConfigError::InvalidIssuer(error.to_string()))?;
    if issuer.cannot_be_a_base() {
        return Err(OidcConfigError::IssuerCannotBeBase);
    }
    issuer.set_query(None);
    issuer.set_fragment(None);
    let normalized_path = issuer.path().trim_end_matches('/').to_owned();
    issuer.set_path(&normalized_path);
    Ok(issuer)
}

fn validate_nonempty(name: &'static str, value: &str) -> Result<(), OidcConfigError> {
    if value.is_empty() {
        Err(OidcConfigError::EmptyValue(name))
    } else {
        Ok(())
    }
}

fn validate_duration(name: &'static str, duration: Duration) -> Result<(), OidcConfigError> {
    if duration.is_zero() {
        Err(OidcConfigError::ZeroDuration(name))
    } else {
        Ok(())
    }
}

/// Invalid OIDC verifier configuration.
#[derive(Debug, Error)]
pub enum OidcConfigError {
    /// A required string is empty.
    #[error("{0} must not be empty")]
    EmptyValue(&'static str),
    /// The issuer is not an absolute URL.
    #[error("issuer must be a valid absolute URL: {0}")]
    InvalidIssuer(String),
    /// The issuer URL cannot be used as a hierarchical base URL.
    #[error("issuer URL cannot be used as a base URL")]
    IssuerCannotBeBase,
    /// No signing algorithms were allowed.
    #[error("signing-algorithm allowlist must not be empty")]
    EmptyAlgorithmAllowlist,
    /// A network or cache duration is zero.
    #[error("{0} must be greater than zero")]
    ZeroDuration(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keycloak_defaults_build_realm_discovery_url() -> Result<(), OidcConfigError> {
        let config = OidcConfig::keycloak("https://identity.example.com/base/", "my realm", "api")?;
        assert_eq!(
            config.issuer(),
            "https://identity.example.com/base/realms/my%20realm"
        );
        assert_eq!(
            config.discovery_url().as_str(),
            "https://identity.example.com/base/realms/my%20realm/.well-known/openid-configuration"
        );
        Ok(())
    }

    #[test]
    fn empty_allowlists_and_zero_timeouts_are_rejected() -> Result<(), OidcConfigError> {
        let config = OidcConfig::new("https://identity.example.com/realms/test", "api")?;
        assert!(matches!(
            config.clone().with_allowed_algorithms([]),
            Err(OidcConfigError::EmptyAlgorithmAllowlist)
        ));
        assert!(matches!(
            config.with_request_timeout(Duration::ZERO),
            Err(OidcConfigError::ZeroDuration("request timeout"))
        ));
        Ok(())
    }
}
