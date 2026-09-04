//! Credential checks that run before a connection is stored or resumed.

use std::{fmt, future::Future, pin::Pin, time::Duration};

use thiserror::Error;

use crate::ConnectionHealth;

/// Largest provider response accepted by the credential-probe contract.
///
/// Adapters must stop reading once this many bytes have been received. A body
/// beyond the limit maps to [`CredentialProbeError::InvalidData`].
pub const MAX_CREDENTIAL_PROBE_RESPONSE_BYTES: usize = 64 * 1024;

/// Largest external account identifier accepted by [`ExternalAccountId`].
pub const MAX_EXTERNAL_ACCOUNT_ID_BYTES: usize = 1024;

/// An external account identifier that Baukit carries without interpreting.
///
/// The identifier has no `Display` implementation and its `Debug` output is
/// redacted. Products may persist [`as_str`](Self::as_str), but should not put
/// it in logs, metrics, or public errors.
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct ExternalAccountId(String);

impl ExternalAccountId {
    /// Validates and wraps an external account identifier.
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidExternalAccountId> {
        let value = value.into();
        if value.is_empty() {
            return Err(InvalidExternalAccountId::Empty);
        }
        if value.len() > MAX_EXTERNAL_ACCOUNT_ID_BYTES {
            return Err(InvalidExternalAccountId::TooLong);
        }
        Ok(Self(value))
    }

    /// Returns the identifier for product-owned persistence or account mapping.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the owned identifier for a product account model.
    #[must_use]
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Debug for ExternalAccountId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ExternalAccountId")
            .field(&"[redacted]")
            .finish()
    }
}

/// Why an external account identifier could not be represented safely.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum InvalidExternalAccountId {
    /// The provider response contained no identifier.
    #[error("external account identifier is empty")]
    Empty,
    /// The identifier exceeded [`MAX_EXTERNAL_ACCOUNT_ID_BYTES`].
    #[error("external account identifier is too long")]
    TooLong,
}

/// A successful credential probe.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialProbeSuccess {
    /// Provider account identity, carried without provider-specific parsing.
    pub external_account_id: ExternalAccountId,
}

impl CredentialProbeSuccess {
    /// Creates a successful probe result.
    #[must_use]
    pub const fn new(external_account_id: ExternalAccountId) -> Self {
        Self {
            external_account_id,
        }
    }

    /// Returns the connection health established by a successful probe.
    #[must_use]
    pub const fn health(&self) -> ConnectionHealth {
        ConnectionHealth::Healthy
    }
}

/// A safe, provider-neutral credential-probe failure.
///
/// Variants carry no provider response text. `Display` and `Debug` are safe for
/// public error conversion and outcome-only logs.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum CredentialProbeError {
    /// The provider rejected or revoked the credential.
    #[error("provider credential was revoked")]
    Revoked,
    /// The credential is valid but lacks product-required access.
    #[error("provider credential is missing required access")]
    MissingScope,
    /// The provider rate limited the check.
    #[error("provider credential probe was rate limited")]
    RateLimited {
        /// Provider retry delay parsed from `Retry-After`, when valid.
        retry_after: Option<Duration>,
    },
    /// The check exceeded the adapter's finite timeout.
    #[error("provider credential probe timed out")]
    Timeout,
    /// The provider could not be reached or returned a transient server error.
    #[error("provider credential probe is unavailable")]
    Unavailable,
    /// The provider returned an unexpected status, malformed data, or too much data.
    #[error("provider credential probe returned invalid data")]
    InvalidData,
}

impl CredentialProbeError {
    /// Creates a rate-limit failure while preserving a provider retry delay.
    #[must_use]
    pub const fn rate_limited(retry_after: Option<Duration>) -> Self {
        Self::RateLimited { retry_after }
    }

