use std::{error::Error, sync::Arc, time::Duration};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use baukit_auth::{AuthState, OidcConfig, OidcVerifier};
use baukit_config::HttpConfig;
use serde_json::Value;
use tower::ServiceExt as _;

use {{ context.app_crate }}_api::{ApiState, router};
use {{ context.app_crate }}_bin::InMemoryItemRepository;
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
            items: ItemService::new(repository.clone()),
            users: UserService::new(repository),
            auth: AuthState::new(verifier),
        },
        &HttpConfig::default(),
    )?;

    baukit_test::check_auth_router_conformance(&app, "/me", &issuer, AUDIENCE).await?;

    let claims = issuer.claims("seeded-test-subject", AUDIENCE, Duration::from_secs(300))?;
    let token = issuer.mint(&claims)?;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/me")
                .header(
                    header::AUTHORIZATION,
                    baukit_test::authorization_header(&token)?,
                )
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?;
    assert_eq!(body["subject"], "seeded-test-subject");
    assert!(body["id"].as_str().is_some());

    Ok(())
}
