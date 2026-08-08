use axum::http::{HeaderValue, header::InvalidHeaderValue};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::{Deserialize, Serialize};

/// Configurable registered claims for authentication fixtures.
///
/// Omitted values are not serialized, making it possible to exercise handlers
/// that accept or reject each registered claim independently.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct JwtClaims {
    /// Subject (`sub`) identifying the test principal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    /// Issuer (`iss`) expected by the test application.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iss: Option<String>,
    /// Audience (`aud`) expected by the test application.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aud: Option<String>,
    /// Expiry (`exp`) as seconds since the Unix epoch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exp: Option<u64>,
}

impl JwtClaims {
    /// Creates an empty claims set.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            sub: None,
            iss: None,
            aud: None,
            exp: None,
        }
    }

    /// Sets the subject claim.
    #[must_use]
    pub fn subject(mut self, subject: impl Into<String>) -> Self {
        self.sub = Some(subject.into());
        self
    }

    /// Sets the issuer claim.
    #[must_use]
    pub fn issuer(mut self, issuer: impl Into<String>) -> Self {
        self.iss = Some(issuer.into());
        self
    }

    /// Sets the audience claim.
    #[must_use]
    pub fn audience(mut self, audience: impl Into<String>) -> Self {
        self.aud = Some(audience.into());
        self
    }

    /// Sets the expiry claim in Unix-epoch seconds.
    #[must_use]
    pub const fn expires_at(mut self, expiry: u64) -> Self {
        self.exp = Some(expiry);
        self
    }
}

/// Encodes claims as an HS256 JWT with a test-only shared secret.
pub fn hs256_token(
    secret: &[u8],
    claims: &JwtClaims,
) -> Result<String, jsonwebtoken::errors::Error> {
    encode(
        &Header::new(Algorithm::HS256),
        claims,
        &EncodingKey::from_secret(secret),
    )
}

/// Encodes claims as an RS256 JWT using a PEM-encoded RSA private key.
pub fn rs256_token(
    private_key_pem: &[u8],
    claims: &JwtClaims,
) -> Result<String, jsonwebtoken::errors::Error> {
    let key = EncodingKey::from_rsa_pem(private_key_pem)?;
    encode(&Header::new(Algorithm::RS256), claims, &key)
}

/// Creates an HTTP `Authorization` header value for a generated JWT.
pub fn authorization_header(token: &str) -> Result<HeaderValue, InvalidHeaderValue> {
    HeaderValue::from_str(&format!("Bearer {token}"))
}

#[cfg(test)]
mod tests {
    use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};

    use super::*;

    #[test]
    fn creates_hs256_fixture_with_configured_claims() -> Result<(), Box<dyn std::error::Error>> {
        let claims = JwtClaims::new()
            .subject("user-123")
            .issuer("fixture")
            .audience("api")
            .expires_at(4_102_444_800);
        let token = hs256_token(b"fixture-secret", &claims)?;
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(&["fixture"]);
        validation.set_audience(&["api"]);
        let decoded = decode::<JwtClaims>(
            &token,
            &DecodingKey::from_secret(b"fixture-secret"),
            &validation,
        )?;

        assert_eq!(decoded.claims, claims);
        assert_eq!(decode_header(&token)?.alg, Algorithm::HS256);
        assert_eq!(
            authorization_header(&token)?.to_str()?,
            format!("Bearer {token}")
        );
        Ok(())
    }
}
