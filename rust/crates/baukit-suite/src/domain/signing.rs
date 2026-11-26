//! Suite signature v2 binds the `x-suite-replay` flag, which the v1
//! `baukit-webhook-v1` signature leaves unsigned.

use baukit_core::webhook_signature::{
    WEBHOOK_SIGNATURE_PREFIX, sign_webhook_hmac_sha256, verify_webhook_hmac_sha256,
};

/// Carries the v2 signature next to `x-suite-signature`, so v1 receivers keep working.
pub const SUITE_SIGNATURE_V2_HEADER: &str = "x-suite-signature-v2";
const SIGNATURE_V2_PREFIX: &str = "v2=";
const REPLAY_FIELD: &[u8] = b"baukit-suite-v2\nreplay=";

/// The bytes v2 hands to the v1 signer in place of the body: a version line and
/// the replay flag ahead of the raw body. Suite bodies are JSON objects, so no
/// v1 body starts with this prefix.
pub fn suite_v2_signed_body(replay: bool, body: &[u8]) -> Vec<u8> {
    let mut input = REPLAY_FIELD.to_vec();
    input.push(if replay { b'1' } else { b'0' });
    input.push(b'\n');
    input.extend_from_slice(body);
    input
}

pub fn sign_suite_v2(
    secret: &[u8],
    timestamp: i64,
    delivery_id: &str,
    replay: bool,
    body: &[u8],
) -> String {
    let v1 = sign_webhook_hmac_sha256(
        secret,
        timestamp,
        delivery_id,
        &suite_v2_signed_body(replay, body),
    );
    let tag = v1.strip_prefix(WEBHOOK_SIGNATURE_PREFIX).unwrap_or(&v1);
    format!("{SIGNATURE_V2_PREFIX}{tag}")
}

pub fn verify_suite_v2(
    secret: &[u8],
    timestamp: i64,
    delivery_id: &str,
    replay: bool,
    body: &[u8],
    signature: &str,
) -> bool {
    let Some(tag) = signature.strip_prefix(SIGNATURE_V2_PREFIX) else {
        return false;
    };
    verify_webhook_hmac_sha256(
        [secret],
        timestamp,
        delivery_id,
        &suite_v2_signed_body(replay, body),
        &format!("{WEBHOOK_SIGNATURE_PREFIX}{tag}"),
    )
}
