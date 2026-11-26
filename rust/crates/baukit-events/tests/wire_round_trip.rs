#![cfg(test)]
use baukit_events::{EVENT_SCHEMA_VERSION, EventEnvelope, wire_instant};
use jiff::Timestamp;
use serde_json::{Map, json};

fn envelope_at(occurred_at: Timestamp) -> EventEnvelope {
    EventEnvelope {
        event_id: "event-1".into(),
        event_type: "alpha.activity.completed".into(),
        user_id: "user-1".into(),
        occurred_at,
        source_app: "alpha".into(),
        schema_version: EVENT_SCHEMA_VERSION,
        payload: Map::new(),
    }
}

#[test]
fn a_nanosecond_instant_serializes_to_a_parseable_millisecond() {
    let precise = Timestamp::new(1_764_000_000, 123_456_789).expect("instant");
    let wire = serde_json::to_value(envelope_at(precise)).expect("serialize");

    assert_eq!(wire["occurredAt"], json!("2025-11-24T16:00:00.123Z"));
    let parsed: EventEnvelope = serde_json::from_value(wire).expect("own output parses");
    assert_eq!(
        parsed.occurred_at,
        Timestamp::new(1_764_000_000, 123_000_000).expect("instant")
    );
}

#[test]
fn a_whole_second_keeps_its_short_form() {
    let whole = Timestamp::new(1_764_000_000, 0).expect("instant");
    let wire = serde_json::to_value(envelope_at(whole)).expect("serialize");

    assert_eq!(wire["occurredAt"], json!("2025-11-24T16:00:00Z"));
}

#[test]
fn a_wire_instant_envelope_equals_its_parsed_json() {
    let envelope = envelope_at(wire_instant(
        Timestamp::new(1_764_000_000, 987_654_321).expect("instant"),
    ));
    let wire = serde_json::to_value(&envelope).expect("serialize");

    assert_eq!(
        serde_json::from_value::<EventEnvelope>(wire).expect("parse"),
        envelope
    );
}
