//! The device registry port: which push tokens belong to which owner.

use std::{fmt, future::Future, num::NonZeroU32, pin::Pin};

use chrono::{DateTime, Utc};
use thiserror::Error;
use uuid::Uuid;

use crate::PushOutcome;

/// Longest device token the registry accepts, in bytes.
pub const MAX_DEVICE_TOKEN_LENGTH: usize = 512;

/// Longest device time zone name the registry accepts, in bytes.
pub const MAX_DEVICE_TIME_ZONE_LENGTH: usize = 64;

/// Devices an owner keeps when a store is built without an explicit cap.
pub const DEFAULT_DEVICES_PER_OWNER: NonZeroU32 = NonZeroU32::new(10).expect("ten is not zero");

/// A value rejected before it reached a store.
///
/// The display text names the field only. It never repeats the rejected value,
/// so the error is safe to log or return to a client.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum PushValidationError {
    /// The device token is empty, too long, or not printable ASCII.
    #[error("device token is invalid")]
    DeviceToken,
    /// The time zone name is empty, too long, or has characters outside an IANA name.
    #[error("device time zone is invalid")]
    DeviceTimeZone,
    /// The delivery kind is empty, too long, or has characters outside `[a-z0-9_.-]`.
    #[error("delivery kind is invalid")]
    DeliveryKind,
}

/// A provider-issued device token.
///
/// A token addresses one app installation, so anyone holding it can send that
/// device a notification. `Debug` redacts it and there is no `Display`; read
/// it with [`DeviceToken::expose`] only where it leaves for the provider or the
/// store.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DeviceToken(String);

impl DeviceToken {
    /// Validates a token: 1 to [`MAX_DEVICE_TOKEN_LENGTH`] bytes of visible ASCII.
    ///
    /// Trim the value before calling this; surrounding whitespace is rejected
    /// rather than silently removed.
    ///
    /// # Errors
    ///
    /// Returns [`PushValidationError::DeviceToken`] for any other value.
    pub fn new(token: impl Into<String>) -> Result<Self, PushValidationError> {
        let token = token.into();
        let valid = !token.is_empty()
            && token.len() <= MAX_DEVICE_TOKEN_LENGTH
            && token.bytes().all(|byte| byte.is_ascii_graphic());
        if valid {
            Ok(Self(token))
        } else {
            Err(PushValidationError::DeviceToken)
        }
    }

    /// Returns the token for a provider request or a store query.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for DeviceToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DeviceToken(<redacted>)")
    }
}

/// The time zone a device reported when it registered, as an IANA name.
///
/// The registry checks the shape only. Resolve the name with a time zone
/// database before registering if the product schedules by it.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DeviceTimeZone(String);

impl DeviceTimeZone {
    /// Validates the shape of an IANA name such as `Europe/Berlin` or `Etc/GMT+5`.
    ///
    /// # Errors
    ///
    /// Returns [`PushValidationError::DeviceTimeZone`] when the name is empty,
    /// longer than [`MAX_DEVICE_TIME_ZONE_LENGTH`], or has a character outside
    /// ASCII letters, digits, `/`, `_`, `+`, and `-`.
    pub fn new(name: impl Into<String>) -> Result<Self, PushValidationError> {
        let name = name.into();
        let valid = !name.is_empty()
            && name.len() <= MAX_DEVICE_TIME_ZONE_LENGTH
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"/_+-".contains(&byte));
        if valid {
            Ok(Self(name))
        } else {
            Err(PushValidationError::DeviceTimeZone)
        }
    }

    /// Returns the IANA name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The mobile platform a device token belongs to.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DevicePlatform {
    /// Apple iOS or iPadOS.
    Ios,
    /// Google Android.
    Android,
}

impl DevicePlatform {
    /// Returns the stored form, `ios` or `android`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ios => "ios",
            Self::Android => "android",
        }
    }

    /// Parses the stored form.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ios" => Some(Self::Ios),
            "android" => Some(Self::Android),
            _ => None,
        }
    }
}

/// One device's request to receive pushes for an owner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceRegistration {
    /// The account the device receives notifications for.
    pub owner_id: Uuid,
    /// The device's current provider token.
    pub token: DeviceToken,
    /// The platform the token belongs to.
    pub platform: DevicePlatform,
    /// The time zone the device reported, if the product schedules by it.
    pub time_zone: Option<DeviceTimeZone>,
    /// The service instant of this registration.
    pub registered_at: DateTime<Utc>,
}

