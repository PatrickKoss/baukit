#![cfg(feature = "platform")]
use async_trait::async_trait;
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair as _};
use axum::{
    Extension,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use baukit_suite::platform::*;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tower::ServiceExt;
use uuid::Uuid;

const TICKETS: &str = include_str!("../../../../fixtures/entitlement-tickets/v1/vectors.json");
const BILLING: &str =
    include_str!("../../../../fixtures/entitlement-tickets/v1/billing-vectors.json");
const CATALOG: &str = include_str!("../../../../fixtures/plans/v1/catalog.json");
const PRODUCT: &str = "eigenruhe";
const SUBJECT: &str = "suite-test|alice";
const PLANS_URL: &str = "https://accounts.example.test/plans";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    issuer: String,
    test_seed_hex: String,
    jwks: TicketJwks,
}
#[derive(Deserialize)]
struct BillingFixture {
    issuer: String,
    jwks: TicketJwks,
    vectors: Vec<Vector>,
}
#[derive(Deserialize)]
struct Vector {
    name: String,
    ticket: String,
    audience: String,
    now: i64,
    expected: String,
    claims: Option<TicketClaims>,
}

#[test]
fn billing_claim_vectors_verify_or_fail_as_recorded() {
    let fixture: BillingFixture = serde_json::from_str(BILLING).expect("fixture");
    for v in fixture.vectors {
        let result = verify_ticket(
            &v.ticket,
            &fixture.jwks,
            &fixture.issuer,
            &v.audience,
            v.now,
        );
        if v.expected == "ok" {
            let claims = result.expect(&v.name);
            assert!(claims.billing.is_some(), "{}", v.name);
            assert_eq!(Some(claims), v.claims, "{}", v.name);
        } else {
            assert_eq!(
                result.expect_err(&v.name).to_string(),
                v.expected,
                "{}",
                v.name
            );
        }
    }
}

#[test]
fn billing_claim_round_trips_its_wire_names() {
    let claim: BillingClaim =
        serde_json::from_value(json!({"channel":"app_store","soldBy":"hebkit"})).expect("claim");
    assert_eq!(claim.channel, BillingChannel::AppStore);
    assert_eq!(
        serde_json::to_value(&claim).expect("json"),
        json!({"channel":"app_store","soldBy":"hebkit"})
    );
}

fn seed() -> Vec<u8> {
    let fixture: Fixture = serde_json::from_str(TICKETS).expect("fixture");
    (0..fixture.test_seed_hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&fixture.test_seed_hex[i..i + 2], 16).expect("hex"))
        .collect()
}
fn sign(claims: &Value) -> String {
    let fixture: Fixture = serde_json::from_str(TICKETS).expect("fixture");
    let pair = Ed25519KeyPair::from_seed_unchecked(&seed()).expect("seed");
    assert_eq!(
        URL_SAFE_NO_PAD.encode(pair.public_key().as_ref()),
        fixture.jwks.keys[0].x
    );
    let header = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&json!({"alg":"EdDSA","kid":fixture.jwks.keys[0].kid})).expect("header"),
    );
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).expect("claims"));
    let input = format!("{header}.{payload}");
    let signature = URL_SAFE_NO_PAD.encode(pair.sign(input.as_bytes()).as_ref());
    format!("{input}.{signature}")
}
fn claims(features: &[&str], revision: u64, billing: Option<Value>) -> Value {
    let fixture: Fixture = serde_json::from_str(TICKETS).expect("fixture");
    let now = chrono::Utc::now().timestamp();
    let mut claims = json!({
        "iss": fixture.issuer,
        "sub": SUBJECT,
        "aud": PRODUCT,
        "tier": if features.is_empty() { "free" } else { "premium" },
        "features": features,
        "rev": revision,
        "iat": now - 1,
        "exp": now + 3600,
    });
    if let Some(billing) = billing {
        claims["billing"] = billing;
    }
    claims
}

struct StaticKeys;
#[async_trait]
impl TicketJwksSource for StaticKeys {
    async fn fetch(&self) -> Result<TicketJwks, TicketError> {
        let fixture: Fixture = serde_json::from_str(TICKETS).expect("fixture");
        Ok(fixture.jwks)
    }
}

#[derive(Default)]
struct ScriptedTickets {
    next: Mutex<Option<Result<String, EntitlementSourceError>>>,
    calls: AtomicUsize,
}
impl ScriptedTickets {
    fn answer(&self, answer: Result<String, EntitlementSourceError>) {
        *self.next.lock().expect("next") = Some(answer);
    }
}
#[async_trait]
impl TicketSource for ScriptedTickets {
    async fn ticket(&self, _: &str) -> Result<String, EntitlementSourceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.next
            .lock()
            .expect("next")
            .clone()
            .unwrap_or(Err(EntitlementSourceError::Unavailable))
    }
}

