use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::{Client, StatusCode, Url};
use ring::{digest, signature};
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::config::ClaimPath;
use crate::{
    ApiToken, ClerkVerifier, OidcConfig, SigningAlgorithm, WorkOsVerifier, config::TokenProfile,
};

const UNKNOWN_KEY_TTL: std::time::Duration = std::time::Duration::from_secs(30);
const MAX_UNKNOWN_KEYS: usize = 128;

/// A verified, provider-neutral application identity.
///
/// Only the stable subject and explicitly configured context cross the auth
/// boundary. API-token principals also retain the verified stored token
/// metadata. Raw JWT claims are deliberately not exposed.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Principal {
    subject: String,
    issuer: Option<String>,
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
            organization: None,
            tenant: None,
            client_id: None,
            scopes: BTreeSet::new(),
            profile_claims: BTreeMap::new(),
            api_token: None,
        }
    }

    pub(crate) fn from_api_token(api_token: ApiToken) -> Self {
        let subject = api_token.owner_id.to_string();
        Self {
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

    /// Returns whether an exact normalized issuer is configured.
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
    /// The same normalized issuer was configured more than once.
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
    client: Client,
    jwks_uri: Url,
    cache: Mutex<JwksCache>,
}

#[derive(Default)]
struct JwksCache {
    fetched_at: Option<Instant>,
    set: JwkSet,
    unknown_keys: BTreeMap<[u8; 32], Instant>,
}

impl OidcVerifier {
    /// Constructs a verifier with an explicit JWKS endpoint.
    ///
    /// Use this when the token issuer is a public URL but key retrieval must use
    /// a distinct private-network URL. Token issuer validation remains bound to
    /// [`OidcConfig::issuer`]; only discovery is bypassed.
    pub fn from_jwks_uri(
        config: OidcConfig,
        jwks_uri: impl AsRef<str>,
    ) -> Result<Self, VerificationError> {
        let client = Client::builder()
            .timeout(config.request_timeout)
            .build()
            .map_err(VerificationError::Client)?;
        let jwks_uri =
            Url::parse(jwks_uri.as_ref()).map_err(|_| VerificationError::InvalidJwksUri)?;
        Ok(Self {
            inner: Arc::new(VerifierInner {
                config,
                client,
                jwks_uri,
                cache: Mutex::new(JwksCache::default()),
            }),
        })
    }

    /// Discovers the configured issuer and constructs a verifier.
    ///
    /// The discovery response must report the exact configured issuer. JWKS are
    /// fetched lazily on first verification and then cached according to the
    /// configured TTL.
    pub async fn discover(config: OidcConfig) -> Result<Self, VerificationError> {
        let client = Client::builder()
            .timeout(config.request_timeout)
            .build()
            .map_err(VerificationError::Client)?;
        let discovery_url = config.discovery_url();
        let response = client
            .get(discovery_url)
            .send()
            .await
            .map_err(discovery_request_error)?;
        if !response.status().is_success() {
            return Err(VerificationError::DiscoveryStatus(response.status()));
        }
        let metadata: DiscoveryDocument = response
            .json()
            .await
            .map_err(VerificationError::InvalidDiscoveryDocument)?;
        if metadata.issuer != config.issuer.as_str() {
            return Err(VerificationError::DiscoveryIssuerMismatch);
        }
        let jwks_uri =
            Url::parse(&metadata.jwks_uri).map_err(|_| VerificationError::InvalidJwksUri)?;
        Ok(Self {
            inner: Arc::new(VerifierInner {
                config,
                client,
                jwks_uri,
                cache: Mutex::new(JwksCache::default()),
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
                config,
                client: self.inner.client.clone(),
                jwks_uri: self.inner.jwks_uri.clone(),
                cache: Mutex::new(JwksCache::default()),
            }),
        }
    }

    /// Verifies one access token using the configured issuer, audience, and algorithms.
    pub async fn verify(&self, token: &str) -> Result<Principal, VerificationError> {
        self.verify_token(token, None).await
    }

    /// Verifies an ID token and requires its `nonce` to match the login request.
    ///
    /// The expected nonce must be non-empty. The caller owns nonce generation,
    /// storage, and single-use login state. Signature, issuer, audience, `azp`,
    /// expiry, and configured claim checks also apply.
    pub async fn verify_id_token(
        &self,
        token: &str,
        expected_nonce: &str,
    ) -> Result<Principal, VerificationError> {
        if expected_nonce.is_empty() {
            return Err(VerificationError::WrongNonce);
        }
        self.verify_token(token, Some(expected_nonce)).await
    }

    async fn verify_token(
        &self,
        token: &str,
        expected_nonce: Option<&str>,
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
        let key_id = header.kid.ok_or(VerificationError::MissingKeyId)?;
        let key = self.key_for(&key_id).await?;
        key.verify(algorithm, &signing_input, &signature)?;
        if let Some(expected_nonce) = expected_nonce {
            match claims.extra.get("nonce") {
                Some(Value::String(nonce))
                    if crate::constant_time_eq(nonce.as_bytes(), expected_nonce.as_bytes()) => {}
                _ => return Err(VerificationError::WrongNonce),
            }
        }
        claims.validate(&self.inner.config)
    }

    async fn key_for(&self, key_id: &str) -> Result<Jwk, VerificationError> {
        let mut cache = self.inner.cache.lock().await;
        let fresh = cache
            .fetched_at
            .is_some_and(|fetched_at| fetched_at.elapsed() < self.inner.config.cache_ttl);
        if fresh && let Some(key) = cache.set.find(key_id).cloned() {
            cache.unknown_keys.remove(&unknown_key_hash(key_id));
            return Ok(key);
        }

        if fresh
            && cache
                .unknown_keys
                .get(&unknown_key_hash(key_id))
                .is_some_and(|cached_at| cached_at.elapsed() < UNKNOWN_KEY_TTL)
        {
            return Err(VerificationError::UnknownKeyId);
        }

        // A missing kid refreshes even a fresh cache, which handles provider key
        // rotation without waiting for the normal TTL. Holding the mutex avoids
        // a request stampede during refresh.
        let set = self.fetch_jwks().await?;
        let key = set.find(key_id).cloned();
        cache.set = set;
        cache.fetched_at = Some(Instant::now());
        if let Some(key) = key {
            cache.unknown_keys.remove(&unknown_key_hash(key_id));
            Ok(key)
        } else {
            cache.remember_unknown_key(key_id);
            Err(VerificationError::UnknownKeyId)
        }
    }

    async fn fetch_jwks(&self) -> Result<JwkSet, VerificationError> {
        let response = self
            .inner
            .client
            .get(self.inner.jwks_uri.clone())
            .send()
            .await
            .map_err(jwks_request_error)?;
        if !response.status().is_success() {
            return Err(VerificationError::JwksStatus(response.status()));
        }
        response
            .json()
            .await
            .map_err(VerificationError::InvalidJwksDocument)
    }
}

impl JwksCache {
    fn remember_unknown_key(&mut self, key_id: &str) {
        self.unknown_keys
            .retain(|_, cached_at| cached_at.elapsed() < UNKNOWN_KEY_TTL);
        if self.unknown_keys.len() >= MAX_UNKNOWN_KEYS
            && let Some(oldest) = self
                .unknown_keys
                .iter()
                .min_by_key(|(_, cached_at)| *cached_at)
                .map(|(key_id, _)| *key_id)
        {
            self.unknown_keys.remove(&oldest);
        }
        self.unknown_keys
            .insert(unknown_key_hash(key_id), Instant::now());
    }
}

fn unknown_key_hash(key_id: &str) -> [u8; 32] {
    let hash = digest::digest(&digest::SHA256, key_id.as_bytes());
    let mut bytes = [0; 32];
    bytes.copy_from_slice(hash.as_ref());
    bytes
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
            .field("jwks_uri", &self.inner.jwks_uri)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct DiscoveryDocument {
    issuer: String,
    jwks_uri: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct JwkSet {
    keys: Vec<Jwk>,
}

impl JwkSet {
    fn find(&self, key_id: &str) -> Option<&Jwk> {
        self.keys
            .iter()
            .find(|key| key.kid.as_deref() == Some(key_id))
    }
}

#[derive(Clone, Debug, Deserialize)]
struct Jwk {
    kty: String,
    kid: Option<String>,
    #[serde(rename = "use")]
    key_use: Option<String>,
    key_ops: Option<Vec<String>>,
    alg: Option<String>,
    n: Option<String>,
    e: Option<String>,
    crv: Option<String>,
    x: Option<String>,
    y: Option<String>,
}

impl Jwk {
    fn verify(
        &self,
        algorithm: SigningAlgorithm,
        message: &[u8],
        token_signature: &[u8],
    ) -> Result<(), VerificationError> {
        if self.key_use.as_deref().is_some_and(|usage| usage != "sig")
            || self
                .key_ops
                .as_ref()
                .is_some_and(|operations| !operations.iter().any(|operation| operation == "verify"))
            || self
                .alg
                .as_deref()
                .is_some_and(|alg| alg != algorithm.name())
        {
            return Err(VerificationError::KeyNotUsable);
        }
        match algorithm {
            SigningAlgorithm::Rs256 => self.verify_rsa(
                &signature::RSA_PKCS1_2048_8192_SHA256,
                message,
                token_signature,
            ),
            SigningAlgorithm::Rs384 => self.verify_rsa(
                &signature::RSA_PKCS1_2048_8192_SHA384,
                message,
                token_signature,
            ),
            SigningAlgorithm::Rs512 => self.verify_rsa(
                &signature::RSA_PKCS1_2048_8192_SHA512,
                message,
                token_signature,
            ),
            SigningAlgorithm::Ps256 => self.verify_rsa(
                &signature::RSA_PSS_2048_8192_SHA256,
                message,
                token_signature,
            ),
            SigningAlgorithm::Ps384 => self.verify_rsa(
                &signature::RSA_PSS_2048_8192_SHA384,
                message,
                token_signature,
            ),
            SigningAlgorithm::Ps512 => self.verify_rsa(
                &signature::RSA_PSS_2048_8192_SHA512,
                message,
                token_signature,
            ),
            SigningAlgorithm::Es256 => self.verify_ec(
                "P-256",
                &signature::ECDSA_P256_SHA256_FIXED,
                message,
                token_signature,
            ),
            SigningAlgorithm::Es384 => self.verify_ec(
                "P-384",
                &signature::ECDSA_P384_SHA384_FIXED,
                message,
                token_signature,
            ),
            SigningAlgorithm::EdDsa => {
                if self.kty != "OKP" || self.crv.as_deref() != Some("Ed25519") {
                    return Err(VerificationError::KeyAlgorithmMismatch);
                }
                let public_key = decode_component(self.x.as_deref())?;
                signature::UnparsedPublicKey::new(&signature::ED25519, public_key)
                    .verify(message, token_signature)
                    .map_err(|_| VerificationError::InvalidSignature)
            }
        }
    }

    fn verify_rsa(
        &self,
        algorithm: &'static signature::RsaParameters,
        message: &[u8],
        token_signature: &[u8],
    ) -> Result<(), VerificationError> {
        if self.kty != "RSA" {
            return Err(VerificationError::KeyAlgorithmMismatch);
        }
        let modulus = decode_component(self.n.as_deref())?;
        let exponent = decode_component(self.e.as_deref())?;
        signature::RsaPublicKeyComponents {
            n: &modulus,
            e: &exponent,
        }
        .verify(algorithm, message, token_signature)
        .map_err(|_| VerificationError::InvalidSignature)
    }

    fn verify_ec(
        &self,
        curve: &str,
        algorithm: &'static dyn signature::VerificationAlgorithm,
        message: &[u8],
        token_signature: &[u8],
    ) -> Result<(), VerificationError> {
        if self.kty != "EC" || self.crv.as_deref() != Some(curve) {
            return Err(VerificationError::KeyAlgorithmMismatch);
        }
        let x = decode_component(self.x.as_deref())?;
        let y = decode_component(self.y.as_deref())?;
        let mut public_key = Vec::with_capacity(1 + x.len() + y.len());
        public_key.push(4);
        public_key.extend_from_slice(&x);
        public_key.extend_from_slice(&y);
        signature::UnparsedPublicKey::new(algorithm, public_key)
            .verify(message, token_signature)
            .map_err(|_| VerificationError::InvalidSignature)
    }
}

fn decode_component(value: Option<&str>) -> Result<Vec<u8>, VerificationError> {
    URL_SAFE_NO_PAD
        .decode(value.ok_or(VerificationError::InvalidJwk)?)
        .map_err(|_| VerificationError::InvalidJwk)
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

impl JwtClaims {
    fn validate(self, config: &OidcConfig) -> Result<Principal, VerificationError> {
        let subject = self
            .sub
            .as_deref()
            .filter(|subject| !subject.is_empty())
            .map(str::to_owned)
            .ok_or(VerificationError::MissingSubject)?;
        if self.iss.as_deref() != Some(config.issuer.as_str()) {
            return Err(VerificationError::WrongIssuer);
        }
        validate_token_profile(&self, config)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| VerificationError::Clock)?
            .as_secs();
        let skew = config.clock_skew.as_secs();
        let expiry = self.exp.ok_or(VerificationError::MissingExpiry)?;
        if now > expiry.saturating_add(skew) {
            return Err(VerificationError::Expired);
        }
        if self
            .nbf
            .is_some_and(|not_before| not_before > now.saturating_add(skew))
        {
            return Err(VerificationError::NotYetValid);
        }
        let organization = mapped_claim(&self.extra, config.claim_mapping.organization.as_ref())?;
        let tenant = mapped_claim(&self.extra, config.claim_mapping.tenant.as_ref())?;
        let client_id = mapped_claim(&self.extra, config.claim_mapping.client_id.as_ref())?;
        let scopes = scope_claim(self.extra.get(config.claim_mapping.scope.as_ref()))?;
        let profile_claims = selected_profile_claims(&self.extra, &config.claim_mapping.profile);
        Ok(Principal {
            subject,
            issuer: Some(config.issuer().to_owned()),
            organization,
            tenant,
            client_id,
            scopes,
            profile_claims,
            api_token: None,
        })
    }
}

fn validate_token_profile(
    claims: &JwtClaims,
    config: &OidcConfig,
) -> Result<(), VerificationError> {
    if !config.audiences.is_empty() {
        validate_audience(claims, config)?;
    }
    match &config.token_profile {
        TokenProfile::Oidc => validate_oidc_authorized_party(claims, config),
        TokenProfile::Clerk { authorized_parties } => match claims.extra.get("azp") {
            None | Some(Value::Null) => Ok(()),
            Some(Value::String(value)) if authorized_parties.contains(value) => Ok(()),
            Some(Value::String(_)) => Err(VerificationError::WrongAuthorizedParty),
            Some(_) => Err(VerificationError::InvalidPrincipalContext),
        },
        TokenProfile::WorkOs { client_id } => match claims.extra.get("client_id") {
            Some(Value::String(value)) if value == client_id => Ok(()),
            Some(Value::String(_)) | None | Some(Value::Null) => {
                Err(VerificationError::WrongClientId)
            }
            Some(_) => Err(VerificationError::InvalidPrincipalContext),
        },
    }
}

fn validate_oidc_authorized_party(
    claims: &JwtClaims,
    config: &OidcConfig,
) -> Result<(), VerificationError> {
    let several_audiences = claims
        .aud
        .as_ref()
        .is_some_and(|audience| audience.values().count() > 1);
    match claims.extra.get("azp") {
        None if !several_audiences => Ok(()),
        Some(Value::String(client))
            if config.allowed_clients.contains(client)
                && (!several_audiences || client == &config.client_id) =>
        {
            Ok(())
        }
        _ => Err(VerificationError::WrongAuthorizedParty),
    }
}

fn validate_audience(claims: &JwtClaims, config: &OidcConfig) -> Result<(), VerificationError> {
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

fn scope_claim(value: Option<&Value>) -> Result<BTreeSet<String>, VerificationError> {
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

fn selected_profile_claims(
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
enum Audience {
    One(String),
    Many(Vec<String>),
}

impl Audience {
    fn values(&self) -> Box<dyn Iterator<Item = &String> + '_> {
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

fn discovery_request_error(error: reqwest::Error) -> VerificationError {
    if error.is_timeout() {
        VerificationError::DiscoveryTimeout
    } else {
        VerificationError::DiscoveryRequest(error)
    }
}

fn jwks_request_error(error: reqwest::Error) -> VerificationError {
    if error.is_timeout() {
        VerificationError::JwksTimeout
    } else {
        VerificationError::JwksRequest(error)
    }
}

/// Failure while discovering an issuer or verifying an access token.
#[derive(Debug, Error)]
pub enum VerificationError {
    /// The HTTP client could not be created.
    #[error("could not create OIDC HTTP client: {0}")]
    Client(#[source] reqwest::Error),
    /// OIDC discovery exceeded the configured timeout.
    #[error("OIDC discovery timed out")]
    DiscoveryTimeout,
    /// OIDC discovery could not be requested.
    #[error("OIDC discovery request failed: {0}")]
    DiscoveryRequest(#[source] reqwest::Error),
    /// OIDC discovery returned a non-success status.
    #[error("OIDC discovery returned HTTP {0}")]
    DiscoveryStatus(StatusCode),
    /// OIDC discovery returned invalid JSON.
    #[error("OIDC discovery document was invalid: {0}")]
    InvalidDiscoveryDocument(#[source] reqwest::Error),
    /// Discovery metadata did not report the configured issuer exactly.
    #[error("OIDC discovery issuer did not match configuration")]
    DiscoveryIssuerMismatch,
    /// Discovery metadata contained an invalid JWKS URL.
    #[error("OIDC discovery returned an invalid JWKS URI")]
    InvalidJwksUri,
    /// A JWKS request exceeded the configured timeout.
    #[error("JWKS request timed out")]
    JwksTimeout,
    /// JWKS could not be requested.
    #[error("JWKS request failed: {0}")]
    JwksRequest(#[source] reqwest::Error),
    /// JWKS returned a non-success status.
    #[error("JWKS returned HTTP {0}")]
    JwksStatus(StatusCode),
    /// JWKS returned invalid JSON.
    #[error("JWKS document was invalid: {0}")]
    InvalidJwksDocument(#[source] reqwest::Error),
    /// The JWT did not have three valid base64url-encoded segments.
    #[error("access token was malformed")]
    MalformedToken,
    /// The JWT signing algorithm was not in the configured allowlist.
    #[error("access token signing algorithm is not allowed")]
    DisallowedAlgorithm,
    /// A critical JWT header is unsupported.
    #[error("access token contains unsupported critical headers")]
    UnsupportedCriticalHeader,
    /// The JWT omitted its key identifier.
    #[error("access token has no key identifier")]
    MissingKeyId,
    /// No current provider key matched the JWT key identifier.
    #[error("access token key identifier is unknown")]
    UnknownKeyId,
    /// The matching JWK cannot be used for signature verification.
    #[error("provider key is not usable for signature verification")]
    KeyNotUsable,
    /// The JWK type does not match the JWT algorithm.
    #[error("provider key type does not match the signing algorithm")]
    KeyAlgorithmMismatch,
    /// A required JWK component was missing or malformed.
    #[error("provider key is invalid")]
    InvalidJwk,
    /// JWT signature verification failed.
    #[error("access token signature is invalid")]
    InvalidSignature,
    /// The subject claim was absent or empty.
    #[error("access token subject is missing")]
    MissingSubject,
    /// The issuer claim did not match configuration.
    #[error("access token issuer is invalid")]
    WrongIssuer,
    /// The token names an issuer outside the configured allowlist.
    #[error("access token issuer is not configured")]
    UnconfiguredIssuer,
    /// No token audience matched configuration.
    #[error("access token audience is invalid")]
    WrongAudience,
    /// The authorized party was missing when required or outside the allowed clients.
    #[error("access token authorized party is invalid")]
    WrongAuthorizedParty,
    /// The ID token's nonce was missing, malformed, or did not match the login request.
    #[error("ID token nonce is invalid")]
    WrongNonce,
    /// A WorkOS token did not name the configured application client.
    #[error("access token client ID is invalid")]
    WrongClientId,
    /// The required expiry claim was absent.
    #[error("access token expiry is missing")]
    MissingExpiry,
    /// The token has expired outside the configured clock skew.
    #[error("access token has expired")]
    Expired,
    /// The token is not valid yet outside the configured clock skew.
    #[error("access token is not valid yet")]
    NotYetValid,
    /// The system clock is before the Unix epoch.
    #[error("system clock is invalid")]
    Clock,
    /// A configured principal-context claim was not a non-empty string, or the
    /// scope claim was neither a string nor an array of non-empty strings.
    #[error("principal context claim has an invalid shape")]
    InvalidPrincipalContext,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsigned_and_malformed_tokens_are_rejected_before_key_lookup() {
        assert!(matches!(
            ParsedToken::parse("abc.def"),
            Err(VerificationError::MalformedToken)
        ));
        assert!(matches!(
            ParsedToken::parse("eyJhbGciOiJub25lIn0.e30."),
            Err(VerificationError::InvalidSignature)
        ));
    }

    #[test]
    fn principal_exposes_only_normalized_fields() {
        let principal = Principal::new("subject")
            .with_organization("org")
            .with_tenant("tenant");
        assert_eq!(principal.subject(), "subject");
        assert_eq!(principal.issuer(), None);
        assert_eq!(principal.organization(), Some("org"));
        assert_eq!(principal.tenant(), Some("tenant"));
        assert_eq!(principal.client_id(), None);
        assert!(principal.scopes().is_empty());
        assert_eq!(principal.grants(), None);
        assert!(principal.profile_claims().is_empty());
        assert_eq!(principal.api_token(), None);
    }

    #[test]
    fn scope_claims_accept_strings_and_string_arrays() {
        let expected = BTreeSet::from(["read".to_owned(), "write".to_owned()]);
        assert_eq!(
            scope_claim(Some(&Value::from(" read  write read "))).ok(),
            Some(expected.clone())
        );
        assert_eq!(
            scope_claim(Some(&serde_json::json!(["read", "write"]))).ok(),
            Some(expected)
        );
        assert_eq!(scope_claim(None).ok(), Some(BTreeSet::new()));
        assert_eq!(scope_claim(Some(&Value::Null)).ok(), Some(BTreeSet::new()));
        for invalid in [
            serde_json::json!(42),
            serde_json::json!(["read", 1]),
            serde_json::json!([""]),
            serde_json::json!({"read": true}),
        ] {
            assert!(matches!(
                scope_claim(Some(&invalid)),
                Err(VerificationError::InvalidPrincipalContext)
            ));
        }
    }

    #[test]
    fn profile_claims_copy_only_selected_strings_and_booleans() {
        let claims = BTreeMap::from([
            ("email".to_owned(), Value::from("ada@example.com")),
            ("email_verified".to_owned(), Value::from(true)),
            ("preferred_username".to_owned(), Value::from(42)),
            ("phone_number".to_owned(), Value::from("+100")),
        ]);
        let selected = BTreeSet::from([
            "email".to_owned(),
            "email_verified".to_owned(),
            "preferred_username".to_owned(),
            "name".to_owned(),
        ]);
        let profile = selected_profile_claims(&claims, &selected);
        assert_eq!(
            profile,
            BTreeMap::from([
                (
                    "email".to_owned(),
                    ProfileClaim::String("ada@example.com".to_owned())
                ),
                ("email_verified".to_owned(), ProfileClaim::Bool(true)),
            ])
        );
        assert_eq!(profile["email"].as_str(), Some("ada@example.com"));
        assert_eq!(profile["email_verified"].as_bool(), Some(true));
    }

    #[test]
    fn unknown_key_cache_has_a_hard_entry_bound() {
        let mut cache = JwksCache::default();
        for index in 0..=MAX_UNKNOWN_KEYS {
            cache.remember_unknown_key(&format!("unknown-{index}"));
        }
        assert_eq!(cache.unknown_keys.len(), MAX_UNKNOWN_KEYS);
        assert!(
            cache
                .unknown_keys
                .contains_key(&unknown_key_hash("unknown-128"))
        );
    }
}
