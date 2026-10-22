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
