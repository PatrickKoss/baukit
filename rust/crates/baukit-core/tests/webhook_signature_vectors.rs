//! Runs the shared vectors in `fixtures/webhooks/signature-v1.json`.

use baukit_core::webhook_signature::{
    sign_webhook_hmac_sha256, verify_webhook_hmac_sha256, webhook_signing_input,
};
use serde::Deserialize;

const VECTORS: &str = include_str!("../../../../fixtures/webhooks/signature-v1.json");
const HEX_RADIX: u32 = 16;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Vectors {
    version: u32,
    signing_cases: Vec<SigningCase>,
    verification_cases: Vec<VerificationCase>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SigningCase {
    name: String,
    secret: String,
    timestamp: i64,
    delivery_id: String,
    body: Option<String>,
    body_hex: Option<String>,
    signing_input_hex: String,
    signature: String,
}

impl SigningCase {
    fn body(&self) -> Vec<u8> {
        match (&self.body, &self.body_hex) {
            (Some(body), None) => body.as_bytes().to_vec(),
            (None, Some(hex)) => decode_hex(hex),
            _ => panic!("{} needs exactly one of body and bodyHex", self.name),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VerificationCase {
    name: String,
    candidate_secrets: Vec<String>,
    timestamp: i64,
    delivery_id: String,
    body: String,
    signature: String,
    expected: bool,
}

fn vectors() -> Vectors {
    let vectors: Vectors = serde_json::from_str(VECTORS).expect("vectors should parse");
    assert_eq!(vectors.version, 1);
    vectors
}

fn decode_hex(hex: &str) -> Vec<u8> {
    hex.as_bytes()
        .chunks(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("hex should be ASCII");
            u8::from_str_radix(pair, HEX_RADIX).expect("hex should decode")
        })
        .collect()
}

#[test]
fn signing_cases_match_the_published_bytes_and_signatures() {
    for case in vectors().signing_cases {
        let body = case.body();
        assert_eq!(
            webhook_signing_input(case.timestamp, &case.delivery_id, &body),
            decode_hex(&case.signing_input_hex),
            "{}",
            case.name
        );
        assert_eq!(
            sign_webhook_hmac_sha256(
                case.secret.as_bytes(),
                case.timestamp,
                &case.delivery_id,
                &body
            ),
            case.signature,
            "{}",
            case.name
        );
    }
}

#[test]
fn verification_cases_match_the_expected_result() {
    for case in vectors().verification_cases {
        let verified = verify_webhook_hmac_sha256(
            case.candidate_secrets.iter().map(String::as_bytes),
            case.timestamp,
            &case.delivery_id,
            case.body.as_bytes(),
            &case.signature,
        );
        assert_eq!(verified, case.expected, "{}", case.name);
    }
}
