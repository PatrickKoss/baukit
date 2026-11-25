#![no_main]

use std::sync::LazyLock;

use aws_lc_rs::signature::{ED25519, Ed25519KeyPair, UnparsedPublicKey};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use baukit_data_subject::{
    DiscoveryDocument, DsseEnvelope, DsseSignature, RECEIPT_PAYLOAD_TYPE, ReceiptPayload,
    ReceiptVerifier, pae,
};
use libfuzzer_sys::fuzz_target;

const DISCOVERY: &[u8] = include_bytes!("../../../fixtures/data-subject/v1/discovery.json");
const PRIVATE_KEY: &[u8] =
    include_bytes!("../../../fixtures/data-subject/v1/test-only-private.der");
const PUBLIC_KEY: &[u8] = include_bytes!("../../../fixtures/data-subject/v1/test-only-public.der");
const SPKI_KEY_OFFSET: usize = 12;

static DOCUMENT: LazyLock<DiscoveryDocument> =
    LazyLock::new(|| serde_json::from_slice(DISCOVERY).expect("discovery fixture"));
static VERIFIER: LazyLock<ReceiptVerifier> =
    LazyLock::new(|| ReceiptVerifier::from_discovery(&DOCUMENT).expect("fixture keys"));
static PAIR: LazyLock<Ed25519KeyPair> =
    LazyLock::new(|| Ed25519KeyPair::from_pkcs8(PRIVATE_KEY).expect("test-only key"));

fn signed(payload: &[u8]) -> DsseEnvelope {
    DsseEnvelope {
        payload_type: RECEIPT_PAYLOAD_TYPE.into(),
        payload: STANDARD.encode(payload),
        signatures: vec![DsseSignature {
            keyid: DOCUMENT.receipt_keys[0].key_id.clone(),
            sig: STANDARD.encode(PAIR.sign(&pae(RECEIPT_PAYLOAD_TYPE, payload)).as_ref()),
        }],
    }
}

fn check(envelope: &DsseEnvelope) {
    let Ok(accepted) = VERIFIER.verify(envelope) else {
        return;
    };
    let payload = STANDARD
        .decode(&envelope.payload)
        .expect("accepted payload is base64");
    let input = pae(&envelope.payload_type, &payload);
    let key = UnparsedPublicKey::new(&ED25519, &PUBLIC_KEY[SPKI_KEY_OFFSET..]);
    assert!(envelope.signatures.iter().any(|signature| {
        signature.keyid == DOCUMENT.receipt_keys[0].key_id
            && STANDARD
                .decode(&signature.sig)
                .is_ok_and(|sig| key.verify(&input, &sig).is_ok())
    }));
    let reparsed: ReceiptPayload = serde_json::from_slice(&payload).expect("payload re-parses");
    assert_eq!(reparsed, accepted);
    accepted
        .canonical_bytes()
        .expect("accepted payload re-validates");
}

fuzz_target!(|data: &[u8]| {
    let Some((&mode, rest)) = data.split_first() else {
        return;
    };
    if mode & 1 == 0 {
        check(&signed(rest));
    } else if let Ok(envelope) = serde_json::from_slice::<DsseEnvelope>(rest) {
        check(&envelope);
    }
});
