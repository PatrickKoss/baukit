//! Data subject v1 wire types, identifier normalization, and Ed25519 DSSE receipts.
//!
//! The core is available with `default-features = false`. The default `server`
//! feature adds HTTP routes, a PostgreSQL store, and durable erasure execution.
#![deny(missing_docs)]

pub mod codes;
mod receipt;
#[cfg(feature = "server")]
mod server;
mod types;
mod validation;

#[cfg(feature = "server")]
pub use server::*;

pub use receipt::{
    RECEIPT_PAYLOAD_TYPE, RECEIPT_TYPE, ReceiptError, ReceiptSigner, ReceiptVerifier, pae,
};
pub use types::*;
pub use validation::{
    CustomIdentifierType, IdentifierType, NormalizedIdentifier, NormalizedRequest, ValidationError,
    normalize_request, validate_idempotency_key, validate_subject_reference,
};

/// The supported HTTP contract version.
pub const CONTRACT_VERSION: &str = "1";
