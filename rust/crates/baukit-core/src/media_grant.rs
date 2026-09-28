//! Short-lived HMAC-SHA256 grants for immutable media served behind an edge verifier.
//!
//! A grant is the query `expires=<unix seconds>&keyId=<id>&mode=playback&signature=<base64url>`
//! appended to a media path. The signature is HMAC-SHA256 over
//! `"{path}\n{expires}\nplayback\n{keyId}"`, keyed with the base64url-decoded secret. The njs
//! verifier in `deploy/media-grants` checks the same vectors as this module.

use std::fmt;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::hmac;
use thiserror::Error;
use zeroize::Zeroizing;

/// The only grant mode. It is part of the signing input and of the query.
pub const MEDIA_GRANT_MODE: &str = "playback";
/// The longest lifetime a signer issues, in seconds.
pub const MAX_GRANT_LIFETIME_SECONDS: u64 = 3_600;
/// How far a verifier's clock may lag the signer's before a fresh grant looks too far ahead.
pub const MAX_CLOCK_SKEW_SECONDS: u64 = 60;
/// The shortest decoded secret a key accepts.
pub const MIN_SECRET_BYTES: usize = 32;
/// The longest key ID.
pub const MAX_KEY_ID_BYTES: usize = 64;
/// The longest media path.
pub const MAX_PATH_BYTES: usize = 512;

const MAX_EXPIRES: u64 = 9_999_999_999;
const MAX_EXPIRES_DIGITS: usize = 10;
const SIGNATURE_BYTES: usize = 32;
const SIGNATURE_BASE64URL_BYTES: usize = 43;
const EXPIRES_PARAM: &str = "expires=";
const KEY_ID_PARAM: &str = "keyId=";
const MODE_PARAM: &str = "mode=";
const SIGNATURE_PARAM: &str = "signature=";
const PARAM_SEPARATORS: usize = 3;
const MAX_QUERY_BYTES: usize = EXPIRES_PARAM.len()
    + MAX_EXPIRES_DIGITS
    + KEY_ID_PARAM.len()
    + MAX_KEY_ID_BYTES
    + MODE_PARAM.len()
    + MEDIA_GRANT_MODE.len()
    + SIGNATURE_PARAM.len()
    + SIGNATURE_BASE64URL_BYTES
    + PARAM_SEPARATORS;
const REDACTED: &str = "<redacted>";

/// A key or key ring that cannot be configured. The message never contains key material.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum MediaGrantKeyError {
    /// The key ID is empty, longer than 64 bytes, or not `[A-Za-z0-9][A-Za-z0-9_-]*`.
    #[error("media grant key ID is invalid")]
    InvalidKeyId,
    /// The secret is not canonical unpadded base64url.
    #[error("media grant secret is not canonical unpadded base64url")]
    InvalidSecretEncoding,
    /// The decoded secret is shorter than [`MIN_SECRET_BYTES`].
    #[error("media grant secret decodes to fewer than 32 bytes")]
    SecretTooShort,
    /// The previous key reuses the current key's ID.
    #[error("media grant previous key reuses the current key ID")]
    DuplicateKeyId,
}

impl MediaGrantKeyError {
    /// Returns the stable snake_case code shared with the njs verifier.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidKeyId => "invalid_key_id",
            Self::InvalidSecretEncoding => "invalid_secret_encoding",
            Self::SecretTooShort => "secret_too_short",
            Self::DuplicateKeyId => "duplicate_key_id",
        }
    }
}

/// A grant that cannot be signed or verified. The message never contains the grant.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum MediaGrantError {
    /// The request method is not `GET` or `HEAD`.
    #[error("media grant method is not GET or HEAD")]
    InvalidMethod,
    /// The path is not a normalized media path.
    #[error("media grant path is invalid")]
    InvalidPath,
    /// The query does not follow the grant grammar.
    #[error("media grant query is invalid")]
    InvalidQuery,
    /// The grant expired at or before the current instant.
    #[error("media grant expired")]
    Expired,
    /// The expiry is further ahead than the lifetime and skew allow.
    #[error("media grant expiry is too far ahead")]
    ExpiryTooFar,
    /// The key ID names neither the current nor the previous key.
    #[error("media grant key is unknown")]
    UnknownKey,
    /// The signature does not match.
    #[error("media grant signature is invalid")]
    InvalidSignature,
}

