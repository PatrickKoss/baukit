use std::{fmt, future::Future, pin::Pin};

use serde::{Deserialize, Serialize};
use sqlx::PgConnection;

use crate::{CategoryEntry, NormalizedIdentifier, Restriction};

/// Boxed asynchronous result for product database methods.
pub type DataSubjectFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A product's internal subject, never a copy of a lookup identifier.
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalSubject {
    /// Stable internal product record ID. Use an opaque surrogate ID.
    pub subject_id: String,
    /// Provider subject to fence and process with the configured identity retention.
    pub authentication_subject: Option<String>,
}

impl fmt::Debug for LocalSubject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LocalSubject { .. }")
    }
}

/// Safe category counts and legal obligations returned after product erasure.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ErasureReport {
    /// Categories affected by this erasure, without record content or identifiers.
    pub categories: Vec<CategoryEntry>,
    /// Obligations preventing complete deletion.
    pub restrictions: Vec<Restriction>,
}

/// Product database work. Methods must not commit or call external systems.
pub trait DataSubjectProduct: Send + Sync {
    /// Resolves transient identifiers during acceptance, in the POST transaction.
    /// Return stable internal IDs, never the supplied lookup values.
    fn resolve<'a>(
        &'a self,
        connection: &'a mut PgConnection,
        identifiers: &'a [NormalizedIdentifier],
    ) -> DataSubjectFuture<'a, Result<Vec<LocalSubject>, sqlx::Error>>;

    /// Erases the subject's owned records in the execution transaction.
    /// A subject removed since resolution must be safe to erase again.
    fn erase<'a>(
        &'a self,
        connection: &'a mut PgConnection,
        subject: &'a LocalSubject,
    ) -> DataSubjectFuture<'a, Result<ErasureReport, sqlx::Error>>;
}
