//! One-call wiring of the PostgreSQL suite adapters for a product backend.
//!
//! ```ignore
//! let settings = SuiteSettings::new(APP_ID, PEERS_JSON, CATALOG_JSON, &config.suite, cipher)?;
//! let suite = PostgresSuite::builder(pool.clone(), settings, limiter)
//!     .identity(Arc::new(ProductSuiteIdentitySource::new(pool.clone())))
//!     .replay(Arc::new(ProductSuiteReplaySource))
//!     .applier(Arc::new(ProductSuiteEventApplier))
//!     .erasure_lookup(
//!         Arc::new(ProductSuiteErasureOwnerLookup::new(pool.clone())),
//!         Arc::new(PostgresProfileErasure),
//!     )
//!     .build()?;
//! let repository = PostgresRepository::new(pool.clone(), suite.outbox.clone());
//! let routes = Router::new().nest("/api/v1", suite.router(suite.api()));
//! let worker = suite.worker(Arc::new(job_store))?;
//! ```

use std::sync::Arc;

use baukit_config::ValidationErrors;
use baukit_credential_vault::CredentialCipher;
use baukit_ratelimit::RateLimitStore;
use sqlx::PgPool;

use crate::{
    adapters::{
        peer::ReqwestSuitePeerClient,
        postgres::{
            PostgresSuiteErasure, PostgresSuiteEventOutbox, PostgresSuiteLinkStore,
            SuiteErasureOwnerLookup,
        },
    },
    config::SuiteConfig,
    domain::{PayloadCatalog, SuiteDataError},
    ports::{
        SuiteErasureLinks, SuiteEventApplier, SuiteIdentitySource, SuitePeerClient,
        SuiteReplaySource,
    },
    services::{
        SuiteContext, SuiteDeliveryService, SuiteDependencies, SuiteErasureNotificationService,
        SuiteIngestService, SuiteLinkService, SuiteModule, SuiteServiceConfig,
    },
};

/// Product catalog constants and the deployment's suite configuration.
pub struct SuiteSettings {
    pub app_id: String,
    pub peers_json: String,
    pub catalog: Arc<PayloadCatalog>,
    pub config: SuiteConfig,
    pub cipher: Option<CredentialCipher>,
}

impl SuiteSettings {
    /// Parses the payload catalog. `app_id` is also the metric prefix.
    ///
    /// # Errors
    ///
    /// `Data` when `catalog_json` is not a valid payload catalog.
    pub fn new(
        app_id: &str,
        peers_json: &str,
        catalog_json: &str,
        config: &SuiteConfig,
        cipher: Option<CredentialCipher>,
    ) -> Result<Self, SuiteComposeError> {
        let catalog = Arc::new(PayloadCatalog::from_json(catalog_json)?);
        Ok(Self::with_catalog(
            app_id, peers_json, catalog, config, cipher,
        ))
    }

