//! Expo push adapter implementing the ticket then receipt protocol.

use std::collections::{BTreeMap, HashMap};

use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use baukit_http::{classify_http_status, classify_transport_error};
use serde::{Deserialize, Serialize};

use crate::{
    MAX_RECEIPT_BATCH_SIZE, PushDeliveryStatus, PushError, PushFuture, PushMessage, PushOptions,
    PushOptionsError, PushOutcome, PushReceipt, PushReceiptFuture, PushReceiptSource,
    PushRejection, PushSender, PushTicketId,
};

const OK_STATUS: &str = "ok";

/// Expo push adapter.
///
/// Expo answers `/push/send` with one *ticket* per notification. A ticket only
/// says Expo accepted the notification; delivery is confirmed later through
/// `/push/getReceipts`. This adapter runs both phases per batch, so one
/// [`PushSender::send`] call returns settled outcomes wherever Expo has them and
/// [`PushDeliveryStatus::Accepted`] for the receipts that are not ready yet.
/// As a [`PushReceiptSource`] it reads those receipts later, up to
/// [`MAX_RECEIPT_BATCH_SIZE`] IDs per request.
///
/// Cloning is cheap; the inner `reqwest` client shares its connection pool.
#[derive(Clone)]
pub struct ExpoPushSender {
    client: reqwest::Client,
    guarded: Option<baukit_egress::GuardedClient>,
    options: PushOptions,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExpoMessage<'a> {
    to: &'a str,
    title: &'a str,
    body: &'a str,
    sound: &'static str,
    data: &'a BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    channel_id: Option<&'a str>,
}

#[derive(Deserialize)]
struct ExpoTicketResponse {
    data: Option<Vec<ExpoTicket>>,
}

#[derive(Deserialize)]
struct ExpoTicket {
    status: String,
    id: Option<String>,
    details: Option<ExpoErrorDetails>,
}

#[derive(Deserialize)]
struct ExpoReceiptResponse {
    data: Option<HashMap<String, ExpoReceipt>>,
}

#[derive(Deserialize)]
struct ExpoReceipt {
    status: String,
    details: Option<ExpoErrorDetails>,
}

#[derive(Deserialize)]
struct ExpoErrorDetails {
    error: Option<String>,
}

impl std::fmt::Debug for ExpoPushSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExpoPushSender")
            .field("batch_size", &self.options.batch_size())
            .field("request_timeout", &self.options.request_timeout())
            .field("guarded", &self.guarded.is_some())
            .finish_non_exhaustive()
    }
}

impl ExpoPushSender {
    /// Builds a sender against the public Expo API with default options.
    pub fn new() -> Result<Self, PushOptionsError> {
        Self::with_options(PushOptions::default())
    }

    /// Builds a sender from validated options.
    pub fn with_options(options: PushOptions) -> Result<Self, PushOptionsError> {
        let client = reqwest::Client::builder()
            .timeout(options.request_timeout())
            .build()
            .map_err(|error| PushOptionsError::InvalidEndpoint(error.to_string()))?;
        Ok(Self {
            client,
            options,
            guarded: None,
        })
    }

    /// Sends ticket requests and immediate or deferred receipt requests through `client`.
    ///
    /// The client's policy, timeouts and response limits apply. Configure its
    /// request timeout explicitly if it should match [`PushOptions::request_timeout`].
    #[must_use]
    pub fn with_guarded_client(mut self, client: baukit_egress::GuardedClient) -> Self {
        self.guarded = Some(client);
        self
    }

    /// Returns the options this sender was built from.
    #[must_use]
    pub const fn options(&self) -> &PushOptions {
        &self.options
    }

