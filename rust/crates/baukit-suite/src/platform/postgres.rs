use super::*;
use async_trait::async_trait;
use sqlx::{PgConnection, PgPool};

/// Copy into a forward product migration. No peer-link or jobs schema required.
pub const PLATFORM_MIGRATION_SQL: &str = include_str!("../../migrations/004_platform.sql");

#[derive(Clone)]
pub struct PostgresPlatformInbox {
    pool: PgPool,
}
impl PostgresPlatformInbox {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    /// Purge a bounded batch. Keep receipts for at least 30 days before pruning.
    pub async fn purge_received_before(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
        batch: u32,
    ) -> Result<u64, PlatformError> {
        if batch == 0 {
            return Err(PlatformError::Invalid);
        }
        Ok(sqlx::query("DELETE FROM platform_inbound_receipts WHERE event_id IN (SELECT event_id FROM platform_inbound_receipts WHERE received_at < $1 ORDER BY received_at,event_id LIMIT $2 FOR UPDATE SKIP LOCKED)")
            .bind(cutoff).bind(i64::from(batch)).execute(&self.pool).await.map_err(storage)?.rows_affected())
    }
}
#[async_trait]
impl PlatformInbox for PostgresPlatformInbox {
    async fn receive(
        &self,
        event: &PlatformEvent,
        body_hash: [u8; 32],
        handler: &dyn PlatformEventHandler,
    ) -> Result<ReceiptOutcome, PlatformError> {
        // Avoid network work for completed receipts, releasing the connection
        // before prepare. The transactional insert below resolves concurrent claims.
        let previous: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT body_hash FROM platform_inbound_receipts WHERE event_id=$1")
                .bind(&event.envelope().event_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(storage)?;
        if let Some(previous) = previous {
            return if previous == body_hash {
                Ok(ReceiptOutcome::Duplicate)
            } else {
                Err(PlatformError::EventIdConflict)
            };
        }
        let known = !matches!(event.kind(), PlatformEventKind::Unknown);
        let prepared = if known {
            handler.prepare(event).await?
        } else {
            None
        };
        let handler = prepared.as_deref().unwrap_or(handler);
        let mut tx = self.pool.begin().await.map_err(storage)?;
        let inserted = sqlx::query("INSERT INTO platform_inbound_receipts(event_id,body_hash) VALUES($1,$2) ON CONFLICT DO NOTHING")
            .bind(&event.envelope().event_id).bind(body_hash.as_slice())
            .execute(&mut *tx).await.map_err(storage)?.rows_affected();
        if inserted == 0 {
            let previous: Vec<u8> = sqlx::query_scalar(
                "SELECT body_hash FROM platform_inbound_receipts WHERE event_id=$1",
            )
            .bind(&event.envelope().event_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(storage)?;
            return if previous == body_hash {
                Ok(ReceiptOutcome::Duplicate)
            } else {
                Err(PlatformError::EventIdConflict)
            };
        }
        if known {
            let mut context = EventContext::with_postgres(&mut tx);
            handler.handle(event, &mut context).await?;
        }
        tx.commit().await.map_err(storage)?;
        Ok(ReceiptOutcome::Applied)
    }
}
#[derive(Clone)]
pub struct PostgresEntitlementCache {
    pool: PgPool,
}
impl PostgresEntitlementCache {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    /// Remove all subject snapshots in the caller's erasure transaction.
    pub async fn forget_in_transaction(
        subject: &str,
        connection: &mut PgConnection,
    ) -> Result<(), PlatformError> {
        sqlx::query("DELETE FROM platform_entitlement_cache WHERE subject=$1")
            .bind(subject)
            .execute(connection)
            .await
            .map_err(storage)?;
        Ok(())
    }
    pub async fn store_in_transaction(
        snapshot: &EntitlementSnapshot,
        connection: &mut PgConnection,
    ) -> Result<(), PlatformError> {
        snapshot.validate()?;
        sqlx::query("INSERT INTO platform_entitlement_cache(subject,product_id,tier,features,revision,valid_until,grace_until) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(subject,product_id) DO UPDATE SET tier=EXCLUDED.tier,features=EXCLUDED.features,revision=EXCLUDED.revision,valid_until=EXCLUDED.valid_until,grace_until=EXCLUDED.grace_until,updated_at=now() WHERE platform_entitlement_cache.revision < EXCLUDED.revision OR (platform_entitlement_cache.revision=EXCLUDED.revision AND platform_entitlement_cache.valid_until < EXCLUDED.valid_until)")
            .bind(&snapshot.subject).bind(&snapshot.product_id).bind(&snapshot.tier)
            .bind(sqlx::types::Json(&snapshot.features)).bind(i64::try_from(snapshot.revision).map_err(|_| PlatformError::Invalid)?)
            .bind(snapshot.valid_until).bind(snapshot.grace_until)
            .execute(connection).await.map_err(storage)?;
        Ok(())
    }
}
#[derive(sqlx::FromRow)]
struct SnapshotRow {
    subject: String,
    product_id: String,
    tier: String,
    features: sqlx::types::Json<Vec<String>>,
    revision: i64,
    valid_until: i64,
    grace_until: Option<i64>,
}
#[async_trait]
impl EntitlementCache for PostgresEntitlementCache {
    async fn get(
        &self,
        subject: &str,
        product_id: &str,
        now: i64,
    ) -> Result<Option<EntitlementSnapshot>, PlatformError> {
        let row: Option<SnapshotRow> = sqlx::query_as("SELECT subject,product_id,tier,features,revision,valid_until,grace_until FROM platform_entitlement_cache WHERE subject=$1 AND product_id=$2 AND valid_until>$3")
            .bind(subject).bind(product_id).bind(now).fetch_optional(&self.pool).await.map_err(storage)?;
        row.map(|row| {
            Ok(EntitlementSnapshot {
                subject: row.subject,
                product_id: row.product_id,
                tier: row.tier,
                features: row.features.0,
                revision: u64::try_from(row.revision).map_err(|_| PlatformError::Invalid)?,
                valid_until: row.valid_until,
                grace_until: row.grace_until,
            })
        })
        .transpose()
    }
    async fn revision(&self, subject: &str, product_id: &str) -> Result<u64, PlatformError> {
        let revision: Option<i64> = sqlx::query_scalar(
            "SELECT revision FROM platform_entitlement_cache WHERE subject=$1 AND product_id=$2",
        )
        .bind(subject)
        .bind(product_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?;
        u64::try_from(revision.unwrap_or(0)).map_err(|_| PlatformError::Invalid)
    }
    async fn forget(&self, subject: &str) -> Result<(), PlatformError> {
        let mut connection = self.pool.acquire().await.map_err(storage)?;
        Self::forget_in_transaction(subject, &mut connection).await
    }
    async fn store(&self, snapshot: &EntitlementSnapshot) -> Result<(), PlatformError> {
        let mut connection = self.pool.acquire().await.map_err(storage)?;
        Self::store_in_transaction(snapshot, &mut connection).await
    }
    async fn store_in_context(
        &self,
        snapshot: &EntitlementSnapshot,
        context: &mut EventContext<'_>,
    ) -> Result<(), PlatformError> {
        match context.postgres() {
            Some(connection) => Self::store_in_transaction(snapshot, connection).await,
            None => self.store(snapshot).await,
        }
    }
}
fn storage(_: sqlx::Error) -> PlatformError {
    PlatformError::Unavailable
}
