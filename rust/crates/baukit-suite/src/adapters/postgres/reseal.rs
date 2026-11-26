use super::storage;
use crate::domain::EncryptedPayload;
use crate::ports::{LegacySecret, LegacySecretKind, SuiteStoreError};
use uuid::Uuid;

type LegacyRow = (Uuid, Vec<u8>, Vec<u8>, i32);

const fn select_sql(kind: LegacySecretKind) -> &'static str {
    match kind {
        LegacySecretKind::LinkSecret => {
            "SELECT id, secret_ciphertext, secret_nonce, secret_key_version FROM suite_links
             WHERE secret_record_id IS NULL AND id>$1 ORDER BY id LIMIT $2 FOR UPDATE SKIP LOCKED"
        }
        LegacySecretKind::RequestVerifier => {
            "SELECT id, verifier_ciphertext, verifier_nonce, verifier_key_version
             FROM suite_link_requests
             WHERE verifier_record_id IS NULL AND consumed_at IS NULL AND id>$1
             ORDER BY id LIMIT $2 FOR UPDATE SKIP LOCKED"
        }
        LegacySecretKind::RequestSecret => {
            "SELECT id, secret_ciphertext, secret_nonce, secret_key_version
             FROM suite_link_requests
             WHERE secret_record_id IS NULL AND secret_ciphertext IS NOT NULL
             AND consumed_at IS NULL AND id>$1
             ORDER BY id LIMIT $2 FOR UPDATE SKIP LOCKED"
        }
    }
}

const fn update_sql(kind: LegacySecretKind) -> &'static str {
    match kind {
        LegacySecretKind::LinkSecret => {
            "UPDATE suite_links SET secret_ciphertext=$2, secret_nonce=$3, secret_key_version=$4,
             secret_record_id=$5 WHERE id=$1 AND secret_record_id IS NULL"
        }
        LegacySecretKind::RequestVerifier => {
            "UPDATE suite_link_requests SET verifier_ciphertext=$2, verifier_nonce=$3,
             verifier_key_version=$4, verifier_record_id=$5
             WHERE id=$1 AND verifier_record_id IS NULL AND consumed_at IS NULL"
        }
        LegacySecretKind::RequestSecret => {
            "UPDATE suite_link_requests SET secret_ciphertext=$2, secret_nonce=$3,
             secret_key_version=$4, secret_record_id=$5
             WHERE id=$1 AND secret_record_id IS NULL AND secret_ciphertext IS NOT NULL
             AND consumed_at IS NULL"
        }
    }
}

pub(super) async fn lock_legacy_secrets(
    tx: &mut sqlx::PgConnection,
    kind: LegacySecretKind,
    after: Uuid,
    limit: u32,
) -> Result<Vec<LegacySecret>, SuiteStoreError> {
    let rows: Vec<LegacyRow> = sqlx::query_as(select_sql(kind))
        .bind(after)
        .bind(i64::from(limit))
        .fetch_all(tx)
        .await
        .map_err(storage)?;
    Ok(rows
        .into_iter()
        .map(|(row_id, ciphertext, nonce, key_version)| LegacySecret {
            row_id,
            payload: EncryptedPayload {
                ciphertext,
                nonce,
                key_version,
                record_id: None,
            },
        })
        .collect())
}

pub(super) async fn store_resealed_secret(
    tx: &mut sqlx::PgConnection,
    kind: LegacySecretKind,
    row_id: Uuid,
    sealed: &EncryptedPayload,
) -> Result<bool, SuiteStoreError> {
    let Some(record_id) = sealed.record_id else {
        return Err(SuiteStoreError::InvalidData(
            "a resealed secret needs a record id".into(),
        ));
    };
    let updated = sqlx::query(update_sql(kind))
        .bind(row_id)
        .bind(&sealed.ciphertext)
        .bind(&sealed.nonce)
        .bind(sealed.key_version)
        .bind(record_id)
        .execute(tx)
        .await
        .map_err(storage)?
        .rows_affected();
    Ok(updated == 1)
}
