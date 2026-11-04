use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use url::Url;
use uuid::Uuid;

use super::*;

/// Seconds a client waits before retrying a `503` plan or credit read.
pub const PLAN_RETRY_AFTER_SECONDS: u64 = 5;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanMode {
    Hub,
    Standalone,
}

/// The plan a product gates on. `billing` is set only by `refresh`, which
/// serves `GET /me/entitlement`; request gating reads features only.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AccountEntitlement {
    pub mode: PlanMode,
    pub suite_subject: String,
    pub tier: String,
    pub features: Vec<String>,
    pub revision: u64,
    /// The hub `/plans` page in hub mode, null in standalone mode.
    pub plans_url: Option<String>,
    pub billing: Option<BillingClaim>,
    /// The organization a member read the plan for, absent for the account's own plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization: Option<EntitlementOrganization>,
}
impl AccountEntitlement {
    pub fn has_feature(&self, feature: &str) -> bool {
        self.features.iter().any(|granted| granted == feature)
    }
}

/// One error set for every product. Loading never applies on the server, and
/// `Unavailable` denies access: no feature is granted on a failed read.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PlanError {
    #[error("suite_identity_missing")]
    SuiteIdentityMissing,
    #[error("entitlements_unavailable")]
    EntitlementsUnavailable,
    #[error("credits_unavailable")]
    CreditsUnavailable,
    #[error("feature_not_entitled")]
    FeatureNotEntitled { feature: String },
    /// The organization is unknown or the caller is not a member. Both read the same.
    #[error("organization_not_accessible")]
    OrganizationNotAccessible,
    #[error("organization_id_invalid")]
    OrganizationIdInvalid,
}

#[async_trait]
pub trait AccountPlan: Send + Sync {
    fn mode(&self) -> PlanMode;
    /// A freshly verified plan for `GET /me/entitlement`.
    async fn refresh(&self, suite_subject: &str) -> Result<AccountEntitlement, PlanError>;
    /// The cached plan for request gating, without `billing`.
    async fn current(&self, suite_subject: &str) -> Result<AccountEntitlement, PlanError>;
    async fn credits(&self, suite_subject: &str) -> Result<CreditBalance, PlanError>;
    /// A freshly read organization grant. Without organization support every
    /// organization is `organization_not_accessible`.
    async fn organization_refresh(
        &self,
        _organization_id: Uuid,
    ) -> Result<OrganizationEntitlement, PlanError> {
        Err(PlanError::OrganizationNotAccessible)
    }
    /// The cached organization grant for request gating, without the name.
    async fn organization_current(
        &self,
        _organization_id: Uuid,
    ) -> Result<OrganizationEntitlement, PlanError> {
        Err(PlanError::OrganizationNotAccessible)
    }
    /// Whether the hub confirms `suite_subject` can use the product in the organization.
    async fn is_member(
        &self,
        _organization_id: Uuid,
        _suite_subject: &str,
    ) -> Result<bool, PlanError> {
        Ok(false)
    }
}

/// Grants the subject's plan when it includes `feature`. A missing subject, a
/// failed read, an inaccessible organization and a locked feature are distinct errors.
pub async fn require_feature(
    plan: &dyn AccountPlan,
    subject: impl Into<FeatureSubject>,
    feature: &str,
) -> Result<FeatureGrant, PlanError> {
    let grant = feature_grant(plan, subject.into()).await?;
    if grant.has_feature(feature) {
        return Ok(grant);
    }
    Err(PlanError::FeatureNotEntitled {
        feature: feature.into(),
    })
}

async fn feature_grant(
    plan: &dyn AccountPlan,
    subject: FeatureSubject,
) -> Result<FeatureGrant, PlanError> {
    match subject {
        FeatureSubject::Account(suite_subject) => {
            Ok(plan.current(&suite_identity(suite_subject)?).await?.into())
        }
        FeatureSubject::Member {
            organization_id,
            suite_subject,
        } => {
            require_member(plan, organization_id, &suite_identity(suite_subject)?).await?;
            Ok(plan.organization_current(organization_id).await?.into())
        }
        FeatureSubject::Organization(organization_id) => {
            Ok(plan.organization_current(organization_id).await?.into())
        }
    }
}

