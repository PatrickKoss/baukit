use std::{fmt, future::Future, pin::Pin};

use chrono::{DateTime, Utc};
use sqlx::{PgExecutor, PgPool, Postgres, Transaction};
use uuid::Uuid;

pub(crate) type StoreFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, sqlx::Error>> + Send + 'a>>;

/// One row of `baukit_auth_browser_sessions`. Token columns hold sealed bytes.
/// Debug output redacts the secret hash and every token column.
#[derive(Clone)]
pub(crate) struct SessionRow {
    pub(crate) id: Uuid,
    pub(crate) secret_hash: Vec<u8>,
    pub(crate) family_id: Uuid,
    pub(crate) subject: String,
    pub(crate) key_version: i32,
    pub(crate) refresh_token: Vec<u8>,
    pub(crate) id_token: Option<Vec<u8>>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) last_used_at: DateTime<Utc>,
    pub(crate) idle_expires_at: DateTime<Utc>,
    pub(crate) absolute_expires_at: DateTime<Utc>,
    pub(crate) revoked_at: Option<DateTime<Utc>>,
    pub(crate) rotated_to: Option<Uuid>,
    pub(crate) grace_access_token: Option<Vec<u8>>,
    pub(crate) grace_expires_at: Option<DateTime<Utc>>,
    pub(crate) grace_until: Option<DateTime<Utc>>,
}

impl fmt::Debug for SessionRow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionRow")
            .field("id", &self.id)
            .field("secret_hash", &"[REDACTED]")
            .field("family_id", &self.family_id)
            .field("subject", &self.subject)
            .field("key_version", &self.key_version)
            .field("refresh_token", &"[REDACTED]")
            .field("id_token", &self.id_token.as_ref().map(|_| "[REDACTED]"))
            .field("created_at", &self.created_at)
            .field("last_used_at", &self.last_used_at)
            .field("idle_expires_at", &self.idle_expires_at)
            .field("absolute_expires_at", &self.absolute_expires_at)
            .field("revoked_at", &self.revoked_at)
            .field("rotated_to", &self.rotated_to)
            .field(
                "grace_access_token",
                &self.grace_access_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("grace_expires_at", &self.grace_expires_at)
            .field("grace_until", &self.grace_until)
            .finish()
    }
}

/// Session persistence port. Every operation runs inside one transaction.
pub(crate) trait SessionStore: Send + Sync {
    fn begin(&self) -> StoreFuture<'_, Box<dyn SessionTransaction>>;
}

pub(crate) trait SessionTransaction: Send {
    /// Loads a row and holds its lock until commit or drop.
    fn lock<'a>(&'a mut self, id: Uuid) -> StoreFuture<'a, Option<SessionRow>>;
    fn insert<'a>(&'a mut self, row: &'a SessionRow) -> StoreFuture<'a, ()>;
    fn mark_rotated<'a>(
        &'a mut self,
        id: Uuid,
        successor: Uuid,
        now: DateTime<Utc>,
    ) -> StoreFuture<'a, ()>;
    fn revoke_family<'a>(&'a mut self, family_id: Uuid, now: DateTime<Utc>) -> StoreFuture<'a, ()>;
    fn delete_family<'a>(&'a mut self, family_id: Uuid) -> StoreFuture<'a, ()>;
    /// Returns the family's newest row that was not rotated.
    fn family_head<'a>(&'a mut self, family_id: Uuid) -> StoreFuture<'a, Option<SessionRow>>;
    fn commit(self: Box<Self>) -> StoreFuture<'static, ()>;
}

/// PostgreSQL adapter over `baukit_auth_browser_sessions`.
#[derive(Clone, Debug)]
pub(crate) struct PostgresSessionStore {
    pool: PgPool,
}

impl PostgresSessionStore {
    pub(crate) const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl SessionStore for PostgresSessionStore {
    fn begin(&self) -> StoreFuture<'_, Box<dyn SessionTransaction>> {
        Box::pin(async move {
            let transaction = self.pool.begin().await?;
            Ok(Box::new(PostgresSessionTransaction { transaction }) as Box<dyn SessionTransaction>)
        })
    }
}

