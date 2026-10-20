use std::{fs, path::PathBuf};

use aws_lc_rs::{
    digest::{SHA256, digest},
    signature::Ed25519KeyPair,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use baukit_data_subject::*;
use serde::Deserialize;
use serde_json::json;

fn fixture(name: &str) -> Vec<u8> {
    fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../fixtures/data-subject/v1")
            .join(name),
    )
    .expect("read runtime fixture")
}

fn discovery() -> DiscoveryDocument {
    serde_json::from_slice(&fixture("discovery.json")).expect("discovery fixture")
}

fn signer() -> ReceiptSigner {
    ReceiptSigner::from_pkcs8_der(&fixture("test-only-private.der")).expect("test-only signing key")
}

fn payload() -> ReceiptPayload {
    serde_json::from_slice(&fixture("completed.payload.json")).expect("payload fixture")
}

fn envelope() -> DsseEnvelope {
    serde_json::from_slice(&fixture("completed.envelope.json")).expect("envelope fixture")
}

fn sign_raw(body: &[u8]) -> DsseEnvelope {
    let pair =
        Ed25519KeyPair::from_pkcs8(&fixture("test-only-private.der")).expect("test-only key");
    DsseEnvelope {
        payload_type: RECEIPT_PAYLOAD_TYPE.into(),
        payload: STANDARD.encode(body),
        signatures: vec![DsseSignature {
            keyid: discovery().receipt_keys[0].key_id.clone(),
            sig: STANDARD.encode(pair.sign(&pae(RECEIPT_PAYLOAD_TYPE, body)).as_ref()),
        }],
    }
}

#[derive(Deserialize)]
struct Case {
    file: String,
    result: String,
    canonical: bool,
}

#[test]
fn runtime_vectors_match_labels_and_signer_reproduces_exact_envelope_bytes() {
    let verifier = ReceiptVerifier::from_discovery(&discovery()).expect("fixture keys");
    let cases: Vec<Case> = serde_json::from_slice(&fixture("cases.json")).expect("case manifest");
    assert_eq!(cases.len(), 8);
    let mut counts = [0; 4];
    for case in cases {
        let bytes = fixture(&case.file);
        let envelope: DsseEnvelope = serde_json::from_slice(&bytes).expect("case envelope");
        match case.result.as_str() {
            "accepted" => {
                counts[0] += 1;
                let verified = verifier
                    .verify(&envelope)
                    .expect("positive vector verifies");
                let stem = case.file.trim_end_matches(".envelope.json");
                assert_eq!(
                    STANDARD.decode(&envelope.payload).expect("base64"),
                    fixture(&format!("{stem}.payload.json"))
                );
                assert_eq!(
                    pae(
                        &envelope.payload_type,
                        &STANDARD.decode(&envelope.payload).expect("base64")
                    ),
                    fixture(&format!("{stem}.pae"))
                );
                if case.canonical {
                    let signed = signer().sign(&verified).expect("sign canonical fixture");
                    let mut generated = serde_json::to_vec(&signed).expect("serialize envelope");
                    generated.push(b'\n');
                    assert_eq!(generated, bytes);
                    assert_eq!(
                        verified.canonical_bytes().expect("canonical payload"),
                        fixture(&format!("{stem}.payload.json"))
                    );
                } else {
                    assert_ne!(
                        verified.canonical_bytes().expect("canonical payload"),
                        STANDARD.decode(&envelope.payload).expect("base64")
                    );
                }
            }
            "signature" => {
                counts[1] += 1;
                assert_eq!(verifier.verify(&envelope), Err(ReceiptError::Signature));
            }
            "payloadType" => {
                counts[2] += 1;
                assert_eq!(verifier.verify(&envelope), Err(ReceiptError::PayloadType));
            }
            "unknownKey" => {
                counts[3] += 1;
                assert_eq!(verifier.verify(&envelope), Err(ReceiptError::UnknownKey));
            }
            _ => panic!("unrecognized fixture result label"),
        }
    }
    assert_eq!(counts, [5, 1, 1, 1]);
}

#[test]
fn key_id_hashes_spki_der_and_pem_and_der_signers_match() {
    let pem_signer = ReceiptSigner::from_pkcs8_pem(
        std::str::from_utf8(&fixture("test-only-private.pem")).expect("PEM text"),
    )
    .expect("PEM signer");
    let der_signer = signer();
    let key = der_signer.receipt_key();
    assert_eq!(key, &discovery().receipt_keys[0]);
    assert_eq!(
        key.public_key_pem.as_bytes(),
        fixture("test-only-public.pem")
    );
    let expected: String = digest(&SHA256, &fixture("test-only-public.der"))
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(key.key_id, expected);
    let decoded = pem::parse(&key.public_key_pem).expect("public PEM");
    assert_ne!(
        digest(&SHA256, &decoded.contents()[12..]).as_ref(),
        digest(&SHA256, decoded.contents()).as_ref()
    );
    assert_eq!(
        pem_signer.sign(&payload()).expect("PEM signature"),
        der_signer.sign(&payload()).expect("DER signature")
    );
}

