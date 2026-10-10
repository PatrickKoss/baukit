# @baukit/events

## Unreleased

## 0.10.11

### Patch Changes

- Release the coordinated baukit 0.10.11 train.

## 0.10.10

### Patch Changes

- Release the coordinated baukit 0.10.10 train.

## 0.10.9

### Patch Changes

- Release the coordinated baukit 0.10.9 train.

## 0.10.8

### Patch Changes

- Release the coordinated baukit 0.10.8 train.

## 0.10.7

### Patch Changes

- Release the coordinated baukit 0.10.7 train.

## 0.10.6

### Patch Changes

- Release the coordinated baukit 0.10.6 train.

## 0.10.5

### Patch Changes

- Release the coordinated baukit 0.10.5 train.

## 0.10.4

### Patch Changes

- Release the coordinated baukit 0.10.4 train.

## 0.10.3

### Patch Changes

- Release the coordinated baukit 0.10.3 train.

## 0.10.2

### Patch Changes

- Release the coordinated baukit 0.10.2 train.

## 0.10.1

### Patch Changes

- Release the coordinated baukit 0.10.1 train.

## 0.10.0

- Move shipped notes out of Unreleased into their release sections.

### Minor Changes

- Release the coordinated baukit 0.10.0 train.

## 0.9.0

### Minor Changes

- Release the coordinated baukit 0.9.0 train.

## 0.8.0

### Minor Changes

- Release the coordinated baukit 0.8.0 train.

## 0.7.4

### Patch Changes

- Release the coordinated baukit 0.7.4 train.

## 0.7.3

- Publish the TypeScript sources referenced by JavaScript and declaration maps. Check source paths in the packed archive.

### Patch Changes

- Release the coordinated baukit 0.7.3 train.

## 0.7.2

### Patch Changes

- Release the coordinated baukit 0.7.2 train.

## 0.7.1

### Patch Changes

- Release the coordinated baukit 0.7.1 train.

## 0.7.0

### Minor Changes

- Release the coordinated baukit 0.7.0 train.

## 0.6.0

### Minor Changes

- be675db: `@baukit/events` now depends on zod 4.6.5 instead of zod 3. `EventEnvelopeSchema`, `EventPayloadSchema`, `EventPayloadValueSchema`, and `IngestOutcomeSchema` are zod 4 schemas, so a consumer that composes them with its own schemas or reads their issue objects needs zod 4 as well. Validation rules and the issue messages (`event_id_invalid`, `event_type_invalid`, `event_schema_unsupported`) are unchanged.
- Release the coordinated baukit 0.6.0 train.

## 0.5.2

### Patch Changes

- Release the coordinated baukit 0.5.2 train.

## 0.5.1

### Patch Changes

- Release the coordinated baukit 0.5.1 train.

## 0.5.0

### Minor Changes

- 4793121: Move wire names to camelCase to match the Baukit HTTP APIs.

  Breaking changes:

  - `@baukit/api-runtime`: `parseApiErrorEnvelope` reads only `error.requestId`. An envelope that carries `request_id` and no `requestId` is no longer an `ApiError`. `ApiErrorEnvelope.error.request_id` is now `requestId`.
  - `@baukit/events`: `EventEnvelopeSchema` and `EventEnvelope` use `eventId`, `userId`, `occurredAt`, `sourceApp`, and `schemaVersion` instead of `event_id`, `user_id`, `occurred_at`, `source_app`, and `schema_version`, and still describe schema version 1. `IngestOutcomeSchema` uses `ledgerEntryId` instead of `ledger_entry_id`. `EventPayloadSchema` accepts camelCase keys (`/^[a-z][a-zA-Z0-9]{0,63}$/`) and rejects snake_case keys.
  - `@baukit/sync-client`: `toSnakeCaseSnapshot`, `toSnakeCaseFailure`, `SnakeCaseSyncStatusSnapshot`, and `SnakeCaseSyncFailure` are removed. Read the camelCase `SyncStatusSnapshot` directly. The documented `resync_required` detail is `details.horizonRevision` and the pull page flag is `hasMore`.

- 8d268e1: Add a `default` export condition next to `import` on every export except the ESM-only `./vitest` subpaths. Jest and other CommonJS-condition resolvers now find `@baukit/*` without a `moduleNameMapper`. Each package's `test` script packs the package and resolves every export under `require` conditions from the archive.
- Release the coordinated baukit 0.5.0 train.

## 0.4.0

### Minor Changes

- Release the coordinated baukit 0.4.0 train.

## 0.3.0

### Minor Changes

- Release the coordinated baukit 0.3.0 train.

## 0.2.1

### Patch Changes

- Release the coordinated baukit 0.2.1 train.

## 0.2.0

### Minor Changes

- Release the coordinated baukit 0.2.0 train.

## 0.1.2

### Patch Changes

- Release the coordinated baukit 0.1.2 train.

## 0.1.1

### Patch Changes

- Release the coordinated baukit 0.1.1 train.

## 0.1.0

### Minor Changes

- First public release of `@baukit/events`.
