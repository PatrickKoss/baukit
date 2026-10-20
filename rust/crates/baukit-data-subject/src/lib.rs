//! Data subject v1 wire types, identifier normalization, and Ed25519 DSSE receipts.
//!
//! The core is available with `default-features = false`. The `server` feature
//! is reserved for the HTTP, operation-store, and worker adapters.
#![deny(missing_docs)]

pub mod codes;
mod receipt;
mod types;
mod validation;

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
