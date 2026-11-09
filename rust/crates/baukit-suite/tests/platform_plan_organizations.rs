#![cfg(test)]
#![cfg(feature = "platform")]
use async_trait::async_trait;
use axum::{
    Extension,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use baukit_suite::platform::*;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, AtomicUsize, Ordering},
    },
};
use tower::ServiceExt;
use uuid::Uuid;

const PRODUCT: &str = "ahp";
const ALICE: &str = "suite-test|alice";
const PLANS_URL: &str = "https://accounts.example.test/plans";
const PRIVACY: &str = "ahp.privacy";
const AUTOMATION: &str = "ahp.automation";
const STUDIO: Uuid = Uuid::from_u128(0x0a);
const AGENCY: Uuid = Uuid::from_u128(0x0b);
const STRANGERS: Uuid = Uuid::from_u128(0x0c);
const UNKNOWN: Uuid = Uuid::from_u128(0x0d);

#[derive(Default)]
struct ScriptedGrants {
    summaries: Mutex<BTreeMap<Uuid, Result<OrganizationSummary, OrganizationError>>>,
    pages: Mutex<BTreeMap<(String, Option<String>), SubjectOrganizationPage>>,
    membership_down: std::sync::atomic::AtomicBool,
    summary_calls: AtomicUsize,
    page_calls: AtomicUsize,
}
impl ScriptedGrants {
    fn organization(&self, id: Uuid, tier: &str, features: &[&str], revision: u64) {
        self.summaries.lock().expect("summaries").insert(
            id,
            Ok(OrganizationSummary {
                organization_id: id,
                name: Some(format!("Org {id}")),
                status: OrganizationStatus::Active,
                product_id: PRODUCT.into(),
                tier: tier.into(),
                features: features.iter().map(|f| (*f).to_owned()).collect(),
                paid_quantity: Some(5),
                assigned_seats: 1,
                revision,
                sso_connection_active: false,
                directory_sync_active: false,
            }),
        );
    }
    fn fail(&self, id: Uuid, error: OrganizationError) {
        self.summaries
            .lock()
            .expect("summaries")
            .insert(id, Err(error));
    }
    fn page(&self, subject: &str, cursor: Option<&str>, ids: &[Uuid], next: Option<&str>) {
        self.pages.lock().expect("pages").insert(
            (subject.into(), cursor.map(Into::into)),
            SubjectOrganizationPage {
                items: ids
                    .iter()
                    .map(|id| SubjectOrganization {
                        organization_id: *id,
                        account_role: OrganizationRole::Member,
                    })
                    .collect(),
                next_cursor: next.map(Into::into),
            },
        );
    }
}
#[async_trait]
impl OrganizationDirectory for ScriptedGrants {
    async fn summary(&self, id: Uuid) -> Result<OrganizationSummary, OrganizationError> {
        self.summary_calls.fetch_add(1, Ordering::SeqCst);
        self.summaries
            .lock()
            .expect("summaries")
            .get(&id)
            .cloned()
            .unwrap_or(Err(OrganizationError::NotFound))
    }
}
#[async_trait]
impl OrganizationGrants for ScriptedGrants {
    async fn subject_organizations(
        &self,
        subject: &str,
        cursor: Option<&str>,
    ) -> Result<SubjectOrganizationPage, OrganizationError> {
        self.page_calls.fetch_add(1, Ordering::SeqCst);
        if self.membership_down.load(Ordering::SeqCst) {
            return Err(OrganizationError::Unavailable);
        }
        Ok(self
            .pages
            .lock()
            .expect("pages")
            .get(&(subject.into(), cursor.map(Into::into)))
            .cloned()
            .unwrap_or(SubjectOrganizationPage {
                items: Vec::new(),
                next_cursor: None,
            }))
    }
}

/// Reads the in-memory cache at a clock the test moves forward.
#[derive(Default)]
struct ShiftedCache {
    inner: InMemoryEntitlementCache,
    offset: AtomicI64,
}
#[async_trait]
impl EntitlementCache for ShiftedCache {
    async fn get(
        &self,
        subject: &str,
        product_id: &str,
        now: i64,
    ) -> Result<Option<EntitlementSnapshot>, PlatformError> {
        let now = now + self.offset.load(Ordering::SeqCst);
        self.inner.get(subject, product_id, now).await
    }
    async fn revision(&self, subject: &str, product_id: &str) -> Result<u64, PlatformError> {
        self.inner.revision(subject, product_id).await
    }
    async fn forget(&self, subject: &str) -> Result<(), PlatformError> {
        self.inner.forget(subject).await
    }
    async fn store(&self, snapshot: &EntitlementSnapshot) -> Result<(), PlatformError> {
        let mut shifted = snapshot.clone();
        shifted.valid_until += self.offset.load(Ordering::SeqCst);
        self.inner.store(&shifted).await
    }
}