impl MediaGrantError {
    /// Returns the stable snake_case code shared with the njs verifier.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidMethod => "invalid_method",
            Self::InvalidPath => "invalid_path",
            Self::InvalidQuery => "invalid_query",
            Self::Expired => "expired",
            Self::ExpiryTooFar => "expiry_too_far",
            Self::UnknownKey => "unknown_key",
            Self::InvalidSignature => "invalid_signature",
        }
    }
}

/// One signing key: a public key ID and a secret that never leaves this type.
#[derive(Clone)]
pub struct MediaGrantKey {
    key_id: String,
    key: hmac::Key,
}

impl MediaGrantKey {
    /// Loads a key from its ID and a canonical unpadded base64url secret of at least 32 bytes.
    ///
    /// # Errors
    ///
    /// Returns [`MediaGrantKeyError`] when the ID or the secret encoding is invalid.
    pub fn from_base64url(
        key_id: impl Into<String>,
        secret_base64url: &str,
    ) -> Result<Self, MediaGrantKeyError> {
        let key_id = key_id.into();
        if !valid_key_id(&key_id) {
            return Err(MediaGrantKeyError::InvalidKeyId);
        }
        let secret = decode_canonical(secret_base64url)
            .map(Zeroizing::new)
            .ok_or(MediaGrantKeyError::InvalidSecretEncoding)?;
        if secret.len() < MIN_SECRET_BYTES {
            return Err(MediaGrantKeyError::SecretTooShort);
        }
        Ok(Self {
            key_id,
            key: hmac::Key::new(hmac::HMAC_SHA256, &secret),
        })
    }

    /// Returns the key ID carried in every grant this key signs.
    #[must_use]
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    /// Signs `path` until `expires`, both checked against `now` in Unix seconds.
    ///
    /// # Errors
    ///
    /// Returns [`MediaGrantError::InvalidPath`] for a path outside the grammar,
    /// [`MediaGrantError::InvalidQuery`] for an expiry over ten digits,
    /// [`MediaGrantError::Expired`] when `expires <= now`, and
    /// [`MediaGrantError::ExpiryTooFar`] when the lifetime exceeds
    /// [`MAX_GRANT_LIFETIME_SECONDS`].
    pub fn sign(&self, path: &str, expires: u64, now: u64) -> Result<MediaGrant, MediaGrantError> {
        if !valid_media_path(path) {
            return Err(MediaGrantError::InvalidPath);
        }
        if expires > MAX_EXPIRES {
            return Err(MediaGrantError::InvalidQuery);
        }
        check_expiry(expires, now, MAX_GRANT_LIFETIME_SECONDS)?;
        let tag = hmac::sign(
            &self.key,
            signing_input(path, expires, &self.key_id).as_bytes(),
        );
        Ok(MediaGrant {
            expires,
            key_id: self.key_id.clone(),
            signature: URL_SAFE_NO_PAD.encode(tag.as_ref()),
        })
    }

    fn verify_signature(
        &self,
        path: &str,
        expires: u64,
        signature: &[u8],
    ) -> Result<(), MediaGrantError> {
        hmac::verify(
            &self.key,
            signing_input(path, expires, &self.key_id).as_bytes(),
            signature,
        )
        .map_err(|_| MediaGrantError::InvalidSignature)
    }
}

impl fmt::Debug for MediaGrantKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MediaGrantKey")
            .field("key_id", &self.key_id)
            .field("secret", &REDACTED)
            .finish()
    }
}

/// The current signing key and, during rotation, the previous one.
///
/// Build one ring at startup and share it; it has no reload API. To rotate, restart with the new
/// key as current and the old key as previous, then restart without the previous key once the
/// longest grant lifetime has passed.
#[derive(Clone, Debug)]
pub struct MediaGrantKeyRing {
    current: MediaGrantKey,
    previous: Option<MediaGrantKey>,
}

