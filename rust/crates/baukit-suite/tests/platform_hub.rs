#![cfg(feature = "platform")]
use axum::{
    Json, Router,
    body::{Bytes, to_bytes},
    extract::{Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use baukit_config::{Secret, Validate};
use baukit_egress::{AddressPolicy, EgressOptions, GuardedClient};
use baukit_suite::platform::*;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use uuid::Uuid;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const RESPONSES: &str = include_str!("../../../../fixtures/platform-api/v1/responses.json");
const ENVELOPES: &str = include_str!("../../../../fixtures/platform-events/v1/envelopes.json");
const TOKEN_PATH: &str = "/realms/suite/protocol/openid-connect/token";
const SECRET: &str = "recorded-client-secret";
const SUBJECT: &str = "suite-dev|alice";
const RESERVATION: &str = "018f1234-5678-7abc-8def-012345678901";
const ORGANIZATION: &str = "018f1234-5678-7abc-8def-012345678901";

fn recorded(name: &str) -> (u16, Value) {
    let responses: Value = serde_json::from_str(RESPONSES).expect("recorded responses");
    let entry = &responses["responses"][name];
    let status = entry["status"].as_u64().expect("status");
    (
        u16::try_from(status).expect("status"),
        entry["body"].clone(),
    )
}

#[derive(Clone, Debug)]
struct Recorded {
    method: String,
    path: String,
    authorization: Option<String>,
    body: Option<Value>,
}

#[derive(Default)]
struct HubState {
    script: Mutex<VecDeque<(u16, Value)>>,
    requests: Mutex<Vec<Recorded>>,
    tokens: Mutex<VecDeque<(u16, Value)>>,
    token_forms: Mutex<Vec<String>>,
}
impl HubState {
    fn script(&self, names: &[&str]) {
        self.script
            .lock()
            .expect("script")
            .extend(names.iter().map(|name| recorded(name)));
    }
    fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().expect("requests").clone()
    }
    fn token_calls(&self) -> usize {
        self.token_forms.lock().expect("forms").len()
    }
}

fn respond(status: u16, body: Value) -> Response {
    (StatusCode::from_u16(status).expect("status"), Json(body)).into_response()
}

async fn token(State(state): State<Arc<HubState>>, body: Bytes) -> Response {
    state
        .token_forms
        .lock()
        .expect("forms")
        .push(String::from_utf8(body.to_vec()).expect("form"));
    let issued = state.token_forms.lock().expect("forms").len();
    let (status, mut body) = state
        .tokens
        .lock()
        .expect("tokens")
        .pop_front()
        .unwrap_or_else(|| recorded("token"));
    if status == 200 {
        body["access_token"] = json!(format!("recorded-platform-token-{issued}"));
    }
    respond(status, body)
}

async fn platform(State(state): State<Arc<HubState>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, 64 * 1024).await.expect("body");
    state.requests.lock().expect("requests").push(Recorded {
        method: parts.method.to_string(),
        path: parts.uri.path().to_owned(),
        authorization: parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        body: (!body.is_empty()).then(|| serde_json::from_slice(&body).expect("json body")),
    });
    let (status, body) = state
        .script
        .lock()
        .expect("script")
        .pop_front()
        .unwrap_or((500, json!({})));
    respond(status, body)
}

struct MockHub {
    state: Arc<HubState>,
    base: String,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for MockHub {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl MockHub {
    async fn start() -> Result<Self, Box<dyn std::error::Error>> {
        let state = Arc::new(HubState::default());
        let app = Router::new()
            .route(TOKEN_PATH, post(token))
            .fallback(platform)
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("mock hub");
        });
        Ok(Self {
            state,
            base,
            server,
        })
    }
    fn config(&self) -> HubClientConfig {
        HubClientConfig {
            api_url: format!("{}/api", self.base),
            token_url: format!("{}{TOKEN_PATH}", self.base),
            client_id: None,
            client_secret: Some(Secret::new(SECRET.into())),
        }
    }
    fn client(&self, product_id: &str) -> Result<Arc<HubClient>, Box<dyn std::error::Error>> {
        Ok(Arc::new(HubClient::new(
            &self.config(),
            product_id,
            guarded()?,
        )?))
    }
}

