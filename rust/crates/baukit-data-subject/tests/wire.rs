use std::{fmt::Debug, fs, path::Path};

use baukit_data_subject::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

fn round_trip<T>(expected: Value) -> T
where
    T: DeserializeOwned + Serialize + PartialEq + Debug,
{
    let parsed: T = serde_json::from_value(expected.clone()).expect("parse wire fields");
    assert_eq!(
        serde_json::to_value(&parsed).expect("serialize wire fields"),
        expected
    );
    let encoded = serde_json::to_vec(&parsed).expect("encode");
    let decoded: T = serde_json::from_slice(&encoded).expect("decode");
    assert_eq!(parsed, decoded);
    parsed
}

#[test]
fn discovery_and_nested_fields_match_the_contract() {
    let discovery = round_trip::<DiscoveryDocument>(json!({
        "contractVersion": "1",
        "service": {"name": "fixture", "version": "1.0.0"},
        "operations": ["erasure", "export"],
        "identifierTypes": ["email", "accountId", "customerId", "phone", "x-fixture-id"],
        "scopes": {"erasure": "data-subject.erase", "export": "data-subject.export"},
        "limits": {"maxIdentifiers": 16, "maxIdentifierLength": 256, "maxExportPartBytes": 8388608},
        "receiptKeys": [{"keyId": "fixture-key-id", "algorithm": "ed25519",
            "publicKeyPem": "fixture-public-key", "status": "current"}]
    }));
    assert_eq!(discovery.limits, Limits::default());
    assert_eq!(discovery.scopes, Scopes::default());
    round_trip::<Service>(json!({"name":"fixture", "version":"1"}));
    round_trip::<Scopes>(json!({"erasure":"data-subject.erase", "export":"data-subject.export"}));
    round_trip::<Limits>(
        json!({"maxIdentifiers":16, "maxIdentifierLength":256, "maxExportPartBytes":8388608}),
    );
    round_trip::<ReceiptKey>(
        json!({"keyId":"fixture-key-id", "algorithm":"ed25519", "publicKeyPem":"fixture-public-key", "status":"previous"}),
    );
}

#[test]
fn identifiers_and_request_fields_match_the_contract() {
    round_trip::<Identifier>(json!({"type":"accountId", "value":"synthetic-fixture"}));
    round_trip::<DataSubjectRequest>(
        json!({"subjectReference":"fixture-reference-01", "identifiers":[{"type":"email", "value":"fixture@example.invalid"}]}),
    );
    for name in ["email", "accountId", "customerId", "phone", "x-fixture-id"] {
        assert_eq!(round_trip::<IdentifierType>(json!(name)).as_str(), name);
    }
}

#[test]
fn category_restriction_export_and_error_fields_match_the_contract() {
    round_trip::<CategoryEntry>(json!({"category":"records", "action":"erased", "count":3}));
    round_trip::<Restriction>(
        json!({"category":"invoices", "reason":"Statutory retention", "legalBasis":"Fixture obligation", "retainUntil":"2030-12-31"}),
    );
    round_trip::<Restriction>(
        json!({"category":"invoices", "reason":"Statutory retention", "legalBasis":"Fixture obligation", "retainUntil":null}),
    );
    round_trip::<ExportPart>(json!({"partNumber":1, "sha256":"00".repeat(32), "bytes":42}));
    round_trip::<OperationError>(json!({"code":"fixture_failure"}));
    assert!(serde_json::from_value::<Restriction>(json!({"category":"invoices", "reason":"fixture", "legalBasis":"fixture", "retainUntil":"2030-02-30"})).is_err());
}

#[test]
fn pending_resource_preserves_nulls_and_empty_arrays() {
    round_trip::<OperationResource>(json!({
        "operationId":"00000000-0000-4000-8000-000000000001",
        "kind":"erasure", "status":"pending", "subjectReference":"fixture-reference-01",
        "createdAt":"2026-10-20T10:00:00Z", "completedAt":null,
        "categories":[], "restrictions":[], "parts":[], "receipt":null, "error":null
    }));
}

