//! The provider-neutral push delivery port.

use std::{collections::BTreeMap, pin::Pin, time::Duration};

use thiserror::Error;

/// One notification addressed to a single device token.
///
/// `data` travels alongside the visible notification and reaches the app when
/// the user opens it. Keys are ordered so the serialized payload is stable,
/// which keeps request bodies comparable in tests and logs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PushMessage {
    /// Provider-issued device token the notification is delivered to.
    pub token: String,
    /// Notification title shown on the device.
    pub title: String,
    /// Notification body shown on the device.
    pub body: String,
    /// Optional key-value payload delivered with the notification.
    pub data: BTreeMap<String, String>,
    /// Provider channel the notification belongs to, when the platform has channels.
    ///
    /// Android reads this to pick the notification channel. Leave it `None` for
    /// the provider default.
    pub channel_id: Option<String>,
}

impl PushMessage {
    /// Creates a message for a token with a title and body and no extra data.
    pub fn new(
        token: impl Into<String>,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            token: token.into(),
            title: title.into(),
            body: body.into(),
            data: BTreeMap::new(),
            channel_id: None,
        }
    }

    /// Adds one key-value pair to the notification payload.
    #[must_use]
    pub fn with_data(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.data.insert(key.into(), value.into());
        self
    }

    /// Sets the provider channel the notification belongs to.
    #[must_use]
    pub fn with_channel_id(mut self, channel_id: impl Into<String>) -> Self {
        self.channel_id = Some(channel_id.into());
        self
    }
}

/// Why a provider rejected one notification.
///
/// The variants cover the vocabulary push providers actually return. Anything
/// outside it becomes [`PushRejection::Other`] with the provider's own string,
/// so a new provider code is visible in logs instead of being silently dropped.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PushRejection {
    /// The token no longer belongs to an installed app.
    ///
    /// This is permanent. Delete the token; see [`PushOutcome::is_token_dead`].
    DeviceNotRegistered,
    /// The serialized notification exceeded the provider's size limit.
    MessageTooBig,
    /// Too many notifications were sent to this token in too short a window.
    MessageRateExceeded,
    /// The provider credential was missing, malformed, or rejected.
    InvalidCredentials,
    /// The upstream platform (APNs, FCM) refused the notification.
    ProviderError,
    /// A provider code outside the known vocabulary.
    Other(String),
}

impl PushRejection {
    /// Parses a provider error code into a rejection.
    #[must_use]
    pub fn from_code(code: &str) -> Self {
        match code {
            "DeviceNotRegistered" => Self::DeviceNotRegistered,
            "MessageTooBig" => Self::MessageTooBig,
            "MessageRateExceeded" => Self::MessageRateExceeded,
            "InvalidCredentials" => Self::InvalidCredentials,
            "MismatchSenderId" | "ProviderError" => Self::ProviderError,
            other => Self::Other(other.to_owned()),
        }
    }

    /// Returns whether resending the same notification can succeed.
    ///
    /// Only [`PushRejection::MessageRateExceeded`] and
    /// [`PushRejection::ProviderError`] are worth another attempt. A dead
    /// token, an oversized payload, and a rejected credential all fail the same
    /// way again.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(self, Self::MessageRateExceeded | Self::ProviderError)
    }
}

/// What happened to one notification, keyed by the token it was addressed to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PushOutcome {
    /// The device token the notification was addressed to.
    pub token: String,
    /// The delivery state the provider reported.
    pub status: PushDeliveryStatus,
}

impl PushOutcome {
    /// Returns whether the token should be deleted from the caller's store.
    ///
    /// True only for [`PushRejection::DeviceNotRegistered`]. Every other
    /// rejection describes the notification, not the token.
    #[must_use]
    pub const fn is_token_dead(&self) -> bool {
        matches!(
            &self.status,
            PushDeliveryStatus::Rejected(PushRejection::DeviceNotRegistered)
        )
    }
}

/// The delivery state a provider reported for one notification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PushDeliveryStatus {
    /// The provider took the notification but has not reported an outcome yet.
    ///
    /// Providers confirm delivery asynchronously. Treat this as neither success
    /// nor failure and do not resend on it.
    Accepted,
    /// The provider handed the notification to the device platform.
    Delivered,
    /// The provider refused the notification.
    Rejected(PushRejection),
}

