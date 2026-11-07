use super::{lock_owner, storage};
use crate::domain::*;
use crate::ports::*;
use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

/// Locks and returns every owner with rows the cleanup pass may delete.
pub(super) async fn lock_cleanup_owners(
    tx: &mut sqlx::PgConnection,
    now: DateTime<Utc>,
) -> Result<Vec<Uuid>, SuiteStoreError> {
    let owners: Vec<Uuid> = sqlx::query_scalar(
        r#"SELECT user_id FROM suite_link_requests WHERE expires_at<$1
        UNION SELECT user_id FROM suite_link_codes WHERE expires_at<$1
        UNION SELECT user_id FROM suite_links WHERE (status='revoked' AND revoked_at<$2)
            OR (created_at<$3 AND last_delivery_at IS NULL AND last_received_at IS NULL)
        UNION SELECT l.user_id FROM suite_links l WHERE
            EXISTS(SELECT 1 FROM suite_failed_delivery_jobs f WHERE f.link_id=l.id AND NOT f.accounted)
            OR EXISTS(SELECT 1 FROM job_outbox j WHERE j.payload->>'link_id'=l.id::text
                AND j.job_type=ANY($5)
                AND j.status IN ('succeeded','failed','cancelled') AND j.updated_at<$4)
        ORDER BY user_id"#,
    )
    .bind(now - Duration::days(1))
    .bind(now - Duration::days(30))
    .bind(now - Duration::days(7))
    .bind(now - Duration::days(SUITE_TERMINAL_JOB_RETENTION_DAYS))
    .bind(SUITE_JOB_TYPES.as_slice())
    .fetch_all(&mut *tx)
    .await
    .map_err(storage)?;
    for owner in &owners {
        lock_owner(tx, *owner).await?;
    }
    Ok(owners)
}

/// Deletes expired link requests and codes, returning both counts.
pub(super) async fn delete_expired_requests(
    tx: &mut sqlx::PgConnection,
    now: DateTime<Utc>,
    owners: &[Uuid],
) -> Result<(u64, u64), SuiteStoreError> {
    let requests =
        sqlx::query(r#"DELETE FROM suite_link_requests WHERE expires_at<$1 AND user_id=ANY($2)"#)
            .bind(now - Duration::days(1))
            .bind(owners)
            .execute(&mut *tx)
            .await
            .map_err(storage)?
            .rows_affected();
    let codes =
        sqlx::query(r#"DELETE FROM suite_link_codes WHERE expires_at<$1 AND user_id=ANY($2)"#)
            .bind(now - Duration::days(1))
            .bind(owners)
            .execute(&mut *tx)
            .await
            .map_err(storage)?
            .rows_affected();
    Ok((requests, codes))
}

/// Deletes long-revoked and never-used links with their jobs, returning both link counts.
pub(super) async fn delete_expired_links(
    tx: &mut sqlx::PgConnection,
    now: DateTime<Utc>,
    owners: &[Uuid],
) -> Result<(u64, u64), SuiteStoreError> {
    let expired_links: Vec<Uuid> = sqlx::query_scalar(
        r#"SELECT id FROM suite_links WHERE user_id=ANY($3) AND ((status='revoked' AND revoked_at<$1)
         OR (created_at<$2 AND last_delivery_at IS NULL AND last_received_at IS NULL))
         ORDER BY id FOR UPDATE"#,
    )
    .bind(now - Duration::days(30))
    .bind(now - Duration::days(7))
    .bind(owners)
    .fetch_all(&mut *tx)
    .await
    .map_err(storage)?;
    let ids: Vec<String> = expired_links.into_iter().map(|id| id.to_string()).collect();
    sqlx::query(r#"DELETE FROM job_outbox WHERE job_type=ANY($2) AND payload->>'link_id'=ANY($1)"#)
        .bind(&ids)
        .bind(SUITE_JOB_TYPES.as_slice())
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
    let revoked_links = sqlx::query(
        r#"DELETE FROM suite_links WHERE status='revoked' AND revoked_at<$1 AND id::text=ANY($2)"#,
    )
    .bind(now - Duration::days(30))
    .bind(&ids)
    .execute(&mut *tx)
    .await
    .map_err(storage)?
    .rows_affected();
    let unused_links = sqlx::query(
        r#"DELETE FROM suite_links WHERE created_at<$1
        AND last_delivery_at IS NULL
        AND last_received_at IS NULL AND id::text=ANY($2)"#,
    )
    .bind(now - Duration::days(7))
    .bind(&ids)
    .execute(&mut *tx)
    .await
    .map_err(storage)?
    .rows_affected();
    Ok((revoked_links, unused_links))
}

/// Deletes finished suite jobs past retention for the given links or for links that no longer exist.
pub(super) async fn delete_terminal_jobs(
    tx: &mut sqlx::PgConnection,
    now: DateTime<Utc>,
    links: &[Uuid],
) -> Result<u64, SuiteStoreError> {
    let linked_ids: Vec<String> = links.iter().map(Uuid::to_string).collect();
    Ok(sqlx::query(
        r#"DELETE FROM job_outbox j
        WHERE j.job_type=ANY($3)
        AND j.status IN ('succeeded','failed','cancelled') AND j.updated_at<$1
        AND (j.payload->>'link_id'=ANY($2)
            OR NOT EXISTS(SELECT 1 FROM suite_links l WHERE l.id::text=j.payload->>'link_id'))"#,
    )
    .bind(now - Duration::days(SUITE_TERMINAL_JOB_RETENTION_DAYS))
    .bind(&linked_ids)
    .bind(SUITE_JOB_TYPES.as_slice())
    .execute(&mut *tx)
    .await
    .map_err(storage)?
    .rows_affected())
}
