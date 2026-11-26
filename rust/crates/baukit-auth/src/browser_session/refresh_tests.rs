use super::*;

fn stored_refresh_tokens(rows: &BTreeMap<Uuid, SessionRow>) -> Result<Vec<String>, TestError> {
    let cipher = cipher()?;
    rows.values()
        .filter(|row| row.rotated_to.is_none() && row.revoked_at.is_none())
        .map(|row| {
            crypto::open(&cipher, row, REFRESH_TOKEN_FIELD)
                .map(|token| token.to_string())
                .map_err(Into::into)
        })
        .collect()
}

#[tokio::test]
async fn dropped_refresh_request_still_stores_the_rotated_token() -> TestResult {
    let idp = FakeIdp {
        refresh_delay: Some(Duration::from_millis(50)),
        ..FakeIdp::default()
    };
    let harness = harness(idp)?;
    let first = signed_in(&harness).await?;
    let dropped =
        tokio::time::timeout(Duration::from_millis(10), refreshed(&harness, &first)).await;
    assert!(dropped.is_err(), "the request should still be in flight");
    while harness.idp.refresh_count() == 0 {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    let rows = harness.store.snapshot().await;
    assert_eq!(stored_refresh_tokens(&rows)?, ["refresh-1"]);
    let replay = refreshed(&harness, &first).await?;
    assert_eq!(replay.status, StatusCode::OK);
    assert_eq!(replay.body["accessToken"], "access-1");
    Ok(())
}

#[tokio::test]
async fn failed_commit_after_rotation_retries_with_the_new_token() -> TestResult {
    let harness = harness(FakeIdp::default())?;
    let first = signed_in(&harness).await?;
    harness.store.failing_commits.store(1, Ordering::SeqCst);
    let refreshed = refreshed(&harness, &first).await?;
    assert_eq!(refreshed.status, StatusCode::OK);
    assert_eq!(refreshed.body["accessToken"], "access-1");
    let rows = harness.store.snapshot().await;
    assert_eq!(stored_refresh_tokens(&rows)?, ["refresh-1"]);
    assert_eq!(harness.idp.refresh_count(), 1);
    Ok(())
}

#[tokio::test]
async fn refresh_response_carries_relative_expiry() -> TestResult {
    let harness = harness(FakeIdp::default())?;
    let first = signed_in(&harness).await?;
    let refreshed = refreshed(&harness, &first).await?;
    assert_eq!(refreshed.body["expiresIn"], ACCESS_LIFETIME.as_secs());
    harness.clock.advance(SignedDuration::from_secs(4));
    let replay = super::refreshed(&harness, &first).await?;
    assert_eq!(replay.body["expiresIn"], ACCESS_LIFETIME.as_secs() - 4);
    Ok(())
}

#[tokio::test]
async fn exchange_ends_the_previous_cookie_family() -> TestResult {
    let harness = harness(FakeIdp::default())?;
    let first = signed_in(&harness).await?;
    let response = send(
        &harness.app,
        request("exchange", Some(&first), Some(&exchange_body()))?,
    )
    .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let rows = harness.store.snapshot().await;
    assert_eq!(
        rows.values().filter(|row| row.revoked_at.is_none()).count(),
        1
    );
    assert_eq!(
        refreshed(&harness, &first).await?.status,
        StatusCode::UNAUTHORIZED
    );
    Ok(())
}

#[tokio::test]
async fn logout_with_a_stale_rotated_cookie_keeps_the_live_family() -> TestResult {
    let harness = harness(FakeIdp::default())?;
    let first = signed_in(&harness).await?;
    let second = rotated_cookie(&harness, &first).await?;
    harness.clock.advance(SignedDuration::from_secs(11));
    let response = send(&harness.app, request("logout", Some(&first), None)?).await?;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(harness.store.snapshot().await.len(), 2);
    assert!(harness.idp.revoked().is_empty());
    assert_eq!(refreshed(&harness, &second).await?.status, StatusCode::OK);

    Ok(())
}

#[tokio::test]
async fn logout_inside_the_grace_window_ends_the_family() -> TestResult {
    let harness = harness(FakeIdp::default())?;
    let first = signed_in(&harness).await?;
    rotated_cookie(&harness, &first).await?;
    let response = send(&harness.app, request("logout", Some(&first), None)?).await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(harness.store.snapshot().await.is_empty());
    Ok(())
}

#[tokio::test]
async fn lost_refresh_response_is_recovered_by_the_previous_cookie() -> TestResult {
    let harness = harness(FakeIdp::default())?;
    let first = signed_in(&harness).await?;
    rotated_cookie(&harness, &first).await?;
    harness.clock.advance(SignedDuration::from_secs(30));
    let retried = refreshed(&harness, &first).await?;
    assert_eq!(retried.status, StatusCode::OK);
    assert_eq!(retried.body["accessToken"], "access-2");
    let third = cookie_pair(&retried.set_cookie.ok_or("no recovered cookie")?)?;
    assert!(harness.idp.revoked().is_empty());
    assert_eq!(refreshed(&harness, &third).await?.status, StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn previous_cookie_after_the_head_was_used_still_revokes_the_family() -> TestResult {
    let harness = harness(FakeIdp::default())?;
    let first = signed_in(&harness).await?;
    let second = rotated_cookie(&harness, &first).await?;
    rotated_cookie(&harness, &second).await?;
    harness.clock.advance(SignedDuration::from_secs(30));
    assert_eq!(
        refreshed(&harness, &first).await?.status,
        StatusCode::UNAUTHORIZED
    );
    assert!(
        harness
            .store
            .snapshot()
            .await
            .values()
            .all(|row| row.revoked_at.is_some())
    );
    Ok(())
}
