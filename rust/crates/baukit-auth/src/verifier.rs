use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use jiff::Timestamp;
use reqwest::Url;
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

use crate::oidc_http::{self, Document};
use crate::{ApiToken, ClerkVerifier, OidcConfig, SigningAlgorithm, WorkOsVerifier};

mod claims;
mod error;
mod jwks;

pub use error::VerificationError;

use claims::Audience;
use jwks::{HttpKeySource, Jwk, JwkSet, JwksCache, JwksPolicy, KeySource};

/// A verified, provider-neutral application identity.
///
/// The stable subject, verified login time, and explicitly configured context
/// cross the auth boundary. API-token principals also retain the verified stored
/// token metadata. Raw JWT claims are deliberately not exposed.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Principal {
    subject: String,
    issuer: Option<String>,
    authenticated_at: Option<Timestamp>,
    organization: Option<String>,
    tenant: Option<String>,
    client_id: Option<String>,
    scopes: BTreeSet<String>,
    profile_claims: BTreeMap<String, ProfileClaim>,
    api_token: Option<ApiToken>,
}

/// One verified profile claim selected with
/// [`PrincipalClaimMapping::profile_claims`](crate::PrincipalClaimMapping::profile_claims).
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ProfileClaim {
    /// A string claim such as `email` or `name`.
    String(String),
    /// A boolean claim such as `email_verified`.
    Bool(bool),
}

impl ProfileClaim {
    /// Returns the value when the claim is a string.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            Self::Bool(_) => None,
        }
    }

    /// Returns the value when the claim is a boolean.
    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            Self::String(_) => None,
        }
    }
}

impl Principal {
    /// Creates an internal principal without organization, tenant, or OAuth client context.
    #[must_use]
    pub fn new(subject: impl Into<String>) -> Self {
        Self {
            subject: subject.into(),
            issuer: None,
            authenticated_at: None,
            organization: None,
            tenant: None,
            client_id: None,
            scopes: BTreeSet::new(),
            profile_claims: BTreeMap::new(),
            api_token: None,
        }
    }

    /// Builds the principal for a verified API token.
    ///
    /// [`ApiTokenVerifier`](crate::ApiTokenVerifier) passes the owner id as the
    /// subject. Products whose token owner is not the OIDC subject pass the
    /// subject they resolved for the owner, so authorization and erasure fences
    /// see the same subject as for an OIDC login.
    ///
    /// This constructor does not check revocation or expiry. The caller must
    /// verify the token against the current store on every request. Stored grants
    /// remain in [`Self::grants`]; [`Self::scopes`] stays empty for API tokens.
    #[must_use]
    pub fn from_api_token(subject: impl Into<String>, api_token: ApiToken) -> Self {
        Self {
            authenticated_at: Some(api_token.created_at),
            api_token: Some(api_token),
            ..Self::new(subject)
        }
    }

    /// Adds organization context to a principal created by a trusted adapter.
    #[must_use]
    pub fn with_organization(mut self, organization: impl Into<String>) -> Self {
        self.organization = Some(organization.into());
        self
    }

    /// Adds the login time supplied by a trusted adapter.
    #[must_use]
    pub fn with_authenticated_at(mut self, authenticated_at: Timestamp) -> Self {
        self.authenticated_at = Some(authenticated_at);
        self
    }

    /// Returns the verified `auth_time`, which stays unchanged on token refresh.
    ///
    /// API tokens return their creation time. Absent claims and internal
    /// principals return `None`.
    #[must_use]
    pub const fn authenticated_at(&self) -> Option<Timestamp> {
        self.authenticated_at
    }

    /// Adds tenant context to a principal created by a trusted adapter.
    #[must_use]
    pub fn with_tenant(mut self, tenant: impl Into<String>) -> Self {
        self.tenant = Some(tenant.into());
        self
    }

