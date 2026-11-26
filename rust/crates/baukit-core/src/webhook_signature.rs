//! The `baukit-webhook-v1` HMAC-SHA256 signature for outbound webhooks.
//!
//! A sender signs the delivery with [`sign_webhook_hmac_sha256`] and sends the
//! timestamp, the delivery ID, and the signature in headers the product names.
//! A receiver checks them with [`verify_webhook_hmac_sha256`] against the
//! current and retained rotation keys. `fixtures/webhooks/signature-v1.json`
//! pins the signing bytes, signatures, and verification results.

use aws_lc_rs::hmac;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

/// The version line that starts every signing input, without its newline.
pub const WEBHOOK_SIGNATURE_VERSION: &str = "baukit-webhook-v1";

/// The prefix of every signature value.
pub const WEBHOOK_SIGNATURE_PREFIX: &str = "v1=";

const FIELD_SEPARATOR: u8 = b'\n';

/// Builds the exact bytes covered by the signature.
///
/// The input is the version line, the decimal Unix timestamp, the decimal byte
/// length of the delivery ID, the delivery ID, and the raw request body. Each
/// field before the body ends with `\n`. The length makes the two variable
/// fields unambiguous.
#[must_use]
pub fn webhook_signing_input(timestamp: i64, delivery_id: &str, body: &[u8]) -> Vec<u8> {
    let timestamp = timestamp.to_string();
    let length = delivery_id.len().to_string();
    let mut input = Vec::new();
    for field in [
        WEBHOOK_SIGNATURE_VERSION.as_bytes(),
        timestamp.as_bytes(),
        length.as_bytes(),
        delivery_id.as_bytes(),
    ] {
        input.extend_from_slice(field);
        input.push(FIELD_SEPARATOR);
    }
    input.extend_from_slice(body);
    input
}

/// Signs one delivery with HMAC-SHA256.
///
/// The value starts with `v1=` followed by the unpadded base64url tag. Every
/// retry of one delivery must reuse the same timestamp, delivery ID, and body,
/// so the receiver sees the same signature. The function does not retain the
/// secret.
#[must_use]
pub fn sign_webhook_hmac_sha256(
    secret: &[u8],
    timestamp: i64,
    delivery_id: &str,
    body: &[u8],
) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
    let tag = hmac::sign(&key, &webhook_signing_input(timestamp, delivery_id, body));
    format!(
        "{WEBHOOK_SIGNATURE_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(tag.as_ref())
    )
}

/// Verifies a signature against each candidate secret in constant time.
///
/// Pass the current key and any rotation key still inside its overlap. A
/// missing prefix, padded or non-base64url encoding, or a truncated tag returns
/// `false`. The function checks no timestamp window and no delivery ID reuse;
/// both are receiver policy.
#[must_use]
pub fn verify_webhook_hmac_sha256<'a>(
    candidate_secrets: impl IntoIterator<Item = &'a [u8]>,
    timestamp: i64,
    delivery_id: &str,
    body: &[u8],
    signature: &str,
) -> bool {
    let Some(encoded) = signature.strip_prefix(WEBHOOK_SIGNATURE_PREFIX) else {
        return false;
    };
    let Ok(supplied) = URL_SAFE_NO_PAD.decode(encoded) else {
        return false;
    };
    let input = webhook_signing_input(timestamp, delivery_id, body);
    candidate_secrets.into_iter().any(|secret| {
        let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
        hmac::verify(&key, &input, &supplied).is_ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_accepts_the_previous_key_and_rejects_a_changed_timestamp() {
        let body = br#"{"event":"created"}"#;
        let signature =
            sign_webhook_hmac_sha256(b"current-secret", 1_800_000_000, "delivery-7", body);

        assert_eq!(signature, "v1=UpNJdPkf1wS7p7DY75L8nz7Rz_BUPFFlEOX3ma4py7w");
        assert!(verify_webhook_hmac_sha256(
            [b"previous-secret".as_slice(), b"current-secret".as_slice()],
            1_800_000_000,
            "delivery-7",
            body,
            &signature,
        ));
        assert!(!verify_webhook_hmac_sha256(
            [b"previous-secret".as_slice()],
            1_800_000_000,
            "delivery-7",
            body,
            &signature,
        ));
        assert!(!verify_webhook_hmac_sha256(
            [b"current-secret".as_slice()],
            1_800_000_001,
            "delivery-7",
            body,
            &signature,
        ));
    }

    mod properties {
        use proptest::prelude::*;

        use super::super::*;

        const SECRET: &[u8] = b"property-secret";

        fn flip(bytes: &mut [u8], bit: usize) {
            let index = bit / 8 % bytes.len();
            bytes[index] ^= 1 << (bit % 8);
        }

        proptest! {
            #[test]
            fn signing_input_is_injective(
                left in (any::<i64>(), ".{0,24}", prop::collection::vec(any::<u8>(), 0..32)),
                right in (any::<i64>(), ".{0,24}", prop::collection::vec(any::<u8>(), 0..32)),
            ) {
                prop_assume!(left != right);
                prop_assert_ne!(
                    webhook_signing_input(left.0, &left.1, &left.2),
                    webhook_signing_input(right.0, &right.1, &right.2)
                );
            }

            #[test]
            fn any_bit_flip_fails_verification(
                timestamp in any::<i64>(),
                id in "[ -~]{1,24}",
                body in prop::collection::vec(any::<u8>(), 1..64),
                bit in any::<usize>(),
                target in 0_u8..4,
            ) {
                let signature = sign_webhook_hmac_sha256(SECRET, timestamp, &id, &body);
                prop_assert!(verify_webhook_hmac_sha256([SECRET], timestamp, &id, &body, &signature));
                let (mut timestamp, mut id, mut body, mut signature) =
                    (timestamp, id.into_bytes(), body, signature.into_bytes());
                match target {
                    0 => timestamp ^= 1_i64 << (bit % 64),
                    1 => flip(&mut id, bit),
                    2 => flip(&mut body, bit),
                    _ => flip(&mut signature, bit),
                }
                let id = String::from_utf8_lossy(&id).into_owned();
                let signature = String::from_utf8_lossy(&signature).into_owned();
                prop_assert!(!verify_webhook_hmac_sha256([SECRET], timestamp, &id, &body, &signature));
            }
        }
    }
}
