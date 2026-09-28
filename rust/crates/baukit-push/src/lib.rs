//! Provider-neutral push delivery with an Expo adapter.
//!
//! The crate separates a small [`PushSender`] port from its [`ExpoPushSender`]
//! adapter. Domain code builds [`PushMessage`] values and reads [`PushOutcome`]
//! values; nothing above the port names a provider. Scheduling, quiet hours,
//! and deciding who gets notified stay in the product.
//!
//! # Two-phase delivery
//!
//! Expo answers a send with a *ticket* per notification, then confirms delivery
//! later through a *receipt*. [`ExpoPushSender`] runs both phases per batch, so
//! one call returns [`PushDeliveryStatus::Delivered`] or
//! [`PushDeliveryStatus::Rejected`] wherever Expo has settled, and
//! [`PushDeliveryStatus::Accepted`] where it has not. Never resend on
//! `Accepted`; the notification is in flight.
//!
//! # Deferred receipts
//!
//! An `Accepted` outcome carries the provider's [`PushTicketId`]. Record those
//! with [`PendingReceiptStore::record_accepted`] and let the product's job
//! runner call [`poll_pending_receipts`], which reads the receipts through the
//! [`PushReceiptSource`] port and invalidates a `DeviceNotRegistered` token
//! that Expo only reported after the send. The crate ships no scheduler.
//!
//! # Device registry
//!
//! A device token stops working once the app is uninstalled. Expo reports that
//! as `DeviceNotRegistered`, which becomes
//! [`PushRejection::DeviceNotRegistered`]. The [`DeviceRegistry`] port stores
//! each owner's tokens and removes the dead ones in one call after a send:
//!
//! ```rust
//! use baukit_push::{DeviceRegistry, PushMessage, PushSender};
//! use chrono::Utc;
//!
//! async fn deliver(
//!     sender: &impl PushSender,
//!     registry: &impl DeviceRegistry,
//!     messages: Vec<PushMessage>,
//! ) -> Result<u64, Box<dyn std::error::Error>> {
//!     let sent_at = Utc::now();
//!     let outcomes = sender.send(messages).await?;
//!     Ok(registry.invalidate_dead_tokens(&outcomes, sent_at).await?)
//! }
//! ```
//!
//! The registry also registers, rotates, unregisters, lists, and erases
//! tokens, and caps devices per owner by evicting the least recently
//! registered. The optional `sqlx-postgres` feature adds
//! `PostgresDeviceRegistry` over the schema in
//! [`POSTGRES_PUSH_DEVICES_MIGRATION_SQL`]. [`DeviceToken`] redacts itself in
//! `Debug`, so [`PushMessage`] and [`PushOutcome`] are safe to log, and no
//! error carries a token.
//!
//! # Daily delivery claims
//!
//! A scheduled sender claims a [`DeliveryClaim`] keyed by owner, local date,
//! and kind before it sends, and releases it if the send fails, so two workers
//! never deliver the same daily notification twice. The [`DeliveryClaimStore`]
//! port is separate from the registry; event-driven senders skip it.
//! Eligibility, copy, quiet hours, and channel choice stay in the product.
//!
//! # Retries
//!
//! Whole-request failures arrive as [`PushError::Transport`] carrying a
//! [`baukit_http::RetryClass`], so an Expo rate limit with a `Retry-After`
//! header reaches the caller as a concrete delay through
//! [`PushError::retry_after`]. Per-notification refusals are not errors:
//! inspect [`PushRejection::is_retryable`] on each outcome instead.

#![deny(missing_docs)]

mod config;
mod delivery_claim;
mod expo;
#[cfg(feature = "test-support")]
mod fake;
#[cfg(feature = "test-support")]
mod memory;
mod pending_receipt;
mod port;
#[cfg(feature = "sqlx-postgres")]
mod postgres;
mod registry;

pub use config::{
    DEFAULT_EXPO_ENDPOINT, MAX_BATCH_SIZE, MAX_RECEIPT_BATCH_SIZE, PushConfig, PushOptions,
    PushOptionsError,
};
pub use delivery_claim::{
    DeliveryClaim, DeliveryClaimStore, DeliveryKind, MAX_DELIVERY_KIND_LENGTH,
};
pub use expo::ExpoPushSender;
#[cfg(feature = "test-support")]
pub use fake::FakePushSender;
#[cfg(feature = "test-support")]
pub use memory::{MemoryDeliveryClaimStore, MemoryDeviceRegistry, MemoryPendingReceiptStore};
pub use pending_receipt::{
    PendingReceipt, PendingReceiptStore, RECEIPT_POLL_DELAY, RECEIPT_RETENTION, ReceiptPoll,
    ReceiptPollError, accepted_receipts, poll_pending_receipts,
};
pub use port::{
    MAX_PUSH_TICKET_ID_LENGTH, PushDeliveryStatus, PushError, PushFuture, PushMessage, PushOutcome,
    PushReceipt, PushReceiptFuture, PushReceiptSource, PushRejection, PushSender, PushTicketId,
};
#[cfg(feature = "sqlx-postgres")]
pub use postgres::{
    PostgresDeliveryClaimStore, PostgresDeviceRegistry, PostgresPendingReceiptStore,
    erase_owner_delivery_claims, erase_owner_push_devices, purge_delivery_claims,
};
pub use registry::{
    DEFAULT_DEVICES_PER_OWNER, DevicePlatform, DeviceRegistration, DeviceRegistry, DeviceTimeZone,
    DeviceToken, MAX_DEVICE_TIME_ZONE_LENGTH, MAX_DEVICE_TOKEN_LENGTH, PushStoreError,
    PushStoreFuture, PushValidationError, RegisteredDevice, RegistrationOutcome, dead_tokens,
};

/// Reference PostgreSQL schema for the `sqlx-postgres` feature's `PostgresDeviceRegistry`.
///
/// Copy this SQL into a product migration; do not execute it dynamically during
/// application startup. Then add the product's owner foreign key with
/// `ON DELETE CASCADE`, as the file's header describes.
pub const POSTGRES_PUSH_DEVICES_MIGRATION_SQL: &str =
    include_str!("../migrations/0001_baukit_push_devices.sql");

/// Reference PostgreSQL schema for the `sqlx-postgres` feature's `PostgresDeliveryClaimStore`.
///
/// Only products that send scheduled pushes need it. Copy it into a product
/// migration and add the owner foreign key the file's header describes.
pub const POSTGRES_PUSH_DELIVERY_CLAIMS_MIGRATION_SQL: &str =
    include_str!("../migrations/0002_baukit_push_delivery_claims.sql");

/// Reference PostgreSQL schema for the `sqlx-postgres` feature's `PostgresPendingReceiptStore`.
///
/// Only products that poll receipts after the send need it. Copy it into a
/// product migration; it has no owner column.
pub const POSTGRES_PUSH_PENDING_RECEIPTS_MIGRATION_SQL: &str =
    include_str!("../migrations/0003_baukit_push_pending_receipts.sql");

// Compiles the README's examples so they cannot drift from the API.
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
struct ReadmeDoctests;