fn guarded() -> Result<GuardedClient, Box<dyn std::error::Error>> {
    Ok(GuardedClient::new(
        EgressOptions::default()
            .with_policy(AddressPolicy::AllowLoopback)
            .with_error_body_limit(16 * 1024)?,
    )?)
}

fn reservation_id() -> Uuid {
    Uuid::parse_str(RESERVATION).expect("uuid")
}

#[tokio::test]
async fn hub_credits_round_trip_recorded_responses() -> TestResult {
    let hub = MockHub::start().await?;
    let credits = HubCredits::new(hub.client("hebkit")?);
    hub.state.script(&[
        "creditBalance",
        "reservationPending",
        "reservationSettled",
        "reservationReleased",
    ]);

    let balance = credits.balance(SUBJECT).await?;
    assert_eq!(
        balance,
        serde_json::from_value(recorded("creditBalance").1)?
    );
    assert_eq!(balance.available, 10);
    assert_eq!(balance.packs[0].remaining, 10);

    assert_eq!(
        credits.reserve(SUBJECT, 0, "free-action").await?,
        CreditHold::Free
    );
    let CreditHold::Reserved(held) = credits.reserve(SUBJECT, 4, "job-1").await? else {
        return Err("paid action must reserve".into());
    };
    assert_eq!(held.status, ReservationStatus::Pending);
    assert_eq!(held.charged, None);
    let settled = credits.settle(held.id, 9).await?;
    assert_eq!(settled.status, ReservationStatus::Settled);
    assert_eq!(settled.charged, Some(4));
    let released_id = Uuid::parse_str("018f1234-5678-7abc-8def-012345678902")?;
    let released = credits.release(released_id).await?;
    assert_eq!(released.status, ReservationStatus::Released);

    let requests = hub.state.requests();
    let calls: Vec<_> = requests
        .iter()
        .map(|r| (r.method.as_str(), r.path.as_str(), r.body.clone()))
        .collect();
    assert_eq!(
        calls,
        vec![
            ("GET", "/api/platform/v1/credits/suite-dev|alice", None),
            (
                "POST",
                "/api/platform/v1/credits/reservations",
                Some(json!({"suiteSubject": SUBJECT, "amount": 4, "idempotencyKey": "job-1"}))
            ),
            (
                "POST",
                "/api/platform/v1/credits/reservations/018f1234-5678-7abc-8def-012345678901/settle",
                Some(json!({"actualAmount": 9}))
            ),
            (
                "POST",
                "/api/platform/v1/credits/reservations/018f1234-5678-7abc-8def-012345678902/release",
                Some(json!({}))
            ),
        ]
    );
    assert!(
        requests
            .iter()
            .all(|r| r.authorization.as_deref() == Some("Bearer recorded-platform-token-1"))
    );
    assert_eq!(
        hub.state.token_calls(),
        1,
        "one cached token serves every call"
    );
    let form = hub.state.token_forms.lock().expect("forms")[0].clone();
    assert_eq!(
        form,
        format!("grant_type=client_credentials&client_id=platform-hebkit&client_secret={SECRET}")
    );
    Ok(())
}

