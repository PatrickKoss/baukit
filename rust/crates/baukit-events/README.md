# baukit-events

`baukit-events` defines the version 1 JSON envelope used for domain events sent
between products. It owns the wire fields, five stable validation codes, and
the seven-day replay boundary. Products still own their event names, payload
schemas, persistence, authorization, rules, outbox, and delivery code.

`event_id` is the sender's idempotency key. A receiver stores the first outcome
for that ID and returns it on a retry. `user_id` must match the identity subject
bound to the authenticated connection. An event exactly seven days old is
valid. An older event returns `event_too_old` and must not change current state.

```rust
use chrono::Utc;
use baukit_events::{EventEnvelope, validate_event_envelope};

# fn check(
#     envelope: &EventEnvelope,
#     connection_subject: &str,
# ) -> Result<(), baukit_events::EventValidationCode> {
validate_event_envelope(envelope, connection_subject, Utc::now())?;
# Ok(())
# }
```

The fixture at `fixtures/events/event-envelope-v1.json` is the contract. Rust
and TypeScript tests call their production validators against every case.