    /// For products that build the catalog in code.
    pub fn with_catalog(
        app_id: &str,
        peers_json: &str,
        catalog: Arc<PayloadCatalog>,
        config: &SuiteConfig,
        cipher: Option<CredentialCipher>,
    ) -> Self {
        Self {
            app_id: app_id.to_owned(),
            peers_json: peers_json.to_owned(),
            catalog,
            config: config.clone(),
            cipher,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SuiteComposeError {
    #[error("suite configuration is invalid: {0}")]
    Config(#[from] ValidationErrors),
    #[error("suite catalog or peer registry is invalid: {0}")]
    Data(#[from] SuiteDataError),
    #[error("suite peer client could not be created: {0}")]
    Peer(String),
    #[error("suite public rate limit is invalid: {0}")]
    Quota(#[from] baukit_ratelimit::QuotaError),
    #[error("suite builder is missing {0}")]
    Missing(&'static str),
}

/// Fully wired suite services backed by PostgreSQL.
pub struct PostgresSuite {
    pub context: Arc<SuiteContext>,
    pub outbox: Arc<PostgresSuiteEventOutbox>,
    pub store: Arc<PostgresSuiteLinkStore>,
    pub delivery: Arc<SuiteDeliveryService>,
    /// Present when the builder received [`PostgresSuiteBuilder::erasure_lookup`].
    pub erasure: Option<Arc<PostgresSuiteErasure>>,
    applier: Option<Arc<dyn SuiteEventApplier>>,
    limiter: Arc<dyn RateLimitStore>,
    #[cfg(feature = "http")]
    public_limit: crate::adapters::public_limit::SuitePublicRateLimit,
}

#[must_use = "call build() to wire the suite"]
pub struct PostgresSuiteBuilder {
    pool: PgPool,
    settings: SuiteSettings,
    limiter: Arc<dyn RateLimitStore>,
    identity: Option<Arc<dyn SuiteIdentitySource>>,
    replay: Option<Arc<dyn SuiteReplaySource>>,
    applier: Option<Arc<dyn SuiteEventApplier>>,
    erasure: Option<(
        Arc<dyn SuiteErasureOwnerLookup>,
        Arc<dyn baukit_erasure::ProductErasure>,
    )>,
    peer: Option<Arc<dyn SuitePeerClient>>,
    share_xp: Option<bool>,
    #[cfg(feature = "http")]
    public_limit: Option<crate::adapters::public_limit::SuitePublicRateLimit>,
}

impl PostgresSuite {
    /// `limiter` backs the service quotas and the public route IP limit.
    pub fn builder(
        pool: PgPool,
        settings: SuiteSettings,
        limiter: Arc<dyn RateLimitStore>,
    ) -> PostgresSuiteBuilder {
        PostgresSuiteBuilder {
            pool,
            settings,
            limiter,
            identity: None,
            replay: None,
            applier: None,
            erasure: None,
            peer: None,
            share_xp: None,
            #[cfg(feature = "http")]
            public_limit: None,
        }
    }

    /// Protocol module with the builder's applier. Without one, inbound events are rejected.
    pub fn module(&self) -> SuiteModule {
        self.module_with_ingest(SuiteIngestService::with_optional_applier(
            self.context.clone(),
            self.applier.clone(),
        ))
    }

    /// Protocol module for an applier that needs the outbox, such as a repository-backed one.
    pub fn module_with(&self, applier: Arc<dyn SuiteEventApplier>) -> SuiteModule {
        self.module_with_ingest(SuiteIngestService::new(self.context.clone(), applier))
    }

    pub fn api(&self) -> Arc<dyn crate::services::SuiteApi> {
        Arc::new(self.module())
    }

    /// Erasure notifier that revokes the links of an erased subject at each peer.
    pub fn notifier(&self) -> Option<Arc<SuiteErasureNotificationService>> {
        self.erasure
            .clone()
            .map(|erasure| self.erasure_notifier(erasure))
    }

    /// Notifier over product-supplied erasure links.
    pub fn erasure_notifier(
        &self,
        links: Arc<dyn SuiteErasureLinks>,
    ) -> Arc<SuiteErasureNotificationService> {
        Arc::new(SuiteErasureNotificationService::new(
            links,
            self.delivery.clone(),
        ))
    }

    pub fn limiter(&self) -> Arc<dyn RateLimitStore> {
        self.limiter.clone()
    }

    fn module_with_ingest(&self, ingest: SuiteIngestService) -> SuiteModule {
        SuiteModule {
            links: SuiteLinkService::new(self.context.clone()),
            delivery: SuiteDeliveryService::new(self.context.clone()),
            ingest,
        }
    }
}

impl PostgresSuiteBuilder {
    pub fn identity(mut self, identity: Arc<dyn SuiteIdentitySource>) -> Self {
        self.identity = Some(identity);
        self
    }

    pub fn replay(mut self, replay: Arc<dyn SuiteReplaySource>) -> Self {
        self.replay = Some(replay);
        self
    }

    /// Inbound event applier. Use [`PostgresSuite::module_with`] when it needs the outbox.
    pub fn applier(mut self, applier: Arc<dyn SuiteEventApplier>) -> Self {
        self.applier = Some(applier);
        self
    }

    /// Wraps the product erasure so suite rows are deleted in the same transaction.
    pub fn erasure_lookup(
        mut self,
        owners: Arc<dyn SuiteErasureOwnerLookup>,
        product: Arc<dyn baukit_erasure::ProductErasure>,
    ) -> Self {
        self.erasure = Some((owners, product));
        self
    }

    /// Replaces the guarded reqwest client, for example with a test double.
    pub fn peer(mut self, peer: Arc<dyn SuitePeerClient>) -> Self {
        self.peer = Some(peer);
        self
    }

    /// Overrides `SuiteConfig::share_xp` for products that never emit XP.
    pub fn share_xp(mut self, share_xp: bool) -> Self {
        self.share_xp = Some(share_xp);
        self
    }

    #[cfg(feature = "http")]
    pub fn public_rate_limit(
        mut self,
        limit: crate::adapters::public_limit::SuitePublicRateLimit,
    ) -> Self {
        self.public_limit = Some(limit);
        self
    }

    /// Validates the configuration and requires a cipher whenever peers are configured.
    ///
    /// # Errors
    ///
    /// `Missing` without an identity or replay source, `Config` when the suite config, peer registry
    /// or cipher requirement fails validation, `Data` for an invalid registry, `Peer` when the default
    /// peer client cannot be built, and `Quota` when the default public limit is invalid.
    pub fn build(self) -> Result<PostgresSuite, SuiteComposeError> {
        let Self {
            pool,
            settings,
            limiter,
            identity,
            replay,
            applier,
            erasure,
            peer,
            share_xp,
            #[cfg(feature = "http")]
            public_limit,
        } = self;
        let identity = identity.ok_or(SuiteComposeError::Missing("identity"))?;
        let replay = replay.ok_or(SuiteComposeError::Missing("replay"))?;
        let config = &settings.config;
        config.validate_for(
            &settings.app_id,
            &settings.peers_json,
            settings.cipher.as_ref(),
        )?;
        let registry = Arc::new(config.registry(&settings.app_id, &settings.peers_json)?);
        let share_xp = share_xp.unwrap_or(config.share_xp);
        let peer = match peer {
            Some(peer) => peer,
            None => Arc::new(
                ReqwestSuitePeerClient::new(config.allow_loopback)
                    .map_err(|error| SuiteComposeError::Peer(error.to_string()))?,
            ),
        };
        let outbox = Arc::new(PostgresSuiteEventOutbox::new(
            pool.clone(),
            registry.clone(),
            settings.catalog.clone(),
            identity.clone(),
            share_xp,
            settings.app_id.clone(),
        ));
        let store = Arc::new(PostgresSuiteLinkStore::new(pool));
        let context = Arc::new(SuiteContext::new(
            registry,
            settings.catalog,
            SuiteDependencies {
                store: store.clone(),
                outbox: outbox.clone(),
                peer,
                limiter: limiter.clone(),
                replay,
                identities: identity,
            },
            SuiteServiceConfig {
                metric_prefix: settings.app_id,
                cipher: settings.cipher,
                initial_replay_days: config.initial_replay_days,
                share_xp,
            },
        ));
        let erasure = erasure.map(|(owners, product)| {
            Arc::new(PostgresSuiteErasure::new((*store).clone(), owners, product))
        });
        Ok(PostgresSuite {
            delivery: Arc::new(SuiteDeliveryService::new(context.clone())),
            context,
            outbox,
            store,
            erasure,
            applier,
            limiter,
            #[cfg(feature = "http")]
            public_limit: match public_limit {
                Some(limit) => limit,
                None => crate::adapters::public_limit::SuitePublicRateLimit::new()?,
            },
        })
    }
}

#[cfg(feature = "http")]
impl PostgresSuite {
    /// All suite routes; callback, exchange and peer revoke carry the per-IP limit.
    /// Nest it under the product API prefix behind the middleware that inserts `SuiteUser`.
    pub fn router(&self, api: Arc<dyn crate::services::SuiteApi>) -> axum::Router {
        crate::adapters::public_limit::limited_router(
            crate::adapters::http::SuiteHttpState { api },
            self.limiter.clone(),
            self.public_limit.clone(),
        )
    }
}

#[cfg(feature = "jobs")]
impl PostgresSuite {
    /// Delivery worker on `SUITE_QUEUE` with four slots and 30 s to 6 h retry backoff.
    ///
    /// # Errors
    ///
    /// Same as [`Self::worker_with`] with the default worker config.
    pub fn worker(
        &self,
        store: Arc<dyn baukit_jobs::JobStore>,
    ) -> Result<baukit_jobs::WorkerRunner, baukit_jobs::RunnerError> {
        self.worker_with(store, self.delivery.clone(), default_worker_config())
    }

    /// Worker for a product delivery runner that wraps [`PostgresSuite::delivery`].
    ///
    /// # Errors
    ///
    /// `RunnerError::InvalidConfig` when `config` is invalid for the suite job types.
    pub fn worker_with(
        &self,
        store: Arc<dyn baukit_jobs::JobStore>,
        delivery: Arc<dyn crate::services::SuiteDeliveryRunner>,
        config: baukit_jobs::WorkerConfig,
    ) -> Result<baukit_jobs::WorkerRunner, baukit_jobs::RunnerError> {
        baukit_jobs::WorkerRunner::new(
            store,
            Arc::new(crate::adapters::jobs::SuiteJobHandler::new(delivery)),
            config,
        )
    }
}

#[cfg(feature = "jobs")]
const WORKER_CONCURRENCY: usize = 4;
#[cfg(feature = "jobs")]
const RETRY_INITIAL: std::time::Duration = std::time::Duration::from_secs(30);
#[cfg(feature = "jobs")]
const RETRY_MAX: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);

#[cfg(feature = "jobs")]
pub fn default_worker_config() -> baukit_jobs::WorkerConfig {
    baukit_jobs::WorkerConfig {
        queue: crate::domain::SUITE_QUEUE,
        concurrency: WORKER_CONCURRENCY,
        retry_initial: RETRY_INITIAL,
        retry_max: RETRY_MAX,
        ..baukit_jobs::WorkerConfig::default()
    }
}
