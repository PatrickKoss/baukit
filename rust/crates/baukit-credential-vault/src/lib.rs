//! Envelope encryption for credentials a service holds on someone's behalf.
//!
//! Products that connect to third-party providers end up storing access
//! tokens, refresh tokens, and API keys. This crate owns the part of that job
//! which is identical everywhere: a versioned AES-256-GCM keyring read from
//! configuration, per-field encryption bound to the credential scope, and key
//! rotation without a data migration. The storage adapter stays product-local
//! behind the [`CredentialVault`] port, because the table shape and the
//! ownership rules are product decisions.
//!
//! # Encryption model
//!
//! Each secret field is sealed separately with its own random 12-byte nonce.
//! The associated data binds the scope id, the key version, and the field name,
//! so a ciphertext lifted from one row cannot be replayed into another scope,
//! swapped between fields, or replayed under a different key version. All of
//! those failures surface as the same
//! [`CredentialVaultError::DecryptionFailed`], which carries no detail worth
//! leaking into a log.
//!
//! Plaintext lives in [`CredentialSecrets`], which zeroizes every value on drop
//! and implements neither `Debug` nor `Display`.
//!
//! # Example
//!
//! ```
//! use baukit_credential_vault::{CredentialCipher, CredentialSecrets};
//! use uuid::Uuid;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let keyring = format!("1:{}", base64_key());
//! let cipher = CredentialCipher::parse(&keyring)?;
//!
//! let scope_id = Uuid::now_v7();
//! let secrets = CredentialSecrets::new().with("access_token", b"provider-token".to_vec())?;
//!
//! let encrypted = cipher.encrypt(scope_id, &secrets)?;
//! let restored = cipher.decrypt(&encrypted)?;
//!
//! assert_eq!(restored.get("access_token"), Some(b"provider-token".as_slice()));
//! # Ok(())
//! # }
//! # fn base64_key() -> String {
//! #     use base64::{Engine as _, engine::general_purpose::STANDARD};
//! #     STANDARD.encode([7_u8; 32])
//! # }
//! ```
//!
//! # Configuration and rotation
//!
//! [`CredentialVaultConfig`] reads the keyring from
//! `<APP>__CREDENTIAL_VAULT__KEYRING`. The README documents the string format
//! and the rotation procedure.

#![deny(missing_docs)]

mod cipher;
mod config;
mod error;
mod model;
mod port;

pub use cipher::{CredentialCipher, KEY_LENGTH, NONCE_LENGTH};
pub use config::CredentialVaultConfig;
pub use error::CredentialVaultError;
pub use model::{CredentialSecrets, EncryptedCredentials, EncryptedField};
pub use port::{CredentialVault, VaultFuture};

#[cfg(test)]
mod tests;
