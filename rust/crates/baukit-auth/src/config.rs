use std::{borrow::Cow, collections::BTreeSet, time::Duration};

use reqwest::Url;
use thiserror::Error;

#[derive(Clone, Debug)]
pub(crate) enum TokenProfile {
    Oidc,
    Clerk {
        authorized_parties: BTreeSet<String>,
    },
    ClientBound {
        client_id: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ClaimPath(Vec<String>);

impl ClaimPath {
    fn top_level(claim: impl Into<String>) -> Self {
        Self(vec![claim.into()])
    }

    fn nested(claim: impl Into<String>, member: impl Into<String>) -> Self {
        Self(vec![claim.into(), member.into()])
    }

    pub(crate) fn segments(&self) -> &[String] {
        &self.0
    }
}

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

const DEFAULT_SCOPE_CLAIM: &str = "scope";

/// Configuration that maps provider claims onto Baukit's stable principal fields.
///
/// The RFC 9068 `scope` claim is mapped into [`Principal::scopes`](crate::Principal::scopes)
/// by default. Every other claim stays private until configured here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrincipalClaimMapping {
    pub(crate) organization: Option<ClaimPath>,
    pub(crate) tenant: Option<ClaimPath>,
    pub(crate) client_id: Option<ClaimPath>,
    pub(crate) scope: Cow<'static, str>,
    pub(crate) profile: BTreeSet<String>,
}

impl Default for PrincipalClaimMapping {
    fn default() -> Self {
        Self::new()
    }
}

impl PrincipalClaimMapping {
    /// Creates a mapping with the standard `sub` identity and `scope` claims.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            organization: None,
            tenant: None,
            client_id: None,
            scope: Cow::Borrowed(DEFAULT_SCOPE_CLAIM),
            profile: BTreeSet::new(),
        }
    }

    /// Maps a top-level provider claim into [`Principal::organization`](crate::Principal::organization).
    #[must_use]
    pub fn organization_claim(mut self, claim: impl Into<String>) -> Self {
        self.organization = Some(ClaimPath::top_level(claim));
        self
    }

    /// Maps a top-level provider claim into [`Principal::tenant`](crate::Principal::tenant).
    #[must_use]
    pub fn tenant_claim(mut self, claim: impl Into<String>) -> Self {
        self.tenant = Some(ClaimPath::top_level(claim));
        self
    }

    /// Maps a verified top-level provider claim into [`Principal::client_id`](crate::Principal::client_id).
    ///
    /// For example, Keycloak uses `azp`. No client claim is mapped by default.
    #[must_use]
    pub fn client_id_claim(mut self, claim: impl Into<String>) -> Self {
        self.client_id = Some(ClaimPath::top_level(claim));
        self
    }

    /// Reads [`Principal::scopes`](crate::Principal::scopes) from another top-level claim.
    ///
    /// RFC 9068 access tokens carry `scope` as a space-delimited string. Some
    /// providers emit an array of strings under another name, such as `scp`.
    /// Both shapes are accepted.
    #[must_use]
    pub fn scope_claim(mut self, claim: impl Into<String>) -> Self {
        self.scope = Cow::Owned(claim.into());
        self
    }

    /// Adds verified top-level claims to [`Principal::profile_claims`](crate::Principal::profile_claims).
    ///
    /// Only string and boolean values are copied, for example `email`,
    /// `email_verified`, `name`, and `picture`. Unselected claims stay private.
    #[must_use]
    pub fn profile_claims<I, T>(mut self, claims: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<String>,
    {
        self.profile.extend(claims.into_iter().map(Into::into));
        self
    }

    pub(crate) fn clerk() -> Self {
        Self {
            organization: Some(ClaimPath::nested("o", "id")),
            ..Self::new()
        }
    }

    pub(crate) fn client_bound() -> Self {
        Self {
            organization: Some(ClaimPath::top_level("org_id")),
            client_id: Some(ClaimPath::top_level("client_id")),
            ..Self::new()
        }
    }
}

/// Provider-neutral OIDC verification configuration.
#[derive(Clone, Debug)]
pub struct OidcConfig {
    pub(crate) issuer: Url,
    pub(crate) audiences: BTreeSet<String>,
    pub(crate) client_id: Option<String>,
    pub(crate) allowed_clients: Option<BTreeSet<String>>,
    pub(crate) algorithms: BTreeSet<SigningAlgorithm>,
    pub(crate) cache_ttl: Duration,
    pub(crate) request_timeout: Duration,
    pub(crate) clock_skew: Duration,
    pub(crate) claim_mapping: PrincipalClaimMapping,
    pub(crate) token_profile: TokenProfile,
}

