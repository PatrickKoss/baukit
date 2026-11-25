//! Verified twins of three production kernels. Each module mirrors its
//! production counterpart line for line and carries the proof of one V-ID from
//! `docs/quality-security/F-correctness-verification.md`. The `equivalence`
//! crate ties every twin back to production with Kani and proptest.

pub mod bucket;
pub mod hlc;
pub mod pull;
