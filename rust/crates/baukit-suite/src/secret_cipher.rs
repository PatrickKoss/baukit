use std::collections::BTreeMap;

use crate::domain::EncryptedPayload;
use baukit_credential_vault::{
    CredentialCipher, CredentialSecrets, CredentialVaultError, EncryptedCredentials, EncryptedField,
};
use uuid::Uuid;

#[derive(Clone, Default)]
pub(crate) struct SecretCipher(Option<CredentialCipher>);

impl SecretCipher {
    pub(crate) fn new(cipher: Option<CredentialCipher>) -> Self {
        Self(cipher)
    }

    pub(crate) fn encrypt(
        &self,
        scope_id: Uuid,
        field_name: &str,
        plaintext: &[u8],
    ) -> Result<EncryptedPayload, CredentialVaultError> {
        let cipher = self
            .0
            .as_ref()
            .ok_or(CredentialVaultError::InvalidConfiguration)?;
        let secrets = CredentialSecrets::new().with(field_name, plaintext.to_vec())?;
        let mut encrypted = cipher.encrypt(scope_id, &secrets)?;
        let field = encrypted
            .fields
            .remove(field_name)
            .ok_or(CredentialVaultError::InvalidConfiguration)?;
        Ok(EncryptedPayload {
            ciphertext: field.ciphertext,
            nonce: field.nonce,
            key_version: encrypted.key_version,
        })
    }

    pub(crate) fn decrypt(
        &self,
        scope_id: Uuid,
        field_name: &str,
        encrypted: &EncryptedPayload,
    ) -> Result<Vec<u8>, CredentialVaultError> {
        let cipher = self
            .0
            .as_ref()
            .ok_or(CredentialVaultError::DecryptionFailed)?;
        let encrypted = EncryptedCredentials {
            scope_id,
            fields: BTreeMap::from([(
                field_name.to_owned(),
                EncryptedField {
                    ciphertext: encrypted.ciphertext.clone(),
                    nonce: encrypted.nonce.clone(),
                },
            )]),
            key_version: encrypted.key_version,
        };
        cipher
            .decrypt(&encrypted)?
            .get(field_name)
            .map(<[u8]>::to_vec)
            .ok_or(CredentialVaultError::DecryptionFailed)
    }
}
