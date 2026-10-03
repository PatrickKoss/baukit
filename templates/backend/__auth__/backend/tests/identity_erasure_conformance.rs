use std::{error::Error, path::PathBuf, sync::Arc, time::Duration};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, header},
};
use baukit_auth::{AuthState, OidcConfig, OidcVerifier};
use baukit_config::{Environment, HttpConfig, Secret};
use baukit_erasure::{
    ErasureFuture, ErasureService, ErasureState, IdentityDeletionHandler, PostgresErasureStore,
};
use baukit_jobs::{PostgresJobStore, WorkerConfig, WorkerRunner};
use baukit_runtime::ShutdownToken;
use baukit_test::{
    FakeIdentityAccountDeleter, IdentityErasureAdapter, IdentityErasureResponse,
    IdentityErasureSnapshot, MockOidcServer,
};
use sqlx::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;

use {{ context.app_crate }}_api::{ApiState, ErasureApi, router};
use {{ context.app_crate }}_bin::{AuthConfig, identity_erasure};
use {{ context.app_crate }}_postgres::PostgresProfileErasure;
use {{ context.app_crate }}_postgres::{PostgresItemRepository, PostgresUserRepository};
use {{ context.app_crate }}_services::{ItemService, UserService};

const AUDIENCE: &str = "{{ context.app_name }}-backend";
type TestError = Box<dyn Error + Send + Sync>;

fn adapter_result<'a, T: Send + 'a>(
    future: impl std::future::Future<Output = Result<T, TestError>> + Send + 'a,
) -> ErasureFuture<'a, Result<T, std::io::Error>> {
    Box::pin(async move { future.await.map_err(std::io::Error::other) })
}

