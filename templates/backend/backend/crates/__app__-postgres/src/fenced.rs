use sqlx::{PgPool, Postgres, Transaction};

use {{ context.app_crate }}_ports::RepositoryError;

/// Begins a product write that an erasure of the requesting subject cannot race.
pub(crate) async fn begin_write(
    pool: &PgPool,
) -> Result<Transaction<'static, Postgres>, RepositoryError> {
{% if context.auth_enabled %}    baukit_erasure::begin_fenced_write(pool)
        .await
        .map_err(|error| match error {
            baukit_erasure::ErasureError::ProfileErased => RepositoryError::ProfileErased,
            error => RepositoryError::unavailable(error),
        })
{% else %}    baukit_ops::begin(pool)
        .await
        .map_err(RepositoryError::unavailable)
{% endif %}}