impl MediaGrantKeyRing {
    /// Builds a ring. Grants signed by either key verify; new grants use `current`.
    ///
    /// # Errors
    ///
    /// Returns [`MediaGrantKeyError::DuplicateKeyId`] when both keys share an ID.
    pub fn new(
        current: MediaGrantKey,
        previous: Option<MediaGrantKey>,
    ) -> Result<Self, MediaGrantKeyError> {
        if previous
            .as_ref()
            .is_some_and(|key| key.key_id == current.key_id)
        {
            return Err(MediaGrantKeyError::DuplicateKeyId);
        }
        Ok(Self { current, previous })
    }

    /// Returns the key that signs new grants.
    #[must_use]
    pub fn current(&self) -> &MediaGrantKey {
        &self.current
    }

    /// Returns the retiring key that still verifies, if any.
    #[must_use]
    pub fn previous(&self) -> Option<&MediaGrantKey> {
        self.previous.as_ref()
    }

    /// Signs with the current key. See [`MediaGrantKey::sign`].
    ///
    /// # Errors
    ///
    /// Returns the errors of [`MediaGrantKey::sign`].
    pub fn sign(&self, path: &str, expires: u64, now: u64) -> Result<MediaGrant, MediaGrantError> {
        self.current.sign(path, expires, now)
    }

    /// Verifies a grant request, comparing the signature in constant time.
    ///
    /// # Errors
    ///
    /// Returns the first failing check in this order: method, path, query, expiry, key,
    /// signature.
    pub fn verify(
        &self,
        request: MediaGrantRequest<'_>,
    ) -> Result<VerifiedMediaGrant, MediaGrantError> {
        if !matches!(request.method, "GET" | "HEAD") {
            return Err(MediaGrantError::InvalidMethod);
        }
        if !valid_media_path(request.path) {
            return Err(MediaGrantError::InvalidPath);
        }
        let query = parse_query(request.query)?;
        check_expiry(
            query.expires,
            request.now,
            MAX_GRANT_LIFETIME_SECONDS + MAX_CLOCK_SKEW_SECONDS,
        )?;
        let key = self.key(query.key_id).ok_or(MediaGrantError::UnknownKey)?;
        key.verify_signature(request.path, query.expires, &query.signature)?;
        Ok(VerifiedMediaGrant {
            expires: query.expires,
            key_id: key.key_id.clone(),
        })
    }

    fn key(&self, key_id: &str) -> Option<&MediaGrantKey> {
        if self.current.key_id == key_id {
            return Some(&self.current);
        }
        self.previous.as_ref().filter(|key| key.key_id == key_id)
    }
}

/// A signed grant for one path.
#[derive(Clone, Eq, PartialEq)]
pub struct MediaGrant {
    expires: u64,
    key_id: String,
    signature: String,
}

impl MediaGrant {
    /// Returns the expiry in Unix seconds.
    #[must_use]
    pub const fn expires(&self) -> u64 {
        self.expires
    }

    /// Returns the ID of the signing key.
    #[must_use]
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    /// Returns the query to append after `?`. It is a bearer credential until it expires.
    #[must_use]
    pub fn query(&self) -> String {
        format!(
            "{EXPIRES_PARAM}{}&{KEY_ID_PARAM}{}&{MODE_PARAM}{MEDIA_GRANT_MODE}&{SIGNATURE_PARAM}{}",
            self.expires, self.key_id, self.signature
        )
    }
}

impl fmt::Debug for MediaGrant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MediaGrant")
            .field("expires", &self.expires)
            .field("key_id", &self.key_id)
            .field("signature", &REDACTED)
            .finish()
    }
}

/// A request to verify: the raw method, the raw path, the raw query, and the verifier's clock.
///
/// Pass the path exactly as received, before percent-decoding or dot-segment removal.
#[derive(Clone, Copy)]
pub struct MediaGrantRequest<'a> {
    /// The HTTP method.
    pub method: &'a str,
    /// The raw request path.
    pub path: &'a str,
    /// The raw query without the leading `?`.
    pub query: &'a str,
    /// The verifier's current time in Unix seconds.
    pub now: u64,
}