    async fn send_chunk(&self, messages: &[PushMessage]) -> Result<Vec<PushOutcome>, PushError> {
        let payload = messages.iter().map(expo_message).collect::<Vec<_>>();
        let response: ExpoTicketResponse = self.post(self.options.endpoint(), &payload).await?;
        let tickets = response
            .data
            .ok_or_else(|| PushError::InvalidResponse("ticket response has no data".to_owned()))?;
        if tickets.len() != messages.len() {
            return Err(PushError::InvalidResponse(format!(
                "expected {} tickets, got {}",
                messages.len(),
                tickets.len()
            )));
        }

        let mut outcomes = Vec::with_capacity(messages.len());
        let mut pending = Vec::new();
        for (message, ticket) in messages.iter().zip(tickets) {
            match rejection(ticket.details.as_ref()) {
                Some(rejection) => outcomes.push(PushOutcome {
                    token: message.token.clone(),
                    status: PushDeliveryStatus::Rejected(rejection),
                }),
                None if ticket.status == OK_STATUS => {
                    pending.push((ticket_id(ticket.id)?, message.token.clone()));
                }
                None => outcomes.push(PushOutcome {
                    token: message.token.clone(),
                    status: PushDeliveryStatus::Rejected(PushRejection::ProviderError),
                }),
            }
        }
        if pending.is_empty() {
            return Ok(outcomes);
        }

        let ids = pending.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
        // Accepted tickets must survive a failed immediate receipt request.
        // The durable receipt job settles them without sending again.
        let mut receipts = self.fetch_receipts(&ids).await.unwrap_or_else(|error| {
            tracing::debug!(
                target: "baukit_push",
                retryable = error.is_retryable(),
                "immediate receipt fetch failed; tickets stay accepted"
            );
            HashMap::new()
        });
        outcomes.extend(pending.into_iter().map(|(id, token)| {
            let status = match receipts.remove(&id) {
                Some(receipt) => receipt.into(),
                // Expo has not settled this notification yet. Callers must
                // not resend on it; a deferred receipt poll reports the final
                // state.
                None => PushDeliveryStatus::Accepted(id),
            };
            PushOutcome { token, status }
        }));
        Ok(outcomes)
    }

    async fn fetch_receipts(
        &self,
        ids: &[PushTicketId],
    ) -> Result<HashMap<PushTicketId, PushReceipt>, PushError> {
        let mut settled = HashMap::new();
        for chunk in ids.chunks(MAX_RECEIPT_BATCH_SIZE) {
            let request = chunk.iter().map(PushTicketId::as_str).collect::<Vec<_>>();
            let response: ExpoReceiptResponse = self
                .post(
                    self.options.receipts_endpoint(),
                    &serde_json::json!({ "ids": request }),
                )
                .await?;
            let mut receipts = response.data.unwrap_or_default();
            settled.extend(chunk.iter().filter_map(|id| {
                receipts
                    .remove(id.as_str())
                    .map(|receipt| (id.clone(), push_receipt(&receipt)))
            }));
        }
        Ok(settled)
    }

    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        endpoint: &str,
        payload: &impl Serialize,
    ) -> Result<T, PushError> {
        if let Some(client) = &self.guarded {
            return self.post_guarded(client, endpoint, payload).await;
        }
        let mut request = self.client.post(endpoint).json(payload);
        if let Some(token) = self.options.access_token() {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.map_err(|error| {
            tracing::debug!(target: "baukit_push", %error, endpoint, "expo request failed");
            PushError::Transport {
                class: classify_transport_error(error.is_timeout()),
            }
        })?;
        let status = response.status();
        if !status.is_success() {
            return Err(transport_error(status, response.headers()));
        }
        response.json::<T>().await.map_err(|error| {
            PushError::InvalidResponse(format!("{endpoint} response did not parse: {error}"))
        })
    }

    async fn post_guarded<T: serde::de::DeserializeOwned>(
        &self,
        client: &baukit_egress::GuardedClient,
        endpoint: &str,
        payload: &impl Serialize,
    ) -> Result<T, PushError> {
        let url = endpoint
            .parse()
            .map_err(|_| PushError::InvalidResponse("invalid endpoint".into()))?;
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        if let Some(token) = self.options.access_token() {
            let mut authorization: HeaderValue = format!("Bearer {token}")
                .parse()
                .map_err(|_| PushError::InvalidResponse("invalid credential".into()))?;
            authorization.set_sensitive(true);
            headers.insert(header::AUTHORIZATION, authorization);
        }
        let body = serde_json::to_vec(payload)
            .map_err(|_| PushError::InvalidResponse("invalid request".into()))?;
        let response = client
            .execute(
                baukit_egress::EgressRequest::post(url)
                    .with_headers(headers)
                    .with_body(body),
            )
            .await
            .map_err(|error| PushError::Transport {
                class: error.retry_class(),
            })?;
        serde_json::from_slice(response.body())
            .map_err(|_| PushError::InvalidResponse("invalid response".into()))
    }
}

fn transport_error(status: StatusCode, headers: &HeaderMap) -> PushError {
    // Expo rejects a whole request for a bad token or a rate limit, so a 4xx
    // that is not one of those still classifies as permanent and the caller
    // learns not to retry it.
    let class = classify_http_status(status, headers, &[]);
    PushError::Transport { class }
}

