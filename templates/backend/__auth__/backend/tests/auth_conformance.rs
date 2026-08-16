use std::{error::Error, sync::Arc, time::Duration};

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
use baukit_auth::{AuthState, OidcConfig, OidcVerifier};
use baukit_config::HttpConfig;
use serde_json::Value;
use tower::ServiceExt as _;

use {{ context.app_crate }}_api::{ApiState, router};
use {{ context.app_crate }}_bin::{InMemoryItemRepository, InMemoryUserRepository};
use {{ context.app_crate }}_services::{ItemService, UserService};

const AUDIENCE: &str = "{{ context.app_name }}-backend";

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