    /// Returns the provider's stable subject identifier.
    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// Returns the verified OIDC issuer when the principal came from an OIDC token.
    ///
    /// Internal principals created with [`Principal::new`] have no issuer. OIDC
    /// identities should be keyed by the `(issuer, subject)` pair because `sub`
    /// is only unique within one issuer.
    #[must_use]
    pub fn issuer(&self) -> Option<&str> {
        self.issuer.as_deref()
    }

    /// Returns normalized organization context when configured and present.
    #[must_use]
    pub fn organization(&self) -> Option<&str> {
        self.organization.as_deref()
    }

    /// Returns normalized tenant context when configured and present.
    #[must_use]
    pub fn tenant(&self) -> Option<&str> {
        self.tenant.as_deref()
    }

    /// Returns the OAuth client identity from a configured, verified provider claim.
    ///
    /// Unconfigured or absent claims, API tokens, and internal principals return
    /// `None`. Products must apply their own client allowlist before granting
    /// client-restricted access. The client ID is not a user or tenant identity.
    #[must_use]
    pub fn client_id(&self) -> Option<&str> {
        self.client_id.as_deref()
    }

    /// Returns the verified OAuth scopes of an OIDC principal.
    ///
    /// The set comes from the configured scope claim, `scope` by default. It is
    /// empty for API tokens, internal principals, and tokens without the claim.
    #[must_use]
    pub const fn scopes(&self) -> &BTreeSet<String> {
        &self.scopes
    }

    /// Returns the stored grants when an API token was verified.
    ///
    /// OIDC principals and internal principals return `None`, so a caller can
    /// tell "no grants" apart from "not an API token".
    #[must_use]
    pub fn grants(&self) -> Option<&BTreeSet<String>> {
        self.api_token.as_ref().map(|token| &token.grants)
    }

    /// Returns the verified profile claims selected by the verifier's configuration.
    #[must_use]
    pub const fn profile_claims(&self) -> &BTreeMap<String, ProfileClaim> {
        &self.profile_claims
    }

    /// Returns one selected profile claim when the token carried it.
    #[must_use]
    pub fn profile_claim(&self, name: &str) -> Option<&ProfileClaim> {
        self.profile_claims.get(name)
    }

    /// Returns the stored token metadata when an API token was verified.
    ///
    /// OIDC principals and internal principals created with [`Principal::new`]
    /// return `None`.
    #[must_use]
    pub const fn api_token(&self) -> Option<&ApiToken> {
        self.api_token.as_ref()
    }
}

/// Provider port used by the Axum integration to verify bearer access tokens.
///
/// Product-specific adapters can implement this trait while domain handlers
/// continue to consume only [`Principal`].
pub trait IdentityVerifier: Send + Sync {
    /// Verifies an encoded bearer token and returns its internal principal.
    fn verify<'a>(
        &'a self,
        token: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Principal, VerificationError>> + Send + 'a>>;
}

/// Standard OIDC discovery and JWKS-backed JWT verifier.
#[derive(Clone)]
pub struct OidcVerifier {
    inner: Arc<VerifierInner>,
}

/// OIDC verifier that accepts tokens from an explicit set of issuers.
///
/// The unverified `iss` claim is used only to select a preconfigured verifier.
/// That verifier then performs the normal signature, issuer, audience, expiry,
/// and claim validation before a principal is returned.
#[derive(Clone)]
pub struct MultiIssuerVerifier {
    verifiers: Arc<BTreeMap<String, OidcVerifier>>,
}

/// One already-constructed verifier that [`MultiIssuerVerifier::from_verifiers`] can route to.
#[derive(Clone, Debug)]
pub enum IssuerVerifier {
    /// A generic OIDC issuer.
    Oidc(OidcVerifier),
    /// A Clerk instance, keeping its authorized-party checks.
    Clerk(ClerkVerifier),
    /// A WorkOS AuthKit issuer, keeping its client ID check.
    WorkOs(WorkOsVerifier),
}

impl IssuerVerifier {
    fn into_oidc(self) -> OidcVerifier {
        match self {
            Self::Oidc(verifier) => verifier,
            Self::Clerk(verifier) => verifier.into_oidc(),
            Self::WorkOs(verifier) => verifier.into_oidc(),
        }
    }
}

