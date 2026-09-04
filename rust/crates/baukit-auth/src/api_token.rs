use std::{collections::BTreeMap, fmt, future::Future, pin::Pin, sync::Arc};

use chrono::{DateTime, Utc};
use ring::{
    digest,
    rand::{SecureRandom as _, SystemRandom},
};
use thiserror::Error;
use uuid::Uuid;

use crate::{IdentityVerifier, Principal, VerificationError};

/// Marker used when a product does not choose its own.
pub const DEFAULT_API_TOKEN_MARKER: &str = "bk_";

const SECRET_LENGTH: usize = 32;
const DISPLAY_PREFIX_LENGTH: usize = 8;
const MAX_NAME_LENGTH: usize = 100;
const MAX_POLICY_CODE_LENGTH: usize = 64;
const MAX_POLICY_DETAIL_COUNT: usize = 8;
const MAX_POLICY_DETAIL_NAME_LENGTH: usize = 64;
const BASE62: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// Boxed future returned by [`ApiTokenStore`] operations.
pub type ApiTokenStoreFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// One stored personal access token, without any secret material.
///
/// `display_prefix` is the leading characters of the presented token, kept so a
/// list view can tell two tokens apart. It is a strict prefix of the secret and
/// never enough to reconstruct it.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ApiToken {
    /// Stable identifier of the token row.
    pub id: Uuid,
    /// Principal the token acts for.
    pub owner_id: Uuid,
    /// Human-chosen label shown in a token list.
    pub name: String,
    /// Leading characters of the presented token, safe to display.
    pub display_prefix: String,
    /// When the token was issued.
    pub created_at: DateTime<Utc>,
    /// When the token stops being accepted, if it expires at all.
    pub expires_at: Option<DateTime<Utc>>,
    /// When the token last authenticated a request.
    pub last_used_at: Option<DateTime<Utc>>,
    /// When the token was revoked, if it was.
    pub revoked_at: Option<DateTime<Utc>>,
}

impl ApiToken {
    /// Returns whether the token is still usable at `now`.
    #[must_use]
    pub fn is_active_at(&self, now: DateTime<Utc>) -> bool {
        self.revoked_at.is_none() && self.expires_at.is_none_or(|expires_at| expires_at > now)
    }
}

/// A newly issued token together with the only copy of its secret.
///
/// The secret exists in memory once. Return it to the caller in the creation
/// response and drop it. Nothing else can recover it from the store.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssuedApiToken {
    /// The stored token metadata.
    pub token: ApiToken,
    /// The full presented token, including its marker.
    pub secret: String,
}

/// Creation request for a personal access token.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewApiToken {
    /// Human-chosen label, 1 to 100 characters after trimming.
    pub name: String,
    /// Optional expiry, which must be in the future.
    pub expires_at: Option<DateTime<Utc>>,
}

impl NewApiToken {
    /// Creates a request for a token that never expires.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            expires_at: None,
        }
    }

    /// Sets the expiry of the token being created.
    #[must_use]
    pub fn expiring_at(mut self, expires_at: DateTime<Utc>) -> Self {
        self.expires_at = Some(expires_at);
        self
    }
}

/// Row an adapter must persist for a newly issued token.
///
/// `secret_hash` is a SHA-256 digest of the presented token. Store it as
/// opaque bytes and index it; the plaintext is never written.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApiTokenRecord {
    /// Stable identifier of the token row.
    pub id: Uuid,
    /// Principal the token acts for.
    pub owner_id: Uuid,
    /// Human-chosen label shown in a token list.
    pub name: String,
    /// SHA-256 digest of the presented token.
    pub secret_hash: Vec<u8>,
    /// Leading characters of the presented token, safe to display.
    pub display_prefix: String,
    /// When the token was issued.
    pub created_at: DateTime<Utc>,
    /// When the token stops being accepted, if it expires at all.
    pub expires_at: Option<DateTime<Utc>>,
}

/// A stored token together with the hash it was looked up by.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredApiToken {
    /// The stored token metadata.
    pub token: ApiToken,
    /// The SHA-256 digest persisted for this token.
    pub secret_hash: Vec<u8>,
}

/// A policy decision that an adapter may safely pass to product API code.
///
/// Codes and detail names are bounded snake_case identifiers. Detail values
/// are `u32`, so a rejection cannot carry SQL text, provider messages, or an
/// unbounded payload into a response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApiTokenPolicyRejection {
    code: String,
    details: BTreeMap<String, u32>,
}

impl ApiTokenPolicyRejection {
    /// Creates a rejection with no numeric details.
    ///
    /// The code must be a non-empty snake_case identifier of at most 64 ASCII
    /// characters.
    pub fn new(code: impl Into<String>) -> Result<Self, ApiTokenPolicyRejectionError> {
        let code = code.into();
        if !is_valid_policy_identifier(&code, MAX_POLICY_CODE_LENGTH) {
            return Err(ApiTokenPolicyRejectionError::InvalidCode);
        }
        Ok(Self {
            code,
            details: BTreeMap::new(),
        })
    }

    /// Adds or replaces one bounded numeric detail.
    ///
    /// The name follows the same snake_case rules as the code. A rejection can
    /// hold at most eight distinct details.
    pub fn with_detail(
        mut self,
        name: impl Into<String>,
        value: u32,
    ) -> Result<Self, ApiTokenPolicyRejectionError> {
        let name = name.into();
        if !is_valid_policy_identifier(&name, MAX_POLICY_DETAIL_NAME_LENGTH) {
            return Err(ApiTokenPolicyRejectionError::InvalidDetailName);
        }
        if !self.details.contains_key(&name) && self.details.len() == MAX_POLICY_DETAIL_COUNT {
            return Err(ApiTokenPolicyRejectionError::TooManyDetails);
        }
        self.details.insert(name, value);
        Ok(self)
    }

