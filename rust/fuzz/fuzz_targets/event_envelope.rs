#![no_main]

use baukit_events::{EventEnvelope, validate_event_envelope};
use chrono::{DateTime, Utc};
use libfuzzer_sys::fuzz_target;

const EXPECTED_USER_ID: &str = "oidc-subject-1";
const NOW: &str = "2026-08-24T12:00:00Z";

fuzz_target!(|data: &[u8]| {
    let Ok(envelope) = serde_json::from_slice::<EventEnvelope>(data) else {
        return;
    };
    let now: DateTime<Utc> = NOW.parse().expect("fixed timestamp");
    if validate_event_envelope(&envelope, EXPECTED_USER_ID, now).is_ok() {
        assert_eq!(envelope.user_id, EXPECTED_USER_ID);
        assert!(!envelope.event_id.is_empty());
    }
    let encoded = serde_json::to_vec(&envelope).expect("serialize envelope");
    let decoded: EventEnvelope = serde_json::from_slice(&encoded).expect("roundtrip envelope");
    assert_eq!(decoded, envelope);
});
