use crate::ports::{LegacySecretKind, SuiteCleanupOutcome, SuiteLinkStore, SuiteStoreError};
use crate::secret_cipher::SecretCipher;
use uuid::Uuid;

/// Rows each re-seal transaction locks.
pub const SUITE_RESEAL_BATCH: u32 = 100;

/// Seals every v1 secret again as a vault v2 record, `batch` rows per transaction.
///
/// A secret that fails to open stays as it is and counts as unreadable. Rows that
/// another transaction holds are skipped and picked up by the next run.
pub(crate) async fn reseal_legacy_secrets(
    store: &dyn SuiteLinkStore,
    cipher: &SecretCipher,
    batch: u32,
) -> Result<SuiteCleanupOutcome, SuiteStoreError> {
    let mut outcome = SuiteCleanupOutcome::default();
    if !cipher.is_configured() {
        return Ok(outcome);
    }
    let batch = batch.max(1);
    for kind in LegacySecretKind::ALL {
        let mut after = Uuid::nil();
        loop {
            let mut tx = store.begin_transaction().await?;
            let rows = store
                .legacy_secrets_in_transaction(&mut tx, kind, after, batch)
                .await?;
            let Some(last) = rows.last() else {
                break;
            };
            after = last.row_id;
            for row in &rows {
                let sealed = cipher
                    .decrypt(row.row_id, kind.field(), &row.payload)
                    .and_then(|plaintext| cipher.encrypt(row.row_id, kind.field(), &plaintext));
                let Ok(sealed) = sealed else {
                    outcome.unreadable_secrets += 1;
                    continue;
                };
                if store
                    .reseal_secret_in_transaction(&mut tx, kind, row.row_id, &sealed)
                    .await?
                {
                    outcome.resealed_secrets += 1;
                }
            }
            store.commit_transaction(tx).await?;
            if rows.len() < batch as usize {
                break;
            }
        }
    }
    Ok(outcome)
}