    /// Returns the stable product-owned rejection code.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Returns the safe numeric details ordered by name.
    #[must_use]
    pub fn details(&self) -> &BTreeMap<String, u32> {
        &self.details
    }

    /// Returns one numeric detail by name.
    #[must_use]
    pub fn detail(&self, name: &str) -> Option<u32> {
        self.details.get(name).copied()
    }
}

/// Failure raised while constructing a safe policy rejection.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ApiTokenPolicyRejectionError {
    /// The policy code was empty, too long, or not snake_case.
    #[error("API token policy code must be bounded snake_case")]
    InvalidCode,
    /// A numeric detail name was empty, too long, or not snake_case.
    #[error("API token policy detail name must be bounded snake_case")]
    InvalidDetailName,
    /// More than eight distinct numeric details were added.
    #[error("API token policy rejection cannot contain more than eight details")]
    TooManyDetails,
}

/// Failure returned by a product's [`ApiTokenStore`] adapter.
///
/// `Internal` retains a diagnostic string for internal logs, but its display
/// text is generic. `PolicyRejected` contains only validated public data.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ApiTokenStoreError {
    /// The adapter could not complete the operation.
    #[error("API token store failed")]
    Internal(String),
    /// Product policy rejected the operation with safe structured details.
    #[error("API token store policy rejected the operation")]
    PolicyRejected(ApiTokenPolicyRejection),
}

impl ApiTokenStoreError {
    /// Wraps private adapter diagnostics without adding them to display text.
    pub fn internal(error: impl fmt::Display) -> Self {
        Self::Internal(error.to_string())
    }
}

/// Storage-neutral port for personal access tokens.
///
/// Baukit owns secret generation, hashing, and expiry checks. The row shape,
/// the ownership join, and the retention rules stay product-local, so the
/// adapter lives in the product. An adapter must never persist the plaintext
/// secret and must look tokens up by hash only.
pub trait ApiTokenStore: Send + Sync {
    /// Persists one newly issued token and returns its stored form.
    fn create(
        &self,
        record: ApiTokenRecord,
    ) -> ApiTokenStoreFuture<'_, Result<ApiToken, ApiTokenStoreError>>;

    /// Returns the token whose stored hash equals `secret_hash`, if any.
    ///
    /// Revoked and expired tokens must still be returned. The service decides
    /// whether they authenticate, so an adapter never encodes that policy. The
    /// stored hash comes back with the token so the service can confirm the
    /// match in constant time rather than trusting the index lookup.
    fn find_by_hash<'a>(
        &'a self,
        secret_hash: &'a [u8],
    ) -> ApiTokenStoreFuture<'a, Result<Option<StoredApiToken>, ApiTokenStoreError>>;

    /// Records that a token authenticated a request at `used_at`.
    fn touch_last_used(
        &self,
        token_id: Uuid,
        used_at: DateTime<Utc>,
    ) -> ApiTokenStoreFuture<'_, Result<(), ApiTokenStoreError>>;

    /// Revokes one token owned by `owner_id`.
    ///
    /// Returns `false` when the owner has no unrevoked token with that id, so
    /// revoking someone else's token is indistinguishable from a missing one.
    fn revoke(
        &self,
        owner_id: Uuid,
        token_id: Uuid,
        revoked_at: DateTime<Utc>,
    ) -> ApiTokenStoreFuture<'_, Result<bool, ApiTokenStoreError>>;

    /// Lists the tokens belonging to one owner, newest first.
    fn list_for_owner(
        &self,
        owner_id: Uuid,
    ) -> ApiTokenStoreFuture<'_, Result<Vec<ApiToken>, ApiTokenStoreError>>;
}

/// Failure raised while issuing, listing, revoking, or verifying a token.
#[derive(Debug, Error)]
pub enum ApiTokenError {
    /// The requested name was empty or longer than 100 characters.
    #[error("API token name must contain between 1 and 100 characters")]
    InvalidName,
    /// The requested expiry was not in the future.
    #[error("API token expiry must be in the future")]
    InvalidExpiry,
    /// No token with that id belongs to the owner.
    #[error("API token was not found")]
    NotFound,
    /// The presented token is malformed, unknown, or revoked.
    ///
    /// The variant carries no detail. A wrong marker, a wrong length, an
    /// unknown hash, and a revoked token are indistinguishable to callers and
    /// to logs, so probing cannot tell a real token id from a fabricated one.
    #[error("API token is invalid")]
    Invalid,
    /// The presented token exists but its expiry has passed.
    #[error("API token has expired")]
    Expired,
    /// Product policy rejected the operation with safe structured details.
    #[error("API token operation was rejected by policy")]
    PolicyRejected(ApiTokenPolicyRejection),
    /// The backing store could not complete the operation.
    ///
    /// The source retains private diagnostics, but this error's display text
    /// never includes them.
    #[error("API token storage failed")]
    Storage(#[source] ApiTokenStoreError),
}

fn map_store_error(error: ApiTokenStoreError) -> ApiTokenError {
    match error {
        ApiTokenStoreError::PolicyRejected(rejection) => ApiTokenError::PolicyRejected(rejection),
        error @ ApiTokenStoreError::Internal(_) => ApiTokenError::Storage(error),
    }
}

fn is_valid_policy_identifier(value: &str, maximum_length: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum_length
        && !value.starts_with('_')
        && !value.ends_with('_')
        && !value.contains("__")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// Presentation format of a personal access token.
///
/// A token is a marker followed by 32 base62 characters. The marker makes a
/// leaked token searchable and lets an inbound request pick the token path
/// without trying to parse a JWT first. Products pick their own marker; the
/// default is [`DEFAULT_API_TOKEN_MARKER`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApiTokenFormat {
    marker: String,
}

impl Default for ApiTokenFormat {
    fn default() -> Self {
        Self {
            marker: DEFAULT_API_TOKEN_MARKER.to_owned(),
        }
    }
}

impl ApiTokenFormat {
    /// Creates a format with a product-specific marker.
    ///
    /// The marker must be non-empty ASCII alphanumerics ending in `_`, so it
    /// cannot collide with the base62 body and stays safe in a header.
    pub fn new(marker: impl Into<String>) -> Result<Self, ApiTokenFormatError> {
        let marker = marker.into();
        let Some(body) = marker.strip_suffix('_') else {
            return Err(ApiTokenFormatError::MissingSeparator);
        };
        if body.is_empty() || !body.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return Err(ApiTokenFormatError::InvalidMarker);
        }
        Ok(Self { marker })
    }

