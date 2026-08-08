use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};

use axum::Router;
use baukit_ops::{
    OpsRouter, PrometheusHandle, ReadinessError, ReadinessRegistry, RegistrationError,
    ServiceIdentity, TrafficGate,
};
use {{ context.app_crate }}_domain::Item;
use {{ context.app_crate }}_ports::{ItemRepository, PortFuture, RepositoryError};
use {{ context.app_crate }}_services::ItemService;
use uuid::Uuid;

#[derive(Clone, Default)]
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

pub fn operations_router(
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
}