struct NoTickets;
#[async_trait]
impl TicketSource for NoTickets {
    async fn ticket(&self, _: &str) -> Result<String, EntitlementSourceError> {
        Err(EntitlementSourceError::Unavailable)
    }
}
struct NoKeys;
#[async_trait]
impl TicketJwksSource for NoKeys {
    async fn fetch(&self) -> Result<TicketJwks, TicketError> {
        Err(TicketError::KeysUnavailable)
    }
}
struct NoCredits;
#[async_trait]
impl Credits for NoCredits {
    async fn reserve(&self, _: &str, _: u64, _: &str) -> Result<CreditHold, CreditsError> {
        Err(CreditsError::Unavailable)
    }
    async fn settle(&self, _: Uuid, _: u64) -> Result<CreditReservation, CreditsError> {
        Err(CreditsError::Unavailable)
    }
    async fn release(&self, _: Uuid) -> Result<CreditReservation, CreditsError> {
        Err(CreditsError::Unavailable)
    }
    async fn balance(&self, _: &str) -> Result<CreditBalance, CreditsError> {
        Err(CreditsError::Unavailable)
    }
}

fn hub(grants: Arc<ScriptedGrants>, cache: Arc<ShiftedCache>) -> HubAccountPlan {
    let verifier = TicketVerifier::with_source(
        "https://accounts.example.test".into(),
        PRODUCT.into(),
        Arc::new(NoKeys),
    )
    .expect("verifier");
    HubAccountPlan::new(
        PRODUCT.into(),
        VerifiedTicketSource::new(Arc::new(NoTickets), verifier),
        cache,
        Arc::new(NoCredits),
        PLANS_URL,
    )
    .expect("plan")
    .with_organizations(grants)
}

/// Alice holds seats in a business studio and a starter agency.
fn alice_in_two_organizations() -> (Arc<ScriptedGrants>, Arc<ShiftedCache>, HubAccountPlan) {
    let grants = Arc::new(ScriptedGrants::default());
    grants.organization(STUDIO, "business", &[PRIVACY, AUTOMATION], 3);
    grants.organization(AGENCY, "starter", &[PRIVACY], 1);
    grants.organization(STRANGERS, "business", &[PRIVACY, AUTOMATION], 1);
    grants.page(ALICE, None, &[STUDIO], Some("next"));
    grants.page(ALICE, Some("next"), &[AGENCY], None);
    let cache = Arc::new(ShiftedCache::default());
    let plan = hub(grants.clone(), cache.clone());
    (grants, cache, plan)
}

fn member(organization_id: Uuid) -> FeatureSubject {
    FeatureSubject::Member {
        organization_id,
        suite_subject: Some(ALICE.into()),
    }
}

fn locked(feature: &str) -> Result<FeatureGrant, PlanError> {
    Err(PlanError::FeatureNotEntitled {
        feature: feature.into(),
    })
}

#[tokio::test]
async fn one_account_gets_each_organizations_own_tier() {
    let (_, _, plan) = alice_in_two_organizations();
    let studio = require_feature(&plan, member(STUDIO), AUTOMATION)
        .await
        .expect("studio automation");
    assert_eq!(
        (studio.tier.as_str(), studio.organization_id),
        ("business", Some(STUDIO))
    );
    assert_eq!(
        require_feature(&plan, member(AGENCY), AUTOMATION).await,
        locked(AUTOMATION)
    );
    let agency = require_feature(&plan, member(AGENCY), PRIVACY)
        .await
        .expect("agency privacy");
    assert_eq!(agency.tier, "starter");
    assert_eq!(
        require_feature(&plan, Some(ALICE), PRIVACY).await,
        Err(PlanError::EntitlementsUnavailable),
        "the account's own plan is a separate read"
    );
}

#[tokio::test]
async fn tokens_and_workers_check_the_organization_without_a_suite_subject() {
    let (grants, _, plan) = alice_in_two_organizations();
    let worker = require_feature(&plan, FeatureSubject::Organization(AGENCY), PRIVACY)
        .await
        .expect("worker");
    assert_eq!(worker.organization_id, Some(AGENCY));
    assert_eq!(
        require_feature(&plan, FeatureSubject::Organization(AGENCY), AUTOMATION).await,
        locked(AUTOMATION)
    );
    assert_eq!(
        grants.page_calls.load(Ordering::SeqCst),
        0,
        "an owned token needs no membership"
    );

    let plan: Arc<dyn AccountPlan> = Arc::new(plan);
    let token = Extension(PlanOrganization(STUDIO));
    let (status, _, body) = gate(plan.clone(), vec![], Some(token)).await;
    assert_eq!(
        (status, body["organizationId"].clone()),
        (StatusCode::OK, json!(STUDIO))
    );
    let (status, _, body) = gate(
        plan,
        vec![(ORGANIZATION_HEADER, AGENCY.to_string())],
        Some(token),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "organization_not_accessible");
}

