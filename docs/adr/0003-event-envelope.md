# ADR 0003: Suite events share one envelope

## Status
Amended 2026-11-27.

## Context

Products in the suite exchange events that award credits, activate rules and feed another product's
timeline. Senders retry from a durable outbox, so retries are normal. Without one envelope and one
idempotency field, every receiver has to guess whether two deliveries describe the same action.

The envelope must work in Rust services and in offline TypeScript clients. Its rules are small and
deterministic, so both implementations can share one test corpus. Product event names and payloads
do not belong in baukit.

## Decision

`baukit-events` and `@baukit/events` define schema version 1 (`EVENT_SCHEMA_VERSION`). The envelope
has `eventId`, `type`, `userId`, `occurredAt`, `sourceApp`, `schemaVersion` and an object `payload`.
`eventId` is the idempotency key.

Deserialization of `EventEnvelope` enforces the wire shape and fails with an `EventShapeError`:

- unknown fields fail, because the raw envelope uses `deny_unknown_fields`;
- `userId` must not be empty or padded and holds at most 255 characters;
- `sourceApp` matches `^[a-z][a-z0-9_-]{0,31}$`;
- `occurredAt` is `YYYY-MM-DDTHH:MM:SS[.fff]Z` and names a real instant;
- payload keys match `^[a-z][a-zA-Z0-9]{0,63}$`, with at most 32 keys.

`validate_event_envelope(envelope, expected_user_id, now)` then returns one of six
`EventValidationCode` values, checked in this order:

| Code | Rule |
|---|---|
| `event_schema_unsupported` | `schemaVersion` is not 1. It runs first, so older rules never read a newer envelope. |
| `event_id_invalid` | `eventId` is empty, padded with whitespace or longer than 64 Unicode scalar values. |
| `event_type_invalid` | `type` is not three dot-separated segments of lowercase letters, digits and `_`, each starting with a letter and at most 32 characters. |
| `event_user_mismatch` | `userId` differs from the authenticated subject. |
| `event_too_old` | `occurredAt` is more than 604800 seconds before `now`. The exact seven-day boundary passes. |
| `event_in_future` | `occurredAt` is more than 300 seconds after `now`. |

The ingest response is `IngestOutcome`, `{ "outcome", "ledgerEntryId" }`. `outcome` is one of
`granted`, `no_rule`, `capped`, `duplicate` and `rejected`.

`rust/crates/baukit-events/tests/generated.rs` writes `fixtures/events/generated-v1.json`, a corpus
of envelopes with the outcome Rust reaches for each. The TypeScript tests must reach the same
outcome for every case. Products own storage, authentication, payload validation, grants, outbox
writes, delivery and duplicate retention.

## Enforced by

- `rust/crates/baukit-events/tests/generated.rs`: `generated_corpus_is_current` fails when the
  committed corpus differs from what the Rust rules produce.
- `typescript/packages/events/src/index.test.ts`: the "Rust-generated envelope corpus" suite runs
  every corpus case through the TypeScript parser and validator.
- `rust/crates/baukit-events/tests/fixtures.rs`: `event_envelope_matches_shared_fixtures` checks the
  hand-written `fixtures/events/event-envelope-v1.json`.

## Consequences

A sender can retry one event ID until it gets a stored result. A receiver rejects a mismatched user
before it reads the payload. Backlogs older than seven days stay visible as rejected events but
cannot change a current balance. A sender whose clock runs more than five minutes fast gets
`event_in_future` and has to fix its clock. Adding an envelope field needs a new schema version,
because unknown fields fail.

## History

- 2026-08-24: accepted with five codes. The file entered this repository with baukit 0.1.0,
  commit `af6e2b4f`.
- 2026-09-27, commit `47931219`: field names moved from snake_case (`event_id`, `user_id`,
  `occurred_at`, `source_app`, `schema_version`, `ledger_entry_id`) to camelCase, and payload keys
  became camelCase. The schema stayed at version 1, because no product had events in flight. Event
  type names and codes are values and kept snake_case.
- 2026-11-25, commit `83f87059`: one envelope spec in Rust and TypeScript. It added
  `event_in_future` with the five-minute skew, the shape limits, `deny_unknown_fields` and the
  generated corpus.
