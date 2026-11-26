//! D-02: the Rust implementation writes `fixtures/events/generated-v1.json`,
//! and the TypeScript package must reach the same outcome for every case.
//! Regenerate with `BAUKIT_UPDATE_FIXTURES=1 cargo test -p baukit-events --test generated`.

#![cfg(test)]

use baukit_events::{EventEnvelope, validate_event_envelope};
use jiff::Timestamp;
use serde_json::{Map, Value, json};

const PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../fixtures/events/generated-v1.json"
);
const NOW: &str = "2026-08-24T12:00:00Z";
const USER: &str = "oidc-subject-1";

fn canonical() -> Map<String, Value> {
    let Value::Object(map) = json!({
        "eventId": "0198f3a2-7c1e-7d42-9a31-2b9f0c9a1d77",
        "type": "hebkit.workout.completed",
        "userId": USER,
        "occurredAt": "2026-08-24T10:00:00Z",
        "sourceApp": "hebkit",
        "schemaVersion": 1,
        "payload": {"durationMinutes": 42}
    }) else {
        unreachable!("literal object")
    };
    map
}

fn with(field: &str, value: Value) -> Value {
    let mut envelope = canonical();
    envelope.insert(field.to_owned(), value);
    Value::Object(envelope)
}

fn payload_with_keys(count: usize) -> Value {
    Value::Object(
        (0..count)
            .map(|index| (format!("k{index}"), json!(index)))
            .collect(),
    )
}

fn cases() -> Vec<(String, Value)> {
    let mut cases = vec![("canonical".to_owned(), Value::Object(canonical()))];
    let mut add = |name: &str, field: &str, value: Value| {
        cases.push((format!("{field} {name}"), with(field, value)));
    };
    for (name, value) in [
        ("1 char", "a".to_owned()),
        ("255 chars", "u".repeat(255)),
        ("256 chars", "u".repeat(256)),
        ("empty", String::new()),
        ("leading space", " u".to_owned()),
        ("trailing newline", "u\n".to_owned()),
        ("trailing BOM", "u\u{feff}".to_owned()),
        ("255 astral chars", "😀".repeat(255)),
        ("other user", "someone-else".to_owned()),
    ] {
        add(name, "userId", json!(value));
    }
    for (name, value) in [
        ("BOM edge", "\u{feff}id"),
        ("NEL edge", "id\u{85}"),
        ("NBSP edge", "\u{a0}id"),
        ("inner space", "a b"),
    ] {
        add(name, "eventId", json!(value));
    }
    for (name, value) in [
        ("32 chars", format!("a{}", "b".repeat(31))),
        ("33 chars", format!("a{}", "b".repeat(32))),
        ("hyphen and underscore", "a-b_c9".to_owned()),
        ("uppercase", "Hebkit".to_owned()),
        ("leading digit", "9app".to_owned()),
        ("trailing newline", "hebkit\n".to_owned()),
        ("empty", String::new()),
    ] {
        add(name, "sourceApp", json!(value));
    }
    for (name, value) in [
        ("millis", "2026-08-24T10:00:00.123Z"),
        ("one fraction digit", "2026-08-24T10:00:00.1Z"),
        ("micros", "2026-08-24T10:00:00.123456Z"),
        ("offset", "2026-08-24T12:00:00+02:00"),
        ("lowercase z", "2026-08-24T10:00:00z"),
        ("space separator", "2026-08-24 10:00:00Z"),
        ("no seconds", "2026-08-24T10:00Z"),
        ("february 30", "2026-02-30T10:00:00Z"),
        ("leap second", "2026-06-30T23:59:60Z"),
        ("hour 24", "2026-08-24T24:00:00Z"),
        ("trailing newline", "2026-08-24T10:00:00Z\n"),
        ("300 s ahead", "2026-08-24T12:05:00Z"),
        ("301 s ahead", "2026-08-24T12:05:01Z"),
        ("300.001 s ahead", "2026-08-24T12:05:00.001Z"),
        ("exactly 7 days old", "2026-08-17T12:00:00Z"),
        ("7 days and 1 ms old", "2026-08-17T11:59:59.999Z"),
    ] {
        add(name, "occurredAt", json!(value));
    }
    for (name, value) in [
        ("32 keys", payload_with_keys(32)),
        ("33 keys", payload_with_keys(33)),
        ("64 char key", json!({ format!("k{}", "A".repeat(63)): 1 })),
        ("65 char key", json!({ format!("k{}", "A".repeat(64)): 1 })),
        ("snake key", json!({"duration_minutes": 1})),
        ("upper first", json!({"Duration": 1})),
        ("trailing newline key", json!({"duration\n": 1})),
        ("empty key", json!({"": 1})),
        ("empty", json!({})),
    ] {
        add(name, "payload", value);
    }
    let mut extra = canonical();
    extra.insert("extra".to_owned(), json!(true));
    cases.push(("unknown field".to_owned(), Value::Object(extra)));
    cases
}

fn outcome(envelope: &Value) -> Value {
    let Ok(parsed) = serde_json::from_value::<EventEnvelope>(envelope.clone()) else {
        return json!({"parses": false, "code": null});
    };
    let now: Timestamp = NOW.parse().expect("fixed now");
    let code = validate_event_envelope(&parsed, USER, now)
        .err()
        .map(|code| code.as_str());
    json!({"parses": true, "code": code})
}

fn document() -> String {
    let cases: Vec<Value> = cases()
        .into_iter()
        .map(|(name, envelope)| {
            let expected = outcome(&envelope);
            json!({"name": name, "envelope": envelope, "expected": expected})
        })
        .collect();
    let document = json!({
        "description": "Generated by rust/crates/baukit-events/tests/generated.rs; do not edit.",
        "now": NOW,
        "expectedUserId": USER,
        "cases": cases,
    });
    serde_json::to_string_pretty(&document).expect("serialize corpus") + "\n"
}

#[test]
fn generated_corpus_is_current() {
    let generated = document();
    if std::env::var_os("BAUKIT_UPDATE_FIXTURES").is_some() {
        std::fs::write(PATH, &generated).expect("write corpus");
        return;
    }
    let committed = std::fs::read_to_string(PATH).unwrap_or_default();
    assert!(
        committed == generated,
        "fixtures/events/generated-v1.json is stale; rerun with BAUKIT_UPDATE_FIXTURES=1"
    );
}
