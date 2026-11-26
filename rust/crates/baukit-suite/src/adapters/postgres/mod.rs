use crate::domain::*;
use crate::ports::*;
pub use crate::{
    SUITE_DELIVERY_PROBES_MIGRATION_SQL, SUITE_LOCK_ORDER_MIGRATION_SQL, SUITE_MIGRATION_SQL,
    SUITE_RECORD_IDS_MIGRATION_SQL, SUITE_RUNTIME_MIGRATION_SQL,
};
use async_trait::async_trait;
use jiff::{SignedDuration, Timestamp};
use jiff_sqlx::{Timestamp as SqlTimestamp, ToSqlx};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

mod cleanup;
mod erasure;
use cleanup::cleanup_in_transaction;
mod outbox;
mod probe;
mod reseal;
pub use erasure::{PostgresSuiteErasure, SuiteErasureOwnerLookup};
mod rows;
pub use outbox::PostgresSuiteEventOutbox;
use rows::{CodeRow, DeliveryRow, InboxRow, LinkRow, RequestRow, same_microsecond};

pub type SuiteTransaction = Transaction<'static, Postgres>;

#[derive(Clone)]
pub struct PostgresSuiteLinkStore {
    pool: PgPool,
}
impl PostgresSuiteLinkStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    async fn health(
        &self,
        owner: Uuid,
        id: Uuid,
        action: DeliveryAction,
        now: Timestamp,
    ) -> Result<DeliveryState, SuiteStoreError> {
        let mut tx = self.begin_transaction().await?;
        let link = self.link_for_update(&mut tx, owner, id).await?;
        let before = DeliveryState::from(&link);
        let after = if before.status == LinkStatus::Revoked {
            before
        } else {
            after_delivery(before, action, now)
        };
        sqlx::query(
            r#"UPDATE suite_links
            SET status=$2,
            delivery_health=$3,
            consecutive_failures=$4,
            last_delivery_at=CASE
            WHEN $5
            THEN $6
            ELSE last_delivery_at END,
            last_failure_at=CASE
            WHEN $7::text IS NOT NULL
            THEN $6
            ELSE last_failure_at END,
            last_failure_code=COALESCE($7,
            last_failure_code),
            revoked_at=CASE
            WHEN $2='revoked'
            THEN COALESCE(revoked_at,
            $6)
            ELSE revoked_at END,
            updated_at=$6, next_probe_at=$9, failed_probes=$10
            WHERE id=$1 AND user_id=$8"#,
        )
        .bind(id)
        .bind(after.status.as_str())
        .bind(after.health.as_str())
        .bind(i32::try_from(after.consecutive_failures).map_err(invalid)?)
        .bind(action == DeliveryAction::Delivered)
        .bind(now.to_sqlx())
        .bind(action.failure_code())
        .bind(owner)
        .bind(after.next_probe_at.map(|at| at.to_sqlx()))
        .bind(i32::try_from(after.failed_probes).map_err(invalid)?)
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
        self.commit_transaction(tx).await?;
        Ok(after)
    }
}
pub(crate) fn storage(e: impl std::fmt::Display + std::any::Any) -> SuiteStoreError {
    let error = &e as &dyn std::any::Any;
    let database = error.downcast_ref::<sqlx::Error>().or_else(|| {
        match error.downcast_ref::<baukit_jobs::StoreError>() {
            Some(baukit_jobs::StoreError::Database(error)) => Some(error),
            _ => None,
        }
    });
    if database
        .and_then(sqlx::Error::as_database_error)
        .and_then(|error| error.code())
        .is_some_and(|code| matches!(code.as_ref(), "57014" | "55P03"))
    {
        return SuiteStoreError::Timeout;
    }
    if matches!(
        error.downcast_ref::<sqlx::Error>(),
        Some(
            sqlx::Error::Decode(_)
                | sqlx::Error::ColumnDecode { .. }
                | sqlx::Error::ColumnNotFound(_)
                | sqlx::Error::ColumnIndexOutOfBounds { .. }
        )
    ) || matches!(
        error.downcast_ref::<baukit_jobs::StoreError>(),
        Some(baukit_jobs::StoreError::InvalidData(_))
    ) {
        return invalid(e);
    }
    if let Some(baukit_jobs::StoreError::Database(
        sqlx::Error::Decode(_) | sqlx::Error::ColumnDecode { .. },
    )) = error.downcast_ref::<baukit_jobs::StoreError>()
    {
        return invalid(e);
    }
    SuiteStoreError::Storage(e.to_string())
}
pub(crate) fn invalid(e: impl std::fmt::Display) -> SuiteStoreError {
    SuiteStoreError::InvalidData(e.to_string())
}
fn changed(count: u64) -> Result<(), SuiteStoreError> {
    if count == 1 {
        Ok(())
    } else {
        Err(SuiteStoreError::NotFound)
    }
}

