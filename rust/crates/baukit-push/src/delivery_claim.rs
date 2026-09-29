//! At-most-once daily delivery claims for scheduled pushes.

use std::num::NonZeroU32;

use chrono::{DateTime, NaiveDate, Utc};
use uuid::Uuid;

use crate::{PushStoreError, PushStoreFuture, PushValidationError};

/// Longest delivery kind accepted, in bytes.
pub const MAX_DELIVERY_KIND_LENGTH: usize = 64;

/// A product-defined name for one kind of scheduled notification.
///
/// 1 to [`MAX_DELIVERY_KIND_LENGTH`] bytes of `[a-z0-9_.-]`, starting with a
/// letter or digit, for example `training_reminder`.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DeliveryKind(String);

impl DeliveryKind {
    /// Validates a kind name.
    ///
    /// # Errors
    ///
    /// Returns [`PushValidationError::DeliveryKind`] for any name outside the rule.
    pub fn new(kind: impl Into<String>) -> Result<Self, PushValidationError> {
        let kind = kind.into();
        let starts_well = kind
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit());
        let valid = starts_well
            && kind.len() <= MAX_DELIVERY_KIND_LENGTH
            && kind.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_.-".contains(&byte)
            });
        if valid {
            Ok(Self(kind))
        } else {
            Err(PushValidationError::DeliveryKind)
        }
    }

    /// Returns the kind name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The right to send one kind of notification to one owner on one local date.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct DeliveryClaim {
    /// The account the notification is for.
    pub owner_id: Uuid,
    /// The date in the owner's time zone the notification belongs to.
    pub local_date: NaiveDate,
    /// The kind of notification.
    pub kind: DeliveryKind,
}

impl DeliveryClaim {
    /// Creates a claim key.
    #[must_use]
    pub const fn new(owner_id: Uuid, local_date: NaiveDate, kind: DeliveryKind) -> Self {
        Self {
            owner_id,
            local_date,
            kind,
        }
    }
}

/// Storage port for daily delivery claims.
///
/// A scheduled sender claims before it sends, so two workers evaluating the
/// same owner never both deliver. If the whole send fails, it releases the
/// claim and a later run may try again. Event-driven senders do not need this
/// port. Claims grow by one row per owner, kind, and day, so schedule
/// [`DeliveryClaimStore::purge`].
pub trait DeliveryClaimStore: Send + Sync {
    /// Records the claim; returns `false` when someone already holds it.
    fn claim(
        &self,
        claim: DeliveryClaim,
        claimed_at: DateTime<Utc>,
    ) -> PushStoreFuture<'_, Result<bool, PushStoreError>>;

    /// Drops a claim after a failed send; returns whether it existed.
    fn release(&self, claim: DeliveryClaim) -> PushStoreFuture<'_, Result<bool, PushStoreError>>;

    /// Deletes up to `limit` claims for local dates before `before`, oldest
    /// date first; returns how many went.
    ///
    /// Call it again until it returns less than `limit`.
    fn purge(
        &self,
        before: NaiveDate,
        limit: NonZeroU32,
    ) -> PushStoreFuture<'_, Result<u64, PushStoreError>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kind_is_a_short_lowercase_identifier() {
        for valid in ["training_reminder", "weekly-summary", "daily.v2", "7day"] {
            assert!(DeliveryKind::new(valid).is_ok(), "{valid}");
        }
        assert!(DeliveryKind::new("a".repeat(MAX_DELIVERY_KIND_LENGTH)).is_ok());
        for invalid in [
            String::new(),
            "_leading".to_owned(),
            "Upper".to_owned(),
            "with space".to_owned(),
            "a".repeat(MAX_DELIVERY_KIND_LENGTH + 1),
        ] {
            assert_eq!(
                DeliveryKind::new(invalid),
                Err(PushValidationError::DeliveryKind)
            );
        }
    }
}