impl From<OidcVerifier> for IssuerVerifier {
    fn from(verifier: OidcVerifier) -> Self {
        Self::Oidc(verifier)
    }
}

impl From<ClerkVerifier> for IssuerVerifier {
    fn from(verifier: ClerkVerifier) -> Self {
        Self::Clerk(verifier)
    }
}

impl From<WorkOsVerifier> for IssuerVerifier {
    fn from(verifier: WorkOsVerifier) -> Self {
        Self::WorkOs(verifier)
    }
}

impl MultiIssuerVerifier {
    /// Routes between already-constructed OIDC, Clerk, and WorkOS verifiers.
    ///
    /// Each verifier keeps its own provider checks. At least one verifier is
    /// required, and no two may share an issuer.
    ///
    /// # Errors
    ///
    /// Returns [`MultiIssuerError::NoIssuers`] when `verifiers` is empty and
    /// [`MultiIssuerError::DuplicateIssuer`] when two verifiers share an issuer.
    pub fn from_verifiers<I, V>(verifiers: I) -> Result<Self, MultiIssuerError>
    where
        I: IntoIterator<Item = V>,
        V: Into<IssuerVerifier>,
    {
        let mut by_issuer = BTreeMap::new();
        for verifier in verifiers {
            let verifier = verifier.into().into_oidc();
            let issuer = verifier.issuer().to_owned();
            match by_issuer.entry(issuer) {
                Entry::Vacant(entry) => {
                    entry.insert(verifier);
                }
                Entry::Occupied(entry) => {
                    return Err(MultiIssuerError::DuplicateIssuer(entry.key().clone()));
                }
            }
        }
        if by_issuer.is_empty() {
            return Err(MultiIssuerError::NoIssuers);
        }
        Ok(Self {
            verifiers: Arc::new(by_issuer),
        })
    }

    /// Discovers every configured issuer and constructs an allowlisted verifier.
    ///
    /// At least one unique issuer must be supplied. Discovery is completed for
    /// the whole set before the verifier is returned, so partial configuration
    /// never reaches request handling.
    ///
    /// # Errors
    ///
    /// Returns [`MultiIssuerError::NoIssuers`] when `configs` is empty,
    /// [`MultiIssuerError::DuplicateIssuer`] when two configs share an issuer, and
    /// [`MultiIssuerError::Discovery`] naming the first issuer whose discovery failed.
    pub async fn discover<I>(configs: I) -> Result<Self, MultiIssuerError>
    where
        I: IntoIterator<Item = OidcConfig>,
    {
        let mut configs_by_issuer = BTreeMap::new();
        for config in configs {
            let issuer = config.issuer().to_owned();
            match configs_by_issuer.entry(issuer.clone()) {
                Entry::Vacant(entry) => {
                    entry.insert(config);
                }
                Entry::Occupied(_) => return Err(MultiIssuerError::DuplicateIssuer(issuer)),
            }
        }
        if configs_by_issuer.is_empty() {
            return Err(MultiIssuerError::NoIssuers);
        }

        let mut verifiers = BTreeMap::new();
        for (issuer, config) in configs_by_issuer {
            let verifier = OidcVerifier::discover(config).await.map_err(|source| {
                MultiIssuerError::Discovery {
                    issuer: issuer.clone(),
                    source,
                }
            })?;
            verifiers.insert(issuer, verifier);
        }
        Ok(Self {
            verifiers: Arc::new(verifiers),
        })
    }

