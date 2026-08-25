//! Integration tests for [`ExpoPushSender`] against a local fake Expo endpoint.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header::RETRY_AFTER},
    routing::post,
};
use baukit_http::RetryClass;
use baukit_push::{
    ExpoPushSender, PushDeliveryStatus, PushError, PushMessage, PushOptions, PushOptionsError,
    PushOutcome, PushRejection, PushSender,
};
use serde_json::{Value, json};

struct FakeExpo {
    base_url: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FakeExpo {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeExpo {
    fn send_endpoint(&self) -> String {
        format!("{}/push/send", self.base_url)
    }
}

async fn start(app: Router) -> FakeExpo {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("fake Expo should bind an ephemeral port");
    let address = listener
        .local_addr()
        .expect("fake Expo should report its address");
    let task = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("fake Expo should serve");
    });
    FakeExpo {
        base_url: format!("http://{address}"),
        task,
    }
}

fn sender(server: &FakeExpo, batch_size: usize) -> Result<ExpoPushSender, PushOptionsError> {
    ExpoPushSender::with_options(
        PushOptions::new(server.send_endpoint())?.with_batch_size(batch_size)?,
    )
}

fn message(token: &str) -> PushMessage {
    PushMessage::new(token, "Reminder", "Your session starts soon")
        .with_data("session_id", "abc")
        .with_channel_id("reminders")
}

fn by_token(outcomes: Vec<PushOutcome>) -> HashMap<String, PushDeliveryStatus> {
    outcomes
        .into_iter()
        .map(|outcome| (outcome.token, outcome.status))
        .collect()
}

#[derive(Default)]
struct RecordedRequests {
    sends: Mutex<Vec<Vec<String>>>,
    receipts: Mutex<Vec<Vec<String>>>,
}

/// Answers every notification with an accepted ticket named after its token.
async fn tickets_for_every_token(
    State(recorded): State<Arc<RecordedRequests>>,
    Json(payload): Json<Vec<Value>>,
) -> Json<Value> {
    recorded.sends.lock().expect("sends lock").push(
        payload
            .iter()
            .map(|item| item["to"].as_str().expect("token").to_owned())
            .collect(),
    );
    let tickets = payload
        .iter()
        .map(|item| {
            assert_eq!(item["sound"], "default");
            assert_eq!(item["channelId"], "reminders");
            assert_eq!(item["data"]["session_id"], "abc");
            json!({"status": "ok", "id": format!("ticket-{}", item["to"].as_str().expect("token"))})
        })
        .collect::<Vec<_>>();
    Json(json!({ "data": tickets }))
}

async fn recorded_receipts(
    State(recorded): State<Arc<RecordedRequests>>,
    Json(payload): Json<Value>,
) -> Json<Value> {
    let ids = payload["ids"]
        .as_array()
        .expect("receipt ids")
        .iter()
        .map(|id| id.as_str().expect("receipt id").to_owned())
        .collect::<Vec<_>>();
    recorded
        .receipts
        .lock()
        .expect("receipts lock")
        .push(ids.clone());
    let data = ids
        .into_iter()
        .map(|id| {
            let receipt = if id == "ticket-two" {
                json!({"status": "error", "details": {"error": "DeviceNotRegistered"}})
            } else {
                json!({"status": "ok"})
            };
            (id, receipt)
        })
        .collect::<serde_json::Map<_, _>>();
    Json(json!({ "data": data }))
}

