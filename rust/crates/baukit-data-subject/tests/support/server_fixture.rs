use std::{
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use aws_lc_rs::hmac;
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::State,
    http::{HeaderMap, Request, StatusCode},
    middleware,
    response::IntoResponse,
    routing::get,
};
use baukit_auth::{
    ApiTokenVerifier, AuthState, NewApiToken, OidcConfig, OidcVerifier, Principal,
    PrincipalClaimMapping, establish_principal,
};
use baukit_config::Secret;
use baukit_data_subject::*;
use baukit_erasure::{
    ErasureService, IdentityRetention, PostgresErasureStore, reject_fenced_subject,
};
use baukit_jobs::{JobHandler, PostgresJobStore, WorkerConfig, WorkerRunner};
use baukit_runtime::ShutdownToken;
use baukit_test::{
    FakeIdentityAccountDeleter, InMemoryApiTokenStore, MockOidcServer, PostgresTestContainer,
};
use chrono::Utc;
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};
use tower::ServiceExt as _;
use uuid::Uuid;

pub const ERASURES: &str = "/api/v1/data-subject/erasures";
const SIGNING_KEY: &[u8] =
    include_bytes!("../../../../../fixtures/data-subject/v1/test-only-private.der");

pub fn request_body(values: &[&str]) -> Value {
    json!({"subjectReference":"opaque-reference-01", "identifiers":values.iter().map(|value| json!({"type":"email", "value":value})).collect::<Vec<_>>()})
}

fn lookup_hash(value: &str) -> Vec<u8> {
    hmac::sign(
        &hmac::Key::new(hmac::HMAC_SHA256, b"fixture-product-key-at-least-32-bytes"),
        value.as_bytes(),
    )
    .as_ref()
    .to_vec()
}

#[derive(Default)]
pub struct Product {
    pub resolves: AtomicUsize,
    pub erases: AtomicUsize,
    pub exports: AtomicUsize,
    pub fail_export: AtomicBool,
    pub oversized_export: AtomicBool,
    pub mutate_export: AtomicBool,
    pub fail_resolve: AtomicBool,
    pub fail_subject: AtomicBool,
}

impl DataSubjectProduct for Product {
    fn resolve<'a>(
        &'a self,
        connection: &'a mut PgConnection,
        identifiers: &'a [NormalizedIdentifier],
    ) -> DataSubjectFuture<'a, Result<Vec<LocalSubject>, sqlx::Error>> {
        Box::pin(async move {
            self.resolves.fetch_add(1, Ordering::SeqCst);
            if self.fail_resolve.load(Ordering::SeqCst) {
                return Err(sqlx::Error::Protocol("erase@example.invalid".into()));
            }
            let hashes = identifiers
                .iter()
                .map(|identifier| lookup_hash(identifier.value()))
                .collect::<Vec<_>>();
            let rows: Vec<(String, Option<String>)> = sqlx::query_as("SELECT id, authentication_subject FROM product_subjects WHERE lookup_hash = ANY($1)").bind(hashes).fetch_all(connection).await?;
            Ok(rows
                .into_iter()
                .map(|(subject_id, authentication_subject)| LocalSubject {
                    subject_id,
                    authentication_subject,
                })
                .collect())
        })
    }

    fn erase<'a>(
        &'a self,
        connection: &'a mut PgConnection,
        subject: &'a LocalSubject,
    ) -> DataSubjectFuture<'a, Result<ErasureReport, sqlx::Error>> {
        Box::pin(async move {
            self.erases.fetch_add(1, Ordering::SeqCst);
            let retained: i64 =
                sqlx::query_scalar("SELECT count(*) FROM legal_records WHERE subject_id = $1")
                    .bind(&subject.subject_id)
                    .fetch_one(&mut *connection)
                    .await?;
            let count = sqlx::query("DELETE FROM product_subjects WHERE id = $1")
                .bind(&subject.subject_id)
                .execute(connection)
                .await?
                .rows_affected();
            if self.fail_subject.load(Ordering::SeqCst)
                && subject.subject_id == "internal-restricted"
            {
                return Err(sqlx::Error::Protocol("restricted@example.invalid".into()));
            }
            let mut report = ErasureReport {
                categories: vec![CategoryEntry {
                    category: "profiles".into(),
                    action: CategoryAction::Erased,
                    count,
                }],
                restrictions: vec![],
            };
            if retained > 0 {
                report.categories.push(CategoryEntry {
                    category: "invoices".into(),
                    action: CategoryAction::Retained,
                    count: retained as u64,
                });
                report.restrictions.push(Restriction {
                    category: "invoices".into(),
                    reason: "Tax retention".into(),
                    legal_basis: "Tax code".into(),
                    retain_until: None,
                });
            }
            Ok(report)
        })
    }
}