#[tokio::test]
async fn a_lost_organization_grant_applies_within_the_cache_window() {
    let (grants, cache, plan) = alice_in_two_organizations();
    assert!(
        require_feature(&plan, member(STUDIO), AUTOMATION)
            .await
            .is_ok()
    );
    grants.organization(STUDIO, "starter", &[PRIVACY], 4);
    assert!(
        require_feature(&plan, member(STUDIO), AUTOMATION)
            .await
            .is_ok(),
        "the cached grant serves until the window ends"
    );
    assert_eq!(grants.summary_calls.load(Ordering::SeqCst), 1);
    cache
        .offset
        .store(ORGANIZATION_CACHE_TTL_SECONDS, Ordering::SeqCst);
    assert_eq!(
        require_feature(&plan, member(STUDIO), AUTOMATION).await,
        locked(AUTOMATION)
    );
    assert_eq!(grants.summary_calls.load(Ordering::SeqCst), 2);
    grants.fail(STUDIO, OrganizationError::NotFound);
    cache
        .offset
        .store(2 * ORGANIZATION_CACHE_TTL_SECONDS, Ordering::SeqCst);
    assert_eq!(
        require_feature(&plan, FeatureSubject::Organization(STUDIO), PRIVACY).await,
        Err(PlanError::OrganizationNotAccessible)
    );
}

#[tokio::test]
async fn a_suspended_organization_grants_nothing() {
    let (grants, _, plan) = alice_in_two_organizations();
    let mut summary = grants.summary(STUDIO).await.expect("summary");
    summary.status = OrganizationStatus::Suspended;
    grants
        .summaries
        .lock()
        .expect("summaries")
        .insert(STUDIO, Ok(summary));
    assert_eq!(
        require_feature(&plan, member(STUDIO), PRIVACY).await,
        locked(PRIVACY)
    );
}

