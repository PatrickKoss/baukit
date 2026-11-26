//! Per-install secrets that guard moving a device token to another owner.

use std::fmt;

use aws_lc_rs::digest;
use uuid::Uuid;

use crate::PushValidationError;

/// Shortest accepted install secret, in bytes.
pub const MIN_INSTALL_SECRET_LENGTH: usize = 16;
/// Longest accepted install secret, in bytes.
pub const MAX_INSTALL_SECRET_LENGTH: usize = 256;

/// A random secret an app installation generates once and sends with every registration.
///
/// The registry stores only its SHA-256 digest. A token held by one owner moves to another
/// owner only when the registration presents the same secret, so learning a device token
/// alone no longer lets anyone take it over. `Debug` redacts the value.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceInstallSecret(String);

impl DeviceInstallSecret {
    /// Accepts 16 to 256 bytes of visible ASCII.
    ///
    /// # Errors
    ///
    /// Returns [`PushValidationError::InstallSecret`] for any other value.
    pub fn new(secret: impl Into<String>) -> Result<Self, PushValidationError> {
        let secret = secret.into();
        let valid = (MIN_INSTALL_SECRET_LENGTH..=MAX_INSTALL_SECRET_LENGTH).contains(&secret.len())
            && secret.bytes().all(|byte| byte.is_ascii_graphic());
        if valid {
            Ok(Self(secret))
        } else {
            Err(PushValidationError::InstallSecret)
        }
    }

    /// Returns the digest the registry stores and compares.
    #[must_use]
    pub fn digest(&self) -> Vec<u8> {
        digest::digest(&digest::SHA256, self.0.as_bytes())
            .as_ref()
            .to_vec()
    }
}

impl fmt::Debug for DeviceInstallSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DeviceInstallSecret(<redacted>)")
    }
}

/// Whether a registration may take a token that may already be stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TakeoverDecision {
    /// The token is new or already belongs to the registering owner.
    Keep,
    /// The token moves to the registering owner, who presented the install secret.
    Move,
    /// The token moves without proof because its row predates install secrets.
    MoveLegacy,
    /// The token belongs to another owner whose install secret was not presented.
    Refuse,
}

/// Decides a registration against the stored owner and secret digest of its token.
#[must_use]
pub fn takeover_decision(
    stored: Option<(Uuid, Option<&[u8]>)>,
    owner_id: Uuid,
    presented: Option<&[u8]>,
) -> TakeoverDecision {
    match stored {
        None => TakeoverDecision::Keep,
        Some((stored_owner, _)) if stored_owner == owner_id => TakeoverDecision::Keep,
        Some((_, None)) => TakeoverDecision::MoveLegacy,
        Some((_, Some(digest))) if presented == Some(digest) => TakeoverDecision::Move,
        Some(_) => TakeoverDecision::Refuse,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_is_redacted_and_bounded() {
        let secret = DeviceInstallSecret::new("install-secret-0001").expect("valid secret");
        assert!(!format!("{secret:?}").contains("install"));
        for invalid in [
            "short".to_owned(),
            "x".repeat(MAX_INSTALL_SECRET_LENGTH + 1),
            "with space in secret".to_owned(),
        ] {
            assert_eq!(
                DeviceInstallSecret::new(invalid),
                Err(PushValidationError::InstallSecret)
            );
        }
    }

    #[test]
    fn only_the_holder_or_the_install_secret_keeps_or_moves_a_token() {
        let (holder, other) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let digest = [7_u8; 32];
        let wrong = [8_u8; 32];
        assert_eq!(takeover_decision(None, other, None), TakeoverDecision::Keep);
        assert_eq!(
            takeover_decision(Some((holder, Some(&digest))), holder, None),
            TakeoverDecision::Keep
        );
        assert_eq!(
            takeover_decision(Some((holder, None)), other, None),
            TakeoverDecision::MoveLegacy
        );
        assert_eq!(
            takeover_decision(Some((holder, Some(&digest))), other, Some(&digest)),
            TakeoverDecision::Move
        );
        for presented in [None, Some(wrong.as_slice())] {
            assert_eq!(
                takeover_decision(Some((holder, Some(&digest))), other, presented),
                TakeoverDecision::Refuse
            );
        }
    }
}
