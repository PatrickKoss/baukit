use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};

use axum::Router;
use baukit_config::{Validate{% if context.auth_oidc or context.worker %}, ValidationError{% endif %}, ValidationErrors};
{% if context.worker %}use baukit_jobs::WorkerRunner;
{% endif %}use baukit_ops::{
    OpsRouter, PrometheusHandle, ReadinessError, ReadinessRegistry, RegistrationError,
    ServiceIdentity, TrafficGate,
};

{% if context.auth_oidc %}use {{ context.app_crate }}_domain::{InternalUser, Item};
{% else %}use {{ context.app_crate }}_domain::Item;
{% endif %}use {{ context.app_crate }}_ports::{ItemRepository, PortFuture, RepositoryError{% if context.auth_oidc %}, UserRepository{% endif %}};
use {{ context.app_crate }}_services::ItemService;

use serde::Deserialize;
use uuid::Uuid;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
{% if context.auth_oidc or context.worker %}pub struct ProductConfig {
{% if context.auth_oidc %}    pub auth: AuthConfig,
{% endif %}{% if context.worker %}    pub worker: WorkerProductConfig,
{% endif %}}
{% else %}pub struct ProductConfig {}
{% endif %}
impl Validate for ProductConfig {
    fn validate(&self) -> Result<(), ValidationErrors> {
{% if context.auth_oidc or context.worker %}        let mut errors = Vec::new();
{% if context.auth_oidc %}        if let Err(auth) = self.auth.validate() {
            errors.extend(auth.into_errors());
        }
{% endif %}{% if context.worker %}        if let Err(worker) = self.worker.validate() {
            errors.extend(worker.into_errors());
        }
{% endif %}        if errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationErrors::new(errors))
        }
{% else %}        Ok(())
{% endif %}    }
}

{% if context.auth_oidc %}#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct AuthConfig {
    pub issuer: String,
    pub audience: String,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            issuer: "http://localhost:8081/realms/{{ context.app_name }}".to_owned(),
            audience: "{{ context.app_name }}-backend".to_owned(),
        }
    }
}

impl Validate for AuthConfig {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Vec::new();
        if self.issuer.trim().is_empty() {
            errors.push(ValidationError::new("auth.issuer", "must not be empty"));
        }
        if self.audience.trim().is_empty() {
            errors.push(ValidationError::new("auth.audience", "must not be empty"));
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

{% if context.auth_oidc %}#[derive(Clone, Default)]
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
}{% endif %}
