use std::sync::Arc;

{% if context.auth_oidc %}use {{ context.app_crate }}_domain::{DomainError, InternalUser, Item};
{% else %}use {{ context.app_crate }}_domain::{DomainError, Item};
{% endif %}
use {{ context.app_crate }}_ports::{ItemRepository, RepositoryError{% if context.auth_oidc %}, UserRepository{% endif %}};
use thiserror::Error;
use uuid::Uuid;

#[derive(Clone)]
pub struct ItemService {
    repository: Arc<dyn ItemRepository>,
}

{% if context.auth_oidc %}#[derive(Clone)]
pub struct UserService {
    repository: Arc<dyn UserRepository>,
}

impl UserService {
    pub fn new(repository: Arc<dyn UserRepository>) -> Self {
        Self { repository }
    }

    pub async fn resolve_subject(&self, subject: &str) -> Result<InternalUser, ServiceError> {
        self.repository
            .resolve_subject(subject.to_owned())
            .await
            .map_err(Into::into)
    }
}

{% endif %}impl ItemService {
    pub fn new(repository: Arc<dyn ItemRepository>) -> Self {
        Self { repository }
    }

    pub async fn list(&self) -> Result<Vec<Item>, ServiceError> {
        self.repository.list().await.map_err(Into::into)
    }

    pub async fn get(&self, id: Uuid) -> Result<Item, ServiceError> {
        self.repository.get(id).await?.ok_or(ServiceError::NotFound)
    }

    pub async fn create(&self, name: String) -> Result<Item, ServiceError> {
        let item = Item::new(Uuid::now_v7(), name)?;
        self.repository.create(item).await.map_err(Into::into)
    }

    pub async fn update(&self, id: Uuid, name: String) -> Result<Item, ServiceError> {
        let item = Item::new(id, name)?;
        self.repository
            .update(item)
            .await?
            .ok_or(ServiceError::NotFound)
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), ServiceError> {
        if self.repository.delete(id).await? {
            Ok(())
        } else {
            Err(ServiceError::NotFound)
        }
    }

    pub async fn ready(&self) -> Result<(), ServiceError> {
        self.repository.ready().await.map_err(Into::into)
    }
}

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("item not found")]
    NotFound,
    #[error(transparent)]
    Invalid(#[from] DomainError),
    #[error(transparent)]
    Repository(#[from] RepositoryError),
}
