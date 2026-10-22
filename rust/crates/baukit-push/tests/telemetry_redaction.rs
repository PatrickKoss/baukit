//! Receipt failure logs report retryability without notification data.

use std::{
    io,
    sync::{Arc, Mutex},
};

use axum::{Json, Router, http::StatusCode, routing::post};
use baukit_egress::{AddressPolicy, EgressOptions, GuardedClient};
use baukit_push::{
    DeviceToken, ExpoPushSender, PushDeliveryStatus, PushMessage, PushOptions, PushSender,
};
use serde_json::json;

#[derive(Clone, Default)]
struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl io::Write for CapturedLogs {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("logs").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn receipt_failures_log_retryability_and_keep_notification_data_private()
-> Result<(), Box<dyn std::error::Error>> {
    let logs = CapturedLogs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    for guarded in [false, true] {
        for status in [StatusCode::SERVICE_UNAVAILABLE, StatusCode::BAD_REQUEST] {
            let app = Router::new()
                .route(
                    "/push/send",
                    post(|| async {
                        Json(json!({"data": [{"status": "ok", "id": "ticket-secret"}]}))
                    }),
                )
                .route(
                    "/push/getReceipts",
                    post(move || async move { (status, "provider-body-secret") }),
                );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let endpoint = format!("http://{}/push/send?key=url-secret", listener.local_addr()?);
            let task = tokio::spawn(async move {
                axum::serve(listener, app).await.expect("server");
            });
            let sender = ExpoPushSender::with_options(
                PushOptions::new(endpoint)?.with_access_token("access-secret")?,
            )?;
            let sender = if guarded {
                sender.with_guarded_client(GuardedClient::new(
                    EgressOptions::default().with_policy(AddressPolicy::AllowLoopback),
                )?)
            } else {
                sender
            };
            let message = PushMessage::new(
                DeviceToken::new("token-secret")?,
                "title-secret",
                "body-secret",
            )
            .with_data("payload-secret", "value-secret");
            let outcomes = sender.send(vec![message]).await?;
            assert!(
                matches!(&outcomes[0].status, PushDeliveryStatus::Accepted(id) if id.as_str() == "ticket-secret")
            );
            task.abort();
        }
    }

    let logs = String::from_utf8(logs.0.lock().expect("logs").clone())?;
    let failures = logs
        .lines()
        .filter(|line| line.contains("immediate receipt fetch failed; tickets stay accepted"))
        .collect::<Vec<_>>();
    assert_eq!(failures.len(), 4, "{logs}");
    assert_eq!(
        failures
            .iter()
            .filter(|line| line.contains("retryable=true"))
            .count(),
        2,
        "{logs}"
    );
    assert_eq!(
        failures
            .iter()
            .filter(|line| line.contains("retryable=false"))
            .count(),
        2,
        "{logs}"
    );
    assert!(failures.iter().all(|line| line.contains("DEBUG")), "{logs}");
    for secret in [
        "ticket-secret",
        "provider-body-secret",
        "url-secret",
        "access-secret",
        "token-secret",
        "title-secret",
        "body-secret",
        "payload-secret",
        "value-secret",
    ] {
        assert!(!logs.contains(secret), "{secret} leaked into {logs}");
    }
    Ok(())
}
