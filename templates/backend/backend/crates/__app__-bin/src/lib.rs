use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};

use axum::Router;
use baukit_config::{Validate{% if context.auth_enabled or context.worker %}, ValidationError{% endif %}, ValidationErrors};
{% if context.worker %}use baukit_jobs::WorkerRunner;
{% endif %}use baukit_ops::{
    OpsRouter, PrometheusHandle, ReadinessError, ReadinessRegistry, RegistrationError,
    ServiceIdentity, TrafficGate,
};

{% if context.auth_enabled %}use {{ context.app_crate }}_domain::InternalUser;
use {{ context.app_crate }}_domain::Item;
{% else %}use {{ context.app_crate }}_domain::Item;
{% endif %}use {{ context.app_crate }}_ports::ItemRepository;
use {{ context.app_crate }}_ports::PortFuture;
use {{ context.app_crate }}_ports::RepositoryError;
{% if context.auth_enabled %}use {{ context.app_crate }}_ports::UserRepository;
{% endif %}
use {{ context.app_crate }}_services::ItemService;

use serde::Deserialize;
use uuid::Uuid;

mod config;
pub use config::config_loader;

{% if context.auth_enabled %}mod identity;
{% if context.mcp %}pub use identity::{auth_verifier, mcp_policy, mcp_verifier};
{% else %}pub use identity::auth_verifier;
{% endif %}
const PRODUCT: &str = "{{ context.app_name }}";

{% endif %}#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
{% if context.auth_enabled or context.worker %}pub struct ProductConfig {
{% if context.auth_enabled %}    pub auth: AuthConfig,
{% if context.mcp %}    pub mcp: baukit_mcp::McpConfig,
{% endif %}{% endif %}{% if context.worker %}    pub worker: WorkerProductConfig,
{% endif %}}
{% else %}pub struct ProductConfig {}
{% endif %}
impl Validate for ProductConfig {
    fn validate(&self) -> Result<(), ValidationErrors> {
{% if context.auth_enabled or context.worker %}        let mut errors = Vec::new();
{% if context.auth_enabled %}        if let Err(auth) = self.auth.validate() {
            errors.extend(auth.into_errors());
        }
{% endif %}{% if context.worker %}        if let Err(worker) = self.worker.validate() {
            errors.extend(worker.into_errors());
        }
{% endif %}{% if context.mcp %}        if self.auth.provider != AuthProvider::Oidc
            && (self.mcp.introspection_client_id.is_some()
                || self.mcp.introspection_client_secret.is_some())
        {
            errors.push(ValidationError::new(
                "mcp.introspection_client_id",
                "Keycloak introspection requires the OIDC provider",
            ));
        }
        if self.mcp.enabled
            && self.auth.provider == AuthProvider::Clerk
            && self
                .mcp
                .oauth_client_id
                .as_deref()
                .is_none_or(|id| id.trim().is_empty())
        {
            errors.push(ValidationError::new(
                "mcp.oauth_client_id",
                "Clerk MCP requires a dedicated OAuth client ID",
            ));
        }
        if self.mcp.enabled
            && let Err(error) = self.mcp.validate()
        {
            errors.push(ValidationError::new("mcp", error.to_string()));
        }
{% endif %}        if errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationErrors::new(errors))
        }
{% else %}        Ok(())
{% endif %}    }
}

{% if context.auth_enabled %}#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AuthProvider {
    Oidc,
    Clerk,
    Workos,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct AuthConfig {
    pub provider: AuthProvider,
    pub client_id: String,
    pub publishable_key: String,
    pub authorized_parties: Vec<String>,
    pub jwks_uri: Option<String>,
    pub issuer: String,
    pub audience: String,
    pub identity_admin_base_url: String,
    pub identity_admin_realm: String,
    pub identity_admin_client_secret: Option<baukit_config::Secret<String>>,
    pub erasure_hash_key: Option<baukit_config::Secret<String>>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            provider: AuthProvider::{{ "Oidc" if context.auth_oidc else "Clerk" if context.auth_clerk else "Workos" }},
            client_id: String::new(),
            publishable_key: String::new(),
            authorized_parties: vec!["http://localhost:5173".into()],
            jwks_uri: None,
            issuer: {% if context.auth_oidc %}format!("http://localhost:{{ context.keycloak_host_port }}/realms/{PRODUCT}"),{% elif context.auth_workos %}"https://api.workos.com/".into(),{% else %}String::new(),{% endif %}
            audience: format!("{PRODUCT}-backend"),
            identity_admin_base_url: {% if context.auth_oidc %}"http://localhost:{{ context.keycloak_host_port }}"{% elif context.auth_clerk %}"https://api.clerk.com/v1"{% else %}"https://api.workos.com"{% endif %}.to_owned(),
            identity_admin_realm: PRODUCT.to_owned(),
            identity_admin_client_secret: None,
            erasure_hash_key: None,
        }
    }
}

