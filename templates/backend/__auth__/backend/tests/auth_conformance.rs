use std::{error::Error, path::PathBuf, process::Command, sync::Arc, time::Duration};

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
    middleware,
};
use baukit_auth::{AuthState, OidcConfig, OidcVerifier};
use baukit_config::HttpConfig;
use baukit_ratelimit::{InMemoryRateLimitStore, Quota, RateLimitOptions};
use serde_json::Value;
use tower::ServiceExt as _;

use {{ context.app_crate }}_api::{ApiState, router};
use {{ context.app_crate }}_bin::{InMemoryItemRepository, InMemoryUserRepository};
use {{ context.app_crate }}_services::{ItemService, UserService};

const AUDIENCE: &str = "{{ context.app_name }}-backend";

#[test]
fn generated_keycloak_tools_pass_their_offline_checks() -> Result<(), Box<dyn Error>> {
    let product_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    for arguments in [
        vec!["-m", "unittest", "discover", "-s", "scripts/tests"],
        vec![
            "scripts/keycloak_policy.py",
            "--environment-class",
            "development",
        ],
        vec!["scripts/reconcile_keycloak.py", "--check"],
    ] {
        let output = Command::new("python3")
            .args(arguments)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .current_dir(&product_root)
            .output()?;
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

#[tokio::test]
async fn protected_route_conforms_and_maps_subject_to_internal_user() -> Result<(), Box<dyn Error>>
{
    let issuer = baukit_test::MockOidcServer::start().await?;
    let verifier = OidcVerifier::discover(
        OidcConfig::new(issuer.issuer(), AUDIENCE)?.with_clock_skew(Duration::ZERO),
    )
    .await?;
    let repository = Arc::new(InMemoryItemRepository::new());
    let app = router(
        ApiState {
            items: ItemService::new(repository),
            users: UserService::new(Arc::new(InMemoryUserRepository::new())),
            auth: AuthState::new(verifier),
        },
        &HttpConfig::default(),
    )?;

    baukit_test::check_auth_router_conformance(&app, "/me", &issuer, AUDIENCE).await?;

    for (method, path) in [
        (Method::GET, "/items"),
        (Method::POST, "/items"),
        (Method::GET, "/items/00000000-0000-0000-0000-000000000000"),
        (Method::PUT, "/items/00000000-0000-0000-0000-000000000000"),
        (
            Method::DELETE,
            "/items/00000000-0000-0000-0000-000000000000",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"name":"protected"}"#))?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
    }

    let session = issuer.issue_session("seeded-test-subject", AUDIENCE, Duration::from_secs(1))?;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/me")
                .header(
                    header::AUTHORIZATION,
                    baukit_test::authorization_header(session.access_token())?,
                )
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?;
    assert_eq!(body["subject"], "seeded-test-subject");
    assert!(body["id"].as_str().is_some());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/items")
                .header(
                    header::AUTHORIZATION,
                    baukit_test::authorization_header(session.access_token())?,
                )
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"name":"protected"}"#))?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::CREATED);

    tokio::time::sleep(Duration::from_secs(2)).await;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/me")
                .header(
                    header::AUTHORIZATION,
                    baukit_test::authorization_header(session.access_token())?,
                )
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response.headers()[header::WWW_AUTHENTICATE],
        "Bearer error=\"invalid_token\", hint=\"expired\""
    );

    issuer.set_refresh_delay(Duration::from_millis(20));
    let refreshed = issuer.refresh_session(session.refresh_token()).await?;
    let (concurrent_a, concurrent_b) = tokio::join!(
        issuer.refresh_session(session.refresh_token()),
        issuer.refresh_session(session.refresh_token()),
    );
    concurrent_a?;
    concurrent_b?;
    assert_eq!(issuer.refresh_request_count(), 3);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/me")
                .header(
                    header::AUTHORIZATION,
                    baukit_test::authorization_header(refreshed.access_token())?,
                )
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    assert!(issuer.reject_refresh(session.refresh_token()));
    assert!(matches!(
        issuer.refresh_session(session.refresh_token()).await,
        Err(baukit_test::JwtFixtureError::RefreshRejected { ref code })
            if code == "invalid_grant"
    ));
    let revoked =
        issuer.issue_session("revoked-test-subject", AUDIENCE, Duration::from_secs(60))?;
    assert!(issuer.revoke_session(revoked.refresh_token()));
    assert!(matches!(
        issuer.refresh_session(revoked.refresh_token()).await,
        Err(baukit_test::JwtFixtureError::RefreshRejected { ref code })
            if code == "invalid_grant"
    ));

    Ok(())
}

#[tokio::test]
async fn authentication_runs_before_identity_rate_limiting() -> Result<(), Box<dyn Error>> {
    let issuer = baukit_test::MockOidcServer::start().await?;
    let verifier = OidcVerifier::discover(
        OidcConfig::new(issuer.issuer(), AUDIENCE)?.with_clock_skew(Duration::ZERO),
    )
    .await?;
    let auth = AuthState::new(verifier);
    let app = router(
        ApiState {
            items: ItemService::new(Arc::new(InMemoryItemRepository::new())),
            users: UserService::new(Arc::new(InMemoryUserRepository::new())),
            auth: auth.clone(),
        },
        &HttpConfig::default(),
    )?;
    let mut options = RateLimitOptions::default();
    options.identity.quota = Quota::new(1, Duration::from_secs(60), 0)?;
    options.ip.enabled = false;
    let app = baukit_ratelimit::layers(app, InMemoryRateLimitStore::default(), options).layer(
        middleware::from_fn_with_state(auth, baukit_auth::establish_principal),
    );
    let alice = issuer.mint(&issuer.claims("alice", AUDIENCE, Duration::from_secs(60))?)?;
    let bob = issuer.mint(&issuer.claims("bob", AUDIENCE, Duration::from_secs(60))?)?;

    for (token, expected) in [
        (&alice, StatusCode::OK),
        (&alice, StatusCode::TOO_MANY_REQUESTS),
        (&bob, StatusCode::OK),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/me")
                    .header(
                        header::AUTHORIZATION,
                        baukit_test::authorization_header(token)?,
                    )
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), expected);
    }

    Ok(())
}
