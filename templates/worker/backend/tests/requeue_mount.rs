use std::{
    error::Error,
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
};

use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode, header},
};
use baukit_config::Secret;
use baukit_ops::jobs_admin::{FailedJobRequeue, REQUEUE_PATH, RequeueFuture};
use tower::ServiceExt as _;
use uuid::Uuid;

use {{ context.app_crate }}_bin::REQUEUE_REQUESTS_PER_PERIOD;
use {{ context.app_crate }}_bin::mount_requeue_route;

const JOB_TYPES: &[&str] = &["test.requeue"];
const CLIENT_PORT: u16 = 40_000;

struct NoRequeue;

impl FailedJobRequeue for NoRequeue {
    fn requeue_failed<'a>(&'a self, _job_type: &'a str, _ids: &'a [Uuid]) -> RequeueFuture<'a> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

fn operations(admin_token: Option<&Secret<String>>) -> Result<Router, Box<dyn Error>> {
    Ok(mount_requeue_route(
        Router::new(),
        Arc::new(NoRequeue),
        JOB_TYPES,
        admin_token,
    )?)
}

fn requeue_request(token: &str) -> Result<Request<Body>, axum::http::Error> {
    let mut request = Request::post(REQUEUE_PATH)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"jobType":"test.requeue","ids":["0190a5c4-0000-7000-8000-000000000000"]}"#,
        ))?;
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from((
            Ipv4Addr::LOCALHOST,
            CLIENT_PORT,
        ))));
    Ok(request)
}

#[tokio::test]
async fn the_requeue_route_is_absent_without_an_admin_token() -> Result<(), Box<dyn Error>> {
    let response = operations(None)?
        .oneshot(requeue_request("anything")?)
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn the_requeue_route_rejects_a_wrong_token() -> Result<(), Box<dyn Error>> {
    let token = Secret::new("expected-admin-token".to_owned());
    let response = operations(Some(&token))?
        .oneshot(requeue_request("wrong-admin-token")?)
        .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
async fn the_requeue_route_accepts_the_configured_token() -> Result<(), Box<dyn Error>> {
    let token = Secret::new("expected-admin-token".to_owned());
    let response = operations(Some(&token))?
        .oneshot(requeue_request("expected-admin-token")?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn the_requeue_route_answers_429_once_the_ip_quota_is_used_up() -> Result<(), Box<dyn Error>>
{
    let token = Secret::new("expected-admin-token".to_owned());
    let limited = operations(Some(&token))?;
    for _ in 0..REQUEUE_REQUESTS_PER_PERIOD {
        let response = limited
            .clone()
            .oneshot(requeue_request("expected-admin-token")?)
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
    }
    let response = limited
        .oneshot(requeue_request("expected-admin-token")?)
        .await?;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let response = operations(None)?
        .oneshot(requeue_request("expected-admin-token")?)
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    Ok(())
}
