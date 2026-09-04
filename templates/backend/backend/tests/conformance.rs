use std::{error::Error, sync::Arc{% if context.auth_oidc %}, time::Duration{% endif %}};

use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
};
{% if context.auth_oidc %}use baukit_auth::{AuthState, OidcConfig, OidcVerifier};
{% endif %}use baukit_config::HttpConfig;
use baukit_ops::TrafficGate;
use baukit_runtime::{DeploymentEnvironment, ProcessKind, ServiceInfo, build_info};
use baukit_telemetry::TelemetryBuilder;
use serde_json::Value;
use tower::ServiceExt as _;

use {{ context.app_crate }}_api::{ApiState, router};
use {{ context.app_crate }}_bin::{InMemoryItemRepository{% if context.auth_oidc %}, InMemoryUserRepository{% endif %}, operations_router};
{% if context.auth_oidc %}use {{ context.app_crate }}_services::{ItemService, UserService};
{% else %}use {{ context.app_crate }}_services::ItemService;
{% endif %}
#[tokio::test]
async fn health_and_metrics_conform_to_baukit() -> Result<(), Box<dyn Error>> {
    let service_info = ServiceInfo::new(
        "{{ context.app_name }}",
        ProcessKind::Api,
        build_info!(),
        DeploymentEnvironment::Local,
    );
    let telemetry = TelemetryBuilder::new(service_info.telemetry_identity().clone())
        .filter("off")
        .init()?;
    let repository = Arc::new(InMemoryItemRepository::new());
    let items = ItemService::new(repository.clone());
{% if context.auth_oidc %}    const AUDIENCE: &str = "{{ context.app_name }}-backend";
    let users = UserService::new(Arc::new(InMemoryUserRepository::new()));
    let issuer = baukit_test::MockOidcServer::start().await?;
    let verifier = OidcVerifier::discover(OidcConfig::new(issuer.issuer(), AUDIENCE)?).await?;
{% endif %}    let api = router(
        ApiState {
            items: items.clone(),
{% if context.auth_oidc %}            users,
            auth: AuthState::new(verifier),
{% endif %}        },
        &HttpConfig::default(),
    )?;
    let request = Request::builder().method(Method::GET).uri("/items");
{% if context.auth_oidc %}    let claims = issuer.claims("conformance-user", AUDIENCE, Duration::from_secs(60))?;
    let token = issuer.mint(&claims)?;
    let request = request.header(
        axum::http::header::AUTHORIZATION,
        baukit_test::authorization_header(&token)?,
    );
{% endif %}    let response = api.oneshot(request.body(Body::empty())?).await?;
    assert_eq!(response.status(), StatusCode::OK);

    let (ops, readiness) = operations_router(
        items,
        service_info.telemetry_identity().clone(),
        telemetry.prometheus_handle().clone(),
        TrafficGate::new(),
    )?;
    baukit_test::assert_ops_router_conformance(&ops, &readiness).await;
    baukit_test::assert_metrics_conformance(telemetry.prometheus_handle().render(), true);
    telemetry.shutdown()?;
    Ok(())
}

#[tokio::test]
async fn json_rejections_keep_their_protocol_statuses() -> Result<(), Box<dyn Error>> {
    let repository = Arc::new(InMemoryItemRepository::new());
{% if context.auth_oidc %}    const AUDIENCE: &str = "{{ context.app_name }}-backend";
    let users = UserService::new(Arc::new(InMemoryUserRepository::new()));
    let issuer = baukit_test::MockOidcServer::start().await?;
    let verifier = OidcVerifier::discover(OidcConfig::new(issuer.issuer(), AUDIENCE)?).await?;
    let claims = issuer.claims("json-test-user", AUDIENCE, Duration::from_secs(60))?;
    let token = issuer.mint(&claims)?;
{% endif %}    let config = HttpConfig {
        body_size_limit: 64,
        ..HttpConfig::default()
    };
    let app = router(
        ApiState {
            items: ItemService::new(repository),
{% if context.auth_oidc %}            users,
            auth: AuthState::new(verifier),
{% endif %}        },
        &config,
    )?;

    let oversized_name = "x".repeat(64);
    let oversized_body = [r#"{"name":""#, &oversized_name, r#""}"#].concat();
    for (content_type, body, status, code) in [
        (
            Some("application/json"),
            "{".to_owned(),
            StatusCode::BAD_REQUEST,
            "invalid_json",
        ),
        (
            None,
            r#"{"name":"item"}"#.to_owned(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
        ),
        (
            Some("application/json"),
            r#"{"name":1}"#.to_owned(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_failed",
        ),
        (
            Some("application/json"),
            oversized_body,
            StatusCode::PAYLOAD_TOO_LARGE,
            "payload_too_large",
        ),
    ] {
        let request = Request::builder().method(Method::POST).uri("/items");
{% if context.auth_oidc %}        let request = request.header(
            header::AUTHORIZATION,
            baukit_test::authorization_header(&token)?,
        );
{% endif %}        let request = if let Some(content_type) = content_type {
            request.header(header::CONTENT_TYPE, content_type)
        } else {
            request
        };
        let response = app.clone().oneshot(request.body(Body::from(body))?).await?;
        assert_eq!(response.status(), status, "{code}");
        let response_body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?;
        assert_eq!(response_body["error"]["code"], code);
    }

    Ok(())
}
