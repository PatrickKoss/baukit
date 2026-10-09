#![cfg(feature = "api-providers")]
use baukit_config::Secret;
use baukit_erasure::{
    ApiDeletionConfig, ClerkAccountDeleter, IdentityAccountDeleter, IdentityDeletionError,
    IdentityRetention, WorkOsAccountDeleter,
};
use std::{error::Error, sync::Arc, time::Duration};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

struct DeletionCase {
    deleter: Arc<dyn IdentityAccountDeleter>,
    endpoint: &'static str,
}

fn deleters(base: &str) -> Result<[DeletionCase; 2], IdentityDeletionError> {
    let config = ApiDeletionConfig {
        base_url: base.into(),
        api_key: Secret::new("private-key".into()),
        allow_local_http: true,
    };
    Ok([
        DeletionCase {
            deleter: Arc::new(ClerkAccountDeleter::new(config.clone())?),
            endpoint: "/users/user_123",
        },
        DeletionCase {
            deleter: Arc::new(WorkOsAccountDeleter::new(config)?),
            endpoint: "/user_management/users/user_123",
        },
    ])
}

#[tokio::test]
async fn deletion_is_authenticated_idempotent_and_classifies_failures() -> Result<(), Box<dyn Error>>
{
    let server = MockServer::start().await;
    for DeletionCase { deleter, endpoint } in deleters(&server.uri())? {
        for (status, expected) in [
            (200, Ok(())),
            (204, Ok(())),
            (404, Ok(())),
            (429, Err(IdentityDeletionError::Retryable)),
            (503, Err(IdentityDeletionError::Retryable)),
            (401, Err(IdentityDeletionError::Permanent)),
            (403, Err(IdentityDeletionError::Permanent)),
        ] {
            server.reset().await;
            Mock::given(method("DELETE"))
                .and(path(endpoint))
                .and(header("authorization", "Bearer private-key"))
                .respond_with(ResponseTemplate::new(status))
                .expect(1)
                .mount(&server)
                .await;
            assert_eq!(
                deleter.delete_account("user_123").await,
                expected,
                "{endpoint} {status}"
            );
            server.verify().await;
        }
        for invalid in ["", ".", ".."] {
            assert_eq!(
                deleter.delete_account(invalid).await,
                Err(IdentityDeletionError::Permanent)
            );
        }
    }
    Ok(())
}

#[test]
fn rejects_insecure_urls_and_redacts_keys() {
    for base in [
        "http://remote.example",
        "https://user:pass@example.com",
        "https://example.com/?token=x",
        "https://example.com/#x",
    ] {
        assert!(deleters(base).is_err(), "{base}");
    }
    let config = ApiDeletionConfig {
        base_url: "https://api.example".into(),
        api_key: Secret::new("private-key".into()),
        allow_local_http: false,
    };
    assert!(!format!("{config:?}").contains("private-key"));
}

#[path = "support/postgres_fixture.rs"]
mod postgres_fixture;

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn server_errors_are_retried_until_the_durable_operation_fails() -> Result<(), Box<dyn Error>>
{
    use baukit_erasure::{ErasureService, ErasureState, IdentityDeletionHandler};
    use baukit_jobs::{PostgresJobStore, WorkerConfig, WorkerRunner};
    use baukit_runtime::ShutdownToken;
    let server = MockServer::start().await;
    for DeletionCase { deleter, endpoint } in deleters(&server.uri())? {
        server.reset().await;
        Mock::given(method("DELETE"))
            .and(path(endpoint))
            .respond_with(ResponseTemplate::new(503))
            .expect(3)
            .mount(&server)
            .await;
        let (_container, pool, store, _fake, _service) = postgres_fixture::fixture().await?;
        let service = ErasureService::new(
            store.clone(),
            IdentityRetention::Delete {
                deleter: deleter.clone(),
                provider_id: "api".into(),
                inline_timeout: Duration::from_secs(1),
                max_attempts: 2,
            },
        )?;
        let pending = service
            .erase(
                "user_123",
                "provider-delete-key-01",
                &postgres_fixture::Product,
            )
            .await?;
        assert_eq!(pending.status, ErasureState::Pending);
        let runner = WorkerRunner::new(
            Arc::new(PostgresJobStore::new(pool.clone())),
            Arc::new(IdentityDeletionHandler::new(
                store.clone(),
                "api".into(),
                deleter,
            )),
            WorkerConfig {
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
                if store
                    .status("user_123", pending.operation_id)
                    .await?
                    .is_some_and(|value| value.status == ErasureState::Failed)
                {
                    break Ok::<_, baukit_erasure::ErasureError>(());
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        shutdown.trigger();
        worker.await??;
        result??;
        assert_eq!(postgres_fixture::count(&pool, "job_outbox").await?, 1);
        server.verify().await;
    }
    Ok(())
}
