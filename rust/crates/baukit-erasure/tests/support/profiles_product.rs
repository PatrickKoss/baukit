use baukit_erasure::{ErasureFuture, ProductErasure};
use sqlx::PgConnection;

pub struct Product;
impl ProductErasure for Product {
    fn erase<'a>(
        &'a self,
        connection: &'a mut PgConnection,
        subject: &'a str,
    ) -> ErasureFuture<'a, Result<(), sqlx::Error>> {
        Box::pin(async move {
            sqlx::query("DELETE FROM profiles WHERE subject = $1")
                .bind(subject)
                .execute(connection)
                .await?;
            Ok(())
        })
    }
}
