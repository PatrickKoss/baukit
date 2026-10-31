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

/// A member's role in the organization, independent of the product.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrganizationRole {
    Owner,
    Admin,
    Billing,
    Member,
}

/// A seat holder for the calling product.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationMember {
    pub suite_subject: String,
    pub account_role: OrganizationRole,
}

/// One page of seat holders, sorted by the hub's account order.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizationMemberPage {
    pub items: Vec<OrganizationMember>,
    /// Pass to the next `members` call. `None` on the last page.
    #[serde(default)]
    pub next_cursor: Option<String>,
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
    /// Reads `GET /platform/v1/organizations/{id}/members?product=<caller>&cursor=`.
    /// Start with `None` and pass each `next_cursor` until it is `None`. A
    /// malformed or foreign cursor is `HubError::Rejected` with `invalid_pagination`.
    pub async fn members(
        &self,
        organization_id: Uuid,
        cursor: Option<&str>,
    ) -> Result<OrganizationMemberPage, OrganizationError> {
        let id = organization_id.to_string();
        let mut query = vec![("product", self.hub.product_id())];
        if let Some(cursor) = cursor {
            query.push(("cursor", cursor));
        }
        Ok(self
            .hub
            .get_json_with_query(&["organizations", &id, "members"], &query)
            .await?)
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