impl Validate for AuthConfig {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Vec::new();
        if self.issuer.trim().is_empty() {
            errors.push(ValidationError::new("auth.issuer", "must not be empty"));
        }
        if self.provider == AuthProvider::Oidc && self.audience.trim().is_empty() {
            errors.push(ValidationError::new("auth.audience", "must not be empty"));
        }
        if self.provider == AuthProvider::Workos && self.client_id.trim().is_empty() {
            errors.push(ValidationError::new(
                "auth.client_id",
                "WorkOS requires its application client ID",
            ));
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationErrors::new(errors))
        }
    }
}

{% endif %}{% if context.worker %}#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct WorkerProductConfig {
    pub concurrency: usize,
    pub lease_duration_seconds: u64,
    pub job_timeout_seconds: u64,
    pub poll_interval_milliseconds: u64,
}

impl Default for WorkerProductConfig {
    fn default() -> Self {
        Self {
            concurrency: 5,
            lease_duration_seconds: 15 * 60,
            job_timeout_seconds: 10 * 60,
            poll_interval_milliseconds: 250,
        }
    }
}

impl Validate for WorkerProductConfig {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Vec::new();
        if !(1..=64).contains(&self.concurrency) {
            errors.push(ValidationError::new(
                "worker.concurrency",
                "must be between 1 and 64",
            ));
        }
        for (field, value) in [
            ("worker.lease_duration_seconds", self.lease_duration_seconds),
            ("worker.job_timeout_seconds", self.job_timeout_seconds),
            (
                "worker.poll_interval_milliseconds",
                self.poll_interval_milliseconds,
            ),
        ] {
            if value == 0 {
                errors.push(ValidationError::new(field, "must be greater than zero"));
            }
        }
        if self.job_timeout_seconds >= self.lease_duration_seconds {
            errors.push(ValidationError::new(
                "worker.job_timeout_seconds",
                "must be less than worker.lease_duration_seconds",
            ));
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationErrors::new(errors))
        }
    }
}

{% endif %}#[derive(Clone, Default)]
pub struct InMemoryItemRepository {
    items: Arc<RwLock<BTreeMap<Uuid, Item>>>,
}

impl InMemoryItemRepository {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl ItemRepository for InMemoryItemRepository {
    fn list(&self) -> PortFuture<'_, Result<Vec<Item>, RepositoryError>> {
        Box::pin(async move {
            let items = self
                .items
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            Ok(items.values().cloned().collect())
        })
    }

    fn get(&self, id: Uuid) -> PortFuture<'_, Result<Option<Item>, RepositoryError>> {
        Box::pin(async move {
            let items = self
                .items
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            Ok(items.get(&id).cloned())
        })
    }

    fn create(&self, item: Item) -> PortFuture<'_, Result<Item, RepositoryError>> {
        Box::pin(async move {
            let mut items = self
                .items
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if items.contains_key(&item.id) {
                return Err(RepositoryError::Conflict);
            }
            items.insert(item.id, item.clone());
            Ok(item)
        })
    }

    fn update(&self, item: Item) -> PortFuture<'_, Result<Option<Item>, RepositoryError>> {
        Box::pin(async move {
            let mut items = self
                .items
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let std::collections::btree_map::Entry::Occupied(mut entry) = items.entry(item.id) {
                entry.insert(item.clone());
                Ok(Some(item))
            } else {
                Ok(None)
            }
        })
    }

    fn delete(&self, id: Uuid) -> PortFuture<'_, Result<bool, RepositoryError>> {
        Box::pin(async move {
            let mut items = self
                .items
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            Ok(items.remove(&id).is_some())
        })
    }

    fn ready(&self) -> PortFuture<'_, Result<(), RepositoryError>> {
        Box::pin(async { Ok(()) })
    }
}

{% if context.auth_enabled %}#[derive(Clone, Default)]
pub struct InMemoryUserRepository {
    users: Arc<RwLock<BTreeMap<String, Uuid>>>,
}

