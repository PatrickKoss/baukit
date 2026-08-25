use std::{
    collections::{BTreeMap, btree_map::Entry},
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

use crate::{ApiToken, OidcConfig, SigningAlgorithm};

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
    api_token: Option<ApiToken>,
}

impl Principal {
    /// Creates an internal principal without organization or tenant context.
    #[must_use]
    pub fn new(subject: impl Into<String>) -> Self {
        Self {
            subject: subject.into(),
            issuer: None,
            organization: None,
            tenant: None,
            api_token: None,
        }
    }

    pub(crate) fn from_api_token(api_token: ApiToken) -> Self {
        Self {
            subject: api_token.owner_id.to_string(),
            issuer: None,
            organization: None,
            tenant: None,
            api_token: Some(api_token),
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

impl MultiIssuerVerifier {
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

    /// Verifies one access token using the configured issuer, audience, and algorithms.
    pub async fn verify(&self, token: &str) -> Result<Principal, VerificationError> {
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
            .filter(|subject| !subject.is_empty())
            .ok_or(VerificationError::MissingSubject)?;
        if self.iss.as_deref() != Some(config.issuer.as_str()) {
            return Err(VerificationError::WrongIssuer);
        }
        let audience = self.aud.ok_or(VerificationError::WrongAudience)?;
        if !audience
            .values()
            .any(|audience| config.audiences.contains(audience))
        {
            return Err(VerificationError::WrongAudience);
        }
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
        let organization = mapped_claim(&self.extra, config.claim_mapping.organization.as_deref())?;
        let tenant = mapped_claim(&self.extra, config.claim_mapping.tenant.as_deref())?;
        Ok(Principal {
            subject,
            issuer: Some(config.issuer().to_owned()),
            organization,
            tenant,
            api_token: None,
        })
    }
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
    claim_name: Option<&str>,
) -> Result<Option<String>, VerificationError> {
    let Some(claim_name) = claim_name else {
        return Ok(None);
    };
    match claims.get(claim_name) {
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
    /// A configured principal-context claim was not a non-empty string.
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
        assert_eq!(principal.api_token(), None);
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
