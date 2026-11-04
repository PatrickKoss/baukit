use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};
use uuid::Uuid;

use super::*;

/// How long a fetched organization grant or confirmed membership stays cached.
pub const ORGANIZATION_CACHE_TTL_SECONDS: i64 = 60;
/// The request header that names the organization a request acts for.
pub const ORGANIZATION_HEADER: &str = "x-organization-id";
const MAX_MEMBERSHIP_PAGES: usize = 100;

/// Who a feature check is for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FeatureSubject {
    /// The suite account's own plan. `None` answers `suite_identity_missing`.
    Account(Option<String>),
    /// A member acting for an organization: the organization's grant, once the
    /// hub confirms the membership.
    Member {
        organization_id: Uuid,
        suite_subject: Option<String>,
    },
    /// An API token or worker owned by the organization, with no suite subject.
    Organization(Uuid),
}
impl From<Option<&str>> for FeatureSubject {
    fn from(subject: Option<&str>) -> Self {
        Self::Account(subject.map(Into::into))
    }
}

/// The organization an entitlement belongs to.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EntitlementOrganization {
    pub organization_id: Uuid,
    pub name: Option<String>,
}

/// An organization's tier and features from the hub. A suspended organization
/// keeps its tier and has no features.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrganizationEntitlement {
    pub organization: EntitlementOrganization,
    pub tier: String,
    pub features: Vec<String>,
    pub revision: u64,
}

/// What `require_feature` granted, for either subject.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeatureGrant {
    pub tier: String,
    pub features: Vec<String>,
    pub revision: u64,
    /// `None` for an account's own plan.
    pub organization_id: Option<Uuid>,
}
impl FeatureGrant {
    pub fn has_feature(&self, feature: &str) -> bool {
        self.features.iter().any(|granted| granted == feature)
    }
}
impl From<AccountEntitlement> for FeatureGrant {
    fn from(entitlement: AccountEntitlement) -> Self {
        Self {
            tier: entitlement.tier,
            features: entitlement.features,
            revision: entitlement.revision,
            organization_id: entitlement
                .organization
                .map(|organization| organization.organization_id),
        }
    }
}
impl From<OrganizationEntitlement> for FeatureGrant {
    fn from(entitlement: OrganizationEntitlement) -> Self {
        Self {
            tier: entitlement.tier,
            features: entitlement.features,
            revision: entitlement.revision,
            organization_id: Some(entitlement.organization.organization_id),
        }
    }
}

/// The hub reads behind organization subjects: the summary for the grant and
/// the subject's organizations for membership.
#[async_trait]
pub trait OrganizationGrants: OrganizationDirectory {
    async fn subject_organizations(
        &self,
        suite_subject: &str,
        cursor: Option<&str>,
    ) -> Result<SubjectOrganizationPage, OrganizationError>;
}
#[async_trait]
impl OrganizationGrants for HubOrganizations {
    async fn subject_organizations(
        &self,
        suite_subject: &str,
        cursor: Option<&str>,
    ) -> Result<SubjectOrganizationPage, OrganizationError> {
        self.organizations_for_subject(suite_subject, cursor).await
    }
}

/// The cache key of an organization grant in the `EntitlementCache`.
pub fn organization_cache_subject(organization_id: Uuid) -> String {
    format!("organization:{organization_id}")
}

/// Organization grants and memberships for `HubAccountPlan`, cached for
/// `ORGANIZATION_CACHE_TTL_SECONDS`.
pub(crate) struct HubOrganizationPlans {
    product_id: String,
    grants: Arc<dyn OrganizationGrants>,
    cache: Arc<dyn EntitlementCache>,
    members: Mutex<HashMap<(Uuid, String), i64>>,
}
impl HubOrganizationPlans {
    pub(crate) fn new(
        product_id: String,
        grants: Arc<dyn OrganizationGrants>,
        cache: Arc<dyn EntitlementCache>,
    ) -> Self {
        Self {
            product_id,
            grants,
            cache,
            members: Mutex::default(),
        }
    }

