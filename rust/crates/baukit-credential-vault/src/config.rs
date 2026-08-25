use baukit_config::{Secret, Validate, ValidationError, ValidationErrors};
use serde::Deserialize;

use crate::{cipher::CredentialCipher, error::CredentialVaultError};

/// Keyring configuration for a product's credential vault section.
///
/// Nest the struct in the product configuration that
/// [`baukit_config::BaukitConfig`] flattens, so the environment override lands
/// under the application prefix:
///
/// ```text
/// ORDERS__CREDENTIAL_VAULT__KEYRING=2:<base64 32 bytes>,1:<base64 32 bytes>
/// ```
///
/// The keyring is a [`Secret`], so neither `Debug` nor `Display` reveals key
/// material and the value is zeroized on drop.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct CredentialVaultConfig {
    /// Comma-separated `version:base64key` entries, highest version writes.
    ///
    /// Leave it empty in deployments that store no third-party credentials;
    /// [`CredentialVaultConfig::is_enabled`] then reports the vault as off and
    /// [`CredentialVaultConfig::cipher`] returns `None`.
    pub keyring: Secret<String>,
}

impl Default for CredentialVaultConfig {
    fn default() -> Self {
        Self {
            keyring: Secret::new(String::new()),
        }
    }
}

impl CredentialVaultConfig {
    /// Returns whether a keyring was supplied.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        !self.keyring.expose().trim().is_empty()
    }

    /// Builds the cipher, or `None` when no keyring is configured.
    pub fn cipher(&self) -> Result<Option<CredentialCipher>, CredentialVaultError> {
        if !self.is_enabled() {
            return Ok(None);
        }
        CredentialCipher::parse(self.keyring.expose()).map(Some)
    }
}

impl Validate for CredentialVaultConfig {
    fn validate(&self) -> Result<(), ValidationErrors> {
        if self.is_enabled() && self.cipher().is_err() {
            return Err(ValidationErrors::new(vec![ValidationError::new(
                "keyring",
                "must be comma-separated version:base64key entries with distinct \
                 positive versions and 32-byte keys",
            )]));
        }
        Ok(())
    }
}
