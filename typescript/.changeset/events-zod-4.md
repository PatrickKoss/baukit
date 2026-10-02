---
'@baukit/events': minor
---

`@baukit/events` now depends on zod 4.6.5 instead of zod 3. `EventEnvelopeSchema`, `EventPayloadSchema`, `EventPayloadValueSchema`, and `IngestOutcomeSchema` are zod 4 schemas, so a consumer that composes them with its own schemas or reads their issue objects needs zod 4 as well. Validation rules and the issue messages (`event_id_invalid`, `event_type_invalid`, `event_schema_unsupported`) are unchanged.
