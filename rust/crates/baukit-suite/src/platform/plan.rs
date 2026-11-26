use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use url::Url;
use uuid::Uuid;

use super::*;

/// Seconds a client waits before retrying a `503` plan or credit read.
pub const PLAN_RETRY_AFTER_SECONDS: u64 = 5;
const FREE_TIER: &str = "free";

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
    /// The role `suite_subject` holds in the organization, `None` for a non-member.
    async fn membership(
        &self,
        _organization_id: Uuid,
        _suite_subject: &str,
    ) -> Result<Option<OrganizationRole>, PlanError> {
        Ok(None)
    }
    /// Whether `suite_subject` can use the product in the organization.
    async fn is_member(
        &self,
        organization_id: Uuid,
        suite_subject: &str,
    ) -> Result<bool, PlanError> {
        Ok(self
            .membership(organization_id, suite_subject)
            .await?
            .is_some())
    }
    /// Drops a cached membership, for `organization.membership_changed` and `seat.changed`.
    async fn evict_member(&self, _organization_id: Uuid, _suite_subject: &str) {}
    /// Drops every cached membership and the cached grant of an organization.
    async fn evict_organization(&self, _organization_id: Uuid) -> Result<(), PlanError> {
        Ok(())
    }
}

/// Confirms `suite_subject` is a member whose role satisfies `required`, and
/// returns that role.
///
/// # Errors
///
/// `SuiteIdentityMissing` without a subject, `OrganizationNotAccessible` for a
/// non-member or a role that does not satisfy `required`, and any error from
/// `plan.membership`.
pub async fn require_role(
    plan: &dyn AccountPlan,
    organization_id: Uuid,
    suite_subject: Option<String>,
    required: OrganizationRole,
) -> Result<OrganizationRole, PlanError> {
    let subject = suite_identity(suite_subject)?;
    match plan.membership(organization_id, &subject).await? {
        Some(role) if role.satisfies(required) => Ok(role),
        _ => Err(PlanError::OrganizationNotAccessible),
    }
}

