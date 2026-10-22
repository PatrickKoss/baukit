use async_trait::async_trait;
use aws_lc_rs::signature::{ED25519, UnparsedPublicKey};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use baukit_egress::{EgressRequest, GuardedClient};
use serde::{Deserialize, Deserializer, Serialize};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::{sync::Mutex, time::Instant};
use url::Url;

pub const MAX_TICKET_LIFETIME_SECONDS: i64 = 7 * 24 * 60 * 60;
pub const TICKET_IAT_LEEWAY_SECONDS: i64 = 60;
const JWKS_TTL: Duration = Duration::from_secs(300);
const REFRESH_COOLDOWN: Duration = Duration::from_secs(5);
const MAX_JWKS_BYTES: usize = 64 * 1024;
const MAX_JWKS_KEYS: usize = 64;
const MAX_TICKET_BYTES: usize = 32 * 1024;
const MAX_KID_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum TicketError {
    #[error("ticket_invalid")]
    Invalid,
    #[error("ticket_expired")]
    Expired,
    #[error("ticket_wrong_audience")]
    WrongAudience,
    #[error("ticket_keys_unavailable")]
    KeysUnavailable,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TicketClaims {
    pub iss: String,
    pub sub: String,
    pub aud: String,
    pub tier: String,
    pub features: Vec<String>,
    pub rev: u64,
    pub iat: i64,
    pub exp: i64,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "grace"
    )]
    pub grace: Option<i64>,
}
fn grace<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    i64::deserialize(d).map(Some)
}
impl TicketClaims {
    pub fn verify_claims(&self, issuer: &str, audience: &str, now: i64) -> Result<(), TicketError> {
        if self.iss.is_empty()
            || self.iss != issuer
            || self.sub.is_empty()
            || self.aud.is_empty()
            || self.tier.is_empty()
            || self.iat < 0
            || self.iat > now.saturating_add(TICKET_IAT_LEEWAY_SECONDS)
            || self.exp <= self.iat
            || self
                .exp
                .checked_sub(self.iat)
                .is_none_or(|age| age > MAX_TICKET_LIFETIME_SECONDS)
        {
            return Err(TicketError::Invalid);
        }
        if self.exp <= now {
            return Err(TicketError::Expired);
        }
        if self.aud != audience {
            return Err(TicketError::WrongAudience);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TicketJwk {
    pub kid: String,
    pub kty: String,
    pub crv: String,
    pub x: String,
    pub alg: Option<String>,
    #[serde(rename = "use")]
    pub key_use: Option<String>,
    pub key_ops: Option<Vec<String>>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct TicketJwks {
    pub keys: Vec<TicketJwk>,
}
impl TicketJwks {
    fn by_kid(&self) -> Result<BTreeMap<String, TicketJwk>, TicketError> {
        if self.keys.len() > MAX_JWKS_KEYS {
            return Err(TicketError::Invalid);
        }
        let mut keys = BTreeMap::new();
        for key in &self.keys {
            if key.kid.is_empty()
                || key.kid.len() > MAX_KID_BYTES
                || keys.insert(key.kid.clone(), key.clone()).is_some()
            {
                return Err(TicketError::Invalid);
            }
        }
        Ok(keys)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    alg: String,
    kid: String,
    typ: Option<String>,
}
struct ParsedTicket<'a> {
    kid: String,
    input: &'a str,
    payload: Vec<u8>,
    signature: Vec<u8>,
}
fn decode(s: &str) -> Result<Vec<u8>, TicketError> {
    URL_SAFE_NO_PAD.decode(s).map_err(|_| TicketError::Invalid)
}
impl<'a> ParsedTicket<'a> {
    fn parse(ticket: &'a str) -> Result<Self, TicketError> {
        if ticket.len() > MAX_TICKET_BYTES {
            return Err(TicketError::Invalid);
        }
        let mut parts = ticket.split('.');
        let (Some(header), Some(payload), Some(signature), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(TicketError::Invalid);
        };
        let header: Header =
            serde_json::from_slice(&decode(header)?).map_err(|_| TicketError::Invalid)?;
        if header.alg != "EdDSA"
            || header.kid.is_empty()
            || header.kid.len() > MAX_KID_BYTES
            || header.typ.is_some_and(|typ| typ != "JWT")
        {
            return Err(TicketError::Invalid);
        }
        let (input, _) = ticket.rsplit_once('.').ok_or(TicketError::Invalid)?;
        Ok(Self {
            kid: header.kid,
            input,
            payload: decode(payload)?,
            signature: decode(signature)?,
        })
    }
    fn verify(
        &self,
        key: &TicketJwk,
        issuer: &str,
        audience: &str,
        now: i64,
    ) -> Result<TicketClaims, TicketError> {
        if key.kty != "OKP"
            || key.crv != "Ed25519"
            || key.alg.as_deref().is_some_and(|alg| alg != "EdDSA")
            || key.key_use.as_deref().is_some_and(|usage| usage != "sig")
            || key
                .key_ops
                .as_ref()
                .is_some_and(|ops| !ops.iter().any(|op| op == "verify"))
        {
            return Err(TicketError::Invalid);
        }
        UnparsedPublicKey::new(&ED25519, decode(&key.x)?)
            .verify(self.input.as_bytes(), &self.signature)
            .map_err(|_| TicketError::Invalid)?;
        let claims: TicketClaims =
            serde_json::from_slice(&self.payload).map_err(|_| TicketError::Invalid)?;
        claims.verify_claims(issuer, audience, now)?;
        Ok(claims)
    }
}

/// Offline verification with an already trusted public-key set.
pub fn verify_ticket(
    ticket: &str,
    jwks: &TicketJwks,
    issuer: &str,
    audience: &str,
    now: i64,
) -> Result<TicketClaims, TicketError> {
    let parsed = ParsedTicket::parse(ticket)?;
    let keys = jwks.by_kid()?;
    parsed.verify(
        keys.get(&parsed.kid).ok_or(TicketError::Invalid)?,
        issuer,
        audience,
        now,
    )
}

/// Key retrieval is injectable. Production uses `GuardedJwksSource`.
#[async_trait]
pub trait TicketJwksSource: Send + Sync {
    async fn fetch(&self) -> Result<TicketJwks, TicketError>;
}
pub struct GuardedJwksSource {
    url: Url,
    client: GuardedClient,
}
impl GuardedJwksSource {
    pub fn new(url: &str, client: GuardedClient) -> Result<Self, TicketError> {
        let url = Url::parse(url).map_err(|_| TicketError::Invalid)?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(TicketError::Invalid);
        }
        Ok(Self { url, client })
    }
}
#[async_trait]
impl TicketJwksSource for GuardedJwksSource {
    async fn fetch(&self) -> Result<TicketJwks, TicketError> {
        let response = self
            .client
            .execute(EgressRequest::get(self.url.clone()))
            .await
            .map_err(|_| TicketError::KeysUnavailable)?;
        if response.body().len() > MAX_JWKS_BYTES {
            return Err(TicketError::KeysUnavailable);
        }
        serde_json::from_slice(response.body()).map_err(|_| TicketError::KeysUnavailable)
    }
}
#[derive(Default)]
struct KeyCache {
    keys: BTreeMap<String, TicketJwk>,
    fetched_at: Option<Instant>,
    attempted_at: Option<Instant>,
    unknown_attempted_at: Option<Instant>,
}
#[derive(Clone)]
pub struct TicketVerifier {
    issuer: String,
    product_id: String,
    source: Arc<dyn TicketJwksSource>,
    cache: Arc<Mutex<KeyCache>>,
}
impl TicketVerifier {
    pub fn new(
        issuer: String,
        product_id: String,
        jwks_url: &str,
        client: GuardedClient,
    ) -> Result<Self, TicketError> {
        Self::with_source(
            issuer,
            product_id,
            Arc::new(GuardedJwksSource::new(jwks_url, client)?),
        )
    }
    pub fn with_source(
        issuer: String,
        product_id: String,
        source: Arc<dyn TicketJwksSource>,
    ) -> Result<Self, TicketError> {
        if issuer.is_empty() || product_id.is_empty() {
            return Err(TicketError::Invalid);
        }
        Ok(Self {
            issuer,
            product_id,
            source,
            cache: Arc::default(),
        })
    }
    pub async fn verify(&self, ticket: &str) -> Result<TicketClaims, TicketError> {
        self.verify_at(ticket, chrono::Utc::now().timestamp()).await
    }
    pub async fn verify_at(&self, ticket: &str, now: i64) -> Result<TicketClaims, TicketError> {
        let parsed = ParsedTicket::parse(ticket)?;
        let key = {
            let mut cache = self.cache.lock().await;
            let fresh = cache.fetched_at.is_some_and(|at| at.elapsed() < JWKS_TTL);
            if fresh && let Some(key) = cache.keys.get(&parsed.kid) {
                key.clone()
            } else {
                // A global cooldown bounds unknown-kid floods and failed fetches.
                // Holding the mutex coalesces concurrent refreshes.
                if fresh {
                    if cache
                        .unknown_attempted_at
                        .is_some_and(|at| at.elapsed() < REFRESH_COOLDOWN)
                    {
                        return Err(TicketError::Invalid);
                    }
                    cache.unknown_attempted_at = Some(Instant::now());
                } else if cache
                    .attempted_at
                    .is_some_and(|at| at.elapsed() < REFRESH_COOLDOWN)
                {
                    return Err(TicketError::KeysUnavailable);
                }
                cache.attempted_at = Some(Instant::now());
                let keys = self
                    .source
                    .fetch()
                    .await?
                    .by_kid()
                    .map_err(|_| TicketError::KeysUnavailable)?;
                cache.keys = keys;
                cache.fetched_at = Some(Instant::now());
                cache
                    .keys
                    .get(&parsed.kid)
                    .cloned()
                    .ok_or(TicketError::Invalid)?
            }
        };
        parsed.verify(&key, &self.issuer, &self.product_id, now)
    }
}
