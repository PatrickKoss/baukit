use axum::{
    Extension, Json, Router,
    extract::{FromRequestParts, State},
    http::{StatusCode, request::Parts},
    routing::get,
};
use baukit_http::{ApiError, ErrorEnvelope};
use std::{collections::BTreeMap, sync::Arc};
use uuid::Uuid;

use super::*;

/// The verified suite subject of the request. The product's authentication
/// layer inserts it; `None` or a missing extension answers 403
/// `suite_identity_missing`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PlanSubject(pub Option<String>);

/// The organization that owns the request's API token or worker. The product's
/// authentication layer inserts it; such a request has no suite subject.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlanOrganization(pub Uuid);

/// Reads the request's `FeatureSubject`: a `PlanOrganization` extension, else
/// the `X-Organization-Id` header with the `PlanSubject` as the member, else
/// the `PlanSubject` alone. A header naming another organization than
/// `PlanOrganization` is `organization_not_accessible`.
impl<S: Send + Sync> FromRequestParts<S> for FeatureSubject {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(feature_subject(parts)?)
    }
}

fn feature_subject(parts: &Parts) -> Result<FeatureSubject, PlanError> {
    let named = named_organization(parts)?;
    if let Some(PlanOrganization(owner)) = parts.extensions.get::<PlanOrganization>().copied() {
        return match named {
            Some(named) if named != owner => Err(PlanError::OrganizationNotAccessible),
            _ => Ok(FeatureSubject::Organization(owner)),
        };
    }
    let suite_subject = parts
        .extensions
        .get::<PlanSubject>()
        .and_then(|PlanSubject(subject)| subject.clone());
    Ok(match named {
        Some(organization_id) => FeatureSubject::Member {
            organization_id,
            suite_subject,
        },
        None => FeatureSubject::Account(suite_subject),
    })
}

fn named_organization(parts: &Parts) -> Result<Option<Uuid>, PlanError> {
    parts
        .headers
        .get(ORGANIZATION_HEADER)
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|value| Uuid::parse_str(value.trim()).ok())
                .ok_or(PlanError::OrganizationIdInvalid)
        })
        .transpose()
}

impl From<PlanError> for ApiError {
    fn from(error: PlanError) -> Self {
        let code = error.to_string();
        match error {
            PlanError::SuiteIdentityMissing => ApiError::new(
                StatusCode::FORBIDDEN,
                code,
                "Sign in with a suite account to use plans and credits",
            ),
            PlanError::EntitlementsUnavailable => ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                code,
                "Plans cannot be checked right now",
            )
            .with_retry_after(PLAN_RETRY_AFTER_SECONDS),
            PlanError::CreditsUnavailable => ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                code,
                "Credits cannot be read right now",
            )
            .with_retry_after(PLAN_RETRY_AFTER_SECONDS),
            PlanError::FeatureNotEntitled { feature } => ApiError::new(
                StatusCode::FORBIDDEN,
                code,
                "The current plan does not include this feature",
            )
            .with_details(BTreeMap::from([(
                "feature".to_owned(),
                serde_json::Value::String(feature),
            )])),
            PlanError::OrganizationNotAccessible => ApiError::new(
                StatusCode::FORBIDDEN,
                code,
                "The organization is not available to this account",
            ),
            PlanError::OrganizationIdInvalid => ApiError::new(
                StatusCode::BAD_REQUEST,
                code,
                "X-Organization-Id must be an organization UUID",
            ),
        }
    }
}

/// `GET /me/entitlement` and `GET /me/credits`. Nest it under the product's
/// API prefix behind the authentication layer that inserts `PlanSubject`.
pub fn account_plan_router(plan: Arc<dyn AccountPlan>) -> Router {
    Router::new()
        .route("/me/entitlement", get(entitlement))
        .route("/me/credits", get(credits))
        .with_state(plan)
}

#[derive(utoipa::OpenApi)]
#[openapi(
    paths(entitlement, credits),
    components(schemas(
        AccountEntitlement,
        EntitlementOrganization,
        PlanMode,
        BillingClaim,
        BillingChannel,
        CreditBalance,
        CreditGrant,
        CreditPack
    ))
)]
pub struct AccountPlanOpenApi;

fn subject(extension: Option<Extension<PlanSubject>>) -> Result<String, PlanError> {
    suite_identity(extension.and_then(|Extension(PlanSubject(subject))| subject))
}

#[utoipa::path(
    get,
    path = "/me/entitlement",
    operation_id = "getAccountEntitlement",
    description = "Verifies a fresh hub ticket and returns its plan, or the configured grant in standalone mode. `billing` is present only when the ticket carried the claim. With `X-Organization-Id`, returns that organization's grant for a member, with `organization` set and no `billing` or `plansUrl`.",
    security(("bearerAuth" = [])),
    params(
        ("X-Organization-Id" = Option<Uuid>, Header, description = "The organization the request acts for")
    ),
    responses(
        (status = 200, description = "Current plan", body = AccountEntitlement),
        (status = 400, description = "organization_id_invalid: `X-Organization-Id` is not a UUID", body = ErrorEnvelope),
        (status = 401, description = "Authentication required", body = ErrorEnvelope),
        (status = 403, description = "suite_identity_missing: the session has no suite subject; organization_not_accessible: the organization is unknown or the account is not a member", body = ErrorEnvelope),
        (status = 503, description = "entitlements_unavailable: retry after `Retry-After`", body = ErrorEnvelope)
    ),
    tag = "plan"
)]
async fn entitlement(
    State(plan): State<Arc<dyn AccountPlan>>,
    subject: FeatureSubject,
) -> Result<Json<AccountEntitlement>, ApiError> {
    Ok(Json(read_entitlement(plan.as_ref(), subject).await?))
}

async fn read_entitlement(
    plan: &dyn AccountPlan,
    subject: FeatureSubject,
) -> Result<AccountEntitlement, PlanError> {
    let (organization_id, suite_subject) = match subject {
        FeatureSubject::Account(suite_subject) => {
            return plan.refresh(&suite_identity(suite_subject)?).await;
        }
        FeatureSubject::Organization(_) => return Err(PlanError::SuiteIdentityMissing),
        FeatureSubject::Member {
            organization_id,
            suite_subject,
        } => (organization_id, suite_identity(suite_subject)?),
    };
    require_member(plan, organization_id, &suite_subject).await?;
    let grant = plan.organization_refresh(organization_id).await?;
    Ok(AccountEntitlement {
        mode: plan.mode(),
        suite_subject,
        tier: grant.tier,
        features: grant.features,
        revision: grant.revision,
        plans_url: None,
        billing: None,
        organization: Some(grant.organization),
    })
}

#[utoipa::path(
    get,
    path = "/me/credits",
    operation_id = "getAccountCredits",
    description = "The product's credit wallet from the hub. Standalone mode returns an empty wallet at revision 0.",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Credit balance", body = CreditBalance),
        (status = 401, description = "Authentication required", body = ErrorEnvelope),
        (status = 403, description = "suite_identity_missing: the session has no suite subject", body = ErrorEnvelope),
        (status = 503, description = "credits_unavailable: retry after `Retry-After`", body = ErrorEnvelope)
    ),
    tag = "plan"
)]
async fn credits(
    State(plan): State<Arc<dyn AccountPlan>>,
    extension: Option<Extension<PlanSubject>>,
) -> Result<Json<CreditBalance>, ApiError> {
    let subject = subject(extension)?;
    Ok(Json(plan.credits(&subject).await?))
}