/// Grants the subject's plan when it includes `feature`. A missing subject, a
/// failed read, an inaccessible organization and a locked feature are distinct errors.
///
/// # Errors
///
/// `PlanError::SuiteIdentityMissing` when an account or member subject has no suite subject,
/// `OrganizationNotAccessible` when a member subject is not a member, `FeatureNotEntitled` when
/// the plan lacks `feature`, and any error `plan` returns from `current`, `is_member` or
/// `organization_current`, such as `EntitlementsUnavailable`.
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
/// wallet because every standalone action is free. Membership comes from the
/// product's `OrganizationMembership`; pass `NoOrganizationMembers` when the
/// product has no organizations.
pub struct StandaloneAccountPlan {
    entitlements: Entitlements,
    credits: ZeroCostCredits,
    members: Arc<dyn OrganizationMembership>,
}
impl StandaloneAccountPlan {
    pub fn new(
        product_id: String,
        grant: StandaloneEntitlements,
        members: Arc<dyn OrganizationMembership>,
    ) -> Self {
        Self {
            credits: ZeroCostCredits::new(product_id.clone()),
            entitlements: Entitlements::standalone(product_id, grant),
            members,
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
    async fn membership(
        &self,
        organization_id: Uuid,
        suite_subject: &str,
    ) -> Result<Option<OrganizationRole>, PlanError> {
        self.members.role(organization_id, suite_subject).await
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
            .send_json(
                HubRequest::idempotent(
                    &["entitlements", "tickets"],
                    &TicketRequest {
                        suite_subject,
                        product_id: self.hub.product_id(),
                        include_billing: true,
                    },
                )
                .map_err(|error| hub_source_error(&error))?,
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
        HubError::CoolingDown { retry_after } => EntitlementSourceError::CoolingDown {
            retry_after: *retry_after,
        },
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
    /// # Errors
    ///
    /// `EntitlementSourceError::Unavailable` when the ticket source fails or keys cannot be fetched,
    /// `Invalid` when the ticket fails verification or names another subject or product. Ticket
    /// source errors pass through.
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
    clock: Arc<dyn PlatformClock>,
}
impl HubAccountPlan {
    /// `plans_url` is the hub web `/plans` page, as an absolute HTTP(S) URL.
    ///
    /// # Errors
    ///
    /// `PlatformError::Invalid` when `plans_url` is not an absolute HTTP(S) URL with a host or
    /// `product_id` is empty or holds `/`, which organization cache keys use.
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
        if product_id.is_empty() || product_id.contains('/') {
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
            clock: Arc::new(SystemPlatformClock),
        })
    }
    /// Enables organization subjects. `HubOrganizations` needs `organizations:read`.
    #[must_use]
    pub fn with_organizations(mut self, grants: Arc<dyn OrganizationGrants>) -> Self {
        self.organizations = Some(HubOrganizationPlans::new(
            self.product_id.clone(),
            grants,
            self.cache.clone(),
            self.clock.clone(),
        ));
        self
    }
    /// Replaces the clock behind organization cache expiry and free snapshots.
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn PlatformClock>) -> Self {
        if let Some(organizations) = self.organizations.as_mut() {
            organizations.set_clock(clock.clone());
        }
        self.clock = clock;
        self
    }
    fn organizations(&self) -> Result<&HubOrganizationPlans, PlanError> {
        self.organizations
            .as_ref()
            .ok_or(PlanError::OrganizationNotAccessible)
    }
}
impl HubAccountPlan {
    /// A free answer supersedes a cached paid snapshot, so it takes the next
    /// revision. A cached free snapshot keeps its revision, so repeated free
    /// answers do not outrun the hub's own revision.
    async fn free_revision(&self, suite_subject: &str) -> Result<u64, PlanError> {
        let unavailable = |_| PlanError::EntitlementsUnavailable;
        let revision = self
            .cache
            .revision(suite_subject, &self.product_id)
            .await
            .map_err(unavailable)?;
        let cached = self
            .cache
            .get(
                suite_subject,
                &self.product_id,
                self.clock.now().as_second(),
            )
            .await
            .map_err(unavailable)?;
        let already_free = cached.is_some_and(|snapshot| {
            snapshot.revision == revision
                && snapshot.tier == FREE_TIER
                && snapshot.features.is_empty()
        });
        Ok(if already_free {
            revision
        } else {
            revision.saturating_add(1)
        })
    }
    async fn free(&self, suite_subject: &str) -> Result<AccountEntitlement, PlanError> {
        let revision = self.free_revision(suite_subject).await?;
        let snapshot = EntitlementSnapshot {
            subject: suite_subject.into(),
            product_id: self.product_id.clone(),
            tier: FREE_TIER.into(),
            features: Vec::new(),
            revision,
            valid_until: self
                .clock
                .now()
                .as_second()
                .saturating_add(FREE_CACHE_TTL_SECONDS),
            grace_until: None,
        };
        self.cache
            .store(&snapshot)
            .await
            .map_err(|_| PlanError::EntitlementsUnavailable)?;
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
            .current(suite_subject, self.clock.now().as_second())
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
    async fn membership(
        &self,
        organization_id: Uuid,
        suite_subject: &str,
    ) -> Result<Option<OrganizationRole>, PlanError> {
        match &self.organizations {
            Some(organizations) => {
                organizations
                    .membership(organization_id, suite_subject)
                    .await
            }
            None => Ok(None),
        }
    }
    async fn evict_member(&self, organization_id: Uuid, suite_subject: &str) {
        if let Some(organizations) = &self.organizations {
            organizations.evict(organization_id, suite_subject);
        }
    }
    async fn evict_organization(&self, organization_id: Uuid) -> Result<(), PlanError> {
        match &self.organizations {
            Some(organizations) => organizations.evict_organization(organization_id).await,
            None => Ok(()),
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