fn ticket_id(id: Option<String>) -> Result<PushTicketId, PushError> {
    id.ok_or_else(|| PushError::InvalidResponse("accepted ticket has no id".to_owned()))
        .and_then(|id| {
            PushTicketId::new(id).map_err(|error| PushError::InvalidResponse(error.to_string()))
        })
}

fn push_receipt(receipt: &ExpoReceipt) -> PushReceipt {
    match rejection(receipt.details.as_ref()) {
        Some(rejection) => PushReceipt::Rejected(rejection),
        None if receipt.status == OK_STATUS => PushReceipt::Delivered,
        None => PushReceipt::Rejected(PushRejection::ProviderError),
    }
}

fn expo_message(message: &PushMessage) -> ExpoMessage<'_> {
    ExpoMessage {
        to: message.token.expose(),
        title: &message.title,
        body: &message.body,
        sound: "default",
        data: &message.data,
        channel_id: message.channel_id.as_deref(),
    }
}

fn rejection(details: Option<&ExpoErrorDetails>) -> Option<PushRejection> {
    details
        .and_then(|details| details.error.as_deref())
        .map(PushRejection::from_code)
}

impl PushSender for ExpoPushSender {
    fn send<'a>(&'a self, batch: Vec<PushMessage>) -> PushFuture<'a> {
        Box::pin(async move {
            let mut outcomes = Vec::with_capacity(batch.len());
            for chunk in batch.chunks(self.options.batch_size()) {
                outcomes.extend(self.send_chunk(chunk).await?);
            }
            Ok(outcomes)
        })
    }
}

impl PushReceiptSource for ExpoPushSender {
    fn receipts<'a>(&'a self, tickets: Vec<PushTicketId>) -> PushReceiptFuture<'a> {
        Box::pin(async move { self.fetch_receipts(&tickets).await })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use baukit_http::RetryClass;

    use super::*;

    fn message(token: &str) -> PushMessage {
        let token = crate::DeviceToken::new(token).expect("valid test token");
        PushMessage::new(token, "Reminder", "Your session starts soon")
            .with_data("session_id", "abc")
    }

    #[test]
    fn a_message_serializes_into_expos_camel_case_shape() {
        let payload = serde_json::to_value(expo_message(
            &message("ExponentPushToken[x]").with_channel_id("reminders"),
        ))
        .expect("message should serialize");
        assert_eq!(payload["to"], "ExponentPushToken[x]");
        assert_eq!(payload["title"], "Reminder");
        assert_eq!(payload["body"], "Your session starts soon");
        assert_eq!(payload["sound"], "default");
        assert_eq!(payload["channelId"], "reminders");
        assert_eq!(payload["data"]["session_id"], "abc");
    }

    #[test]
    fn a_message_without_a_channel_omits_the_field_entirely() {
        let payload =
            serde_json::to_value(expo_message(&message("token"))).expect("message serializes");
        assert!(payload.get("channelId").is_none());
    }

    #[test]
    fn ticket_error_details_map_onto_neutral_rejections() {
        let parsed: ExpoTicket = serde_json::from_value(serde_json::json!({
            "status": "error",
            "details": {"error": "DeviceNotRegistered"}
        }))
        .expect("ticket should parse");
        assert_eq!(parsed.status, "error");
        assert_eq!(
            rejection(parsed.details.as_ref()),
            Some(PushRejection::DeviceNotRegistered)
        );
    }

    #[test]
    fn an_error_without_details_has_no_specific_rejection() {
        let parsed: ExpoTicket =
            serde_json::from_value(serde_json::json!({"status": "error"})).expect("ticket parses");
        assert_eq!(rejection(parsed.details.as_ref()), None);
    }

    #[test]
    fn receipts_parse_keyed_by_ticket_id() {
        let parsed: ExpoReceiptResponse = serde_json::from_value(serde_json::json!({
            "data": {
                "ticket-1": {"status": "ok"},
                "ticket-2": {"status": "error", "details": {"error": "MessageTooBig"}}
            }
        }))
        .expect("receipts should parse");
        let data = parsed.data.expect("receipts have data");
        assert_eq!(data["ticket-1"].status, OK_STATUS);
        assert_eq!(
            rejection(data["ticket-2"].details.as_ref()),
            Some(PushRejection::MessageTooBig)
        );
    }

    #[test]
    fn upstream_statuses_classify_into_transport_errors() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::RETRY_AFTER,
            axum::http::HeaderValue::from_static("12"),
        );
        assert_eq!(
            transport_error(StatusCode::TOO_MANY_REQUESTS, &headers).retry_after(),
            Some(Duration::from_secs(12))
        );
        assert!(matches!(
            transport_error(StatusCode::UNAUTHORIZED, &HeaderMap::new()),
            PushError::Transport {
                class: RetryClass::Revoked
            }
        ));
        assert!(matches!(
            transport_error(StatusCode::BAD_GATEWAY, &HeaderMap::new()),
            PushError::Transport {
                class: RetryClass::Unavailable
            }
        ));
        assert!(!transport_error(StatusCode::BAD_REQUEST, &HeaderMap::new()).is_retryable());
    }
}