    /// Constructs an allowlisted verifier from explicit JWKS endpoints.
    ///
    /// This is useful when tokens contain a public issuer URL but the verifier
    /// must fetch keys through a private network endpoint. The configured
    /// issuer is still validated exactly against each token's `iss` claim.
    ///
    /// # Errors
    ///
    /// Returns [`MultiIssuerError::DuplicateIssuer`] when two configs share an issuer,
    /// [`MultiIssuerError::Configuration`] when a JWKS URL does not parse or its egress
    /// client cannot be built, and [`MultiIssuerError::NoIssuers`] when `configs` is empty.
    pub fn from_jwks_uris<I, S>(configs: I) -> Result<Self, MultiIssuerError>
    where
        I: IntoIterator<Item = (OidcConfig, S)>,
        S: AsRef<str>,
    {
        let mut verifiers = BTreeMap::new();
        for (config, jwks_uri) in configs {
            let issuer = config.issuer().to_owned();
            match verifiers.entry(issuer.clone()) {
                Entry::Vacant(entry) => {
                    let verifier =
                        OidcVerifier::from_jwks_uri(config, jwks_uri).map_err(|source| {
                            MultiIssuerError::Configuration {
                                issuer: issuer.clone(),
                                source,
                            }
                        })?;
                    entry.insert(verifier);
                }
                Entry::Occupied(_) => return Err(MultiIssuerError::DuplicateIssuer(issuer)),
            }
        }
        if verifiers.is_empty() {
            return Err(MultiIssuerError::NoIssuers);
        }
        Ok(Self {
            verifiers: Arc::new(verifiers),
        })
    }

    /// Verifies one access token against its configured issuer.
    ///
    /// # Errors
    ///
    /// Returns [`VerificationError::MalformedToken`] when the token cannot be parsed,
    /// [`VerificationError::WrongIssuer`] when it has no `iss`, and
    /// [`VerificationError::UnconfiguredIssuer`] when no verifier matches it. Otherwise
    /// fails like [`OidcVerifier::verify`] for the matching issuer.
    pub async fn verify(&self, token: &str) -> Result<Principal, VerificationError> {
        let issuer = ParsedToken::parse(token)?
            .claims
            .iss
            .ok_or(VerificationError::WrongIssuer)?;
        let verifier = self
            .verifiers
            .get(&issuer)
            .ok_or(VerificationError::UnconfiguredIssuer)?;
        verifier.verify(token).await
    }

    /// Returns whether the exact issuer text is configured.
    #[must_use]
    pub fn supports_issuer(&self, issuer: &str) -> bool {
        self.verifiers.contains_key(issuer)
    }
}

impl IdentityVerifier for MultiIssuerVerifier {
    fn verify<'a>(
        &'a self,
        token: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Principal, VerificationError>> + Send + 'a>> {
        Box::pin(MultiIssuerVerifier::verify(self, token))
    }
}