#[test]
fn pae_pins_spacing_empty_payload_and_byte_lengths() {
    assert_eq!(pae("", b""), b"DSSEv1 0  0 ");
    assert_eq!(pae("text/plain", b"hello"), b"DSSEv1 10 text/plain 5 hello");
    assert_eq!(pae("é", "雪".as_bytes()), "DSSEv1 2 é 3 雪".as_bytes());
    assert_eq!(pae("t", b"\0 \n"), b"DSSEv1 1 t 3 \0 \n");
}

#[test]
fn malformed_and_wrong_kind_private_keys_are_safe_errors() {
    for der in [Vec::new(), vec![0; 32], fixture("test-only-public.der")] {
        assert!(matches!(
            ReceiptSigner::from_pkcs8_der(&der),
            Err(ReceiptError::SigningKey)
        ));
    }
    for pem in [
        "PRIVATE-FIXTURE-MARKER".to_owned(),
        String::from_utf8(fixture("test-only-public.pem")).expect("public PEM"),
        pem::encode(&pem::Pem::new("PRIVATE KEY", vec![0; 32])),
    ] {
        let error = ReceiptSigner::from_pkcs8_pem(&pem)
            .err()
            .expect("reject invalid private key");
        assert_eq!(error, ReceiptError::SigningKey);
        assert!(!error.to_string().contains("PRIVATE-FIXTURE-MARKER"));
    }
}

#[test]
fn discovery_rejects_bad_version_key_ids_and_public_keys() {
    let mut bad = discovery();
    bad.contract_version = "2".into();
    assert!(matches!(
        ReceiptVerifier::from_discovery(&bad),
        Err(ReceiptError::ContractVersion)
    ));
    for mutation in 0..7 {
        let mut keys = discovery().receipt_keys;
        match mutation {
            0 => keys[0].key_id = "0".repeat(64),
            1 => keys[0].key_id = keys[0].key_id.to_uppercase(),
            2 => keys[0].public_key_pem = "PRIVATE-FIXTURE-MARKER".into(),
            3 => {
                keys[0].public_key_pem =
                    String::from_utf8(fixture("test-only-private.pem")).expect("PEM")
            }
            4 => keys[0].public_key_pem = pem::encode(&pem::Pem::new("PUBLIC KEY", vec![0; 32])),
            5 => keys[0].public_key_pem = pem::encode(&pem::Pem::new("PUBLIC KEY", vec![0; 44])),
            _ => {
                keys[0].public_key_pem = pem::encode(&pem::Pem::new(
                    "CERTIFICATE",
                    fixture("test-only-public.der"),
                ))
            }
        }
        assert!(matches!(
            ReceiptVerifier::from_receipt_keys(&keys),
            Err(ReceiptError::ReceiptKeys)
        ));
    }
}

#[test]
fn discovery_requires_unique_ids_and_exactly_one_current_key() {
    let current = discovery().receipt_keys.remove(0);
    let mut previous = current.clone();
    previous.status = ReceiptKeyStatus::Previous;
    for keys in [
        vec![],
        vec![previous.clone()],
        vec![current.clone(), current.clone()],
        vec![current, previous],
    ] {
        assert!(matches!(
            ReceiptVerifier::from_receipt_keys(&keys),
            Err(ReceiptError::ReceiptKeys)
        ));
    }
}

#[test]
fn rotation_keeps_old_receipts_verifiable() {
    let old = signer();
    let fresh_pair = Ed25519KeyPair::generate().expect("test key");
    let fresh = ReceiptSigner::from_pkcs8_der(fresh_pair.to_pkcs8().expect("PKCS8 v2").as_ref())
        .expect("fresh signer");
    let mut old_key = old.receipt_key().clone();
    old_key.status = ReceiptKeyStatus::Previous;
    let verifier = ReceiptVerifier::from_receipt_keys(&[fresh.receipt_key().clone(), old_key])
        .expect("rotated keys");
    let body = payload();
    assert_eq!(
        verifier.verify(&old.sign(&body).expect("old signature")),
        Ok(body.clone())
    );
    assert_eq!(
        verifier.verify(&fresh.sign(&body).expect("new signature")),
        Ok(body)
    );
    let fresh_only =
        ReceiptVerifier::from_receipt_keys(&[fresh.receipt_key().clone()]).expect("fresh key");
    assert_eq!(
        fresh_only.verify(&envelope()),
        Err(ReceiptError::UnknownKey)
    );
}