struct PostgresSessionTransaction {
    transaction: Transaction<'static, Postgres>,
}

macro_rules! columns {
    () => {
        "id, secret_hash, family_id, subject, key_version, refresh_token, id_token,
    created_at, last_used_at, idle_expires_at, absolute_expires_at, revoked_at, rotated_to,
    grace_access_token, grace_expires_at, grace_until"
    };
}

type RowTuple = (
    Uuid,
    Vec<u8>,
    Uuid,
    String,
    i32,
    Vec<u8>,
    Option<Vec<u8>>,
    DateTime<Utc>,
    DateTime<Utc>,
    DateTime<Utc>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    Option<Uuid>,
    Option<Vec<u8>>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
);

fn from_tuple(row: RowTuple) -> SessionRow {
    let (
        id,
        secret_hash,
        family_id,
        subject,
        key_version,
        refresh_token,
        id_token,
        created_at,
        last_used_at,
        idle_expires_at,
        absolute_expires_at,
        revoked_at,
        rotated_to,
        grace_access_token,
        grace_expires_at,
        grace_until,
    ) = row;
    SessionRow {
        id,
        secret_hash,
        family_id,
        subject,
        key_version,
        refresh_token,
        id_token,
        created_at,
        last_used_at,
        idle_expires_at,
        absolute_expires_at,
        revoked_at,
        rotated_to,
        grace_access_token,
        grace_expires_at,
        grace_until,
    }
}

impl SessionTransaction for PostgresSessionTransaction {
    fn lock<'a>(&'a mut self, id: Uuid) -> StoreFuture<'a, Option<SessionRow>> {
        Box::pin(async move {
            let row: Option<RowTuple> = sqlx::query_as(concat!(
                "SELECT ",
                columns!(),
                " FROM baukit_auth_browser_sessions WHERE id = $1 FOR UPDATE"
            ))
            .bind(id)
            .fetch_optional(&mut *self.transaction)
            .await?;
            Ok(row.map(from_tuple))
        })
    }

    fn insert<'a>(&'a mut self, row: &'a SessionRow) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            sqlx::query(concat!(
                "INSERT INTO baukit_auth_browser_sessions (",
                columns!(),
                ") VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)"
            ))
            .bind(row.id)
            .bind(&row.secret_hash)
            .bind(row.family_id)
            .bind(&row.subject)
            .bind(row.key_version)
            .bind(&row.refresh_token)
            .bind(row.id_token.as_ref())
            .bind(row.created_at)
            .bind(row.last_used_at)
            .bind(row.idle_expires_at)
            .bind(row.absolute_expires_at)
            .bind(row.revoked_at)
            .bind(row.rotated_to)
            .bind(row.grace_access_token.as_ref())
            .bind(row.grace_expires_at)
            .bind(row.grace_until)
            .execute(&mut *self.transaction)
            .await?;
            Ok(())
        })
    }

    fn mark_rotated<'a>(
        &'a mut self,
        id: Uuid,
        successor: Uuid,
        now: DateTime<Utc>,
    ) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            sqlx::query(
                "UPDATE baukit_auth_browser_sessions
                 SET rotated_to = $2, last_used_at = $3,
                     grace_access_token = NULL, grace_expires_at = NULL, grace_until = NULL
                 WHERE id = $1",
            )
            .bind(id)
            .bind(successor)
            .bind(now)
            .execute(&mut *self.transaction)
            .await?;
            Ok(())
        })
    }

    fn revoke_family<'a>(&'a mut self, family_id: Uuid, now: DateTime<Utc>) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            sqlx::query(
                "UPDATE baukit_auth_browser_sessions
                 SET revoked_at = $2,
                     grace_access_token = NULL, grace_expires_at = NULL, grace_until = NULL
                 WHERE family_id = $1 AND revoked_at IS NULL",
            )
            .bind(family_id)
            .bind(now)
            .execute(&mut *self.transaction)
            .await?;
            Ok(())
        })
    }

    fn delete_family<'a>(&'a mut self, family_id: Uuid) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            sqlx::query("DELETE FROM baukit_auth_browser_sessions WHERE family_id = $1")
                .bind(family_id)
                .execute(&mut *self.transaction)
                .await?;
            Ok(())
        })
    }

    fn family_head<'a>(&'a mut self, family_id: Uuid) -> StoreFuture<'a, Option<SessionRow>> {
        Box::pin(async move {
            let row: Option<RowTuple> = sqlx::query_as(concat!(
                "SELECT ",
                columns!(),
                " FROM baukit_auth_browser_sessions
                 WHERE family_id = $1 AND rotated_to IS NULL
                 ORDER BY created_at DESC LIMIT 1 FOR UPDATE"
            ))
            .bind(family_id)
            .fetch_optional(&mut *self.transaction)
            .await?;
            Ok(row.map(from_tuple))
        })
    }

    fn commit(self: Box<Self>) -> StoreFuture<'static, ()> {
        Box::pin(async move { self.transaction.commit().await })
    }
}

