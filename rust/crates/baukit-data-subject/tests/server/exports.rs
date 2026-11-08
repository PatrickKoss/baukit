use super::*;

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn exports_are_opt_in_and_enforce_auth_scope_owner_and_kind_on_every_route() -> TestResult {
    assert_exports_disabled_by_default().await?;
    let f = Fixture::with_exports(|_| {}).await?;
    assert_export_discovery_and_auth_errors(&f).await?;
    assert_export_owner_and_kind_isolation(&f).await
}

async fn assert_exports_disabled_by_default() -> TestResult {
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
            &request_body(&["erase@example.invalid"]),
        )?),
        as_export(disabled.get_request(Some(&token), Uuid::now_v7())?),
        part_request(&disabled, Some(&token), Uuid::now_v7(), "1")?,
    ] {
        assert_eq!(
            disabled.router.clone().oneshot(request).await?.status(),
            StatusCode::NOT_FOUND
        );
    }
    Ok(())
}

async fn assert_export_discovery_and_auth_errors(f: &Fixture) -> TestResult {
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
                &request_body(&["erase@example.invalid"]),
            )?),
            as_export(f.get_request(token.as_deref(), Uuid::now_v7())?),
            part_request(f, token.as_deref(), Uuid::now_v7(), "1")?,
        ] {
            f.assert_error(request, status, code).await?;
        }
    }
    Ok(())
}

async fn assert_export_owner_and_kind_isolation(f: &Fixture) -> TestResult {
    let export = f.token("privacy", "data-subject.export")?;
    let erase = f.token("privacy", "data-subject.erase")?;
    f.assert_error(
        f.post_request(
            Some(&export),
            &["export-auth-key-01"],
            &request_body(&["erase@example.invalid"]),
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
        f,
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
            part_request(f, Some(&other), id, "1")?,
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
        part_request(f, Some(&export), erasure.operation_id, "1")?,
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
    assert_export_post_validation(&f, &token).await?;
    let first_operation_id = assert_concurrent_export_replay(&f, &token).await?;
    assert_failed_export_reconciliation(&f, &token).await?;
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

async fn assert_export_post_validation(f: &Fixture, token: &str) -> TestResult {
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
            as_export(f.post_request(Some(token), &keys, &body)?),
            status,
            code,
        )
        .await?;
    }
    Ok(())
}

async fn assert_concurrent_export_replay(f: &Fixture, token: &str) -> Result<Uuid, Box<dyn Error>> {
    let (left, right) = tokio::join!(
        post_export(
            f,
            token,
            "export-replay-key-01",
            &[" ERASE@EXAMPLE.INVALID "]
        ),
        post_export(f, token, "export-replay-key-01", &["erase@example.invalid"]),
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
            as_export(f.post_request(Some(token), &["export-replay-key-01"], &body)?),
            409,
            codes::IDEMPOTENCY_CONFLICT,
        )
        .await?;
    }
    f.assert_error(
        part_request(f, Some(token), pending.operation_id, "1")?,
        409,
        codes::EXPORT_NOT_READY,
    )
    .await?;
    let first_operation_id = pending.operation_id;
    let complete = f.run_until(pending.operation_id).await?;
    assert_eq!(
        post_export(f, token, "export-replay-key-01", &["erase@example.invalid"]).await?,
        (StatusCode::OK, complete)
    );
    assert_eq!(f.product.exports.load(Ordering::SeqCst), 1);
    Ok(first_operation_id)
}