#[test]
fn envelope_rejects_empty_invalid_and_unknown_signatures() {
    let verifier = ReceiptVerifier::from_discovery(&discovery()).expect("keys");
    let mut signed = envelope();
    signed.signatures.clear();
    assert_eq!(verifier.verify(&signed), Err(ReceiptError::Signature));
    signed = envelope();
    signed.payload = "_not+base64".into();
    assert_eq!(verifier.verify(&signed), Err(ReceiptError::Encoding));
    signed = envelope();
    signed.signatures[0].sig = "_not+base64".into();
    assert_eq!(verifier.verify(&signed), Err(ReceiptError::Encoding));
    signed = envelope();
    signed.signatures[0].sig = STANDARD.encode([0; 63]);
    assert_eq!(verifier.verify(&signed), Err(ReceiptError::Signature));
    signed = envelope();
    signed.signatures[0].sig = STANDARD.encode([0; 64]);
    assert_eq!(verifier.verify(&signed), Err(ReceiptError::Signature));
    signed = envelope();
    signed.signatures.push(DsseSignature {
        keyid: "0".repeat(64),
        sig: STANDARD.encode([0; 64]),
    });
    assert_eq!(verifier.verify(&signed), Err(ReceiptError::UnknownKey));
    signed = envelope();
    signed.signatures.push(DsseSignature {
        keyid: signed.signatures[0].keyid.clone(),
        sig: STANDARD.encode([0; 64]),
    });
    assert_eq!(verifier.verify(&signed), Ok(payload()));
}

#[test]
fn signed_payload_markers_state_and_timestamps_are_checked_after_verification() {
    let verifier = ReceiptVerifier::from_discovery(&discovery()).expect("keys");
    for status in [OperationStatus::Pending, OperationStatus::Failed] {
        let mut body = payload();
        body.status = status;
        assert_eq!(signer().sign(&body), Err(ReceiptError::Payload));
        assert_eq!(
            verifier.verify(&sign_raw(&serde_json::to_vec(&body).expect("JSON"))),
            Err(ReceiptError::Payload)
        );
    }
    for (field, value, expected) in [
        ("receiptType", json!("other"), ReceiptError::Payload),
        ("contractVersion", json!("2"), ReceiptError::ContractVersion),
        ("subjectReference", json!("short"), ReceiptError::Payload),
        (
            "completedAt",
            json!("2026-10-19T10:00:00Z"),
            ReceiptError::Payload,
        ),
        ("operationId", json!("not-a-uuid"), ReceiptError::Payload),
        ("createdAt", json!("not-a-timestamp"), ReceiptError::Payload),
    ] {
        let mut body = serde_json::to_value(payload()).expect("JSON value");
        body[field] = value;
        assert_eq!(
            verifier.verify(&sign_raw(&serde_json::to_vec(&body).expect("JSON"))),
            Err(expected)
        );
    }
    for bytes in [b"not JSON".as_slice(), b"{}", b"\xff"] {
        assert_eq!(
            verifier.verify(&sign_raw(bytes)),
            Err(ReceiptError::Payload)
        );
    }
    let mut body = payload();
    body.receipt_type = "other".into();
    assert_eq!(signer().sign(&body), Err(ReceiptError::Payload));
    body = payload();
    body.contract_version = "2".into();
    assert_eq!(signer().sign(&body), Err(ReceiptError::ContractVersion));
}

#[test]
fn export_part_metadata_matches_exact_fixture_bytes() {
    let verifier = ReceiptVerifier::from_discovery(&discovery()).expect("keys");
    let signed: DsseEnvelope =
        serde_json::from_slice(&fixture("export.envelope.json")).expect("export envelope");
    let payload = verifier.verify(&signed).expect("export receipt");
    assert_eq!(payload.parts.len(), 2);
    for part in payload.parts {
        let bytes = fixture(&format!("export-part-{}.json", part.part_number));
        assert_eq!(part.bytes, bytes.len() as u64);
        assert!(part.bytes <= discovery().limits.max_export_part_bytes);
        let hash: String = digest(&SHA256, &bytes)
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(part.sha256, hash);
        assert!(serde_json::from_slice::<serde_json::Value>(&bytes).is_ok());
    }
}