fn replay_retry_after(last: Option<Timestamp>, now: Timestamp) -> u64 {
    last.map(|last| {
        let millis = (last + SignedDuration::from_secs(SUITE_REPLAY_INTERVAL_SECONDS) - now)
            .as_millis()
            .saturating_add(999)
            .max(0);
        u64::try_from(millis / 1000).unwrap_or(u64::MAX)
    })
    .unwrap_or(0)
}

#[async_trait]
impl SuiteLinkStore for PostgresSuiteLinkStore {
    async fn begin_transaction(&self) -> Result<SuiteTransaction, SuiteStoreError> {
        self.pool.begin().await.map_err(storage)
    }
    async fn commit_transaction(&self, tx: SuiteTransaction) -> Result<(), SuiteStoreError> {
        tx.commit().await.map_err(storage)
    }
    async fn create_request_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        r: &LinkRequest,
        now: Timestamp,
    ) -> Result<(), SuiteStoreError> {
        lock_owner(tx, r.user_id).await?;
        sqlx::query(
            r#"UPDATE suite_link_requests SET consumed_at=$3, initiator_link_id=NULL,
            secret_ciphertext=NULL, secret_nonce=NULL, secret_key_version=NULL, secret_record_id=NULL WHERE user_id=$1
            AND peer_app=$2
            AND consumed_at IS NULL"#,
        )
        .bind(r.user_id)
        .bind(&r.peer_app)
        .bind(now.to_sqlx())
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
        sqlx::query(
            r#"INSERT INTO suite_link_requests(id,
            user_id,
            peer_app,
            state_hash,
            verifier_ciphertext,
            verifier_nonce,
            verifier_key_version,
            verifier_record_id,
            client_state_nonce,
            return_url,
            expires_at,
            created_at)
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)"#,
        )
        .bind(r.id)
        .bind(r.user_id)
        .bind(&r.peer_app)
        .bind(r.state_hash.as_slice())
        .bind(&r.verifier.ciphertext)
        .bind(&r.verifier.nonce)
        .bind(r.verifier.key_version)
        .bind(r.verifier.record_id)
        .bind(&r.client_state_nonce)
        .bind(&r.return_url)
        .bind(r.expires_at.to_sqlx())
        .bind(r.created_at.to_sqlx())
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
        Ok(())
    }
    async fn find_request_by_state_hash(
        &self,
        hash: [u8; 32],
    ) -> Result<Option<LinkRequest>, SuiteStoreError> {
        sqlx::query_as::<_, RequestRow>(r#"SELECT id, user_id, peer_app, state_hash, verifier_ciphertext, verifier_nonce,
            verifier_key_version, verifier_record_id, client_state_nonce, return_url, link_id, initiator_link_id,
            secret_ciphertext, secret_nonce, secret_key_version, secret_record_id, expires_at, consumed_at, created_at FROM suite_link_requests WHERE state_hash=$1"#)
.bind(hash.as_slice())
            .fetch_optional(&self.pool)
            .await
            .map_err(storage)?
            .map(TryInto::try_into)
            .transpose()
    }
    async fn request_for_owner_for_update(
        &self,
        tx: &mut sqlx::PgConnection,
        id: Uuid,
        owner: Uuid,
    ) -> Result<Option<LinkRequest>, SuiteStoreError> {
        lock_owner(tx, owner).await?;
        sqlx::query_as::<_, RequestRow>(r#"SELECT id, user_id, peer_app, state_hash, verifier_ciphertext, verifier_nonce,
            verifier_key_version, verifier_record_id, client_state_nonce, return_url, link_id, initiator_link_id,
            secret_ciphertext, secret_nonce, secret_key_version, secret_record_id, expires_at, consumed_at, created_at FROM suite_link_requests WHERE id=$1 AND user_id=$2 FOR UPDATE"#)
.bind(id)
.bind(owner)
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage)?
        .map(TryInto::try_into)
        .transpose()
    }
    async fn prepare_exchange_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        id: Uuid,
        owner: Uuid,
        exchange: &PreparedExchange,
    ) -> Result<(), SuiteStoreError> {
        let count = sqlx::query(r#"UPDATE suite_link_requests SET initiator_link_id=$3, secret_ciphertext=$4, secret_nonce=$5, secret_key_version=$6, secret_record_id=$7 WHERE id=$1 AND user_id=$2 AND consumed_at IS NULL AND initiator_link_id IS NULL"#)
.bind(id)
.bind(owner)
.bind(exchange.link_id)
.bind(&exchange.secret.ciphertext)
.bind(&exchange.secret.nonce)
.bind(exchange.secret.key_version)
.bind(exchange.secret.record_id)
            .execute(&mut *tx).await.map_err(storage)?.rows_affected();
        if count != 1 {
            return Err(SuiteStoreError::CodeInvalid);
        }
        Ok(())
    }
    async fn replay_retry_after(
        &self,
        owner: Uuid,
        id: Uuid,
        now: Timestamp,
    ) -> Result<u64, SuiteStoreError> {
        let last: Option<SqlTimestamp> = sqlx::query_scalar(
            r#"SELECT last_replay_at FROM suite_links WHERE id=$1 AND user_id=$2"#,
        )
        .bind(id)
        .bind(owner)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?
        .ok_or(SuiteStoreError::NotFound)?;
        Ok(replay_retry_after(last.map(SqlTimestamp::to_jiff), now))
    }
    async fn consume_request_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        id: Uuid,
        owner: Uuid,
        now: Timestamp,
    ) -> Result<(), SuiteStoreError> {
        let n = sqlx::query(
            r#"UPDATE suite_link_requests
            SET consumed_at=$3, initiator_link_id=NULL, secret_ciphertext=NULL,
                secret_nonce=NULL, secret_key_version=NULL, secret_record_id=NULL
            WHERE id=$1
            AND user_id=$2
            AND consumed_at IS NULL
            AND expires_at>$3
            AND link_id IS NULL"#,
        )
        .bind(id)
        .bind(owner)
        .bind(now.to_sqlx())
        .execute(&mut *tx)
        .await
        .map_err(storage)?
        .rows_affected();
        if n != 1 {
            return Err(SuiteStoreError::CodeInvalid);
        }
        Ok(())
    }
    async fn record_request_link_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        id: Uuid,
        owner: Uuid,
        link: Uuid,
    ) -> Result<(), SuiteStoreError> {
        let n = sqlx::query(
            r#"UPDATE suite_link_requests
            SET link_id=$3
            WHERE id=$1
            AND user_id=$2
            AND consumed_at IS NOT NULL
            AND link_id IS NULL"#,
        )
        .bind(id)
        .bind(owner)
        .bind(link)
        .execute(&mut *tx)
        .await
        .map_err(storage)?
        .rows_affected();
        if n != 1 {
            return Err(SuiteStoreError::CodeInvalid);
        }
        Ok(())
    }
    async fn create_code_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        c: &LinkCode,
    ) -> Result<(), SuiteStoreError> {
        sqlx::query(
            r#"INSERT INTO suite_link_codes(code_hash,
            user_id,
            peer_app,
            code_challenge,
            suite_subject,
            auto_approved,
            expires_at,
            created_at)
            VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
        )
        .bind(c.code_hash.as_slice())
        .bind(c.user_id)
        .bind(&c.peer_app)
        .bind(&c.code_challenge)
        .bind(c.suite_subject.as_deref())
        .bind(c.auto_approved)
        .bind(c.expires_at.to_sqlx())
        .bind(c.created_at.to_sqlx())
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
        Ok(())
    }
    async fn code_for_update(
        &self,
        tx: &mut sqlx::PgConnection,
        hash: [u8; 32],
    ) -> Result<Option<LinkCode>, SuiteStoreError> {
        let owner: Option<Uuid> =
            sqlx::query_scalar("SELECT user_id FROM suite_link_codes WHERE code_hash=$1")
                .bind(hash.as_slice())
                .fetch_optional(&mut *tx)
                .await
                .map_err(storage)?;
        if let Some(owner) = owner {
            lock_owner(tx, owner).await?;
        }
        sqlx::query_as::<_, CodeRow>(r#"SELECT code_hash, user_id, peer_app, code_challenge, suite_subject, auto_approved, link_id, expires_at, consumed_at, created_at FROM suite_link_codes WHERE code_hash=$1 FOR UPDATE"#)
.bind(hash.as_slice())
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage)?
        .map(TryInto::try_into)
        .transpose()
    }
    async fn link_for_code_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        hash: [u8; 32],
    ) -> Result<Option<SuiteLink>, SuiteStoreError> {
        sqlx::query_as::<_, LinkRow>(r#"SELECT l.id, l.user_id, l.peer_app, l.role, l.remote_link_id, l.remote_subject, l.remote_display_name, l.suite_subject, l.status, l.secret_ciphertext, l.secret_nonce, l.secret_key_version, l.secret_record_id, l.sends, l.receives, l.share_xp, l.reward_mode, l.delivery_health, l.consecutive_failures, l.next_probe_at, l.failed_probes, l.last_delivery_at, l.last_failure_at, l.last_failure_code, l.last_received_at, l.created_at, l.updated_at, l.revoked_at FROM suite_links l JOIN suite_link_codes c ON c.link_id=l.id WHERE c.code_hash=$1"#)
.bind(hash.as_slice())
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage)?.map(TryInto::try_into).transpose()
    }
    async fn consume_code_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        hash: [u8; 32],
        link: Uuid,
        now: Timestamp,
    ) -> Result<(), SuiteStoreError> {
        let n = sqlx::query(
            r#"UPDATE suite_link_codes
            SET consumed_at=$3,link_id=$2
            WHERE code_hash=$1
            AND consumed_at IS NULL
            AND expires_at>$3
            AND link_id IS NULL"#,
        )
        .bind(hash.as_slice())
        .bind(link)
        .bind(now.to_sqlx())
        .execute(&mut *tx)
        .await
        .map_err(storage)?
        .rows_affected();
        if n != 1 {
            return Err(SuiteStoreError::CodeInvalid);
        }
        Ok(())
    }
    async fn lock_ingest_user(
        &self,
        tx: &mut sqlx::PgConnection,
        user_id: Uuid,
    ) -> Result<(), SuiteStoreError> {
        lock_owner(tx, user_id).await?;
        Ok(())
    }
    async fn link_for_update(
        &self,
        tx: &mut sqlx::PgConnection,
        owner: Uuid,
        id: Uuid,
    ) -> Result<SuiteLink, SuiteStoreError> {
        lock_owner(tx, owner).await?;
        lock_link_row(tx, owner, id).await
    }
    async fn active_link_for_update(
        &self,
        tx: &mut sqlx::PgConnection,
        owner: Uuid,
        peer: &str,
    ) -> Result<Option<SuiteLink>, SuiteStoreError> {
        lock_owner(tx, owner).await?;
        sqlx::query_as::<_, LinkRow>(r#"SELECT id, user_id, peer_app, role, remote_link_id, remote_subject,
            remote_display_name, suite_subject, status, secret_ciphertext, secret_nonce,
            secret_key_version, secret_record_id, sends, receives, share_xp, reward_mode, delivery_health,
            consecutive_failures, next_probe_at, failed_probes, last_delivery_at, last_failure_at, last_failure_code,
            last_received_at, created_at, updated_at, revoked_at FROM suite_links WHERE user_id=$1 AND peer_app=$2 AND status<>'revoked' ORDER BY id FOR UPDATE"#)
.bind(owner)
.bind(peer)
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage)?.map(TryInto::try_into).transpose()
    }
    async fn insert_link_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        l: &SuiteLink,
    ) -> Result<(), SuiteStoreError> {
        lock_owner(tx, l.user_id).await?;
        sqlx::query(
            r#"INSERT INTO suite_links(id,
            user_id,
            peer_app,
            role,
            remote_link_id,
            remote_subject,
            remote_display_name,
            suite_subject,
            status,
            secret_ciphertext,
            secret_nonce,
            secret_key_version,
            secret_record_id,
            sends,
            receives,
            share_xp,
            reward_mode,
            delivery_health,
            created_at,
            updated_at)
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20)"#,
        )
        .bind(l.id)
        .bind(l.user_id)
        .bind(&l.peer_app)
        .bind(l.role.as_str())
        .bind(l.remote_link_id)
        .bind(&l.remote_subject)
        .bind(l.remote_display_name.as_deref())
        .bind(l.suite_subject.as_deref())
        .bind(l.status.as_str())
        .bind(&l.secret.ciphertext)
        .bind(&l.secret.nonce)
        .bind(l.secret.key_version)
        .bind(l.secret.record_id)
        .bind(&l.sends)
        .bind(&l.receives)
        .bind(l.share_xp)
        .bind(l.reward_mode.as_str())
        .bind(l.delivery_health.as_str())
        .bind(l.created_at.to_sqlx())
        .bind(l.updated_at.to_sqlx())
        .execute(&mut *tx)
        .await
        .map_err(storage)?;
        Ok(())
    }
    async fn revoke_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        owner: Uuid,
        id: Uuid,
        now: Timestamp,
    ) -> Result<(), SuiteStoreError> {
        self.link_for_update(tx, owner, id).await?;
        changed(sqlx::query(r#"UPDATE suite_links SET status='revoked',revoked_at=COALESCE(revoked_at,$2),updated_at=$2 WHERE id=$1 AND user_id=$3"#)
.bind(id)
.bind(now.to_sqlx())
.bind(owner)
        .execute(&mut *tx)
        .await
        .map_err(storage)?.rows_affected())
    }
    async fn find_link(&self, id: Uuid) -> Result<Option<SuiteLink>, SuiteStoreError> {
        sqlx::query_as::<_, LinkRow>(
            r#"SELECT id, user_id, peer_app, role, remote_link_id, remote_subject,
            remote_display_name, suite_subject, status, secret_ciphertext, secret_nonce,
            secret_key_version, secret_record_id, sends, receives, share_xp, reward_mode, delivery_health,
            consecutive_failures, next_probe_at, failed_probes, last_delivery_at, last_failure_at, last_failure_code,
            last_received_at, created_at, updated_at, revoked_at FROM suite_links WHERE id=$1"#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?
        .map(TryInto::try_into)
        .transpose()
    }
    async fn find_link_for_owner(
        &self,
        id: Uuid,
        owner: Uuid,
    ) -> Result<Option<SuiteLink>, SuiteStoreError> {
        let owned: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM suite_links WHERE id=$1 AND user_id=$2)",
        )
        .bind(id)
        .bind(owner)
        .fetch_one(&self.pool)
        .await
        .map_err(storage)?;
        if !owned {
            return Ok(None);
        }
        sqlx::query_as::<_, LinkRow>(
            r#"SELECT id, user_id, peer_app, role, remote_link_id, remote_subject,
            remote_display_name, suite_subject, status, secret_ciphertext, secret_nonce,
            secret_key_version, secret_record_id, sends, receives, share_xp, reward_mode, delivery_health,
            consecutive_failures, next_probe_at, failed_probes, last_delivery_at, last_failure_at, last_failure_code,
            last_received_at, created_at, updated_at, revoked_at FROM suite_links WHERE id=$1 AND user_id=$2"#,
        )
        .bind(id)
        .bind(owner)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?
        .map(TryInto::try_into)
        .transpose()
    }
    async fn list_links(&self, owner: Uuid) -> Result<Vec<SuiteLink>, SuiteStoreError> {
        sqlx::query_as::<_, LinkRow>(r#"SELECT id, user_id, peer_app, role, remote_link_id, remote_subject,
            remote_display_name, suite_subject, status, secret_ciphertext, secret_nonce,
            secret_key_version, secret_record_id, sends, receives, share_xp, reward_mode, delivery_health,
            consecutive_failures, next_probe_at, failed_probes, last_delivery_at, last_failure_at, last_failure_code,
            last_received_at, created_at, updated_at, revoked_at FROM suite_links WHERE user_id=$1 AND status<>'revoked' ORDER BY created_at,id"#)
.bind(owner)
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?.into_iter()
        .map(TryInto::try_into)
        .collect()
    }
    async fn update_preferences(
        &self,
        owner: Uuid,
        id: Uuid,
        xp: Option<bool>,
        mode: Option<RewardMode>,
        now: Timestamp,
    ) -> Result<SuiteLink, SuiteStoreError> {
        let mut tx = self.begin_transaction().await?;
        self.link_for_update(&mut tx, owner, id).await?;
        let row = sqlx::query_as::<_, LinkRow>(
            r#"UPDATE suite_links
            SET share_xp=COALESCE($3,share_xp),reward_mode=COALESCE($4,reward_mode),updated_at=$5
            WHERE id=$1
            AND user_id=$2
            AND status<>'revoked'
            RETURNING id, user_id, peer_app, role, remote_link_id, remote_subject,
            remote_display_name, suite_subject, status, secret_ciphertext, secret_nonce,
            secret_key_version, secret_record_id, sends, receives, share_xp, reward_mode, delivery_health,
            consecutive_failures, next_probe_at, failed_probes, last_delivery_at, last_failure_at, last_failure_code,
            last_received_at, created_at, updated_at, revoked_at"#,
        )
        .bind(id)
        .bind(owner)
        .bind(xp)
        .bind(mode.map(RewardMode::as_str))
        .bind(now.to_sqlx())
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage)?
        .ok_or(SuiteStoreError::NotFound)?
        .try_into()?;
        self.commit_transaction(tx).await?;
        Ok(row)
    }
    async fn reenable(
        &self,
        owner: Uuid,
        id: Uuid,
        now: Timestamp,
    ) -> Result<SuiteLink, SuiteStoreError> {
        let mut tx = self.begin_transaction().await?;
        self.link_for_update(&mut tx, owner, id).await?;
        let row = sqlx::query_as::<_, LinkRow>(r#"UPDATE suite_links
            SET delivery_health='healthy',consecutive_failures=0,next_probe_at=NULL,failed_probes=0,last_failure_code=NULL,updated_at=$3
            WHERE id=$1
            AND user_id=$2
            AND status='active'
            RETURNING id, user_id, peer_app, role, remote_link_id, remote_subject,
            remote_display_name, suite_subject, status, secret_ciphertext, secret_nonce,
            secret_key_version, secret_record_id, sends, receives, share_xp, reward_mode, delivery_health,
            consecutive_failures, next_probe_at, failed_probes, last_delivery_at, last_failure_at, last_failure_code,
            last_received_at, created_at, updated_at, revoked_at"#)
.bind(id)
.bind(owner)
.bind(now.to_sqlx())
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage)?.ok_or(SuiteStoreError::NotFound)?.try_into()?;
        self.commit_transaction(tx).await?;
        Ok(row)
    }
    async fn record_delivery(
        &self,
        owner: Uuid,
        id: Uuid,
        action: DeliveryAction,
        now: Timestamp,
    ) -> Result<DeliveryState, SuiteStoreError> {
        self.health(owner, id, action, now).await
    }
    async fn claim_probe(
        &self,
        owner: Uuid,
        id: Uuid,
        now: Timestamp,
    ) -> Result<bool, SuiteStoreError> {
        probe::claim(self, owner, id, now).await
    }
    async fn complete_delivery(
        &self,
        owner: Uuid,
        id: Uuid,
        lease: &DeliveryLease,
        now: Timestamp,
    ) -> Result<(), SuiteStoreError> {
        let mut tx = self.begin_transaction().await?;
        let link = self.link_for_update(&mut tx, owner, id).await?;
        let state = after_delivery(DeliveryState::from(&link), DeliveryAction::Delivered, now);
        changed(
            sqlx::query(
                r#"UPDATE suite_links SET delivery_health=$2,
            consecutive_failures=$3, next_probe_at=NULL, failed_probes=0,
            last_delivery_at=$4,
            updated_at=$4 WHERE id=$1 AND user_id=$5"#,
            )
            .bind(id)
            .bind(state.health.as_str())
            .bind(i32::try_from(state.consecutive_failures).map_err(invalid)?)
            .bind(now.to_sqlx())
            .bind(owner)
            .execute(&mut *tx)
            .await
            .map_err(storage)?
            .rows_affected(),
        )?;
        if !baukit_jobs::PostgresJobStore::new(self.pool.clone())
            .complete_lease_in_transaction(
                &mut tx,
                &baukit_jobs::JobLease::new(lease.job_id, lease.worker_id.clone(), lease.attempt),
                now,
            )
            .await
            .map_err(storage)?
        {
            return Err(storage("suite delivery lease was lost"));
        }
        self.commit_transaction(tx).await
    }
    async fn inbox_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        id: Uuid,
        event: InboxIdentity<'_>,
    ) -> Result<InboxLookup, SuiteStoreError> {
        let row = sqlx::query_as::<_, InboxRow>(
            r#"SELECT payload_hash,
            event_type,
            occurred_at,
            outcome,
            ledger_entry_id FROM suite_inbound_events WHERE link_id=$1
            AND event_id=$2"#,
        )
        .bind(id)
        .bind(event.event_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage)?;
        let Some(row) = row else {
            return Ok(InboxLookup::New);
        };
        if row.payload_hash != event.payload_hash
            || row.event_type != event.event_type
            || !same_microsecond(row.occurred_at.to_jiff(), event.occurred_at)
        {
            return Err(SuiteStoreError::EventIdConflict);
        }
        Ok(InboxLookup::Duplicate(AppliedOutcome {
            outcome: match row.outcome.as_str() {
                "granted" => AppliedOutcomeStatus::Granted,
                "no_rule" => AppliedOutcomeStatus::NoRule,
                "capped" => AppliedOutcomeStatus::Capped,
                _ => return Err(invalid("invalid inbox outcome")),
            },
            ledger_entry_id: row.ledger_entry_id,
        }))
    }
    async fn record_inbound_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        e: &SuiteInboundEvent,
    ) -> Result<(), SuiteStoreError> {
        let written = sqlx::query(
            r#"INSERT INTO suite_inbound_events(link_id,
            event_id,
            user_id,
            event_type,
            occurred_at,
            payload_hash,
            replay,
            outcome,
            ledger_entry_id,
            received_at)
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
            ON CONFLICT (link_id,event_id) DO UPDATE
            SET outcome=EXCLUDED.outcome,ledger_entry_id=EXCLUDED.ledger_entry_id
            WHERE suite_inbound_events.payload_hash=EXCLUDED.payload_hash"#,
        )
        .bind(e.link_id)
        .bind(&e.event_id)
        .bind(e.user_id)
        .bind(&e.event_type)
        .bind(e.occurred_at.to_sqlx())
        .bind(e.payload_hash.as_slice())
        .bind(e.replay)
        .bind(match e.outcome.outcome {
            AppliedOutcomeStatus::Granted => "granted",
            AppliedOutcomeStatus::NoRule => "no_rule",
            AppliedOutcomeStatus::Capped => "capped",
        })
        .bind(e.outcome.ledger_entry_id.as_deref())
        .bind(e.received_at.to_sqlx())
        .execute(&mut *tx)
        .await
        .map_err(storage)?
        .rows_affected();
        if written != 1 {
            return Err(SuiteStoreError::EventIdConflict);
        }
        self.record_received_in_transaction(tx, e.user_id, e.link_id, e.received_at)
            .await
    }
    async fn record_received_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        owner: Uuid,
        id: Uuid,
        now: Timestamp,
    ) -> Result<(), SuiteStoreError> {
        changed(
            sqlx::query(
                r#"UPDATE suite_links SET last_received_at=$2,updated_at=$2 WHERE id=$1 AND user_id=$3"#,
            )
                .bind(id)
                .bind(now.to_sqlx())
                .bind(owner)
                .execute(&mut *tx)
                .await
                .map_err(storage)?
                .rows_affected(),
        )
    }
    async fn deliveries(
        &self,
        owner: Uuid,
        id: Uuid,
        limit: u32,
    ) -> Result<Vec<SuiteDeliveryRecord>, SuiteStoreError> {
        self.find_link_for_owner(id, owner)
            .await?
            .ok_or(SuiteStoreError::NotFound)?;
        sqlx::query_as::<_, DeliveryRow>(
            r#"SELECT id,
            payload->'envelope'->>'type' AS event_type,
            status,
            attempts AS attempt_count,
            last_error,
            created_at,
            updated_at
            FROM job_outbox
            WHERE job_type='suite.events.deliver'
            AND payload->>'link_id'=$1
            ORDER BY created_at DESC,id DESC LIMIT $2"#,
        )
        .bind(id.to_string())
        .bind(i64::from(limit.min(20)))
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?
        .into_iter()
        .map(|r| {
            Ok(SuiteDeliveryRecord {
                job_id: r.id,
                link_id: id,
                event_type: r.event_type,
                status: r.status,
                attempt_count: r.attempt_count.try_into().map_err(invalid)?,
                last_error_code: r.last_error,
                created_at: r.created_at.to_jiff(),
                updated_at: r.updated_at.to_jiff(),
            })
        })
        .collect()
    }
    async fn cleanup(&self, now: Timestamp) -> Result<SuiteCleanupOutcome, SuiteStoreError> {
        let mut tx = self.begin_transaction().await?;
        let outcome = cleanup_in_transaction(&mut tx, now).await?;
        self.commit_transaction(tx).await?;
        Ok(outcome)
    }
    async fn legacy_secrets_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        kind: LegacySecretKind,
        after: Uuid,
        limit: u32,
    ) -> Result<Vec<LegacySecret>, SuiteStoreError> {
        reseal::lock_legacy_secrets(tx, kind, after, limit).await
    }
    async fn reseal_secret_in_transaction(
        &self,
        tx: &mut sqlx::PgConnection,
        kind: LegacySecretKind,
        row_id: Uuid,
        sealed: &EncryptedPayload,
    ) -> Result<bool, SuiteStoreError> {
        reseal::store_resealed_secret(tx, kind, row_id, sealed).await
    }
}

