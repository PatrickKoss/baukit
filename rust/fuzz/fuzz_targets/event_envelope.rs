#![no_main]

use std::sync::LazyLock;

use baukit_events::{
    EventEnvelope, MAX_EVENT_AGE_SECONDS, MAX_EVENT_CLOCK_SKEW_SECONDS, MAX_EVENT_PAYLOAD_KEYS,
    MAX_USER_ID_CHARACTERS, validate_event_envelope,
};
use jiff::{SignedDuration, Timestamp};
use libfuzzer_sys::fuzz_target;

const EXPECTED_USER_ID: &str = "oidc-subject-1";
static NOW: LazyLock<Timestamp> = LazyLock::new(|| "2026-08-24T12:00:00Z".parse().expect("now"));

fn assert_shape(envelope: &EventEnvelope) {
    let source = envelope.source_app.as_bytes();
    assert!(source.first().is_some_and(u8::is_ascii_lowercase));
    assert!(source.len() <= 32);
    assert!(source.iter().all(|byte| byte.is_ascii_lowercase()
        || byte.is_ascii_digit()
        || matches!(byte, b'_' | b'-')));
    assert!(!envelope.user_id.is_empty());
    assert_eq!(envelope.user_id.trim(), envelope.user_id);
    assert!(envelope.user_id.chars().count() <= MAX_USER_ID_CHARACTERS);
    assert!(envelope.payload.len() <= MAX_EVENT_PAYLOAD_KEYS);
    for key in envelope.payload.keys() {
        assert!(key.as_bytes().first().is_some_and(u8::is_ascii_lowercase));
        assert!(key.len() <= 64 && key.bytes().all(|byte| byte.is_ascii_alphanumeric()));
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(envelope) = serde_json::from_slice::<EventEnvelope>(data) else {
        return;
    };
    assert_shape(&envelope);
    if validate_event_envelope(&envelope, EXPECTED_USER_ID, *NOW).is_ok() {
        assert_eq!(envelope.user_id, EXPECTED_USER_ID);
        assert!(!envelope.event_id.is_empty());
        let offset = envelope.occurred_at.duration_since(*NOW);
        assert!(offset >= -SignedDuration::from_secs(MAX_EVENT_AGE_SECONDS));
        assert!(offset <= SignedDuration::from_secs(MAX_EVENT_CLOCK_SKEW_SECONDS));
    }
    let encoded = serde_json::to_vec(&envelope).expect("serialize envelope");
    let decoded: EventEnvelope = serde_json::from_slice(&encoded).expect("roundtrip envelope");
    assert_eq!(decoded, envelope);
});
