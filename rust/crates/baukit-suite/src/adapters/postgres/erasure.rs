use super::PostgresSuiteLinkStore;
use crate::{
    domain::SuiteLink,
    ports::{SuiteErasureLinks, SuiteStoreError},
};
use async_trait::async_trait;
use baukit_erasure::{ErasureFuture, ProductErasure};
use sqlx::PgConnection;
use std::sync::Arc;
use uuid::Uuid;

/// Product-owned subject lookup. Implement both reads against the same identity mapping.
#[async_trait]
pub trait SuiteErasureOwnerLookup: Send + Sync {
    /// Resolves the owner before remote revokes. No transaction is open.
    async fn owner(&self, subject: &str) -> Result<Option<Uuid>, sqlx::Error>;
    /// Resolves the owner without row locks, before suite takes its owner advisory lock.
    /// Product erasure may lock the identity row after the suite rows have been deleted.
    async fn owner_in_transaction(
        &self,
        connection: &mut PgConnection,
        subject: &str,
    ) -> Result<Option<Uuid>, sqlx::Error>;
}
/// Deletes suite rows before delegating to product erasure in the same transaction.
pub struct PostgresSuiteErasure {
    store: PostgresSuiteLinkStore,
    owners: Arc<dyn SuiteErasureOwnerLookup>,
    product: Arc<dyn ProductErasure>,
}
impl PostgresSuiteErasure {
    /// Wraps the product's deletion implementation.
    pub fn new(
        store: PostgresSuiteLinkStore,
        owners: Arc<dyn SuiteErasureOwnerLookup>,
        product: Arc<dyn ProductErasure>,
    ) -> Self {
        Self {
            store,
            owners,
            product,
        }
    }
}
impl ProductErasure for PostgresSuiteErasure {
    fn erase<'a>(
        &'a self,
        connection: &'a mut PgConnection,
        subject: &'a str,
    ) -> ErasureFuture<'a, Result<(), sqlx::Error>> {
        Box::pin(async move {
            if let Some(owner) = self
                .owners
                .owner_in_transaction(connection, subject)
                .await?
            {
                self.store.erase_owner(connection, owner).await?;
            }
            self.product.erase(connection, subject).await
        })
    }
}
#[async_trait]
impl SuiteErasureLinks for PostgresSuiteErasure {
    async fn links_for_erasure(&self, subject: &str) -> Result<Vec<SuiteLink>, SuiteStoreError> {
        match self.owners.owner(subject).await.map_err(super::storage)? {
            Some(owner) => self
                .store
                .links_for_owner_erasure(owner)
                .await
                .map_err(super::storage),
            None => Ok(Vec::new()),
        }
    }
}
