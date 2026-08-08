use std::{error::Error, sync::Arc};

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use baukit_config::HttpConfig;
use baukit_ops::TrafficGate;
use baukit_runtime::{DeploymentEnvironment, ProcessKind, ServiceInfo, build_info};
use baukit_telemetry::TelemetryBuilder;
use {{ context.app_crate }}_api::{ApiState, router};
use {{ context.app_crate }}_bin::{InMemoryItemRepository, operations_router};
use {{ context.app_crate }}_services::ItemService;
use tower::ServiceExt as _;

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
    let items = ItemService::new(Arc::new(InMemoryItemRepository::new()));
    let api = router(
        ApiState {
            items: items.clone(),
        },
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