async fn assert_failed_export_reconciliation(f: &Fixture, token: &str) -> TestResult {
    f.product.mutate_export.store(true, Ordering::SeqCst);
    f.product.fail_export.store(true, Ordering::SeqCst);
    let (_, pending) = post_export(
        f,
        token,
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
            f,
            token,
            "export-failure-key-01",
            &["erase@example.invalid", "restricted@example.invalid"]
        )
        .await?,
        (StatusCode::OK, failed)
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn client_downloads_every_deterministic_part_over_http_and_verifies_the_signed_digests()
-> TestResult {
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
    let http = HttpExportClient {
        client: baukit_test::test_http_client().no_proxy().build()?,
        url,
        token,
    };
    let discovery: DiscoveryDocument = http
        .client
        .get(format!("{}/.well-known/baukit/data-subject", http.url))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let mut first_parts = None;
    for key in ["http-export-key-01", "http-export-key-02"] {
        let (complete, payload) = http.complete_export(&f, &discovery, key).await?;
        let downloaded = http.download_parts(&f, &complete, &payload).await?;
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

struct HttpExportClient {
    client: reqwest::Client,
    url: String,
    token: String,
}

impl HttpExportClient {
    async fn complete_export(
        &self,
        f: &Fixture,
        discovery: &DiscoveryDocument,
        key: &str,
    ) -> Result<(OperationResource, ReceiptPayload), Box<dyn Error>> {
        let response = self
            .client
            .post(format!("{}/api/v1/data-subject/exports", self.url))
            .bearer_auth(&self.token)
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
        let complete: OperationResource = self
            .client
            .get(format!(
                "{}/api/v1/data-subject/exports/{}",
                self.url, pending.operation_id
            ))
            .bearer_auth(&self.token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        assert_eq!(complete.status, OperationStatus::Completed);
        assert_eq!(complete.kind, OperationKind::Export);
        let payload = ReceiptVerifier::from_discovery(discovery)?
            .verify(complete.receipt.as_ref().ok_or("receipt")?)?;
        assert_eq!(payload.operation_id, complete.operation_id);
        assert_eq!(payload.kind, complete.kind);
        assert_eq!(payload.status, complete.status);
        assert_eq!(payload.service, discovery.service);
        assert_eq!(payload.subject_reference, complete.subject_reference);
        assert_eq!(payload.categories, complete.categories);
        assert_eq!(payload.parts, complete.parts);
        assert_eq!(payload.created_at, complete.created_at);
        assert_eq!(Some(payload.completed_at), complete.completed_at);
        assert_http_export_categories(&complete);
        Ok((complete, payload))
    }

    async fn download_parts(
        &self,
        f: &Fixture,
        complete: &OperationResource,
        payload: &ReceiptPayload,
    ) -> Result<Vec<Vec<u8>>, Box<dyn Error>> {
        let mut downloaded = Vec::new();
        let mut records = std::collections::BTreeMap::<String, Vec<Value>>::new();
        for (index, part) in payload.parts.iter().enumerate() {
            assert_eq!(
                part.part_number,
                u32::try_from(index + 1).expect("part number fits u32")
            );
            let bytes = self
                .download_part(f, complete.operation_id, part, payload)
                .await?;
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
            downloaded.push(bytes);
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
        Ok(downloaded)
    }

    async fn download_part(
        &self,
        f: &Fixture,
        operation_id: Uuid,
        part: &ExportPart,
        payload: &ReceiptPayload,
    ) -> Result<Vec<u8>, Box<dyn Error>> {
        use aws_lc_rs::digest::{SHA256, digest};
        let response = self
            .client
            .get(format!(
                "{}/api/v1/data-subject/exports/{operation_id}/parts/{}",
                self.url, part.part_number
            ))
            .bearer_auth(&self.token)
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
            .bind(operation_id).bind(i64::from(part.part_number)).fetch_one(&f.pool).await?;
        assert_eq!(stored.as_slice(), bytes.as_ref());
        assert_eq!(hash, sha256);
        assert_eq!(
            u64::try_from(length).expect("stored length is non-negative"),
            part.bytes
        );
        assert_eq!(expires_at, payload.completed_at + TimeDelta::days(7));
        Ok(bytes.to_vec())
    }
}

fn assert_http_export_categories(complete: &OperationResource) {
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
    assert_invalid_parts_and_part_expiry(&f, &token, &complete).await?;
    sqlx::query("UPDATE data_subject_operations SET export_expires_at = now() - interval '1 second' WHERE id = $1").bind(pending.operation_id).execute(&f.pool).await?;
    sqlx::query("UPDATE data_subject_export_parts SET expires_at = now() - interval '1 second' WHERE operation_id = $1").bind(pending.operation_id).execute(&f.pool).await?;
    let (_, retained) =
        post_export(&f, &token, "retained-export-01", &["other@example.invalid"]).await?;
    let retained = f.run_until(retained.operation_id).await?;
    assert_expired_parts_purge_one_per_batch(&f, &token, &complete).await?;
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

async fn assert_invalid_parts_and_part_expiry(
    f: &Fixture,
    token: &str,
    complete: &OperationResource,
) -> TestResult {
    for number in ["0", "999", "-1", "invalid", "4294967296"] {
        f.assert_error(
            part_request(f, Some(token), complete.operation_id, number)?,
            404,
            codes::OPERATION_NOT_FOUND,
        )
        .await?;
    }
    let (earliest, latest): (chrono::DateTime<Utc>, chrono::DateTime<Utc>) = sqlx::query_as("SELECT min(expires_at), max(expires_at) FROM data_subject_export_parts WHERE operation_id = $1")
        .bind(complete.operation_id).fetch_one(&f.pool).await?;
    assert_eq!(
        earliest,
        complete.completed_at.ok_or("completion")? + TimeDelta::hours(2)
    );
    assert_eq!(earliest, latest);
    assert_eq!(
        f.store
            .export_part("privacy", complete.operation_id, 1, earliest)
            .await,
        Err(DataSubjectError::ExportExpired)
    );
    Ok(())
}

async fn assert_expired_parts_purge_one_per_batch(
    f: &Fixture,
    token: &str,
    complete: &OperationResource,
) -> TestResult {
    for remaining in (0..complete.parts.len()).rev() {
        f.assert_error(
            part_request(f, Some(token), complete.operation_id, "1")?,
            410,
            codes::EXPORT_EXPIRED,
        )
        .await?;
        assert_eq!(f.store.purge_expired(Utc::now(), 1).await?, 0);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM data_subject_export_parts WHERE operation_id = $1"
            )
            .bind(complete.operation_id)
            .fetch_one(&f.pool)
            .await?,
            i64::try_from(remaining).expect("remaining count fits i64")
        );
    }
    f.assert_error(
        part_request(f, Some(token), complete.operation_id, "1")?,
        410,
        codes::EXPORT_EXPIRED,
    )
    .await?;
    f.assert_error(
        part_request(f, Some(token), complete.operation_id, "999")?,
        404,
        codes::OPERATION_NOT_FOUND,
    )
    .await?;
    let (_, _, status) = f
        .send(as_export(
            f.get_request(Some(token), complete.operation_id)?,
        ))
        .await?;
    assert_eq!(
        &serde_json::from_value::<OperationResource>(status)?,
        complete
    );
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