struct EndpointAdapter {
    pool: PgPool,
    issuer: MockOidcServer,
    store: PostgresErasureStore,
    app: Option<Router>,
    handler: Option<IdentityDeletionHandler>,
    subject: String,
    prefix: &'static str,
}
impl EndpointAdapter {
    async fn request(
        &self,
        subject: &str,
        method: &str,
        path: &str,
        key: Option<&str>,
    ) -> Result<IdentityErasureResponse, TestError> {
        let claims = self
            .issuer
            .claims(subject, AUDIENCE, Duration::from_secs(60))?;
        let token = self.issuer.mint(&claims)?;
        let mut request = Request::builder()
            .method(method)
            .uri(format!("{}{}", self.prefix, path))
            .header(
                header::AUTHORIZATION,
                baukit_test::authorization_header(&token)?,
            );
        if let Some(key) = key {
            request = request.header("Idempotency-Key", key);
        }
        let response = self
            .app
            .as_ref()
            .ok_or("router not seeded")?
            .clone()
            .oneshot(request.body(Body::empty())?)
            .await?;
        let status = response.status().as_u16();
        if status == 202 {
            assert!(response.headers().contains_key(header::LOCATION));
        }
        let body = serde_json::from_slice(&to_bytes(response.into_body(), 16 * 1024).await?)?;
        Ok(IdentityErasureResponse { status, body })
    }
}
impl IdentityErasureAdapter for EndpointAdapter {
    type Error = std::io::Error;
    fn seed<'a>(
        &'a mut self,
        subject: &'a str,
        fake: Arc<FakeIdentityAccountDeleter>,
    ) -> ErasureFuture<'a, Result<(), Self::Error>> {
        adapter_result(async move {
            let users = UserService::new(Arc::new(PostgresUserRepository::new(
                self.pool.clone(),
                self.store.clone(),
            )));
            users.resolve_subject(subject).await?;
            let verifier =
                OidcVerifier::discover(OidcConfig::new(self.issuer.issuer(), AUDIENCE)?).await?;
            let service = ErasureService::new(
                self.store.clone(),
                fake.clone(),
                "keycloak".into(),
                Duration::from_secs(1),
                3,
            )?;
            self.handler = Some(IdentityDeletionHandler::new(
                self.store.clone(),
                "keycloak".into(),
                fake,
            ));
            let app = router(
                ApiState {
                    items: ItemService::new(Arc::new(PostgresItemRepository::new(
                        self.pool.clone(),
                    ))),
                    users,
                    auth: AuthState::new(verifier),
                    erasure: ErasureApi {
                        service,
                        product: Arc::new(PostgresProfileErasure),
                    },
                },
                &HttpConfig::default(),
            )?;
            self.app = Some(if self.prefix.is_empty() {
                app
            } else {
                Router::new().nest(self.prefix, app)
            });
            self.subject = subject.to_owned();
            Ok(())
        })
    }
    fn delete<'a>(
        &'a self,
        subject: &'a str,
        key: &'a str,
    ) -> ErasureFuture<'a, Result<IdentityErasureResponse, Self::Error>> {
        adapter_result(self.request(subject, "DELETE", "/me", Some(key)))
    }
    fn status<'a>(
        &'a self,
        subject: &'a str,
        operation: Uuid,
    ) -> ErasureFuture<'a, Result<IdentityErasureResponse, Self::Error>> {
        adapter_result(async move {
            self.request(subject, "GET", &format!("/me/erasures/{operation}"), None)
                .await
        })
    }
    fn resolve<'a>(
        &'a self,
        subject: &'a str,
    ) -> ErasureFuture<'a, Result<IdentityErasureResponse, Self::Error>> {
        adapter_result(self.request(subject, "GET", "/me", None))
    }
    fn snapshot<'a>(
        &'a self,
        subject: &'a str,
    ) -> ErasureFuture<'a, Result<IdentityErasureSnapshot, Self::Error>> {
        adapter_result(async move {
            let (product_rows, identity_jobs): (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM user_identities WHERE subject = $1), (SELECT count(*) FROM job_outbox WHERE payload->>'subject' = $1)")
                .bind(subject).fetch_one(&self.pool).await?;
            let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM erasure_operations WHERE response::text LIKE '%' || $1 || '%'").bind(subject).fetch_one(&self.pool).await?;
            Ok(IdentityErasureSnapshot {
                product_rows: u64::try_from(product_rows)?,
                identity_jobs: u64::try_from(identity_jobs)?,
                raw_subject_rows: u64::try_from(product_rows + identity_jobs + receipts)?,
                fenced: self.store.is_fenced(subject).await?,
            })
        })
    }
    fn run_worker(&self) -> ErasureFuture<'_, Result<(), Self::Error>> {
        adapter_result(async move {
            let operation: Uuid =
                sqlx::query_scalar("SELECT id FROM erasure_operations WHERE subject_hash = $1")
                    .bind(self.store.subject_hash(&self.subject))
                    .fetch_one(&self.pool)
                    .await?;
            let runner = WorkerRunner::new(
                Arc::new(PostgresJobStore::new(self.pool.clone())),
                Arc::new(self.handler.clone().ok_or("handler not seeded")?),
                WorkerConfig {
                    queue: "identity-erasure",
                    concurrency: 1,
                    poll_interval: Duration::from_millis(10),
                    ..WorkerConfig::default()
                },
            )?;
            let shutdown = ShutdownToken::new(Duration::from_secs(1));
            let task = tokio::spawn(runner.run(shutdown.clone()));
            let result = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if self
                        .store
                        .status(&self.subject, operation)
                        .await?
                        .is_some_and(|receipt| receipt.status == ErasureState::Completed)
                    {
                        break Ok::<_, baukit_erasure::ErasureError>(());
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await;
            shutdown.trigger();
            task.await??;
            result??;
            Ok(())
        })
    }
}
#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn endpoint_identity_erasure_conforms() -> Result<(), TestError> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let fixture = baukit_test::start_postgres_with_migrations(&migrations).await?;
    let pool = PgPool::connect(fixture.connection_url()).await?;
    let mut adapter = EndpointAdapter {
        store: PostgresErasureStore::new(
            pool.clone(),
            Secret::new("test-hash-key-with-at-least-32-bytes".into()),
        )?,
        pool,
        issuer: MockOidcServer::start().await?,
        app: None,
        handler: None,
        subject: String::new(),
        prefix: "",
    };
    baukit_test::check_identity_erasure_conformance(
        &mut adapter,
        "erasure-subject",
        "other-subject",
    )
    .await?;
    adapter.prefix = "/api";
    baukit_test::check_identity_erasure_conformance(
        &mut adapter,
        "nested-erasure-subject",
        "nested-other-subject",
    )
    .await?;
    for key in [None, Some("short"), Some("contains a space in the key")] {
        let response = adapter.request("new-subject", "DELETE", "/me", key).await?;
        assert_eq!(response.status, 400);
        assert_eq!(
            response.body["error"]["code"],
            "erasure_idempotency_key_invalid"
        );
    }
    let response = adapter
        .request("erasure-subject", "GET", "/items", None)
        .await?;
    assert_eq!(response.status, 401);
    assert_eq!(response.body["error"]["code"], "profile_erased");
    let response = adapter.resolve("erasure-subject").await?;
    assert_eq!(response.status, 401);
    assert_eq!(response.body["error"]["code"], "profile_erased");
    let token = adapter.issuer.mint(&adapter.issuer.claims(
        "erasure-subject",
        AUDIENCE,
        Duration::from_secs(60),
    )?)?;
    let response = adapter
        .app
        .as_ref()
        .ok_or("router not seeded")?
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/missing-route")
                .header(
                    header::AUTHORIZATION,
                    baukit_test::authorization_header(&token)?,
                )
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    let users = UserService::new(Arc::new(PostgresUserRepository::new(
        adapter.pool.clone(),
        adapter.store.clone(),
    )));
    assert!(matches!(
        users.resolve_subject("erasure-subject").await,
        Err({{ context.app_crate }}_services::ServiceError::Repository(
            {{ context.app_crate }}_ports::RepositoryError::ProfileErased
        ))
    ));
    adapter.pool.close().await;
    Ok(())
}

#[tokio::test]
async fn identity_configuration_requires_production_secrets_and_https() -> Result<(), TestError> {
    let pool = PgPool::connect_lazy("postgres://postgres:postgres@localhost/configuration-test")?;
    let mut config = AuthConfig::default();
    assert!(identity_erasure(pool.clone(), &config, Environment::Local).is_ok());
    config.identity_admin_base_url = "https://keycloak.example.test".into();
    assert!(identity_erasure(pool.clone(), &config, Environment::Production).is_err());
    config.identity_admin_client_secret = Some(Secret::new("configured-client-secret".into()));
    assert!(identity_erasure(pool.clone(), &config, Environment::Production).is_err());
    config.erasure_hash_key = Some(Secret::new(
        "configured-hash-key-with-at-least-32-bytes".into(),
    ));
    assert!(identity_erasure(pool.clone(), &config, Environment::Production).is_ok());
    config.identity_admin_base_url = "http://keycloak.example.test".into();
    assert!(identity_erasure(pool.clone(), &config, Environment::Production).is_err());
    config.identity_admin_base_url = "https://keycloak.example.test".into();
    config.erasure_hash_key = Some(Secret::new("too-short".into()));
    assert!(identity_erasure(pool, &config, Environment::Production).is_err());
    Ok(())
}
