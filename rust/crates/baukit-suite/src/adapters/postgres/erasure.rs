use super::{PostgresSuiteLinkStore, links_for_erasure, lock_owner};
use crate::{
    domain::{SUITE_JOB_TYPES, SuiteLink},
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
    async fn owner_in_transaction(
        &self,
        connection: &mut PgConnection,
        subject: &str,
    ) -> Result<Option<Uuid>, sqlx::Error>;
    /// Locks the product owner row with `FOR UPDATE` after the suite advisory lock.
    /// This waits for domain writes and their outbox jobs before suite cleanup starts.
    async fn lock_owner_in_transaction(
        &self,
        connection: &mut PgConnection,
        owner: Uuid,
    ) -> Result<(), sqlx::Error>;
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
                super::lock_owner(connection, owner)
                    .await
                    .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
                self.owners
                    .lock_owner_in_transaction(connection, owner)
                    .await?;
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

impl PostgresSuiteLinkStore {
    /// Links that a peer may still consider active. Call before the erasure transaction.
    ///
    /// # Errors
    ///
    /// Returns the `sqlx::Error` when the link query fails.
    pub async fn links_for_owner_erasure(
        &self,
        owner: Uuid,
    ) -> Result<Vec<SuiteLink>, sqlx::Error> {
        links_for_erasure(&self.pool, owner).await
    }
    /// Deletes suite jobs and every suite row in the product's erasure transaction.
    /// Follow the crate lock order when locking product rows.
    /// `PostgresSuiteErasure` takes the owner locks before calling this method.
    ///
    /// # Errors
    ///
    /// Returns `sqlx::Error::Protocol` when the owner lock fails and the `sqlx::Error` of any failed
    /// select or delete.
    pub async fn erase_owner(
        &self,
        connection: &mut sqlx::PgConnection,
        owner: Uuid,
    ) -> Result<(), sqlx::Error> {
        lock_owner(connection, owner)
            .await
            .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
        sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM suite_links WHERE user_id=$1 ORDER BY id FOR UPDATE",
        )
        .bind(owner)
        .fetch_all(&mut *connection)
        .await?;
        sqlx::query("DELETE FROM job_outbox WHERE job_type=ANY($2) AND payload->>'link_id' IN (SELECT id::text FROM suite_links WHERE user_id=$1)")
            .bind(owner).bind(SUITE_JOB_TYPES.as_slice()).execute(&mut *connection).await?;
        for statement in [
            "DELETE FROM suite_inbound_events WHERE user_id=$1",
            "DELETE FROM suite_link_codes WHERE user_id=$1",
            "DELETE FROM suite_link_requests WHERE user_id=$1",
            "DELETE FROM suite_links WHERE user_id=$1",
        ] {
            sqlx::query(statement)
                .bind(owner)
                .execute(&mut *connection)
                .await?;
        }
        Ok(())
    }
}
