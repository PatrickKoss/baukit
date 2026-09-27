use std::num::NonZeroU32;

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::{PgConnection, PgExecutor, PgPool, Row as _, postgres::PgRow};
use uuid::Uuid;

use crate::{
    DEFAULT_DEVICES_PER_OWNER, DeliveryClaim, DeliveryClaimStore, DevicePlatform,
    DeviceRegistration, DeviceRegistry, DeviceTimeZone, DeviceToken, PushStoreError,
    PushStoreFuture, RegisteredDevice, RegistrationOutcome,
};

const OWNER_LOCK_NAMESPACE: &str = "baukit_push.devices:";

/// PostgreSQL [`DeviceRegistry`] over the table in
/// [`POSTGRES_PUSH_DEVICES_MIGRATION_SQL`](crate::POSTGRES_PUSH_DEVICES_MIGRATION_SQL).
///
/// `register` and `rotate` run in one transaction under a per-owner advisory
/// lock, so concurrent registrations for one owner cannot overshoot the cap.
/// The token is the primary key; registering a token another owner holds moves
/// it in the same statement.
#[derive(Clone, Debug)]
pub struct PostgresDeviceRegistry {
    pool: PgPool,
    devices_per_owner: NonZeroU32,
}

impl PostgresDeviceRegistry {
    /// Creates a registry that keeps [`DEFAULT_DEVICES_PER_OWNER`] devices per owner.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self {
            pool,
            devices_per_owner: DEFAULT_DEVICES_PER_OWNER,
        }
    }

    /// Sets how many devices each owner keeps before the oldest are evicted.
    #[must_use]
    pub const fn with_devices_per_owner(mut self, maximum: NonZeroU32) -> Self {
        self.devices_per_owner = maximum;
        self
    }

    async fn register_device(
        &self,
        previous: Option<DeviceToken>,
        registration: DeviceRegistration,
    ) -> Result<RegistrationOutcome, PushStoreError> {
        let mut transaction = self.pool.begin().await.map_err(PushStoreError::internal)?;
        lock_owner(&mut transaction, registration.owner_id)
            .await
            .map_err(PushStoreError::internal)?;
        if let Some(previous) = previous.filter(|previous| *previous != registration.token) {
            delete_owned(&mut *transaction, registration.owner_id, &previous)
                .await
                .map_err(PushStoreError::internal)?;
        }
        upsert_device(&mut *transaction, &registration)
            .await
            .map_err(PushStoreError::internal)?;
        let evicted = evict_over_cap(
            &mut transaction,
            registration.owner_id,
            &registration.token,
            self.devices_per_owner,
        )
        .await
        .map_err(PushStoreError::internal)?;
        transaction
            .commit()
            .await
            .map_err(PushStoreError::internal)?;
        Ok(RegistrationOutcome { evicted })
    }

    async fn unregister_device(
        &self,
        owner_id: Uuid,
        token: DeviceToken,
    ) -> Result<bool, PushStoreError> {
        delete_owned(&self.pool, owner_id, &token)
            .await
            .map(|deleted| deleted == 1)
            .map_err(PushStoreError::internal)
    }

    async fn list_devices(&self, owner_id: Uuid) -> Result<Vec<RegisteredDevice>, PushStoreError> {
        let rows = sqlx::query(
            "SELECT owner_id, token, platform, time_zone, created_at, last_registered_at
             FROM push_devices WHERE owner_id = $1
             ORDER BY last_registered_at DESC, created_at DESC, token DESC",
        )
        .bind(owner_id)
        .fetch_all(&self.pool)
        .await
        .map_err(PushStoreError::internal)?;
        rows.iter().map(registered_device).collect()
    }

    async fn invalidate_tokens(
        &self,
        tokens: Vec<DeviceToken>,
        sent_at: DateTime<Utc>,
    ) -> Result<u64, PushStoreError> {
        if tokens.is_empty() {
            return Ok(0);
        }
        let deleted = sqlx::query(
            "DELETE FROM push_devices WHERE token = ANY($1) AND last_registered_at <= $2",
        )
        .bind(tokens.iter().map(DeviceToken::expose).collect::<Vec<_>>())
        .bind(sent_at)
        .execute(&self.pool)
        .await
        .map_err(PushStoreError::internal)?
        .rows_affected();
        Ok(deleted)
    }
}