impl OidcConfig {
    /// Creates configuration for an issuer and one required audience.
    ///
    /// Access tokens have no client allowlist by default. ID-token verification
    /// requires an explicit client ID through [`Self::with_client_id`].
    pub fn new(
        issuer: impl AsRef<str>,
        audience: impl Into<String>,
    ) -> Result<Self, OidcConfigError> {
        let issuer = normalized_issuer(issuer.as_ref())?;
        let audience = audience.into();
        validate_nonempty("audience", &audience)?;
        Ok(Self {
            issuer,
            client_id: None,
            allowed_clients: None,
            audiences: BTreeSet::from([audience]),
            algorithms: BTreeSet::from([SigningAlgorithm::Rs256]),
            cache_ttl: Duration::from_secs(300),
            request_timeout: Duration::from_secs(5),
            clock_skew: Duration::from_secs(60),
            claim_mapping: PrincipalClaimMapping::new(),
            token_profile: TokenProfile::Oidc,
        })
    }

    pub(crate) fn clerk<I, T>(
        issuer: impl AsRef<str>,
        authorized_parties: I,
    ) -> Result<Self, OidcConfigError>
    where
        I: IntoIterator<Item = T>,
        T: Into<String>,
    {
        let authorized_parties = nonempty_values("authorized party", authorized_parties)?;
        Ok(Self {
            issuer: normalized_issuer(issuer.as_ref())?,
            audiences: BTreeSet::new(),
            client_id: None,
            allowed_clients: None,
            algorithms: BTreeSet::from([SigningAlgorithm::Rs256]),
            cache_ttl: Duration::from_secs(300),
            request_timeout: Duration::from_secs(5),
            clock_skew: Duration::from_secs(5),
            claim_mapping: PrincipalClaimMapping::clerk(),
            token_profile: TokenProfile::Clerk { authorized_parties },
        })
    }

    pub(crate) fn client_bound(
        issuer: impl AsRef<str>,
        client_id: impl Into<String>,
    ) -> Result<Self, OidcConfigError> {
        let client_id = client_id.into();
        validate_nonempty("client ID", &client_id)?;
        Ok(Self {
            issuer: normalized_issuer(issuer.as_ref())?,
            audiences: BTreeSet::new(),
            client_id: Some(client_id.clone()),
            allowed_clients: None,
            algorithms: BTreeSet::from([SigningAlgorithm::Rs256]),
            cache_ttl: Duration::from_secs(300),
            request_timeout: Duration::from_secs(5),
            clock_skew: Duration::from_secs(60),
            claim_mapping: PrincipalClaimMapping::client_bound(),
            token_profile: TokenProfile::ClientBound { client_id },
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

    /// Sets the OAuth client ID required for ID-token verification.
    ///
    /// ID tokens must include this client in `aud`. Several audiences require
    /// this exact client in `azp`, as does any present `azp` on an ID token.
    /// This does not restrict the clients allowed to send access tokens.
    pub fn with_client_id(mut self, client_id: impl Into<String>) -> Result<Self, OidcConfigError> {
        let client_id = client_id.into();
        validate_nonempty("client ID", &client_id)?;
        self.client_id = Some(client_id);
        Ok(self)
    }

    /// Requires access tokens to carry `azp` from this allowlist.
    ///
    /// At least one non-empty client ID is required. ID-token verification uses
    /// [`Self::with_client_id`] independently of this list.
    pub fn with_allowed_clients<I, T>(mut self, clients: I) -> Result<Self, OidcConfigError>
    where
        I: IntoIterator<Item = T>,
        T: Into<String>,
    {
        self.allowed_clients = Some(nonempty_values("client ID", clients)?);
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

    /// Configures optional organization, tenant, OAuth client, scope, and profile claim mappings.
    #[must_use]
    pub fn with_principal_claims(mut self, mapping: PrincipalClaimMapping) -> Self {
        self.claim_mapping = mapping;
        self
    }

    pub(crate) fn with_profile_claims<I, T>(mut self, claims: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<String>,
    {
        self.claim_mapping = self.claim_mapping.profile_claims(claims);
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

fn nonempty_values<I, T>(name: &'static str, values: I) -> Result<BTreeSet<String>, OidcConfigError>
where
    I: IntoIterator<Item = T>,
    T: Into<String>,
{
    let values = values.into_iter().map(Into::into).collect::<BTreeSet<_>>();
    if values.is_empty() || values.iter().any(String::is_empty) {
        Err(OidcConfigError::EmptyValue(name))
    } else {
        Ok(values)
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