/// A failure that prevented the whole batch from being processed.
///
/// Per-notification refusals are not errors; they arrive as
/// [`PushDeliveryStatus::Rejected`] inside a successful result.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum PushError {
    /// The request never reached the provider, or the provider is failing.
    #[error("push transport failed: {class:?}")]
    Transport {
        /// How the caller should react, including any provider-requested delay.
        class: baukit_http::RetryClass,
    },
    /// The provider answered with a body that does not match its contract.
    #[error("push provider returned an invalid response: {0}")]
    InvalidResponse(String),
}

impl PushError {
    /// Returns whether sending the same batch again can succeed.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        match self {
            Self::Transport { class } => class.is_retryable(),
            Self::InvalidResponse(_) => false,
        }
    }

    /// Returns the wait the provider asked for, if it named one.
    #[must_use]
    pub const fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::Transport { class } => class.retry_after(),
            Self::InvalidResponse(_) => None,
        }
    }
}

/// The future returned by [`PushSender::send`].
pub type PushFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<PushOutcome>, PushError>> + Send + 'a>>;

/// Outbound port for delivering notifications to devices.
///
/// Implementations split a batch into provider-sized chunks themselves. The
/// returned outcomes cover every message in the batch, in no guaranteed order;
/// match them to messages by token.
pub trait PushSender: Send + Sync {
    /// Delivers a batch of notifications and reports the outcome of each.
    fn send<'a>(&'a self, batch: Vec<PushMessage>) -> PushFuture<'a>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_provider_codes_map_onto_the_neutral_vocabulary() {
        assert_eq!(
            PushRejection::from_code("DeviceNotRegistered"),
            PushRejection::DeviceNotRegistered
        );
        assert_eq!(
            PushRejection::from_code("MessageTooBig"),
            PushRejection::MessageTooBig
        );
        assert_eq!(
            PushRejection::from_code("MessageRateExceeded"),
            PushRejection::MessageRateExceeded
        );
        assert_eq!(
            PushRejection::from_code("InvalidCredentials"),
            PushRejection::InvalidCredentials
        );
        assert_eq!(
            PushRejection::from_code("MismatchSenderId"),
            PushRejection::ProviderError
        );
    }

    #[test]
    fn an_unknown_code_is_preserved_verbatim() {
        assert_eq!(
            PushRejection::from_code("SomeNewExpoCode"),
            PushRejection::Other("SomeNewExpoCode".to_owned())
        );
    }

    #[test]
    fn only_rate_limits_and_provider_faults_are_worth_resending() {
        assert!(PushRejection::MessageRateExceeded.is_retryable());
        assert!(PushRejection::ProviderError.is_retryable());
        assert!(!PushRejection::DeviceNotRegistered.is_retryable());
        assert!(!PushRejection::MessageTooBig.is_retryable());
        assert!(!PushRejection::InvalidCredentials.is_retryable());
        assert!(!PushRejection::Other("x".to_owned()).is_retryable());
    }

    #[test]
    fn only_an_unregistered_device_marks_a_token_dead() {
        let dead = PushOutcome {
            token: "t".to_owned(),
            status: PushDeliveryStatus::Rejected(PushRejection::DeviceNotRegistered),
        };
        assert!(dead.is_token_dead());
        for status in [
            PushDeliveryStatus::Accepted,
            PushDeliveryStatus::Delivered,
            PushDeliveryStatus::Rejected(PushRejection::MessageTooBig),
        ] {
            let outcome = PushOutcome {
                token: "t".to_owned(),
                status,
            };
            assert!(!outcome.is_token_dead());
        }
    }

    #[test]
    fn transport_errors_carry_the_providers_retry_delay() {
        let error = PushError::Transport {
            class: baukit_http::RetryClass::RetryAfter(Duration::from_secs(30)),
        };
        assert!(error.is_retryable());
        assert_eq!(error.retry_after(), Some(Duration::from_secs(30)));

        let invalid = PushError::InvalidResponse("no data field".to_owned());
        assert!(!invalid.is_retryable());
        assert_eq!(invalid.retry_after(), None);
    }

    #[test]
    fn a_message_builds_with_data_and_a_channel() {
        let message = PushMessage::new("token", "Title", "Body")
            .with_data("session_id", "abc")
            .with_channel_id("reminders");
        assert_eq!(message.data["session_id"], "abc");
        assert_eq!(message.channel_id.as_deref(), Some("reminders"));
    }
}