    /// Returns the stable outcome code for persistence, metrics, and public mapping.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Revoked => "revoked",
            Self::MissingScope => "missing_scope",
            Self::RateLimited { .. } => "rate_limited",
            Self::Timeout => "timeout",
            Self::Unavailable => "unavailable",
            Self::InvalidData => "invalid_data",
        }
    }

    /// Returns the connection health established by this failure.
    #[must_use]
    pub const fn health(self) -> ConnectionHealth {
        match self {
            Self::Revoked | Self::MissingScope => ConnectionHealth::NeedsReconnect,
            Self::RateLimited { .. } | Self::Timeout | Self::Unavailable => {
                ConnectionHealth::Degraded
            }
            Self::InvalidData => ConnectionHealth::Failed,
        }
    }

    /// Returns whether retrying the same probe may succeed.
    #[must_use]
    pub const fn is_retryable(self) -> bool {
        matches!(
            self,
            Self::RateLimited { .. } | Self::Timeout | Self::Unavailable
        )
    }

    /// Returns the provider's retry delay for a rate limit, when present.
    #[must_use]
    pub const fn retry_after(self) -> Option<Duration> {
        match self {
            Self::RateLimited { retry_after } => retry_after,
            _ => None,
        }
    }
}

/// Result returned by [`CredentialProbe::probe`].
pub type CredentialProbeResult = Result<CredentialProbeSuccess, CredentialProbeError>;

/// Future returned by [`CredentialProbe::probe`].
pub type CredentialProbeFuture<'a> =
    Pin<Box<dyn Future<Output = CredentialProbeResult> + Send + 'a>>;

/// Product adapter that checks one provider credential.
///
/// Implementations own the endpoint, headers, required scopes, response
/// parsing, and provider account model. They must use a finite request timeout,
/// stop reading at [`MAX_CREDENTIAL_PROBE_RESPONSE_BYTES`], discard provider
/// response text, and map the result to [`CredentialProbeError`].
pub trait CredentialProbe: Send + Sync {
    /// Checks `credential` and returns its opaque external account identity.
    fn probe<'a>(&'a self, credential: &'a [u8]) -> CredentialProbeFuture<'a>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_account_ids_are_bounded_and_redacted() {
        assert_eq!(
            ExternalAccountId::new(""),
            Err(InvalidExternalAccountId::Empty)
        );
        assert_eq!(
            ExternalAccountId::new("x".repeat(MAX_EXTERNAL_ACCOUNT_ID_BYTES + 1)),
            Err(InvalidExternalAccountId::TooLong)
        );

        let account_id = ExternalAccountId::new("private-account-42").expect("valid identifier");
        assert_eq!(account_id.as_str(), "private-account-42");
        assert_eq!(
            format!("{account_id:?}"),
            "ExternalAccountId(\"[redacted]\")"
        );
    }

    #[test]
    fn outcomes_have_stable_health_and_retry_behavior() {
        let success = CredentialProbeSuccess::new(
            ExternalAccountId::new("account").expect("valid identifier"),
        );
        assert_eq!(success.health(), ConnectionHealth::Healthy);

        let limited = CredentialProbeError::rate_limited(Some(Duration::from_secs(41)));
        assert_eq!(limited.code(), "rate_limited");
        assert_eq!(limited.health(), ConnectionHealth::Degraded);
        assert_eq!(limited.retry_after(), Some(Duration::from_secs(41)));
        assert!(limited.is_retryable());

        for error in [
            CredentialProbeError::Revoked,
            CredentialProbeError::MissingScope,
        ] {
            assert_eq!(error.health(), ConnectionHealth::NeedsReconnect);
            assert!(!error.is_retryable());
        }
        assert_eq!(
            CredentialProbeError::InvalidData.health(),
            ConnectionHealth::Failed
        );
        for error in [
            CredentialProbeError::Timeout,
            CredentialProbeError::Unavailable,
        ] {
            assert_eq!(error.health(), ConnectionHealth::Degraded);
            assert!(error.is_retryable());
            assert_eq!(error.retry_after(), None);
        }
    }

    #[test]
    fn public_error_formatting_contains_no_dynamic_text() {
        let provider_text = "private-provider-response";
        for error in [
            CredentialProbeError::Revoked,
            CredentialProbeError::MissingScope,
            CredentialProbeError::rate_limited(Some(Duration::from_secs(17))),
            CredentialProbeError::Timeout,
            CredentialProbeError::Unavailable,
            CredentialProbeError::InvalidData,
        ] {
            assert!(!error.to_string().contains(provider_text));
            assert!(!format!("{error:?}").contains(provider_text));
        }
    }
}