impl std::fmt::Debug for MultiIssuerVerifier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MultiIssuerVerifier")
            .field("issuers", &self.verifiers.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// Failure while constructing a multi-issuer verifier.
#[derive(Debug, Error)]
pub enum MultiIssuerError {
    /// No issuer configuration was supplied.
    #[error("at least one OIDC issuer must be configured")]
    NoIssuers,
    /// The same issuer text was configured more than once.
    #[error("OIDC issuer was configured more than once: {0}")]
    DuplicateIssuer(String),
    /// One configured issuer could not be discovered.
    #[error("could not discover OIDC issuer {issuer}: {source}")]
    Discovery {
        /// The exact configured issuer whose discovery failed.
        issuer: String,
        /// The underlying discovery failure.
        #[source]
        source: VerificationError,
    },
    /// One configured issuer had an invalid explicit JWKS endpoint.
    #[error("could not configure OIDC issuer {issuer}: {source}")]
    Configuration {
        /// The exact configured issuer whose endpoint was invalid.
        issuer: String,
        /// The underlying configuration failure.
        #[source]
        source: VerificationError,
    },
}

struct VerifierInner {
    config: OidcConfig,
    source: KeySource,
    jwks: JwksCache,
}

impl OidcVerifier {
    /// Constructs a verifier with an explicit JWKS endpoint.
    ///
    /// Use this when the token issuer is a public URL but key retrieval must use
    /// a distinct private-network URL. Token issuer validation remains bound to
    /// [`OidcConfig::issuer`]; only discovery is bypassed.
    ///
    /// # Errors
    ///
    /// Returns [`VerificationError::InvalidJwksUri`] when `jwks_uri` does not parse,
    /// and [`VerificationError::EgressEndpoint`], [`VerificationError::EgressOptions`]
    /// or [`VerificationError::Client`] when the egress client cannot be built.
    pub fn from_jwks_uri(
        config: OidcConfig,
        jwks_uri: impl AsRef<str>,
    ) -> Result<Self, VerificationError> {
        let jwks_uri =
            Url::parse(jwks_uri.as_ref()).map_err(|_| VerificationError::InvalidJwksUri)?;
        let client = oidc_http::configured_client(&config, &jwks_uri)?;
        Ok(Self {
            inner: Arc::new(VerifierInner {
                jwks: JwksCache::new(JwksPolicy::from_config(&config)),
                config,
                source: KeySource::Http(Arc::new(HttpKeySource { client, jwks_uri })),
            }),
        })
    }

    /// Constructs a verifier that checks tokens against a fixed JWKS document.
    ///
    /// The keys never refresh. Use this for offline verification and tests.
    ///
    /// # Errors
    ///
    /// Returns [`VerificationError::InvalidJwksDocument`] when `jwks` is not a
    /// JSON object with a `keys` array.
    pub fn from_jwks_document(config: OidcConfig, jwks: &str) -> Result<Self, VerificationError> {
        let set: JwkSet =
            serde_json::from_str(jwks).map_err(VerificationError::InvalidJwksDocument)?;
        Ok(Self {
            inner: Arc::new(VerifierInner {
                jwks: JwksCache::new(JwksPolicy::from_config(&config)),
                config,
                source: KeySource::Static(Arc::new(set)),
            }),
        })
    }

    /// Verifies an access token at a fixed Unix time. Fuzzing support only.
    #[cfg(feature = "fuzzing")]
    #[doc(hidden)]
    pub async fn verify_at(&self, token: &str, now: u64) -> Result<Principal, VerificationError> {
        self.verify_token_at(token, TokenVerification::Access, now)
            .await
    }

    /// Verifies an ID token at a fixed Unix time. Fuzzing support only.
    #[cfg(feature = "fuzzing")]
    #[doc(hidden)]
    pub async fn verify_id_token_at(
        &self,
        token: &str,
        expected_nonce: &str,
        now: u64,
    ) -> Result<Principal, VerificationError> {
        if expected_nonce.is_empty() {
            return Err(VerificationError::WrongNonce);
        }
        let client_id = self
            .inner
            .config
            .client_id
            .as_deref()
            .ok_or(VerificationError::MissingClientId)?;
        let verification = TokenVerification::Id {
            client_id,
            expected_nonce: Some(expected_nonce),
        };
        self.verify_token_at(token, verification, now).await
    }

    /// Discovers the configured issuer and constructs a verifier.
    ///
    /// The discovery response must report the exact configured issuer. JWKS are
    /// fetched lazily on first verification and then cached according to the
    /// configured TTL.
    ///
    /// # Errors
    ///
    /// Returns the egress client errors of [`Self::from_jwks_uri`], a discovery
    /// transport error (`DiscoveryTimeout`, `DiscoveryRequest`, `DiscoveryStatus`,
    /// `InvalidDiscoveryDocument`), [`VerificationError::DiscoveryIssuerMismatch`]
    /// when metadata names another issuer, and `InvalidJwksUri` for a bad `jwks_uri`.
    pub async fn discover(config: OidcConfig) -> Result<Self, VerificationError> {
        let discovery = oidc_http::configured_client(&config, &config.issuer_url)?;
        let metadata: DiscoveryDocument =
            oidc_http::get_json(&discovery, config.discovery_url(), Document::Discovery).await?;
        if metadata.issuer != config.issuer.as_str() {
            return Err(VerificationError::DiscoveryIssuerMismatch);
        }
        let jwks_uri =
            Url::parse(&metadata.jwks_uri).map_err(|_| VerificationError::InvalidJwksUri)?;
        let client = oidc_http::discovered_jwks_client(&config, discovery, &jwks_uri)?;
        Ok(Self {
            inner: Arc::new(VerifierInner {
                jwks: JwksCache::new(JwksPolicy::from_config(&config)),
                config,
                source: KeySource::Http(Arc::new(HttpKeySource { client, jwks_uri })),
            }),
        })
    }

    /// Returns the exact issuer this verifier accepts.
    #[must_use]
    pub fn issuer(&self) -> &str {
        self.inner.config.issuer()
    }

    pub(crate) fn config(&self) -> &OidcConfig {
        &self.inner.config
    }

    pub(crate) fn with_config(&self, config: OidcConfig) -> Self {
        Self {
            inner: Arc::new(VerifierInner {
                jwks: JwksCache::new(JwksPolicy::from_config(&config)),
                config,
                source: self.inner.source.clone(),
            }),
        }
    }

    /// Verifies one access token using the configured issuer, audience, and algorithms.
    ///
    /// # Errors
    ///
    /// Returns a [`VerificationError`] naming the failed check: a malformed token, a
    /// disallowed algorithm, an unknown key or bad signature, a wrong issuer, audience,
    /// client or scope, an expired or not-yet-valid token, or a JWKS fetch failure.
    pub async fn verify(&self, token: &str) -> Result<Principal, VerificationError> {
        self.verify_token(token, TokenVerification::Access).await
    }

    /// Verifies an ID token and requires its `nonce` to match the login request.
    ///
    /// Configure the OAuth client through [`OidcConfig::with_client_id`]. The
    /// expected nonce must be non-empty. The caller owns nonce generation,
    /// storage, and single-use login state. Signature, issuer, client audience,
    /// `azp`, expiry, and configured claim checks also apply.
    ///
    /// # Errors
    ///
    /// Returns [`VerificationError::WrongNonce`] for an empty or mismatched nonce and
    /// [`VerificationError::MissingClientId`] when the config has no client ID.
    /// Otherwise fails with the same checks as [`Self::verify`].
    pub async fn verify_id_token(
        &self,
        token: &str,
        expected_nonce: &str,
    ) -> Result<Principal, VerificationError> {
        if expected_nonce.is_empty() {
            return Err(VerificationError::WrongNonce);
        }
        let client_id = self
            .inner
            .config
            .client_id
            .as_deref()
            .ok_or(VerificationError::MissingClientId)?;
        self.verify_token(
            token,
            TokenVerification::Id {
                client_id,
                expected_nonce: Some(expected_nonce),
            },
        )
        .await
    }

    /// Verifies a token-endpoint ID token from a code exchange that sent no nonce.
    #[cfg(feature = "browser-session")]
    pub(crate) async fn verify_exchanged_id_token(
        &self,
        token: &str,
    ) -> Result<Principal, VerificationError> {
        let client_id = self
            .inner
            .config
            .client_id
            .as_deref()
            .ok_or(VerificationError::MissingClientId)?;
        self.verify_token(
            token,
            TokenVerification::Id {
                client_id,
                expected_nonce: None,
            },
        )
        .await
    }

    async fn verify_token(
        &self,
        token: &str,
        verification: TokenVerification<'_>,
    ) -> Result<Principal, VerificationError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| VerificationError::Clock)?
            .as_secs();
        self.verify_token_at(token, verification, now).await
    }

    async fn verify_token_at(
        &self,
        token: &str,
        verification: TokenVerification<'_>,
        now: u64,
    ) -> Result<Principal, VerificationError> {
        let ParsedToken {
            header,
            claims,
            signing_input,
            signature,
        } = ParsedToken::parse(token)?;
        let algorithm = SigningAlgorithm::parse(&header.alg)
            .filter(|algorithm| self.inner.config.algorithms.contains(algorithm))
            .ok_or(VerificationError::DisallowedAlgorithm)?;
        if header.crit.is_some_and(|critical| !critical.is_empty()) {
            return Err(VerificationError::UnsupportedCriticalHeader);
        }
        if matches!(verification, TokenVerification::Access)
            && self.inner.config.require_access_token_type
            && !header.typ.as_deref().is_some_and(is_access_token_type)
        {
            return Err(VerificationError::WrongTokenType);
        }
        let key_id = header.kid.ok_or(VerificationError::MissingKeyId)?;
        let key = self.key_for(&key_id).await?;
        key.verify(algorithm, &signing_input, &signature)?;
        if let TokenVerification::Id {
            expected_nonce: Some(expected_nonce),
            ..
        } = verification
        {
            match claims.extra.get("nonce") {
                Some(Value::String(nonce))
                    if crate::constant_time_eq(nonce.as_bytes(), expected_nonce.as_bytes()) => {}
                _ => return Err(VerificationError::WrongNonce),
            }
        }
        claims.validate(&self.inner.config, verification, now)
    }

    async fn key_for(&self, key_id: &str) -> Result<Jwk, VerificationError> {
        self.inner
            .jwks
            .key_for(&self.inner.source, key_id, self.inner.jwks.now())
            .await
    }

    #[cfg(test)]
    async fn fetch_jwks(&self) -> Result<JwkSet, VerificationError> {
        use jwks::FetchKeys as _;
        self.inner.source.fetch().await
    }
}

