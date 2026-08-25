use std::{future::Future, pin::Pin};

{% if context.auth_oidc %}use {{ context.app_crate }}_domain::{InternalUser, Item};
{% else %}use {{ context.app_crate }}_domain::Item;
{% endif %}
use thiserror::Error;
use uuid::Uuid;

pub type PortFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait ItemRepository: Send + Sync + 'static {
    fn list(&self) -> PortFuture<'_, Result<Vec<Item>, RepositoryError>>;
    fn get(&self, id: Uuid) -> PortFuture<'_, Result<Option<Item>, RepositoryError>>;
    fn create(&self, item: Item) -> PortFuture<'_, Result<Item, RepositoryError>>;
    fn update(&self, item: Item) -> PortFuture<'_, Result<Option<Item>, RepositoryError>>;
    fn delete(&self, id: Uuid) -> PortFuture<'_, Result<bool, RepositoryError>>;
    fn ready(&self) -> PortFuture<'_, Result<(), RepositoryError>>;
}

{% if context.auth_oidc %}pub trait UserRepository: Send + Sync + 'static {
    fn resolve_subject(
        &self,
        subject: String,
    ) -> PortFuture<'_, Result<InternalUser, RepositoryError>>;
}

{% endif %}#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("item already exists")]
    Conflict,
    #[error("repository unavailable")]
    Unavailable(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl RepositoryError {
    pub fn unavailable(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Unavailable(Box::new(error))
    }
}
