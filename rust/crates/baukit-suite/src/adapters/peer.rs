use crate::domain::*;
use crate::ports::*;
use async_trait::async_trait;
use baukit_core::webhook_signature::sign_webhook_hmac_sha256;
use baukit_egress::{
    AddressPolicy, EgressClientError, EgressError, EgressOptions, EgressRequest, GuardedClient,
    ResponseBody,
};
use reqwest::{
    Method, Url,
    header::{HeaderMap, HeaderName, HeaderValue},
};

#[derive(Clone)]
pub struct ReqwestSuitePeerClient {
    client: GuardedClient,
}
impl ReqwestSuitePeerClient {
    pub fn new(allow_loopback: bool) -> Result<Self, EgressClientError> {
        let options = if allow_loopback {
            EgressOptions::default().with_policy(AddressPolicy::AllowLoopback)
        } else {
            EgressOptions::default()
        };
        Ok(Self {
            client: GuardedClient::new(options)?,
        })
    }
    async fn signed(
        &self,
        peer: &ActivePeer,
        path: &str,
        call: SuiteSignedCall<'_>,
    ) -> Result<SuitePeerResponse, SuitePeerError> {
        let mut headers = HeaderMap::new();
        for (key, value) in [
            ("content-type", "application/json".to_owned()),
            (
                "x-suite-signature",
                sign_webhook_hmac_sha256(call.secret, call.timestamp, call.delivery_id, call.body),
            ),
            ("x-suite-timestamp", call.timestamp.to_string()),
            ("x-suite-delivery-id", call.delivery_id.to_owned()),
            ("x-suite-source", call.source_app.to_owned()),
        ] {
            headers.insert(
                HeaderName::from_static(key),
                HeaderValue::from_str(&value).map_err(|_| SuitePeerError::InvalidResponse)?,
            );
        }
        if call.replay {
            headers.insert("x-suite-replay", HeaderValue::from_static("1"));
        }
        let request = EgressRequest::new(Method::POST, endpoint(peer, path)?)
            .with_headers(headers)
            .with_body(call.body.to_vec())
            .with_response_body(ResponseBody::Discard);
        match self.client.execute(request).await {
            Ok(response) => Ok(SuitePeerResponse {
                status: response.status().as_u16(),
                retry_after: None,
            }),
            Err(EgressError::Status { status, class }) => Ok(SuitePeerResponse {
                status: status.as_u16(),
                retry_after: class.retry_after(),
            }),
            Err(e) => Err(peer_error(e)),
        }
    }
}
#[async_trait]
impl SuitePeerClient for ReqwestSuitePeerClient {
    async fn exchange(
        &self,
        peer: &ActivePeer,
        input: &ExchangeRequest,
    ) -> Result<ExchangeResponse, SuitePeerError> {
        let mut headers = HeaderMap::new();
        headers.insert("content-type", HeaderValue::from_static("application/json"));
        let response = self
            .client
            .execute(
                EgressRequest::new(Method::POST, endpoint(peer, "/suite/links/exchange")?)
                    .with_headers(headers)
                    .with_body(
                        serde_json::to_vec(input).map_err(|_| SuitePeerError::InvalidResponse)?,
                    ),
            )
            .await
            .map_err(peer_error)?;
        if response.status().as_u16() != 201 {
            return Err(SuitePeerError::InvalidResponse);
        }
        serde_json::from_slice(response.body()).map_err(|_| SuitePeerError::InvalidResponse)
    }
    async fn revoke(
        &self,
        peer: &ActivePeer,
        call: SuiteSignedCall<'_>,
    ) -> Result<SuitePeerResponse, SuitePeerError> {
        let path = format!("/suite/links/{}/revoke", call.remote_link_id);
        self.signed(peer, &path, call).await
    }
    async fn deliver(
        &self,
        peer: &ActivePeer,
        call: SuiteSignedCall<'_>,
    ) -> Result<SuitePeerResponse, SuitePeerError> {
        let path = format!("/suite/inbound/{}", call.remote_link_id);
        self.signed(peer, &path, call).await
    }
}
fn endpoint(peer: &ActivePeer, path: &str) -> Result<Url, SuitePeerError> {
    let mut url = Url::parse(&peer.api_url).map_err(|_| SuitePeerError::InvalidResponse)?;
    url.set_path(&format!("{}{}", url.path().trim_end_matches('/'), path));
    Ok(url)
}
fn peer_error(e: EgressError) -> SuitePeerError {
    match e {
        EgressError::Timeout => SuitePeerError::Timeout,
        EgressError::Resolve => SuitePeerError::Dns,
        EgressError::Status { status, .. } => SuitePeerError::Rejected {
            status: status.as_u16(),
            code: "suite_peer_rejected".to_owned(),
        },
        EgressError::Transport(_) => SuitePeerError::Transport,
        _ => SuitePeerError::InvalidResponse,
    }
}
