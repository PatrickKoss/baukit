use super::{PostgresSuiteLinkStore, invalid, storage};
use crate::domain::{DeliveryState, claim_probe};
use crate::ports::{SuiteLinkStore, SuiteStoreError};
use jiff::Timestamp;
use jiff_sqlx::ToSqlx;
use uuid::Uuid;

/// Claims the due probe of a disabled link. Only one caller wins; the claim
/// moves `next_probe_at` before the peer call.
pub(super) async fn claim(
    store: &PostgresSuiteLinkStore,
    owner: Uuid,
    id: Uuid,
    now: Timestamp,
) -> Result<bool, SuiteStoreError> {
    let mut tx = store.begin_transaction().await?;
    let link = store.link_for_update(&mut tx, owner, id).await?;
    if !link.probe_due(now) {
        return Ok(false);
    }
    let claimed = claim_probe(DeliveryState::from(&link), now);
    sqlx::query(
        "UPDATE suite_links SET next_probe_at=$3, failed_probes=$4, updated_at=$5 \
         WHERE id=$1 AND user_id=$2",
    )
    .bind(id)
    .bind(owner)
    .bind(claimed.next_probe_at.map(|at| at.to_sqlx()))
    .bind(i32::try_from(claimed.failed_probes).map_err(invalid)?)
    .bind(now.to_sqlx())
    .execute(&mut *tx)
    .await
    .map_err(storage)?;
    store.commit_transaction(tx).await?;
    Ok(true)
}