    /// Returns the marker every token of this format starts with.
    #[must_use]
    pub fn marker(&self) -> &str {
        &self.marker
    }

    /// Returns whether a presented bearer credential claims this format.
    ///
    /// This is a cheap routing check on the marker only. It says nothing about
    /// whether the token is well formed, known, or still valid.
    #[must_use]
    pub fn claims_format(&self, presented: &str) -> bool {
        presented.starts_with(&self.marker)
    }

    /// Returns whether a presented credential has the exact shape of a token.
    #[must_use]
    pub fn is_well_formed(&self, presented: &str) -> bool {
        presented.len() == self.marker.len() + SECRET_LENGTH
            && presented.starts_with(&self.marker)
            && presented[self.marker.len()..]
                .bytes()
                .all(|byte| BASE62.contains(&byte))
    }

    /// Generates one token secret from the system CSPRNG.
    ///
    /// Bytes at or above the largest multiple of 62 are drawn again rather than
    /// reduced, so every alphabet character stays equally likely. Taking the
    /// remainder directly would make the first eight characters slightly more
    /// common.
    fn generate(&self, random: &SystemRandom) -> Result<String, ApiTokenError> {
        const LIMIT: u8 = 256_u16.div_euclid(BASE62.len() as u16) as u8 * BASE62.len() as u8;

        let mut secret = String::with_capacity(self.marker.len() + SECRET_LENGTH);
        secret.push_str(&self.marker);
        let mut buffer = [0_u8; SECRET_LENGTH];
        while secret.len() < self.marker.len() + SECRET_LENGTH {
            random.fill(&mut buffer).map_err(|_| {
                ApiTokenError::Storage(ApiTokenStoreError::internal(
                    "system randomness is unavailable",
                ))
            })?;
            for byte in buffer {
                if byte < LIMIT && secret.len() < self.marker.len() + SECRET_LENGTH {
                    secret.push(char::from(BASE62[usize::from(byte % BASE62.len() as u8)]));
                }
            }
        }
        Ok(secret)
    }

    /// Returns the leading characters safe to store and display.
    fn display_prefix<'a>(&self, secret: &'a str) -> &'a str {
        &secret[..self.marker.len() + DISPLAY_PREFIX_LENGTH]
    }
}

/// Failure raised while configuring a token marker.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ApiTokenFormatError {
    /// The marker did not end in `_`.
    #[error("API token marker must end in an underscore")]
    MissingSeparator,
    /// The marker was empty or contained non-alphanumeric characters.
    #[error("API token marker must be non-empty ASCII alphanumerics")]
    InvalidMarker,
}

/// Hashes a presented token exactly the way the service stores it.
///
/// Adapters and migrations that need the stored form of a known secret use
/// this; nothing else should hash tokens by hand.
#[must_use]
pub fn hash_api_token(presented: &str) -> Vec<u8> {
    digest::digest(&digest::SHA256, presented.as_bytes())
        .as_ref()
        .to_vec()
}

/// Compares two digests without an early exit on the first differing byte.
fn digests_are_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (left_byte, right_byte) in left.iter().zip(right) {
        difference |= left_byte ^ right_byte;
    }
    difference == 0
}

/// Issues, lists, revokes, and verifies personal access tokens.
///
/// The service is the second inbound credential kind next to OIDC JWTs. It
/// holds no session state, so a clone is cheap and safe to put in Axum state.
#[derive(Clone)]
pub struct ApiTokenService {
    store: Arc<dyn ApiTokenStore>,
    format: ApiTokenFormat,
    random: Arc<SystemRandom>,
}

impl fmt::Debug for ApiTokenService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApiTokenService")
            .field("marker", &self.format.marker)
            .finish_non_exhaustive()
    }
}

impl ApiTokenService {
    /// Creates a service issuing tokens with the default marker.
    #[must_use]
    pub fn new(store: Arc<dyn ApiTokenStore>) -> Self {
        Self::with_format(store, ApiTokenFormat::default())
    }

    /// Creates a service issuing tokens with a product-specific marker.
    #[must_use]
    pub fn with_format(store: Arc<dyn ApiTokenStore>, format: ApiTokenFormat) -> Self {
        Self {
            store,
            format,
            random: Arc::new(SystemRandom::new()),
        }
    }

    /// Returns the token format this service issues and accepts.
    #[must_use]
    pub fn format(&self) -> &ApiTokenFormat {
        &self.format
    }

    /// Issues one token for `owner_id`, returning its secret exactly once.
    pub async fn issue(
        &self,
        owner_id: Uuid,
        request: NewApiToken,
    ) -> Result<IssuedApiToken, ApiTokenError> {
        self.issue_at(owner_id, request, Utc::now()).await
    }