#[test]
fn terminal_resource_and_dsse_fields_match_the_contract() {
    let envelope = json!({"payloadType":RECEIPT_PAYLOAD_TYPE, "payload":"e30=", "signatures":[{"keyid":"fixture-key-id", "sig":"AA=="}]});
    round_trip::<DsseEnvelope>(envelope.clone());
    round_trip::<DsseSignature>(json!({"keyid":"fixture-key-id", "sig":"AA=="}));
    let mut resource = json!({
        "operationId":"00000000-0000-4000-8000-000000000001",
        "kind":"export", "status":"completed", "subjectReference":"fixture-reference-01",
        "createdAt":"2026-10-20T10:00:00Z", "completedAt":"2026-10-20T10:00:01Z",
        "categories":[{"category":"records", "action":"exported", "count":1}],
        "restrictions":[], "parts":[{"partNumber":1, "sha256":"00".repeat(32), "bytes":42}],
        "receipt":envelope, "error":null
    });
    round_trip::<OperationResource>(resource.clone());
    resource["status"] = json!("failed");
    resource["receipt"] = Value::Null;
    resource["error"] = json!({"code":"fixture_failure"});
    round_trip::<OperationResource>(resource);
}

#[test]
fn receipt_payload_fields_match_the_contract() {
    round_trip::<ReceiptPayload>(json!({
        "receiptType":"baukit.data-subject.receipt.v1", "contractVersion":"1",
        "operationId":"00000000-0000-4000-8000-000000000001",
        "kind":"erasure", "status":"restricted", "subjectReference":"fixture-reference-01",
        "service":{"name":"fixture", "version":"1.0.0"},
        "createdAt":"2026-10-20T10:00:00Z", "completedAt":"2026-10-20T10:00:01Z",
        "categories":[{"category":"invoices", "action":"retained", "count":1}],
        "restrictions":[{"category":"invoices", "reason":"Statutory retention", "legalBasis":"Fixture obligation", "retainUntil":null}],
        "parts":[]
    }));
}

#[test]
fn enum_values_and_terminal_states_are_pinned() {
    for (name, status, terminal, receipt) in [
        ("pending", OperationStatus::Pending, false, false),
        ("completed", OperationStatus::Completed, true, true),
        ("notFound", OperationStatus::NotFound, true, true),
        ("restricted", OperationStatus::Restricted, true, true),
        ("failed", OperationStatus::Failed, true, false),
    ] {
        assert_eq!(round_trip::<OperationStatus>(json!(name)), status);
        assert_eq!(status.is_terminal(), terminal);
        assert_eq!(status.has_receipt(), receipt);
    }
    for name in ["erasure", "export"] {
        round_trip::<OperationKind>(json!(name));
    }
    for name in ["erased", "exported", "retained"] {
        round_trip::<CategoryAction>(json!(name));
    }
    for name in ["current", "previous"] {
        round_trip::<ReceiptKeyStatus>(json!(name));
    }
    round_trip::<ReceiptKeyAlgorithm>(json!("ed25519"));
    assert!(serde_json::from_value::<OperationStatus>(json!("not_found")).is_err());
    assert!(serde_json::from_value::<ReceiptKeyAlgorithm>(json!("Ed25519")).is_err());
}

#[test]
fn canonical_strings_dates_and_fractional_timestamps_are_pinned() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/data-subject/v1/completed.payload.json");
    let mut payload: ReceiptPayload =
        serde_json::from_slice(&fs::read(path).expect("fixture")).expect("payload");
    payload.service.name = "fixture\"\\\n\t\u{0001}é".into();
    payload.completed_at = "2026-10-20T10:00:01.123456789Z".parse().expect("timestamp");
    let encoded =
        String::from_utf8(payload.canonical_bytes().expect("canonical payload")).expect("UTF-8");
    assert!(encoded.contains(r#""service":{"name":"fixture\"\\\n\t\u0001é","version":"1.0.0"}"#));
    assert!(encoded.contains(r#""completedAt":"2026-10-20T10:00:01.123456789Z""#));
    for fraction in ["", ".123", ".123456"] {
        let time = format!("2026-10-20T10:00:01{fraction}Z");
        payload.completed_at = time.parse().expect("timestamp");
        assert!(
            String::from_utf8(payload.canonical_bytes().expect("canonical payload"))
                .expect("UTF-8")
                .contains(&format!("\"completedAt\":\"{time}\""))
        );
    }
}
