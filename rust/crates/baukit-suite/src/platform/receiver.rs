use std::sync::Arc;

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use baukit_config::Secret;
use baukit_core::webhook_signature::verify_webhook_hmac_sha256;
use baukit_http::ApiError;
use sha2::{Digest, Sha256};

use super::*;
use crate::domain::{SUITE_MAX_BODY_BYTES, signature_timestamp_valid};

const MAX_DELIVERY_ID_BYTES: usize = 256;

#[derive(Clone)]
pub struct PlatformReceiver {
    decoder: PlatformEventDecoder,
    current_secret: Secret<String>,
    previous_secret: Option<Secret<String>>,
    inbox: Arc<dyn PlatformInbox>,
    handler: Arc<dyn PlatformEventHandler>,
    clock: Arc<dyn PlatformClock>,
    body_limit: usize,
}
impl PlatformReceiver {
    pub fn new(
        decoder: PlatformEventDecoder,
        current_secret: Secret<String>,
        previous_secret: Option<Secret<String>>,
        inbox: Arc<dyn PlatformInbox>,
        handler: Arc<dyn PlatformEventHandler>,
    ) -> Result<Self, PlatformError> {
        if current_secret.expose().is_empty()
            || previous_secret
                .as_ref()
                .is_some_and(|s| s.expose().is_empty())
        {
            return Err(PlatformError::Invalid);
        }
        Ok(Self {
            decoder,
            current_secret,
            previous_secret,
            inbox,
            handler,
            clock: Arc::new(SystemPlatformClock),
            body_limit: SUITE_MAX_BODY_BYTES,
        })
    }
    pub fn with_clock(mut self, clock: Arc<dyn PlatformClock>) -> Self {
        self.clock = clock;
        self
    }
    pub fn with_body_limit(mut self, body_limit: usize) -> Result<Self, PlatformError> {
        if body_limit == 0 {
            return Err(PlatformError::Invalid);
        }
        self.body_limit = body_limit;
        Ok(self)
    }
}

pub fn router(state: PlatformReceiver) -> Router {
    Router::new()
        .route("/platform/inbound", post(inbound))
        .with_state(state)
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(inbound))]
pub struct PlatformOpenApi;

#[utoipa::path(
    post,
    path = "/platform/inbound",
    operation_id = "platform_inbound",
    summary = "Receive a signed hub event",
    description = "Authenticates raw JSON using the X-Suite signature headers. Duplicate and unknown events are acknowledged. Unavailable handlers or storage return 503 for retry.",
    request_body(content = serde_json::Value, content_type = "application/json"),
    params(
        ("x-suite-source" = String, Header, description = "Configured hub source"),
        ("x-suite-timestamp" = String, Header, description = "Canonical Unix seconds within 300 seconds"),
        ("x-suite-delivery-id" = String, Header, description = "Delivery attempt ID"),
        ("x-suite-signature" = String, Header, description = "HMAC-SHA256 signature")
    ),
    responses(
        (status = 204, description = "Applied, duplicate or unknown event"),
        (status = 401, description = "Invalid signature headers"),
        (status = 409, description = "Event ID reused with changed bytes"),
        (status = 413, description = "Body too large"),
        (status = 415, description = "JSON content type required"),
        (status = 422, description = "Invalid event"),
        (status = 503, description = "Retry after handler or storage failure")
    ),
    tag = "platform"
)]
async fn inbound(
    State(state): State<PlatformReceiver>,
    headers: HeaderMap,
    body: Body,
) -> Result<StatusCode, ApiError> {
    let body = to_bytes(body, state.body_limit).await.map_err(|_| {
        ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "body_too_large",
            "The request body is too large",
        )
    })?;
    let invalid_signature = || {
        ApiError::new(
            StatusCode::UNAUTHORIZED,
            "platform_signature_invalid",
            "A valid platform signature is required",
        )
    };
    let required = |name| single_header(&headers, name).ok_or_else(invalid_signature);
    let timestamp = required("x-suite-timestamp")?;
    let parsed = timestamp.parse::<i64>().map_err(|_| invalid_signature())?;
    let now = state.clock.now();
    let delivery_id = required("x-suite-delivery-id")?;
    if parsed < 0
        || parsed.to_string() != timestamp
        || !signature_timestamp_valid(parsed, now.timestamp())
        || delivery_id.is_empty()
        || delivery_id.len() > MAX_DELIVERY_ID_BYTES
        || delivery_id.chars().any(char::is_control)
        || required("x-suite-source")? != state.decoder.source_app()
        || !verify_webhook_hmac_sha256(
            std::iter::once(state.current_secret.expose().as_bytes()).chain(
                state
                    .previous_secret
                    .as_ref()
                    .map(|s| s.expose().as_bytes()),
            ),
            parsed,
            delivery_id,
            &body,
            required("x-suite-signature")?,
        )
    {
        return Err(invalid_signature());
    }
    if !single_header(&headers, "content-type")
        .and_then(|s| s.split(';').next())
        .is_some_and(|s| s.trim().eq_ignore_ascii_case("application/json"))
    {
        return Err(ApiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "platform_content_type_invalid",
            "The request content type must be JSON",
        ));
    }
    let event = state.decoder.decode(&body, now).map_err(api_error)?;
    let handler = TimedHandler {
        inner: state.handler.as_ref(),
        now,
    };
    state
        .inbox
        .receive(&event, Sha256::digest(&body).into(), &handler)
        .await
        .map_err(api_error)?;
    Ok(StatusCode::NO_CONTENT)
}
fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    values.next().is_none().then_some(value)
}
fn api_error(error: PlatformError) -> ApiError {
    let status = match error {
        PlatformError::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        PlatformError::Invalid => StatusCode::UNPROCESSABLE_ENTITY,
        PlatformError::EventIdConflict => StatusCode::CONFLICT,
    };
    ApiError::new(
        status,
        error.to_string(),
        "The platform event could not be received",
    )
}

struct TimedHandler<'a> {
    inner: &'a dyn PlatformEventHandler,
    now: chrono::DateTime<chrono::Utc>,
}
#[async_trait::async_trait]
impl PlatformEventHandler for TimedHandler<'_> {
    async fn prepare(
        &self,
        event: &PlatformEvent,
    ) -> Result<Option<Arc<dyn PlatformEventHandler>>, PlatformError> {
        self.inner.prepare_at(event, self.now).await
    }
    async fn handle(
        &self,
        event: &PlatformEvent,
        context: &mut EventContext<'_>,
    ) -> Result<(), PlatformError> {
        self.inner.handle(event, context).await
    }
}
