use aws_lc_rs::{
    digest,
    rand::{SecureRandom as _, SystemRandom},
};
use thiserror::Error;

use super::{
    ApiTokenError, ApiTokenStoreError, BASE62, BASE62_LEN, DEFAULT_API_TOKEN_MARKER,
    DISPLAY_PREFIX_LENGTH, GeneratedApiToken, SECRET_LENGTH,
};

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
    ///
    /// # Errors
    ///
    /// Returns [`ApiTokenFormatError::MissingSeparator`] when `marker` does not end
    /// in `_`, and [`ApiTokenFormatError::InvalidMarker`] when the part before it is
    /// empty or contains anything other than ASCII alphanumerics.
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

    /// Generates a token secret, hash, and display prefix without a store.
    ///
    /// Uses the system CSPRNG and rejection sampling. Show the secret once in
    /// the creation response. Store only [`GeneratedApiToken::secret_hash`]
    /// and optionally [`GeneratedApiToken::display_prefix`]. Look up presented
    /// credentials by `hash_api_token(presented)`, as [`ApiTokenVerifier`] does.
    ///
    /// # Errors
    ///
    /// Returns [`ApiTokenError::Storage`] when the system CSPRNG cannot supply bytes.
    pub fn generate_secret(&self) -> Result<GeneratedApiToken, ApiTokenError> {
        let secret = self.generate(&SystemRandom::new())?;
        let secret_hash = hash_api_token(&secret);
        let display_prefix = self.display_prefix(&secret).to_owned();
        Ok(GeneratedApiToken {
            secret,
            secret_hash,
            display_prefix,
        })
    }

    /// Generates one token secret from the system CSPRNG.
    ///
    /// Bytes at or above the largest multiple of 62 are drawn again rather than
    /// reduced, so every alphabet character stays equally likely. Taking the
    /// remainder directly would make the first eight characters slightly more
    /// common.
    fn generate(&self, random: &SystemRandom) -> Result<String, ApiTokenError> {
        const LIMIT: u8 = u8::MAX / BASE62_LEN * BASE62_LEN;

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
                    secret.push(base62_char(byte % BASE62_LEN)?);
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

fn base62_char(digit: u8) -> Result<char, ApiTokenError> {
    BASE62
        .get(usize::from(digit))
        .map(|byte| char::from(*byte))
        .ok_or_else(|| {
            ApiTokenError::Storage(ApiTokenStoreError::internal(
                "base62 digit is outside the alphabet",
            ))
        })
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
