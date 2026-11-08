use super::*;

type PolicyDenialCase = (
    baukit_mcp::PolicyDenial,
    StatusCode,
    &'static str,
    Option<&'static str>,
);

fn policy_denial_cases() -> Vec<PolicyDenialCase> {
    use baukit_mcp::PolicyDenial;
    vec![
        (
            PolicyDenial::Inactive,
            StatusCode::UNAUTHORIZED,
            "invalid_token",
            Some("account:read identity:read"),
        ),
        (
            PolicyDenial::InsufficientScope(vec!["account:write".into()]),
            StatusCode::FORBIDDEN,
            "insufficient_scope",
            Some("account:write"),
        ),
        (
            PolicyDenial::Unavailable,
            StatusCode::SERVICE_UNAVAILABLE,
            "authentication_policy_unavailable",
            None,
        ),
        (
            PolicyDenial::RateLimited(Duration::from_secs(7)),
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            None,
        ),
        (
            PolicyDenial::InsufficientScope(vec!["unsafe\"".into()]),
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid_policy_scopes",
            None,
        ),
        (
            PolicyDenial::InsufficientScope(vec![]),
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid_policy_scopes",
            None,
        ),
    ]
}

fn assert_policy_denial_headers(
    headers: &http::HeaderMap,
    expected: StatusCode,
    code: &str,
    challenge_scope: Option<&str>,
) {
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    if let Some(scope) = challenge_scope {
        assert_eq!(
            headers[header::WWW_AUTHENTICATE],
            format!(
                "Bearer resource_metadata=\"https://mcp.example/.well-known/oauth-protected-resource\", error=\"{code}\", scope=\"{scope}\""
            )
        );
    } else {
        assert!(!headers.contains_key(header::WWW_AUTHENTICATE));
    }
    if expected == StatusCode::TOO_MANY_REQUESTS {
        assert_eq!(headers[header::RETRY_AFTER], "7");
    }
}

fn assert_policy_decision_metrics(snapshotter: &metrics_util::debugging::Snapshotter) {
    use metrics_util::debugging::DebugValue;
    let measurements = snapshotter.snapshot().into_vec();
    let mut outcomes = std::collections::BTreeMap::new();
    for (key, _, _, value) in &measurements {
        if key.key().name() == "mcp_authentication_policy_decisions_total" {
            let labels = key.key().labels().collect::<Vec<_>>();
            assert_eq!(labels.len(), 1);
            assert_eq!(labels[0].key(), "outcome");
            let DebugValue::Counter(count) = value else {
                panic!("expected counter")
            };
            outcomes.insert(labels[0].value(), *count);
        }
        if key.key().name() == "mcp_authentication_policy_duration_seconds" {
            assert_eq!(key.key().labels().count(), 0);
            assert!(matches!(value, DebugValue::Histogram(values) if values.len() == 13));
        }
    }
    assert_eq!(
        outcomes,
        std::collections::BTreeMap::from([
            ("allowed", 1),
            ("inactive", 2),
            ("insufficient_scope", 6),
            ("limited", 2),
            ("unavailable", 2),
        ])
    );
    assert!(
        measurements
            .iter()
            .any(|(key, _, _, _)| key.key().name() == "mcp_authentication_policy_duration_seconds")
    );
}

#[tokio::test]
async fn policy_denials_have_safe_http_shapes_before_dispatch() -> Result<(), Box<dyn Error>> {
    use metrics_util::debugging::DebuggingRecorder;
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _recorder_guard = metrics::set_default_local_recorder(&recorder);
    let issuer = MockOidcServer::start().await?;
    let config = config(&issuer);
    let token = issuer.mint(
        &issuer
            .claims("alice", &config.resource_url, Duration::from_secs(300))?
            .claim("scope", "identity:read account:read"),
    )?;
    for (denial, expected, code, challenge_scope) in policy_denial_cases() {
        let policy = Arc::new(ProductPolicy {
            denial: Some(denial),
            scopes: vec![],
            calls: Default::default(),
        });
        let app = router(
            config.clone(),
            verifier(&config).await?,
            McpServices::new(Arc::new(IdentityTool)),
            Arc::new(InMemoryRateLimitStore::default()),
            policy.clone(),
        )
        .await?;
        for request in [list(), call()] {
            let (status, headers, body) = send(&app, Some(&token), request).await?;
            assert_eq!(status, expected);
            assert_eq!(body, json!({"error": code}));
            assert_policy_denial_headers(&headers, expected, code, challenge_scope);
        }
        assert_eq!(policy.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        let (status, _, _) = send(&app, Some("invalid-jwt"), list()).await?;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(policy.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }
    let app = app(config).await?;
    assert_eq!(send(&app, Some(&token), list()).await?.0, StatusCode::OK);
    assert_policy_decision_metrics(&snapshotter);
    Ok(())
}

#[tokio::test]
async fn effective_scopes_filter_discovery_and_dispatch_and_map_account()
-> Result<(), Box<dyn Error>> {
    let issuer = MockOidcServer::start().await?;
    let config = config(&issuer);
    let token = issuer.mint(
        &issuer
            .claims("alice", &config.resource_url, Duration::from_secs(300))?
            .claim("scope", "identity:read account:read")
            .claim("azp", "registered-client"),
    )?;
    for (scopes, allowed) in [
        (vec!["identity:read"], false),
        (
            vec!["identity:read", "account:reader", "injected:write"],
            false,
        ),
        (
            vec!["identity:read", "account:read", "injected:write"],
            true,
        ),
    ] {
        let policy = Arc::new(ProductPolicy {
            denial: None,
            scopes: scopes.into_iter().map(str::to_owned).collect(),
            calls: Default::default(),
        });
        let app = router(
            config.clone(),
            verifier(&config).await?,
            McpServices::new(Arc::new(IdentityTool)),
            Arc::new(InMemoryRateLimitStore::default()),
            policy.clone(),
        )
        .await?;
        let (status, _, body) = send(&app, Some(&token), list()).await?;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["result"]["tools"].as_array().ok_or("tools")?.len(),
            usize::from(allowed)
        );
        let (status, _, body) = send(&app, Some(&token), call()).await?;
        if allowed {
            assert_eq!(status, StatusCode::OK);
            assert_eq!(
                body["result"]["structuredContent"],
                json!({"subject":"linked-account", "issuer":issuer.issuer(), "client":"registered-client", "scopes":["account:read","identity:read"]})
            );
        } else {
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert_eq!(body["error"], "insufficient_scope");
        }
        assert_eq!(policy.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }
    Ok(())
}
