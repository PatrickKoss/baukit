use std::sync::Arc;

use crate::domain::*;
use crate::ports::*;
use async_trait::async_trait;
use baukit_jobs::{NewJob, PostgresJobStore};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::{invalid, replay_retry_after, rows::LinkRow, storage};

#[derive(Clone, Copy, Eq, PartialEq)]
enum EmissionKind {
    Live,
    Replay,
    Test,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EmissionSkipReason {
    Invalid,
    Timeout,
    Storage,
}
impl EmissionSkipReason {
    fn from_error(error: &SuiteStoreError) -> Self {
        match error {
            SuiteStoreError::InvalidData(_) => Self::Invalid,
            SuiteStoreError::Timeout => Self::Timeout,
            _ => Self::Storage,
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Self::Invalid => "invalid",
            Self::Timeout => "timeout",
            Self::Storage => "storage",
        }
    }
    fn code(self) -> &'static str {
        match self {
            Self::Invalid => "suite_payload_invalid",
            Self::Timeout => "suite_emission_timeout",
            Self::Storage => "suite_emission_storage_failed",
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("suite emission failed for {event_type}")]
pub struct SuiteEmissionError {
    event_type: String,
    #[source]
    error: SuiteStoreError,
}
impl SuiteEmissionError {
    fn new(event_type: &str, error: SuiteStoreError) -> Self {
        Self {
            event_type: event_type.to_owned(),
            error,
        }
    }
}

fn record_emission_skip(
    metric_prefix: &str,
    event_type: &str,
    error: &SuiteStoreError,
    kind: EmissionKind,
) {
    let reason = EmissionSkipReason::from_error(error);
    tracing::warn!(
        event_type,
        code = reason.code(),
        reason = reason.as_str(),
        "Skipping suite emission"
    );
    if kind == EmissionKind::Live {
        metrics::counter!(format!("{metric_prefix}_suite_emission_skipped_total"), "reason" => reason.as_str()).increment(1);
    }
}

#[derive(Clone)]
pub struct PostgresSuiteEventOutbox {
    catalog: Arc<PayloadCatalog>,
    identities: Arc<dyn SuiteIdentitySource>,
    metric_prefix: String,
    jobs: PostgresJobStore,
    registry: Arc<PeerRegistry>,
    share_xp: bool,
}
impl PostgresSuiteEventOutbox {
    pub fn new(
        pool: PgPool,
        registry: Arc<PeerRegistry>,
        catalog: Arc<PayloadCatalog>,
        identities: Arc<dyn SuiteIdentitySource>,
        share_xp: bool,
        metric_prefix: String,
    ) -> Self {
        metrics::describe_counter!(
            format!("{metric_prefix}_suite_emission_skipped_total"),
            "Suite emissions skipped by reason"
        );
        for reason in [
            EmissionSkipReason::Invalid,
            EmissionSkipReason::Timeout,
            EmissionSkipReason::Storage,
        ] {
            metrics::counter!(format!("{metric_prefix}_suite_emission_skipped_total"), "reason" => reason.as_str()).increment(0);
        }
        Self {
            jobs: PostgresJobStore::new(pool.clone()),
            catalog,
            identities,
            metric_prefix,
            registry,
            share_xp,
        }
    }
    pub fn enabled(&self) -> bool {
        !self.registry.standalone()
    }
    async fn enqueue_for_link(
        &self,
        tx: &mut PgConnection,
        link: &SuiteLink,
        events: &[SuiteEvent],
        replay: Option<DateTime<Utc>>,
        test: bool,
    ) -> Result<u64, SuiteEmissionError> {
        if !link.can_enqueue() || self.registry.active_peer(&link.peer_app).is_none() {
            return Ok(0);
        }
        let mut count = 0;
        let kind = if test {
            EmissionKind::Test
        } else if replay.is_some() {
            EmissionKind::Replay
        } else {
            EmissionKind::Live
        };
        for event in events {
            if !test && !link.sends.contains(&event.event_type) {
                continue;
            }
            if let Err(error) = self
                .catalog
                .validate(&event.event_type, event.payload.clone())
            {
                record_emission_skip(
                    &self.metric_prefix,
                    &event.event_type,
                    &invalid(error),
                    kind,
                );
                continue;
            }
            let envelope = event.envelope_for_link(
                &self
                    .identities
                    .identity_in_transaction(tx, link.user_id)
                    .await
                    .map_err(|error| SuiteEmissionError::new(&event.event_type, error))?
                    .subject,
                self.registry.own().id.as_str(),
                self.share_xp,
                link.share_xp,
            );
            let key = match replay {
                Some(started_at) => format!(
                    "{}:{}:replay:{}",
                    envelope.event_id,
                    link.id,
                    started_at.to_rfc3339()
                ),
                None => format!("{}:{}", envelope.event_id, link.id),
            };
            let job = NewJob::new(
                SUITE_EVENTS_DELIVER_JOB_TYPE,
                serde_json::to_value(SuiteDeliverJobV1::new(link.id, envelope, replay.is_some()))
                    .map_err(|error| SuiteEmissionError::new(&event.event_type, invalid(error)))?,
                SUITE_MAX_ATTEMPTS,
            )
            .idempotency_key(key);
            count += u64::from(
                self.jobs
                    .enqueue_in_transaction(tx, job)
                    .await
                    .map_err(|error| SuiteEmissionError::new(&event.event_type, storage(error)))?
                    .created,
            );
        }
        Ok(count)
    }
    async fn enqueue_events(
        &self,
        tx: &mut PgConnection,
        owner: Uuid,
        events: &[SuiteEvent],
    ) -> Result<u64, SuiteEmissionError> {
        if self.registry.standalone() || events.is_empty() {
            return Ok(0);
        }
        let links = sqlx::query_as::<_, LinkRow>(
            r#"SELECT id, user_id, peer_app, role, remote_link_id, remote_subject,
            remote_display_name, suite_subject, status, secret_ciphertext, secret_nonce,
            secret_key_version, sends, receives, share_xp, reward_mode, delivery_health,
            consecutive_failures, last_delivery_at, last_failure_at, last_failure_code,
            last_received_at, created_at, updated_at, revoked_at
            FROM suite_links
            WHERE user_id=$1
            AND status='active'
            AND delivery_health<>'disabled'
            AND consecutive_failures<20
            ORDER BY id"#,
        )
        .bind(owner)
        .fetch_all(&mut *tx)
        .await
        .map_err(|error| SuiteEmissionError::new("unknown", storage(error)))?;
        let mut count = 0;
        for row in links {
            count += self
                .enqueue_for_link(
                    tx,
                    &row.try_into()
                        .map_err(|error| SuiteEmissionError::new("unknown", error))?,
                    events,
                    None,
                    false,
                )
                .await?;
        }
        Ok(count)
    }
}
#[async_trait]
impl SuiteEventOutbox for PostgresSuiteEventOutbox {
    async fn enqueue_in_transaction(
        &self,
        tx: &mut PgConnection,
        owner: Uuid,
        events: &[SuiteEvent],
    ) -> Result<u64, SuiteStoreError> {
        self.enqueue_events(tx, owner, events)
            .await
            .map_err(|error| error.error)
    }
    async fn enqueue_revoke_in_transaction(
        &self,
        tx: &mut PgConnection,
        id: Uuid,
    ) -> Result<(), SuiteStoreError> {
        self.jobs
            .enqueue_in_transaction(
                tx,
                NewJob::new(
                    SUITE_LINKS_REVOKE_JOB_TYPE,
                    serde_json::to_value(SuiteRevokeJobV1::new(id)).map_err(invalid)?,
                    SUITE_MAX_ATTEMPTS,
                )
                .idempotency_key(id.to_string()),
            )
            .await
            .map_err(storage)?;
        Ok(())
    }
    async fn enqueue_replay_in_transaction(
        &self,
        tx: &mut PgConnection,
        owner: Uuid,
        id: Uuid,
        since: NaiveDate,
        now: DateTime<Utc>,
        events: &[SuiteEvent],
    ) -> Result<u64, SuiteStoreError> {
        validate_replay_since(since, now.date_naive()).map_err(invalid)?;
        let link: SuiteLink = sqlx::query_as::<_, LinkRow>(
            r#"SELECT id, user_id, peer_app, role, remote_link_id, remote_subject,
            remote_display_name, suite_subject, status, secret_ciphertext, secret_nonce,
            secret_key_version, sends, receives, share_xp, reward_mode, delivery_health,
            consecutive_failures, last_delivery_at, last_failure_at, last_failure_code,
            last_received_at, created_at, updated_at, revoked_at
            FROM suite_links
            WHERE id=$1
            AND user_id=$2 FOR UPDATE"#,
        )
        .bind(id)
        .bind(owner)
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage)?
        .ok_or(SuiteStoreError::NotFound)?
        .try_into()?;
        ensure_active(&link)?;
        let changed = sqlx::query(
            r#"UPDATE suite_links SET last_replay_at=$2 WHERE id=$1
            AND (last_replay_at IS NULL
            OR last_replay_at<=$3)"#,
        )
        .bind(id)
        .bind(now)
        .bind(now - Duration::seconds(SUITE_REPLAY_INTERVAL_SECONDS))
        .execute(&mut *tx)
        .await
        .map_err(storage)?
        .rows_affected();
        if changed != 1 {
            let last: DateTime<Utc> = sqlx::query_scalar::<_, Option<DateTime<Utc>>>(
                r#"SELECT last_replay_at FROM suite_links WHERE id=$1"#,
            )
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(storage)?
            .ok_or_else(|| invalid("replay time is missing"))?;
            return Err(SuiteStoreError::ReplayTooSoon(replay_retry_after(
                Some(last),
                now,
            )));
        }
        self.enqueue_for_link(tx, &link, events, Some(now), false)
            .await
            .map_err(|error| error.error)
    }
    async fn enqueue_test_in_transaction(
        &self,
        tx: &mut PgConnection,
        owner: Uuid,
        id: Uuid,
        event: &SuiteEvent,
    ) -> Result<(), SuiteStoreError> {
        let link: SuiteLink = sqlx::query_as::<_, LinkRow>(r#"SELECT id, user_id, peer_app, role, remote_link_id, remote_subject,
            remote_display_name, suite_subject, status, secret_ciphertext, secret_nonce,
            secret_key_version, sends, receives, share_xp, reward_mode, delivery_health,
            consecutive_failures, last_delivery_at, last_failure_at, last_failure_code,
            last_received_at, created_at, updated_at, revoked_at FROM suite_links WHERE id=$1 AND user_id=$2 FOR UPDATE"#)
.bind(id)
.bind(owner)
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage)?
        .ok_or(SuiteStoreError::NotFound)?
        .try_into()?;
        ensure_active(&link)?;
        if self.registry.active_peer(&link.peer_app).is_none() {
            return Err(SuiteStoreError::LinkInactive);
        }
        self.enqueue_for_link(tx, &link, std::slice::from_ref(event), None, true)
            .await
            .map_err(|error| error.error)?;
        Ok(())
    }
}
fn ensure_active(link: &SuiteLink) -> Result<(), SuiteStoreError> {
    if link.status == LinkStatus::Revoked {
        return Err(SuiteStoreError::LinkRevoked);
    }
    if !link.can_enqueue() || link.status != LinkStatus::Active {
        return Err(SuiteStoreError::LinkInactive);
    }
    Ok(())
}
