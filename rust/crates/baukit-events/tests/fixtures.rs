use baukit_events::{
    EVENT_SCHEMA_VERSION, EventEnvelope, EventValidationCode, MAX_EVENT_AGE_SECONDS,
    MAX_EVENT_ID_CHARACTERS, MAX_EVENT_PAYLOAD_KEYS, MAX_EVENT_TYPE_SEGMENT_CHARACTERS,
    validate_event_envelope,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct FixtureDocument {
    contract_version: u32,
    representation: Value,
    constants: FixtureConstants,
    cases: Vec<FixtureCase>,
}

#[derive(Deserialize)]
struct FixtureConstants {
    schema_version: u32,
    maximum_event_id_characters: usize,
    maximum_event_type_segment_characters: usize,
    maximum_event_age_seconds: i64,
    maximum_payload_keys: usize,
}

#[derive(Deserialize)]
struct FixtureCase {
    name: String,
    input: FixtureInput,
    expected_code: Option<EventValidationCode>,
}

#[derive(Deserialize)]
struct FixtureInput {
    envelope: EventEnvelope,
    expected_user_id: String,
    now: DateTime<Utc>,
}

#[test]
fn event_envelope_matches_shared_fixtures() {
    let fixtures: FixtureDocument = serde_json::from_str(include_str!(
        "../../../../fixtures/events/event-envelope-v1.json"
    ))
    .expect("event fixtures must be valid");
    assert_eq!(fixtures.contract_version, EVENT_SCHEMA_VERSION);
    assert_eq!(fixtures.constants.schema_version, EVENT_SCHEMA_VERSION);
    assert_eq!(
        fixtures.constants.maximum_event_id_characters,
        MAX_EVENT_ID_CHARACTERS
    );
    assert_eq!(
        fixtures.constants.maximum_event_type_segment_characters,
        MAX_EVENT_TYPE_SEGMENT_CHARACTERS
    );
    assert_eq!(
        fixtures.constants.maximum_event_age_seconds,
        MAX_EVENT_AGE_SECONDS
    );
    assert_eq!(
        fixtures.constants.maximum_payload_keys,
        MAX_EVENT_PAYLOAD_KEYS
    );
    assert_eq!(fixtures.representation["timestamps"], "rfc3339_utc_z");
    for fixture in fixtures.cases {
        assert_eq!(
            validate_event_envelope(
                &fixture.input.envelope,
                &fixture.input.expected_user_id,
                fixture.input.now,
            )
            .err(),
            fixture.expected_code,
            "{}",
            fixture.name
        );
    }
}
