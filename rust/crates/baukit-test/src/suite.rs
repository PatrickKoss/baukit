//! Suite protocol fixtures and a second app served through an in-process router.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request},
};
use baukit_core::webhook_signature::sign_webhook_hmac_sha256;
use baukit_suite::{
    domain::{ActivePeer, ExchangeRequest, ExchangeResponse},
    ports::{SuitePeerClient, SuitePeerError, SuitePeerResponse, SuiteSignedCall},
};
use std::sync::{Arc, RwLock};
use tower::ServiceExt as _;

/// Scripted network receiver for checking signed request bytes and headers.
pub use crate::ScriptedWebhookReceiver as ScriptedSuiteReceiver;

/// Reads a checked-in suite fixture. Unknown names return `None`.
pub fn fixture(name: &str) -> Option<&'static str> {
    match name {
        "SHA256SUMS" => Some(include_str!("suite/fixtures/v1/SHA256SUMS")),
        "peers.json" => Some(include_str!("suite/fixtures/v1/peers.json")),
        "peer-metadata.json" => Some(include_str!("suite/fixtures/v1/peer-metadata.json")),
        "catalog.json" => Some(include_str!("suite/fixtures/v1/catalog.json")),
        "signature.json" => Some(include_str!("suite/fixtures/v1/signature.json")),
        "link-protocol.json" => Some(include_str!("suite/fixtures/v1/link-protocol.json")),
        "event-ids.json" => Some(include_str!("suite/fixtures/v1/event-ids.json")),
        "payload-validator.json" => Some(include_str!("suite/fixtures/v1/payload-validator.json")),
        _ => None,
    }
}

/// Runs the other app's real suite router without a listening socket.
#[derive(Clone, Default)]
pub struct InProcessSuitePeer {
    router: Arc<RwLock<Option<Router>>>,
}
impl InProcessSuitePeer {
    /// Creates an unmounted peer so two app contexts can be wired before either router exists.
    pub fn new() -> Self {
        Self::default()
    }
    /// Installs the other role's router. Poisoned locks are reported as errors.
    pub fn mount(&self, router: Router) -> Result<(), SuitePeerError> {
        *self.router.write().map_err(|_| SuitePeerError::Transport)? = Some(router);
        Ok(())
    }
    async fn request(
        &self,
        request: Request<Body>,
    ) -> Result<(u16, Option<std::time::Duration>, Vec<u8>), SuitePeerError> {
        let router = self
            .router
            .read()
            .map_err(|_| SuitePeerError::Transport)?
            .clone()
            .ok_or(SuitePeerError::Transport)?;
        let response = router
            .oneshot(request)
            .await
            .map_err(|_| SuitePeerError::Transport)?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok())
            .map(std::time::Duration::from_secs);
        let bytes = to_bytes(response.into_body(), 65_536)
            .await
            .map_err(|_| SuitePeerError::InvalidResponse)?;
        Ok((status, retry_after, bytes.to_vec()))
    }
    async fn signed(
        &self,
        path: String,
        call: SuiteSignedCall<'_>,
    ) -> Result<SuitePeerResponse, SuitePeerError> {
        let mut request = Request::builder()
            .method(Method::POST)
            .uri(path)
            .header("content-type", "application/json")
            .header(
                "x-suite-signature",
                sign_webhook_hmac_sha256(call.secret, call.timestamp, call.delivery_id, call.body),
            )
            .header("x-suite-source", call.source_app)
            .header("x-suite-timestamp", call.timestamp.to_string())
            .header("x-suite-delivery-id", call.delivery_id);
        if call.replay {
            request = request.header("x-suite-replay", "1");
        }
        let (status, retry_after, _) = self
            .request(
                request
                    .body(Body::from(call.body.to_vec()))
                    .map_err(|_| SuitePeerError::InvalidResponse)?,
            )
            .await?;
        Ok(SuitePeerResponse {
            status,
            retry_after,
        })
    }
}
#[async_trait::async_trait]
impl SuitePeerClient for InProcessSuitePeer {
    async fn exchange(
        &self,
        _: &ActivePeer,
        request: &ExchangeRequest,
    ) -> Result<ExchangeResponse, SuitePeerError> {
        let bytes = serde_json::to_vec(request).map_err(|_| SuitePeerError::InvalidResponse)?;
        let request = Request::builder()
            .method(Method::POST)
            .uri("/suite/links/exchange")
            .header("content-type", "application/json")
            .body(Body::from(bytes))
            .map_err(|_| SuitePeerError::InvalidResponse)?;
        let (status, _, body) = self.request(request).await?;
        if status != 201 {
            let code = serde_json::from_slice::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v["error"]["code"].as_str().map(str::to_owned))
                .unwrap_or_else(|| "suite_peer_rejected".into());
            return Err(SuitePeerError::Rejected { status, code });
        }
        serde_json::from_slice(&body).map_err(|_| SuitePeerError::InvalidResponse)
    }
    async fn deliver(
        &self,
        _: &ActivePeer,
        call: SuiteSignedCall<'_>,
    ) -> Result<SuitePeerResponse, SuitePeerError> {
        self.signed(format!("/suite/inbound/{}", call.remote_link_id), call)
            .await
    }
    async fn revoke(
        &self,
        _: &ActivePeer,
        call: SuiteSignedCall<'_>,
    ) -> Result<SuitePeerResponse, SuitePeerError> {
        self.signed(format!("/suite/links/{}/revoke", call.remote_link_id), call)
            .await
    }
}
