//! Push configuration and the validated options the adapter consumes.

use std::time::Duration;

use baukit_config::{Secret, Validate, ValidationError, ValidationErrors};
use serde::{Deserialize, Deserializer};
use thiserror::Error;

/// Expo's public push send endpoint.
pub const DEFAULT_EXPO_ENDPOINT: &str = "https://exp.host/--/api/v2/push/send";

/// Largest batch Expo accepts in one `/push/send` request.
pub const MAX_BATCH_SIZE: usize = 100;

const DEFAULT_BATCH_SIZE: usize = 100;
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(8);

/// Deserializable push configuration for a product's config section.
///
/// Push is opt-in, so this does not live in `baukit_config::BaukitConfig`.
/// Products embed it in their own product config, which puts environment
/// overrides on the usual nested path, for example `ORDERS__PUSH__BATCH_SIZE`.
/// `request_timeout_ms` is read in milliseconds and exposed as a [`Duration`].
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct PushConfig {
    /// Full push send endpoint URL.
    pub endpoint: String,
    /// Provider access token. Debug and display formatting redact its value.
    ///
    /// Expo only requires this when the project enforces push security.
    pub access_token: Secret<String>,
    /// Number of notifications sent in one upstream request.
    pub batch_size: usize,
    /// Timeout applied independently to the ticket and receipt requests.
    #[serde(
        rename = "request_timeout_ms",
        deserialize_with = "duration_from_millis"
    )]
    pub request_timeout: Duration,
}

fn duration_from_millis<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    u64::deserialize(deserializer).map(Duration::from_millis)
}

impl Default for PushConfig {
    fn default() -> Self {
        Self {
            endpoint: DEFAULT_EXPO_ENDPOINT.to_owned(),
            access_token: Secret::new(String::new()),
            batch_size: DEFAULT_BATCH_SIZE,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }
}

impl Validate for PushConfig {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Vec::new();
        if self.endpoint.trim().is_empty() {
            errors.push(ValidationError::new("endpoint", "must not be empty"));
        } else if validated_endpoints(&self.endpoint).is_err() {
            errors.push(ValidationError::new(
                "endpoint",
                "must be an absolute HTTP URL ending in /send",
            ));
        }
        if self.batch_size == 0 || self.batch_size > MAX_BATCH_SIZE {
            errors.push(ValidationError::new(
                "batch_size",
                format!("must be between 1 and {MAX_BATCH_SIZE}"),
            ));
        }
        if self.request_timeout.is_zero() {
            errors.push(ValidationError::new(
                "request_timeout_ms",
                "must be greater than zero",
            ));
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationErrors::new(errors))
        }
    }
}

/// Validated options an [`ExpoPushSender`](crate::ExpoPushSender) is built from.
///
/// Construct these with [`PushOptions::new`] in code, or with
/// [`PushOptions::from_config`] from a loaded [`PushConfig`]. Both reject the
/// same invalid values, so a bad endpoint or batch size fails at startup rather
/// than on the first notification.
#[derive(Clone)]
pub struct PushOptions {
    endpoint: String,
    receipts_endpoint: String,
    access_token: Option<String>,
    batch_size: usize,
    request_timeout: Duration,
}

impl PushOptions {
    /// Creates options for a full push send endpoint URL.
    pub fn new(endpoint: impl Into<String>) -> Result<Self, PushOptionsError> {
        let endpoint = endpoint.into();
        let (endpoint, receipts_endpoint) = validated_endpoints(&endpoint)?;
        Ok(Self {
            endpoint,
            receipts_endpoint,
            access_token: None,
            batch_size: DEFAULT_BATCH_SIZE,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        })
    }

    /// Converts a loaded configuration section into validated options.
    ///
    /// An empty `access_token` means the project does not enforce push
    /// security, so no `Authorization` header is sent.
    pub fn from_config(config: &PushConfig) -> Result<Self, PushOptionsError> {
        config.validate()?;
        let access_token = config.access_token.expose().trim();
        let (endpoint, receipts_endpoint) = validated_endpoints(&config.endpoint)?;
        Ok(Self {
            endpoint,
            receipts_endpoint,
            access_token: (!access_token.is_empty()).then(|| access_token.to_owned()),
            batch_size: config.batch_size,
            request_timeout: config.request_timeout,
        })
    }

