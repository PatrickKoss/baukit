use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// Allocates the owner's next revision inside the caller's transaction.
///
/// The `UPDATE ... RETURNING` takes a row lock for the duration of the
/// transaction, so concurrent writers for one owner serialize and each sees a
/// distinct, increasing value. Different owners touch different rows and do not
/// block each other. A rollback discards the allocation.
///
/// The owner's counter row must exist; call [`ensure_owner`] once when the
/// owner is created.
///
/// # Errors
///
/// Returns [`sqlx::Error::RowNotFound`] when the owner has no counter row, and
/// any other database error unchanged.
pub async fn next_revision(
    transaction: &mut Transaction<'_, Postgres>,
    owner_id: Uuid,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "UPDATE sync_revisions
         SET last_revision = last_revision + 1
         WHERE owner_id = $1
         RETURNING last_revision",
    )
    .bind(owner_id)
    .fetch_one(&mut **transaction)
    .await
}

/// Creates the owner's revision counter if it does not exist yet.
///
/// Call this when the owner is created, in the same transaction. Calling it
/// again is harmless and never resets an existing counter.
///
/// # Errors
///
/// Returns any database error unchanged.
pub async fn ensure_owner(
    transaction: &mut Transaction<'_, Postgres>,
    owner_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO sync_revisions (owner_id)
         VALUES ($1)
         ON CONFLICT (owner_id) DO NOTHING",
    )
    .bind(owner_id)
    .execute(&mut **transaction)
    .await
    .map(|_| ())
}

/// Reads the owner's current revision without allocating a new one.
///
/// Use this to answer "what is the newest revision a pull could return". It
/// takes no lock, so a concurrent writer may advance the counter immediately
/// after the read.
///
/// # Errors
///
/// Returns any database error unchanged. An owner without a counter row reads
/// as `None`.
pub async fn current_revision(
    transaction: &mut Transaction<'_, Postgres>,
    owner_id: Uuid,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT last_revision FROM sync_revisions WHERE owner_id = $1")
        .bind(owner_id)
        .fetch_optional(&mut **transaction)
        .await
}

/// Reads and locks the owner's current revision without allocating a new one.
///
/// The row lock lasts until the caller's transaction commits or rolls back.
/// Use this before a read-dependent write that must not race another revision
/// allocation. [`current_revision`] remains the non-locking read for pull
/// boundaries and status queries.
///
/// # Errors
///
/// Returns any database error unchanged. An owner without a counter row reads
/// as `None` and no row is locked.
pub async fn current_revision_for_update(
    transaction: &mut Transaction<'_, Postgres>,
    owner_id: Uuid,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT last_revision FROM sync_revisions WHERE owner_id = $1 FOR UPDATE")
        .bind(owner_id)
        .fetch_optional(&mut **transaction)
        .await
}
