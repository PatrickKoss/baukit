use std::{
    future::Future,
    sync::{
        Arc, Mutex, PoisonError, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use aws_lc_rs::signature;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use baukit_egress::GuardedClient;
use reqwest::Url;
use serde::Deserialize;
use serde_json::Value;

use super::VerificationError;
use crate::kernels::jwks_refresh_allowed;
use crate::oidc_http::{self, Document};
use crate::{OidcConfig, SigningAlgorithm};

const AUTH_KEY_CACHE_REQUESTS_TOTAL: &str = "auth_key_cache_requests_total";
const OIDC_SOURCE: &str = "oidc";
const LOOKUP_FRESH: &str = "fresh";
const LOOKUP_STALE: &str = "stale";
const LOOKUP_FETCHED: &str = "fetched";
const LOOKUP_FAILED: &str = "failed";

fn record(result: &'static str) {
    metrics::counter!(
        AUTH_KEY_CACHE_REQUESTS_TOTAL,
        "source" => OIDC_SOURCE,
        "result" => result
    )
    .increment(1);
}

/// Fetches the provider's current key set.
pub(crate) trait FetchKeys: Sync {
    fn fetch(&self) -> impl Future<Output = Result<JwkSet, VerificationError>> + Send;
}

#[derive(Clone)]
pub(crate) enum KeySource {
    Http(Arc<HttpKeySource>),
    Static(Arc<JwkSet>),
}

pub(crate) struct HttpKeySource {
    pub(crate) client: GuardedClient,
    pub(crate) jwks_uri: Url,
}

impl FetchKeys for KeySource {
    async fn fetch(&self) -> Result<JwkSet, VerificationError> {
        match self {
            Self::Http(http) => {
                oidc_http::get_json(&http.client, http.jwks_uri.clone(), Document::Jwks).await
            }
            Self::Static(set) => Ok(JwkSet::clone(set)),
        }
    }
}

impl std::fmt::Debug for KeySource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(http) => http.jwks_uri.fmt(formatter),
            Self::Static(_) => formatter.write_str("static"),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct JwksPolicy {
    pub(crate) ttl: Duration,
    pub(crate) cooldown: Duration,
    pub(crate) stale_grace: Duration,
}

impl JwksPolicy {
    pub(crate) fn from_config(config: &OidcConfig) -> Self {
        Self {
            ttl: config.cache_ttl,
            cooldown: config.jwks_refresh_cooldown.min(config.cache_ttl),
            stale_grace: config.jwks_stale_grace,
        }
    }

    fn usable_until(&self, fetched_at: Duration) -> Duration {
        fetched_at
            .saturating_add(self.ttl)
            .saturating_add(self.stale_grace)
    }
}

struct Snapshot {
    set: JwkSet,
    fetched_at: Duration,
}

/// Lock-free reads of the last fetched key set with single-flight refreshes.
///
/// Known keys never wait on a fetch. An unknown `kid` may refetch at most once
/// per cooldown across all kids. After the TTL a failed refresh keeps serving
/// the stale set until the grace runs out.
pub(crate) struct JwksCache {
    origin: Instant,
    policy: JwksPolicy,
    snapshot: RwLock<Option<Arc<Snapshot>>>,
    last_attempt: Mutex<Option<Duration>>,
    attempts: AtomicU64,
    refresh: tokio::sync::Mutex<()>,
}

impl JwksCache {
    pub(crate) fn new(policy: JwksPolicy) -> Self {
        Self {
            origin: Instant::now(),
            policy,
            snapshot: RwLock::new(None),
            last_attempt: Mutex::new(None),
            attempts: AtomicU64::new(0),
            refresh: tokio::sync::Mutex::new(()),
        }
    }

    /// Returns the monotonic time this cache measures against.
    pub(crate) fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    pub(crate) async fn key_for<S: FetchKeys>(
        &self,
        source: &S,
        key_id: &str,
        now: Duration,
    ) -> Result<Jwk, VerificationError> {
        let attempts = self.attempts.load(Ordering::Acquire);
        let Some(snapshot) = self.usable_snapshot(now) else {
            return self.fetch_required(source, key_id, now, attempts).await;
        };
        match snapshot.set.find(key_id) {
            Some(key) if now.saturating_sub(snapshot.fetched_at) < self.policy.ttl => {
                record(LOOKUP_FRESH);
                Ok(key.clone())
            }
            Some(key) => self.refresh_stale(source, key_id, key, now).await,
            None => self.refresh_unknown(source, key_id, now, attempts).await,
        }
    }

    fn current(&self) -> Option<Arc<Snapshot>> {
        self.snapshot
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn usable_snapshot(&self, now: Duration) -> Option<Arc<Snapshot>> {
        self.current()
            .filter(|snapshot| now < self.policy.usable_until(snapshot.fetched_at))
    }

    fn claim_refresh(&self, now: Duration) -> bool {
        let mut last = self
            .last_attempt
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !jwks_refresh_allowed(*last, now, self.policy.cooldown) {
            return false;
        }
        *last = Some(now);
        true
    }

    async fn fetch_required<S: FetchKeys>(
        &self,
        source: &S,
        key_id: &str,
        now: Duration,
        attempts: u64,
    ) -> Result<Jwk, VerificationError> {
        let _flight = self.refresh.lock().await;
        if self.attempts.load(Ordering::Acquire) != attempts {
            let snapshot = self
                .usable_snapshot(now)
                .ok_or(VerificationError::Unavailable)?;
            return snapshot
                .set
                .find(key_id)
                .cloned()
                .ok_or(VerificationError::UnknownKeyId);
        }
        {
            let mut last = self
                .last_attempt
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if !jwks_refresh_allowed(*last, now, self.policy.cooldown) {
                return Err(VerificationError::Unavailable);
            }
            *last = Some(now);
        }
        let set = self.fetch_and_store(source, now).await?;
        set.find(key_id)
            .cloned()
            .ok_or(VerificationError::UnknownKeyId)
    }

    async fn refresh_unknown<S: FetchKeys>(
        &self,
        source: &S,
        key_id: &str,
        now: Duration,
        attempts: u64,
    ) -> Result<Jwk, VerificationError> {
        let cooldown_over = {
            let last = self
                .last_attempt
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            jwks_refresh_allowed(*last, now, self.policy.cooldown)
        };
        if !cooldown_over {
            return Err(VerificationError::UnknownKeyId);
        }
        let _flight = self.refresh.lock().await;
        if self.attempts.load(Ordering::Acquire) != attempts || !self.claim_refresh(now) {
            return self
                .current()
                .and_then(|snapshot| snapshot.set.find(key_id).cloned())
                .ok_or(VerificationError::UnknownKeyId);
        }
        let set = self.fetch_and_store(source, now).await?;
        set.find(key_id)
            .cloned()
            .ok_or(VerificationError::UnknownKeyId)
    }

    async fn refresh_stale<S: FetchKeys>(
        &self,
        source: &S,
        key_id: &str,
        stale: &Jwk,
        now: Duration,
    ) -> Result<Jwk, VerificationError> {
        let Ok(_flight) = self.refresh.try_lock() else {
            record(LOOKUP_STALE);
            return Ok(stale.clone());
        };
        if !self.claim_refresh(now) {
            record(LOOKUP_STALE);
            return Ok(stale.clone());
        }
        match self.fetch_and_store(source, now).await {
            Ok(set) => set
                .find(key_id)
                .cloned()
                .ok_or(VerificationError::UnknownKeyId),
            Err(error) => {
                tracing::warn!(error = %error, "JWKS refresh failed; serving the stale key set");
                record(LOOKUP_STALE);
                Ok(stale.clone())
            }
        }
    }

    async fn fetch_and_store<S: FetchKeys>(
        &self,
        source: &S,
        now: Duration,
    ) -> Result<Arc<Snapshot>, VerificationError> {
        let result = source.fetch().await;
        self.attempts.fetch_add(1, Ordering::AcqRel);
        record(if result.is_ok() {
            LOOKUP_FETCHED
        } else {
            LOOKUP_FAILED
        });
        let snapshot = Arc::new(Snapshot {
            set: result?,
            fetched_at: now,
        });
        *self
            .snapshot
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(&snapshot));
        Ok(snapshot)
    }
}

impl Snapshot {
    fn find(&self, key_id: &str) -> Option<&Jwk> {
        self.set.find(key_id)
    }
}

/// A provider key set. Keys that fail to parse are skipped and logged.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(from = "RawJwkSet")]
pub(crate) struct JwkSet {
    keys: Vec<Jwk>,
}

#[derive(Deserialize)]
struct RawJwkSet {
    keys: Vec<Value>,
}

impl From<RawJwkSet> for JwkSet {
    fn from(raw: RawJwkSet) -> Self {
        let keys = raw
            .keys
            .into_iter()
            .filter_map(|key| {
                serde_json::from_value::<Jwk>(key)
                    .map_err(|error| tracing::warn!(error = %error, "skipping unparseable JWK"))
                    .ok()
            })
            .collect();
        Self { keys }
    }
}

impl JwkSet {
    pub(crate) fn find(&self, key_id: &str) -> Option<&Jwk> {
        self.keys
            .iter()
            .find(|key| key.kid.as_deref() == Some(key_id))
    }
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Jwk {
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
    pub(crate) fn verify(
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

#[cfg(test)]
#[path = "jwks_tests.rs"]
mod tests;