impl DataSubjectExportProduct for Product {
    fn export<'a>(
        &'a self,
        connection: &'a mut PgConnection,
        subject: &'a LocalSubject,
    ) -> DataSubjectFuture<'a, Result<Vec<ExportCategory>, sqlx::Error>> {
        Box::pin(async move {
            self.exports.fetch_add(1, Ordering::SeqCst);
            let id: Option<String> =
                sqlx::query_scalar("SELECT id FROM product_subjects WHERE id = $1")
                    .bind(&subject.subject_id)
                    .fetch_optional(&mut *connection)
                    .await?;
            if self.mutate_export.load(Ordering::SeqCst) {
                sqlx::query("DELETE FROM product_subjects WHERE id = $1")
                    .bind(&subject.subject_id)
                    .execute(&mut *connection)
                    .await?;
            }
            if self.fail_export.load(Ordering::SeqCst)
                && subject.subject_id == "internal-restricted"
            {
                return Err(sqlx::Error::Protocol("restricted@example.invalid".into()));
            }
            let Some(id) = id else {
                return Ok(vec![]);
            };
            let value = if self.oversized_export.load(Ordering::SeqCst)
                && subject.subject_id == "internal-restricted"
            {
                "a".repeat(200)
            } else {
                "a".repeat(12)
            };
            Ok(vec![
                ExportCategory {
                    name: "profiles".into(),
                    records: vec![json!({"id": id})],
                },
                ExportCategory {
                    name: "activity".into(),
                    records: (1..=4)
                        .map(|number| json!({"number":number,"value":value}))
                        .collect(),
                },
                ExportCategory {
                    name: "empty".into(),
                    records: vec![],
                },
            ])
        })
    }
}

pub struct Fixture {
    pub _container: PostgresTestContainer,
    pub pool: PgPool,
    pub store: PostgresDataSubjectStore,
    pub erasure: ErasureService,
    pub product: Arc<Product>,
    pub router: Router,
    pub issuer: MockOidcServer,
    pub api_token: String,
    pub deleter: Arc<FakeIdentityAccountDeleter>,
}

impl Fixture {
    pub async fn new(delete_identity: bool) -> Result<Self, Box<dyn Error>> {
        Self::with_config(delete_identity, |_| {}).await
    }

    pub async fn with_config(
        delete_identity: bool,
        configure: impl FnOnce(&mut DataSubjectConfig),
    ) -> Result<Self, Box<dyn Error>> {
        Self::configured(delete_identity, false, configure).await
    }

    pub async fn with_exports(
        configure: impl FnOnce(&mut DataSubjectConfig),
    ) -> Result<Self, Box<dyn Error>> {
        Self::configured(true, true, configure).await
    }

