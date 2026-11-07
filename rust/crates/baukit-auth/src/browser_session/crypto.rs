use aws_lc_rs::{
    digest,
    rand::{SecureRandom as _, SystemRandom},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use baukit_credential_vault::{
    CredentialCipher, CredentialSecrets, CredentialVaultError, EncryptedCredentials,
    EncryptedField, NONCE_LENGTH,
};
use std::collections::BTreeMap;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::secret::constant_time_eq;

const SECRET_BYTES: usize = 32;
const ENCODED_SECRET_LENGTH: usize = 43;
const AAD_DOMAIN: &[u8] = b"baukit-auth/browser-session/v1";

pub(crate) const REFRESH_TOKEN_FIELD: &str = "refresh_token";
pub(crate) const ID_TOKEN_FIELD: &str = "id_token";
pub(crate) const GRACE_ACCESS_TOKEN_FIELD: &str = "grace_access_token";

/// The cookie value: a random 128-bit row id and a 256-bit secret.
pub(crate) struct SessionCredential {
    pub(crate) id: Uuid,
    pub(crate) secret: Zeroizing<String>,
}

impl SessionCredential {
    pub(crate) fn generate(random: &SystemRandom) -> Result<Self, CryptoError> {
        let mut id = [0_u8; 16];
        random.fill(&mut id).map_err(|_| CryptoError::Random)?;
        let mut secret = Zeroizing::new([0_u8; SECRET_BYTES]);
        random
            .fill(secret.as_mut())
            .map_err(|_| CryptoError::Random)?;
        Ok(Self {
            id: Uuid::from_bytes(id),
            secret: Zeroizing::new(URL_SAFE_NO_PAD.encode(secret.as_ref())),
        })
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        let (id, secret) = value.split_once('.')?;
        if id.len() != 32 || secret.len() != ENCODED_SECRET_LENGTH {
            return None;
        }
        let id = Uuid::try_parse(id).ok()?;
        URL_SAFE_NO_PAD.decode(secret).ok()?;
        Some(Self {
            id,
            secret: Zeroizing::new(secret.to_owned()),
        })
    }

    pub(crate) fn encode(&self) -> Zeroizing<String> {
        Zeroizing::new(format!("{}.{}", self.id.simple(), self.secret.as_str()))
    }

    pub(crate) fn secret_hash(&self) -> Vec<u8> {
        digest::digest(&digest::SHA256, self.secret.as_bytes())
            .as_ref()
            .to_vec()
    }

    pub(crate) fn matches(&self, stored_hash: &[u8]) -> bool {
        constant_time_eq(&self.secret_hash(), stored_hash)
    }
}

/// Binds every ciphertext of a row to its id and subject.
pub(crate) fn aad_scope(row_id: Uuid, subject: &str) -> Uuid {
    let mut context = digest::Context::new(&digest::SHA256);
    context.update(AAD_DOMAIN);
    context.update(row_id.as_bytes());
    context.update(subject.as_bytes());
    let digest = context.finish();
    let mut scope = [0_u8; 16];
    for (target, source) in scope.iter_mut().zip(digest.as_ref()) {
        *target = *source;
    }
    Uuid::from_bytes(scope)
}

/// Secrets of one row, sealed under one key version.
pub(crate) struct SealedRow {
    pub(crate) key_version: i32,
    pub(crate) fields: BTreeMap<&'static str, Vec<u8>>,
}

pub(crate) fn seal(
    cipher: &CredentialCipher,
    row_id: Uuid,
    subject: &str,
    fields: &[(&'static str, &str)],
) -> Result<SealedRow, CryptoError> {
    let mut secrets = CredentialSecrets::new();
    for (name, value) in fields {
        secrets
            .insert(*name, value.as_bytes().to_vec())
            .map_err(CryptoError::Vault)?;
    }
    let encrypted = cipher
        .encrypt(aad_scope(row_id, subject), &secrets)
        .map_err(CryptoError::Vault)?;
    let mut sealed = BTreeMap::new();
    for (name, _) in fields {
        let field = encrypted.fields.get(*name).ok_or(CryptoError::Vault(
            CredentialVaultError::InvalidConfiguration,
        ))?;
        let mut packed = field.nonce.clone();
        packed.extend_from_slice(&field.ciphertext);
        sealed.insert(*name, packed);
    }
    Ok(SealedRow {
        key_version: encrypted.key_version,
        fields: sealed,
    })
}

pub(crate) fn open(
    cipher: &CredentialCipher,
    row_id: Uuid,
    subject: &str,
    key_version: i32,
    name: &'static str,
    packed: &[u8],
) -> Result<Zeroizing<String>, CryptoError> {
    let (nonce, ciphertext) = packed
        .split_at_checked(NONCE_LENGTH)
        .ok_or(CryptoError::Vault(CredentialVaultError::DecryptionFailed))?;
    let encrypted = EncryptedCredentials {
        scope_id: aad_scope(row_id, subject),
        fields: BTreeMap::from([(
            name.to_owned(),
            EncryptedField {
                ciphertext: ciphertext.to_vec(),
                nonce: nonce.to_vec(),
            },
        )]),
        key_version,
    };
    let secrets = cipher.decrypt(&encrypted).map_err(CryptoError::Vault)?;
    let value = secrets
        .get(name)
        .ok_or(CryptoError::Vault(CredentialVaultError::DecryptionFailed))?;
    let text = std::str::from_utf8(value)
        .map_err(|_| CryptoError::Vault(CredentialVaultError::DecryptionFailed))?;
    Ok(Zeroizing::new(text.to_owned()))
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CryptoError {
    #[error("system random source failed")]
    Random,
    #[error("sealed row lacks the {0} field")]
    MissingField(&'static str),
    #[error("credential vault failed: {0}")]
    Vault(#[source] CredentialVaultError),
}
