# ADR 0003: Shared suite event envelope

## Status

Accepted, 2026-08-24.

## Context

Products in the suite exchange events that can award credits, activate rules,
and feed another product's timeline. Retries are normal because senders use a
durable outbox. Without one envelope and one idempotency field, each receiver
has to infer whether two deliveries describe the same domain action.

The envelope must work in Rust services and offline TypeScript clients. The
rules are small and deterministic, so separate implementations can share one
fixture corpus. Product event names and payloads do not belong in Baukit.

## Decision

Add `baukit-events` and `@baukit/events` in the coordinated v0.1.0 train. Both
packages define schema version 1 with `event_id`, `type`, `user_id`,
`occurred_at`, `source_app`, `schema_version`, and an object payload.

`event_id` is opaque text with 1 to 64 Unicode scalar values and is the
idempotency key. Event types contain exactly three lower-snake-case segments,
each limited to 32 characters. The receiver compares `user_id` with the
authenticated connection subject. Events more than 604800 seconds old return
`event_too_old`; the exact seven-day boundary remains valid.

Validation uses five stable codes: `event_id_invalid`, `event_type_invalid`,
`event_user_mismatch`, `event_too_old`, and `event_schema_unsupported`. Schema
version validation runs first so a newer envelope cannot be interpreted with
older rules.

The shared packages also define the generic ingestion outcome vocabulary.
Products own storage, authentication, payload validation, grants, outbox
writes, delivery, and duplicate retention.

## Consequences

A sender can retry the same event ID until it receives a stored result. A
receiver can reject a mismatched user before it reads the payload. Replayed
backlogs older than seven days remain visible as rejected events but cannot
change a current credit balance or rule state.

The first users keep local copies until v0.1.0 is tagged. Their upgrade
deletes those copies and changes one dependency without changing JSON on the
wire.
