use std::{collections::BTreeMap, fmt, sync::Arc};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use ring::{
    aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey},
    rand::{SecureRandom as _, SystemRandom},
};
use uuid::Uuid;
use zeroize::Zeroize;

use crate::{
    error::CredentialVaultError,
    model::{CredentialSecrets, EncryptedCredentials, EncryptedField},
};

/// Length in bytes of an AES-256 key.
pub const KEY_LENGTH: usize = 32;

/// Length in bytes of the per-field AES-GCM nonce.
pub const NONCE_LENGTH: usize = NONCE_LEN;

/// A versioned AES-256-GCM keyring that encrypts and decrypts credential fields.
///
/// The highest-numbered version in the ring is the write key; every listed
/// version stays available for reads, which is what makes key rotation a
/// two-step operation rather than a migration. See the crate README for the
/// keyring string format and the rotation procedure.
///
/// Cloning is cheap: clones share the parsed key material.
#[derive(Clone)]
pub struct CredentialCipher {
    active_version: i32,
    keys: Arc<BTreeMap<i32, KeyMaterial>>,
    random: SystemRandom,
}

struct KeyMaterial {
    key: LessSafeKey,
    bytes: [u8; KEY_LENGTH],
}

impl Drop for KeyMaterial {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

impl CredentialCipher {
    /// Parses a keyring from its `version:base64key,...` string form.
    ///
    /// Versions are positive integers and each key decodes to exactly 32 bytes
    /// of standard base64. The highest version becomes the write key. Duplicate
    /// versions, malformed base64, wrong key lengths, and empty rings are all
    /// rejected as [`CredentialVaultError::InvalidConfiguration`].
    pub fn parse(value: &str) -> Result<Self, CredentialVaultError> {
        let mut keys = BTreeMap::new();
        for entry in value
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
        {
            let (version, encoded_key) = entry
                .split_once(':')
                .ok_or(CredentialVaultError::InvalidConfiguration)?;
            let version = version
                .trim()
                .parse::<i32>()
                .map_err(|_| CredentialVaultError::InvalidConfiguration)?;
            if version <= 0 {
                return Err(CredentialVaultError::InvalidConfiguration);
            }
            if keys
                .insert(version, parse_key(encoded_key.trim())?)
                .is_some()
            {
                return Err(CredentialVaultError::InvalidConfiguration);
            }
        }
        let active_version = keys
            .keys()
            .next_back()
            .copied()
            .ok_or(CredentialVaultError::InvalidConfiguration)?;
        Ok(Self {
            active_version,
            keys: Arc::new(keys),
            random: SystemRandom::new(),
        })
    }

    /// Returns the keyring version used for new ciphertext.
    #[must_use]
    pub const fn active_version(&self) -> i32 {
        self.active_version
    }

    /// Returns every version the ring can decrypt, in ascending order.
    pub fn known_versions(&self) -> impl Iterator<Item = i32> {
        self.keys.keys().copied()
    }

    /// Encrypts every secret in the set under the active key version.
    ///
    /// Each field gets a fresh random nonce and associated data binding the
    /// scope id, the key version, and the field name, so ciphertext cannot be
    /// replayed into another scope, another field, or another key version.
    pub fn encrypt(
        &self,
        scope_id: Uuid,
        secrets: &CredentialSecrets,
    ) -> Result<EncryptedCredentials, CredentialVaultError> {
        if secrets.is_empty() {
            return Err(CredentialVaultError::InvalidConfiguration);
        }
        let key = self
            .keys
            .get(&self.active_version)
            .ok_or(CredentialVaultError::InvalidConfiguration)?;
        let mut fields = BTreeMap::new();
        for (name, plaintext) in secrets.iter() {
            let field = self.encrypt_field(key, scope_id, name, plaintext)?;
            fields.insert(name.to_owned(), field);
        }
        Ok(EncryptedCredentials {
            scope_id,
            fields,
            key_version: self.active_version,
        })
    }

