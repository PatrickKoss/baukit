use baukit_egress::{EgressClientError, EgressError, EgressOptionsError};
use reqwest::StatusCode;
use thiserror::Error;

/// Failure while discovering an issuer or verifying an access token.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum VerificationError {
    /// Verification could not complete because a backing service is unavailable.
    #[error("identity verification is temporarily unavailable")]
    Unavailable,
    /// The HTTP client could not be created.
    #[error("could not create OIDC HTTP client")]
    Client(#[source] EgressClientError),
    /// The issuer or JWKS URL cannot form the default egress allowance.
    #[error("OIDC endpoint cannot be used as an egress endpoint")]
    EgressEndpoint,
    /// The egress options derived from the configuration are invalid.
    #[error("invalid OIDC egress options")]
    EgressOptions(#[source] EgressOptionsError),
    /// OIDC discovery exceeded the configured timeout.
    #[error("OIDC discovery timed out")]
    DiscoveryTimeout,
    /// OIDC discovery failed while sending the request or reading the response.
    #[error("OIDC discovery request failed")]
    DiscoveryRequest(#[source] Box<EgressError>),
    /// OIDC discovery returned a non-success status.
    #[error("OIDC discovery returned HTTP {0}")]
    DiscoveryStatus(StatusCode),
    /// OIDC discovery returned invalid JSON.
    #[error("OIDC discovery document was invalid")]
    InvalidDiscoveryDocument(#[source] serde_json::Error),
    /// Discovery metadata did not report the configured issuer exactly.
    #[error("OIDC discovery issuer did not match configuration")]
    DiscoveryIssuerMismatch,
    /// Discovery metadata contained an invalid JWKS URL.
    #[error("OIDC discovery returned an invalid JWKS URI")]
    InvalidJwksUri,
    /// A JWKS request exceeded the configured timeout.
    #[error("JWKS request timed out")]
    JwksTimeout,
    /// JWKS failed while sending the request or reading the response.
    #[error("JWKS request failed")]
    JwksRequest(#[source] Box<EgressError>),
    /// JWKS returned a non-success status.
    #[error("JWKS returned HTTP {0}")]
    JwksStatus(StatusCode),
    /// JWKS returned invalid JSON.
    #[error("JWKS document was invalid")]
    InvalidJwksDocument(#[source] serde_json::Error),
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
    /// An access token lacked the required `typ: at+jwt` header or carried an ID-token `nonce`.
    #[error("token type is not an access token")]
    WrongTokenType,
    /// The ID token's nonce was missing, malformed, or did not match the login request.
    #[error("ID token nonce is invalid")]
    WrongNonce,
    /// ID-token verification requires an explicitly configured OAuth client ID.
    #[error("ID token verification requires a configured client ID")]
    MissingClientId,
    /// A WorkOS token did not name the configured application client.
    #[error("access token client ID is invalid")]
    WrongClientId,
    /// The token did not grant every scope the verifier requires.
    #[error("access token lacks a required scope")]
    MissingScope,
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

impl VerificationError {
    /// Returns whether verification failed because infrastructure is temporarily unavailable.
    ///
    /// Retry these failures without asking the caller to replace its credentials.
    #[must_use]
    pub fn is_unavailable(&self) -> bool {
        match self {
            Self::Unavailable | Self::DiscoveryTimeout | Self::JwksTimeout => true,
            Self::DiscoveryRequest(error) | Self::JwksRequest(error) => error.is_retryable(),
            Self::DiscoveryStatus(status) | Self::JwksStatus(status) => {
                is_transient_status(*status)
            }
            _ => false,
        }
    }

    /// Returns whether verification failed because of server or provider configuration.
    ///
    /// These failures need operator attention rather than a client retry or sign-in.
    #[must_use]
    pub fn is_misconfigured(&self) -> bool {
        match self {
            Self::Client(_)
            | Self::EgressEndpoint
            | Self::EgressOptions(_)
            | Self::InvalidDiscoveryDocument(_)
            | Self::DiscoveryIssuerMismatch
            | Self::InvalidJwksUri
            | Self::InvalidJwksDocument(_)
            | Self::InvalidJwk
            | Self::MissingClientId
            | Self::Clock => true,
            Self::DiscoveryStatus(status) | Self::JwksStatus(status) => {
                !is_transient_status(*status)
            }
            Self::DiscoveryRequest(error) | Self::JwksRequest(error) => !error.is_retryable(),
            _ => false,
        }
    }
}

fn is_transient_status(status: StatusCode) -> bool {
    status.is_server_error()
        || matches!(
            status,
            StatusCode::TOO_MANY_REQUESTS | StatusCode::REQUEST_TIMEOUT
        )
}
