---
'@baukit/api-runtime': minor
'@baukit/events': minor
'@baukit/sync-client': minor
---

Move wire names to camelCase to match the Baukit HTTP APIs.

Breaking changes:

- `@baukit/api-runtime`: `parseApiErrorEnvelope` reads only `error.requestId`. An envelope that carries `request_id` and no `requestId` is no longer an `ApiError`. `ApiErrorEnvelope.error.request_id` is now `requestId`.
- `@baukit/events`: `EventEnvelopeSchema` and `EventEnvelope` use `eventId`, `userId`, `occurredAt`, `sourceApp`, and `schemaVersion` instead of `event_id`, `user_id`, `occurred_at`, `source_app`, and `schema_version`, and still describe schema version 1. `IngestOutcomeSchema` uses `ledgerEntryId` instead of `ledger_entry_id`. `EventPayloadSchema` accepts camelCase keys (`/^[a-z][a-zA-Z0-9]{0,63}$/`) and rejects snake_case keys.
- `@baukit/sync-client`: `toSnakeCaseSnapshot`, `toSnakeCaseFailure`, `SnakeCaseSyncStatusSnapshot`, and `SnakeCaseSyncFailure` are removed. Read the camelCase `SyncStatusSnapshot` directly. The documented `resync_required` detail is `details.horizonRevision` and the pull page flag is `hasMore`.