impl fmt::Debug for MediaGrantRequest<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MediaGrantRequest")
            .field("method", &self.method)
            .field("path", &self.path)
            .field("query", &REDACTED)
            .field("now", &self.now)
            .finish()
    }
}

/// A grant that passed verification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedMediaGrant {
    expires: u64,
    key_id: String,
}

impl VerifiedMediaGrant {
    /// Returns the expiry in Unix seconds.
    #[must_use]
    pub const fn expires(&self) -> u64 {
        self.expires
    }

    /// Returns the ID of the key that verified the grant.
    #[must_use]
    pub fn key_id(&self) -> &str {
        &self.key_id
    }
}

/// Returns the exact bytes the signature covers: `"{path}\n{expires}\nplayback\n{key_id}"`.
#[must_use]
pub fn signing_input(path: &str, expires: u64, key_id: &str) -> String {
    format!("{path}\n{expires}\n{MEDIA_GRANT_MODE}\n{key_id}")
}

/// Returns whether `path` is a normalized media path.
///
/// The path starts with `/`, is at most [`MAX_PATH_BYTES`] bytes, and every segment is one or more
/// of `[A-Za-z0-9._-]` not starting with `.`. That excludes percent-encoding, empty segments,
/// dot segments, and hidden files, so a valid path is already in the form a proxy normalizes to.
#[must_use]
pub fn valid_media_path(path: &str) -> bool {
    path.len() <= MAX_PATH_BYTES
        && path
            .strip_prefix('/')
            .is_some_and(|rest| rest.split('/').all(valid_path_segment))
}

struct ParsedQuery<'a> {
    expires: u64,
    key_id: &'a str,
    signature: Vec<u8>,
}

fn parse_query(raw: &str) -> Result<ParsedQuery<'_>, MediaGrantError> {
    if raw.len() > MAX_QUERY_BYTES {
        return Err(MediaGrantError::InvalidQuery);
    }
    let mut params = raw.split('&');
    let expires = param(params.next(), EXPIRES_PARAM)?;
    let key_id = param(params.next(), KEY_ID_PARAM)?;
    let mode = param(params.next(), MODE_PARAM)?;
    let signature = param(params.next(), SIGNATURE_PARAM)?;
    if params.next().is_some() || mode != MEDIA_GRANT_MODE || !valid_key_id(key_id) {
        return Err(MediaGrantError::InvalidQuery);
    }
    let expires = parse_expires(expires).ok_or(MediaGrantError::InvalidQuery)?;
    let signature = decode_signature(signature).ok_or(MediaGrantError::InvalidQuery)?;
    Ok(ParsedQuery {
        expires,
        key_id,
        signature,
    })
}

fn param<'a>(part: Option<&'a str>, name: &str) -> Result<&'a str, MediaGrantError> {
    part.and_then(|value| value.strip_prefix(name))
        .filter(|value| !value.is_empty())
        .ok_or(MediaGrantError::InvalidQuery)
}

fn parse_expires(text: &str) -> Option<u64> {
    let canonical = text.len() <= MAX_EXPIRES_DIGITS
        && text.bytes().all(|byte| byte.is_ascii_digit())
        && !text.starts_with('0');
    canonical.then(|| text.parse().ok()).flatten()
}

fn decode_signature(text: &str) -> Option<Vec<u8>> {
    if text.len() != SIGNATURE_BASE64URL_BYTES {
        return None;
    }
    decode_canonical(text).filter(|bytes| bytes.len() == SIGNATURE_BYTES)
}

fn decode_canonical(text: &str) -> Option<Vec<u8>> {
    if text.is_empty() || !text.bytes().all(is_base64url_byte) {
        return None;
    }
    let bytes = URL_SAFE_NO_PAD.decode(text).ok()?;
    (URL_SAFE_NO_PAD.encode(&bytes) == text).then_some(bytes)
}

fn check_expiry(expires: u64, now: u64, max_ahead: u64) -> Result<(), MediaGrantError> {
    if expires <= now {
        return Err(MediaGrantError::Expired);
    }
    if expires - now > max_ahead {
        return Err(MediaGrantError::ExpiryTooFar);
    }
    Ok(())
}