impl InMemoryUserRepository {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl UserRepository for InMemoryUserRepository {
    fn resolve_subject(
        &self,
        subject: String,
    ) -> PortFuture<'_, Result<InternalUser, RepositoryError>> {
        Box::pin(async move {
            let mut users = self
                .users
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let id = *users.entry(subject.clone()).or_insert_with(Uuid::now_v7);
            Ok(InternalUser { id, subject })
        })
    }
}

{% endif %}pub fn operations_router(
    service: ItemService,
    identity: ServiceIdentity,
    metrics: PrometheusHandle,
    traffic_gate: TrafficGate,
) -> Result<(Router, ReadinessRegistry), RegistrationError> {
    let readiness = ReadinessRegistry::new();
    readiness.register_fn_default("item_repository", move || {
        let service = service.clone();
        async move {
            service
                .ready()
                .await
                .map_err(|_| ReadinessError::new("item repository is unavailable"))
        }
    })?;
    let router = OpsRouter::new(identity, metrics)
        .with_readiness(readiness.clone())
        .with_traffic_gate(traffic_gate)
        .into_router();
    Ok((router, readiness))
}{% if context.worker %}

pub fn worker_operations_router(
    runner: WorkerRunner,
    identity: ServiceIdentity,
    metrics: PrometheusHandle,
    traffic_gate: TrafficGate,
) -> Result<(Router, ReadinessRegistry), RegistrationError> {
    let readiness = ReadinessRegistry::new();
    readiness.register_fn_default("job_store", move || {
        let runner = runner.clone();
        async move {
            runner
                .ready()
                .await
                .map_err(|_| ReadinessError::new("job store cannot probe the durable outbox"))
        }
    })?;
    let router = OpsRouter::new(identity, metrics)
        .with_readiness(readiness.clone())
        .with_traffic_gate(traffic_gate)
        .into_router();
    Ok((router, readiness))
}{% endif %}{% if context.auth_enabled %}

pub fn identity_erasure(
    pool: sqlx::PgPool,
    auth: &AuthConfig,
    environment: baukit_config::Environment,
) -> Result<
    (
        baukit_erasure::ErasureService,
        baukit_erasure::IdentityDeletionHandler,
    ),
    Box<dyn std::error::Error>,
> {
    let local = environment == baukit_config::Environment::Local;
    let secret = |value: &Option<baukit_config::Secret<String>>, default: &str| {
        value
            .clone()
            .or_else(|| local.then(|| baukit_config::Secret::new(default.to_owned())))
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "identity erasure secrets must be configured outside local development",
                )
            })
    };
    let api_config = || -> Result<baukit_erasure::ApiDeletionConfig, std::io::Error> {
        Ok(baukit_erasure::ApiDeletionConfig {
            base_url: auth.identity_admin_base_url.clone(),
            api_key: auth.identity_admin_client_secret.clone().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "provider API key is required for identity erasure",
                )
            })?,
            allow_local_http: local,
        })
    };
    let (provider, deleter): (&str, Arc<dyn baukit_erasure::IdentityAccountDeleter>) =
        match auth.provider {
            AuthProvider::Oidc => (
                "keycloak",
                Arc::new(baukit_erasure::KeycloakAccountDeleter::new(
                    baukit_erasure::KeycloakDeletionConfig {
                        base_url: auth.identity_admin_base_url.clone(),
                        realm: auth.identity_admin_realm.clone(),
                        client_id: auth.audience.clone(),
                        client_secret: secret(
                            &auth.identity_admin_client_secret,
                            "local-backend-secret",
                        )?,
                        allow_local_http: local,
                    },
                )?),
            ),
            AuthProvider::Clerk => (
                "clerk",
                Arc::new(baukit_erasure::ClerkAccountDeleter::new(api_config()?)?),
            ),
            AuthProvider::Workos => (
                "workos",
                Arc::new(baukit_erasure::WorkOsAccountDeleter::new(api_config()?)?),
            ),
        };
    let store = baukit_erasure::PostgresErasureStore::new(
        pool,
        secret(
            &auth.erasure_hash_key,
            "local-erasure-hash-key-not-for-production",
        )?,
    )?;
    let handler = baukit_erasure::IdentityDeletionHandler::new(
        store.clone(),
        provider.into(),
        deleter.clone(),
    );
    let service = baukit_erasure::ErasureService::new(
        store,
        baukit_erasure::IdentityRetention::Delete {
            deleter,
            provider_id: provider.into(),
            inline_timeout: std::time::Duration::from_secs(3),
            max_attempts: 12,
        },
    )?;
    Ok((service, handler))
}{% endif %}
