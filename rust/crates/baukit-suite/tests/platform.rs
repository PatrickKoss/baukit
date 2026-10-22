#![cfg(feature = "platform")]
use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use baukit_config::{Secret, Validate};
use baukit_core::webhook_signature::sign_webhook_hmac_sha256;
use baukit_suite::platform::*;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tower::ServiceExt;

const EVENTS: &str = include_str!("../../../../fixtures/platform-events/v1/envelopes.json");
const INVALID: &str = include_str!("../../../../fixtures/platform-events/v1/invalid.json");
const TICKETS: &str = include_str!("../../../../fixtures/entitlement-tickets/v1/vectors.json");
fn fixture_source() -> String {
    serde_json::from_str::<Vec<Value>>(EVENTS).expect("fixture")[0]["sourceApp"]
        .as_str()
        .expect("source")
        .into()
}
fn decoder() -> PlatformEventDecoder {
    PlatformEventDecoder::new(fixture_source(), None).expect("decoder")
}
fn decode(value: &Value) -> Result<PlatformEvent, PlatformError> {
    decoder().decode(&serde_json::to_vec(value).expect("json"), Clock.now())
}
fn event() -> PlatformEvent {
    decode(&serde_json::from_str::<Vec<Value>>(EVENTS).expect("fixture")[0]).expect("event")
}
#[derive(Default)]
struct Handler {
    calls: AtomicUsize,
    fail: AtomicUsize,
}
#[async_trait]
impl PlatformEventHandler for Handler {
    async fn handle(
        &self,
        _: &PlatformEvent,
        _: &mut EventContext<'_>,
    ) -> Result<(), PlatformError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) == 1 {
            Err(PlatformError::Unavailable)
        } else {
            Ok(())
        }
    }
}
struct Clock;
impl PlatformClock for Clock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).expect("time")
    }
}
#[test]
fn catalog_and_typed_events_conform_to_every_fixture() {
    let fixture_catalog = include_str!("../../../../fixtures/platform-events/v1/catalog.json");
    assert_eq!(
        PLATFORM_CATALOG_JSON,
        fixture_catalog.replace(&format!("{}.", fixture_source()), "platform.")
    );
    let events: Vec<Value> = serde_json::from_str(EVENTS).expect("fixtures");
    let catalog: Value = serde_json::from_str(PLATFORM_CATALOG_JSON).expect("catalog");
    assert_eq!(
        events.len(),
        catalog["types"].as_array().expect("types").len()
    );
    for value in events {
        let event = decode(&value).expect("typed event");
        assert!(!matches!(event.kind(), PlatformEventKind::Unknown));
        assert_eq!(serde_json::to_value(&event).expect("serialize"), value);
        let kind = serde_json::to_value(event.kind()).expect("kind");
        assert_eq!(kind["payload"], value["payload"]);
        assert_eq!(
            kind["type"],
            value["type"]
                .as_str()
                .expect("type")
                .split_once('.')
                .expect("prefix")
                .1
        );
    }
    for case in serde_json::from_str::<Vec<Value>>(INVALID).expect("negative fixtures") {
        let result = decode(&case["envelope"]);
        if case["name"] == "unknown event" {
            assert!(matches!(
                result.expect("unknown type").kind(),
                PlatformEventKind::Unknown
            ));
        } else {
            assert!(result.is_err(), "{}", case["name"]);
        }
    }
}
#[derive(Deserialize)]
struct TicketFixture {
    issuer: String,
    jwks: TicketJwks,
    vectors: Vec<TicketVector>,
}
#[derive(Deserialize)]
struct TicketVector {
    name: String,
    ticket: String,
    audience: String,
    now: i64,
    expected: String,
    claims: Option<TicketClaims>,
}
#[test]
fn shared_ticket_vectors() {
    let fixture: TicketFixture = serde_json::from_str(TICKETS).expect("fixture");
    for v in fixture.vectors {
        let result = verify_ticket(
            &v.ticket,
            &fixture.jwks,
            &fixture.issuer,
            &v.audience,
            v.now,
        );
        if v.expected == "ok" {
            assert_eq!(result.ok(), v.claims, "{}", v.name);
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
#[tokio::test]
async fn memory_deduplicates_concurrent_events_rejects_conflicts_and_retries_failure() {
    let inbox = InMemoryPlatformInbox::default();
    let handler = Handler::default();
    let event = event();
    handler.fail.store(1, Ordering::SeqCst);
    assert_eq!(
        inbox.receive(&event, [1; 32], &handler).await,
        Err(PlatformError::Unavailable)
    );
    handler.fail.store(0, Ordering::SeqCst);
    let (a, b) = tokio::join!(
        inbox.receive(&event, [1; 32], &handler),
        inbox.receive(&event, [1; 32], &handler)
    );
    assert_eq!(a, Ok(ReceiptOutcome::Applied));
    assert_eq!(b, Ok(ReceiptOutcome::Duplicate));
    assert_eq!(handler.calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        inbox.receive(&event, [2; 32], &handler).await,
        Err(PlatformError::EventIdConflict)
    );
}
fn request(body: &str, secret: &str, timestamp: i64) -> Request<Body> {
    Request::post("/platform/inbound")
        .header("content-type", "application/json")
        .header("x-suite-source", fixture_source())
        .header("x-suite-timestamp", timestamp.to_string())
        .header("x-suite-delivery-id", "delivery-1")
        .header(
            "x-suite-signature",
            sign_webhook_hmac_sha256(secret.as_bytes(), timestamp, "delivery-1", body.as_bytes()),
        )
        .body(Body::from(body.to_owned()))
        .expect("request")
}
#[tokio::test]
async fn receiver_statuses_rotation_window_body_limit_and_unknown_type() {
    let handler = Arc::new(Handler::default());
    let receiver = PlatformReceiver::new(
        decoder(),
        Secret::new("current".into()),
        Some(Secret::new("previous".into())),
        Arc::new(InMemoryPlatformInbox::default()),
        handler.clone(),
    )
    .expect("receiver")
    .with_clock(Arc::new(Clock));
    let app = router(receiver.clone());
    let body = serde_json::to_string(&event()).expect("json");
    for (secret, time, status) in [
        ("current", 1_800_000_000, 204),
        ("previous", 1_800_000_300, 204),
        ("current", 1_799_999_700, 204),
        ("current", 1_800_000_301, 401),
        ("current", 1_799_999_699, 401),
        ("wrong", 1_800_000_000, 401),
    ] {
        assert_eq!(
            app.clone()
                .oneshot(request(&body, secret, time))
                .await
                .expect("response")
                .status()
                .as_u16(),
            status
        );
    }
    let mut doubled = request(&body, "current", 1_800_000_000);
    doubled
        .headers_mut()
        .append("x-suite-timestamp", "1800000000".parse().expect("header"));
    assert_eq!(
        app.clone()
            .oneshot(doubled)
            .await
            .expect("response")
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let mut wrong_content = request(&body, "current", 1_800_000_000);
    wrong_content
        .headers_mut()
        .insert("content-type", "text/plain".parse().expect("header"));
    assert_eq!(
        app.clone()
            .oneshot(wrong_content)
            .await
            .expect("response")
            .status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert_eq!(
        app.clone()
            .oneshot(request("{}", "current", 1_800_000_000))
            .await
            .expect("response")
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let limited = router(receiver.with_body_limit(1).expect("limit"));
    assert_eq!(
        limited
            .oneshot(request(&body, "current", 1_800_000_000))
            .await
            .expect("response")
            .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let mut unknown: Value = serde_json::from_str(&body).expect("json");
    unknown["eventId"] = json!("unknown");
    unknown["type"] = json!("hub.unknown.changed");
    let unknown = unknown.to_string();
    assert_eq!(
        app.clone()
            .oneshot(request(&unknown, "current", 1_800_000_000))
            .await
            .expect("response")
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(handler.calls.load(Ordering::SeqCst), 1);
    let mut changed: Value = serde_json::from_str(&body).expect("json");
    changed["payload"]["revision"] = json!(8);
    assert_eq!(
        app.clone()
            .oneshot(request(&changed.to_string(), "current", 1_800_000_000))
            .await
            .expect("response")
            .status(),
        StatusCode::CONFLICT
    );
    changed["eventId"] = json!("retry");
    handler.fail.store(1, Ordering::SeqCst);
    assert_eq!(
        app.clone()
            .oneshot(request(&changed.to_string(), "current", 1_800_000_000))
            .await
            .expect("response")
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    handler.fail.store(0, Ordering::SeqCst);
    assert_eq!(
        app.oneshot(request(&changed.to_string(), "current", 1_800_000_000))
            .await
            .expect("response")
            .status(),
        StatusCode::NO_CONTENT
    );
}
fn snapshot(revision: u64, until: i64) -> EntitlementSnapshot {
    EntitlementSnapshot {
        subject: "suite|person-123".into(),
        product_id: "hebkit".into(),
        tier: "premium".into(),
        features: vec!["export".into()],
        revision,
        valid_until: until,
        grace_until: None,
    }
}
async fn cache_conformance(cache: &dyn EntitlementCache) {
    assert_eq!(
        cache
            .get("suite|person-123", "hebkit", 0)
            .await
            .expect("get"),
        None
    );
    cache.store(&snapshot(10, 100)).await.expect("store");
    cache.store(&snapshot(9, 200)).await.expect("stale");
    cache.store(&snapshot(10, 90)).await.expect("shorter");
    assert_eq!(
        cache
            .get("suite|person-123", "hebkit", 99)
            .await
            .expect("get"),
        Some(snapshot(10, 100))
    );
    assert_eq!(
        cache
            .get("suite|person-123", "hebkit", 100)
            .await
            .expect("expiry"),
        None
    );
    cache
        .store(&snapshot(9, 200))
        .await
        .expect("expired revision cursor");
    assert_eq!(
        cache
            .get("suite|person-123", "hebkit", 100)
            .await
            .expect("expiry"),
        None
    );
    cache.store(&snapshot(10, 300)).await.expect("renewal");
    assert_eq!(
        cache
            .get("suite|person-123", "hebkit", 200)
            .await
            .expect("renewed"),
        Some(snapshot(10, 300))
    );
    assert_eq!(
        cache.get("someone", "hebkit", 0).await.expect("subject"),
        None
    );
    assert_eq!(
        cache
            .get("suite|person-123", "other", 0)
            .await
            .expect("product"),
        None
    );
    let mut erased = snapshot(22, 10);
    erased.subject = "erased".into();
    cache.store(&erased).await.expect("expired erasure row");
    erased.product_id = "other".into();
    cache.store(&erased).await.expect("second product");
    cache.forget("erased").await.expect("forget");
    cache.forget("erased").await.expect("repeat forget");
    for product in ["hebkit", "other"] {
        assert_eq!(cache.get("erased", product, 0).await.expect("erased"), None);
        assert_eq!(
            cache
                .revision("erased", product)
                .await
                .expect("cursor erased"),
            0
        );
    }
    assert_eq!(
        cache
            .revision("suite|person-123", "hebkit")
            .await
            .expect("other subject retained"),
        10
    );
}
#[tokio::test]
async fn memory_cache_and_standalone_read_api() {
    let cache = Arc::new(InMemoryEntitlementCache::default());
    cache_conformance(cache.as_ref()).await;
    let service = Entitlements::hub("hebkit".into(), cache, Arc::new(Absent));
    assert!(
        service
            .has_entitlement("suite|person-123", "export", 200)
            .await
            .expect("feature")
    );
    assert!(
        !service
            .has_entitlement("suite|person-123", "export", 300)
            .await
            .expect("expired")
    );
    let standalone = Entitlements::standalone(
        "alpha".into(),
        StandaloneEntitlements {
            tier: "premium".into(),
            features: vec!["export".into()],
        },
    );
    assert!(
        standalone
            .has_tier("anyone", "premium", 1_800_000_000)
            .await
            .expect("tier")
    );
    assert!(
        standalone
            .has_entitlement("anyone", "export", 1_800_000_000)
            .await
            .expect("feature")
    );
    assert!(
        !standalone
            .has_entitlement("anyone", "unknown", 0)
            .await
            .expect("unknown")
    );
}
#[test]
fn config_rejects_partial_settings_and_redacts_secrets() {
    assert!(PlatformConfig::default().validate().is_ok());
    let config: PlatformConfig = serde_json::from_value(json!({"hub_issuer":"https://hub.example", "jwks_url":"https://hub.example/jwks", "product_id":"alpha", "source_app":fixture_source(), "receiver_secret":"private", "previous_receiver_secret":"old"})).expect("config");
    config.validate().expect("valid");
    let cache = Arc::new(InMemoryEntitlementCache::default());
    assert!(config.entitlements(cache.clone(), None).is_err());
    assert!(
        config
            .entitlements(cache.clone(), Some(Arc::new(Absent)))
            .is_ok()
    );
    assert!(PlatformConfig::default().entitlements(cache, None).is_ok());
    assert!(!format!("{config:?}").contains("private"));
    for patch in [
        json!({"hub_issuer":""}),
        json!({"jwks_url":""}),
        json!({"product_id":""}),
        json!({"source_app":""}),
        json!({"event_type_prefix":"nested.prefix"}),
        json!({"receiver_secret":null}),
        json!({"previous_receiver_secret":""}),
        json!({"jwks_url":"https://user:pass@hub.example/keys"}),
    ] {
        let mut value = json!({"hub_issuer":"https://hub.example","jwks_url":"https://hub.example/jwks","product_id":"alpha", "source_app":fixture_source(),"receiver_secret":"private"});
        value
            .as_object_mut()
            .expect("object")
            .extend(patch.as_object().expect("patch").clone());
        assert!(
            serde_json::from_value::<PlatformConfig>(value)
                .expect("settings")
                .validate()
                .is_err()
        );
    }
}

struct Keys {
    keys: std::sync::Mutex<TicketJwks>,
    calls: AtomicUsize,
    fail: AtomicUsize,
}
#[async_trait]
impl TicketJwksSource for Keys {
    async fn fetch(&self) -> Result<TicketJwks, TicketError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) == 1 {
            return Err(TicketError::KeysUnavailable);
        }
        Ok(self.keys.lock().expect("keys").clone())
    }
}
#[tokio::test(start_paused = true)]
async fn jwks_cache_bounds_unknown_key_refresh_rotation_ttl_and_outages() {
    let fixture: TicketFixture = serde_json::from_str(TICKETS).expect("fixture");
    let vector = &fixture.vectors[0];
    let source = Arc::new(Keys {
        keys: std::sync::Mutex::new(fixture.jwks.clone()),
        calls: AtomicUsize::new(0),
        fail: AtomicUsize::new(0),
    });
    let verifier = TicketVerifier::with_source(
        fixture.issuer.clone(),
        vector.audience.clone(),
        source.clone(),
    )
    .expect("verifier");
    verifier
        .verify_at(&vector.ticket, vector.now)
        .await
        .expect("first fetch");
    verifier
        .verify_at(&vector.ticket, vector.now)
        .await
        .expect("cached");
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    let unknown = fixture
        .vectors
        .iter()
        .find(|v| v.name == "unknown-kid")
        .expect("unknown");
    for _ in 0..100 {
        assert_eq!(
            verifier.verify_at(&unknown.ticket, unknown.now).await,
            Err(TicketError::Invalid)
        );
    }
    assert_eq!(source.calls.load(Ordering::SeqCst), 2);
    tokio::time::advance(std::time::Duration::from_secs(5)).await;
    // The unknown-kid vector has a valid signature under the fixture public key.
    source.keys.lock().expect("keys").keys[0].kid = "missing".into();
    verifier
        .verify_at(&unknown.ticket, unknown.now)
        .await
        .expect("rotated kid");
    assert_eq!(source.calls.load(Ordering::SeqCst), 3);
    tokio::time::advance(std::time::Duration::from_secs(300)).await;
    source.fail.store(1, Ordering::SeqCst);
    for _ in 0..100 {
        assert_eq!(
            verifier.verify_at(&unknown.ticket, unknown.now).await,
            Err(TicketError::KeysUnavailable)
        );
    }
    assert_eq!(source.calls.load(Ordering::SeqCst), 4);
    tokio::time::advance(std::time::Duration::from_secs(5)).await;
    source.fail.store(0, Ordering::SeqCst);
    let (a, b) = tokio::join!(
        verifier.verify_at(&unknown.ticket, unknown.now),
        verifier.verify_at(&unknown.ticket, unknown.now)
    );
    assert!(a.is_ok() && b.is_ok());
    assert_eq!(source.calls.load(Ordering::SeqCst), 5);
}
#[tokio::test]
async fn guarded_jwks_fetch_uses_network_adapter_and_rejects_redirects()
-> Result<(), Box<dyn std::error::Error>> {
    use axum::{Json, Router, routing::get};
    use baukit_egress::{AddressPolicy, EgressOptions, GuardedClient};
    let fixture: TicketFixture = serde_json::from_str(TICKETS)?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/keys", listener.local_addr()?);
    let app = Router::new()
        .route("/keys", get(move || async move { Json(fixture.jwks) }))
        .route(
            "/redirect",
            get(|| async { axum::response::Redirect::temporary("/keys") }),
        );
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let client =
        GuardedClient::new(EgressOptions::default().with_policy(AddressPolicy::AllowLoopback))?;
    let verifier = TicketVerifier::new(fixture.issuer, "hebkit".into(), &url, client.clone())?;
    let vector = &fixture.vectors[0];
    verifier.verify_at(&vector.ticket, vector.now).await?;
    let source = GuardedJwksSource::new(&url.replace("/keys", "/redirect"), client)?;
    assert!(matches!(
        source.fetch().await,
        Err(TicketError::KeysUnavailable)
    ));
    server.abort();
    Ok(())
}
struct Pull {
    snapshot: EntitlementSnapshot,
}
#[async_trait]
impl EntitlementSource for Pull {
    async fn pull(&self, _: &str, _: &str) -> Result<EntitlementSnapshot, EntitlementSourceError> {
        Ok(self.snapshot.clone())
    }
}
#[tokio::test]
async fn entitlement_events_pull_and_cache_but_do_not_accept_stale_or_misbound_state() {
    let cache = Arc::new(InMemoryEntitlementCache::default());
    let next = Arc::new(Handler::default());
    for (rev, subject, product, expected) in [
        (7, "suite|person-123", "hebkit", true),
        (6, "suite|person-123", "hebkit", false),
        (8, "other", "hebkit", false),
        (8, "suite|person-123", "other", false),
    ] {
        let mut snapshot = snapshot(rev, 100);
        snapshot.subject = subject.into();
        snapshot.product_id = product.into();
        let handler = EntitlementEventHandler::new(
            "hebkit".into(),
            cache.clone(),
            Arc::new(Pull { snapshot }),
            next.clone(),
        );
        let result = InMemoryPlatformInbox::default()
            .receive(&event(), [0; 32], &handler)
            .await;
        assert_eq!(result.is_ok(), expected);
    }
    assert_eq!(
        cache
            .get("suite|person-123", "hebkit", 99)
            .await
            .expect("cache")
            .expect("entry")
            .revision,
        7
    );
    assert_eq!(next.calls.load(Ordering::SeqCst), 0);
    let account =
        decode(&serde_json::from_str::<Vec<Value>>(EVENTS).expect("events")[1]).expect("account");
    let handler = EntitlementEventHandler::new(
        "hebkit".into(),
        cache,
        Arc::new(Pull {
            snapshot: snapshot(7, 100),
        }),
        next.clone(),
    );
    InMemoryPlatformInbox::default()
        .receive(&account, [0; 32], &handler)
        .await
        .expect("next");
    assert_eq!(next.calls.load(Ordering::SeqCst), 1);
}

#[cfg(feature = "postgres")]
mod postgres {
    use super::*;
    struct StoreHandler {
        cache: PostgresEntitlementCache,
        fail: bool,
        calls: AtomicUsize,
    }
    #[async_trait]
    impl PlatformEventHandler for StoreHandler {
        async fn handle(
            &self,
            _: &PlatformEvent,
            context: &mut EventContext<'_>,
        ) -> Result<(), PlatformError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            assert!(context.postgres().is_some());
            self.cache
                .store_in_context(&snapshot(20, 500), context)
                .await?;
            if self.fail {
                Err(PlatformError::Unavailable)
            } else {
                Ok(())
            }
        }
    }
    #[tokio::test]
    #[ignore = "requires Docker PostgreSQL"]
    async fn postgres_cache_receipts_concurrency_and_rollback()
    -> Result<(), Box<dyn std::error::Error>> {
        let container = baukit_test::start_postgres().await?;
        let pool = sqlx::PgPool::connect(container.connection_url()).await?;
        sqlx::raw_sql(PLATFORM_MIGRATION_SQL).execute(&pool).await?;
        let cache = PostgresEntitlementCache::new(pool.clone());
        cache_conformance(&cache).await;
        let inbox = PostgresPlatformInbox::new(pool.clone());
        let fail = StoreHandler {
            cache: cache.clone(),
            fail: true,
            calls: AtomicUsize::new(0),
        };
        let event = event();
        assert_eq!(
            inbox.receive(&event, [1; 32], &fail).await,
            Err(PlatformError::Unavailable)
        );
        assert_eq!(
            cache
                .get("suite|person-123", "hebkit", 0)
                .await?
                .expect("existing")
                .revision,
            10
        );
        let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM platform_inbound_receipts")
            .fetch_one(&pool)
            .await?;
        assert_eq!(receipts, 0);
        let handler = StoreHandler {
            cache: cache.clone(),
            fail: false,
            calls: AtomicUsize::new(0),
        };
        let (a, b) = tokio::join!(
            inbox.receive(&event, [1; 32], &handler),
            inbox.receive(&event, [1; 32], &handler)
        );
        let outcomes = [a?, b?];
        assert!(
            outcomes.contains(&ReceiptOutcome::Applied)
                && outcomes.contains(&ReceiptOutcome::Duplicate)
        );
        assert_eq!(handler.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            cache
                .get("suite|person-123", "hebkit", 0)
                .await?
                .expect("committed")
                .revision,
            20
        );
        assert_eq!(
            inbox.receive(&event, [2; 32], &handler).await,
            Err(PlatformError::EventIdConflict)
        );
        pool.close().await;
        Ok(())
    }
}

#[test]
fn platform_settings_load_through_nested_environment() {
    use baukit_config::{ConfigLoader, Environment, ValidationErrors};
    const CHILD: &str = "BAUKIT_PLATFORM_SETTINGS_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().expect("executable"))
            .args([
                "--exact",
                "platform_settings_load_through_nested_environment",
            ])
            .env(CHILD, "1")
            .env(
                "PLATFORM_SETTINGS_TEST__PLATFORM__HUB_ISSUER",
                "https://hub.example",
            )
            .env(
                "PLATFORM_SETTINGS_TEST__PLATFORM__JWKS_URL",
                "https://hub.example/keys",
            )
            .env("PLATFORM_SETTINGS_TEST__PLATFORM__PRODUCT_ID", "alpha")
            .env(
                "PLATFORM_SETTINGS_TEST__PLATFORM__SOURCE_APP",
                fixture_source(),
            )
            .env(
                "PLATFORM_SETTINGS_TEST__PLATFORM__EVENT_TYPE_PREFIX",
                "events",
            )
            .env(
                "PLATFORM_SETTINGS_TEST__PLATFORM__RECEIVER_SECRET",
                "secret-test-value",
            )
            .env(
                "PLATFORM_SETTINGS_TEST__PLATFORM__PREVIOUS_RECEIVER_SECRET",
                "previous-test-value",
            )
            .env(
                "PLATFORM_SETTINGS_TEST__PLATFORM__STANDALONE_DEFAULT__TIER",
                "premium",
            )
            .output()
            .expect("child");
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    #[derive(Default, Deserialize)]
    struct Product {
        platform: PlatformConfig,
    }
    impl Validate for Product {
        fn validate(&self) -> Result<(), ValidationErrors> {
            self.platform.validate()
        }
    }
    let settings = ConfigLoader::new("platform-settings-test", Environment::Local)
        .expect("loader")
        .without_dotenv()
        .without_local_file()
        .load::<Product>()
        .expect("settings")
        .product
        .platform;
    assert_eq!(settings.hub_issuer.as_deref(), Some("https://hub.example"));
    assert_eq!(settings.product_id, "alpha");
    assert_eq!(settings.source_app, fixture_source());
    assert_eq!(settings.event_type_prefix.as_deref(), Some("events"));
    assert_eq!(
        settings.receiver_secret.as_ref().expect("secret").expose(),
        "secret-test-value"
    );
    assert_eq!(
        settings
            .previous_receiver_secret
            .as_ref()
            .expect("secret")
            .expose(),
        "previous-test-value"
    );
    assert_eq!(settings.standalone_default.tier, "premium");
    assert!(
        settings
            .receiver(
                Arc::new(InMemoryPlatformInbox::default()),
                Arc::new(Handler::default())
            )
            .expect("receiver")
            .is_some()
    );
    assert!(
        PlatformConfig::default()
            .receiver(
                Arc::new(InMemoryPlatformInbox::default()),
                Arc::new(Handler::default())
            )
            .expect("standalone")
            .is_none()
    );
}

struct BlockingHandler {
    entered: Arc<tokio::sync::Notify>,
}
#[async_trait]
impl PlatformEventHandler for BlockingHandler {
    async fn handle(
        &self,
        _: &PlatformEvent,
        context: &mut EventContext<'_>,
    ) -> Result<(), PlatformError> {
        #[cfg(feature = "postgres")]
        if let Some(connection) = context.postgres() {
            PostgresEntitlementCache::store_in_transaction(&snapshot(99, 999), connection).await?;
        }
        #[cfg(not(feature = "postgres"))]
        let _ = context;
        self.entered.notify_one();
        std::future::pending().await
    }
}
async fn cancellation_conformance(inbox: Arc<dyn PlatformInbox>) {
    let entered = Arc::new(tokio::sync::Notify::new());
    let blocking = Arc::new(BlockingHandler {
        entered: entered.clone(),
    });
    let running = {
        let inbox = inbox.clone();
        tokio::spawn(async move { inbox.receive(&event(), [7; 32], blocking.as_ref()).await })
    };
    entered.notified().await;
    running.abort();
    assert!(running.await.expect_err("aborted").is_cancelled());
    let handler = Handler::default();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        inbox.receive(&event(), [7; 32], &handler),
    )
    .await
    .expect("released claim");
    assert_eq!(result, Ok(ReceiptOutcome::Applied));
    assert_eq!(handler.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn memory_cancelled_handler_releases_event_id() {
    cancellation_conformance(Arc::new(InMemoryPlatformInbox::default())).await;
}
#[cfg(feature = "postgres")]
#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn postgres_cancelled_handler_rolls_back_cache_and_releases_event_id()
-> Result<(), Box<dyn std::error::Error>> {
    let container = baukit_test::start_postgres().await?;
    let pool = sqlx::PgPool::connect(container.connection_url()).await?;
    sqlx::raw_sql(PLATFORM_MIGRATION_SQL).execute(&pool).await?;
    cancellation_conformance(Arc::new(PostgresPlatformInbox::new(pool.clone()))).await;
    let cache = PostgresEntitlementCache::new(pool.clone());
    assert_eq!(cache.get("suite|person-123", "hebkit", 0).await?, None);
    assert_eq!(
        PostgresPlatformInbox::new(pool.clone())
            .receive(&event(), [7; 32], &Handler::default())
            .await?,
        ReceiptOutcome::Duplicate
    );
    pool.close().await;
    Ok(())
}

struct Absent;
#[async_trait]
impl EntitlementSource for Absent {
    async fn pull(&self, _: &str, _: &str) -> Result<EntitlementSnapshot, EntitlementSourceError> {
        Err(EntitlementSourceError::NoEntitlements)
    }
}

struct CountingPull {
    calls: AtomicUsize,
    value: std::sync::Mutex<Result<EntitlementSnapshot, EntitlementSourceError>>,
}
#[async_trait]
impl EntitlementSource for CountingPull {
    async fn pull(&self, _: &str, _: &str) -> Result<EntitlementSnapshot, EntitlementSourceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        self.value.lock().expect("value").clone()
    }
}
#[tokio::test]
async fn hub_reads_pull_on_first_use_and_expiry_with_single_flight() {
    let cache = Arc::new(InMemoryEntitlementCache::default());
    let source = Arc::new(CountingPull {
        calls: AtomicUsize::new(0),
        value: std::sync::Mutex::new(Ok(snapshot(7, 100))),
    });
    let reads = Entitlements::hub("hebkit".into(), cache.clone(), source.clone());
    let mut tasks = Vec::new();
    for _ in 0..20 {
        let reads = reads.clone();
        tasks.push(tokio::spawn(async move {
            reads.current("suite|person-123", 0).await
        }));
    }
    for task in tasks {
        assert_eq!(task.await.expect("join"), Ok(Some(snapshot(7, 100))));
    }
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        cache
            .get("suite|person-123", "hebkit", 0)
            .await
            .expect("stored"),
        Some(snapshot(7, 100))
    );
    *source.value.lock().expect("value") = Ok(snapshot(8, 200));
    assert_eq!(
        reads.current("suite|person-123", 100).await,
        Ok(Some(snapshot(8, 200)))
    );
    assert_eq!(source.calls.load(Ordering::SeqCst), 2);
    assert!(
        reads
            .has_tier("suite|person-123", "premium", 199)
            .await
            .expect("cached")
    );
    assert_eq!(source.calls.load(Ordering::SeqCst), 2);
    *source.value.lock().expect("value") = Err(EntitlementSourceError::Unavailable);
    assert_eq!(
        reads.has_tier("suite|person-123", "premium", 200).await,
        Err(PlatformError::Unavailable)
    );
    assert_eq!(
        cache
            .get("suite|person-123", "hebkit", 200)
            .await
            .expect("expired grant"),
        None
    );
    *source.value.lock().expect("value") = Ok(snapshot(7, 300));
    assert_eq!(
        reads.current("suite|person-123", 200).await,
        Err(PlatformError::Unavailable)
    );
}
#[tokio::test]
async fn hub_free_fallback_maps_only_absent_and_inactive_and_keeps_revision_cursors() {
    for (status, code) in [
        (404, "platform_subject_not_found"),
        (403, "account_inactive"),
    ] {
        let cache = Arc::new(InMemoryEntitlementCache::default());
        let source = Arc::new(CountingPull {
            calls: AtomicUsize::new(0),
            value: std::sync::Mutex::new(Err(EntitlementSourceError::from_hub_response(
                status, code,
            ))),
        });
        let reads = Entitlements::hub("hebkit".into(), cache.clone(), source.clone());
        assert!(
            reads
                .has_tier("suite|person-123", "free", 0)
                .await
                .expect("first use free")
        );
        assert!(
            !reads
                .has_entitlement("suite|person-123", "export", 0)
                .await
                .expect("no grants")
        );
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
        cache
            .store(&snapshot(12, 100))
            .await
            .expect("old paid grant");
        let free = reads
            .current("suite|person-123", 100)
            .await
            .expect("free after expiry")
            .expect("snapshot");
        assert_eq!(free.tier, "free");
        assert_eq!(free.revision, 12);
        assert_eq!(free.valid_until, 100 + FREE_CACHE_TTL_SECONDS);
        cache
            .store(&snapshot(11, 500))
            .await
            .expect("stale grant ignored");
        assert!(
            reads
                .has_tier("suite|person-123", "free", 101)
                .await
                .expect("still free")
        );
    }
    for (status, code) in [
        (404, "not_found"),
        (403, "product_unavailable"),
        (403, "scope_missing"),
        (403, "product_mismatch"),
        (401, "account_inactive"),
        (500, "platform_subject_not_found"),
    ] {
        assert_eq!(
            EntitlementSourceError::from_hub_response(status, code),
            EntitlementSourceError::Unavailable
        );
    }
}

#[test]
fn decoder_uses_configured_source_and_distinct_prefix_and_limits_future_events() {
    let mut value = serde_json::from_str::<Vec<Value>>(EVENTS).expect("events")[0].clone();
    value["sourceApp"] = json!("identity");
    value["type"] = json!("notifications.entitlements.changed");
    let decoder = PlatformEventDecoder::new("identity".into(), Some("notifications".into()))
        .expect("decoder");
    let body = serde_json::to_vec(&value).expect("json");
    assert!(matches!(
        decoder.decode(&body, Clock.now()).expect("event").kind(),
        PlatformEventKind::EntitlementsChanged(_)
    ));
    assert!(self::decoder().decode(&body, Clock.now()).is_err());
    for (seconds, accepted) in [(300, true), (301, false), (-40 * 24 * 3600, true)] {
        value["occurredAt"] =
            json!((Clock.now() + chrono::TimeDelta::seconds(seconds)).to_rfc3339());
        assert_eq!(
            decoder
                .decode(&serde_json::to_vec(&value).expect("json"), Clock.now())
                .is_ok(),
            accepted
        );
    }
}
#[test]
fn ticket_metadata_none_algorithm_and_large_revisions_are_rejected() {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    let fixture: TicketFixture = serde_json::from_str(TICKETS).expect("fixture");
    let v = &fixture.vectors[0];
    for patch in [
        json!({"use":"enc"}),
        json!({"alg":"HS256"}),
        json!({"key_ops":["sign"]}),
        json!({"kty":"RSA"}),
        json!({"crv":"Ed448"}),
    ] {
        let mut value = serde_json::to_value(&fixture.jwks).expect("keys");
        value["keys"][0]
            .as_object_mut()
            .expect("key")
            .extend(patch.as_object().expect("patch").clone());
        let keys = serde_json::from_value(value).expect("keys");
        assert_eq!(
            verify_ticket(&v.ticket, &keys, &fixture.issuer, &v.audience, v.now),
            Err(TicketError::Invalid)
        );
    }
    let mut keys = fixture.jwks.clone();
    keys.keys.push(keys.keys[0].clone());
    assert_eq!(
        verify_ticket(&v.ticket, &keys, &fixture.issuer, &v.audience, v.now),
        Err(TicketError::Invalid)
    );
    let (_, rest) = v.ticket.split_once('.').expect("ticket");
    let header = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&json!({"alg":"none", "kid":fixture.jwks.keys[0].kid})).expect("header"),
    );
    assert_eq!(
        verify_ticket(
            &format!("{header}.{rest}"),
            &fixture.jwks,
            &fixture.issuer,
            &v.audience,
            v.now
        ),
        Err(TicketError::Invalid)
    );
    let mut claims = v.claims.clone().expect("claims");
    claims.rev = MAX_TICKET_REVISION;
    claims
        .verify_claims(&fixture.issuer, &v.audience, v.now)
        .expect("safe integer");
    claims.rev += 1;
    assert_eq!(
        claims.verify_claims(&fixture.issuer, &v.audience, v.now),
        Err(TicketError::Invalid)
    );
}

struct SlowKeys {
    keys: TicketJwks,
    calls: AtomicUsize,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
#[async_trait]
impl TicketJwksSource for SlowKeys {
    async fn fetch(&self) -> Result<TicketJwks, TicketError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) > 0 {
            self.entered.notify_one();
            self.release.notified().await;
        }
        Ok(self.keys.clone())
    }
}
#[tokio::test]
async fn cached_ticket_verification_does_not_wait_for_unknown_key_refresh() {
    let fixture: TicketFixture = serde_json::from_str(TICKETS).expect("fixture");
    let first = &fixture.vectors[0];
    let source = Arc::new(SlowKeys {
        keys: fixture.jwks.clone(),
        calls: AtomicUsize::new(0),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let verifier =
        TicketVerifier::with_source(fixture.issuer, first.audience.clone(), source.clone())
            .expect("verifier");
    verifier
        .verify_at(&first.ticket, first.now)
        .await
        .expect("prime");
    let unknown = fixture
        .vectors
        .iter()
        .find(|v| v.name == "unknown-kid")
        .expect("unknown");
    let verifier2 = verifier.clone();
    let ticket = unknown.ticket.clone();
    let now = unknown.now;
    let refreshing = tokio::spawn(async move { verifier2.verify_at(&ticket, now).await });
    source.entered.notified().await;
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        verifier.verify_at(&first.ticket, first.now),
    )
    .await
    .expect("cached key remains available")
    .expect("valid");
    source.release.notify_one();
    assert_eq!(refreshing.await.expect("join"), Err(TicketError::Invalid));
    assert_eq!(source.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn platform_openapi_describes_signature_headers_and_delivery_responses() {
    use utoipa::OpenApi;
    let doc = serde_json::to_value(PlatformOpenApi::openapi()).expect("document");
    let route = &doc["paths"]["/platform/inbound"]["post"];
    for status in ["204", "401", "409", "413", "415", "422", "503"] {
        assert!(route["responses"].get(status).is_some(), "{status}");
    }
    assert_eq!(route["parameters"].as_array().expect("headers").len(), 4);
    assert!(
        route["requestBody"]["content"]
            .get("application/json")
            .is_some()
    );
}

#[test]
fn ticket_verifier_keeps_config_validation_details() {
    use baukit_egress::{EgressOptions, GuardedClient};
    let client = GuardedClient::new(EgressOptions::default()).expect("client");
    let config = PlatformConfig {
        hub_issuer: Some("bad issuer".into()),
        ..Default::default()
    };
    let errors = config
        .ticket_verifier(client)
        .err()
        .expect("validation errors");
    assert_eq!(errors, config.validate().expect_err("same details"));
}

#[cfg(feature = "postgres")]
struct BlockingSource {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
#[cfg(feature = "postgres")]
#[async_trait]
impl EntitlementSource for BlockingSource {
    async fn pull(&self, _: &str, _: &str) -> Result<EntitlementSnapshot, EntitlementSourceError> {
        self.entered.notify_one();
        self.release.notified().await;
        Ok(snapshot(7, i64::MAX))
    }
}
#[cfg(feature = "postgres")]
#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn postgres_prepares_without_a_connection_erases_transactionally_and_purges_batches()
-> Result<(), Box<dyn std::error::Error>> {
    let container = baukit_test::start_postgres().await?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(container.connection_url())
        .await?;
    sqlx::raw_sql(PLATFORM_MIGRATION_SQL).execute(&pool).await?;
    let cache = Arc::new(PostgresEntitlementCache::new(pool.clone()));
    let inbox = Arc::new(PostgresPlatformInbox::new(pool.clone()));
    let source = Arc::new(BlockingSource {
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let handler = EntitlementEventHandler::new(
        "hebkit".into(),
        cache.clone(),
        source.clone(),
        Arc::new(Handler::default()),
    );
    let receiving = {
        let inbox = inbox.clone();
        tokio::spawn(async move { inbox.receive(&event(), [4; 32], &handler).await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), source.entered.notified()).await?;
    let mut connection =
        tokio::time::timeout(std::time::Duration::from_secs(1), pool.acquire()).await??;
    let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM platform_inbound_receipts")
        .fetch_one(&mut *connection)
        .await?;
    assert_eq!(receipts, 0, "the source pull precedes the receipt claim");
    PostgresEntitlementCache::store_in_transaction(&snapshot(9, i64::MAX), &mut connection).await?;
    drop(connection);
    source.release.notify_one();
    assert_eq!(receiving.await??, ReceiptOutcome::Applied);
    assert_eq!(
        cache
            .get("suite|person-123", "hebkit", 0)
            .await?
            .expect("cache")
            .revision,
        9,
        "a pull must not overwrite a concurrent newer event"
    );

    let mut tx = pool.begin().await?;
    let mut context = EventContext::with_postgres(&mut tx);
    PostgresEntitlementCache::forget_in_transaction(
        "suite|person-123",
        context.postgres().expect("transaction"),
    )
    .await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM platform_entitlement_cache")
        .fetch_one(&mut *tx)
        .await?;
    assert_eq!(count, 0);
    tx.rollback().await?;
    assert_eq!(
        cache.revision("suite|person-123", "hebkit").await?,
        9,
        "erasure rolls back with the product transaction"
    );
    let mut tx = pool.begin().await?;
    PostgresEntitlementCache::forget_in_transaction("suite|person-123", &mut tx).await?;
    tx.commit().await?;
    assert_eq!(cache.revision("suite|person-123", "hebkit").await?, 0);

    let cutoff = Utc::now() - chrono::TimeDelta::days(30);
    for (id, at) in [
        ("old-1", cutoff - chrono::TimeDelta::days(1)),
        ("old-2", cutoff - chrono::TimeDelta::days(1)),
        ("old-3", cutoff - chrono::TimeDelta::seconds(1)),
        ("boundary", cutoff),
        ("recent", Utc::now()),
    ] {
        sqlx::query("INSERT INTO platform_inbound_receipts(event_id,body_hash,received_at) VALUES($1,$2,$3)").bind(id).bind([0_u8;32].as_slice()).bind(at).execute(&pool).await?;
    }
    assert_eq!(
        inbox.purge_received_before(cutoff, 0).await,
        Err(PlatformError::Invalid)
    );
    assert_eq!(inbox.purge_received_before(cutoff, 2).await?, 2);
    assert_eq!(inbox.purge_received_before(cutoff, 2).await?, 1);
    assert_eq!(inbox.purge_received_before(cutoff, 2).await?, 0);
    let ids: Vec<String> =
        sqlx::query_scalar("SELECT event_id FROM platform_inbound_receipts ORDER BY event_id")
            .fetch_all(&pool)
            .await?;
    assert_eq!(
        ids,
        vec![
            "boundary".to_owned(),
            event().envelope().event_id.clone(),
            "recent".to_owned()
        ]
    );
    pool.close().await;
    Ok(())
}

#[tokio::test]
async fn source_failure_is_retryable_and_duplicate_receipts_need_no_pull() {
    let cache = Arc::new(InMemoryEntitlementCache::default());
    let source = Arc::new(CountingPull {
        calls: AtomicUsize::new(0),
        value: std::sync::Mutex::new(Err(EntitlementSourceError::Invalid)),
    });
    let handler = EntitlementEventHandler::new(
        "hebkit".into(),
        cache.clone(),
        source.clone(),
        Arc::new(Handler::default()),
    );
    let inbox = InMemoryPlatformInbox::default();
    assert_eq!(
        inbox.receive(&event(), [1; 32], &handler).await,
        Err(PlatformError::Unavailable)
    );
    assert_eq!(
        cache
            .revision("suite|person-123", "hebkit")
            .await
            .expect("no mutation"),
        0
    );
    *source.value.lock().expect("value") = Ok(snapshot(7, 100));
    assert_eq!(
        inbox.receive(&event(), [1; 32], &handler).await,
        Ok(ReceiptOutcome::Applied)
    );
    *source.value.lock().expect("value") = Err(EntitlementSourceError::Unavailable);
    assert_eq!(
        inbox.receive(&event(), [1; 32], &handler).await,
        Ok(ReceiptOutcome::Duplicate)
    );
    assert_eq!(source.calls.load(Ordering::SeqCst), 2);
}

struct SkipsPreparation;
#[async_trait]
impl PlatformInbox for SkipsPreparation {
    async fn receive(
        &self,
        event: &PlatformEvent,
        _: [u8; 32],
        handler: &dyn PlatformEventHandler,
    ) -> Result<ReceiptOutcome, PlatformError> {
        handler.handle(event, &mut EventContext::detached()).await?;
        Ok(ReceiptOutcome::Applied)
    }
}

#[tokio::test]
async fn skipped_entitlement_preparation_returns_retryable_503() {
    let handler = EntitlementEventHandler::new(
        "hebkit".into(),
        Arc::new(InMemoryEntitlementCache::default()),
        Arc::new(Pull {
            snapshot: snapshot(7, i64::MAX),
        }),
        Arc::new(Handler::default()),
    );
    let receiver = PlatformReceiver::new(
        decoder(),
        Secret::new("current".into()),
        None,
        Arc::new(SkipsPreparation),
        Arc::new(handler),
    )
    .expect("receiver")
    .with_clock(Arc::new(Clock));
    let body = serde_json::to_string(&event()).expect("event");
    let response = router(receiver)
        .oneshot(request(&body, "current", Clock.now().timestamp()))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

struct NoGrant;
#[async_trait]
impl EntitlementSource for NoGrant {
    async fn pull(&self, _: &str, _: &str) -> Result<EntitlementSnapshot, EntitlementSourceError> {
        Err(EntitlementSourceError::NoEntitlements)
    }
}

#[tokio::test]
async fn receiver_clock_controls_prepared_free_snapshot_deadline() {
    let cache = Arc::new(InMemoryEntitlementCache::default());
    let handler = EntitlementEventHandler::new(
        "hebkit".into(),
        cache.clone(),
        Arc::new(NoGrant),
        Arc::new(Handler::default()),
    );
    let receiver = PlatformReceiver::new(
        decoder(),
        Secret::new("current".into()),
        None,
        Arc::new(InMemoryPlatformInbox::default()),
        Arc::new(handler),
    )
    .expect("receiver")
    .with_clock(Arc::new(Clock));
    let body = serde_json::to_string(&event()).expect("event");
    let response = router(receiver)
        .oneshot(request(&body, "current", Clock.now().timestamp()))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let snapshot = cache
        .get("suite|person-123", "hebkit", Clock.now().timestamp())
        .await
        .expect("cache")
        .expect("free snapshot");
    assert_eq!(snapshot.tier, "free");
    assert_eq!(
        snapshot.valid_until,
        Clock.now().timestamp() + FREE_CACHE_TTL_SECONDS
    );
}
