# baukit-events

`baukit-events` defines the version 1 JSON envelope for domain events sent
between products. It owns the wire fields, the stable validation codes and the
seven-day replay boundary. Products still own their event names, payload
schemas, persistence, authorization, rules, outbox and delivery code.

## Exports

| Item | What it is |
|---|---|
| `EventEnvelope` | The envelope. Deserializing it checks the wire shape and rejects unknown fields. |
| `validate_event_envelope(envelope, expected_user_id, now)` | The checks that need context: schema version, id, type, owner and age. |
| `EventValidationCode` | The stable failure codes, with `as_str()` for the wire value. |
| `EventShapeError` | Why deserialization rejected the shape: `UserId`, `SourceApp`, `OccurredAt` or `Payload`. |
| `IngestOutcome`, `IngestOutcomeStatus` | The ingestion response, `{ "outcome", "ledgerEntryId" }`. |
| `wire_instant` | Truncates a `Timestamp` to the millisecond precision `occurredAt` carries. |

The envelope fields are `eventId`, `type`, `userId`, `occurredAt`, `sourceApp`,
`schemaVersion` and `payload`. `type` has the form `<app>.<noun>.<verb>`.
Payload keys are camelCase, at most `MAX_EVENT_PAYLOAD_KEYS` (32) of them.
`IngestOutcomeStatus` is `granted`, `no_rule`, `capped`, `duplicate` or
`rejected`.

`validate_event_envelope` returns one of these codes:

| Code | When |
|---|---|
| `event_schema_unsupported` | `schemaVersion` is not `EVENT_SCHEMA_VERSION` (1). |
| `event_id_invalid` | `eventId` is empty, padded or longer than `MAX_EVENT_ID_CHARACTERS` (64). |
| `event_type_invalid` | `type` does not have three bounded lower-snake-case segments. |
| `event_user_mismatch` | `userId` differs from the subject bound to the connection. |
| `event_too_old` | `occurredAt` is more than `MAX_EVENT_AGE_SECONDS` (seven days) before `now`. |
| `event_in_future` | `occurredAt` is more than `MAX_EVENT_CLOCK_SKEW_SECONDS` (five minutes) after `now`. |

`eventId` is the sender's idempotency key. A receiver stores the first outcome
for that ID and returns it on a retry. An event exactly seven days old is
valid. An older event must not change current state.

## Example

```rust
use baukit_events::{EventEnvelope, EventValidationCode, validate_event_envelope};
use jiff::Timestamp;

let envelope: EventEnvelope = serde_json::from_str(
    r#"{
        "eventId": "evt-1",
        "type": "orders.order.placed",
        "userId": "user-1",
        "occurredAt": "2026-01-01T12:00:00Z",
        "sourceApp": "orders",
        "schemaVersion": 1,
        "payload": { "orderId": "o-1" }
    }"#,
)?;
let now: Timestamp = "2026-01-02T12:00:00Z".parse()?;

assert_eq!(validate_event_envelope(&envelope, "user-1", now), Ok(()));
assert_eq!(
    validate_event_envelope(&envelope, "user-2", now),
    Err(EventValidationCode::EventUserMismatch),
);
# Ok::<(), Box<dyn std::error::Error>>(())
```

The fixture at `fixtures/events/event-envelope-v1.json` is the contract. Rust
and TypeScript tests call their production validators against every case.
`@baukit/events` is the TypeScript side.

The reasoning is in [ADR 0003](../../../docs/adr/0003-event-envelope.md). See
the [suite and platform guide](../../../docs/guide/suite-and-platform.md) for
how products send and receive events.
