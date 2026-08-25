//! Expo push adapter implementing the ticket then receipt protocol.

use std::collections::{BTreeMap, HashMap};

use axum::http::{HeaderMap, StatusCode};
use baukit_http::{classify_http_status, classify_transport_error};
use serde::{Deserialize, Serialize};

use crate::{
    PushDeliveryStatus, PushError, PushFuture, PushMessage, PushOptions, PushOptionsError,
    PushOutcome, PushRejection, PushSender,
};

const OK_STATUS: &str = "ok";

/// Expo push adapter.
///
/// Expo answers `/push/send` with one *ticket* per notification. A ticket only
/// says Expo accepted the notification; delivery is confirmed later through
/// `/push/getReceipts`. This adapter runs both phases per batch, so one
/// [`PushSender::send`] call returns settled outcomes wherever Expo has them and
/// [`PushDeliveryStatus::Accepted`] for the receipts that are not ready yet.
///
/// Cloning is cheap; the inner `reqwest` client shares its connection pool.
#[derive(Clone, Debug)]
pub struct ExpoPushSender {
    client: reqwest::Client,
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
        Ok(Self { client, options })
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
        let mut pending = HashMap::new();
        for (message, ticket) in messages.iter().zip(tickets) {
            match rejection(ticket.details.as_ref()) {
                Some(rejection) => outcomes.push(PushOutcome {
                    token: message.token.clone(),
                    status: PushDeliveryStatus::Rejected(rejection),
                }),
                None if ticket.status == OK_STATUS => {
                    let id = ticket.id.ok_or_else(|| {
                        PushError::InvalidResponse("accepted ticket has no id".to_owned())
                    })?;
                    pending.insert(id, message.token.clone());
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

        let ids = pending.keys().cloned().collect::<Vec<_>>();
        let response: ExpoReceiptResponse = self
            .post(
                self.options.receipts_endpoint(),
                &serde_json::json!({ "ids": ids }),
            )
            .await?;
        let receipts = response.data.unwrap_or_default();
        outcomes.extend(pending.into_iter().map(|(id, token)| PushOutcome {
            token,
            status: receipts.get(&id).map_or(
                // Expo has not settled this notification yet. Callers must not
                // resend on it; the next receipt poll or a later batch reports
                // the final state.
                PushDeliveryStatus::Accepted,
                |receipt| match rejection(receipt.details.as_ref()) {
                    Some(rejection) => PushDeliveryStatus::Rejected(rejection),
                    None if receipt.status == OK_STATUS => PushDeliveryStatus::Delivered,
                    None => PushDeliveryStatus::Rejected(PushRejection::ProviderError),
                },
            ),
        }));
        Ok(outcomes)
    }

    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        endpoint: &str,
        payload: &impl Serialize,
    ) -> Result<T, PushError> {
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
}

fn transport_error(status: StatusCode, headers: &HeaderMap) -> PushError {
    // Expo rejects a whole request for a bad token or a rate limit, so a 4xx
    // that is not one of those still classifies as permanent and the caller
    // learns not to retry it.
    let class = classify_http_status(status, headers, &[]);
    PushError::Transport { class }
}

fn expo_message(message: &PushMessage) -> ExpoMessage<'_> {
    ExpoMessage {
        to: &message.token,
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use baukit_http::RetryClass;

    use super::*;

    fn message(token: &str) -> PushMessage {
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