    async fn configured(
        delete_identity: bool,
        exports: bool,
        configure: impl FnOnce(&mut DataSubjectConfig),
    ) -> Result<Self, Box<dyn Error>> {
        let container = baukit_test::start_postgres().await?;
        let pool = PgPool::connect(container.connection_url()).await?;
        for sql in [
            baukit_jobs::POSTGRES_MIGRATION_SQL,
            baukit_jobs::POSTGRES_MIGRATION_0002_SQL,
            baukit_jobs::POSTGRES_MIGRATION_0003_SQL,
            baukit_erasure::POSTGRES_MIGRATION_SQL,
            baukit_erasure::POSTGRES_REONBOARDING_MIGRATION_SQL,
            POSTGRES_MIGRATION_SQL,
            POSTGRES_EXPORTS_MIGRATION_SQL,
            "CREATE TABLE product_subjects (id TEXT PRIMARY KEY, authentication_subject TEXT, lookup_hash BYTEA NOT NULL); CREATE TABLE legal_records (subject_id TEXT NOT NULL); INSERT INTO legal_records VALUES ('internal-restricted');",
        ] {
            sqlx::raw_sql(sql).execute(&pool).await?;
        }
        for (id, subject, identifier) in [
            ("internal-erased", "auth-erased", "erase@example.invalid"),
            ("internal-other", "auth-other", "other@example.invalid"),
            (
                "internal-restricted",
                "auth-restricted",
                "restricted@example.invalid",
            ),
        ] {
            sqlx::query("INSERT INTO product_subjects (id, authentication_subject, lookup_hash) VALUES ($1, $2, $3)")
                .bind(id).bind(subject).bind(lookup_hash(identifier)).execute(&pool).await?;
        }
        let erasure_store = PostgresErasureStore::new(
            pool.clone(),
            Secret::new("fixture-fence-key-with-at-least-32-bytes".into()),
        )?;
        let deleter = Arc::new(FakeIdentityAccountDeleter::default());
        let retention = if delete_identity {
            IdentityRetention::Delete {
                deleter: deleter.clone(),
                provider_id: "fixture".into(),
                inline_timeout: Duration::from_secs(1),
                max_attempts: 2,
            }
        } else {
            IdentityRetention::Retain
        };
        let erasure = ErasureService::new(erasure_store, retention)?;
        let product = Arc::new(Product::default());
        let mut config = DataSubjectConfig::new(
            Service {
                name: "fixture".into(),
                version: "1.0.0".into(),
            },
            ["privacy".into(), "privacy-other".into()]
                .into_iter()
                .collect(),
            vec![
                IdentifierType::Email,
                IdentifierType::AccountId,
                IdentifierType::Phone,
            ],
            ReceiptSigner::from_pkcs8_der(SIGNING_KEY)?,
            Secret::new("fixture-operation-key-with-at-least-32-bytes".into()),
            erasure.clone(),
        );
        if exports {
            config = config.with_export(product.clone());
        }
        config.max_attempts = 2;
        configure(&mut config);
        let store = PostgresDataSubjectStore::new(pool.clone(), config)?;
        let issuer = MockOidcServer::start().await?;
        let verifier = OidcVerifier::from_jwks_uri(
            OidcConfig::new(issuer.issuer(), "fixture")?.with_principal_claims(
                PrincipalClaimMapping::default().client_id_claim("client_id"),
            ),
            issuer.jwks_url(),
        )?;
        let (_, tokens) = InMemoryApiTokenStore::with_service();
        let api_token = tokens
            .issue(
                Uuid::now_v7(),
                NewApiToken::new("fixture")
                    .with_grants(["data-subject.erase", "data-subject.export"]),
            )
            .await?
            .secret;
        let auth = AuthState::new(ApiTokenVerifier::new(tokens, Arc::new(verifier)));
        let session_routes = Router::new()
            .route("/session", get(session))
            .with_state(erasure.clone());
        let router = authenticated_routes(store.clone(), product.clone())
            .merge(session_routes)
            .layer(middleware::from_fn_with_state(
                auth.clone(),
                establish_principal,
            ))
            .merge(discovery_routes(store.clone()));
        let router = baukit_http::layers(router, baukit_http::HttpOptions::default());
        Ok(Self {
            _container: container,
            pool,
            store,
            erasure,
            product,
            router,
            issuer,
            api_token,
            deleter,
        })
    }

    pub fn token(&self, client: &str, scopes: &str) -> Result<String, Box<dyn Error>> {
        Ok(self.issuer.mint(
            &self
                .issuer
                .claims("machine", "fixture", Duration::from_secs(300))?
                .claim("client_id", client)
                .claim("scope", scopes),
        )?)
    }

    pub fn token_without_client(&self) -> Result<String, Box<dyn Error>> {
        Ok(self.issuer.mint(
            &self
                .issuer
                .claims("interactive", "fixture", Duration::from_secs(300))?
                .claim("scope", "data-subject.erase"),
        )?)
    }

    pub fn session_token(&self, subject: &str) -> Result<String, Box<dyn Error>> {
        Ok(self.issuer.mint(
            &self
                .issuer
                .claims(subject, "fixture", Duration::from_secs(300))?
                .claim("auth_time", Utc::now().timestamp() - 60),
        )?)
    }

