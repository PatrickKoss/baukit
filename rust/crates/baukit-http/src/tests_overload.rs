use std::time::Duration;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::RETRY_AFTER},
    response::{IntoResponse, Response},
    routing::get,
};
use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use serde_json::Value;
use tokio::{task::JoinSet, time::Instant};
use tower::ServiceExt as _;

use super::*;

const SLOW_HANDLER: Duration = Duration::from_secs(5);
const QUEUE_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_CONCURRENCY: usize = 2;
const REQUESTS: usize = 5;

async fn slow_handler() -> &'static str {
    tokio::time::sleep(SLOW_HANDLER).await;
    "done"
}

fn get_request(path: &str) -> Request<Body> {
    Request::builder()
        .uri(path)
        .body(Body::empty())
        .expect("request")
}

async fn body_json(response: Response) -> Value {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    serde_json::from_slice(&bytes).expect("JSON body")
}

fn counter_value(snapshotter: &Snapshotter, name: &str, label: (&str, &str)) -> Option<u64> {
    snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .find(|(key, _, _, _)| {
            key.key().name() == name
                && key
                    .key()
                    .labels()
                    .any(|candidate| candidate.key() == label.0 && candidate.value() == label.1)
        })
        .and_then(|(_, _, _, value)| match value {
            DebugValue::Counter(count) => Some(count),
            _ => None,
        })
}

fn gauge_sum(snapshotter: &Snapshotter, name: &str) -> f64 {
    snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .filter(|(key, _, _, _)| key.key().name() == name)
        .map(|(_, _, _, value)| match value {
            DebugValue::Gauge(gauge) => gauge.into_inner(),
            _ => 0.0,
        })
        .sum()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn excess_requests_are_shed_with_503_and_retry_after_within_the_queue_timeout() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _recorder_guard = metrics::set_default_local_recorder(&recorder);
    let mut options = HttpOptions::default().with_queue_timeout(QUEUE_TIMEOUT);
    options.concurrency_limit = MAX_CONCURRENCY;
    let app = layers(Router::new().route("/slow", get(slow_handler)), options);

    let started = Instant::now();
    let mut requests = JoinSet::new();
    for _ in 0..REQUESTS {
        let app = app.clone();
        requests.spawn(async move {
            let response = app.oneshot(get_request("/slow")).await.expect("response");
            (response, started.elapsed())
        });
    }

    tokio::time::sleep(QUEUE_TIMEOUT / 2).await;
    assert_eq!(gauge_sum(&snapshotter, HTTP_REQUESTS_IN_FLIGHT), 2.0);

    let mut shed = 0;
    let mut succeeded = 0;
    while let Some(joined) = requests.join_next().await {
        let (response, elapsed) = joined.expect("request task joins");
        match response.status() {
            StatusCode::SERVICE_UNAVAILABLE => {
                assert_eq!(elapsed, QUEUE_TIMEOUT);
                assert_eq!(response.headers()[RETRY_AFTER], "1");
                assert!(response.headers().contains_key(X_REQUEST_ID));
                let body = body_json(response).await;
                assert_eq!(body["error"]["code"], "unavailable");
                assert!(body["error"]["requestId"].is_string());
                shed += 1;
            }
            StatusCode::OK => {
                assert_eq!(elapsed, SLOW_HANDLER);
                succeeded += 1;
            }
            other => panic!("unexpected status {other}"),
        }
    }

    assert_eq!((shed, succeeded), (3, 2));
    assert_eq!(
        counter_value(
            &snapshotter,
            HTTP_REQUESTS_SHED_TOTAL,
            ("reason", "concurrency")
        ),
        Some(3)
    );
    assert_eq!(gauge_sum(&snapshotter, HTTP_REQUESTS_IN_FLIGHT), 0.0);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_route_timeout_override_ends_the_request_before_the_global_timeout() {
    const ROUTE_TIMEOUT: Duration = Duration::from_secs(1);
    let app = finalize(
        Router::new()
            .route(
                "/bounded",
                get(slow_handler).layer(route_timeout(ROUTE_TIMEOUT)),
            )
            .route("/unbounded", get(slow_handler)),
        HttpOptions::default(),
    );

    let started = Instant::now();
    let response = app
        .clone()
        .oneshot(get_request("/bounded"))
        .await
        .expect("response");
    assert_eq!(started.elapsed(), ROUTE_TIMEOUT);
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    let body = body_json(response).await;
    assert_eq!(body["error"]["code"], "request_timeout");

    let started = Instant::now();
    let response = app
        .oneshot(get_request("/unbounded"))
        .await
        .expect("response");
    assert_eq!(started.elapsed(), SLOW_HANDLER);
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test(flavor = "current_thread")]
async fn unavailable_rounds_retry_after_up_to_whole_seconds_of_at_least_one() {
    let cases = [
        (Duration::ZERO, "1"),
        (Duration::from_millis(200), "1"),
        (Duration::from_secs(2), "2"),
        (Duration::from_millis(2_001), "3"),
    ];
    for (retry_after, expected) in cases {
        let response = ApiError::unavailable(retry_after).into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers()[RETRY_AFTER], expected);
        let body = body_json(response).await;
        assert_eq!(body["error"]["code"], "unavailable");
        assert_eq!(
            body["error"]["message"],
            "The service is temporarily unavailable"
        );
    }
}

#[test]
fn queue_timeout_defaults_to_one_second() {
    assert_eq!(HttpOptions::default().queue_timeout(), QUEUE_TIMEOUT);
    assert_eq!(
        HttpOptions::default()
            .with_queue_timeout(Duration::ZERO)
            .queue_timeout(),
        Duration::ZERO
    );
}

#[test]
fn duration_buckets_cover_the_default_request_timeout() {
    assert_eq!(DURATION_BUCKETS.last(), Some(&30.0));
}