struct FailingCredits;
#[async_trait]
impl Credits for FailingCredits {
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

fn hub_plan(tickets: Arc<ScriptedTickets>) -> HubAccountPlan {
    let fixture: Fixture = serde_json::from_str(TICKETS).expect("fixture");
    let verifier =
        TicketVerifier::with_source(fixture.issuer, PRODUCT.into(), Arc::new(StaticKeys))
            .expect("verifier");
    HubAccountPlan::new(
        PRODUCT.into(),
        VerifiedTicketSource::new(tickets, verifier),
        Arc::new(InMemoryEntitlementCache::default()),
        Arc::new(FailingCredits),
        PLANS_URL,
    )
    .expect("plan")
}

fn standalone(features: &[&str]) -> StandaloneAccountPlan {
    StandaloneAccountPlan::new(
        PRODUCT.into(),
        StandaloneEntitlements {
            tier: "premium".into(),
            features: features.iter().map(|f| (*f).to_owned()).collect(),
        },
    )
}

#[tokio::test]
async fn standalone_plan_serves_the_configured_grant_and_an_empty_wallet() {
    let plan = standalone(&["eigenruhe.nachtruhe"]);
    let entitlement = plan.refresh(SUBJECT).await.expect("plan");
    assert_eq!(plan.mode(), PlanMode::Standalone);
    assert_eq!(
        serde_json::to_value(&entitlement).expect("json"),
        json!({
            "mode": "standalone",
            "suiteSubject": SUBJECT,
            "tier": "premium",
            "features": ["eigenruhe.nachtruhe"],
            "revision": 0,
            "plansUrl": null,
            "billing": null,
        })
    );
    let credits = plan.credits(SUBJECT).await.expect("credits");
    assert_eq!((credits.total, credits.revision), (0, 0));
    assert_eq!(credits.wallet, PRODUCT);
}

#[tokio::test]
async fn require_feature_separates_identity_lock_and_outage() {
    let plan = standalone(&["eigenruhe.nachtruhe"]);
    assert!(
        require_feature(&plan, Some(SUBJECT), "eigenruhe.nachtruhe")
            .await
            .is_ok()
    );
    assert_eq!(
        require_feature(&plan, Some(SUBJECT), "eigenruhe.studio").await,
        Err(PlanError::FeatureNotEntitled {
            feature: "eigenruhe.studio".into()
        })
    );
    assert_eq!(
        require_feature(&plan, None, "eigenruhe.nachtruhe").await,
        Err(PlanError::SuiteIdentityMissing)
    );
    assert_eq!(
        require_feature(&plan, Some(""), "eigenruhe.nachtruhe").await,
        Err(PlanError::SuiteIdentityMissing)
    );
    let hub = hub_plan(Arc::default());
    assert_eq!(
        require_feature(&hub, Some(SUBJECT), "eigenruhe.nachtruhe").await,
        Err(PlanError::EntitlementsUnavailable)
    );
}

#[tokio::test]
async fn hub_refresh_passes_the_billing_claim_through_and_caches_the_grant() {
    let tickets = Arc::new(ScriptedTickets::default());
    let plan = hub_plan(tickets.clone());
    tickets.answer(Ok(sign(&claims(
        &["eigenruhe.custom_programs"],
        4,
        Some(json!({"channel":"app_store","soldBy":"hebkit"})),
    ))));
    let fresh = plan.refresh(SUBJECT).await.expect("refresh");
    assert_eq!(fresh.mode, PlanMode::Hub);
    assert_eq!(fresh.plans_url.as_deref(), Some(PLANS_URL));
    assert_eq!(
        fresh.billing,
        Some(BillingClaim {
            channel: BillingChannel::AppStore,
            sold_by: Some("hebkit".into()),
            manage_url: None,
        })
    );
    tickets.answer(Err(EntitlementSourceError::Unavailable));
    let cached = plan.current(SUBJECT).await.expect("cached");
    assert_eq!(cached.features, ["eigenruhe.custom_programs"]);
    assert_eq!(cached.revision, 4);
    assert_eq!(cached.billing, None);
    assert_eq!(tickets.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn hub_refresh_without_a_claim_or_entitlement() {
    let tickets = Arc::new(ScriptedTickets::default());
    let plan = hub_plan(tickets.clone());
    tickets.answer(Ok(sign(&claims(&[], 2, None))));
    assert_eq!(plan.refresh(SUBJECT).await.expect("free").billing, None);
    tickets.answer(Err(EntitlementSourceError::NoEntitlements));
    let free = plan.refresh(SUBJECT).await.expect("no entitlements");
    assert_eq!((free.tier.as_str(), free.revision), ("free", 2));
    assert!(free.features.is_empty());
}

#[tokio::test]
async fn hub_refresh_rejects_bad_tickets_as_unavailable() {
    let tickets = Arc::new(ScriptedTickets::default());
    let plan = hub_plan(tickets.clone());
    let mut other = claims(&["eigenruhe.studio"], 1, None);
    other["sub"] = json!("suite-test|mallory");
    for answer in [
        Ok(sign(&other)),
        Ok(sign(&claims(
            &["eigenruhe.studio"],
            1,
            Some(json!({"channel":"web","soldBy":"eigenruhe"})),
        ))),
        Ok("not-a-ticket".into()),
        Err(EntitlementSourceError::Invalid),
        Err(EntitlementSourceError::Unavailable),
    ] {
        tickets.answer(answer);
        assert_eq!(
            plan.refresh(SUBJECT).await,
            Err(PlanError::EntitlementsUnavailable)
        );
    }
    assert_eq!(
        plan.credits(SUBJECT).await,
        Err(PlanError::CreditsUnavailable)
    );
}

#[test]
fn hub_plan_needs_an_http_plans_url_and_a_product() {
    let fixture: Fixture = serde_json::from_str(TICKETS).expect("fixture");
    let verifier =
        TicketVerifier::with_source(fixture.issuer, PRODUCT.into(), Arc::new(StaticKeys))
            .expect("verifier");
    let source = VerifiedTicketSource::new(Arc::new(ScriptedTickets::default()), verifier);
    for (product, url) in [
        (PRODUCT, "/plans"),
        (PRODUCT, "javascript:alert(1)"),
        ("", PLANS_URL),
    ] {
        assert!(
            HubAccountPlan::new(
                product.into(),
                source.clone(),
                Arc::new(InMemoryEntitlementCache::default()),
                Arc::new(FailingCredits),
                url,
            )
            .is_err()
        );
    }
}

async fn call(
    plan: Arc<dyn AccountPlan>,
    path: &str,
    subject: Option<&str>,
) -> (StatusCode, Option<String>, Value) {
    let mut router = account_plan_router(plan);
    if let Some(subject) = subject {
        router = router.layer(Extension(PlanSubject(Some(subject.into()))));
    }
    let response = router
        .oneshot(Request::get(path).body(Body::empty()).expect("request"))
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

#[tokio::test]
async fn router_serves_the_plan_and_wallet() {
    let plan: Arc<dyn AccountPlan> = Arc::new(standalone(&["eigenruhe.nachtruhe"]));
    let (status, _, body) = call(plan.clone(), "/me/entitlement", Some(SUBJECT)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["mode"], "standalone");
    assert_eq!(body["features"], json!(["eigenruhe.nachtruhe"]));
    let (status, _, body) = call(plan, "/me/credits", Some(SUBJECT)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({"suiteSubject":SUBJECT,"wallet":PRODUCT,"revision":0,"total":0,"grant":{"available":0,"expiresAt":null},"packs":[]})
    );
}

#[tokio::test]
async fn hub_entitlement_route_serves_fresh_billing_on_every_read() {
    let tickets = Arc::new(ScriptedTickets::default());
    let plan: Arc<dyn AccountPlan> = Arc::new(hub_plan(tickets.clone()));
    tickets.answer(Ok(sign(&claims(
        &[],
        1,
        Some(json!({"channel":"web","manageUrl":PLANS_URL})),
    ))));
    let (status, _, body) = call(plan.clone(), "/me/entitlement", Some(SUBJECT)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["billing"],
        json!({"channel":"web","manageUrl":PLANS_URL})
    );
    tickets.answer(Ok(sign(&claims(&[], 2, Some(json!({"channel":"family"}))))));
    let (_, _, body) = call(plan.clone(), "/me/entitlement", Some(SUBJECT)).await;
    assert_eq!(body["billing"], json!({"channel":"family"}));
    assert_eq!(body["revision"], 2);
    assert_eq!(tickets.calls.load(Ordering::SeqCst), 2);
    tickets.answer(Err(EntitlementSourceError::Unavailable));
    let (status, _, body) = call(plan, "/me/entitlement", Some(SUBJECT)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"]["code"], "entitlements_unavailable");
}

#[tokio::test]
async fn router_uses_one_set_of_error_codes() {
    let plan: Arc<dyn AccountPlan> = Arc::new(standalone(&[]));
    for path in ["/me/entitlement", "/me/credits"] {
        let (status, retry, body) = call(plan.clone(), path, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(retry, None);
        assert_eq!(body["error"]["code"], "suite_identity_missing");
    }
    let hub: Arc<dyn AccountPlan> = Arc::new(hub_plan(Arc::default()));
    for (path, code) in [
        ("/me/entitlement", "entitlements_unavailable"),
        ("/me/credits", "credits_unavailable"),
    ] {
        let (status, retry, body) = call(hub.clone(), path, Some(SUBJECT)).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(retry.as_deref(), Some("5"));
        assert_eq!(body["error"]["code"], code);
    }
}

#[tokio::test]
async fn locked_feature_errors_name_the_feature() {
    let error: baukit_http::ApiError = PlanError::FeatureNotEntitled {
        feature: "eigenruhe.studio".into(),
    }
    .into();
    let response = axum::response::IntoResponse::into_response(error);
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body"),
    )
    .expect("json");
    assert_eq!(body["error"]["code"], "feature_not_entitled");
    assert_eq!(body["error"]["details"]["feature"], "eigenruhe.studio");
}

#[test]
fn openapi_documents_both_routes() {
    let document =
        serde_json::to_value(<AccountPlanOpenApi as utoipa::OpenApi>::openapi()).expect("openapi");
    for path in ["/me/entitlement", "/me/credits"] {
        assert!(document["paths"][path]["get"].is_object(), "{path}");
    }
    assert_eq!(
        document["components"]["schemas"]["BillingChannel"]["enum"],
        json!([
            "web",
            "app_store",
            "play_store",
            "family",
            "organization",
            "promo"
        ])
    );
}

#[test]
fn feature_keys_compare_with_the_recorded_hub_catalog() {
    let declared = [
        "eigenruhe.custom_programs",
        "eigenruhe.insights_extended",
        "eigenruhe.nachtruhe",
        "eigenruhe.studio",
    ];
    assert_feature_keys_match(CATALOG, PRODUCT, declared);
    let drift = feature_key_drift(
        CATALOG,
        PRODUCT,
        ["eigenruhe.custom_programs", "eigenruhe.offline_mode"],
    )
    .expect("drift");
    assert_eq!(drift.not_in_catalog, ["eigenruhe.offline_mode"]);
    assert_eq!(
        drift.not_declared,
        [
            "eigenruhe.insights_extended",
            "eigenruhe.nachtruhe",
            "eigenruhe.studio"
        ]
    );
    assert!(matches!(
        feature_key_drift(CATALOG, "unknown_product", []),
        Err(PlanCatalogError::UnknownProduct(_))
    ));
}

#[test]
#[should_panic(expected = "not declared")]
fn feature_key_assertion_reports_the_drift() {
    assert_feature_keys_match(CATALOG, PRODUCT, ["eigenruhe.custom_programs"]);
}

#[test]
fn organization_grants_count_as_catalog_keys() {
    let catalog = json!({"plans": [
        {"includedProducts": [{"productId": "hebkit", "features": ["hebkit.advanced_stats"]}]},
        {"includedProducts": [], "organizationProducts": null},
        {"includedProducts": [], "organizationProducts": [
            {"productId": "hebkit", "tier": "trainer", "features": ["hebkit.trainer"]}
        ]}
    ]})
    .to_string();
    assert_eq!(
        catalog_feature_keys(&catalog, "hebkit")
            .expect("keys")
            .into_iter()
            .collect::<Vec<_>>(),
        ["hebkit.advanced_stats", "hebkit.trainer"]
    );
    let only_organization = json!({"plans": [{"includedProducts": [], "organizationProducts": [
        {"productId": "hebkit", "features": ["hebkit.trainer"]}
    ]}]})
    .to_string();
    assert_feature_keys_match(&only_organization, "hebkit", ["hebkit.trainer"]);
}

#[test]
fn recorded_catalog_grants_hebkit_its_trainer_key() {
    assert_feature_keys_match(
        CATALOG,
        "hebkit",
        [
            "hebkit.advanced_stats",
            "hebkit.integrations_sync",
            "hebkit.nutrition_recommendations",
            "hebkit.trainer",
        ],
    );
}
