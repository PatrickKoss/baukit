mod config;
mod product;
mod router;
mod store;
mod worker;

pub use config::{DataSubjectConfig, DataSubjectError};
pub use product::{DataSubjectFuture, DataSubjectProduct, ErasureReport, LocalSubject};
pub use router::{authenticated_routes, discovery_routes};
pub use store::PostgresDataSubjectStore;
pub use worker::DataSubjectHandler;

/// Copy after the jobs and erasure migrations in the product's migration sequence.
pub const POSTGRES_MIGRATION_SQL: &str = include_str!("../../migrations/0001_data_subject.sql");
/// Durable job kind for erasure execution.
pub const ERASURE_JOB_TYPE: &str = "data-subject.erasure";
/// Durable job kind that waits for identity deletion before signing the result.
pub const FINALIZATION_JOB_TYPE: &str = "data-subject.finalize";
/// Maximum rows removed by one purge call.
pub const MAX_PURGE_BATCH_SIZE: u32 = 10_000;