#[tokio::test]
async fn hub_credit_error_codes_map_to_typed_errors() -> TestResult {
    let hub = MockHub::start().await?;
    let credits = HubCredits::new(hub.client("hebkit")?);
    for (name, expected) in [
        ("creditsInsufficient", CreditsError::Insufficient),
        (
            "creditsIdempotencyConflict",
            CreditsError::IdempotencyConflict,
        ),
        (
            "creditsReservationExpired",
            CreditsError::ReservationExpired,
        ),
        (
            "creditsReservationNotFound",
            CreditsError::ReservationNotFound,
        ),
        ("creditsWalletUnknown", CreditsError::WalletUnknown),
        ("creditsAmountInvalid", CreditsError::AmountInvalid),
        (
            "creditsIdempotencyKeyInvalid",
            CreditsError::IdempotencyKeyInvalid,
        ),
        ("creditsUnavailable", CreditsError::Unavailable),
        ("subjectNotFound", CreditsError::SubjectNotFound),
        ("accountInactive", CreditsError::AccountInactive),
        (
            "scopeMissing",
            CreditsError::Hub(HubError::Rejected {
                status: 403,
                code: Some("platform_scope_missing".into()),
            }),
        ),
    ] {
        for call in 0..3 {
            hub.state.script(&[name]);
            let result = match call {
                0 => credits.reserve(SUBJECT, 4, "job-1").await.map(|_| ()),
                1 => credits.settle(reservation_id(), 1).await.map(|_| ()),
                _ => credits.balance(SUBJECT).await.map(|_| ()),
            };
            assert_eq!(result, Err(expected.clone()), "{name} call {call}");
        }
    }
    assert_eq!(hub.state.token_calls(), 1);
    Ok(())
}

#[tokio::test]
async fn hub_credits_reject_another_wallet_or_subject() -> TestResult {
    let hub = MockHub::start().await?;
    let credits = HubCredits::new(hub.client("eigenruhe")?);
    let invalid = CreditsError::Hub(HubError::InvalidResponse);
    hub.state.script(&["creditBalance", "reservationPending"]);
    assert_eq!(credits.balance(SUBJECT).await, Err(invalid.clone()));
    assert_eq!(
        credits.reserve(SUBJECT, 4, "job-1").await,
        Err(invalid.clone())
    );
    let credits = HubCredits::new(hub.client("hebkit")?);
    hub.state.script(&["creditBalance", "reservationPending"]);
    assert_eq!(credits.balance("suite-dev|bob").await, Err(invalid.clone()));
    assert_eq!(
        credits.reserve(SUBJECT, 5, "job-1").await,
        Err(invalid),
        "a reservation for another amount is not this request's reservation"
    );
    Ok(())
}

#[tokio::test]
async fn hub_client_refreshes_once_after_401() -> TestResult {
    let hub = MockHub::start().await?;
    let credits = HubCredits::new(hub.client("hebkit")?);
    hub.state.script(&["unauthenticated", "creditBalance"]);
    credits.balance(SUBJECT).await?;
    let requests = hub.state.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some("Bearer recorded-platform-token-1")
    );
    assert_eq!(
        requests[1].authorization.as_deref(),
        Some("Bearer recorded-platform-token-2")
    );

    hub.state.script(&["unauthenticated", "unauthenticated"]);
    assert_eq!(
        credits.balance(SUBJECT).await,
        Err(CreditsError::Hub(HubError::Unauthorized))
    );
    assert_eq!(hub.state.requests().len(), 4, "only one retry per call");
    assert_eq!(hub.state.token_calls(), 3);
    Ok(())
}

#[tokio::test]
async fn hub_client_refreshes_tokens_before_expiry() -> TestResult {
    let hub = MockHub::start().await?;
    let (_, mut short) = recorded("token");
    short["expires_in"] = json!(2);
    hub.state
        .tokens
        .lock()
        .expect("tokens")
        .extend([(200, short.clone()), (200, short)]);
    let credits = HubCredits::new(hub.client("hebkit")?);
    hub.state
        .script(&["creditBalance", "creditBalance", "creditBalance"]);
    credits.balance(SUBJECT).await?;
    credits.balance(SUBJECT).await?;
    assert_eq!(hub.state.token_calls(), 1);
    // A two-second token is replaced after half its lifetime.
    tokio::time::sleep(Duration::from_millis(1100)).await;
    credits.balance(SUBJECT).await?;
    assert_eq!(hub.state.token_calls(), 2);
    assert_eq!(
        hub.state.requests()[2].authorization.as_deref(),
        Some("Bearer recorded-platform-token-2")
    );
    Ok(())
}

