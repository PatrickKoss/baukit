use super::*;

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
    assert_finalization_waits_for_deadline(&f, operation.operation_id).await?;
    expire_identity_deletion_deadline(&f, operation.operation_id).await?;
    assert_failed_at_deadline(&f, operation.operation_id, &session).await
}

async fn assert_finalization_waits_for_deadline(f: &Fixture, operation_id: Uuid) -> TestResult {
    let started_at: chrono::DateTime<Utc> = sqlx::query_scalar(
        "SELECT identity_deletion_started_at FROM data_subject_operations WHERE id = $1",
    )
    .bind(operation_id)
    .fetch_one(&f.pool)
    .await?;
    let job_id: Uuid = sqlx::query_scalar("UPDATE job_outbox SET run_after = now(), payload = jsonb_set(payload, '{checkCount}', '6') WHERE job_type = $1 AND idempotency_key = $2 RETURNING id")
        .bind(FINALIZATION_JOB_TYPE).bind(operation_id.to_string()).fetch_one(&f.pool).await?;
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
    .bind(operation_id.to_string())
    .fetch_one(&f.pool)
    .await?;
    assert_eq!(run_after, started_at + TimeDelta::seconds(5));
    assert_eq!(
        f.store
            .status("privacy", OperationKind::Erasure, operation_id)
            .await?
            .ok_or("pending operation")?
            .status,
        OperationStatus::Pending
    );
    Ok(())
}

async fn expire_identity_deletion_deadline(f: &Fixture, operation_id: Uuid) -> TestResult {
    sqlx::query("UPDATE data_subject_operations SET identity_deletion_started_at = clock_timestamp() - interval '6 seconds' WHERE id = $1")
        .bind(operation_id).execute(&f.pool).await?;
    sqlx::query(
        "UPDATE job_outbox SET run_after = now() WHERE job_type = $1 AND idempotency_key = $2",
    )
    .bind(FINALIZATION_JOB_TYPE)
    .bind(operation_id.to_string())
    .execute(&f.pool)
    .await?;
    Ok(())
}

async fn assert_failed_at_deadline(f: &Fixture, operation_id: Uuid, session: &str) -> TestResult {
    let failed = f.run_until(operation_id).await?;
    assert_eq!(failed.status, OperationStatus::Failed);
    assert!(failed.receipt.is_none());
    assert!(failed.completed_at.is_some());
    assert_eq!(
        failed.error.as_ref().ok_or("error")?.code,
        codes::OPERATION_FAILED
    );
    assert_eq!(f.subject_count().await?, 2);
    assert_eq!(f.session_status(session).await?, StatusCode::UNAUTHORIZED);
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
            .status("privacy", OperationKind::Erasure, operation_id)
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