impl DeviceRegistration {
    /// Creates a registration without a time zone.
    #[must_use]
    pub const fn new(
        owner_id: Uuid,
        token: DeviceToken,
        platform: DevicePlatform,
        registered_at: DateTime<Utc>,
    ) -> Self {
        Self {
            owner_id,
            token,
            platform,
            time_zone: None,
            registered_at,
        }
    }

    /// Records the time zone the device reported.
    #[must_use]
    pub fn with_time_zone(mut self, time_zone: DeviceTimeZone) -> Self {
        self.time_zone = Some(time_zone);
        self
    }
}

/// A device as the registry stores it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisteredDevice {
    /// The account the device receives notifications for.
    pub owner_id: Uuid,
    /// The device's provider token.
    pub token: DeviceToken,
    /// The platform the token belongs to.
    pub platform: DevicePlatform,
    /// The time zone from the latest registration.
    pub time_zone: Option<DeviceTimeZone>,
    /// When this owner first registered the token.
    pub created_at: DateTime<Utc>,
    /// The latest registration instant; eviction and invalidation compare it.
    pub last_registered_at: DateTime<Utc>,
}

/// What a registration or rotation changed besides the registered device.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RegistrationOutcome {
    /// Older devices of the same owner removed to stay within the cap.
    pub evicted: u64,
}

/// A store failure.
///
/// The display text is fixed. The private diagnostics come from the database
/// driver's message, which carries no bound values, so neither form contains a
/// device token.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum PushStoreError {
    /// The adapter could not complete the operation.
    #[error("push store failed")]
    Internal(String),
}

impl PushStoreError {
    /// Wraps private adapter diagnostics without adding them to display text.
    pub fn internal(error: impl fmt::Display) -> Self {
        Self::Internal(error.to_string())
    }
}

/// The future returned by [`DeviceRegistry`] and [`DeliveryClaimStore`](crate::DeliveryClaimStore) methods.
pub type PushStoreFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Storage port for the push tokens each owner's devices registered.
///
/// A token belongs to at most one owner. Registering a token another owner
/// holds moves it, so a shared device stops receiving the previous account's
/// notifications. Each owner keeps at most a configured number of devices;
/// a registration over the cap evicts that owner's least recently registered
/// devices, never the one being registered. Ties break on `created_at`, then
/// on the token, so eviction is deterministic.
pub trait DeviceRegistry: Send + Sync {
    /// Registers or refreshes one device and evicts over the owner's cap.
    fn register(
        &self,
        registration: DeviceRegistration,
    ) -> PushStoreFuture<'_, Result<RegistrationOutcome, PushStoreError>>;

    /// Replaces the owner's `previous` token with the registration's token in one step.
    ///
    /// The predecessor is removed before the cap is enforced, so rotating
    /// never evicts another device. A predecessor the owner does not hold is
    /// left alone, which makes a retried rotation safe.
    fn rotate(
        &self,
        previous: DeviceToken,
        registration: DeviceRegistration,
    ) -> PushStoreFuture<'_, Result<RegistrationOutcome, PushStoreError>>;

    /// Removes one of the owner's devices; returns whether it existed.
    fn unregister(
        &self,
        owner_id: Uuid,
        token: DeviceToken,
    ) -> PushStoreFuture<'_, Result<bool, PushStoreError>>;

    /// Lists the owner's devices, most recently registered first.
    fn list_for_owner(
        &self,
        owner_id: Uuid,
    ) -> PushStoreFuture<'_, Result<Vec<RegisteredDevice>, PushStoreError>>;

    /// Removes tokens the provider reported dead; returns how many went.
    ///
    /// Pass the instant the send started as `sent_at`. A device registered
    /// again after it keeps its token, because the provider's verdict predates
    /// the new registration.
    fn invalidate(
        &self,
        tokens: Vec<DeviceToken>,
        sent_at: DateTime<Utc>,
    ) -> PushStoreFuture<'_, Result<u64, PushStoreError>>;

    /// Removes every device of one owner; returns how many went.
    fn erase_owner(&self, owner_id: Uuid) -> PushStoreFuture<'_, Result<u64, PushStoreError>>;