#[tokio::test]
async fn hub_client_reports_rejected_credentials_without_the_secret() -> TestResult {
    let hub = MockHub::start().await?;
    hub.state.tokens.lock().expect("tokens").push_back((
        401,
        json!({"error": "unauthorized_client", "error_description": "Invalid client or Invalid client credentials"}),
    ));
    hub.state
        .tokens
        .lock()
        .expect("tokens")
        .push_back((400, json!({"error": "invalid_client"})));
    let client = hub.client("hebkit")?;
    let credits = HubCredits::new(client.clone());
    let unauthorized = Err(CreditsError::Hub(HubError::Unauthorized));
    assert_eq!(credits.balance(SUBJECT).await, unauthorized);
    assert_eq!(credits.balance(SUBJECT).await, unauthorized);
    assert!(hub.state.requests().is_empty());
    let config = hub.config();
    assert!(!format!("{config:?}").contains(SECRET));
    assert!(!format!("{unauthorized:?}").contains(SECRET));
    Ok(())
}

#[tokio::test]
async fn hub_client_settings_are_validated() -> TestResult {
    let hub = MockHub::start().await?;
    let mut config = hub.config();
    assert!(config.validate().is_ok());
    let plain =
        GuardedClient::new(EgressOptions::default().with_policy(AddressPolicy::AllowLoopback))?;
    assert!(
        HubClient::new(&config, "hebkit", plain).is_err(),
        "error bodies are required for typed hub errors"
    );
    assert!(HubClient::new(&config, "", guarded()?).is_err());
    config.client_id = Some("custom-client".into());
    assert!(HubClient::new(&config, "hebkit", guarded()?).is_ok());
    for broken in [
        HubClientConfig {
            api_url: "https://user:pass@hub.example".into(),
            ..hub.config()
        },
        HubClientConfig {
            token_url: "https://hub.example/token#fragment".into(),
            ..hub.config()
        },
        HubClientConfig {
            client_secret: Some(Secret::new(String::new())),
            ..hub.config()
        },
        HubClientConfig {
            client_id: Some(" padded".into()),
            ..hub.config()
        },
    ] {
        assert!(broken.validate().is_err());
    }
    Ok(())
}

#[tokio::test]
async fn organization_summary_reads_recorded_responses() -> TestResult {
    let hub = MockHub::start().await?;
    let id = Uuid::parse_str(ORGANIZATION)?;
    let organizations = HubOrganizations::new(hub.client("runtime_analyzer")?);
    hub.state.script(&[
        "organizationSummary",
        "organizationNotFound",
        "creditsUnavailable",
    ]);
    let summary = organizations.summary(id).await?;
    assert_eq!(
        summary,
        OrganizationSummary {
            organization_id: id,
            status: OrganizationStatus::Active,
            product_id: "runtime_analyzer".into(),
            tier: "professional".into(),
            features: Vec::new(),
            paid_quantity: Some(4),
            assigned_seats: 1,
            revision: 1,
        }
    );
    assert_eq!(
        organizations.summary(id).await,
        Err(OrganizationError::NotFound)
    );
    assert_eq!(
        organizations.summary(id).await,
        Err(OrganizationError::Unavailable)
    );
    assert_eq!(
        hub.state.requests()[0].path,
        format!("/api/platform/v1/organizations/{ORGANIZATION}")
    );

    let (_, mut unlimited) = recorded("organizationSummary");
    unlimited["paidQuantity"] = Value::Null;
    hub.state
        .script
        .lock()
        .expect("script")
        .push_back((200, unlimited));
    assert_eq!(organizations.summary(id).await?.paid_quantity, None);

    let foreign = HubOrganizations::new(hub.client("hebkit")?);
    hub.state
        .script(&["organizationSummary", "organizationSummary"]);
    let invalid = Err(OrganizationError::Hub(HubError::InvalidResponse));
    assert_eq!(foreign.summary(id).await, invalid);
    assert_eq!(organizations.summary(Uuid::new_v4()).await, invalid);
    Ok(())
}