/// Deletes browser session families that can no longer be used.
///
/// A family goes when any of its rows was revoked before `cutoff`, or when its
/// absolute or idle lifetime ended before `cutoff`. Whole families are deleted so a rotated row never
/// outlives the row that detects its reuse. At most `limit` families go per call.
///
/// # Errors
///
/// Returns any database error unchanged.
pub async fn purge_browser_sessions<'e, E>(
    executor: E,
    cutoff: DateTime<Utc>,
    limit: std::num::NonZeroU32,
) -> Result<u64, sqlx::Error>
where
    E: PgExecutor<'e>,
{
    let deleted = sqlx::query(
        "DELETE FROM baukit_auth_browser_sessions WHERE family_id IN (
             SELECT family_id FROM baukit_auth_browser_sessions
             GROUP BY family_id
             HAVING min(revoked_at) < $1
                 OR max(absolute_expires_at) < $1
                 OR max(idle_expires_at) < $1
             ORDER BY family_id
             LIMIT $2
         )",
    )
    .bind(cutoff)
    .bind(i64::from(limit.get()))
    .execute(executor)
    .await?
    .rows_affected();
    Ok(deleted)
}

/// Deletes every browser session of one subject.
///
/// Use it for data-subject erasure and for "sign out everywhere". The refresh
/// tokens stay valid at the IdP until it expires or revokes them.
///
/// # Errors
///
/// Returns any database error unchanged.
pub async fn erase_subject_browser_sessions<'e, E>(
    executor: E,
    subject: &str,
) -> Result<u64, sqlx::Error>
where
    E: PgExecutor<'e>,
{
    let deleted = sqlx::query("DELETE FROM baukit_auth_browser_sessions WHERE subject = $1")
        .bind(subject)
        .execute(executor)
        .await?
        .rows_affected();
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use uuid::Uuid;

    use super::SessionRow;

    #[test]
    fn debug_redacts_hash_and_token_columns() {
        let now = Utc::now();
        let row = SessionRow {
            id: Uuid::nil(),
            secret_hash: b"hash-bytes-xyz".to_vec(),
            family_id: Uuid::nil(),
            subject: "user-1".to_owned(),
            key_version: 1,
            refresh_token: b"refresh-bytes-xyz".to_vec(),
            id_token: Some(b"id-bytes-xyz".to_vec()),
            created_at: now,
            last_used_at: now,
            idle_expires_at: now,
            absolute_expires_at: now,
            revoked_at: None,
            rotated_to: None,
            grace_access_token: Some(b"grace-bytes-xyz".to_vec()),
            grace_expires_at: None,
            grace_until: None,
        };

        let debug = format!("{row:?}");

        for secret in [
            row.secret_hash.as_slice(),
            row.refresh_token.as_slice(),
            b"id-bytes-xyz".as_slice(),
            b"grace-bytes-xyz".as_slice(),
        ] {
            assert!(!debug.contains(&format!("{secret:?}")));
        }
        assert!(debug.contains("user-1"));
        assert!(debug.contains("[REDACTED]"));
    }
}