pub(crate) fn suite_identity(suite_subject: Option<String>) -> Result<String, PlanError> {
    suite_subject
        .filter(|subject| !subject.is_empty())
        .ok_or(PlanError::SuiteIdentityMissing)
}

pub(crate) async fn require_member(
    plan: &dyn AccountPlan,
    organization_id: Uuid,
    suite_subject: &str,
) -> Result<(), PlanError> {
    if plan.is_member(organization_id, suite_subject).await? {
        return Ok(());
    }
    Err(PlanError::OrganizationNotAccessible)
}

/// The static grant from product configuration. Credits read as an empty
/// wallet because every standalone action is free.
pub struct StandaloneAccountPlan {
    entitlements: Entitlements,
    credits: ZeroCostCredits,
}
impl StandaloneAccountPlan {
    pub fn new(product_id: String, grant: StandaloneEntitlements) -> Self {
        Self {
            credits: ZeroCostCredits::new(product_id.clone()),
            entitlements: Entitlements::standalone(product_id, grant),
        }
    }
}
#[async_trait]
impl AccountPlan for StandaloneAccountPlan {
    fn mode(&self) -> PlanMode {
        PlanMode::Standalone
    }
    async fn refresh(&self, suite_subject: &str) -> Result<AccountEntitlement, PlanError> {
        self.current(suite_subject).await
    }
    async fn current(&self, suite_subject: &str) -> Result<AccountEntitlement, PlanError> {
        let snapshot = self
            .entitlements
            .current(suite_subject, 0)
            .await
            .ok()
            .flatten()
            .ok_or(PlanError::EntitlementsUnavailable)?;
        Ok(entitlement(PlanMode::Standalone, snapshot, None, None))
    }
    async fn credits(&self, suite_subject: &str) -> Result<CreditBalance, PlanError> {
        self.credits
            .balance(suite_subject)
            .await
            .map_err(|_| PlanError::CreditsUnavailable)
    }
    async fn organization_refresh(
        &self,
        organization_id: Uuid,
    ) -> Result<OrganizationEntitlement, PlanError> {
        self.organization_current(organization_id).await
    }
    async fn organization_current(
        &self,
        organization_id: Uuid,
    ) -> Result<OrganizationEntitlement, PlanError> {
        let snapshot = self
            .entitlements
            .current(&organization_cache_subject(organization_id), 0)
            .await
            .ok()
            .flatten()
            .ok_or(PlanError::EntitlementsUnavailable)?;
        Ok(organization_entitlement(organization_id, None, snapshot))
    }
    /// Standalone mode has no hub directory: the product's own authentication
    /// decides who acts for an organization.
    async fn is_member(
        &self,
        _organization_id: Uuid,
        _suite_subject: &str,
    ) -> Result<bool, PlanError> {
        Ok(true)
    }
}

/// Fetches a signed entitlement ticket for a subject.
#[async_trait]
pub trait TicketSource: Send + Sync {
    async fn ticket(&self, suite_subject: &str) -> Result<String, EntitlementSourceError>;
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TicketRequest<'a> {
    suite_subject: &'a str,
    product_id: &'a str,
    include_billing: bool,
}
#[derive(Deserialize)]
struct TicketResponse {
    ticket: String,
}

