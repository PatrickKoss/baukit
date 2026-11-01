use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use url::Url;

use super::*;

/// Seconds a client waits before retrying a `503` plan or credit read.
pub const PLAN_RETRY_AFTER_SECONDS: u64 = 5;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanMode {
    Hub,
    Standalone,
}

/// The plan a product gates on. `billing` is set only when the verified
/// ticket carried the claim, so cached reads never include it.
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
}

#[async_trait]
pub trait AccountPlan: Send + Sync {
    fn mode(&self) -> PlanMode;
    /// A freshly verified plan for `GET /me/entitlement`.
    async fn refresh(&self, suite_subject: &str) -> Result<AccountEntitlement, PlanError>;
    /// The cached plan for request gating.
    async fn current(&self, suite_subject: &str) -> Result<AccountEntitlement, PlanError>;
    async fn credits(&self, suite_subject: &str) -> Result<CreditBalance, PlanError>;
}

/// Grants the plan when it includes `feature`. A missing subject, a failed
/// read and a locked feature are three distinct errors.
pub async fn require_feature(
    plan: &dyn AccountPlan,
    suite_subject: Option<&str>,
    feature: &str,
) -> Result<AccountEntitlement, PlanError> {
    let subject = suite_subject
        .filter(|subject| !subject.is_empty())
        .ok_or(PlanError::SuiteIdentityMissing)?;
    let entitlement = plan.current(subject).await?;
    if entitlement.has_feature(feature) {
        return Ok(entitlement);
    }
    Err(PlanError::FeatureNotEntitled {
        feature: feature.into(),
    })
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
}
#[derive(Deserialize)]
struct TicketResponse {
    ticket: String,
}

/// `POST /platform/v1/entitlements/tickets` with service credentials. The
/// platform route never adds the `billing` claim.
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
        })
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
    }
}