    pub(crate) async fn refresh(
        &self,
        organization_id: Uuid,
    ) -> Result<OrganizationEntitlement, PlanError> {
        let summary = self
            .grants
            .summary(organization_id)
            .await
            .map_err(|error| match error {
                OrganizationError::NotFound => PlanError::OrganizationNotAccessible,
                OrganizationError::Unavailable | OrganizationError::Hub(_) => {
                    PlanError::EntitlementsUnavailable
                }
            })?;
        if summary.organization_id != organization_id || summary.product_id != self.product_id {
            return Err(PlanError::EntitlementsUnavailable);
        }
        let features = match summary.status {
            OrganizationStatus::Active => summary.features,
            OrganizationStatus::Suspended => Vec::new(),
        };
        let snapshot = EntitlementSnapshot {
            subject: organization_cache_subject(organization_id),
            product_id: self.product_id.clone(),
            tier: summary.tier,
            features,
            revision: summary.revision,
            valid_until: now().saturating_add(ORGANIZATION_CACHE_TTL_SECONDS),
            grace_until: None,
        };
        self.cache
            .store(&snapshot)
            .await
            .map_err(|_| PlanError::EntitlementsUnavailable)?;
        Ok(organization_entitlement(
            organization_id,
            summary.name,
            snapshot,
        ))
    }

    pub(crate) async fn current(
        &self,
        organization_id: Uuid,
    ) -> Result<OrganizationEntitlement, PlanError> {
        let cached = self
            .cache
            .get(
                &organization_cache_subject(organization_id),
                &self.product_id,
                now(),
            )
            .await
            .map_err(|_| PlanError::EntitlementsUnavailable)?;
        match cached {
            Some(snapshot) => Ok(organization_entitlement(organization_id, None, snapshot)),
            None => self.refresh(organization_id).await,
        }
    }

    pub(crate) async fn is_member(
        &self,
        organization_id: Uuid,
        suite_subject: &str,
    ) -> Result<bool, PlanError> {
        let key = (organization_id, suite_subject.to_owned());
        if self.cached_member(&key) {
            return Ok(true);
        }
        let member = self.hub_member(organization_id, suite_subject).await?;
        if member {
            self.lock_members()
                .insert(key, now().saturating_add(ORGANIZATION_CACHE_TTL_SECONDS));
        }
        Ok(member)
    }

    fn lock_members(&self) -> std::sync::MutexGuard<'_, HashMap<(Uuid, String), i64>> {
        self.members
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn cached_member(&self, key: &(Uuid, String)) -> bool {
        let now = now();
        let mut members = self.lock_members();
        members.retain(|_, until| now < *until);
        members.contains_key(key)
    }

    async fn hub_member(
        &self,
        organization_id: Uuid,
        suite_subject: &str,
    ) -> Result<bool, PlanError> {
        let mut seen = HashSet::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_MEMBERSHIP_PAGES {
            let page = self
                .grants
                .subject_organizations(suite_subject, cursor.as_deref())
                .await
                .map_err(|_| PlanError::EntitlementsUnavailable)?;
            if page
                .items
                .iter()
                .any(|item| item.organization_id == organization_id)
            {
                return Ok(true);
            }
            let Some(next) = page.next_cursor else {
                return Ok(false);
            };
            if !seen.insert(next.clone()) {
                return Err(PlanError::EntitlementsUnavailable);
            }
            cursor = Some(next);
        }
        Err(PlanError::EntitlementsUnavailable)
    }
}

pub(crate) fn organization_entitlement(
    organization_id: Uuid,
    name: Option<String>,
    snapshot: EntitlementSnapshot,
) -> OrganizationEntitlement {
    OrganizationEntitlement {
        organization: EntitlementOrganization {
            organization_id,
            name,
        },
        tier: snapshot.tier,
        features: snapshot.features,
        revision: snapshot.revision,
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
