use std::{future::Future, pin::Pin};

use uuid::Uuid;

use crate::{error::CredentialVaultError, model::CredentialSecrets};

/// Boxed future returned by [`CredentialVault`] operations.
pub type VaultFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Storage-neutral port for credentials held on behalf of an owner.
///
/// Baukit ships the envelope encryption in [`crate::CredentialCipher`]; the
/// storage adapter stays product-local, because the row shape, the ownership
/// join, and the soft-delete rules belong to the product's schema. An adapter
/// encrypts on [`CredentialVault::store`] and decrypts on
/// [`CredentialVault::load`], and never persists plaintext.
///
/// `owner_id` is the principal the credentials belong to, and `scope_id`
/// identifies the individual credential set, such as one linked third-party
/// connection. Write and delete take both so an adapter can enforce ownership
/// in the same statement.
pub trait CredentialVault: Send + Sync {
    /// Stores or replaces the secrets for one scope owned by `owner_id`.
    ///
    /// Returns [`CredentialVaultError::NotFound`] when the owner does not own
    /// an active scope with that id.
    fn store<'a>(
        &'a self,
        owner_id: Uuid,
        scope_id: Uuid,
        secrets: &'a CredentialSecrets,
    ) -> VaultFuture<'a, Result<(), CredentialVaultError>>;

    /// Loads and decrypts the secrets stored for one scope.
    fn load(
        &self,
        scope_id: Uuid,
    ) -> VaultFuture<'_, Result<CredentialSecrets, CredentialVaultError>>;

    /// Removes the secrets for one scope owned by `owner_id`.
    fn delete(
        &self,
        owner_id: Uuid,
        scope_id: Uuid,
    ) -> VaultFuture<'_, Result<(), CredentialVaultError>>;
}