    /// Sets the bearer token sent with every push request.
    ///
    /// An empty or whitespace-only token is rejected; omit the call instead.
    pub fn with_access_token(
        mut self,
        access_token: impl Into<String>,
    ) -> Result<Self, PushOptionsError> {
        let access_token = access_token.into();
        if access_token.trim().is_empty() {
            return Err(PushOptionsError::EmptyAccessToken);
        }
        self.access_token = Some(access_token);
        Ok(self)
    }

    /// Sets how many notifications are sent in one upstream request.
    pub fn with_batch_size(mut self, batch_size: usize) -> Result<Self, PushOptionsError> {
        if batch_size == 0 || batch_size > MAX_BATCH_SIZE {
            return Err(PushOptionsError::InvalidBatchSize(batch_size));
        }
        self.batch_size = batch_size;
        Ok(self)
    }

    /// Sets the timeout applied to each upstream request.
    pub fn with_request_timeout(
        mut self,
        request_timeout: Duration,
    ) -> Result<Self, PushOptionsError> {
        if request_timeout.is_zero() {
            return Err(PushOptionsError::ZeroRequestTimeout);
        }
        self.request_timeout = request_timeout;
        Ok(self)
    }

    /// Returns the full push send endpoint URL.
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Returns the receipt endpoint derived next to the send endpoint.
    #[must_use]
    pub fn receipts_endpoint(&self) -> &str {
        &self.receipts_endpoint
    }

    /// Returns the access token, if one is configured.
    #[must_use]
    pub fn access_token(&self) -> Option<&str> {
        self.access_token.as_deref()
    }

    /// Returns the number of notifications sent in one upstream request.
    #[must_use]
    pub const fn batch_size(&self) -> usize {
        self.batch_size
    }

    /// Returns the per-request timeout.
    #[must_use]
    pub const fn request_timeout(&self) -> Duration {
        self.request_timeout
    }
}

impl Default for PushOptions {
    fn default() -> Self {
        Self::from_config(&PushConfig::default()).expect("the standard push defaults are valid")
    }
}

impl std::fmt::Debug for PushOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PushOptions")
            .field("endpoint", &self.endpoint)
            .field("receipts_endpoint", &self.receipts_endpoint)
            .field(
                "access_token",
                &self.access_token.as_ref().map(|_| "[redacted]"),
            )
            .field("batch_size", &self.batch_size)
            .field("request_timeout", &self.request_timeout)
            .finish()
    }
}