/// `POST /platform/v1/entitlements/tickets` with service credentials and
/// `includeBilling`, so the ticket carries the `billing` claim (Schlüsselbund C27).
pub struct PlatformTicketSource {
    hub: Arc<HubClient>,
}
impl PlatformTicketSource {
    pub fn new(hub: Arc<HubClient>) -> Self {
        Self { hub }
    }
}
#[async_trait]
impl TicketSource for PlatformTicketSource {
    async fn ticket(&self, suite_subject: &str) -> Result<String, EntitlementSourceError> {
        let response: TicketResponse = self
            .hub
            .post_json(
                &["entitlements", "tickets"],
                &TicketRequest {
                    suite_subject,
                    product_id: self.hub.product_id(),
                    include_billing: true,
                },
            )
            .await
            .map_err(|error| hub_source_error(&error))?;
        Ok(response.ticket)
    }
}
fn hub_source_error(error: &HubError) -> EntitlementSourceError {
    match error {
        HubError::Rejected { status, code } => {
            EntitlementSourceError::from_hub_response(*status, code.as_deref().unwrap_or_default())
        }
        HubError::InvalidResponse => EntitlementSourceError::Invalid,
        HubError::Unavailable | HubError::Unauthorized => EntitlementSourceError::Unavailable,
    }
}

/// Verifies tickets from a `TicketSource`, so `Entitlements` and the event
/// handler can pull through it.
#[derive(Clone)]
pub struct VerifiedTicketSource {
    tickets: Arc<dyn TicketSource>,
    verifier: TicketVerifier,
}
impl VerifiedTicketSource {
    pub fn new(tickets: Arc<dyn TicketSource>, verifier: TicketVerifier) -> Self {
        Self { tickets, verifier }
    }
    pub async fn claims(
        &self,
        suite_subject: &str,
        product_id: &str,
    ) -> Result<TicketClaims, EntitlementSourceError> {
        let ticket = self.tickets.ticket(suite_subject).await?;
        let claims = self
            .verifier
            .verify(&ticket)
            .await
            .map_err(|error| match error {
                TicketError::KeysUnavailable => EntitlementSourceError::Unavailable,
                TicketError::Invalid | TicketError::Expired | TicketError::WrongAudience => {
                    EntitlementSourceError::Invalid
                }
            })?;
        if claims.sub != suite_subject || claims.aud != product_id {
            return Err(EntitlementSourceError::Invalid);
        }
        Ok(claims)
    }
}
#[async_trait]
impl EntitlementSource for VerifiedTicketSource {
    async fn pull(
        &self,
        subject: &str,
        product_id: &str,
    ) -> Result<EntitlementSnapshot, EntitlementSourceError> {
        self.claims(subject, product_id).await.map(Into::into)
    }
}

