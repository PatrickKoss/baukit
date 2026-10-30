use super::{HubClient, HubError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrganizationStatus {
    Active,
    Suspended,
}

/// The calling product's view of a business organization.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationSummary {
    pub organization_id: Uuid,
    pub status: OrganizationStatus,
    pub product_id: String,
    pub tier: String,
    pub features: Vec<String>,
    /// `None` for a per-organization plan without a seat limit.
    #[serde(default)]
    pub paid_quantity: Option<u64>,
    pub assigned_seats: u64,
    /// Matches the latest `organization.updated` event, or 0 before the first.
    pub revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum OrganizationError {
    /// `platform_organization_not_found`: unknown, or invisible to this product.
    #[error("platform_organization_not_found")]
    NotFound,
    #[error("organization_unavailable")]
    Unavailable,
    #[error(transparent)]
    Hub(HubError),
}
impl From<HubError> for OrganizationError {
    fn from(error: HubError) -> Self {
        match error {
            HubError::Rejected { status: 404, code }
                if code.as_deref() == Some("platform_organization_not_found") =>
            {
                Self::NotFound
            }
            HubError::Unavailable => Self::Unavailable,
            other => Self::Hub(other),
        }
    }
}

#[async_trait]
pub trait OrganizationDirectory: Send + Sync {
    async fn summary(
        &self,
        organization_id: Uuid,
    ) -> Result<OrganizationSummary, OrganizationError>;
}

/// Reads `GET /platform/v1/organizations/{id}`. Requires `organizations:read`.
#[derive(Clone)]
pub struct HubOrganizations {
    hub: Arc<HubClient>,
}
impl HubOrganizations {
    pub fn new(hub: Arc<HubClient>) -> Self {
        Self { hub }
    }
}
#[async_trait]
impl OrganizationDirectory for HubOrganizations {
    async fn summary(
        &self,
        organization_id: Uuid,
    ) -> Result<OrganizationSummary, OrganizationError> {
        let id = organization_id.to_string();
        let summary: OrganizationSummary = self.hub.get_json(&["organizations", &id]).await?;
        if summary.organization_id != organization_id || summary.product_id != self.hub.product_id()
        {
            return Err(OrganizationError::Hub(HubError::InvalidResponse));
        }
        Ok(summary)
    }
}
