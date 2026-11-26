# `@baukit/events`

`@baukit/events` is the TypeScript half of Baukit's version 1 event envelope. `baukit-events` is the
Rust half.

## Exports

| Export                                                                                                       | What it is                                                                                  |
| ------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------- |
| `EventEnvelopeSchema`, `EventEnvelope`                                                                       | Zod schema and type for the envelope. It rejects unknown fields.                            |
| `EventPayloadSchema`, `EventPayloadValueSchema`                                                              | The payload object: up to `MAX_EVENT_PAYLOAD_KEYS` (32) camelCase keys of JSON values.      |
| `IngestOutcomeSchema`, `IngestOutcome`                                                                       | The ingestion response, `{ outcome, ledgerEntryId }`.                                       |
| `INGEST_OUTCOME_STATUSES`                                                                                    | `granted`, `no_rule`, `capped`, `duplicate` and `rejected`.                                 |
| `validateEventEnvelope(envelope, expectedUserId, now)`                                                       | The checks that need the connection subject and the current time. Returns a code or `null`. |
| `EVENT_VALIDATION_CODES`, `EventValidationCode`                                                              | The stable codes.                                                                           |
| `EVENT_SCHEMA_VERSION`, `MAX_EVENT_AGE_SECONDS`, `MAX_EVENT_CLOCK_SKEW_SECONDS` and the other `MAX_*` limits | The contract's numbers.                                                                     |

The fields are `eventId`, `type`, `userId`, `occurredAt`, `sourceApp`, `schemaVersion` and `payload`.
`validateEventEnvelope` returns one of these codes, checked in this order:

| Code                       | When                                                                  |
| -------------------------- | --------------------------------------------------------------------- |
| `event_schema_unsupported` | `schemaVersion` is not 1.                                             |
| `event_id_invalid`         | `eventId` is empty, padded or longer than 64 characters.              |
| `event_type_invalid`       | `type` is not three lower-snake-case segments, `<app>.<noun>.<verb>`. |
| `event_user_mismatch`      | `userId` differs from `expectedUserId`.                               |
| `event_too_old`            | `occurredAt` is more than seven days before `now`.                    |
| `event_in_future`          | `occurredAt` is more than five minutes after `now`.                   |

## Example

```ts
import { EventEnvelopeSchema, validateEventEnvelope } from '@baukit/events';

const envelope = EventEnvelopeSchema.parse({
  eventId: 'evt-1',
  type: 'orders.order.placed',
  userId: 'user-1',
  occurredAt: '2026-01-01T12:00:00Z',
  sourceApp: 'orders',
  schemaVersion: 1,
  payload: { orderId: 'o-1' },
});

export const accepted = validateEventEnvelope(envelope, 'user-1', '2026-01-02T12:00:00Z');
export const rejected = validateEventEnvelope(envelope, 'user-2', '2026-01-02T12:00:00Z');
// accepted is null, rejected is 'event_user_mismatch'
```

## Boundaries

The package does not choose event names or payload fields. Products document and validate those
schemas at their own ingestion boundary. Persistence, authorization, delivery, the outbox and retry
policy stay in the product.

The fixture at `fixtures/events/event-envelope-v1.json` is what both halves test against, so a change
to one without the other fails. The reasoning is in
[ADR 0003](../../../docs/adr/0003-event-envelope.md). See the
[suite and platform guide](../../../docs/guide/suite-and-platform.md).