pub(crate) async fn links_for_erasure(
    pool: &PgPool,
    owner: Uuid,
) -> Result<Vec<SuiteLink>, sqlx::Error> {
    let rows = sqlx::query_as::<_, LinkRow>(
        r#"SELECT id, user_id, peer_app, role, remote_link_id, remote_subject,
            remote_display_name, suite_subject, status, secret_ciphertext, secret_nonce,
            secret_key_version, secret_record_id, sends, receives, share_xp, reward_mode, delivery_health,
            consecutive_failures, next_probe_at, failed_probes, last_delivery_at, last_failure_at, last_failure_code,
            last_received_at, created_at, updated_at, revoked_at FROM suite_links
            WHERE user_id=$1 AND (status IN ('active','needs_attention') OR (status='revoked'
            AND EXISTS(SELECT 1 FROM job_outbox WHERE job_type='suite.links.revoke'
            AND status IN ('pending','running') AND payload->>'link_id'=suite_links.id::text)))
            ORDER BY id"#,
    )
    .bind(owner)
    .fetch_all(pool)
    .await?;
    let mut links = Vec::new();
    for row in rows {
        match row.try_into() {
            Ok(link) => links.push(link),
            Err(error) => {
                tracing::warn!(code="internal_error", %error, "Skipping corrupt suite erasure revoke")
            }
        }
    }
    Ok(links)
}

async fn lock_owner(tx: &mut sqlx::PgConnection, owner: Uuid) -> Result<(), SuiteStoreError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("baukit_suite.owner:{owner}"))
        .execute(tx)
        .await
        .map_err(storage)?;
    Ok(())
}

async fn try_lock_owner(tx: &mut sqlx::PgConnection, owner: Uuid) -> Result<bool, SuiteStoreError> {
    sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("baukit_suite.owner:{owner}"))
        .fetch_one(tx)
        .await
        .map_err(storage)
}

async fn lock_link_row(
    tx: &mut sqlx::PgConnection,
    owner: Uuid,
    id: Uuid,
) -> Result<SuiteLink, SuiteStoreError> {
    sqlx::query_as::<_, LinkRow>("SELECT * FROM suite_links WHERE id=$1 AND user_id=$2 FOR UPDATE")
        .bind(id)
        .bind(owner)
        .fetch_optional(tx)
        .await
        .map_err(storage)?
        .ok_or(SuiteStoreError::NotFound)?
        .try_into()
}
