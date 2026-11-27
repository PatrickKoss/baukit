use std::time::{Duration, Instant};

use axum::{Router, body::Body, http::Request, routing::get};
use tower::ServiceExt as _;

use super::*;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const ROUTE_TIMEOUT: Duration = Duration::from_millis(500);
const SLACK: Duration = Duration::from_millis(250);

async fn remaining_millis() -> String {
    current_deadline().map_or_else(
        || "none".to_owned(),
        |deadline| deadline.remaining(Instant::now()).as_millis().to_string(),
    )
}

async fn remaining_in(app: Router, path: &str) -> Option<Duration> {
    let request = Request::builder()
        .uri(path)
        .body(Body::empty())
        .expect("request");
    let response = app.oneshot(request).await.expect("response");
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let text = String::from_utf8(bytes.to_vec()).expect("utf-8 body");
    text.parse().ok().map(Duration::from_millis)
}

fn within(remaining: Option<Duration>, budget: Duration) -> bool {
    remaining.is_some_and(|left| left <= budget && left + SLACK >= budget)
}

#[tokio::test]
async fn handlers_see_the_request_deadline_and_the_tighter_route_deadline() {
    let options = HttpOptions {
        request_timeout: REQUEST_TIMEOUT,
        ..HttpOptions::default()
    };
    let app = layers(
        Router::new().route("/global", get(remaining_millis)).route(
            "/route",
            get(remaining_millis).layer(route_timeout(ROUTE_TIMEOUT)),
        ),
        options,
    );

    assert!(within(
        remaining_in(app.clone(), "/global").await,
        REQUEST_TIMEOUT
    ));
    assert!(within(remaining_in(app, "/route").await, ROUTE_TIMEOUT));
}

#[tokio::test]
async fn there_is_no_deadline_outside_a_request() {
    assert_eq!(current_deadline(), None);
    let app = Router::new().route("/bare", get(remaining_millis));
    assert_eq!(remaining_in(app, "/bare").await, None);
}
