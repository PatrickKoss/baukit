use thiserror::Error;

/// Failure raised by a credential vault or by its envelope encryption.
#[derive(Debug, Error)]
pub enum CredentialVaultError {
    /// No credential record exists for the requested scope.
    #[error("stored credentials were not found")]
    NotFound,
    /// Authenticated decryption failed, so the ciphertext is unusable.
    ///
    /// The variant deliberately carries no detail. A wrong key, a wrong
    /// associated context, a truncated nonce, and a tampered ciphertext are
    /// indistinguishable to callers and to logs.
    #[error("stored credentials could not be decrypted")]
    DecryptionFailed,
    /// The keyring or the submitted secret material is unusable.
    #[error("credential vault configuration is invalid")]
    InvalidConfiguration,
    /// The backing store rejected or could not complete the operation.
    #[error("credential storage failed: {0}")]
    Storage(String),
}