#[cfg(test)]
mod guarded_transport_tests {
    use super::*;
    use axum::{Json, Router, routing::post};
    use serde_json::{Value, json};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    #[tokio::test]
    async fn guarded_batches_keep_accepted_tickets_when_receipts_fail()
    -> Result<(), Box<dyn std::error::Error>> {
        let sends = Arc::new(AtomicUsize::new(0));
        let receipt_requests = Arc::new(AtomicUsize::new(0));
        let ready = Arc::new(AtomicBool::new(false));
        let app = Router::new()
            .route(
                "/push/send",
                post({
                    let sends = sends.clone();
                    move |Json(messages): Json<Vec<Value>>| {
                        let sends = sends.clone();
                        async move {
                            assert_eq!(messages.len(), 1, "configured batch size");
                            let id = sends.fetch_add(1, Ordering::SeqCst);
                            Json(json!({"data":[{"status":"ok","id":format!("guarded-{id}")}]}))
                        }
                    }
                }),
            )
            .route(
                "/push/getReceipts",
                post({
                    let ready = ready.clone();
                    let receipt_requests = receipt_requests.clone();
                    move |Json(request): Json<Value>| {
                        let ready = ready.clone();
                        let receipt_requests = receipt_requests.clone();
                        async move {
                            receipt_requests.fetch_add(1, Ordering::SeqCst);
                            if !ready.load(Ordering::SeqCst) {
                                return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({})));
                            }
                            let data = request["ids"]
                                .as_array()
                                .expect("ids")
                                .iter()
                                .map(|id| (
                                    id.as_str().expect("ticket ID").to_owned(),
                                    json!({"status":"error","details":{"error":"DeviceNotRegistered"}}),
                                ))
                                .collect::<serde_json::Map<_,_>>();
                            (StatusCode::OK, Json(json!({"data":data})))
                        }
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let endpoint = format!("http://{}/push/send", listener.local_addr()?);
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("Expo fixture");
        });
        let options = PushOptions::new(&endpoint)?.with_batch_size(1)?;
        let sender = ExpoPushSender::with_options(options.clone())?.with_guarded_client(
            baukit_egress::GuardedClient::new(
                baukit_egress::EgressOptions::default()
                    .with_policy(baukit_egress::AddressPolicy::AllowLoopback),
            )?,
        );
        let messages = ["first", "second"]
            .into_iter()
            .map(|value| {
                PushMessage::new(
                    crate::DeviceToken::new(value).expect("token"),
                    "Reminder",
                    "Ready",
                )
            })
            .collect();
        let outcomes = sender.send(messages).await?;
        assert_eq!(sends.load(Ordering::SeqCst), 2);
        let tickets = outcomes
            .iter()
            .map(|outcome| match &outcome.status {
                PushDeliveryStatus::Accepted(id) => id.clone(),
                other => panic!("accepted ticket lost after receipt failure: {other:?}"),
            })
            .collect::<Vec<_>>();
        ready.store(true, Ordering::SeqCst);
        let receipts = sender.receipts(tickets.clone()).await?;
        assert_eq!(receipts.len(), 2);
        for id in &tickets {
            assert_eq!(
                receipts[id],
                PushReceipt::Rejected(PushRejection::DeviceNotRegistered)
            );
        }
        let blocked = ExpoPushSender::with_options(options)?.with_guarded_client(
            baukit_egress::GuardedClient::new(baukit_egress::EgressOptions::default())?,
        );
        assert!(
            blocked
                .send(vec![PushMessage::new(
                    crate::DeviceToken::new("blocked")?,
                    "Reminder",
                    "Ready"
                )])
                .await
                .is_err()
        );
        assert!(blocked.receipts(tickets).await.is_err());
        assert_eq!(sends.load(Ordering::SeqCst), 2);
        assert_eq!(receipt_requests.load(Ordering::SeqCst), 3);
        task.abort();
        Ok(())
    }
}
