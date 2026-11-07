{% if context.platform %}mod platform;
pub use platform::*;

{% endif %}use std::{future::Future, pin::Pin};

{% if context.auth_enabled %}use {{ context.app_crate }}_domain::InternalUser;
use {{ context.app_crate }}_domain::Item;
{% else %}use {{ context.app_crate }}_domain::Item;
{% endif %}
use thiserror::Error;
use uuid::Uuid;

pub type PortFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Item storage. Every read and write is scoped to the owner, so one owner never sees another's items.
pub trait ItemRepository: Send + Sync + 'static {
    fn list<'a>(&'a self, owner_id: &'a str) -> PortFuture<'a, Result<Vec<Item>, RepositoryError>>;
    fn get<'a>(
        &'a self,
        owner_id: &'a str,
        id: Uuid,
    ) -> PortFuture<'a, Result<Option<Item>, RepositoryError>>;
    fn create(&self, item: Item) -> PortFuture<'_, Result<Item, RepositoryError>>;
    fn update(&self, item: Item) -> PortFuture<'_, Result<Option<Item>, RepositoryError>>;
    fn delete<'a>(
        &'a self,
        owner_id: &'a str,
        id: Uuid,
    ) -> PortFuture<'a, Result<bool, RepositoryError>>;
    fn ready(&self) -> PortFuture<'_, Result<(), RepositoryError>>;
}

{% if context.auth_enabled %}pub trait UserRepository: Send + Sync + 'static {
    fn resolve_subject(
        &self,
        subject: String,
        authenticated_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> PortFuture<'_, Result<InternalUser, RepositoryError>>;
}

{% endif %}#[derive(Debug, Error)]
pub enum RepositoryError {
{% if context.auth_enabled %}    #[error("profile erased")]
    ProfileErased,
{% endif %}    #[error("item already exists")]
    Conflict,
    #[error("repository unavailable")]
    Unavailable(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl RepositoryError {
    pub fn unavailable(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Unavailable(Box::new(error))
    }
}