    /// Decrypts every field under the key version recorded with the ciphertext.
    ///
    /// An unknown version, a wrong nonce length, a wrong scope, a renamed
    /// field, and tampered bytes all fail as
    /// [`CredentialVaultError::DecryptionFailed`].
    pub fn decrypt(
        &self,
        encrypted: &EncryptedCredentials,
    ) -> Result<CredentialSecrets, CredentialVaultError> {
        if encrypted.fields.is_empty() {
            return Err(CredentialVaultError::DecryptionFailed);
        }
        let key = self
            .keys
            .get(&encrypted.key_version)
            .ok_or(CredentialVaultError::DecryptionFailed)?;
        let mut secrets = CredentialSecrets::new();
        for (name, field) in &encrypted.fields {
            let plaintext =
                decrypt_field(key, encrypted.scope_id, encrypted.key_version, name, field)?;
            secrets
                .insert(name.clone(), plaintext)
                .map_err(|_| CredentialVaultError::DecryptionFailed)?;
        }
        Ok(secrets)
    }

    fn encrypt_field(
        &self,
        key: &KeyMaterial,
        scope_id: Uuid,
        name: &str,
        plaintext: &[u8],
    ) -> Result<EncryptedField, CredentialVaultError> {
        let mut nonce_bytes = [0_u8; NONCE_LENGTH];
        self.random
            .fill(&mut nonce_bytes)
            .map_err(|_| CredentialVaultError::InvalidConfiguration)?;
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);
        let aad = associated_data(scope_id, self.active_version, name);
        let mut buffer = plaintext.to_vec();
        let result = key
            .key
            .seal_in_place_append_tag(nonce, Aad::from(&aad), &mut buffer);
        if result.is_err() {
            buffer.zeroize();
            return Err(CredentialVaultError::InvalidConfiguration);
        }
        Ok(EncryptedField {
            ciphertext: buffer,
            nonce: nonce_bytes.to_vec(),
        })
    }
}

impl fmt::Debug for CredentialCipher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialCipher")
            .field("active_version", &self.active_version)
            .field("versions", &self.keys.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

fn parse_key(encoded: &str) -> Result<KeyMaterial, CredentialVaultError> {
    let mut decoded = STANDARD
        .decode(encoded)
        .map_err(|_| CredentialVaultError::InvalidConfiguration)?;
    if decoded.len() != KEY_LENGTH {
        decoded.zeroize();
        return Err(CredentialVaultError::InvalidConfiguration);
    }
    let mut bytes = [0_u8; KEY_LENGTH];
    bytes.copy_from_slice(&decoded);
    decoded.zeroize();
    let Ok(unbound) = UnboundKey::new(&AES_256_GCM, &bytes) else {
        bytes.zeroize();
        return Err(CredentialVaultError::InvalidConfiguration);
    };
    Ok(KeyMaterial {
        key: LessSafeKey::new(unbound),
        bytes,
    })
}

fn decrypt_field(
    key: &KeyMaterial,
    scope_id: Uuid,
    version: i32,
    name: &str,
    field: &EncryptedField,
) -> Result<Vec<u8>, CredentialVaultError> {
    let nonce_bytes: [u8; NONCE_LENGTH] = field
        .nonce
        .as_slice()
        .try_into()
        .map_err(|_| CredentialVaultError::DecryptionFailed)?;
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);
    let aad = associated_data(scope_id, version, name);
    let mut buffer = field.ciphertext.clone();
    let Ok(plaintext) = key.key.open_in_place(nonce, Aad::from(&aad), &mut buffer) else {
        buffer.zeroize();
        return Err(CredentialVaultError::DecryptionFailed);
    };
    let plaintext_length = plaintext.len();
    buffer[plaintext_length..].zeroize();
    buffer.truncate(plaintext_length);
    if buffer.is_empty() {
        return Err(CredentialVaultError::DecryptionFailed);
    }
    Ok(buffer)
}

fn associated_data(scope_id: Uuid, version: i32, name: &str) -> Vec<u8> {
    let mut value = Vec::with_capacity(16 + 4 + name.len());
    value.extend_from_slice(scope_id.as_bytes());
    value.extend_from_slice(&version.to_be_bytes());
    value.extend_from_slice(name.as_bytes());
    value
}