/// Confirms delivery for every requested receipt id.
async fn receipts_all_ok(Json(payload): Json<Value>) -> Json<Value> {
    let data = payload["ids"]
        .as_array()
        .expect("receipt ids")
        .iter()
        .map(|id| {
            (
                id.as_str().expect("receipt id").to_owned(),
                json!({"status": "ok"}),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    Json(json!({ "data": data }))
}

#[tokio::test]
async fn batching_preserves_request_order_and_maps_receipts_to_their_tickets()
-> Result<(), Box<dyn std::error::Error>> {
    let recorded = Arc::new(RecordedRequests::default());
    let server = start(
        Router::new()
            .route("/push/send", post(tickets_for_every_token))
            .route("/push/getReceipts", post(recorded_receipts))
            .with_state(recorded.clone()),
    )
    .await;

    let outcomes = sender(&server, 2)?
        .send(
            ["one", "two", "three", "four", "five"]
                .into_iter()
                .map(message)
                .collect(),
        )
        .await?;

    assert_eq!(
        recorded.sends.lock().expect("sends lock").as_slice(),
        [
            vec!["one".to_owned(), "two".to_owned()],
            vec!["three".to_owned(), "four".to_owned()],
            vec!["five".to_owned()]
        ]
    );
    assert_eq!(recorded.receipts.lock().expect("receipts lock").len(), 3);
    let statuses = by_token(outcomes);
    assert_eq!(statuses.len(), 5);
    assert_eq!(
        statuses["two"],
        PushDeliveryStatus::Rejected(PushRejection::DeviceNotRegistered)
    );
    assert_eq!(statuses["three"], PushDeliveryStatus::Delivered);
    Ok(())
}

#[tokio::test]
async fn a_partial_device_not_registered_leaves_the_rest_delivered()
-> Result<(), Box<dyn std::error::Error>> {
    async fn send() -> Json<Value> {
        Json(json!({"data": [
            {"status": "error", "details": {"error": "DeviceNotRegistered"}},
            {"status": "ok", "id": "live"}
        ]}))
    }
    let server = start(
        Router::new()
            .route("/push/send", post(send))
            .route("/push/getReceipts", post(receipts_all_ok)),
    )
    .await;

    let outcomes = sender(&server, 10)?
        .send(vec![message("gone"), message("live")])
        .await?;
    let dead = outcomes
        .iter()
        .filter(|outcome| outcome.is_token_dead())
        .map(|outcome| outcome.token.clone())
        .collect::<Vec<_>>();
    assert_eq!(dead, vec!["gone".to_owned()]);

    let statuses = by_token(outcomes);
    assert_eq!(
        statuses["gone"],
        PushDeliveryStatus::Rejected(PushRejection::DeviceNotRegistered)
    );
    assert_eq!(statuses["live"], PushDeliveryStatus::Delivered);
    Ok(())
}

#[tokio::test]
async fn receipt_requests_are_skipped_when_every_ticket_failed()
-> Result<(), Box<dyn std::error::Error>> {
    async fn send() -> Json<Value> {
        Json(json!({"data": [
            {"status": "error", "details": {"error": "DeviceNotRegistered"}},
            {"status": "error", "details": {"error": "MessageTooBig"}}
        ]}))
    }
    async fn receipts(
        State(calls): State<Arc<Mutex<usize>>>,
        Json(_payload): Json<Value>,
    ) -> Json<Value> {
        *calls.lock().expect("receipt calls lock") += 1;
        Json(json!({"data": {}}))
    }

    let receipt_calls = Arc::new(Mutex::new(0));
    let server = start(
        Router::new()
            .route("/push/send", post(send))
            .route("/push/getReceipts", post(receipts))
            .with_state(receipt_calls.clone()),
    )
    .await;

    let outcomes = sender(&server, 10)?
        .send(vec![message("gone"), message("too-big")])
        .await?;

    assert_eq!(outcomes.len(), 2);
    assert_eq!(*receipt_calls.lock().expect("receipt calls lock"), 0);
    Ok(())
}

#[tokio::test]
async fn receipt_requests_include_only_successful_ticket_ids()
-> Result<(), Box<dyn std::error::Error>> {
    async fn send() -> Json<Value> {
        Json(json!({"data": [
            {"status": "error", "details": {"error": "DeviceNotRegistered"}},
            {"status": "ok", "id": "ticket-live"},
            {"status": "error", "details": {"error": "MessageTooBig"}}
        ]}))
    }
    async fn receipts(
        State(requests): State<Arc<Mutex<Vec<Vec<String>>>>>,
        Json(payload): Json<Value>,
    ) -> Json<Value> {
        let ids = payload["ids"]
            .as_array()
            .expect("receipt ids")
            .iter()
            .map(|id| id.as_str().expect("receipt id").to_owned())
            .collect::<Vec<_>>();
        requests.lock().expect("receipt requests lock").push(ids);
        Json(json!({"data": {"ticket-live": {"status": "ok"}}}))
    }

    let receipt_requests = Arc::new(Mutex::new(Vec::new()));
    let server = start(
        Router::new()
            .route("/push/send", post(send))
            .route("/push/getReceipts", post(receipts))
            .with_state(receipt_requests.clone()),
    )
    .await;

    let statuses = by_token(
        sender(&server, 10)?
            .send(vec![message("gone"), message("live"), message("too-big")])
            .await?,
    );

    assert_eq!(
        receipt_requests
            .lock()
            .expect("receipt requests lock")
            .as_slice(),
        [vec!["ticket-live".to_owned()]]
    );
    assert_eq!(statuses["live"], PushDeliveryStatus::Delivered);
    Ok(())
}

#[tokio::test]
async fn every_expo_error_code_reaches_the_caller_as_a_neutral_rejection()
-> Result<(), Box<dyn std::error::Error>> {
    async fn send() -> Json<Value> {
        Json(json!({"data": [
            {"status": "error", "details": {"error": "MessageTooBig"}},
            {"status": "error", "details": {"error": "MessageRateExceeded"}},
            {"status": "error", "details": {"error": "InvalidCredentials"}},
            {"status": "error", "details": {"error": "MismatchSenderId"}},
            {"status": "error", "details": {"error": "SomethingBrandNew"}},
            {"status": "error"}
        ]}))
    }
    let server = start(Router::new().route("/push/send", post(send))).await;

    let statuses = by_token(
        sender(&server, 10)?
            .send(
                ["big", "rate", "creds", "sender", "novel", "bare"]
                    .into_iter()
                    .map(message)
                    .collect(),
            )
            .await?,
    );

    assert_eq!(
        statuses["big"],
        PushDeliveryStatus::Rejected(PushRejection::MessageTooBig)
    );
    assert_eq!(
        statuses["rate"],
        PushDeliveryStatus::Rejected(PushRejection::MessageRateExceeded)
    );
    assert_eq!(
        statuses["creds"],
        PushDeliveryStatus::Rejected(PushRejection::InvalidCredentials)
    );
    assert_eq!(
        statuses["sender"],
        PushDeliveryStatus::Rejected(PushRejection::ProviderError)
    );
    assert_eq!(
        statuses["novel"],
        PushDeliveryStatus::Rejected(PushRejection::Other("SomethingBrandNew".to_owned()))
    );
    assert_eq!(
        statuses["bare"],
        PushDeliveryStatus::Rejected(PushRejection::ProviderError)
    );
    Ok(())
}

#[tokio::test]
async fn receipts_reporting_errors_override_accepted_tickets()
-> Result<(), Box<dyn std::error::Error>> {
    async fn send() -> Json<Value> {
        Json(json!({"data": [
            {"status": "ok", "id": "delivered"},
            {"status": "ok", "id": "unregistered"},
            {"status": "ok", "id": "too-big"},
            {"status": "ok", "id": "not-settled"}
        ]}))
    }
    async fn receipts() -> Json<Value> {
        Json(json!({"data": {
            "delivered": {"status": "ok"},
            "unregistered": {"status": "error", "details": {"error": "DeviceNotRegistered"}},
            "too-big": {"status": "error", "details": {"error": "MessageTooBig"}}
        }}))
    }
    let server = start(
        Router::new()
            .route("/push/send", post(send))
            .route("/push/getReceipts", post(receipts)),
    )
    .await;

    let statuses = by_token(
        sender(&server, 10)?
            .send(
                ["delivered", "unregistered", "too-big", "not-settled"]
                    .into_iter()
                    .map(message)
                    .collect(),
            )
            .await?,
    );

    assert_eq!(statuses["delivered"], PushDeliveryStatus::Delivered);
    assert_eq!(
        statuses["unregistered"],
        PushDeliveryStatus::Rejected(PushRejection::DeviceNotRegistered)
    );
    assert_eq!(
        statuses["too-big"],
        PushDeliveryStatus::Rejected(PushRejection::MessageTooBig)
    );
    // Expo has no receipt yet. The notification is in flight, so this is
    // deliberately neither delivered nor rejected.
    assert_eq!(statuses["not-settled"], PushDeliveryStatus::Accepted);
    Ok(())
}

#[tokio::test]
async fn a_rate_limited_send_reports_the_delay_expo_asked_for()
-> Result<(), Box<dyn std::error::Error>> {
    async fn throttled() -> (StatusCode, HeaderMap, Json<Value>) {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, "42".parse().expect("header value"));
        (StatusCode::TOO_MANY_REQUESTS, headers, Json(json!({})))
    }
    let server = start(Router::new().route("/push/send", post(throttled))).await;

    let error = sender(&server, 10)?
        .send(vec![message("token")])
        .await
        .expect_err("a 429 fails the whole batch");
    assert!(error.is_retryable());
    assert_eq!(error.retry_after(), Some(Duration::from_secs(42)));
    assert!(matches!(
        error,
        PushError::Transport {
            class: RetryClass::RetryAfter(_)
        }
    ));
    Ok(())
}

#[tokio::test]
async fn a_rate_limit_without_a_header_still_classifies_as_rate_limited()
-> Result<(), Box<dyn std::error::Error>> {
    async fn throttled() -> (StatusCode, Json<Value>) {
        (StatusCode::TOO_MANY_REQUESTS, Json(json!({})))
    }
    let server = start(Router::new().route("/push/send", post(throttled))).await;

    let error = sender(&server, 10)?
        .send(vec![message("token")])
        .await
        .expect_err("a 429 fails the whole batch");
    assert!(error.is_retryable());
    assert_eq!(error.retry_after(), None);
    Ok(())
}

#[tokio::test]
async fn a_rejected_credential_is_not_retried() -> Result<(), Box<dyn std::error::Error>> {
    async fn unauthorized() -> (StatusCode, Json<Value>) {
        (StatusCode::UNAUTHORIZED, Json(json!({})))
    }
    let server = start(Router::new().route("/push/send", post(unauthorized))).await;

    let error = sender(&server, 10)?
        .send(vec![message("token")])
        .await
        .expect_err("401 fails the batch");
    assert!(!error.is_retryable());
    assert!(matches!(
        error,
        PushError::Transport {
            class: RetryClass::Revoked
        }
    ));
    Ok(())
}

#[tokio::test]
async fn a_configured_access_token_is_sent_as_a_bearer_header()
-> Result<(), Box<dyn std::error::Error>> {
    async fn echo_auth(headers: HeaderMap, Json(payload): Json<Vec<Value>>) -> Json<Value> {
        assert_eq!(
            headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer expo-secret")
        );
        Json(
            json!({"data": payload.iter().map(|_| json!({"status": "error"})).collect::<Vec<_>>()}),
        )
    }
    let server = start(Router::new().route("/push/send", post(echo_auth))).await;
    let sender = ExpoPushSender::with_options(
        PushOptions::new(server.send_endpoint())?.with_access_token("expo-secret")?,
    )?;

    assert_eq!(sender.send(vec![message("token")]).await?.len(), 1);
    Ok(())
}

#[tokio::test]
async fn malformed_responses_are_invalid_rather_than_retryable()
-> Result<(), Box<dyn std::error::Error>> {
    for body in [
        json!({}),
        json!({"data": []}),
        json!({"data": [{"status": "ok"}]}),
    ] {
        let server = start(Router::new().route(
            "/push/send",
            post(move || {
                let body = body.clone();
                async move { Json(body) }
            }),
        ))
        .await;
        let error = sender(&server, 10)?
            .send(vec![message("token")])
            .await
            .expect_err("a malformed body fails");
        assert!(matches!(error, PushError::InvalidResponse(_)), "{error:?}");
        assert!(!error.is_retryable());
    }
    Ok(())
}

#[tokio::test]
async fn an_unreachable_endpoint_is_a_retryable_transport_failure()
-> Result<(), Box<dyn std::error::Error>> {
    let sender = ExpoPushSender::with_options(PushOptions::new("http://127.0.0.1:1/push/send")?)?;
    let error = sender
        .send(vec![message("token")])
        .await
        .expect_err("nothing listens there");
    assert!(error.is_retryable());
    assert!(matches!(
        error,
        PushError::Transport {
            class: RetryClass::Unavailable
        }
    ));
    Ok(())
}

#[tokio::test]
async fn a_slow_endpoint_is_cut_off_by_the_request_timeout()
-> Result<(), Box<dyn std::error::Error>> {
    async fn slow() -> Json<Value> {
        tokio::time::sleep(Duration::from_secs(5)).await;
        Json(json!({"data": [{"status": "error"}]}))
    }
    let server = start(Router::new().route("/push/send", post(slow))).await;
    let sender = ExpoPushSender::with_options(
        PushOptions::new(server.send_endpoint())?
            .with_request_timeout(Duration::from_millis(50))?,
    )?;

    let started = tokio::time::Instant::now();
    let error = sender
        .send(vec![message("token")])
        .await
        .expect_err("the timeout fires");
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(matches!(
        error,
        PushError::Transport {
            class: RetryClass::Timeout
        }
    ));
    Ok(())
}
