use std::{error::Error, path::PathBuf, sync::Arc};

use axum::{
    Json, Router,
    body::Body,
    http::{HeaderMap, Request, StatusCode},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use baukit_config::{Secret, Validate};
use baukit_core::webhook_signature::sign_webhook_hmac_sha256;
use baukit_suite::platform::{EntitlementSource, EntitlementSourceError, PostgresEntitlementCache};
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
use tower::ServiceExt;

use {{ context.app_crate }}_api::HubTicketSource;
use {{ context.app_crate }}_bin::{
    ProductConfig,
    platform::{compose, purge_receipts},
};

const SUBJECT: &str = "suite|person-123";
const PRODUCT: &str = "{{ context.app_crate }}";
const SECRET: &str = "fixture-receiver-secret";

struct Hub {
    base: String,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
}
impl Drop for Hub {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn hub() -> Result<Hub, Box<dyn Error>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let key = SigningKey::from_bytes(&[19; 32]);
    let jwks = json!({"keys": [{"kid": "fixture", "kty": "OKP", "crv": "Ed25519", "alg": "EdDSA", "use": "sig", "x": URL_SAFE_NO_PAD.encode(key.verifying_key().as_bytes())}]});
    let issuer = base.clone();
    let app = Router::new()
        .route("/keys", get(move || async move { Json(jwks) }))
        .route(
            "/tickets",
            post(
                move |headers: HeaderMap, Json(body): Json<Value>| async move {
                    assert_eq!(headers["authorization"], "Bearer fixture-service-token");
                    assert_eq!(body["productId"], PRODUCT);
                    let now = chrono::Utc::now().timestamp();
                    let subject = if body["suiteSubject"] == "suite|misbound" {
                        "suite|another"
                    } else {
                        body["suiteSubject"].as_str().expect("subject")
                    };
                    let claims = json!({"iss": issuer, "sub": subject, "aud": PRODUCT,
                "tier": "paid", "features": ["export"], "rev": 7, "iat": now, "exp": now + 3600});
                    let header =
                        URL_SAFE_NO_PAD.encode(br#"{"alg":"EdDSA","kid":"fixture","typ":"JWT"}"#);
                    let payload =
                        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).expect("claims"));
                    let input = format!("{header}.{payload}");
                    let signature = URL_SAFE_NO_PAD.encode(key.sign(input.as_bytes()).to_bytes());
                    Json(json!({"ticket": format!("{input}.{signature}")}))
                },
            ),
        );
    let task = tokio::spawn(async move { axum::serve(listener, app).await });
    Ok(Hub { base, task })
}

fn settings(base: &str) -> ProductConfig {
    let mut config = ProductConfig::default();
    config.platform.hub_issuer = Some(base.into());
    config.platform.jwks_url = Some(format!("{base}/keys"));
    config.platform.product_id = PRODUCT.into();
    config.platform.source_app = "identity_hub".into();
    config.platform.receiver_secret = Some(Secret::new(SECRET.into()));
    config.platform_hub.ticket_url = Some(format!("{base}/tickets"));
    config.platform_hub.service_token = Some(Secret::new("fixture-service-token".into()));
    config.platform_hub.allow_local_http = true;
    config
}