fn valid_key_id(value: &str) -> bool {
    value.len() <= MAX_KEY_ID_BYTES
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value.bytes().all(is_base64url_byte)
}

fn valid_path_segment(segment: &str) -> bool {
    !segment.is_empty()
        && !segment.starts_with('.')
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn is_base64url_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde::Deserialize;

    use super::*;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Corpus {
        version: u64,
        protocol: Protocol,
        error_codes: ErrorCodes,
        keys: Vec<KeyFixture>,
        key_cases: Vec<KeyCase>,
        ring_cases: Vec<RingCase>,
        sign_cases: Vec<SignCase>,
        verify_cases: Vec<VerifyCase>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Protocol {
        mode: String,
        query_order: Vec<String>,
        max_lifetime_seconds: u64,
        max_clock_skew_seconds: u64,
        min_secret_bytes: usize,
        max_key_id_bytes: usize,
        max_path_bytes: usize,
        max_expires_digits: usize,
        methods: Vec<String>,
    }

    #[derive(Deserialize)]
    struct ErrorCodes {
        key: Vec<String>,
        grant: Vec<String>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct KeyFixture {
        name: String,
        key_id: String,
        secret_base64url: String,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct KeyCase {
        name: String,
        key_id: String,
        secret_base64url: String,
        expected: KeyExpectation,
    }

    #[derive(Deserialize)]
    #[serde(untagged, rename_all_fields = "camelCase")]
    enum KeyExpectation {
        Error { error: String },
        Key { key_id: String },
    }

    #[derive(Deserialize)]
    struct RingCase {
        name: String,
        current: String,
        previous: Option<String>,
        expected: RingExpectation,
    }

    #[derive(Deserialize)]
    #[serde(untagged, rename_all_fields = "camelCase")]
    enum RingExpectation {
        Error { error: String },
        Ring { key_ids: Vec<String> },
    }

    #[derive(Deserialize)]
    struct SignCase {
        name: String,
        key: String,
        path: String,
        expires: u64,
        now: u64,
        expected: SignExpectation,
    }

    #[derive(Deserialize)]
    #[serde(untagged, rename_all_fields = "camelCase")]
    enum SignExpectation {
        Error {
            error: String,
        },
        Grant {
            signing_input: String,
            query: String,
        },
    }

    #[derive(Deserialize)]
    struct VerifyCase {
        name: String,
        method: String,
        path: String,
        query: String,
        now: u64,
        current: String,
        previous: Option<String>,
        expected: VerifyExpectation,
    }

    #[derive(Deserialize)]
    #[serde(untagged, rename_all_fields = "camelCase")]
    enum VerifyExpectation {
        Error { error: String },
        Grant { key_id: String, expires: u64 },
    }

    fn corpus() -> Corpus {
        serde_json::from_str(include_str!(
            "../../../../fixtures/media-grants/vectors-v1.json"
        ))
        .expect("media grant vectors should parse")
    }

    fn keys(corpus: &Corpus) -> HashMap<&str, MediaGrantKey> {
        corpus
            .keys
            .iter()
            .map(|fixture| {
                let key = MediaGrantKey::from_base64url(&fixture.key_id, &fixture.secret_base64url)
                    .unwrap_or_else(|error| panic!("key {}: {}", fixture.name, error.code()));
                (fixture.name.as_str(), key)
            })
            .collect()
    }

    fn named<'a>(keys: &'a HashMap<&str, MediaGrantKey>, name: &str) -> &'a MediaGrantKey {
        keys.get(name)
            .unwrap_or_else(|| panic!("missing fixture key {name}"))
    }

    fn ring(
        keys: &HashMap<&str, MediaGrantKey>,
        current: &str,
        previous: Option<&str>,
    ) -> Result<MediaGrantKeyRing, MediaGrantKeyError> {
        MediaGrantKeyRing::new(
            named(keys, current).clone(),
            previous.map(|name| named(keys, name).clone()),
        )
    }

    #[test]
    fn protocol_constants_match_the_vectors() {
        let corpus = corpus();
        let protocol = &corpus.protocol;
        assert_eq!(corpus.version, 1);
        assert_eq!(protocol.mode, MEDIA_GRANT_MODE);
        assert_eq!(
            protocol.query_order,
            ["expires", "keyId", "mode", "signature"]
        );
        assert_eq!(protocol.max_lifetime_seconds, MAX_GRANT_LIFETIME_SECONDS);
        assert_eq!(protocol.max_clock_skew_seconds, MAX_CLOCK_SKEW_SECONDS);
        assert_eq!(protocol.min_secret_bytes, MIN_SECRET_BYTES);
        assert_eq!(protocol.max_key_id_bytes, MAX_KEY_ID_BYTES);
        assert_eq!(protocol.max_path_bytes, MAX_PATH_BYTES);
        assert_eq!(protocol.max_expires_digits, MAX_EXPIRES_DIGITS);
        assert_eq!(protocol.methods, ["GET", "HEAD"]);
        assert_eq!(MAX_QUERY_BYTES, 157);
    }

    #[test]
    fn error_codes_match_the_vectors() {
        let corpus = corpus();
        let key_codes = [
            MediaGrantKeyError::InvalidKeyId,
            MediaGrantKeyError::InvalidSecretEncoding,
            MediaGrantKeyError::SecretTooShort,
            MediaGrantKeyError::DuplicateKeyId,
        ]
        .map(MediaGrantKeyError::code);
        let grant_codes = [
            MediaGrantError::InvalidMethod,
            MediaGrantError::InvalidPath,
            MediaGrantError::InvalidQuery,
            MediaGrantError::Expired,
            MediaGrantError::ExpiryTooFar,
            MediaGrantError::UnknownKey,
            MediaGrantError::InvalidSignature,
        ]
        .map(MediaGrantError::code);
        assert_eq!(corpus.error_codes.key, key_codes);
        assert_eq!(corpus.error_codes.grant, grant_codes);
    }

    #[test]
    fn key_cases_load_or_fail_with_the_expected_code() {
        for case in corpus().key_cases {
            let result = MediaGrantKey::from_base64url(&case.key_id, &case.secret_base64url);
            match (result, &case.expected) {
                (Ok(key), KeyExpectation::Key { key_id }) => {
                    assert_eq!(key.key_id(), key_id, "{}", case.name);
                }
                (Err(error), KeyExpectation::Error { error: code }) => {
                    assert_eq!(error.code(), code, "{}", case.name);
                }
                (Ok(_), _) => panic!("{} loaded", case.name),
                (Err(error), _) => panic!("{} failed with {}", case.name, error.code()),
            }
        }
    }

    #[test]
    fn ring_cases_build_or_fail_with_the_expected_code() {
        let corpus = corpus();
        let keys = keys(&corpus);
        for case in &corpus.ring_cases {
            let result = ring(&keys, &case.current, case.previous.as_deref());
            match (result, &case.expected) {
                (Ok(ring), RingExpectation::Ring { key_ids }) => {
                    let actual: Vec<&str> = std::iter::once(ring.current())
                        .chain(ring.previous())
                        .map(MediaGrantKey::key_id)
                        .collect();
                    assert_eq!(actual, *key_ids, "{}", case.name);
                }
                (Err(error), RingExpectation::Error { error: code }) => {
                    assert_eq!(error.code(), code, "{}", case.name);
                }
                (Ok(_), _) => panic!("{} built", case.name),
                (Err(error), _) => panic!("{} failed with {}", case.name, error.code()),
            }
        }
    }

    #[test]
    fn sign_cases_produce_the_independent_signatures() {
        let corpus = corpus();
        let keys = keys(&corpus);
        for case in &corpus.sign_cases {
            let key = named(&keys, &case.key);
            let result = key.sign(&case.path, case.expires, case.now);
            match (result, &case.expected) {
                (
                    Ok(grant),
                    SignExpectation::Grant {
                        signing_input: input,
                        query,
                    },
                ) => {
                    assert_eq!(
                        signing_input(&case.path, case.expires, key.key_id()),
                        *input,
                        "{}",
                        case.name
                    );
                    assert_eq!(grant.query(), *query, "{}", case.name);
                    assert_eq!(grant.expires(), case.expires, "{}", case.name);
                    assert_eq!(grant.key_id(), key.key_id(), "{}", case.name);
                }
                (Err(error), SignExpectation::Error { error: code }) => {
                    assert_eq!(error.code(), code, "{}", case.name);
                }
                (Ok(_), _) => panic!("{} signed", case.name),
                (Err(error), _) => panic!("{} failed with {}", case.name, error.code()),
            }
        }
    }

    #[test]
    fn verify_cases_accept_or_reject_with_the_expected_code() {
        let corpus = corpus();
        let keys = keys(&corpus);
        for case in &corpus.verify_cases {
            let ring = ring(&keys, &case.current, case.previous.as_deref())
                .unwrap_or_else(|error| panic!("{} ring: {}", case.name, error.code()));
            let result = ring.verify(MediaGrantRequest {
                method: &case.method,
                path: &case.path,
                query: &case.query,
                now: case.now,
            });
            match (result, &case.expected) {
                (Ok(grant), VerifyExpectation::Grant { key_id, expires }) => {
                    assert_eq!(grant.key_id(), key_id, "{}", case.name);
                    assert_eq!(grant.expires(), *expires, "{}", case.name);
                }
                (Err(error), VerifyExpectation::Error { error: code }) => {
                    assert_eq!(error.code(), code, "{}", case.name);
                }
                (Ok(_), _) => panic!("{} verified", case.name),
                (Err(error), _) => panic!("{} failed with {}", case.name, error.code()),
            }
        }
    }

    #[test]
    fn signed_grants_round_trip_through_a_rotated_ring() {
        let corpus = corpus();
        let keys = keys(&corpus);
        let before = ring(&keys, "current", None).expect("ring before rotation");
        let after = ring(&keys, "next", Some("current")).expect("ring after rotation");
        let path = "/media/clip.mp4";
        let now = 2_000_000_000;
        let old = before.sign(path, now + 60, now).expect("old grant");
        let new = after.sign(path, now + 60, now).expect("new grant");
        for (grant, key_id) in [(&old, "current_2026_09"), (&new, "next-2026-10")] {
            let verified = after
                .verify(MediaGrantRequest {
                    method: "GET",
                    path,
                    query: &grant.query(),
                    now,
                })
                .expect("both keys verify during rotation");
            assert_eq!(verified.key_id(), key_id);
        }
        assert_eq!(
            before
                .verify(MediaGrantRequest {
                    method: "GET",
                    path,
                    query: &new.query(),
                    now,
                })
                .map_err(MediaGrantError::code),
            Err("unknown_key")
        );
    }

    #[test]
    fn debug_and_errors_never_show_key_material_or_signatures() {
        let corpus = corpus();
        let fixture = &corpus.keys[0];
        let keys = keys(&corpus);
        let key = named(&keys, &fixture.name);
        let ring = ring(&keys, &fixture.name, Some("previous")).expect("ring");
        let grant = key
            .sign("/media/clip.mp4", 2_000_000_060, 2_000_000_000)
            .expect("grant");
        let query = grant.query();
        let signature = query
            .rsplit_once("signature=")
            .map(|(_, signature)| signature)
            .expect("signature");
        let request = MediaGrantRequest {
            method: "GET",
            path: "/media/clip.mp4",
            query: &query,
            now: 2_000_000_000,
        };
        let rendered = [
            format!("{key:?}"),
            format!("{ring:?}"),
            format!("{grant:?}"),
            format!("{request:?}"),
            MediaGrantKey::from_base64url("short", "eHh4")
                .expect_err("short secret")
                .to_string(),
        ];
        for text in &rendered {
            assert!(!text.contains(&fixture.secret_base64url), "{text}");
            assert!(!text.contains(signature), "{text}");
        }
        assert!(rendered[0].contains(&fixture.key_id));
        assert!(rendered[3].contains(REDACTED));
    }
}