    pub async fn session_status(&self, token: &str) -> Result<StatusCode, Box<dyn Error>> {
        Ok(self
            .send(
                Request::builder()
                    .uri("/session")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())?,
            )
            .await?
            .0)
    }

    pub fn post_request(
        &self,
        token: Option<&str>,
        keys: &[&str],
        body: Value,
    ) -> Result<Request<Body>, Box<dyn Error>> {
        let mut request = Request::builder()
            .method("POST")
            .uri(ERASURES)
            .header("content-type", "application/json")
            .header("x-request-id", "data-subject-test");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        for key in keys {
            request = request.header("idempotency-key", *key);
        }
        Ok(request.body(Body::from(serde_json::to_vec(&body)?))?)
    }

    pub fn get_request(
        &self,
        token: Option<&str>,
        id: Uuid,
    ) -> Result<Request<Body>, Box<dyn Error>> {
        let mut request = Request::builder()
            .uri(format!("{ERASURES}/{id}"))
            .header("x-request-id", "data-subject-test");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        Ok(request.body(Body::empty())?)
    }

    pub async fn send(
        &self,
        request: Request<Body>,
    ) -> Result<(StatusCode, HeaderMap, Value), Box<dyn Error>> {
        let response = self.router.clone().oneshot(request).await?;
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), 1_048_576).await?;
        Ok((status, headers, serde_json::from_slice(&bytes)?))
    }

    pub async fn assert_error(
        &self,
        request: Request<Body>,
        status: u16,
        code: &str,
    ) -> Result<(), Box<dyn Error>> {
        let (actual, _, body) = self.send(request).await?;
        assert_eq!(actual.as_u16(), status);
        assert_eq!(body["error"]["code"], code);
        assert!(body["error"]["message"].is_string());
        assert!(body["error"]["requestId"].is_string());
        assert_eq!(body["error"]["details"], json!({}));
        Ok(())
    }

    pub async fn post(
        &self,
        token: &str,
        key: &str,
        body: Value,
    ) -> Result<(StatusCode, OperationResource), Box<dyn Error>> {
        let (status, _, body) = self
            .send(self.post_request(Some(token), &[key], body)?)
            .await?;
        Ok((status, serde_json::from_value(body)?))
    }

    pub async fn run_until(&self, id: Uuid) -> Result<OperationResource, Box<dyn Error>> {
        self.run_handler_until(
            Arc::new(DataSubjectHandler::new(
                self.store.clone(),
                self.product.clone(),
            )),
            || async {
                let status: String =
                    sqlx::query_scalar("SELECT status FROM data_subject_operations WHERE id = $1")
                        .bind(id)
                        .fetch_one(&self.pool)
                        .await?;
                Ok(status != "pending")
            },
        )
        .await?;
        let kind: String =
            sqlx::query_scalar("SELECT kind FROM data_subject_operations WHERE id = $1")
                .bind(id)
                .fetch_one(&self.pool)
                .await?;
        Ok(self
            .store
            .status("privacy", serde_json::from_value(json!(kind))?, id)
            .await?
            .ok_or("missing operation")?)
    }

    pub async fn run_local_erasure(&self, id: Uuid) -> Result<(), Box<dyn Error>> {
        self.run_handler_until(Arc::new(DataSubjectHandler::new(self.store.clone(), self.product.clone())), || async {
            let remaining: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM job_outbox WHERE job_type = $1 AND idempotency_key = $2)")
                .bind(ERASURE_JOB_TYPE).bind(id.to_string()).fetch_one(&self.pool).await?;
            Ok(!remaining)
        }).await
    }

    pub async fn run_identities(&self) -> Result<(), Box<dyn Error>> {
        self.run_handler_until(
            Arc::new(baukit_erasure::IdentityDeletionHandler::new(
                self.erasure.store().clone(),
                "fixture".into(),
                self.deleter.clone(),
            )),
            || async {
                let remaining: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM job_outbox WHERE job_type = $1)",
                )
                .bind(baukit_erasure::IDENTITY_DELETE_JOB_TYPE)
                .fetch_one(&self.pool)
                .await?;
                Ok(!remaining)
            },
        )
        .await
    }

    pub async fn run_handler_until<F, Fut>(
        &self,
        handler: Arc<dyn JobHandler>,
        mut done: F,
    ) -> Result<(), Box<dyn Error>>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<bool, sqlx::Error>>,
    {
        let runner = WorkerRunner::new(
            Arc::new(PostgresJobStore::new(self.pool.clone())),
            handler,
            WorkerConfig {
                concurrency: 1,
                poll_interval: Duration::from_millis(10),
                retry_initial: Duration::from_millis(10),
                retry_max: Duration::from_millis(10),
                ..WorkerConfig::default()
            },
        )?;
        let shutdown = ShutdownToken::new(Duration::from_secs(1));
        let worker = tokio::spawn(runner.run(shutdown.clone()));
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if done().await? {
                    return Ok::<_, sqlx::Error>(());
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        shutdown.trigger();
        worker.await??;
        result??;
        Ok(())
    }

    pub async fn operation_count(&self) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar("SELECT count(*) FROM data_subject_operations")
            .fetch_one(&self.pool)
            .await
    }
    pub async fn job_count(&self) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar("SELECT count(*) FROM job_outbox")
            .fetch_one(&self.pool)
            .await
    }
    pub async fn subject_count(&self) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar("SELECT count(*) FROM product_subjects")
            .fetch_one(&self.pool)
            .await
    }

    pub async fn assert_no_stored_values(&self, forbidden: &[&str]) -> Result<(), Box<dyn Error>> {
        let tables: Vec<(String, String)> = sqlx::query_as("SELECT quote_ident(table_name), string_agg(quote_ident(column_name), ', ' ORDER BY ordinal_position) FROM information_schema.columns WHERE table_schema = 'public' GROUP BY table_name ORDER BY table_name")
            .fetch_all(&self.pool).await?;
        assert_eq!(tables.len(), 8);
        for (table, columns) in tables {
            let mut query =
                sqlx::QueryBuilder::new("SELECT row_to_json(record)::text FROM (SELECT ");
            query
                .push(columns)
                .push(" FROM ")
                .push(table)
                .push(") AS record");
            let rows: Vec<String> = query.build_query_scalar().fetch_all(&self.pool).await?;
            assert!(rows.iter().all(|row| forbidden.iter().all(|value| {
                !row.contains(value)
                    && !row.contains(
                        &value
                            .as_bytes()
                            .iter()
                            .map(|byte| format!("{byte:02x}"))
                            .collect::<String>(),
                    )
            })));
        }
        Ok(())
    }
}