fn request(body: &str, secret: &str) -> Request<Body> {
    let now = chrono::Utc::now().timestamp();
    Request::post("/platform/inbound")
        .header("content-type", "application/json")
        .header("x-suite-source", "identity_hub")
        .header("x-suite-timestamp", now.to_string())
        .header("x-suite-delivery-id", "fixture-attempt")
        .header(
            "x-suite-signature",
            sign_webhook_hmac_sha256(secret.as_bytes(), now, "fixture-attempt", body.as_bytes()),
        )
        .body(Body::from(body.to_owned()))
        .expect("request")
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn signed_fixture_event_reads_entitlements_and_retains_receipts() -> Result<(), Box<dyn Error>>
{
    use baukit_suite::platform::EntitlementCache;
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let fixture = baukit_test::start_postgres_with_migrations(&migrations).await?;
    let pool = sqlx::PgPool::connect(fixture.connection_url()).await?;
    let hub = hub().await?;
    let config = settings(&hub.base);
    config.validate()?;
    let services = compose(pool.clone(), &config)?;
    let mut event: Value = serde_json::from_str(include_str!("fixtures/platform-event.json"))?;
    event["occurredAt"] = json!(chrono::Utc::now().to_rfc3339());
    let body = serde_json::to_string(&event)?;
    let response = services
        .router
        .clone()
        .oneshot(request(&body, "wrong"))
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    for _ in 0..2 {
        let response = services
            .router
            .clone()
            .oneshot(request(&body, SECRET))
            .await?;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }
    event["eventId"] = json!("platform-fixture-erasure");
    event["type"] = json!("identity_hub.account.erasure_requested");
    event["payload"] =
        json!({"suiteSubject": SUBJECT, "erasureId": "018f1234-5678-7abc-8def-012345678901"});
    let erasure = serde_json::to_string(&event)?;
    let response = services
        .router
        .clone()
        .oneshot(request(&erasure, SECRET))
        .await?;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let cache = PostgresEntitlementCache::new(pool.clone());
    let now = chrono::Utc::now().timestamp();
    let cached = cache
        .get(SUBJECT, PRODUCT, now)
        .await?
        .ok_or("snapshot missing")?;
    assert_eq!(cached.revision, 7);
    assert!(
        services
            .entitlements
            .has_entitlement(SUBJECT, "export", now)
            .await?
    );
    // A miss exercises the same verified ticket source through the read API.
    assert!(
        services
            .entitlements
            .has_tier("suite|another", "paid", now)
            .await?
    );
    let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM platform_inbound_receipts")
        .fetch_one(&pool)
        .await?;
    assert_eq!(receipts, 1);
    sqlx::query("INSERT INTO platform_inbound_receipts(event_id,body_hash,received_at) VALUES('old',decode(repeat('00',32),'hex'),now()-interval '31 days'),('recent',decode(repeat('00',32),'hex'),now()-interval '29 days')").execute(&pool).await?;
    assert_eq!(purge_receipts(pool.clone()).await?, 1);
    let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM platform_inbound_receipts")
        .fetch_one(&pool)
        .await?;
    assert_eq!(receipts, 2);
{% if context.auth_enabled %}    use baukit_erasure::ProductErasure;
    let mut tx = pool.begin().await?;
    {{ context.app_crate }}_postgres::PostgresProfileErasure
        .erase(&mut tx, SUBJECT)
        .await?;
    tx.rollback().await?;
    assert_eq!(cache.revision(SUBJECT, PRODUCT).await?, 7);
    let mut tx = pool.begin().await?;
    {{ context.app_crate }}_postgres::PostgresProfileErasure
        .erase(&mut tx, SUBJECT)
        .await?;
    tx.commit().await?;
    assert_eq!(cache.revision(SUBJECT, PRODUCT).await?, 0);
    assert!(cache.get(SUBJECT, PRODUCT, now).await?.is_none());
{% endif %}    pool.close().await;
    Ok(())
}

#[tokio::test]
async fn standalone_is_free_and_partial_hub_settings_fail() -> Result<(), Box<dyn Error>> {
    let config = ProductConfig::default();
    let entitlements = config.platform.entitlements(
        Arc::new(baukit_suite::platform::InMemoryEntitlementCache::default()),
        None,
    )?;
    assert!(entitlements.has_tier(SUBJECT, "free", 0).await?);
    assert!(!entitlements.has_entitlement(SUBJECT, "export", 0).await?);
    let mut partial = ProductConfig::default();
    partial.platform.hub_issuer = Some("https://hub.example".into());
    assert!(partial.validate().is_err());
    Ok(())
}

#[tokio::test]
async fn ticket_source_keeps_authentication_and_unrelated_missing_resources_retryable()
-> Result<(), Box<dyn Error>> {
    for (status, code, expected) in [
        (
            404,
            "platform_subject_not_found",
            EntitlementSourceError::NoEntitlements,
        ),
        (
            403,
            "account_inactive",
            EntitlementSourceError::NoEntitlements,
        ),
        (404, "not_found", EntitlementSourceError::Unavailable),
        (403, "scope_missing", EntitlementSourceError::Unavailable),
        (503, "unavailable", EntitlementSourceError::Unavailable),
    ] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let app = Router::new().route(
            "/tickets",
            post(move || async move {
                baukit_http::ApiError::new(
                    StatusCode::from_u16(status).expect("status"),
                    code,
                    "fixture",
                )
            }),
        );
        let task = tokio::spawn(async move { axum::serve(listener, app).await });
        let hub = Hub { base, task };
        let config = settings(&hub.base);
        let source = HubTicketSource::new(&config.platform, &config.platform_hub)?;
        assert_eq!(source.pull(SUBJECT, PRODUCT).await, Err(expected));
    }
    Ok(())
}

#[tokio::test]
async fn ticket_source_rejects_a_signed_ticket_for_another_subject() -> Result<(), Box<dyn Error>> {
    let hub = hub().await?;
    let config = settings(&hub.base);
    let source = HubTicketSource::new(&config.platform, &config.platform_hub)?;
    assert_eq!(
        source.pull("suite|misbound", PRODUCT).await,
        Err(EntitlementSourceError::Invalid)
    );
    Ok(())
}

#[test]
fn hub_runtime_settings_require_a_valid_url_and_service_credential() {
    let config = settings("https://hub.example");
    assert!(config.platform_hub.validate_for(&config.platform).is_ok());
    for url in [
        "ftp://hub.example/tickets",
        "https://user:password@hub.example/tickets",
        "https://hub.example/tickets#fragment",
    ] {
        let mut hub = config.platform_hub.clone();
        hub.ticket_url = Some(url.into());
        assert!(hub.validate_for(&config.platform).is_err(), "{url}");
    }
    let mut hub = config.platform_hub.clone();
    hub.service_token = Some(Secret::new("bad\nheader".into()));
    assert!(hub.validate_for(&config.platform).is_err());
    hub.service_token = None;
    assert!(hub.validate_for(&config.platform).is_err());
}