#[tokio::test]
async fn a_non_member_cannot_tell_whether_the_organization_exists() {
    let (grants, _, plan) = alice_in_two_organizations();
    for organization in [STRANGERS, UNKNOWN] {
        assert_eq!(
            require_feature(&plan, member(organization), PRIVACY).await,
            Err(PlanError::OrganizationNotAccessible)
        );
    }
    assert_eq!(
        grants.summary_calls.load(Ordering::SeqCst),
        0,
        "membership is checked before the organization is read"
    );
    assert_eq!(
        require_feature(
            &plan,
            FeatureSubject::Member {
                organization_id: STUDIO,
                suite_subject: None
            },
            PRIVACY
        )
        .await,
        Err(PlanError::SuiteIdentityMissing)
    );
    let plan: Arc<dyn AccountPlan> = Arc::new(plan);
    let mut bodies = Vec::new();
    for organization in [STRANGERS, UNKNOWN] {
        let (status, _, body) = call(
            plan.clone(),
            vec![(ORGANIZATION_HEADER, organization.to_string())],
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let mut error = body["error"].clone();
        error.as_object_mut().expect("error").remove("requestId");
        bodies.push(error);
    }
    assert_eq!(bodies[0], bodies[1]);
    assert_eq!(bodies[0]["code"], "organization_not_accessible");
}

#[tokio::test]
async fn membership_is_cached_and_an_unreachable_hub_answers_503() {
    let (grants, _, plan) = alice_in_two_organizations();
    assert!(
        require_feature(&plan, member(AGENCY), PRIVACY)
            .await
            .is_ok()
    );
    assert_eq!(grants.page_calls.load(Ordering::SeqCst), 2);
    assert!(
        require_feature(&plan, member(AGENCY), PRIVACY)
            .await
            .is_ok()
    );
    assert_eq!(grants.page_calls.load(Ordering::SeqCst), 2);

    grants.membership_down.store(true, Ordering::SeqCst);
    assert_eq!(
        require_feature(&plan, member(STRANGERS), PRIVACY).await,
        Err(PlanError::EntitlementsUnavailable)
    );
    grants.membership_down.store(false, Ordering::SeqCst);
    grants.fail(STUDIO, OrganizationError::Unavailable);
    let plan: Arc<dyn AccountPlan> = Arc::new(plan);
    let (status, retry, body) = call(plan, vec![(ORGANIZATION_HEADER, STUDIO.to_string())]).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(retry.as_deref(), Some("5"));
    assert_eq!(body["error"]["code"], "entitlements_unavailable");
}

#[tokio::test]
async fn a_repeated_membership_cursor_is_unavailable() {
    let grants = Arc::new(ScriptedGrants::default());
    grants.page(ALICE, None, &[], Some("loop"));
    grants.page(ALICE, Some("loop"), &[], Some("loop"));
    let plan = hub(grants, Arc::default());
    assert_eq!(
        require_feature(&plan, member(STUDIO), PRIVACY).await,
        Err(PlanError::EntitlementsUnavailable)
    );
}

#[tokio::test]
async fn the_entitlement_route_serves_a_members_organization_plan() {
    let (_, _, plan) = alice_in_two_organizations();
    let plan: Arc<dyn AccountPlan> = Arc::new(plan);
    let (status, _, body) = call(
        plan.clone(),
        vec![(ORGANIZATION_HEADER, AGENCY.to_string())],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({
            "mode": "hub",
            "suiteSubject": ALICE,
            "tier": "starter",
            "features": [PRIVACY],
            "revision": 1,
            "plansUrl": null,
            "billing": null,
            "organization": {"organizationId": AGENCY, "name": format!("Org {AGENCY}")},
        })
    );
    let (status, _, body) = call(plan, vec![(ORGANIZATION_HEADER, "studio".into())]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "organization_id_invalid");
}

#[tokio::test]
async fn a_plan_without_organization_support_fails_closed() {
    let verifier = TicketVerifier::with_source(
        "https://accounts.example.test".into(),
        PRODUCT.into(),
        Arc::new(NoKeys),
    )
    .expect("verifier");
    let plan = HubAccountPlan::new(
        PRODUCT.into(),
        VerifiedTicketSource::new(Arc::new(NoTickets), verifier),
        Arc::new(InMemoryEntitlementCache::default()),
        Arc::new(NoCredits),
        PLANS_URL,
    )
    .expect("plan");
    for subject in [member(STUDIO), FeatureSubject::Organization(STUDIO)] {
        assert_eq!(
            require_feature(&plan, subject, PRIVACY).await,
            Err(PlanError::OrganizationNotAccessible)
        );
    }
}

#[tokio::test]
async fn standalone_organizations_get_the_configured_grant() {
    let plan = StandaloneAccountPlan::new(
        PRODUCT.into(),
        StandaloneEntitlements {
            tier: "business".into(),
            features: vec![PRIVACY.into()],
        },
    );
    for subject in [member(STUDIO), FeatureSubject::Organization(AGENCY)] {
        assert_eq!(
            require_feature(&plan, subject.clone(), PRIVACY)
                .await
                .expect("grant")
                .tier,
            "business"
        );
        assert_eq!(
            require_feature(&plan, subject, AUTOMATION).await,
            locked(AUTOMATION)
        );
    }
}

#[test]
fn openapi_documents_the_organization_header_and_codes() {
    let document =
        serde_json::to_value(<AccountPlanOpenApi as utoipa::OpenApi>::openapi()).expect("openapi");
    let operation = &document["paths"]["/me/entitlement"]["get"];
    assert_eq!(operation["parameters"][0]["name"], "X-Organization-Id");
    assert!(
        operation["responses"]["403"]["description"]
            .as_str()
            .is_some_and(|text| text.contains("organization_not_accessible"))
    );
    assert!(document["components"]["schemas"]["EntitlementOrganization"].is_object());
}

async fn call(
    plan: Arc<dyn AccountPlan>,
    headers: Vec<(&str, String)>,
) -> (StatusCode, Option<String>, Value) {
    let router = account_plan_router(plan).layer(Extension(PlanSubject(Some(ALICE.into()))));
    send(router, "/me/entitlement", headers).await
}

async fn gate(
    plan: Arc<dyn AccountPlan>,
    headers: Vec<(&str, String)>,
    token: Option<Extension<PlanOrganization>>,
) -> (StatusCode, Option<String>, Value) {
    let mut router = axum::Router::new()
        .route(
            "/gate",
            axum::routing::get(
                |axum::extract::State(plan): axum::extract::State<Arc<dyn AccountPlan>>,
                 subject: FeatureSubject| async move {
                    require_feature(plan.as_ref(), subject, PRIVACY)
                        .await
                        .map(|grant| axum::Json(json!({"organizationId": grant.organization_id})))
                        .map_err(baukit_http::ApiError::from)
                },
            ),
        )
        .with_state(plan);
    if let Some(token) = token {
        router = router.layer(token);
    }
    send(router, "/gate", headers).await
}

async fn send(
    router: axum::Router,
    path: &str,
    headers: Vec<(&str, String)>,
) -> (StatusCode, Option<String>, Value) {
    let mut request = Request::get(path);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response = router
        .oneshot(request.body(Body::empty()).expect("request"))
        .await
        .expect("response");
    let status = response.status();
    let retry = response
        .headers()
        .get("retry-after")
        .map(|value| value.to_str().expect("header").to_owned());
    let body = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body");
    (status, retry, serde_json::from_slice(&body).expect("json"))
}
