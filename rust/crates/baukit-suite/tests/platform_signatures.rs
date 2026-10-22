#![cfg(feature = "platform")]

use baukit_core::webhook_signature::{sign_webhook_hmac_sha256, verify_webhook_hmac_sha256};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Vector {
    secret: String,
    timestamp: i64,
    delivery_id: String,
    body: String,
    headers: BTreeMap<String, String>,
}
#[derive(Deserialize)]
struct Vectors {
    vectors: Vec<Vector>,
}

#[test]
fn platform_signature_vectors_preserve_raw_utf8_and_empty_bodies() {
    let vectors: Vectors = serde_json::from_str(include_str!(
        "../../../../fixtures/platform-events/v1/signature-vectors.json"
    ))
    .expect("signature fixture");
    assert!(!vectors.vectors.is_empty());
    for v in vectors.vectors {
        let signature = &v.headers["x-suite-signature"];
        assert_eq!(
            &sign_webhook_hmac_sha256(
                v.secret.as_bytes(),
                v.timestamp,
                &v.delivery_id,
                v.body.as_bytes()
            ),
            signature
        );
        assert!(verify_webhook_hmac_sha256(
            [v.secret.as_bytes()],
            v.timestamp,
            &v.delivery_id,
            v.body.as_bytes(),
            signature
        ));
        assert!(!verify_webhook_hmac_sha256(
            [v.secret.as_bytes()],
            v.timestamp + 1,
            &v.delivery_id,
            v.body.as_bytes(),
            signature
        ));
    }
}

use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use baukit_config::Secret;
use baukit_suite::platform::*;
use chrono::{DateTime, Utc};
use std::sync::Arc;
use tower::ServiceExt;

struct Clock(i64);
impl PlatformClock for Clock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(self.0, 0).expect("time")
    }
}
struct Handler;
#[async_trait]
impl PlatformEventHandler for Handler {
    async fn handle(
        &self,
        _: &PlatformEvent,
        _: &mut EventContext<'_>,
    ) -> Result<(), PlatformError> {
        Ok(())
    }
}
fn fixture_request(vector: &Vector) -> Request<Body> {
    let mut request = Request::post("/platform/inbound");
    for (name, value) in &vector.headers {
        request = request.header(name, value);
    }
    request
        .body(Body::from(vector.body.clone()))
        .expect("request")
}
#[tokio::test]
async fn router_verifies_fixture_headers_and_rejects_malformed_delivery_headers() {
    let vectors: Vectors = serde_json::from_str(include_str!(
        "../../../../fixtures/platform-events/v1/signature-vectors.json"
    ))
    .expect("vectors");
    for (index, vector) in vectors.vectors.iter().enumerate() {
        let receiver = PlatformReceiver::new(
            PlatformEventDecoder::new(vector.headers["x-suite-source"].clone(), None)
                .expect("decoder"),
            Secret::new(vector.secret.clone()),
            None,
            Arc::new(InMemoryPlatformInbox::default()),
            Arc::new(Handler),
        )
        .expect("receiver")
        .with_clock(Arc::new(Clock(vector.timestamp)));
        let app = router(receiver);
        let response = app
            .clone()
            .oneshot(fixture_request(vector))
            .await
            .expect("response");
        assert_eq!(
            response.status(),
            if index == 0 {
                StatusCode::NO_CONTENT
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            }
        );
        for (name, value) in [
            ("x-suite-source", "other".to_owned()),
            ("x-suite-timestamp", format!("0{}", vector.timestamp)),
            ("x-suite-delivery-id", String::new()),
            ("x-suite-delivery-id", "a".repeat(257)),
            ("x-suite-delivery-id", "changed-attempt".to_owned()),
        ] {
            let mut request = fixture_request(vector);
            request
                .headers_mut()
                .insert(name, value.parse().expect("header"));
            assert_eq!(
                app.clone()
                    .oneshot(request)
                    .await
                    .expect("response")
                    .status(),
                StatusCode::UNAUTHORIZED,
                "{name}"
            );
        }
        let mut changed_body = fixture_request(vector);
        *changed_body.body_mut() = Body::from(format!("{} ", vector.body));
        assert_eq!(
            app.oneshot(changed_body).await.expect("response").status(),
            StatusCode::UNAUTHORIZED
        );
    }
}
