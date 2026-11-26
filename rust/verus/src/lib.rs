//! Verified twins of three production kernels. Each module mirrors its
//! production counterpart line for line and carries the proof of one V-ID listed in
//! `docs/verification.md`. The `equivalence`
//! crate ties every twin back to production with Kani and proptest.

#![expect(
    clippy::assign_op_pattern,
    clippy::implicit_saturating_sub,
    clippy::indexing_slicing,
    clippy::len_zero,
    clippy::manual_is_multiple_of,
    clippy::manual_range_contains,
    clippy::ptr_arg,
    reason = "the twins use the forms Verus supports inside verus!: no compound assignment, \
              range contains, is_multiple_of, saturating_sub or slice parameters; Verus proves \
              every index in bounds"
)]

pub mod bucket;
pub mod hlc;
pub mod pull;
