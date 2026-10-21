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

/// Product export support, required before exports can be enabled in configuration.
pub trait DataSubjectExportProduct: DataSubjectProduct {
    /// Exports records using the operation transaction without committing or external calls.
    /// Return categories and records in a stable order; category names contain no subject data.
    fn export<'a>(
        &'a self,
        connection: &'a mut PgConnection,
        subject: &'a LocalSubject,
    ) -> DataSubjectFuture<'a, Result<Vec<ExportCategory>, sqlx::Error>>;
}

/// Product records in one named export category. Debug output omits record content.
#[derive(Clone)]
pub struct ExportCategory {
    /// Stable category name, without identifiers or record content.
    pub name: String,
    /// JSON records in product-defined order.
    pub records: Vec<serde_json::Value>,
}

impl fmt::Debug for ExportCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ExportCategory { .. }")
    }
}
