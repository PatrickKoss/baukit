use std::{error::Error, sync::Arc};

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
{% if context.auth_oidc %}use baukit_auth::{AuthState, OidcConfig, OidcVerifier};
{% endif %}use baukit_config::HttpConfig;
use baukit_ops::TrafficGate;
use baukit_runtime::{DeploymentEnvironment, ProcessKind, ServiceInfo, build_info};
use baukit_telemetry::TelemetryBuilder;
use tower::ServiceExt as _;

use {{ context.app_crate }}_api::{ApiState, router};
use {{ context.app_crate }}_bin::{InMemoryItemRepository, operations_router};
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
{% if context.auth_oidc %}    let users = UserService::new(repository);
    let issuer = baukit_test::MockOidcServer::start().await?;
    let verifier =
        OidcVerifier::discover(OidcConfig::new(issuer.issuer(), "{{ context.app_name }}-backend")?).await?;
{% endif %}    let api = router(
        ApiState {
            items: items.clone(),
{% if context.auth_oidc %}            users,
            auth: AuthState::new(verifier),
{% endif %}        },
        &HttpConfig::default(),
    )?;
    let response = api
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/items")
                .body(Body::empty())?,
        )
        .await?;
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
