use axum::{Extension, Json, Router, extract::State, http::StatusCode, routing::get};
use baukit_http::{ApiError, ErrorEnvelope};
use std::{collections::BTreeMap, sync::Arc};

use super::*;

/// The verified suite subject of the request. The product's authentication
/// layer inserts it; `None` or a missing extension answers 403
/// `suite_identity_missing`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PlanSubject(pub Option<String>);

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
    extension
        .and_then(|Extension(PlanSubject(subject))| subject)
        .filter(|subject| !subject.is_empty())
        .ok_or(PlanError::SuiteIdentityMissing)
}

#[utoipa::path(
    get,
    path = "/me/entitlement",
    operation_id = "getAccountEntitlement",
    description = "Verifies a fresh hub ticket and returns its plan, or the configured grant in standalone mode. `billing` is present only when the ticket carried the claim.",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Current plan", body = AccountEntitlement),
        (status = 401, description = "Authentication required", body = ErrorEnvelope),
        (status = 403, description = "suite_identity_missing: the session has no suite subject", body = ErrorEnvelope),
        (status = 503, description = "entitlements_unavailable: retry after `Retry-After`", body = ErrorEnvelope)
    ),
    tag = "plan"
)]
async fn entitlement(
    State(plan): State<Arc<dyn AccountPlan>>,
    extension: Option<Extension<PlanSubject>>,
) -> Result<Json<AccountEntitlement>, ApiError> {
    let subject = subject(extension)?;
    Ok(Json(plan.refresh(&subject).await?))
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
