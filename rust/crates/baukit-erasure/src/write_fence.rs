use std::future::Future;

use jiff::Timestamp;
use sqlx::{PgPool, Postgres, Transaction};

use crate::{ErasureError, PostgresErasureStore};

/// The authenticated session whose product writes must pass the erasure fence.
#[derive(Clone)]
pub struct WriteFence {
    store: PostgresErasureStore,
    subject: String,
    authenticated_at: Option<Timestamp>,
}

impl WriteFence {
    /// Fences writes for `subject` through `store`.
    #[must_use]
    pub fn new(
        store: PostgresErasureStore,
        subject: impl Into<String>,
        authenticated_at: Option<Timestamp>,
    ) -> Self {
        Self {
            store,
            subject: subject.into(),
            authenticated_at,
        }
    }
}

tokio::task_local! {
    static CURRENT_FENCE: WriteFence;
}

/// Runs `future` with `fence` as the fence for [`begin_fenced_write`].
pub async fn with_write_fence<F: Future>(fence: WriteFence, future: F) -> F::Output {
    CURRENT_FENCE.scope(fence, future).await
}

/// Begins a write transaction. Inside [`with_write_fence`] it holds the subject lock and
/// passed the fence, as [`PostgresErasureStore::begin_guarded_write`] does. Outside any
/// scope, such as a job or a migration, it is a plain transaction on `pool`.
///
/// # Errors
///
/// [`ErasureError::ProfileErased`] when the scoped session is fenced, and
/// [`ErasureError::Database`] when the transaction, lock or fence query fails.
pub async fn begin_fenced_write(
    pool: &PgPool,
) -> Result<Transaction<'static, Postgres>, ErasureError> {
    match CURRENT_FENCE.try_with(Clone::clone) {
        Ok(fence) => {
            fence
                .store
                .begin_guarded_write(&fence.subject, fence.authenticated_at)
                .await
        }
        Err(_) => Ok(pool.begin().await?),
    }
}