/// RFC 9068 media type, compared case-insensitively with or without `application/`.
fn is_access_token_type(typ: &str) -> bool {
    let typ = typ.strip_prefix("application/").unwrap_or(typ);
    typ.eq_ignore_ascii_case("at+jwt")
}

impl IdentityVerifier for OidcVerifier {
    fn verify<'a>(
        &'a self,
        token: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Principal, VerificationError>> + Send + 'a>> {
        Box::pin(OidcVerifier::verify(self, token))
    }
}

impl std::fmt::Debug for OidcVerifier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OidcVerifier")
            .field("issuer", &self.inner.config.issuer.as_str())
            .field("jwks_source", &self.inner.source)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct DiscoveryDocument {
    issuer: String,
    jwks_uri: String,
}

#[derive(Clone, Copy)]
enum TokenVerification<'a> {
    Access,
    Id {
        client_id: &'a str,
        expected_nonce: Option<&'a str>,
    },
}

struct ParsedToken {
    header: JwtHeader,
    claims: JwtClaims,
    signing_input: Vec<u8>,
    signature: Vec<u8>,
}

impl ParsedToken {
    fn parse(token: &str) -> Result<Self, VerificationError> {
        let mut segments = token.split('.');
        let (Some(header), Some(claims), Some(token_signature), None) = (
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next(),
        ) else {
            return Err(VerificationError::MalformedToken);
        };
        if token_signature.is_empty() {
            return Err(VerificationError::InvalidSignature);
        }
        let header_bytes = URL_SAFE_NO_PAD
            .decode(header)
            .map_err(|_| VerificationError::MalformedToken)?;
        let claims_bytes = URL_SAFE_NO_PAD
            .decode(claims)
            .map_err(|_| VerificationError::MalformedToken)?;
        let signature = URL_SAFE_NO_PAD
            .decode(token_signature)
            .map_err(|_| VerificationError::MalformedToken)?;
        Ok(Self {
            header: serde_json::from_slice(&header_bytes)
                .map_err(|_| VerificationError::MalformedToken)?,
            claims: serde_json::from_slice(&claims_bytes)
                .map_err(|_| VerificationError::MalformedToken)?,
            signing_input: format!("{header}.{claims}").into_bytes(),
            signature,
        })
    }
}

#[derive(Deserialize)]
struct JwtHeader {
    alg: String,
    kid: Option<String>,
    typ: Option<String>,
    crit: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct JwtClaims {
    sub: Option<String>,
    iss: Option<String>,
    aud: Option<Audience>,
    exp: Option<u64>,
    nbf: Option<u64>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[cfg(test)]
mod tests;