async fn session(
    State(erasure): State<ErasureService>,
    axum::Extension(principal): axum::Extension<Principal>,
) -> impl IntoResponse {
    match reject_fenced_subject(
        erasure.store(),
        principal.subject(),
        principal.authenticated_at(),
    )
    .await
    {
        Ok(()) => axum::Json(json!({"ok":true})).into_response(),
        Err(error) => error.into_response(),
    }
}

#[derive(Clone, Default)]
pub struct Logs(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl Logs {
    pub fn subscriber(&self) -> impl tracing::Subscriber + Send + Sync + 'static {
        tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(self.clone())
            .finish()
    }

    pub fn text(&self) -> String {
        String::from_utf8(self.0.lock().expect("log mutex").clone()).expect("UTF-8 logs")
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Logs {
    type Writer = Self;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

impl std::io::Write for Logs {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        std::io::Write::write(&mut *self.0.lock().expect("log mutex"), bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl baukit_test::DataSubjectConformanceAdapter for Fixture {
    type Error = std::io::Error;

    async fn mint_token(
        &mut self,
        client: baukit_test::DataSubjectTestClient,
        scopes: &[&str],
    ) -> Result<String, Self::Error> {
        use baukit_test::DataSubjectTestClient;
        let client = match client {
            DataSubjectTestClient::Allowed => "privacy",
            DataSubjectTestClient::OtherAllowed => "privacy-other",
            DataSubjectTestClient::Disallowed => "unlisted-client",
            DataSubjectTestClient::ApiToken => return Ok(self.api_token.clone()),
        };
        self.token(client, &scopes.join(" "))
            .map_err(|_| std::io::Error::other("fixture token failed"))
    }

    async fn seed_subject(
        &mut self,
        _kind: OperationKind,
        case: baukit_test::DataSubjectTestSubject,
        _reference: &str,
    ) -> Result<Option<Vec<Identifier>>, Self::Error> {
        use baukit_test::DataSubjectTestSubject;
        let id = Uuid::new_v4().to_string();
        let value = format!("lookup-{}@example.invalid", Uuid::new_v4());
        if case != DataSubjectTestSubject::Unmatched {
            sqlx::query("INSERT INTO product_subjects (id, authentication_subject, lookup_hash) VALUES ($1, NULL, $2)")
                .bind(&id).bind(lookup_hash(&value)).execute(&self.pool).await
                .map_err(|_| std::io::Error::other("fixture subject failed"))?;
        }
        if case == DataSubjectTestSubject::Restricted {
            sqlx::query("INSERT INTO legal_records (subject_id) VALUES ($1)")
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(|_| std::io::Error::other("fixture restriction failed"))?;
        }
        Ok(Some(vec![Identifier {
            identifier_type: IdentifierType::Email,
            value,
        }]))
    }

    async fn run_pending_work(&mut self) -> Result<(), Self::Error> {
        self.run_handler_until(
            Arc::new(DataSubjectHandler::new(self.store.clone(), self.product.clone())),
            || async {
                let pending: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM data_subject_operations WHERE status = 'pending')")
                    .fetch_one(&self.pool).await?;
                Ok(!pending)
            },
        ).await.map_err(|_| std::io::Error::other("fixture worker failed"))
    }
}
