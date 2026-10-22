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
fn event() -> PlatformEvent {
    PlatformEvent::decode(
        &serde_json::to_vec(&serde_json::from_str::<Vec<Value>>(EVENTS).expect("fixture")[0])
            .expect("json"),
    )
    .expect("event")
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
    assert_eq!(
        PLATFORM_CATALOG_JSON,
        include_str!("../../../../fixtures/platform-events/v1/catalog.json")
    );
    let events: Vec<Value> = serde_json::from_str(EVENTS).expect("fixtures");
    let catalog: Value = serde_json::from_str(PLATFORM_CATALOG_JSON).expect("catalog");
    assert_eq!(
        events.len(),
        catalog["types"].as_array().expect("types").len()
    );
    for value in events {
        let event: PlatformEvent = serde_json::from_value(value.clone()).expect("typed event");
        assert!(!matches!(event.kind(), PlatformEventKind::Unknown));
        assert_eq!(serde_json::to_value(&event).expect("serialize"), value);
        let kind = serde_json::to_value(event.kind()).expect("kind");
        assert_eq!(kind["payload"], value["payload"]);
        assert_eq!(kind["type"], value["type"]);
    }
    for case in serde_json::from_str::<Vec<Value>>(INVALID).expect("negative fixtures") {
        let result = serde_json::from_value::<PlatformEvent>(case["envelope"].clone());
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
        .header("x-suite-source", PLATFORM_SOURCE_APP)
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
}
#[tokio::test]
async fn memory_cache_and_standalone_read_api() {
    let cache = Arc::new(InMemoryEntitlementCache::default());
    cache_conformance(cache.as_ref()).await;
    let service = Entitlements::hub("hebkit".into(), cache);
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
    let config: PlatformConfig = serde_json::from_value(json!({"hub_issuer":"https://hub.example", "jwks_url":"https://hub.example/jwks", "product_id":"alpha", "receiver_secret":"private", "previous_receiver_secret":"old"})).expect("config");
    config.validate().expect("valid");
    assert!(!format!("{config:?}").contains("private"));
    for patch in [
        json!({"hub_issuer":""}),
        json!({"jwks_url":""}),
        json!({"product_id":""}),
        json!({"receiver_secret":null}),
        json!({"previous_receiver_secret":""}),
        json!({"jwks_url":"https://user:pass@hub.example/keys"}),
    ] {
        let mut value = json!({"hub_issuer":"https://hub.example","jwks_url":"https://hub.example/jwks","product_id":"alpha","receiver_secret":"private"});
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
    async fn pull(&self, _: &str, _: &str) -> Result<EntitlementSnapshot, PlatformError> {
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
    let account: PlatformEvent = serde_json::from_value(
        serde_json::from_str::<Vec<Value>>(EVENTS).expect("events")[1].clone(),
    )
    .expect("account");
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