/// Hub mode: `refresh` verifies a fresh ticket and stores it in the cache,
/// `current` reads through the cache, and credits come from the hub wallet.
pub struct HubAccountPlan {
    product_id: String,
    source: VerifiedTicketSource,
    cache: Arc<dyn EntitlementCache>,
    entitlements: Entitlements,
    credits: Arc<dyn Credits>,
    plans_url: String,
    organizations: Option<HubOrganizationPlans>,
}
impl HubAccountPlan {
    /// `plans_url` is the hub web `/plans` page, as an absolute HTTP(S) URL.
    pub fn new(
        product_id: String,
        source: VerifiedTicketSource,
        cache: Arc<dyn EntitlementCache>,
        credits: Arc<dyn Credits>,
        plans_url: &str,
    ) -> Result<Self, PlatformError> {
        let plans_url = Url::parse(plans_url)
            .ok()
            .filter(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
            .ok_or(PlatformError::Invalid)?;
        if product_id.is_empty() {
            return Err(PlatformError::Invalid);
        }
        Ok(Self {
            entitlements: Entitlements::hub(
                product_id.clone(),
                cache.clone(),
                Arc::new(source.clone()),
            ),
            product_id,
            source,
            cache,
            credits,
            plans_url: plans_url.into(),
            organizations: None,
        })
    }
    /// Enables organization subjects. `HubOrganizations` needs `organizations:read`.
    pub fn with_organizations(mut self, grants: Arc<dyn OrganizationGrants>) -> Self {
        self.organizations = Some(HubOrganizationPlans::new(
            self.product_id.clone(),
            grants,
            self.cache.clone(),
        ));
        self
    }
    fn organizations(&self) -> Result<&HubOrganizationPlans, PlanError> {
        self.organizations
            .as_ref()
            .ok_or(PlanError::OrganizationNotAccessible)
    }
}
impl HubAccountPlan {
    async fn free(&self, suite_subject: &str) -> Result<AccountEntitlement, PlanError> {
        let revision = self
            .cache
            .revision(suite_subject, &self.product_id)
            .await
            .map_err(|_| PlanError::EntitlementsUnavailable)?;
        let snapshot = EntitlementSnapshot {
            subject: suite_subject.into(),
            product_id: self.product_id.clone(),
            tier: "free".into(),
            features: Vec::new(),
            revision,
            valid_until: chrono::Utc::now()
                .timestamp()
                .saturating_add(FREE_CACHE_TTL_SECONDS),
            grace_until: None,
        };
        Ok(entitlement(
            PlanMode::Hub,
            snapshot,
            Some(self.plans_url.clone()),
            None,
        ))
    }
}
#[async_trait]
impl AccountPlan for HubAccountPlan {
    fn mode(&self) -> PlanMode {
        PlanMode::Hub
    }
    async fn refresh(&self, suite_subject: &str) -> Result<AccountEntitlement, PlanError> {
        let claims = match self.source.claims(suite_subject, &self.product_id).await {
            Ok(claims) => claims,
            Err(EntitlementSourceError::NoEntitlements) => return self.free(suite_subject).await,
            Err(_) => return Err(PlanError::EntitlementsUnavailable),
        };
        let billing = claims.billing.clone();
        let snapshot = EntitlementSnapshot::from(claims);
        self.cache
            .store(&snapshot)
            .await
            .map_err(|_| PlanError::EntitlementsUnavailable)?;
        Ok(entitlement(
            PlanMode::Hub,
            snapshot,
            Some(self.plans_url.clone()),
            billing,
        ))
    }
    async fn current(&self, suite_subject: &str) -> Result<AccountEntitlement, PlanError> {
        let snapshot = self
            .entitlements
            .current(suite_subject, chrono::Utc::now().timestamp())
            .await
            .ok()
            .flatten()
            .ok_or(PlanError::EntitlementsUnavailable)?;
        Ok(entitlement(
            PlanMode::Hub,
            snapshot,
            Some(self.plans_url.clone()),
            None,
        ))
    }
    async fn credits(&self, suite_subject: &str) -> Result<CreditBalance, PlanError> {
        self.credits
            .balance(suite_subject)
            .await
            .map_err(|_| PlanError::CreditsUnavailable)
    }
    async fn organization_refresh(
        &self,
        organization_id: Uuid,
    ) -> Result<OrganizationEntitlement, PlanError> {
        self.organizations()?.refresh(organization_id).await
    }
    async fn organization_current(
        &self,
        organization_id: Uuid,
    ) -> Result<OrganizationEntitlement, PlanError> {
        self.organizations()?.current(organization_id).await
    }
    async fn is_member(
        &self,
        organization_id: Uuid,
        suite_subject: &str,
    ) -> Result<bool, PlanError> {
        match &self.organizations {
            Some(organizations) => {
                organizations
                    .is_member(organization_id, suite_subject)
                    .await
            }
            None => Ok(false),
        }
    }
}

fn entitlement(
    mode: PlanMode,
    snapshot: EntitlementSnapshot,
    plans_url: Option<String>,
    billing: Option<BillingClaim>,
) -> AccountEntitlement {
    AccountEntitlement {
        mode,
        suite_subject: snapshot.subject,
        tier: snapshot.tier,
        features: snapshot.features,
        revision: snapshot.revision,
        plans_url,
        billing,
        organization: None,
    }
}
