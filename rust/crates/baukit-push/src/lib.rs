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
//! # Pruning dead tokens
//!
//! A device token stops working once the app is uninstalled. Expo reports that
//! as `DeviceNotRegistered`, which becomes
//! [`PushRejection::DeviceNotRegistered`]. Delete those tokens after every
//! send, or the same failures repeat forever:
//!
//! ```rust
//! use baukit_push::{PushOutcome, PushSender};
//!
//! async fn deliver(
//!     sender: &impl PushSender,
//!     messages: Vec<baukit_push::PushMessage>,
//! ) -> Result<Vec<String>, baukit_push::PushError> {
//!     let outcomes = sender.send(messages).await?;
//!     Ok(outcomes
//!         .iter()
//!         .filter(|outcome| PushOutcome::is_token_dead(outcome))
//!         .map(|outcome| outcome.token.clone())
//!         .collect())
//! }
//! ```
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
mod expo;
#[cfg(feature = "test-support")]
mod fake;
mod port;

pub use config::{
    DEFAULT_EXPO_ENDPOINT, MAX_BATCH_SIZE, PushConfig, PushOptions, PushOptionsError,
};
pub use expo::ExpoPushSender;
#[cfg(feature = "test-support")]
pub use fake::FakePushSender;
pub use port::{
    PushDeliveryStatus, PushError, PushFuture, PushMessage, PushOutcome, PushRejection, PushSender,
};

// Compiles the README's examples so they cannot drift from the API.
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
struct ReadmeDoctests;
