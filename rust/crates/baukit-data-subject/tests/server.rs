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
    let export = Fixture::with_exports(|config| {
        config.limits.max_export_part_bytes = 110;
    })
    .await?;
    let token = export.token("privacy", "data-subject.export")?;
    let export_forbidden = [
        "erase@example.invalid",
        " ERASE@EXAMPLE.INVALID ",
        "restricted@example.invalid",
        &token,
        &export.api_token,
        "private-export-key-01",
    ];
    let (_, pending) = post_export(
        &export,
        &token,
        "private-export-key-01",
        &[" ERASE@EXAMPLE.INVALID ", "restricted@example.invalid"],
    )
    .await?;
    export.assert_no_stored_values(&export_forbidden).await?;
    let completed = export.run_until(pending.operation_id).await?;
    let payload = ReceiptVerifier::from_discovery(&export.store.discovery())?
        .verify(completed.receipt.as_ref().ok_or("receipt")?)?;
    let json = serde_json::to_string(&payload)?;
    assert!(export_forbidden.iter().all(|value| !json.contains(value)));
    export.assert_no_stored_values(&export_forbidden).await?;
    export.product.fail_export.store(true, Ordering::SeqCst);
    let (_, pending) = post_export(
        &export,
        &token,
        "private-export-failure-01",
        &["restricted@example.invalid"],
    )
    .await?;
    export.run_until(pending.operation_id).await?;
    export.assert_no_stored_values(&export_forbidden).await?;
    assert!(
        export_forbidden
            .iter()
            .all(|value| !logs.text().contains(value))
    );
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
    for invalid in 0..18 {
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
        assert_eq!(config.export_retention, TimeDelta::days(7));
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
            11 => config.identity_deletion_timeout = TimeDelta::days(30) + TimeDelta::seconds(1),
            12 => config.export_retention = TimeDelta::zero(),
            13 => config.export_retention = -TimeDelta::seconds(1),
            14 => config.export_retention = TimeDelta::MAX,
            15 => config.limits.max_export_part_bytes = 0,
            16 => config.limits.max_export_part_bytes = u64::MAX,
            _ => config.operation_retention = TimeDelta::MAX,
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

fn as_export(mut request: Request<Body>) -> Request<Body> {
    *request.uri_mut() = request
        .uri()
        .to_string()
        .replace("/erasures", "/exports")
        .parse()
        .expect("export URI");
    request
}

fn part_request(
    f: &Fixture,
    token: Option<&str>,
    id: Uuid,
    number: &str,
) -> Result<Request<Body>, Box<dyn Error>> {
    let mut request = as_export(f.get_request(token, id)?);
    *request.uri_mut() = format!("/api/v1/data-subject/exports/{id}/parts/{number}").parse()?;
    Ok(request)
}

async fn post_export(
    f: &Fixture,
    token: &str,
    key: &str,
    inputs: &[&str],
) -> Result<(StatusCode, OperationResource), Box<dyn Error>> {
    let (status, _, body) = f
        .send(as_export(f.post_request(
            Some(token),
            &[key],
            request_body(inputs),
        )?))
        .await?;
    Ok((status, serde_json::from_value(body)?))
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn exports_are_opt_in_and_enforce_auth_scope_owner_and_kind_on_every_route() -> TestResult {
    let disabled = Fixture::new(false).await?;
    let token = disabled.token("privacy", "data-subject.export")?;
    assert_eq!(
        disabled.store.discovery().operations,
        [OperationKind::Erasure]
    );
    for request in [
        as_export(disabled.post_request(
            Some(&token),
            &["disabled-export-001"],
            request_body(&["erase@example.invalid"]),
        )?),
        as_export(disabled.get_request(Some(&token), Uuid::now_v7())?),
        part_request(&disabled, Some(&token), Uuid::now_v7(), "1")?,
    ] {
        assert_eq!(
            disabled.router.clone().oneshot(request).await?.status(),
            StatusCode::NOT_FOUND
        );
    }
    let f = Fixture::with_exports(|_| {}).await?;
    let (_, _, discovery) = f
        .send(
            Request::builder()
                .uri("/.well-known/baukit/data-subject")
                .body(Body::empty())?,
        )
        .await?;
    let discovery: DiscoveryDocument = serde_json::from_value(discovery)?;
    assert_eq!(
        discovery.operations,
        [OperationKind::Erasure, OperationKind::Export]
    );
    assert_eq!(discovery.limits.max_export_part_bytes, 8_388_608);
    for (token, status, code) in [
        (None, 401, "unauthenticated"),
        (
            Some(f.token("outsider", "data-subject.export")?),
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
            Some(f.token("privacy", "data-subject.erase")?),
            403,
            codes::INSUFFICIENT_SCOPE,
        ),
        (
            Some(f.token("privacy", "data-subject.export-extra")?),
            403,
            codes::INSUFFICIENT_SCOPE,
        ),
    ] {
        for request in [
            as_export(f.post_request(
                token.as_deref(),
                &["export-auth-key-01"],
                request_body(&["erase@example.invalid"]),
            )?),
            as_export(f.get_request(token.as_deref(), Uuid::now_v7())?),
            part_request(&f, token.as_deref(), Uuid::now_v7(), "1")?,
        ] {
            f.assert_error(request, status, code).await?;
        }
    }
    let export = f.token("privacy", "data-subject.export")?;
    let erase = f.token("privacy", "data-subject.erase")?;
    f.assert_error(
        f.post_request(
            Some(&export),
            &["export-auth-key-01"],
            request_body(&["erase@example.invalid"]),
        )?,
        403,
        codes::INSUFFICIENT_SCOPE,
    )
    .await?;
    f.assert_error(
        f.get_request(Some(&export), Uuid::now_v7())?,
        403,
        codes::INSUFFICIENT_SCOPE,
    )
    .await?;
    let (_, pending) = post_export(
        &f,
        &export,
        "export-owner-key-01",
        &["erase@example.invalid"],
    )
    .await?;
    let other = f.token("privacy-other", "data-subject.export")?;
    for id in [pending.operation_id, Uuid::now_v7()] {
        f.assert_error(
            as_export(f.get_request(Some(&other), id)?),
            404,
            codes::OPERATION_NOT_FOUND,
        )
        .await?;
        f.assert_error(
            part_request(&f, Some(&other), id, "1")?,
            404,
            codes::OPERATION_NOT_FOUND,
        )
        .await?;
    }
    f.assert_error(
        f.get_request(Some(&erase), pending.operation_id)?,
        404,
        codes::OPERATION_NOT_FOUND,
    )
    .await?;
    let (_, erasure) = f
        .post(
            &erase,
            "export-owner-key-01",
            request_body(&["absent@example.invalid"]),
        )
        .await?;
    f.assert_error(
        as_export(f.get_request(Some(&export), erasure.operation_id)?),
        404,
        codes::OPERATION_NOT_FOUND,
    )
    .await?;
    f.assert_error(
        part_request(&f, Some(&export), erasure.operation_id, "1")?,
        404,
        codes::OPERATION_NOT_FOUND,
    )
    .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn export_replays_conflicts_validation_and_failed_job_reconciliation() -> TestResult {
    let f = Fixture::with_exports(|_| {}).await?;
    let token = f.token("privacy", "data-subject.export")?;
    for (keys, body, status, code) in [
        (
            vec![],
            request_body(&["erase@example.invalid"]),
            400,
            codes::IDEMPOTENCY_KEY_INVALID,
        ),
        (
            vec!["short"],
            request_body(&["erase@example.invalid"]),
            400,
            codes::IDEMPOTENCY_KEY_INVALID,
        ),
        (
            vec!["export-replay-key-01"; 2],
            request_body(&["erase@example.invalid"]),
            400,
            codes::IDEMPOTENCY_KEY_INVALID,
        ),
        (
            vec!["export-replay-key-01"],
            request_body(&[]),
            422,
            codes::REQUEST_INVALID,
        ),
        (
            vec!["export-replay-key-01"],
            json!({"subjectReference":"opaque-reference-01","identifiers":[{"type":"x-unsupported","value":"value"}]}),
            422,
            codes::IDENTIFIER_TYPE_UNSUPPORTED,
        ),
    ] {
        f.assert_error(
            as_export(f.post_request(Some(&token), &keys, body)?),
            status,
            code,
        )
        .await?;
    }
    let (left, right) = tokio::join!(
        post_export(
            &f,
            &token,
            "export-replay-key-01",
            &[" ERASE@EXAMPLE.INVALID "]
        ),
        post_export(
            &f,
            &token,
            "export-replay-key-01",
            &["erase@example.invalid"]
        ),
    );
    let (status, pending) = left?;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(right?, (StatusCode::ACCEPTED, pending.clone()));
    assert_eq!(f.product.resolves.load(Ordering::SeqCst), 1);
    assert_eq!(f.job_count().await?, 1);
    for body in [
        request_body(&["other@example.invalid"]),
        json!({"subjectReference":"another-reference-01","identifiers":[{"type":"email","value":"erase@example.invalid"}]}),
    ] {
        f.assert_error(
            as_export(f.post_request(Some(&token), &["export-replay-key-01"], body)?),
            409,
            codes::IDEMPOTENCY_CONFLICT,
        )
        .await?;
    }
    f.assert_error(
        part_request(&f, Some(&token), pending.operation_id, "1")?,
        409,
        codes::EXPORT_NOT_READY,
    )
    .await?;
    let first_operation_id = pending.operation_id;
    let complete = f.run_until(pending.operation_id).await?;
    assert_eq!(
        post_export(
            &f,
            &token,
            "export-replay-key-01",
            &["erase@example.invalid"]
        )
        .await?,
        (StatusCode::OK, complete)
    );
    assert_eq!(f.product.exports.load(Ordering::SeqCst), 1);
    f.product.mutate_export.store(true, Ordering::SeqCst);
    f.product.fail_export.store(true, Ordering::SeqCst);
    let (_, pending) = post_export(
        &f,
        &token,
        "export-failure-key-01",
        &["erase@example.invalid", "restricted@example.invalid"],
    )
    .await?;
    let failed = f.run_until(pending.operation_id).await?;
    assert_eq!(failed.status, OperationStatus::Failed);
    assert_eq!(
        failed.error.as_ref().ok_or("error")?.code,
        codes::OPERATION_FAILED
    );
    assert!(failed.receipt.is_none());
    assert!(failed.categories.is_empty());
    assert!(failed.parts.is_empty());
    assert_eq!(f.subject_count().await?, 3);
    let (attempts, payload): (i32, Value) = sqlx::query_as(
        "SELECT attempts, payload FROM job_outbox WHERE job_type = $1 AND idempotency_key = $2",
    )
    .bind(EXPORT_JOB_TYPE)
    .bind(pending.operation_id.to_string())
    .fetch_one(&f.pool)
    .await?;
    assert_eq!(attempts, 2);
    assert_eq!(payload, json!({"operationId":pending.operation_id}));
    assert_eq!(
        post_export(
            &f,
            &token,
            "export-failure-key-01",
            &["erase@example.invalid", "restricted@example.invalid"]
        )
        .await?,
        (StatusCode::OK, failed)
    );
    let other = f.token("privacy-other", "data-subject.export")?;
    assert_ne!(
        post_export(
            &f,
            &other,
            "export-replay-key-01",
            &["erase@example.invalid"]
        )
        .await?
        .1
        .operation_id,
        first_operation_id
    );

    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn client_downloads_every_deterministic_part_over_http_and_verifies_the_signed_digests()
-> TestResult {
    use aws_lc_rs::digest::{SHA256, digest};
    let f = Fixture::with_exports(|config| {
        config.limits.max_export_part_bytes = 110;
    })
    .await?;
    let token = f.token("privacy", "data-subject.export")?;
    let session = f.session_token("auth-erased")?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let server = tokio::spawn(axum::serve(listener, f.router.clone()).into_future());
    struct StopServer(tokio::task::JoinHandle<std::io::Result<()>>);
    impl Drop for StopServer {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _server = StopServer(server);
    let client = reqwest::Client::builder().no_proxy().build()?;
    let discovery: DiscoveryDocument = client
        .get(format!("{url}/.well-known/baukit/data-subject"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let verifier = ReceiptVerifier::from_discovery(&discovery)?;
    let mut first_parts = None;
    for key in ["http-export-key-01", "http-export-key-02"] {
        let response = client
            .post(format!("{url}/api/v1/data-subject/exports"))
            .bearer_auth(&token)
            .header("Idempotency-Key", key)
            .json(&request_body(&[
                "restricted@example.invalid",
                "erase@example.invalid",
                "erase@example.invalid",
            ]))
            .send()
            .await?;
        assert_eq!(response.status().as_u16(), 202);
        let pending: OperationResource = response.json().await?;
        f.run_until(pending.operation_id).await?;
        let complete: OperationResource = client
            .get(format!(
                "{url}/api/v1/data-subject/exports/{}",
                pending.operation_id
            ))
            .bearer_auth(&token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        assert_eq!(complete.status, OperationStatus::Completed);
        assert_eq!(complete.kind, OperationKind::Export);
        let payload = verifier.verify(complete.receipt.as_ref().ok_or("receipt")?)?;
        assert_eq!(payload.operation_id, complete.operation_id);
        assert_eq!(payload.kind, complete.kind);
        assert_eq!(payload.status, complete.status);
        assert_eq!(payload.service, discovery.service);
        assert_eq!(payload.subject_reference, complete.subject_reference);
        assert_eq!(payload.categories, complete.categories);
        assert_eq!(payload.parts, complete.parts);
        assert_eq!(payload.created_at, complete.created_at);
        assert_eq!(Some(payload.completed_at), complete.completed_at);
        assert_eq!(
            complete.categories,
            vec![
                CategoryEntry {
                    category: "activity".into(),
                    action: CategoryAction::Exported,
                    count: 8
                },
                CategoryEntry {
                    category: "empty".into(),
                    action: CategoryAction::Exported,
                    count: 0
                },
                CategoryEntry {
                    category: "profiles".into(),
                    action: CategoryAction::Exported,
                    count: 2
                },
            ]
        );
        assert!(complete.restrictions.is_empty());
        assert!(complete.parts.len() > 3);
        let mut downloaded = Vec::new();
        let mut records = std::collections::BTreeMap::<String, Vec<Value>>::new();
        for (index, part) in payload.parts.iter().enumerate() {
            assert_eq!(part.part_number, index as u32 + 1);
            let response = client
                .get(format!(
                    "{url}/api/v1/data-subject/exports/{}/parts/{}",
                    pending.operation_id, part.part_number
                ))
                .bearer_auth(&token)
                .send()
                .await?
                .error_for_status()?;
            assert_eq!(response.headers()["content-type"], "application/json");
            assert_eq!(response.headers()["cache-control"], "no-store");
            let bytes = response.bytes().await?;
            assert!(bytes.len() <= 110);
            assert_eq!(bytes.len() as u64, part.bytes);
            let sha256: String = digest(&SHA256, &bytes)
                .as_ref()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            assert_eq!(sha256, part.sha256);
            let (stored, hash, length, expires_at): (Vec<u8>, String, i64, chrono::DateTime<Utc>) = sqlx::query_as("SELECT json_bytes, sha256, bytes, expires_at FROM data_subject_export_parts WHERE operation_id = $1 AND part_number = $2")
                .bind(pending.operation_id).bind(i64::from(part.part_number)).fetch_one(&f.pool).await?;
            assert_eq!(stored.as_slice(), bytes.as_ref());
            assert_eq!(hash, sha256);
            assert_eq!(length as u64, part.bytes);
            assert_eq!(expires_at, payload.completed_at + TimeDelta::days(7));
            let document: Value = serde_json::from_slice(&bytes)?;
            records
                .entry(document["category"].as_str().ok_or("category")?.into())
                .or_default()
                .extend(
                    document["records"]
                        .as_array()
                        .ok_or("records")?
                        .iter()
                        .cloned(),
                );
            downloaded.push(bytes.to_vec());
        }
        assert_eq!(
            records["activity"],
            (0..2)
                .flat_map(|_| (1..=4).map(|number| json!({"number":number,"value":"a".repeat(12)})))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            records["profiles"],
            vec![
                json!({"id":"internal-erased"}),
                json!({"id":"internal-restricted"})
            ]
        );
        if let Some((metadata, bytes)) = &first_parts {
            assert_eq!(&complete.parts, metadata);
            assert_eq!(&downloaded, bytes);
        } else {
            first_parts = Some((complete.parts, downloaded));
        }
    }
    assert_eq!(f.subject_count().await?, 3);
    assert_eq!(f.product.erases.load(Ordering::SeqCst), 0);
    assert_eq!(f.session_status(&session).await?, StatusCode::OK);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM erasure_fences")
            .fetch_one(&f.pool)
            .await?,
        0
    );
    assert!(f.deleter.calls().is_empty());
    assert_eq!(f.job_count().await?, 0);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn oversized_exports_roll_back_all_subjects_and_not_found_exports_have_signed_empty_results()
-> TestResult {
    let f = Fixture::with_exports(|config| {
        config.limits.max_export_part_bytes = 110;
    })
    .await?;
    let token = f.token("privacy", "data-subject.export")?;
    f.product.mutate_export.store(true, Ordering::SeqCst);
    f.product.oversized_export.store(true, Ordering::SeqCst);
    let (_, pending) = post_export(
        &f,
        &token,
        "oversized-export-01",
        &["erase@example.invalid", "restricted@example.invalid"],
    )
    .await?;
    let failed = f.run_until(pending.operation_id).await?;
    assert_eq!(failed.status, OperationStatus::Failed);
    assert!(failed.completed_at.is_some());
    assert_eq!(
        failed.error.as_ref().ok_or("error")?.code,
        codes::EXPORT_RECORD_TOO_LARGE
    );
    assert!(failed.receipt.is_none());
    assert!(failed.parts.is_empty());
    assert!(failed.categories.is_empty());
    assert_eq!(f.subject_count().await?, 3);
    assert_eq!(f.job_count().await?, 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM data_subject_export_parts")
            .fetch_one(&f.pool)
            .await?,
        0
    );
    f.assert_error(
        part_request(&f, Some(&token), pending.operation_id, "1")?,
        404,
        codes::OPERATION_NOT_FOUND,
    )
    .await?;
    assert_eq!(
        post_export(
            &f,
            &token,
            "oversized-export-01",
            &["erase@example.invalid", "restricted@example.invalid"]
        )
        .await?,
        (StatusCode::OK, failed)
    );
    let (_, pending) = post_export(
        &f,
        &token,
        "not-found-export-01",
        &["absent@example.invalid"],
    )
    .await?;
    let complete = f.run_until(pending.operation_id).await?;
    assert_eq!(complete.status, OperationStatus::NotFound);
    assert!(complete.parts.is_empty());
    assert!(complete.categories.is_empty());
    assert!(complete.restrictions.is_empty());
    let payload = ReceiptVerifier::from_discovery(&f.store.discovery())?
        .verify(complete.receipt.as_ref().ok_or("receipt")?)?;
    assert_eq!(payload.status, OperationStatus::NotFound);
    assert!(payload.parts.is_empty());
    f.assert_error(
        part_request(&f, Some(&token), pending.operation_id, "1")?,
        404,
        codes::OPERATION_NOT_FOUND,
    )
    .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn expired_parts_purge_in_bounded_batches_without_removing_receipts_or_other_parts()
-> TestResult {
    let f = Fixture::with_exports(|config| {
        config.limits.max_export_part_bytes = 110;
        config.export_retention = TimeDelta::hours(2);
    })
    .await?;
    let token = f.token("privacy", "data-subject.export")?;
    let (_, pending) =
        post_export(&f, &token, "expiring-export-01", &["erase@example.invalid"]).await?;
    let complete = f.run_until(pending.operation_id).await?;
    for number in ["0", "999", "-1", "invalid", "4294967296"] {
        f.assert_error(
            part_request(&f, Some(&token), pending.operation_id, number)?,
            404,
            codes::OPERATION_NOT_FOUND,
        )
        .await?;
    }
    let (earliest, latest): (chrono::DateTime<Utc>, chrono::DateTime<Utc>) = sqlx::query_as("SELECT min(expires_at), max(expires_at) FROM data_subject_export_parts WHERE operation_id = $1")
        .bind(pending.operation_id).fetch_one(&f.pool).await?;
    assert_eq!(
        earliest,
        complete.completed_at.ok_or("completion")? + TimeDelta::hours(2)
    );
    assert_eq!(earliest, latest);
    assert_eq!(
        f.store
            .export_part("privacy", pending.operation_id, 1, earliest)
            .await,
        Err(DataSubjectError::ExportExpired)
    );
    sqlx::query("UPDATE data_subject_operations SET export_expires_at = now() - interval '1 second' WHERE id = $1").bind(pending.operation_id).execute(&f.pool).await?;
    sqlx::query("UPDATE data_subject_export_parts SET expires_at = now() - interval '1 second' WHERE operation_id = $1").bind(pending.operation_id).execute(&f.pool).await?;
    let (_, retained) =
        post_export(&f, &token, "retained-export-01", &["other@example.invalid"]).await?;
    let retained = f.run_until(retained.operation_id).await?;
    for remaining in (0..complete.parts.len()).rev() {
        f.assert_error(
            part_request(&f, Some(&token), pending.operation_id, "1")?,
            410,
            codes::EXPORT_EXPIRED,
        )
        .await?;
        assert_eq!(f.store.purge_expired(Utc::now(), 1).await?, 0);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM data_subject_export_parts WHERE operation_id = $1"
            )
            .bind(pending.operation_id)
            .fetch_one(&f.pool)
            .await?,
            remaining as i64
        );
    }
    f.assert_error(
        part_request(&f, Some(&token), pending.operation_id, "1")?,
        410,
        codes::EXPORT_EXPIRED,
    )
    .await?;
    f.assert_error(
        part_request(&f, Some(&token), pending.operation_id, "999")?,
        404,
        codes::OPERATION_NOT_FOUND,
    )
    .await?;
    let (_, _, status) = f
        .send(as_export(
            f.get_request(Some(&token), pending.operation_id)?,
        ))
        .await?;
    assert_eq!(
        serde_json::from_value::<OperationResource>(status)?,
        complete
    );
    assert_eq!(f.operation_count().await?, 2);
    assert!(
        f.store
            .export_part("privacy", retained.operation_id, 1, Utc::now())
            .await?
            .is_some()
    );
    sqlx::query("UPDATE data_subject_operations SET purge_after = now() - interval '1 second' WHERE id = $1").bind(retained.operation_id).execute(&f.pool).await?;
    assert_eq!(f.store.purge_expired(Utc::now(), 1).await?, 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM data_subject_export_parts")
            .fetch_one(&f.pool)
            .await?,
        0
    );
    assert_eq!(f.operation_count().await?, 1);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn export_storage_enforces_bytes_digests_json_and_completed_ownership() -> TestResult {
    let f = Fixture::with_exports(|_| {}).await?;
    let token = f.token("privacy", "data-subject.export")?;
    let (_, pending) = post_export(
        &f,
        &token,
        "storage-export-key-01",
        &["erase@example.invalid"],
    )
    .await?;
    let insert = "INSERT INTO data_subject_export_parts (operation_id, part_number, json_bytes, sha256, bytes, expires_at) VALUES ($1, 99, $2, $3, $4, now() + interval '1 day')";
    // A valid part cannot commit while its operation is pending.
    let error = sqlx::query(insert)
        .bind(pending.operation_id)
        .bind(b"{}".as_slice())
        .bind("44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a")
        .bind(2_i64)
        .execute(&f.pool)
        .await
        .expect_err("pending parent");
    assert_eq!(
        error
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("23503")
    );
    f.run_until(pending.operation_id).await?;
    for (bytes, sha256, length) in [
        (b"{}".as_slice(), "0".repeat(64), 2_i64),
        (
            b"{}".as_slice(),
            "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a".into(),
            3,
        ),
        (
            b"bad".as_slice(),
            aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, b"bad")
                .as_ref()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            3,
        ),
    ] {
        assert!(
            sqlx::query(insert)
                .bind(pending.operation_id)
                .bind(bytes)
                .bind(sha256)
                .bind(length)
                .execute(&f.pool)
                .await
                .is_err()
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn receipt_write_failure_rolls_back_part_bytes_and_every_product_change() -> TestResult {
    let f = Fixture::with_exports(|_| {}).await?;
    let token = f.token("privacy", "data-subject.export")?;
    f.product.mutate_export.store(true, Ordering::SeqCst);
    sqlx::raw_sql("CREATE FUNCTION reject_export_receipt() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.kind = 'export' AND NEW.receipt IS NOT NULL THEN RAISE EXCEPTION 'receipt rejected'; END IF; RETURN NEW; END; $$; CREATE TRIGGER reject_export_receipt BEFORE UPDATE ON data_subject_operations FOR EACH ROW EXECUTE FUNCTION reject_export_receipt();")
        .execute(&f.pool).await?;
    let (_, pending) = post_export(
        &f,
        &token,
        "receipt-rollback-key-01",
        &["erase@example.invalid", "restricted@example.invalid"],
    )
    .await?;
    let failed = f.run_until(pending.operation_id).await?;
    assert_eq!(failed.status, OperationStatus::Failed);
    assert_eq!(failed.error.ok_or("error")?.code, codes::OPERATION_FAILED);
    assert!(failed.receipt.is_none());
    assert!(failed.parts.is_empty());
    assert!(failed.categories.is_empty());
    assert_eq!(f.product.exports.load(Ordering::SeqCst), 4);
    assert_eq!(f.subject_count().await?, 3);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM data_subject_export_parts")
            .fetch_one(&f.pool)
            .await?,
        0
    );
    let payload: Value = sqlx::query_scalar("SELECT payload FROM job_outbox WHERE job_type = $1")
        .bind(EXPORT_JOB_TYPE)
        .fetch_one(&f.pool)
        .await?;
    assert_eq!(payload, json!({"operationId": pending.operation_id}));
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn service_conformance_erasure_only() -> TestResult {
    let mut fixture = Fixture::new(false).await?;
    let router = fixture.router.clone();
    baukit_test::check_data_subject_conformance(
        baukit_test::DataSubjectEndpoint::Router(&router),
        &mut fixture,
    )
    .await?;
    assert_eq!(fixture.product.erases.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.product.exports.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn service_conformance_erasure_and_export() -> TestResult {
    let mut fixture =
        Fixture::with_exports(|config| config.limits.max_export_part_bytes = 100).await?;
    let router = fixture.router.clone();
    baukit_test::check_data_subject_conformance(
        baukit_test::DataSubjectEndpoint::Router(&router),
        &mut fixture,
    )
    .await?;
    assert_eq!(fixture.product.erases.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.product.exports.load(Ordering::SeqCst), 1);
    Ok(())
}
