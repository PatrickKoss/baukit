#![cfg(test)]
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
    collection: &'static str,
    missing_body: serde_json::Value,
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
            collection: "/users",
            missing_body: serde_json::json!({"errors": [{"message": "not found", "code": "resource_not_found"}]}),
        },
        DeletionCase {
            deleter: Arc::new(WorkOsAccountDeleter::new(config)?),
            endpoint: "/user_management/users/user_123",
            collection: "/user_management/users",
            missing_body: serde_json::json!({"code": "entity_not_found", "message": "User not found"}),
        },
    ])
}

#[tokio::test]
async fn deletion_is_authenticated_idempotent_and_classifies_failures() -> Result<(), Box<dyn Error>>
{
    let server = MockServer::start().await;
    for DeletionCase {
        deleter,
        endpoint,
        missing_body,
        ..
    } in deleters(&server.uri())?
    {
        for (status, body, expected) in [
            (404, Some(missing_body.clone()), Ok(())),
            (404, None, Err(IdentityDeletionError::Permanent)),
            (
                404,
                Some(
                    serde_json::json!({"errors": [{"code": "route_not_found"}], "code": "not_found"}),
                ),
                Err(IdentityDeletionError::Permanent),
            ),
        ] {
            server.reset().await;
            let mut response = ResponseTemplate::new(status);
            if let Some(body) = body {
                response = response.set_body_json(body);
            }
            Mock::given(method("DELETE"))
                .and(path(endpoint))
                .respond_with(response)
                .expect(1)
                .mount(&server)
                .await;
            assert_eq!(
                deleter.delete_account("user_123").await,
                expected,
                "{endpoint}"
            );
            server.verify().await;
        }
        for (status, expected) in [
            (200, Ok(())),
            (204, Ok(())),
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
        for invalid in ["", ".", "..", "a\nb"] {
            assert_eq!(
                deleter.delete_account(invalid).await,
                Err(IdentityDeletionError::Permanent)
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn probe_requires_the_provider_collection_to_answer() -> Result<(), Box<dyn Error>> {
    let server = MockServer::start().await;
    for DeletionCase {
        deleter,
        collection,
        ..
    } in deleters(&server.uri())?
    {
        server.reset().await;
        assert_eq!(deleter.probe().await, Err(IdentityDeletionError::Permanent));
        Mock::given(method("GET"))
            .and(path(collection))
            .and(header("authorization", "Bearer private-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": []})))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(deleter.probe().await, Ok(()));
        server.verify().await;
    }
    Ok(())
}

#[test]
fn rejects_insecure_urls_and_redacts_keys() {
    for base in [
        "http://remote.example",
        "http://localhost.example.com",
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
    for DeletionCase {
        deleter, endpoint, ..
    } in deleters(&server.uri())?
    {
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
                None,
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

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn misconfigured_base_url_never_completes_an_operation() -> Result<(), Box<dyn Error>> {
    use baukit_erasure::{ErasureService, ErasureState};
    let server = MockServer::start().await;
    let wrong_base = format!("{}/missing-version-prefix", server.uri());
    for DeletionCase { deleter, .. } in deleters(&wrong_base)? {
        assert!(deleter.probe().await.is_err());
        let (_container, _pool, store, _fake, _service) = postgres_fixture::fixture().await?;
        let service = ErasureService::new(
            store.clone(),
            IdentityRetention::Delete {
                deleter,
                provider_id: "api".into(),
                inline_timeout: Duration::from_secs(1),
                max_attempts: 2,
            },
        )?;
        assert!(service.probe_identity_provider().await.is_err());
        let outcome = service
            .erase(
                "alice",
                None,
                "misconfigured-key-01",
                &postgres_fixture::Product,
            )
            .await?;
        assert_eq!(outcome.status, ErasureState::Pending);
        let stored = store.status("alice", outcome.operation_id).await?;
        assert_ne!(
            stored.map(|value| value.status),
            Some(ErasureState::Completed)
        );
    }
    Ok(())
}
