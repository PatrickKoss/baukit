# @baukit/events

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

The package does not choose event names or payload fields. Products document
and validate those schemas at their own ingestion boundary.