    /// Invalidates every token a send reported as [`PushOutcome::is_token_dead`].
    ///
    /// This is the one call to make after [`PushSender::send`](crate::PushSender::send)
    /// returns. Outcomes without a dead token cost no store round trip.
    fn invalidate_dead_tokens(
        &self,
        outcomes: &[PushOutcome],
        sent_at: DateTime<Utc>,
    ) -> PushStoreFuture<'_, Result<u64, PushStoreError>> {
        let tokens = dead_tokens(outcomes);
        if tokens.is_empty() {
            return Box::pin(std::future::ready(Ok(0)));
        }
        self.invalidate(tokens, sent_at)
    }
}

/// Returns the distinct tokens a send reported as dead.
///
/// A dead outcome whose token would not pass [`DeviceToken::new`] is skipped;
/// no registry can hold it.
#[must_use]
pub fn dead_tokens(outcomes: &[PushOutcome]) -> Vec<DeviceToken> {
    let mut tokens = outcomes
        .iter()
        .filter(|outcome| outcome.is_token_dead())
        .filter_map(|outcome| DeviceToken::new(outcome.token.as_str()).ok())
        .collect::<Vec<_>>();
    tokens.sort();
    tokens.dedup();
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PushDeliveryStatus, PushRejection};

    const EXPO_TOKEN: &str = "ExponentPushToken[xxxxxxxxxxxxxxxxxxxxxx]";

    #[test]
    fn a_token_accepts_visible_ascii_up_to_the_limit() {
        assert!(DeviceToken::new(EXPO_TOKEN).is_ok());
        assert!(DeviceToken::new("a".repeat(MAX_DEVICE_TOKEN_LENGTH)).is_ok());
        for invalid in [
            String::new(),
            "a".repeat(MAX_DEVICE_TOKEN_LENGTH + 1),
            " leading".to_owned(),
            "inner space".to_owned(),
            "tab\t".to_owned(),
            "umlaut-ü".to_owned(),
        ] {
            assert_eq!(
                DeviceToken::new(invalid),
                Err(PushValidationError::DeviceToken)
            );
        }
    }

    #[test]
    fn a_token_never_appears_in_debug_output_or_errors() {
        let token = DeviceToken::new(EXPO_TOKEN).expect("valid token");
        let registration = DeviceRegistration::new(
            Uuid::nil(),
            token.clone(),
            DevicePlatform::Ios,
            DateTime::UNIX_EPOCH,
        );
        assert!(!format!("{token:?}").contains("Exponent"));
        assert!(!format!("{registration:?}").contains("Exponent"));
        assert_eq!(token.expose(), EXPO_TOKEN);

        let error = DeviceToken::new(format!("{EXPO_TOKEN} ")).expect_err("space is invalid");
        assert!(!format!("{error} {error:?}").contains("Exponent"));
    }

    #[test]
    fn a_time_zone_accepts_iana_shaped_names_only() {
        for valid in [
            "UTC",
            "Europe/Berlin",
            "America/Argentina/Buenos_Aires",
            "Etc/GMT+5",
        ] {
            assert_eq!(
                DeviceTimeZone::new(valid).map(|zone| zone.as_str().to_owned()),
                Ok(valid.to_owned())
            );
        }
        for invalid in [
            String::new(),
            "Europe/Berlin ".to_owned(),
            "x".repeat(MAX_DEVICE_TIME_ZONE_LENGTH + 1),
            "Europe;Berlin".to_owned(),
        ] {
            assert_eq!(
                DeviceTimeZone::new(invalid),
                Err(PushValidationError::DeviceTimeZone)
            );
        }
    }

    #[test]
    fn platforms_round_trip_through_their_stored_form() {
        for platform in [DevicePlatform::Ios, DevicePlatform::Android] {
            assert_eq!(DevicePlatform::parse(platform.as_str()), Some(platform));
        }
        assert_eq!(DevicePlatform::parse("web"), None);
    }

    #[test]
    fn only_distinct_valid_dead_tokens_are_invalidated() {
        let outcome = |token: &str, status| PushOutcome {
            token: token.to_owned(),
            status,
        };
        let dead = || PushDeliveryStatus::Rejected(PushRejection::DeviceNotRegistered);
        let tokens = dead_tokens(&[
            outcome("b", dead()),
            outcome("a", dead()),
            outcome("b", dead()),
            outcome("not valid", dead()),
            outcome("c", PushDeliveryStatus::Delivered),
            outcome("d", PushDeliveryStatus::Accepted),
            outcome(
                "e",
                PushDeliveryStatus::Rejected(PushRejection::MessageTooBig),
            ),
        ]);
        assert_eq!(
            tokens.iter().map(DeviceToken::expose).collect::<Vec<_>>(),
            ["a", "b"]
        );
    }
}
