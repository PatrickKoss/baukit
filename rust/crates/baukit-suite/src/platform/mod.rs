//! Service-level hub notifications, offline entitlement tickets, credits and
//! organization summaries.
//!
//! Enable `platform` for HTTP, memory adapters and ticket verification. Add
//! `postgres` for durable receipts and entitlement snapshots. These do not use
//! peer links or require user consent. The v1 wire names remain unchanged.

mod cache;
mod config;
mod credits;
mod events;
mod hub;
mod memory;
mod organizations;
#[cfg(feature = "postgres")]
mod postgres;
mod receiver;
mod tickets;

pub use cache::*;
pub use config::PlatformConfig;
pub use credits::*;
pub use events::*;
pub use hub::*;
pub use memory::InMemoryPlatformInbox;
pub use organizations::*;
#[cfg(feature = "postgres")]
pub use postgres::{PLATFORM_MIGRATION_SQL, PostgresEntitlementCache, PostgresPlatformInbox};
pub use receiver::*;
pub use tickets::*;

use async_trait::async_trait;

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PlatformError {
    #[error("platform_unavailable")]
    Unavailable,
    #[error("platform_event_invalid")]
    Invalid,
    #[error("platform_event_id_conflict")]
    EventIdConflict,
}

/// A handler's database writes can share the inbox receipt transaction.
/// External calls and memory adapters must tolerate replay after a failed attempt.
pub struct EventContext<'a> {
    #[cfg(feature = "postgres")]
    connection: Option<&'a mut sqlx::PgConnection>,
    marker: std::marker::PhantomData<&'a ()>,
}
impl<'a> EventContext<'a> {
    /// A context for an inbox without a shared database transaction.
    pub fn detached() -> Self {
        Self {
            #[cfg(feature = "postgres")]
            connection: None,
            marker: std::marker::PhantomData,
        }
    }
    /// Wrap the connection owned by the caller's receipt transaction.
    #[cfg(feature = "postgres")]
    pub fn with_postgres(connection: &'a mut sqlx::PgConnection) -> Self {
        Self {
            connection: Some(connection),
            marker: std::marker::PhantomData,
        }
    }
    #[cfg(feature = "postgres")]
    pub fn postgres(&mut self) -> Option<&mut sqlx::PgConnection> {
        self.connection.as_deref_mut()
    }
}

#[async_trait]
pub trait PlatformEventHandler: Send + Sync {
    /// Perform network work before claiming the receipt or opening a transaction.
    /// When present, the returned handler applies the prepared result instead.
    async fn prepare(
        &self,
        _event: &PlatformEvent,
    ) -> Result<Option<std::sync::Arc<dyn PlatformEventHandler>>, PlatformError> {
        Ok(None)
    }
    /// Prepare using the receiver's request time. Custom handlers that do not
    /// need a clock can keep implementing `prepare` alone.
    /// Overrides must also override `prepare` and delegate to `prepare_at`,
    /// so inboxes calling `prepare` directly use the same preparation.
    async fn prepare_at(
        &self,
        event: &PlatformEvent,
        _now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<std::sync::Arc<dyn PlatformEventHandler>>, PlatformError> {
        self.prepare(event).await
    }
    /// Return `Unavailable` for a retry, or `Invalid` for permanent rejection.
    /// Use `context.postgres()` for writes that must commit with the receipt.
    async fn handle(
        &self,
        event: &PlatformEvent,
        context: &mut EventContext<'_>,
    ) -> Result<(), PlatformError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiptOutcome {
    Applied,
    Duplicate,
}

#[async_trait]
pub trait PlatformInbox: Send + Sync {
    /// Serialize concurrent receipt of an event ID, reject a changed body and
    /// call `handler.prepare` before opening the receipt transaction, and record
    /// completion only after the selected handler succeeds. Unknown types are
    /// recorded without calling the handler. Cancellation must release the claim.
    async fn receive(
        &self,
        event: &PlatformEvent,
        body_hash: [u8; 32],
        handler: &dyn PlatformEventHandler,
    ) -> Result<ReceiptOutcome, PlatformError>;
}