    /// Issues one token as of an explicit instant.
    pub async fn issue_at(
        &self,
        owner_id: Uuid,
        request: NewApiToken,
        now: DateTime<Utc>,
    ) -> Result<IssuedApiToken, ApiTokenError> {
        let name = request.name.trim();
        if name.is_empty() || name.chars().count() > MAX_NAME_LENGTH {
            return Err(ApiTokenError::InvalidName);
        }
        if request.expires_at.is_some_and(|expiry| expiry <= now) {
            return Err(ApiTokenError::InvalidExpiry);
        }

        let secret = self.format.generate(&self.random)?;
        let token = self
            .store
            .create(ApiTokenRecord {
                id: Uuid::now_v7(),
                owner_id,
                name: name.to_owned(),
                secret_hash: hash_api_token(&secret),
                display_prefix: self.format.display_prefix(&secret).to_owned(),
                created_at: now,
                expires_at: request.expires_at,
            })
            .await
            .map_err(map_store_error)?;
        Ok(IssuedApiToken { token, secret })
    }

    /// Lists the tokens belonging to one owner.
    pub async fn list_for_owner(&self, owner_id: Uuid) -> Result<Vec<ApiToken>, ApiTokenError> {
        self.store
            .list_for_owner(owner_id)
            .await
            .map_err(map_store_error)
    }

    /// Revokes one token owned by `owner_id`.
    pub async fn revoke(&self, owner_id: Uuid, token_id: Uuid) -> Result<(), ApiTokenError> {
        self.revoke_at(owner_id, token_id, Utc::now()).await
    }

    /// Revokes one token as of an explicit instant.
    pub async fn revoke_at(
        &self,
        owner_id: Uuid,
        token_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<(), ApiTokenError> {
        let revoked = self
            .store
            .revoke(owner_id, token_id, now)
            .await
            .map_err(map_store_error)?;
        if revoked {
            Ok(())
        } else {
            Err(ApiTokenError::NotFound)
        }
    }

    /// Verifies a presented token and records the use.
    ///
    /// Returns the owning token on success. Malformed, unknown, and revoked
    /// tokens all fail as [`ApiTokenError::Invalid`]; an expired token is
    /// reported separately so a client can be told to reissue, and its last-use
    /// timestamp is left untouched.
    pub async fn verify(&self, presented: &str) -> Result<ApiToken, ApiTokenError> {
        self.verify_at(presented, Utc::now()).await
    }

    /// Verifies a presented token as of an explicit instant.
    pub async fn verify_at(
        &self,
        presented: &str,
        now: DateTime<Utc>,
    ) -> Result<ApiToken, ApiTokenError> {
        if !self.format.is_well_formed(presented) {
            return Err(ApiTokenError::Invalid);
        }
        let presented_hash = hash_api_token(presented);
        let stored = self
            .store
            .find_by_hash(&presented_hash)
            .await
            .map_err(map_store_error)?
            .ok_or(ApiTokenError::Invalid)?;
        if !digests_are_equal(&stored.secret_hash, &presented_hash) {
            return Err(ApiTokenError::Invalid);
        }
        let token = stored.token;
        if token.revoked_at.is_some() {
            return Err(ApiTokenError::Invalid);
        }
        if token.expires_at.is_some_and(|expiry| expiry <= now) {
            return Err(ApiTokenError::Expired);
        }
        self.store
            .touch_last_used(token.id, now)
            .await
            .map_err(map_store_error)?;
        Ok(token)
    }
}

/// Verifier that accepts API tokens and delegates everything else.
///
/// Wrap the product's OIDC verifier so one bearer header serves both kinds:
/// credentials carrying the token marker are verified against the store, and
/// anything else falls through to the JWT path. A marked credential never
/// reaches the JWT verifier, so a malformed token cannot be probed twice.
///
/// The produced [`Principal`] carries the token owner as its subject, no
/// issuer, and the verified token metadata returned by the store.
#[derive(Clone)]
pub struct ApiTokenVerifier {
    tokens: ApiTokenService,
    fallback: Arc<dyn IdentityVerifier>,
}

impl ApiTokenVerifier {
    /// Chains the token service in front of an existing identity verifier.
    #[must_use]
    pub fn new(tokens: ApiTokenService, fallback: Arc<dyn IdentityVerifier>) -> Self {
        Self { tokens, fallback }
    }

    async fn verify_bearer(&self, presented: &str) -> Result<Principal, VerificationError> {
        if !self.tokens.format().claims_format(presented) {
            return self.fallback.verify(presented).await;
        }
        match self.tokens.verify(presented).await {
            Ok(token) => Ok(Principal::from_api_token(token)),
            Err(ApiTokenError::Expired) => Err(VerificationError::Expired),
            Err(_) => Err(VerificationError::InvalidSignature),
        }
    }
}

impl fmt::Debug for ApiTokenVerifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApiTokenVerifier")
            .field("tokens", &self.tokens)
            .finish_non_exhaustive()
    }
}

