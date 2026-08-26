# `@baukit/events`

`@baukit/events` is the TypeScript half of Baukit's version 1 event envelope.
It exports the envelope and ingestion outcome types, Zod schemas, stable
validation codes, and `validateEventEnvelope` for checks that need the
authenticated subject and current time.

```ts
import { EventEnvelopeSchema, validateEventEnvelope } from '@baukit/events';

const envelope = EventEnvelopeSchema.parse(input);
const code = validateEventEnvelope(envelope, connectionSubject, new Date().toISOString());
if (code !== null) throw new Error(code);
```

## Boundaries

The package does not choose event names or payload fields. Products document and validate those
schemas at their own ingestion boundary. Persistence, authorization, delivery, the outbox, and
retry policy stay product-owned as well.

What Baukit fixes here is the envelope: the wire fields, the five validation codes, and the
seven-day replay boundary. `baukit-events` is the Rust half of the same contract, and the fixture
at `fixtures/events/event-envelope-v1.json` is what both sides test against, so a change to one
without the other fails.