impl DeviceRegistry for PostgresDeviceRegistry {
    fn register(
        &self,
        registration: DeviceRegistration,
    ) -> PushStoreFuture<'_, Result<RegistrationOutcome, PushStoreError>> {
        Box::pin(self.register_device(None, registration))
    }

    fn rotate(
        &self,
        previous: DeviceToken,
        registration: DeviceRegistration,
    ) -> PushStoreFuture<'_, Result<RegistrationOutcome, PushStoreError>> {
        Box::pin(self.register_device(Some(previous), registration))
    }

    fn unregister(
        &self,
        owner_id: Uuid,
        token: DeviceToken,
    ) -> PushStoreFuture<'_, Result<bool, PushStoreError>> {
        Box::pin(self.unregister_device(owner_id, token))
    }

    fn list_for_owner(
        &self,
        owner_id: Uuid,
    ) -> PushStoreFuture<'_, Result<Vec<RegisteredDevice>, PushStoreError>> {
        Box::pin(self.list_devices(owner_id))
    }

    fn invalidate(
        &self,
        tokens: Vec<DeviceToken>,
        sent_at: DateTime<Utc>,
    ) -> PushStoreFuture<'_, Result<u64, PushStoreError>> {
        Box::pin(self.invalidate_tokens(tokens, sent_at))
    }

    fn erase_owner(&self, owner_id: Uuid) -> PushStoreFuture<'_, Result<u64, PushStoreError>> {
        Box::pin(async move {
            erase_owner_push_devices(&self.pool, owner_id)
                .await
                .map_err(PushStoreError::internal)
        })
    }
}

/// Deletes every device of one owner and returns how many rows went.
///
/// Products whose owner foreign key uses `ON DELETE CASCADE` get this for free.
/// Call it inside the product's erasure transaction when the owner row is kept
/// or no foreign key exists.
///
/// # Errors
///
/// Returns any database error unchanged.
pub async fn erase_owner_push_devices<'e, E>(
    executor: E,
    owner_id: Uuid,
) -> Result<u64, sqlx::Error>
where
    E: PgExecutor<'e>,
{
    let deleted = sqlx::query("DELETE FROM push_devices WHERE owner_id = $1")
        .bind(owner_id)
        .execute(executor)
        .await?
        .rows_affected();
    Ok(deleted)
}

/// PostgreSQL [`DeliveryClaimStore`] over the table in
/// [`POSTGRES_PUSH_DELIVERY_CLAIMS_MIGRATION_SQL`](crate::POSTGRES_PUSH_DELIVERY_CLAIMS_MIGRATION_SQL).
///
/// A claim is one `INSERT ... ON CONFLICT DO NOTHING` on the primary key
/// `(owner_id, local_date, kind)`, so exactly one concurrent claimer wins.
#[derive(Clone, Debug)]
pub struct PostgresDeliveryClaimStore {
    pool: PgPool,
}

impl PostgresDeliveryClaimStore {
    /// Creates a claim store on the pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn insert_claim(
        &self,
        claim: DeliveryClaim,
        claimed_at: DateTime<Utc>,
    ) -> Result<bool, PushStoreError> {
        let inserted = sqlx::query(
            "INSERT INTO push_delivery_claims (owner_id, local_date, kind, claimed_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (owner_id, local_date, kind) DO NOTHING",
        )
        .bind(claim.owner_id)
        .bind(claim.local_date)
        .bind(claim.kind.as_str())
        .bind(claimed_at)
        .execute(&self.pool)
        .await
        .map_err(PushStoreError::internal)?
        .rows_affected();
        Ok(inserted == 1)
    }

    async fn delete_claim(&self, claim: DeliveryClaim) -> Result<bool, PushStoreError> {
        let deleted = sqlx::query(
            "DELETE FROM push_delivery_claims
             WHERE owner_id = $1 AND local_date = $2 AND kind = $3",
        )
        .bind(claim.owner_id)
        .bind(claim.local_date)
        .bind(claim.kind.as_str())
        .execute(&self.pool)
        .await
        .map_err(PushStoreError::internal)?
        .rows_affected();
        Ok(deleted == 1)
    }
}

impl DeliveryClaimStore for PostgresDeliveryClaimStore {
    fn claim(
        &self,
        claim: DeliveryClaim,
        claimed_at: DateTime<Utc>,
    ) -> PushStoreFuture<'_, Result<bool, PushStoreError>> {
        Box::pin(self.insert_claim(claim, claimed_at))
    }

    fn release(&self, claim: DeliveryClaim) -> PushStoreFuture<'_, Result<bool, PushStoreError>> {
        Box::pin(self.delete_claim(claim))
    }
}

/// Deletes every delivery claim of one owner and returns how many rows went.
///
/// # Errors
///
/// Returns any database error unchanged.
pub async fn erase_owner_delivery_claims<'e, E>(
    executor: E,
    owner_id: Uuid,
) -> Result<u64, sqlx::Error>
where
    E: PgExecutor<'e>,
{
    let deleted = sqlx::query("DELETE FROM push_delivery_claims WHERE owner_id = $1")
        .bind(owner_id)
        .execute(executor)
        .await?
        .rows_affected();
    Ok(deleted)
}