impl IdentityVerifier for ApiTokenVerifier {
    fn verify<'a>(
        &'a self,
        token: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Principal, VerificationError>> + Send + 'a>> {
        Box::pin(self.verify_bearer(token))
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex};

    use chrono::TimeZone as _;

    use super::*;
    use crate::{AuthState, establish_principal};

    #[derive(Default)]
    struct MemoryStore {
        tokens: Mutex<HashMap<Uuid, ApiToken>>,
        hashes: Mutex<HashMap<Vec<u8>, Uuid>>,
        touched: Mutex<Vec<(Uuid, DateTime<Utc>)>>,
        failure: Mutex<Option<ApiTokenStoreError>>,
    }

    impl ApiTokenStore for MemoryStore {
        fn create(
            &self,
            record: ApiTokenRecord,
        ) -> ApiTokenStoreFuture<'_, Result<ApiToken, ApiTokenStoreError>> {
            if let Some(error) = self.failure.lock().expect("failure lock").clone() {
                return Box::pin(std::future::ready(Err(error)));
            }
            let token = ApiToken {
                id: record.id,
                owner_id: record.owner_id,
                name: record.name,
                display_prefix: record.display_prefix,
                created_at: record.created_at,
                expires_at: record.expires_at,
                last_used_at: None,
                revoked_at: None,
            };
            self.hashes
                .lock()
                .expect("hash lock")
                .insert(record.secret_hash, token.id);
            self.tokens
                .lock()
                .expect("token lock")
                .insert(token.id, token.clone());
            Box::pin(std::future::ready(Ok(token)))
        }

        fn find_by_hash<'a>(
            &'a self,
            secret_hash: &'a [u8],
        ) -> ApiTokenStoreFuture<'a, Result<Option<StoredApiToken>, ApiTokenStoreError>> {
            if let Some(error) = self.failure.lock().expect("failure lock").clone() {
                return Box::pin(std::future::ready(Err(error)));
            }
            let found = self
                .hashes
                .lock()
                .expect("hash lock")
                .get(secret_hash)
                .and_then(|id| self.tokens.lock().expect("token lock").get(id).cloned())
                .map(|token| StoredApiToken {
                    token,
                    secret_hash: secret_hash.to_vec(),
                });
            Box::pin(std::future::ready(Ok(found)))
        }

        fn touch_last_used(
            &self,
            token_id: Uuid,
            used_at: DateTime<Utc>,
        ) -> ApiTokenStoreFuture<'_, Result<(), ApiTokenStoreError>> {
            if let Some(error) = self.failure.lock().expect("failure lock").clone() {
                return Box::pin(std::future::ready(Err(error)));
            }
            self.touched
                .lock()
                .expect("touched lock")
                .push((token_id, used_at));
            if let Some(token) = self.tokens.lock().expect("token lock").get_mut(&token_id) {
                token.last_used_at = Some(used_at);
            }
            Box::pin(std::future::ready(Ok(())))
        }

        fn revoke(
            &self,
            owner_id: Uuid,
            token_id: Uuid,
            revoked_at: DateTime<Utc>,
        ) -> ApiTokenStoreFuture<'_, Result<bool, ApiTokenStoreError>> {
            if let Some(error) = self.failure.lock().expect("failure lock").clone() {
                return Box::pin(std::future::ready(Err(error)));
            }
            let mut tokens = self.tokens.lock().expect("token lock");
            let revoked = tokens
                .get_mut(&token_id)
                .filter(|token| token.owner_id == owner_id && token.revoked_at.is_none())
                .is_some_and(|token| {
                    token.revoked_at = Some(revoked_at);
                    true
                });
            Box::pin(std::future::ready(Ok(revoked)))
        }

        fn list_for_owner(
            &self,
            owner_id: Uuid,
        ) -> ApiTokenStoreFuture<'_, Result<Vec<ApiToken>, ApiTokenStoreError>> {
            if let Some(error) = self.failure.lock().expect("failure lock").clone() {
                return Box::pin(std::future::ready(Err(error)));
            }
            let mut owned: Vec<ApiToken> = self
                .tokens
                .lock()
                .expect("token lock")
                .values()
                .filter(|token| token.owner_id == owner_id)
                .cloned()
                .collect();
            owned.sort_by_key(|token| std::cmp::Reverse(token.created_at));
            Box::pin(std::future::ready(Ok(owned)))
        }
    }

    fn instant() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 22, 12, 0, 0)
            .single()
            .expect("valid test instant")
    }

    fn service() -> (Arc<MemoryStore>, ApiTokenService) {
        let store = Arc::new(MemoryStore::default());
        let service = ApiTokenService::new(store.clone());
        (store, service)
    }

    #[test]
    fn policy_rejections_accept_only_bounded_snake_case_data() {
        let rejection = ApiTokenPolicyRejection::new("api_tokens_active_limit_exceeded")
            .expect("valid code")
            .with_detail("maximum", 10)
            .expect("valid detail");
        assert_eq!(rejection.code(), "api_tokens_active_limit_exceeded");
        assert_eq!(rejection.detail("maximum"), Some(10));
        assert_eq!(rejection.details().len(), 1);

        for code in [
            "",
            "_leading",
            "trailing_",
            "double__separator",
            "Uppercase",
            "sql error: SELECT secret_hash",
        ] {
            assert_eq!(
                ApiTokenPolicyRejection::new(code),
                Err(ApiTokenPolicyRejectionError::InvalidCode)
            );
        }
        assert_eq!(
            ApiTokenPolicyRejection::new("a".repeat(MAX_POLICY_CODE_LENGTH + 1)),
            Err(ApiTokenPolicyRejectionError::InvalidCode)
        );

        let mut full = ApiTokenPolicyRejection::new("limit_exceeded").expect("valid code");
        for index in 0..MAX_POLICY_DETAIL_COUNT {
            full = full
                .with_detail(format!("detail_{index}"), index as u32)
                .expect("detail within limit");
        }
        assert_eq!(
            full.with_detail("one_too_many", 9),
            Err(ApiTokenPolicyRejectionError::TooManyDetails)
        );
    }

    #[tokio::test]
    async fn structured_active_token_limit_reaches_the_service_caller() {
        let (store, service) = service();
        let rejection = ApiTokenPolicyRejection::new("api_tokens_active_limit_exceeded")
            .expect("valid code")
            .with_detail("maximum", 10)
            .expect("valid detail");
        *store.failure.lock().expect("failure lock") =
            Some(ApiTokenStoreError::PolicyRejected(rejection.clone()));

        let error = service
            .issue_at(Uuid::now_v7(), NewApiToken::new("Over limit"), instant())
            .await
            .expect_err("policy must reject creation");

        assert!(matches!(
            error,
            ApiTokenError::PolicyRejected(actual) if actual == rejection
        ));
    }

    #[tokio::test]
    async fn internal_store_detail_is_absent_from_public_error_text() {
        use axum::{body::to_bytes, http::StatusCode, response::IntoResponse as _};

        let (store, service) = service();
        let private_detail =
            "duplicate key value violates api_tokens_token_hash_key: secret_hash=abc123";
        *store.failure.lock().expect("failure lock") =
            Some(ApiTokenStoreError::internal(private_detail));

        let error = service
            .list_for_owner(Uuid::now_v7())
            .await
            .expect_err("store must fail");

        assert_eq!(error.to_string(), "API token storage failed");
        assert!(!error.to_string().contains(private_detail));
        assert!(matches!(
            &error,
            ApiTokenError::Storage(ApiTokenStoreError::Internal(detail))
                if detail == private_detail
        ));

        let response = baukit_http::ApiError::internal(error).into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        assert!(!String::from_utf8_lossy(&body).contains(private_detail));
    }

    #[tokio::test]
    async fn issued_secrets_use_the_marker_and_a_base62_body() {
        let (_store, service) = service();
        let issued = service
            .issue_at(Uuid::now_v7(), NewApiToken::new("Automation"), instant())
            .await
            .expect("issue token");

        assert!(issued.secret.starts_with(DEFAULT_API_TOKEN_MARKER));
        assert_eq!(
            issued.secret.len(),
            DEFAULT_API_TOKEN_MARKER.len() + SECRET_LENGTH
        );
        assert!(
            issued.secret[DEFAULT_API_TOKEN_MARKER.len()..]
                .bytes()
                .all(|byte| BASE62.contains(&byte))
        );
        assert!(service.format().is_well_formed(&issued.secret));
    }

    #[tokio::test]
    async fn two_issued_secrets_never_repeat() {
        let (_store, service) = service();
        let owner_id = Uuid::now_v7();
        let first = service
            .issue_at(owner_id, NewApiToken::new("First"), instant())
            .await
            .expect("issue first");
        let second = service
            .issue_at(owner_id, NewApiToken::new("Second"), instant())
            .await
            .expect("issue second");

        assert_ne!(first.secret, second.secret);
    }

    #[tokio::test]
    async fn the_store_only_ever_sees_a_hash_and_a_strict_prefix() {
        let (store, service) = service();
        let issued = service
            .issue_at(Uuid::now_v7(), NewApiToken::new("Automation"), instant())
            .await
            .expect("issue token");

        let hashes = store.hashes.lock().expect("hash lock");
        let stored_hash = hashes.keys().next().expect("stored hash");
        assert_eq!(stored_hash, &hash_api_token(&issued.secret));
        assert_eq!(stored_hash.len(), 32);
        assert_ne!(stored_hash.as_slice(), issued.secret.as_bytes());
        assert!(!String::from_utf8_lossy(stored_hash).contains(&issued.secret));

        assert!(issued.secret.starts_with(&issued.token.display_prefix));
        assert!(issued.token.display_prefix.len() < issued.secret.len());
        assert_eq!(
            issued.token.display_prefix.len(),
            DEFAULT_API_TOKEN_MARKER.len() + DISPLAY_PREFIX_LENGTH
        );
    }

    #[tokio::test]
    async fn verification_accepts_the_issued_secret_and_records_the_use() {
        let (store, service) = service();
        let owner_id = Uuid::now_v7();
        let issued = service
            .issue_at(owner_id, NewApiToken::new("Automation"), instant())
            .await
            .expect("issue token");

        let verified = service
            .verify_at(&issued.secret, instant())
            .await
            .expect("verify token");

        assert_eq!(verified.owner_id, owner_id);
        assert_eq!(verified.id, issued.token.id);
        assert_eq!(
            store.touched.lock().expect("touched lock").as_slice(),
            [(issued.token.id, instant())]
        );
    }

    #[tokio::test]
    async fn malformed_unknown_and_unmarked_credentials_are_invalid() {
        let (store, service) = service();
        let issued = service
            .issue_at(Uuid::now_v7(), NewApiToken::new("Automation"), instant())
            .await
            .expect("issue token");
        let mut wrong = issued.secret.clone();
        wrong.pop();
        wrong.push(if issued.secret.ends_with('a') {
            'b'
        } else {
            'a'
        });

        for candidate in [
            wrong.as_str(),
            "bk_short",
            "bk_+++++++++++++++++++++++++++++++",
            "eyJhbGciOiJIUzI1NiJ9.e30.signature",
            "",
        ] {
            assert!(matches!(
                service.verify_at(candidate, instant()).await,
                Err(ApiTokenError::Invalid)
            ));
        }
        assert!(store.touched.lock().expect("touched lock").is_empty());
    }

    #[tokio::test]
    async fn expired_tokens_report_expiry_without_recording_a_use() {
        let (store, service) = service();
        let issued = service
            .issue_at(
                Uuid::now_v7(),
                NewApiToken::new("Automation")
                    .expiring_at(instant() + chrono::Duration::minutes(1)),
                instant(),
            )
            .await
            .expect("issue token");

        assert!(matches!(
            service
                .verify_at(&issued.secret, instant() + chrono::Duration::minutes(2))
                .await,
            Err(ApiTokenError::Expired)
        ));
        assert!(store.touched.lock().expect("touched lock").is_empty());
        assert!(
            !issued
                .token
                .is_active_at(instant() + chrono::Duration::minutes(2))
        );
    }

    #[tokio::test]
    async fn revoked_tokens_stop_verifying() {
        let (_store, service) = service();
        let owner_id = Uuid::now_v7();
        let issued = service
            .issue_at(owner_id, NewApiToken::new("Automation"), instant())
            .await
            .expect("issue token");

        service
            .revoke_at(owner_id, issued.token.id, instant())
            .await
            .expect("revoke token");

        assert!(matches!(
            service.verify_at(&issued.secret, instant()).await,
            Err(ApiTokenError::Invalid)
        ));
    }

    #[tokio::test]
    async fn revoking_someone_elses_token_is_indistinguishable_from_a_missing_one() {
        let (_store, service) = service();
        let issued = service
            .issue_at(Uuid::now_v7(), NewApiToken::new("Automation"), instant())
            .await
            .expect("issue token");

        assert!(matches!(
            service
                .revoke_at(Uuid::now_v7(), issued.token.id, instant())
                .await,
            Err(ApiTokenError::NotFound)
        ));
        assert!(matches!(
            service
                .revoke_at(issued.token.owner_id, Uuid::now_v7(), instant())
                .await,
            Err(ApiTokenError::NotFound)
        ));
    }

    #[tokio::test]
    async fn listing_returns_only_the_owners_tokens_without_secrets() {
        let (_store, service) = service();
        let owner_id = Uuid::now_v7();
        service
            .issue_at(owner_id, NewApiToken::new("Mine"), instant())
            .await
            .expect("issue owned token");
        service
            .issue_at(Uuid::now_v7(), NewApiToken::new("Theirs"), instant())
            .await
            .expect("issue other token");

        let listed = service.list_for_owner(owner_id).await.expect("list tokens");

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Mine");
        assert!(listed[0].is_active_at(instant()));
    }

    #[tokio::test]
    async fn names_and_expiries_are_validated_before_anything_is_stored() {
        let (store, service) = service();
        let owner_id = Uuid::now_v7();

        assert!(matches!(
            service
                .issue_at(owner_id, NewApiToken::new("   "), instant())
                .await,
            Err(ApiTokenError::InvalidName)
        ));
        assert!(matches!(
            service
                .issue_at(owner_id, NewApiToken::new("n".repeat(101)), instant())
                .await,
            Err(ApiTokenError::InvalidName)
        ));
        assert!(matches!(
            service
                .issue_at(
                    owner_id,
                    NewApiToken::new("Past").expiring_at(instant()),
                    instant()
                )
                .await,
            Err(ApiTokenError::InvalidExpiry)
        ));
        assert!(store.tokens.lock().expect("token lock").is_empty());
    }

    #[test]
    fn markers_must_be_alphanumeric_and_end_in_an_underscore() {
        assert_eq!(
            ApiTokenFormat::new("acme_").expect("valid marker").marker(),
            "acme_"
        );
        assert!(matches!(
            ApiTokenFormat::new("acme"),
            Err(ApiTokenFormatError::MissingSeparator)
        ));
        assert!(matches!(
            ApiTokenFormat::new("_"),
            Err(ApiTokenFormatError::InvalidMarker)
        ));
        assert!(matches!(
            ApiTokenFormat::new("ac me_"),
            Err(ApiTokenFormatError::InvalidMarker)
        ));
    }

    #[tokio::test]
    async fn a_custom_marker_is_carried_into_issued_secrets() {
        let store = Arc::new(MemoryStore::default());
        let service = ApiTokenService::with_format(
            store,
            ApiTokenFormat::new("acme_").expect("valid marker"),
        );

        let issued = service
            .issue_at(Uuid::now_v7(), NewApiToken::new("Automation"), instant())
            .await
            .expect("issue token");

        assert!(issued.secret.starts_with("acme_"));
        assert!(!service.format().claims_format("bk_something"));
        assert!(service.format().claims_format(&issued.secret));
    }

    #[tokio::test]
    async fn a_store_returning_a_mismatched_hash_does_not_authenticate() {
        struct WrongHashStore;

        impl ApiTokenStore for WrongHashStore {
            fn create(
                &self,
                _record: ApiTokenRecord,
            ) -> ApiTokenStoreFuture<'_, Result<ApiToken, ApiTokenStoreError>> {
                unreachable!("not used")
            }

            fn find_by_hash<'a>(
                &'a self,
                _secret_hash: &'a [u8],
            ) -> ApiTokenStoreFuture<'a, Result<Option<StoredApiToken>, ApiTokenStoreError>>
            {
                Box::pin(std::future::ready(Ok(Some(StoredApiToken {
                    token: ApiToken {
                        id: Uuid::now_v7(),
                        owner_id: Uuid::now_v7(),
                        name: "Mismatched".to_owned(),
                        display_prefix: "bk_012345".to_owned(),
                        created_at: instant(),
                        expires_at: None,
                        last_used_at: None,
                        revoked_at: None,
                    },
                    secret_hash: vec![0_u8; 32],
                }))))
            }

            fn touch_last_used(
                &self,
                _token_id: Uuid,
                _used_at: DateTime<Utc>,
            ) -> ApiTokenStoreFuture<'_, Result<(), ApiTokenStoreError>> {
                unreachable!("not used")
            }

            fn revoke(
                &self,
                _owner_id: Uuid,
                _token_id: Uuid,
                _revoked_at: DateTime<Utc>,
            ) -> ApiTokenStoreFuture<'_, Result<bool, ApiTokenStoreError>> {
                unreachable!("not used")
            }

            fn list_for_owner(
                &self,
                _owner_id: Uuid,
            ) -> ApiTokenStoreFuture<'_, Result<Vec<ApiToken>, ApiTokenStoreError>> {
                unreachable!("not used")
            }
        }

        let service = ApiTokenService::new(Arc::new(WrongHashStore));

        assert!(matches!(
            service
                .verify_at("bk_0123456789ABCDEFGHIJKLMNOPQRSTUV", instant())
                .await,
            Err(ApiTokenError::Invalid)
        ));
    }

    struct RecordingFallback {
        seen: Mutex<Vec<String>>,
    }

    impl IdentityVerifier for RecordingFallback {
        fn verify<'a>(
            &'a self,
            token: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<Principal, VerificationError>> + Send + 'a>>
        {
            self.seen.lock().expect("seen lock").push(token.to_owned());
            Box::pin(std::future::ready(Ok(Principal::new("oidc-subject"))))
        }
    }

    #[tokio::test]
    async fn unmarked_credentials_fall_through_to_the_jwt_verifier() {
        let fallback = Arc::new(RecordingFallback {
            seen: Mutex::new(Vec::new()),
        });
        let (_store, service) = service();
        let verifier = ApiTokenVerifier::new(service, fallback.clone());

        let principal = verifier
            .verify("eyJhbGciOiJIUzI1NiJ9.e30.signature")
            .await
            .expect("fallback verification");

        assert_eq!(principal.subject(), "oidc-subject");
        assert!(principal.api_token().is_none());
        assert_eq!(
            fallback.seen.lock().expect("seen lock").as_slice(),
            ["eyJhbGciOiJIUzI1NiJ9.e30.signature"]
        );
    }

    #[tokio::test]
    async fn marked_credentials_never_reach_the_jwt_verifier() {
        let fallback = Arc::new(RecordingFallback {
            seen: Mutex::new(Vec::new()),
        });
        let (_store, service) = service();
        let owner_id = Uuid::now_v7();
        let issued = service
            .issue_at(owner_id, NewApiToken::new("Automation"), instant())
            .await
            .expect("issue token");
        let verifier = ApiTokenVerifier::new(service, fallback.clone());

        let principal = verifier.verify(&issued.secret).await.expect("verify token");
        assert_eq!(principal.subject(), owner_id.to_string());
        assert!(principal.issuer().is_none());
        assert_eq!(principal.api_token(), Some(&issued.token));

        assert!(matches!(
            verifier.verify("bk_malformed").await,
            Err(VerificationError::InvalidSignature)
        ));
        assert!(fallback.seen.lock().expect("seen lock").is_empty());
    }

    #[tokio::test]
    async fn expired_api_tokens_surface_as_expired_verification_failures() {
        let fallback = Arc::new(RecordingFallback {
            seen: Mutex::new(Vec::new()),
        });
        let (_store, service) = service();
        let issued = service
            .issue_at(
                Uuid::now_v7(),
                NewApiToken::new("Automation")
                    .expiring_at(instant() - chrono::Duration::minutes(1)),
                instant() - chrono::Duration::minutes(2),
            )
            .await
            .expect("issue token");
        let verifier = ApiTokenVerifier::new(service, fallback);

        assert!(matches!(
            verifier.verify(&issued.secret).await,
            Err(VerificationError::Expired)
        ));
    }

    #[tokio::test]
    async fn principal_middleware_accepts_oidc_and_api_token_verifiers() {
        use axum::{
            Router,
            body::Body,
            http::{Request, StatusCode, header},
            middleware,
            routing::get,
        };
        use tower::ServiceExt as _;

        async fn handler(principal: Principal) -> String {
            principal.subject().to_owned()
        }

        let fallback = Arc::new(RecordingFallback {
            seen: Mutex::new(Vec::new()),
        });
        let (_store, service) = service();
        let owner_id = Uuid::now_v7();
        let live = service
            .issue_at(owner_id, NewApiToken::new("Live"), instant())
            .await
            .expect("issue live token");
        let revoked = service
            .issue_at(owner_id, NewApiToken::new("Revoked"), instant())
            .await
            .expect("issue revoked token");
        service
            .revoke_at(owner_id, revoked.token.id, instant())
            .await
            .expect("revoke token");

        let auth = AuthState::new(ApiTokenVerifier::new(service, fallback.clone()));
        let router = Router::new()
            .route("/", get(handler))
            .with_state(auth.clone())
            .layer(middleware::from_fn_with_state(auth, establish_principal));
        let request = |secret: &str| {
            Request::builder()
                .uri("/")
                .header(header::AUTHORIZATION, format!("Bearer {secret}"))
                .body(Body::empty())
                .expect("request")
        };

        let accepted = router
            .clone()
            .oneshot(request(&live.secret))
            .await
            .expect("response");
        assert_eq!(accepted.status(), StatusCode::OK);

        let oidc = router
            .clone()
            .oneshot(request("eyJhbGciOiJIUzI1NiJ9.e30.signature"))
            .await
            .expect("response");
        assert_eq!(oidc.status(), StatusCode::OK);
        assert_eq!(
            fallback.seen.lock().expect("seen lock").as_slice(),
            ["eyJhbGciOiJIUzI1NiJ9.e30.signature"]
        );

        let rejected = router
            .oneshot(request(&revoked.secret))
            .await
            .expect("response");
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            rejected.headers()[header::WWW_AUTHENTICATE],
            "Bearer error=\"invalid_token\", hint=\"invalid\""
        );
    }

    #[tokio::test]
    async fn generation_covers_the_whole_base62_alphabet() {
        let (_store, service) = service();
        let owner_id = Uuid::now_v7();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            let issued = service
                .issue_at(owner_id, NewApiToken::new("Sampling"), instant())
                .await
                .expect("issue token");
            seen.extend(issued.secret[DEFAULT_API_TOKEN_MARKER.len()..].bytes());
        }

        assert_eq!(seen.len(), BASE62.len());
    }

    #[test]
    fn hashing_is_sha256_and_never_returns_the_secret() {
        let hash = hash_api_token("bk_0123456789ABCDEFGHIJKLMNOPQRSTUV");

        assert_eq!(hash.len(), 32);
        assert_ne!(hash, b"bk_0123456789ABCDEFGHIJKLMNOPQRSTUV".to_vec());
        assert_eq!(hash, hash_api_token("bk_0123456789ABCDEFGHIJKLMNOPQRSTUV"));
        assert_ne!(hash, hash_api_token("bk_0123456789ABCDEFGHIJKLMNOPQRSTUW"));
    }
}
