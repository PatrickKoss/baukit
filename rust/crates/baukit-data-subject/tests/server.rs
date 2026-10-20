#![cfg(feature = "server")]

#[path = "support/server_fixture.rs"]
mod fixture;

use std::{
    error::Error,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use baukit_data_subject::*;
use baukit_jobs::{JobStore, PostgresJobStore};
use chrono::{TimeDelta, Utc};
use fixture::{Fixture, request_body};
use serde_json::{Value, json};
use tower::ServiceExt as _;
use uuid::Uuid;

type TestResult = Result<(), Box<dyn Error>>;

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn discovery_auth_and_validation_errors_use_the_contract_envelope() -> TestResult {
    let f = Fixture::new(false).await?;
    let (status, headers, discovery) = f
        .send(
            Request::builder()
                .uri("/.well-known/baukit/data-subject")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["cache-control"], "public, max-age=300");
    let discovery: DiscoveryDocument = serde_json::from_value(discovery)?;
    assert_eq!(discovery, f.store.discovery());
    assert_eq!(discovery.operations, [OperationKind::Erasure]);
    assert_eq!(discovery.scopes, Scopes::default());
    ReceiptVerifier::from_discovery(&discovery)?;

    for (token, status, code) in [
        (None, 401, "unauthenticated"),
        (
            Some(f.token("outsider", "data-subject.erase")?),
            403,
            codes::CLIENT_NOT_ALLOWED,
        ),
        (
            Some(f.token_without_client()?),
            403,
            codes::CLIENT_NOT_ALLOWED,
        ),
        (Some(f.api_token.clone()), 403, codes::CLIENT_NOT_ALLOWED),
        (
            Some(f.token("privacy", "")?),
            403,
            codes::INSUFFICIENT_SCOPE,
        ),
        (
            Some(f.token("privacy", "data-subject.export")?),
            403,
            codes::INSUFFICIENT_SCOPE,
        ),
        (
            Some(f.token("privacy", "data-subject.erase-extra")?),
            403,
            codes::INSUFFICIENT_SCOPE,
        ),
    ] {
        f.assert_error(
            f.post_request(
                token.as_deref(),
                &["valid-request-key-01"],
                request_body(&["erase@example.invalid"]),
            )?,
            status,
            code,
        )
        .await?;
        f.assert_error(
            f.get_request(token.as_deref(), Uuid::now_v7())?,
            status,
            code,
        )
        .await?;
    }
    let token = f.token("privacy", "data-subject.erase")?;
    for keys in [
        vec![],
        vec!["short"],
        vec!["has a space in key"],
        vec!["valid-request-key-01", "valid-request-key-01"],
    ] {
        f.assert_error(
            f.post_request(
                Some(&token),
                &keys,
                request_body(&["erase@example.invalid"]),
            )?,
            400,
            codes::IDEMPOTENCY_KEY_INVALID,
        )
        .await?;
    }
    let valid = request_body(&["erase@example.invalid"]);
    let invalid = [
        json!({"subjectReference":"short", "identifiers":valid["identifiers"]}),
        json!({"subjectReference":"opaque-reference-01", "identifiers":[]}),
        json!({"subjectReference":"opaque-reference-01", "identifiers":vec![json!({"type":"email", "value":"a"}); 17]}),
        json!({"subjectReference":"opaque-reference-01", "identifiers":[{"type":"email", "value":"a".repeat(257)}]}),
        json!({"subjectReference":"opaque-reference-01", "identifiers":[{"type":"email", "value":"   "}]}),
        json!({"subjectReference":"opaque-reference-01", "identifiers":[{"type":"phone", "value":"+123"}]}),
        json!({"subjectReference":"opaque-reference-01", "identifiers":[{"type":"x-Bad", "value":"value"}]}),
        json!({"subjectReference":"opaque-reference-01", "identifiers":[{"type":12, "value":"value"}]}),
        json!({"subjectReference":"opaque-reference-01", "identifiers":[{"type":"email", "value":12}]}),
        json!({"subjectReference":"opaque-reference-01"}),
        Value::Null,
    ];
    for body in invalid {
        f.assert_error(
            f.post_request(Some(&token), &["valid-request-key-01"], body)?,
            422,
            codes::REQUEST_INVALID,
        )
        .await?;
    }
    for (content_type, raw) in [
        ("application/json", "{"),
        ("text/plain", "{}"),
        ("application/json", "[]"),
    ] {
        let request = Request::builder()
            .method("POST")
            .uri(fixture::ERASURES)
            .header("authorization", format!("Bearer {token}"))
            .header("idempotency-key", "valid-request-key-01")
            .header("content-type", content_type)
            .body(Body::from(raw))?;
        f.assert_error(request, 422, codes::REQUEST_INVALID).await?;
    }
    f.assert_error(f.post_request(Some(&token), &["valid-request-key-01"], json!({"subjectReference":"opaque-reference-01", "identifiers":[{"type":"x-loyalty", "value":"value"}]}))?, 422, codes::IDENTIFIER_TYPE_UNSUPPORTED).await?;
    assert_eq!(f.operation_count().await?, 0);
    assert_eq!(f.job_count().await?, 0);
    assert_eq!(f.product.resolves.load(Ordering::SeqCst), 0);
    let (status, _) = f
        .post(
            &token,
            "valid-request-key-01,valid-request-key-02",
            request_body(&["erase@example.invalid"]),
        )
        .await?;
    assert_eq!(status, StatusCode::ACCEPTED);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn concurrent_posts_replay_conflict_and_isolate_client_and_kind() -> TestResult {
    let f = Fixture::new(false).await?;
    let token = f.token("privacy", "data-subject.erase")?;
    let barrier = Arc::new(tokio::sync::Barrier::new(9));
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let router = f.router.clone();
        let request = f.post_request(
            Some(&token),
            &["concurrent-key-001"],
            request_body(&[" ERASE@EXAMPLE.INVALID "]),
        )?;
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            let response = router.oneshot(request).await?;
            assert_eq!(response.status(), StatusCode::ACCEPTED);
            Ok::<_, Box<dyn Error + Send + Sync>>(serde_json::from_slice::<OperationResource>(
                &axum::body::to_bytes(response.into_body(), 1_048_576).await?,
            )?)
        }));
    }
    barrier.wait().await;
    let mut operations = Vec::new();
    for task in tasks {
        operations.push(task.await?.map_err(|_| "concurrent POST failed")?);
    }
    assert!(operations.iter().all(|op| op == &operations[0]));
    let first = &operations[0];
    assert_eq!(first.status, OperationStatus::Pending);
    assert!(first.receipt.is_none());
    assert_eq!(f.operation_count().await?, 1);
    assert_eq!(f.job_count().await?, 1);
    assert_eq!(f.product.resolves.load(Ordering::SeqCst), 1);
    let (status, replay) = f
        .post(
            &token,
            "concurrent-key-001",
            request_body(&["erase@example.invalid"]),
        )
        .await?;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(&replay, first);
    f.assert_error(
        f.post_request(
            Some(&token),
            &["concurrent-key-001"],
            request_body(&["other@example.invalid"]),
        )?,
        409,
        codes::IDEMPOTENCY_CONFLICT,
    )
    .await?;
    let mut different_reference = request_body(&["erase@example.invalid"]);
    different_reference["subjectReference"] = json!("another-reference-01");
    f.assert_error(
        f.post_request(Some(&token), &["concurrent-key-001"], different_reference)?,
        409,
        codes::IDEMPOTENCY_CONFLICT,
    )
    .await?;
    let other = f.token("privacy-other", "data-subject.erase")?;
    f.assert_error(
        f.get_request(Some(&other), first.operation_id)?,
        404,
        codes::OPERATION_NOT_FOUND,
    )
    .await?;
    f.assert_error(
        f.get_request(Some(&token), Uuid::now_v7())?,
        404,
        codes::OPERATION_NOT_FOUND,
    )
    .await?;
    let (_, owned) = f
        .post(
            &other,
            "concurrent-key-001",
            request_body(&["erase@example.invalid"]),
        )
        .await?;
    assert_ne!(owned.operation_id, first.operation_id);
    sqlx::query("UPDATE data_subject_operations SET kind = 'export' WHERE id = $1")
        .bind(owned.operation_id)
        .execute(&f.pool)
        .await?;
    f.assert_error(
        f.get_request(Some(&other), owned.operation_id)?,
        404,
        codes::OPERATION_NOT_FOUND,
    )
    .await?;
    let (_, own_kind) = f
        .post(
            &other,
            "concurrent-key-001",
            request_body(&["erase@example.invalid"]),
        )
        .await?;
    assert_ne!(own_kind.operation_id, owned.operation_id);
    f.run_until(first.operation_id).await?;
    let (_, terminal) = f
        .post(
            &token,
            "concurrent-key-001",
            request_body(&["erase@example.invalid"]),
        )
        .await?;
    let (status, replay) = f
        .post(
            &token,
            "concurrent-key-001",
            request_body(&["erase@example.invalid"]),
        )
        .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(replay, terminal);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn signed_results_cover_completed_not_found_and_restricted_with_session_fences() -> TestResult
{
    let f = Fixture::new(false).await?;
    let token = f.token("privacy", "data-subject.erase")?;
    let before = f.session_token("auth-erased")?;
    let other = f.session_token("auth-other")?;
    assert_eq!(f.session_status(&before).await?, StatusCode::OK);
    assert_eq!(f.session_status(&other).await?, StatusCode::OK);
    let discovery = f
        .send(
            Request::builder()
                .uri("/.well-known/baukit/data-subject")
                .body(Body::empty())?,
        )
        .await?
        .2;
    let verifier = ReceiptVerifier::from_discovery(&serde_json::from_value(discovery)?)?;
    for (key, inputs, expected) in [
        (
            "completed-key-001",
            vec!["erase@example.invalid"],
            OperationStatus::Completed,
        ),
        (
            "not-found-key-001",
            vec!["absent@example.invalid"],
            OperationStatus::NotFound,
        ),
        (
            "restricted-key-001",
            vec!["restricted@example.invalid"],
            OperationStatus::Restricted,
        ),
    ] {
        let (_, operation) = f.post(&token, key, request_body(&inputs)).await?;
        let result = f.run_until(operation.operation_id).await?;
        assert_eq!(result.status, expected);
        assert!(result.completed_at.is_some());
        assert!(result.error.is_none());
        let payload = verifier.verify(result.receipt.as_ref().ok_or("missing receipt")?)?;
        assert_eq!(payload.operation_id, result.operation_id);
        assert_eq!(payload.kind, OperationKind::Erasure);
        assert_eq!(payload.status, expected);
        assert_eq!(payload.subject_reference, result.subject_reference);
        assert_eq!(payload.categories, result.categories);
        assert_eq!(payload.restrictions, result.restrictions);
        assert_eq!(payload.created_at, result.created_at);
        assert_eq!(
            payload.completed_at,
            result.completed_at.ok_or("completion")?
        );
        assert_eq!(payload.service, f.store.discovery().service);
        if expected == OperationStatus::NotFound {
            assert!(result.categories.is_empty());
            assert!(result.restrictions.is_empty());
        } else {
            assert_eq!(result.categories[0].action, CategoryAction::Erased);
            assert_eq!(result.categories[0].count, 1);
        }
        if expected == OperationStatus::Restricted {
            assert_eq!(result.restrictions.len(), 1);
            assert_eq!(result.categories[1].action, CategoryAction::Retained);
        }
        let (status, replay) = f.post(&token, key, request_body(&inputs)).await?;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(replay, result);
    }
    assert_eq!(f.session_status(&before).await?, StatusCode::UNAUTHORIZED);
    assert_eq!(f.session_status(&other).await?, StatusCode::OK);
    assert_eq!(f.job_count().await?, 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM erasure_operations WHERE state = 'completed'"
        )
        .fetch_one(&f.pool)
        .await?,
        2
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn execution_failure_rolls_back_every_subject_and_exhaustion_has_no_receipt() -> TestResult {
    let f = Fixture::new(false).await?;
    f.product.fail_subject.store(true, Ordering::SeqCst);
    let token = f.token("privacy", "data-subject.erase")?;
    let (_, operation) = f
        .post(
            &token,
            "failing-request-001",
            request_body(&["erase@example.invalid", "restricted@example.invalid"]),
        )
        .await?;
    let result = f.run_until(operation.operation_id).await?;
    assert_eq!(result.status, OperationStatus::Failed);
    assert_eq!(
        result.error.as_ref().ok_or("missing failure")?.code,
        codes::OPERATION_FAILED
    );
    assert!(result.receipt.is_none());
    assert!(result.categories.is_empty());
    assert!(result.restrictions.is_empty());
    assert_eq!(f.subject_count().await?, 3);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM erasure_fences")
            .fetch_one(&f.pool)
            .await?,
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM erasure_operations")
            .fetch_one(&f.pool)
            .await?,
        0
    );
    let (attempts, status, payload, error): (i32, String, Value, Option<String>) = sqlx::query_as(
        "SELECT attempts, status, payload, last_error FROM job_outbox WHERE job_type = $1",
    )
    .bind(ERASURE_JOB_TYPE)
    .fetch_one(&f.pool)
    .await?;
    assert_eq!(attempts, 2);
    assert_eq!(status, "failed");
    assert_eq!(payload, json!({"operationId":operation.operation_id}));
    assert_eq!(error.as_deref(), Some("data subject execution unavailable"));
    assert_eq!(f.product.resolves.load(Ordering::SeqCst), 1);
    assert_eq!(f.product.erases.load(Ordering::SeqCst), 4);
    let (status, replay) = f
        .post(
            &token,
            "failing-request-001",
            request_body(&["erase@example.invalid", "restricted@example.invalid"]),
        )
        .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(replay, result);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn expired_final_lease_also_fails_the_operation_and_scrubs_subjects() -> TestResult {
    let f = Fixture::new(false).await?;
    let token = f.token("privacy", "data-subject.erase")?;
    let (_, operation) = f
        .post(
            &token,
            "lease-expired-key-01",
            request_body(&["erase@example.invalid"]),
        )
        .await?;
    let jobs = PostgresJobStore::new(f.pool.clone());
    let mut now = Utc::now();
    for _ in 0..2 {
        assert!(
            jobs.claim(
                "expired-worker",
                &[ERASURE_JOB_TYPE],
                now,
                Duration::from_secs(1)
            )
            .await?
            .is_some()
        );
        now += TimeDelta::seconds(2);
    }
    assert!(
        jobs.claim(
            "expired-worker",
            &[ERASURE_JOB_TYPE],
            now,
            Duration::from_secs(1)
        )
        .await?
        .is_none()
    );
    let operation = f
        .store
        .status("privacy", OperationKind::Erasure, operation.operation_id)
        .await?
        .ok_or("missing operation")?;
    assert_eq!(operation.status, OperationStatus::Failed);
    assert!(operation.receipt.is_none());
    assert_eq!(
        operation.error.ok_or("error")?.code,
        codes::OPERATION_FAILED
    );
    assert_eq!(
        sqlx::query_scalar::<_, Value>("SELECT payload FROM job_outbox WHERE job_type = $1")
            .bind(ERASURE_JOB_TYPE)
            .fetch_one(&f.pool)
            .await?,
        json!({"operationId": operation.operation_id})
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn acceptance_rolls_back_when_resolution_or_enqueue_fails() -> TestResult {
    let f = Fixture::new(false).await?;
    let token = f.token("privacy", "data-subject.erase")?;
    f.product.fail_resolve.store(true, Ordering::SeqCst);
    f.assert_error(
        f.post_request(
            Some(&token),
            &["resolve-failure-001"],
            request_body(&["erase@example.invalid"]),
        )?,
        500,
        "internal",
    )
    .await?;
    assert_eq!(f.operation_count().await?, 0);
    f.product.fail_resolve.store(false, Ordering::SeqCst);
    sqlx::raw_sql("CREATE FUNCTION reject_enqueue() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'enqueue rejected'; END; $$; CREATE TRIGGER reject_enqueue BEFORE INSERT ON job_outbox FOR EACH ROW EXECUTE FUNCTION reject_enqueue();").execute(&f.pool).await?;
    f.assert_error(
        f.post_request(
            Some(&token),
            &["enqueue-failure-001"],
            request_body(&["erase@example.invalid"]),
        )?,
        500,
        "internal",
    )
    .await?;
    assert_eq!(f.operation_count().await?, 0);
    assert_eq!(f.job_count().await?, 0);
    assert_eq!(f.subject_count().await?, 3);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn purge_is_bounded_and_removes_only_expired_operations_and_execution_payloads() -> TestResult
{
    let f = Fixture::new(false).await?;
    let token = f.token("privacy", "data-subject.erase")?;
    let mut ids = Vec::new();
    for key in [
        "purge-expired-key-01",
        "purge-expired-key-02",
        "purge-retained-key-01",
    ] {
        ids.push(
            f.post(&token, key, request_body(&["absent@example.invalid"]))
                .await?
                .1
                .operation_id,
        );
    }
    f.run_until(ids[0]).await?;
    sqlx::query("UPDATE data_subject_operations SET purge_after = now() - interval '1 day' WHERE id = ANY($1)").bind(&ids[..2]).execute(&f.pool).await?;
    assert_eq!(
        f.store.purge_expired(Utc::now(), 0).await,
        Err(DataSubjectError::Configuration)
    );
    assert_eq!(
        f.store
            .purge_expired(Utc::now(), MAX_PURGE_BATCH_SIZE + 1)
            .await,
        Err(DataSubjectError::Configuration)
    );
    assert_eq!(f.store.purge_expired(Utc::now(), 1).await?, 1);
    assert_eq!(f.store.purge_expired(Utc::now(), 1).await?, 1);
    assert_eq!(f.store.purge_expired(Utc::now(), 1).await?, 0);
    assert_eq!(f.operation_count().await?, 1);
    assert!(
        f.store
            .status("privacy", OperationKind::Erasure, ids[2])
            .await?
            .is_some()
    );
    let remaining: Vec<String> = sqlx::query_scalar("SELECT idempotency_key FROM job_outbox")
        .fetch_all(&f.pool)
        .await?;
    assert!(remaining.iter().all(|key| key == &ids[2].to_string()));
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn delete_retention_queues_existing_identity_worker_and_keeps_sessions_fenced() -> TestResult
{
    let f = Fixture::new(true).await?;
    let token = f.token("privacy", "data-subject.erase")?;
    let session = f.session_token("auth-erased")?;
    let (_, operation) = f
        .post(
            &token,
            "delete-identity-key-01",
            request_body(&["erase@example.invalid"]),
        )
        .await?;
    f.run_local_erasure(operation.operation_id).await?;
    let pending = f
        .store
        .status("privacy", OperationKind::Erasure, operation.operation_id)
        .await?
        .ok_or("pending operation")?;
    assert_eq!(pending.status, OperationStatus::Pending);
    assert!(pending.receipt.is_none());
    assert!(pending.completed_at.is_none());
    assert_eq!(f.session_status(&session).await?, StatusCode::UNAUTHORIZED);
    assert!(f.deleter.calls().is_empty());
    let payload: Value = sqlx::query_scalar("SELECT payload FROM job_outbox WHERE job_type = $1")
        .bind(baukit_erasure::IDENTITY_DELETE_JOB_TYPE)
        .fetch_one(&f.pool)
        .await?;
    let identity_operation = serde_json::from_value::<Uuid>(payload["operationId"].clone())?;
    assert_eq!(
        f.erasure
            .store()
            .status("auth-erased", identity_operation)
            .await?
            .ok_or("identity operation")?
            .status,
        baukit_erasure::ErasureState::Pending
    );
    f.deleter
        .push_outcome(Err(baukit_erasure::IdentityDeletionError::Retryable));
    f.run_handler_until(
        Arc::new(baukit_erasure::IdentityDeletionHandler::new(
            f.erasure.store().clone(),
            "fixture".into(),
            f.deleter.clone(),
        )),
        || async {
            let state: String =
                sqlx::query_scalar("SELECT state FROM erasure_operations WHERE id = $1")
                    .bind(identity_operation)
                    .fetch_one(&f.pool)
                    .await?;
            Ok(state == "completed")
        },
    )
    .await?;
    assert_eq!(f.deleter.calls().len(), 2);
    assert!(
        f.deleter
            .calls()
            .iter()
            .all(|subject| subject == "auth-erased")
    );
    let completed = f.run_until(operation.operation_id).await?;
    assert_eq!(completed.status, OperationStatus::Completed);
    ReceiptVerifier::from_discovery(&f.store.discovery())?
        .verify(completed.receipt.as_ref().ok_or("receipt")?)?;
    assert_eq!(f.job_count().await?, 0);
    assert_eq!(f.session_status(&session).await?, StatusCode::UNAUTHORIZED);
    assert_eq!(
        f.session_status(&f.session_token("auth-other")?).await?,
        StatusCode::OK
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn identifiers_never_enter_any_table_job_payload_receipt_error_or_captured_log() -> TestResult
{
    let logs = fixture::Logs::default();
    tracing::subscriber::set_global_default(logs.subscriber())?;
    tracing::info!("data subject privacy capture active");
    let f = Fixture::new(true).await?;
    let token = f.token("privacy", "data-subject.erase")?;
    let forbidden = [
        "erase@example.invalid",
        " ERASE@EXAMPLE.INVALID ",
        "restricted@example.invalid",
        "absent@example.invalid",
        &token,
        &f.api_token,
        "private-request-key-01",
    ];
    let (_, operation) = f
        .post(
            &token,
            "private-request-key-01",
            request_body(&[" ERASE@EXAMPLE.INVALID ", "restricted@example.invalid"]),
        )
        .await?;
    f.assert_no_stored_values(&forbidden).await?;
    f.run_local_erasure(operation.operation_id).await?;
    f.assert_no_stored_values(&forbidden).await?;
    f.run_identities().await?;
    let result = f.run_until(operation.operation_id).await?;
    let payload = ReceiptVerifier::from_discovery(&f.store.discovery())?
        .verify(result.receipt.as_ref().ok_or("receipt")?)?;
    let receipt_json = serde_json::to_string(&payload)?;
    assert!(forbidden.iter().all(|value| !receipt_json.contains(value)));
    f.assert_no_stored_values(&forbidden).await?;
    f.product.fail_resolve.store(true, Ordering::SeqCst);
    f.assert_error(
        f.post_request(
            Some(&token),
            &["private-failure-key-01"],
            request_body(&["erase@example.invalid"]),
        )?,
        500,
        "internal",
    )
    .await?;
    f.assert_no_stored_values(&forbidden).await?;
    let text = logs.text();
    assert!(text.contains("data subject privacy capture active"));
    assert!(forbidden.iter().all(|value| !text.contains(value)));
    Ok(())
}

#[tokio::test]
async fn configuration_rejects_unsafe_keys_limits_retention_and_receipt_keys() -> TestResult {
    use baukit_config::Secret;
    use baukit_erasure::{ErasureService, IdentityRetention, PostgresErasureStore};
    let pool = sqlx::PgPool::connect_lazy("postgres://localhost/configuration")?;
    for invalid in 0..12 {
        let erasure = ErasureService::new(
            PostgresErasureStore::new(
                pool.clone(),
                Secret::new("configuration-fence-key-at-least-32-bytes".into()),
            )?,
            IdentityRetention::Retain,
        )?;
        let mut config = DataSubjectConfig::new(
            Service {
                name: "fixture".into(),
                version: "1".into(),
            },
            ["privacy".into()].into_iter().collect(),
            vec![IdentifierType::Email],
            ReceiptSigner::from_pkcs8_der(include_bytes!(
                "../../../../fixtures/data-subject/v1/test-only-private.der"
            ))?,
            Secret::new("configuration-hash-key-at-least-32-bytes".into()),
            erasure,
        );
        assert_eq!(config.identity_deletion_timeout, TimeDelta::days(1));
        match invalid {
            0 => config.hash_key = Secret::new("too-short".into()),
            1 => config.limits.max_identifiers = 0,
            2 => config.operation_retention = TimeDelta::zero(),
            3 => config.max_attempts = 0,
            4 => config.allowed_client_ids.clear(),
            5 => config.identifier_types.push(IdentifierType::Email),
            6 => config
                .previous_receipt_keys
                .push(config.signer.receipt_key().clone()),
            7 => {
                let mut key = config.signer.receipt_key().clone();
                key.status = ReceiptKeyStatus::Previous;
                config.previous_receipt_keys.push(key);
            }
            8 => config.service.name.clear(),
            9 => config.identity_deletion_timeout = TimeDelta::zero(),
            10 => config.identity_deletion_timeout = -TimeDelta::seconds(1),
            _ => config.identity_deletion_timeout = TimeDelta::days(30) + TimeDelta::seconds(1),
        }
        assert!(matches!(
            PostgresDataSubjectStore::new(pool.clone(), config),
            Err(DataSubjectError::Configuration)
        ));
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn identity_finalization_checks_back_off_and_cap_at_sixty_seconds() -> TestResult {
    let f = Fixture::new(true).await?;
    let token = f.token("privacy", "data-subject.erase")?;
    let (_, operation) = f
        .post(
            &token,
            "identity-backoff-key-01",
            request_body(&["erase@example.invalid"]),
        )
        .await?;
    let mut before = Utc::now();
    f.run_local_erasure(operation.operation_id).await?;
    let mut after = Utc::now();
    for (count, seconds) in [1, 2, 4, 8, 16, 32, 60, 60].into_iter().enumerate() {
        let (job_id, payload, run_after, attempts): (Uuid, Value, chrono::DateTime<Utc>, i32) =
            sqlx::query_as("SELECT id, payload, run_after, attempts FROM job_outbox WHERE job_type = $1 AND idempotency_key = $2")
                .bind(FINALIZATION_JOB_TYPE).bind(operation.operation_id.to_string())
                .fetch_one(&f.pool).await?;
        assert_eq!(
            payload,
            json!({"operationId": operation.operation_id, "checkCount": count})
        );
        assert_eq!(attempts, 0);
        assert!(run_after >= before + TimeDelta::seconds(seconds));
        assert!(run_after <= after + TimeDelta::seconds(seconds));
        assert_eq!(f.product.erases.load(Ordering::SeqCst), 1);
        if count < 7 {
            before = Utc::now();
            sqlx::query("UPDATE job_outbox SET run_after = now() WHERE id = $1")
                .bind(job_id)
                .execute(&f.pool)
                .await?;
            f.run_handler_until(
                Arc::new(DataSubjectHandler::new(f.store.clone(), f.product.clone())),
                || async {
                    let remaining: bool = sqlx::query_scalar(
                        "SELECT EXISTS (SELECT 1 FROM job_outbox WHERE id = $1)",
                    )
                    .bind(job_id)
                    .fetch_one(&f.pool)
                    .await?;
                    Ok(!remaining)
                },
            )
            .await?;
            after = Utc::now();
        }
    }
    let pending = f
        .store
        .status("privacy", OperationKind::Erasure, operation.operation_id)
        .await?
        .ok_or("pending operation")?;
    assert_eq!(pending.status, OperationStatus::Pending);
    assert!(pending.receipt.is_none());
    assert!(f.deleter.calls().is_empty());
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn unfinished_identity_deletion_fails_at_the_configured_deadline() -> TestResult {
    let f = Fixture::with_config(true, |config| {
        config.identity_deletion_timeout = TimeDelta::seconds(5);
    })
    .await?;
    let token = f.token("privacy", "data-subject.erase")?;
    let session = f.session_token("auth-erased")?;
    let (_, operation) = f
        .post(
            &token,
            "identity-timeout-key-01",
            request_body(&["erase@example.invalid"]),
        )
        .await?;
    sqlx::query(
        "UPDATE data_subject_operations SET created_at = now() - interval '2 days' WHERE id = $1",
    )
    .bind(operation.operation_id)
    .execute(&f.pool)
    .await?;
    f.run_local_erasure(operation.operation_id).await?;
    let started_at: chrono::DateTime<Utc> = sqlx::query_scalar(
        "SELECT identity_deletion_started_at FROM data_subject_operations WHERE id = $1",
    )
    .bind(operation.operation_id)
    .fetch_one(&f.pool)
    .await?;
    let job_id: Uuid = sqlx::query_scalar("UPDATE job_outbox SET run_after = now(), payload = jsonb_set(payload, '{checkCount}', '6') WHERE job_type = $1 AND idempotency_key = $2 RETURNING id")
        .bind(FINALIZATION_JOB_TYPE).bind(operation.operation_id.to_string()).fetch_one(&f.pool).await?;
    f.run_handler_until(
        Arc::new(DataSubjectHandler::new(f.store.clone(), f.product.clone())),
        || async {
            let remaining: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM job_outbox WHERE id = $1)")
                    .bind(job_id)
                    .fetch_one(&f.pool)
                    .await?;
            Ok(!remaining)
        },
    )
    .await?;
    let run_after: chrono::DateTime<Utc> = sqlx::query_scalar(
        "SELECT run_after FROM job_outbox WHERE job_type = $1 AND idempotency_key = $2",
    )
    .bind(FINALIZATION_JOB_TYPE)
    .bind(operation.operation_id.to_string())
    .fetch_one(&f.pool)
    .await?;
    assert_eq!(run_after, started_at + TimeDelta::seconds(5));
    assert_eq!(
        f.store
            .status("privacy", OperationKind::Erasure, operation.operation_id)
            .await?
            .ok_or("pending operation")?
            .status,
        OperationStatus::Pending
    );
    sqlx::query("UPDATE data_subject_operations SET identity_deletion_started_at = clock_timestamp() - interval '6 seconds' WHERE id = $1")
        .bind(operation.operation_id).execute(&f.pool).await?;
    sqlx::query(
        "UPDATE job_outbox SET run_after = now() WHERE job_type = $1 AND idempotency_key = $2",
    )
    .bind(FINALIZATION_JOB_TYPE)
    .bind(operation.operation_id.to_string())
    .execute(&f.pool)
    .await?;
    let failed = f.run_until(operation.operation_id).await?;
    assert_eq!(failed.status, OperationStatus::Failed);
    assert!(failed.receipt.is_none());
    assert!(failed.completed_at.is_some());
    assert_eq!(
        failed.error.as_ref().ok_or("error")?.code,
        codes::OPERATION_FAILED
    );
    assert_eq!(f.subject_count().await?, 2);
    assert_eq!(f.session_status(&session).await?, StatusCode::UNAUTHORIZED);
    assert_eq!(
        f.session_status(&f.session_token("auth-other")?).await?,
        StatusCode::OK
    );
    assert!(f.deleter.calls().is_empty());
    let pending_identity: i64 =
        sqlx::query_scalar("SELECT count(*) FROM erasure_operations WHERE state = 'pending'")
            .fetch_one(&f.pool)
            .await?;
    assert_eq!(pending_identity, 1);
    assert_eq!(f.job_count().await?, 1);
    f.run_identities().await?;
    assert_eq!(
        f.store
            .status("privacy", OperationKind::Erasure, operation.operation_id)
            .await?,
        Some(failed)
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn provider_failure_never_produces_a_completed_data_subject_receipt() -> TestResult {
    let f = Fixture::new(true).await?;
    let token = f.token("privacy", "data-subject.erase")?;
    let (_, operation) = f
        .post(
            &token,
            "provider-failure-key-01",
            request_body(&["erase@example.invalid"]),
        )
        .await?;
    f.run_local_erasure(operation.operation_id).await?;
    f.deleter
        .push_outcome(Err(baukit_erasure::IdentityDeletionError::Permanent));
    f.run_handler_until(
        Arc::new(baukit_erasure::IdentityDeletionHandler::new(
            f.erasure.store().clone(),
            "fixture".into(),
            f.deleter.clone(),
        )),
        || async {
            let failed: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM erasure_operations WHERE state = 'failed')",
            )
            .fetch_one(&f.pool)
            .await?;
            Ok(failed)
        },
    )
    .await?;
    let failed = f.run_until(operation.operation_id).await?;
    assert_eq!(failed.status, OperationStatus::Failed);
    assert!(failed.receipt.is_none());
    assert_eq!(failed.error.ok_or("error")?.code, codes::OPERATION_FAILED);
    assert_eq!(f.subject_count().await?, 2);
    assert_eq!(
        f.session_status(&f.session_token("auth-erased")?).await?,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        f.session_status(&f.session_token("auth-other")?).await?,
        StatusCode::OK
    );
    Ok(())
}