/// Deletes one batch of claims for local dates before `before`.
///
/// Returns the number of deleted rows. Call it again until it returns less
/// than `limit`. Rows locked by a concurrent writer are skipped, not waited on.
///
/// # Errors
///
/// Returns any database error unchanged.
pub async fn purge_delivery_claims<'e, E>(
    executor: E,
    before: NaiveDate,
    limit: NonZeroU32,
) -> Result<u64, sqlx::Error>
where
    E: PgExecutor<'e>,
{
    let deleted = sqlx::query(
        "DELETE FROM push_delivery_claims WHERE (owner_id, local_date, kind) IN (
             SELECT owner_id, local_date, kind FROM push_delivery_claims
             WHERE local_date < $1
             ORDER BY local_date, owner_id, kind
             LIMIT $2
             FOR UPDATE SKIP LOCKED
         )",
    )
    .bind(before)
    .bind(i64::from(limit.get()))
    .execute(executor)
    .await?
    .rows_affected();
    Ok(deleted)
}

async fn lock_owner(connection: &mut PgConnection, owner_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1 || $2::text, 0))")
        .bind(OWNER_LOCK_NAMESPACE)
        .bind(owner_id)
        .execute(connection)
        .await?;
    Ok(())
}

async fn delete_owned<'e, E>(
    executor: E,
    owner_id: Uuid,
    token: &DeviceToken,
) -> Result<u64, sqlx::Error>
where
    E: PgExecutor<'e>,
{
    let deleted = sqlx::query("DELETE FROM push_devices WHERE owner_id = $1 AND token = $2")
        .bind(owner_id)
        .bind(token.expose())
        .execute(executor)
        .await?
        .rows_affected();
    Ok(deleted)
}

async fn upsert_device<'e, E>(
    executor: E,
    registration: &DeviceRegistration,
) -> Result<(), sqlx::Error>
where
    E: PgExecutor<'e>,
{
    sqlx::query(
        "INSERT INTO push_devices
             (token, owner_id, platform, time_zone, created_at, last_registered_at)
         VALUES ($1, $2, $3, $4, $5, $5)
         ON CONFLICT (token) DO UPDATE SET
             platform = EXCLUDED.platform,
             time_zone = EXCLUDED.time_zone,
             created_at = CASE WHEN push_devices.owner_id = EXCLUDED.owner_id
                 THEN push_devices.created_at ELSE EXCLUDED.created_at END,
             last_registered_at = CASE WHEN push_devices.owner_id = EXCLUDED.owner_id
                 THEN GREATEST(push_devices.last_registered_at, EXCLUDED.last_registered_at)
                 ELSE EXCLUDED.last_registered_at END,
             owner_id = EXCLUDED.owner_id",
    )
    .bind(registration.token.expose())
    .bind(registration.owner_id)
    .bind(registration.platform.as_str())
    .bind(registration.time_zone.as_ref().map(DeviceTimeZone::as_str))
    .bind(registration.registered_at)
    .execute(executor)
    .await?;
    Ok(())
}

async fn evict_over_cap(
    connection: &mut PgConnection,
    owner_id: Uuid,
    kept: &DeviceToken,
    devices_per_owner: NonZeroU32,
) -> Result<u64, sqlx::Error> {
    let others_kept = i64::from(devices_per_owner.get()) - 1;
    let evicted = sqlx::query(
        "DELETE FROM push_devices WHERE owner_id = $1 AND token IN (
             SELECT token FROM push_devices
             WHERE owner_id = $1 AND token <> $2
             ORDER BY last_registered_at DESC, created_at DESC, token DESC
             OFFSET $3
         )",
    )
    .bind(owner_id)
    .bind(kept.expose())
    .bind(others_kept)
    .execute(connection)
    .await?
    .rows_affected();
    Ok(evicted)
}

fn registered_device(row: &PgRow) -> Result<RegisteredDevice, PushStoreError> {
    let token: String = row.try_get("token").map_err(PushStoreError::internal)?;
    let platform: String = row.try_get("platform").map_err(PushStoreError::internal)?;
    let time_zone: Option<String> = row.try_get("time_zone").map_err(PushStoreError::internal)?;
    Ok(RegisteredDevice {
        owner_id: row.try_get("owner_id").map_err(PushStoreError::internal)?,
        token: DeviceToken::new(token).map_err(PushStoreError::internal)?,
        platform: DevicePlatform::parse(&platform)
            .ok_or_else(|| PushStoreError::internal("stored platform is unknown"))?,
        time_zone: time_zone
            .map(DeviceTimeZone::new)
            .transpose()
            .map_err(PushStoreError::internal)?,
        created_at: row
            .try_get("created_at")
            .map_err(PushStoreError::internal)?,
        last_registered_at: row
            .try_get("last_registered_at")
            .map_err(PushStoreError::internal)?,
    })
}
