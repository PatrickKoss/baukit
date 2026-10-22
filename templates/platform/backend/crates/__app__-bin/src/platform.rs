use std::sync::Arc;

use axum::Router;
use baukit_suite::platform::{
    EntitlementEventHandler, EntitlementSource, Entitlements, PlatformEventHandler,
    PostgresEntitlementCache, PostgresPlatformInbox,
};
use {{ context.app_crate }}_api::HubTicketSource;
use {{ context.app_crate }}_ports::PlatformEvents;
use {{ context.app_crate }}_postgres::ProductPlatformHandler;
use {{ context.app_crate }}_services::LoggingPlatformEvents;
use sqlx::PgPool;

use crate::ProductConfig;

const RECEIPT_RETENTION_DAYS: i64 = 30;
const PURGE_BATCH: u32 = 1000;

pub struct PlatformServices {
    pub router: Router,
    pub entitlements: Entitlements,
}

pub fn compose(
    pool: PgPool,
    config: &ProductConfig,
) -> Result<PlatformServices, Box<dyn std::error::Error>> {
    let source: Option<Arc<dyn EntitlementSource>> = if config.platform.standalone() {
        None
    } else {
        Some(Arc::new(HubTicketSource::new(
            &config.platform,
            &config.platform_hub,
        )?))
    };
    compose_with(pool, config, source, Arc::new(LoggingPlatformEvents))
}

/// Products supply their event port and may replace the ticket source with a refreshing credential adapter.
pub fn compose_with(
    pool: PgPool,
    config: &ProductConfig,
    source: Option<Arc<dyn EntitlementSource>>,
    events: Arc<dyn PlatformEvents>,
) -> Result<PlatformServices, Box<dyn std::error::Error>> {
    let cache = Arc::new(PostgresEntitlementCache::new(pool.clone()));
    let inbox = Arc::new(PostgresPlatformInbox::new(pool));
    let entitlements = config
        .platform
        .entitlements(cache.clone(), source.clone())?;
    let handler: Arc<dyn PlatformEventHandler> = Arc::new(ProductPlatformHandler(events));
    let handler = if let Some(source) = source {
        Arc::new(EntitlementEventHandler::new(
            config.platform.product_id.clone(),
            cache,
            source,
            handler,
        )) as Arc<dyn PlatformEventHandler>
    } else {
        handler
    };
    let router = match config.platform.receiver(inbox, handler)? {
        Some(receiver) => baukit_suite::platform::router(receiver),
        None => Router::new(),
    };
    Ok(PlatformServices {
        router,
        entitlements,
    })
}

/// Daily scheduler hook. The cutoff keeps at least 30 days of delivery receipts.
pub async fn purge_receipts(pool: PgPool) -> Result<u64, baukit_suite::platform::PlatformError> {
    let inbox = PostgresPlatformInbox::new(pool);
    let cutoff = chrono::Utc::now() - chrono::TimeDelta::days(RECEIPT_RETENTION_DAYS);
    let mut total = 0;
    loop {
        let removed = inbox.purge_received_before(cutoff, PURGE_BATCH).await?;
        total += removed;
        if removed == 0 {
            return Ok(total);
        }
    }
}