#[tokio::test]
async fn zero_cost_credits_accept_free_actions_and_refuse_paid_ones() -> TestResult {
    let config = PlatformConfig {
        product_id: "hebkit".into(),
        ..PlatformConfig::default()
    };
    let credits = config.credits(None)?;
    assert_eq!(credits.reserve(SUBJECT, 0, "free").await?, CreditHold::Free);
    assert_eq!(
        credits.reserve(SUBJECT, 1, "paid").await,
        Err(CreditsError::WalletUnknown)
    );
    assert_eq!(
        credits
            .reserve(SUBJECT, MAX_CREDIT_AMOUNT + 1, "huge")
            .await,
        Err(CreditsError::AmountInvalid)
    );
    assert_eq!(
        credits.settle(reservation_id(), 0).await,
        Err(CreditsError::ReservationNotFound)
    );
    assert_eq!(
        credits.release(reservation_id()).await,
        Err(CreditsError::ReservationNotFound)
    );
    let balance = credits.balance(SUBJECT).await?;
    assert_eq!((balance.wallet.as_str(), balance.available), ("hebkit", 0));
    Ok(())
}

#[tokio::test]
async fn hub_mode_credits_require_a_matching_hub_client() -> TestResult {
    let hub = MockHub::start().await?;
    let config = PlatformConfig {
        hub_issuer: Some("https://hub.example".into()),
        jwks_url: Some("https://hub.example/keys".into()),
        product_id: "hebkit".into(),
        source_app: "schluesselbund".into(),
        receiver_secret: Some(Secret::new("receiver".into())),
        ..PlatformConfig::default()
    };
    assert!(config.credits(None).is_err());
    assert!(config.credits(Some(hub.client("eigenruhe")?)).is_err());
    let credits = config.credits(Some(hub.client("hebkit")?))?;
    hub.state.script(&["creditBalance"]);
    assert_eq!(credits.balance(SUBJECT).await?.wallet, "hebkit");
    assert_eq!(credits.reserve(SUBJECT, 0, "free").await?, CreditHold::Free);
    assert_eq!(
        hub.state.requests().len(),
        1,
        "zero cost makes no ledger call"
    );
    Ok(())
}

#[test]
fn organization_and_credit_events_decode_with_their_owner() -> TestResult {
    let decoder = PlatformEventDecoder::new("schluesselbund".into(), None)?;
    let now = chrono::DateTime::from_timestamp(1_800_000_000, 0).ok_or("time")?;
    let envelopes: Vec<Value> = serde_json::from_str(ENVELOPES)?;
    let find = |kind: &str| {
        envelopes
            .iter()
            .find(|e| e["type"] == kind)
            .cloned()
            .ok_or("fixture")
    };
    let mut organization = find("schluesselbund.organization.updated")?;
    let event = decoder.decode(&serde_json::to_vec(&organization)?, now)?;
    assert_eq!(
        event.kind(),
        &PlatformEventKind::OrganizationUpdated(OrganizationUpdated {
            organization_id: Uuid::parse_str(ORGANIZATION)?,
            revision: 7,
        })
    );
    organization["userId"] = json!("suite|person-123");
    assert_eq!(
        decoder.decode(&serde_json::to_vec(&organization)?, now),
        Err(PlatformError::Invalid)
    );
    let credits = find("schluesselbund.credits.changed")?;
    let event = decoder.decode(&serde_json::to_vec(&credits)?, now)?;
    assert_eq!(
        event.kind(),
        &PlatformEventKind::CreditsChanged(CreditsChanged {
            suite_subject: "suite|person-123".into(),
            wallet: "hebkit".into(),
            revision: 4,
        })
    );
    Ok(())
}