/// Invalid push adapter configuration.
#[derive(Debug, Error)]
pub enum PushOptionsError {
    /// Shared configuration invariants were violated.
    #[error(transparent)]
    Configuration(#[from] ValidationErrors),
    /// The endpoint is not an absolute HTTP URL ending in `/send`.
    #[error("endpoint must be an absolute HTTP URL ending in /send, got {0:?}")]
    InvalidEndpoint(String),
    /// The access token was set to an empty value.
    #[error("access token must not be empty; omit it instead")]
    EmptyAccessToken,
    /// The batch size is outside the provider's accepted range.
    #[error("batch size must be between 1 and {MAX_BATCH_SIZE}, got {0}")]
    InvalidBatchSize(usize),
    /// The request timeout is zero.
    #[error("request timeout must be greater than zero")]
    ZeroRequestTimeout,
}

fn validated_endpoints(endpoint: &str) -> Result<(String, String), PushOptionsError> {
    let endpoint = endpoint.trim();
    let parsed = reqwest::Url::parse(endpoint)
        .map_err(|_| PushOptionsError::InvalidEndpoint(endpoint.to_owned()))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.path().ends_with("/send")
    {
        return Err(PushOptionsError::InvalidEndpoint(endpoint.to_owned()));
    }
    let receipts = parsed
        .join("getReceipts")
        .map_err(|_| PushOptionsError::InvalidEndpoint(endpoint.to_owned()))?;
    Ok((parsed.to_string(), receipts.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_point_at_the_public_expo_api() {
        let options = PushOptions::default();
        assert_eq!(options.endpoint(), DEFAULT_EXPO_ENDPOINT);
        assert_eq!(
            options.receipts_endpoint(),
            "https://exp.host/--/api/v2/push/getReceipts"
        );
        assert_eq!(options.access_token(), None);
        assert_eq!(options.batch_size(), MAX_BATCH_SIZE);
        assert_eq!(options.request_timeout(), DEFAULT_REQUEST_TIMEOUT);
    }

    #[test]
    fn the_receipts_endpoint_is_derived_next_to_send() -> Result<(), PushOptionsError> {
        let options = PushOptions::new("https://push.example.com/api/push/send")?;
        assert_eq!(options.endpoint(), "https://push.example.com/api/push/send");
        assert_eq!(
            options.receipts_endpoint(),
            "https://push.example.com/api/push/getReceipts"
        );
        Ok(())
    }

    #[test]
    fn invalid_options_are_rejected_at_construction() {
        assert!(matches!(
            PushOptions::new("not a URL"),
            Err(PushOptionsError::InvalidEndpoint(_))
        ));
        assert!(matches!(
            PushOptions::new(""),
            Err(PushOptionsError::InvalidEndpoint(_))
        ));
        assert!(matches!(
            PushOptions::new("https://push.example.com/api/v2"),
            Err(PushOptionsError::InvalidEndpoint(_))
        ));
        let options = PushOptions::default();
        assert!(matches!(
            options.clone().with_batch_size(0),
            Err(PushOptionsError::InvalidBatchSize(0))
        ));
        assert!(matches!(
            options.clone().with_batch_size(MAX_BATCH_SIZE + 1),
            Err(PushOptionsError::InvalidBatchSize(101))
        ));
        assert!(matches!(
            options.clone().with_request_timeout(Duration::ZERO),
            Err(PushOptionsError::ZeroRequestTimeout)
        ));
        assert!(matches!(
            options.with_access_token("   "),
            Err(PushOptionsError::EmptyAccessToken)
        ));
    }

    #[test]
    fn every_invalid_config_field_is_reported_at_once() {
        let config = PushConfig {
            endpoint: String::new(),
            batch_size: 0,
            request_timeout: Duration::ZERO,
            ..PushConfig::default()
        };
        let error = PushOptions::from_config(&config).expect_err("invalid config");
        let rendered = error.to_string();
        assert!(rendered.contains("endpoint"));
        assert!(rendered.contains("batch_size"));
        assert!(rendered.contains("request_timeout_ms"));
    }

    #[test]
    fn a_malformed_endpoint_is_reported_separately_from_an_empty_one() {
        let config = PushConfig {
            endpoint: "not a URL".to_owned(),
            ..PushConfig::default()
        };
        let rendered = PushOptions::from_config(&config)
            .expect_err("invalid config")
            .to_string();
        assert!(rendered.contains("absolute HTTP URL ending in /send"));
    }

    #[test]
    fn an_empty_configured_access_token_sends_no_authorization() -> Result<(), PushOptionsError> {
        assert_eq!(
            PushOptions::from_config(&PushConfig::default())?.access_token(),
            None
        );
        let config = PushConfig {
            access_token: Secret::new("  expo-token  ".to_owned()),
            ..PushConfig::default()
        };
        assert_eq!(
            PushOptions::from_config(&config)?.access_token(),
            Some("expo-token")
        );
        Ok(())
    }

    #[test]
    fn the_access_token_never_appears_in_debug_output() -> Result<(), PushOptionsError> {
        let options = PushOptions::default().with_access_token("super-secret")?;
        let rendered = format!("{options:?}");
        assert!(!rendered.contains("super-secret"));
        assert!(rendered.contains("[redacted]"));
        Ok(())
    }

    #[test]
    fn the_config_section_deserializes_timeouts_in_milliseconds() {
        let config: PushConfig =
            serde_json::from_str(r#"{"batch_size":10,"request_timeout_ms":250}"#)
                .expect("config should deserialize");
        assert_eq!(config.batch_size, 10);
        assert_eq!(config.request_timeout, Duration::from_millis(250));
        assert_eq!(config.endpoint, DEFAULT_EXPO_ENDPOINT);
    }
}
