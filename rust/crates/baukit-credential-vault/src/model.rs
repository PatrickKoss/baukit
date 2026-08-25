use std::collections::BTreeMap;

use uuid::Uuid;
use zeroize::Zeroize;

use crate::error::CredentialVaultError;

/// Maximum length of a secret field name.
const MAX_FIELD_NAME_LENGTH: usize = 64;

/// A named set of plaintext secrets belonging to one credential scope.
///
/// Field names are product vocabulary: an OAuth product might store
/// `access_token` and `refresh_token`, an API-key product a single `api_key`.
/// The vault never interprets them beyond binding each name into the encrypted
/// field's associated data.
///
/// Every secret value is zeroized when the set is dropped, and neither `Debug`
/// nor `Display` is implemented, so a secret cannot reach a log through
/// formatting.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct CredentialSecrets {
    fields: BTreeMap<String, Vec<u8>>,
}

impl CredentialSecrets {
    /// Creates an empty secret set.
    #[must_use]
    pub fn new() -> Self {
        Self {
            fields: BTreeMap::new(),
        }
    }

    /// Adds or replaces one named secret.
    ///
    /// Returns [`CredentialVaultError::InvalidConfiguration`] when the name is
    /// empty, longer than 64 bytes, contains anything other than ASCII
    /// alphanumerics, `_`, `-`, or `.`, or when the value is empty. Rejecting
    /// empty values keeps "absent" and "present but blank" from being confused
    /// after a round trip.
    pub fn insert(
        &mut self,
        name: impl Into<String>,
        value: impl Into<Vec<u8>>,
    ) -> Result<(), CredentialVaultError> {
        let name = name.into();
        let value = value.into();
        if !is_valid_field_name(&name) || value.is_empty() {
            return Err(CredentialVaultError::InvalidConfiguration);
        }
        if let Some(mut previous) = self.fields.insert(name, value) {
            previous.zeroize();
        }
        Ok(())
    }

    /// Adds one named secret and returns the set, for builder-style composition.
    pub fn with(
        mut self,
        name: impl Into<String>,
        value: impl Into<Vec<u8>>,
    ) -> Result<Self, CredentialVaultError> {
        self.insert(name, value)?;
        Ok(self)
    }

    /// Returns the plaintext of one named secret.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.fields.get(name).map(Vec::as_slice)
    }

    /// Returns the secret field names in sorted order.
    pub fn field_names(&self) -> impl Iterator<Item = &str> {
        self.fields.keys().map(String::as_str)
    }

    /// Returns the number of stored secrets.
    #[must_use]
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// Returns whether the set holds no secrets.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.fields
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_slice()))
    }
}

impl Zeroize for CredentialSecrets {
    fn zeroize(&mut self) {
        for value in self.fields.values_mut() {
            value.zeroize();
        }
        self.fields.clear();
    }
}

impl Drop for CredentialSecrets {
    fn drop(&mut self) {
        self.zeroize();
    }
}

/// One encrypted secret field, ready for durable storage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncryptedField {
    /// AES-256-GCM ciphertext with its authentication tag appended.
    pub ciphertext: Vec<u8>,
    /// The 12-byte nonce used for this field, unique per encryption.
    pub nonce: Vec<u8>,
}

/// The encrypted form of a [`CredentialSecrets`] set for one scope.
///
/// The struct is safe to log or serialize: it holds ciphertext, nonces, and the
/// key version, never plaintext.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncryptedCredentials {
    /// Identifier of the credential scope the ciphertext is bound to.
    pub scope_id: Uuid,
    /// Encrypted fields keyed by their plaintext field name.
    pub fields: BTreeMap<String, EncryptedField>,
    /// Keyring version that encrypted these fields and must decrypt them.
    pub key_version: i32,
}

fn is_valid_field_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_FIELD_NAME_LENGTH
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}
