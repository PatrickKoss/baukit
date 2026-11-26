#![cfg(test)]
//! A send that fails part way keeps the tickets Expo already issued.

use std::{
    num::NonZeroU32,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use baukit_push::{
    DevicePlatform, DeviceRegistration, DeviceRegistry, DeviceToken, ExpoPushSender,
    MemoryDeviceRegistry, MemoryPendingReceiptStore, PendingReceiptStore, PushDeliveryStatus,
    PushMessage, PushOptions, PushSender, RECEIPT_POLL_DELAY, ReceiptPoll, poll_pending_receipts,
};
use jiff::Timestamp;
use serde_json::{Value, json};
use uuid::Uuid;

const BATCH_SIZE: usize = 2;
const FAILING_REQUEST: usize = 1;
const POLL_LIMIT: NonZeroU32 = NonZeroU32::MIN.saturating_add(99);
const TOKENS: [&str; 6] = ["a", "b", "c", "d", "e", "f"];

#[derive(Default)]
struct ScriptedExpo {
    sends: Mutex<Vec<Vec<String>>>,
    failed_once: AtomicBool,
    receipts_settled: AtomicBool,
}

async fn scripted_send(
    State(expo): State<Arc<ScriptedExpo>>,
    Json(payload): Json<Vec<Value>>,
) -> (StatusCode, Json<Value>) {
    let tokens = payload
        .iter()
        .map(|item| item["to"].as_str().expect("token").to_owned())
        .collect::<Vec<_>>();
    let request = {
        let mut sends = expo.sends.lock().expect("sends lock");
        sends.push(tokens.clone());
        sends.len() - 1
    };
    if request == FAILING_REQUEST && !expo.failed_once.swap(true, Ordering::SeqCst) {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({})));
    }
    let tickets = tokens
        .iter()
        .map(|token| json!({"status": "ok", "id": format!("ticket-{token}")}))
        .collect::<Vec<_>>();
    (StatusCode::OK, Json(json!({ "data": tickets })))
}

async fn scripted_receipts(
    State(expo): State<Arc<ScriptedExpo>>,
    Json(payload): Json<Value>,
) -> Json<Value> {
    if !expo.receipts_settled.load(Ordering::SeqCst) {
        return Json(json!({ "data": {} }));
    }
    let data = payload["ids"]
        .as_array()
        .expect("receipt ids")
        .iter()
        .map(|id| (id.as_str().expect("id").to_owned(), json!({"status": "ok"})))
        .collect::<serde_json::Map<_, _>>();
    Json(json!({ "data": data }))
}

fn message(value: &str) -> PushMessage {
    PushMessage::new(
        DeviceToken::new(value).expect("valid test token"),
        "Reminder",
        "Ready",
    )
}

fn tokens_of(messages: &[PushMessage]) -> Vec<&str> {
    messages
        .iter()
        .map(|message| message.token.expose())
        .collect()
}

#[tokio::test]
async fn a_failed_middle_chunk_keeps_earlier_tickets_and_a_retry_sends_only_the_rest()
-> Result<(), Box<dyn std::error::Error>> {
    let expo = Arc::new(ScriptedExpo::default());
    let app = Router::new()
        .route("/push/send", post(scripted_send))
        .route("/push/getReceipts", post(scripted_receipts))
        .with_state(expo.clone());
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let endpoint = format!("http://{}/push/send", listener.local_addr()?);
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("fake Expo serves");
    });
    let sender =
        ExpoPushSender::with_options(PushOptions::new(&endpoint)?.with_batch_size(BATCH_SIZE)?)?;
    let owner = Uuid::now_v7();
    let registry = MemoryDeviceRegistry::new();
    let pending = MemoryPendingReceiptStore::new();
    let sent_at = Timestamp::now();
    for value in TOKENS {
        registry
            .register(DeviceRegistration::new(
                owner,
                DeviceToken::new(value)?,
                DevicePlatform::Ios,
                sent_at,
            ))
            .await?;
    }

    let failure = sender
        .send(TOKENS.into_iter().map(message).collect())
        .await
        .expect_err("the second chunk fails");
    assert!(failure.is_retryable());
    assert_eq!(tokens_of(&failure.unsent), ["c", "d", "e", "f"]);
    let accepted = failure
        .outcomes
        .iter()
        .map(|outcome| match &outcome.status {
            PushDeliveryStatus::Accepted(ticket) => ticket.as_str().to_owned(),
            other => panic!("chunk one should stay accepted, got {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(accepted, ["ticket-a", "ticket-b"]);
    assert_eq!(
        pending
            .record_accepted(owner, &failure.outcomes, sent_at)
            .await?,
        2
    );

    expo.receipts_settled.store(true, Ordering::SeqCst);
    let poll = poll_pending_receipts(
        &sender,
        &pending,
        &registry,
        sent_at + RECEIPT_POLL_DELAY,
        POLL_LIMIT,
    )
    .await?;
    assert_eq!(
        poll,
        ReceiptPoll {
            taken: 2,
            settled: 2,
            invalidated: 0
        }
    );

    let retried = sender.send(failure.unsent).await?;
    assert_eq!(retried.len(), 4);
    let sends = expo.sends.lock().expect("sends lock").clone();
    assert_eq!(
        sends,
        [
            vec!["a", "b"],
            vec!["c", "d"],
            vec!["c", "d"],
            vec!["e", "f"]
        ]
    );
    server.abort();
    Ok(())
}
