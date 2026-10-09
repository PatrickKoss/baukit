# @baukit/sync-client

## Unreleased

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

- be675db: Raised peer floors to the versions Baukit now tests against. `@baukit/ui-tokens` takes ESLint 10 (`eslint ^10.11.0`), so products can leave ESLint 9. The Expo SDK 57 peers are `react-native ^0.86.3`, `expo-auth-session ^57.0.13`, `expo-secure-store ^57.0.4`, `expo-web-browser ^57.0.3`, `expo-sqlite ^57.0.3`, `expo-notifications ^57.0.21`, and `expo-network ^57.0.2`. `@baukit/data-contracts-dexie` needs `dexie ^4.4.6`.
- Release the coordinated baukit 0.6.0 train.

## 0.5.2

### Patch Changes

- Release the coordinated baukit 0.5.2 train.

## 0.5.1

### Patch Changes

- Release the coordinated baukit 0.5.1 train.
- a39abd7: `SyncScheduler` retries failed runs when given `retry: { maxRetries, delayMs, onRetryScheduled? }`. `delayMs(error, retryIndex)` returns the wait in milliseconds or `null` for no retry; `fullJitterBackoffMs` from `@baukit/api-runtime/backoff` is the Baukit policy to pass in, so the root entry keeps no runtime dependencies. The retry index restarts after a success. Any trigger during the wait ends it and restarts the index: `trigger()`, the interval, foreground, or connectivity. `stop()` ends the wait without another run, and a wait that ends while backgrounded runs nothing. A bad delay or a throwing `delayMs` reaches `onError` and ends the retries. The constructor throws a `RangeError` for a `maxRetries` that is not a non-negative integer. New types: `SyncSchedulerRetryOptions` and `SyncSchedulerRetry`. Without `retry`, behavior is unchanged.

  `trigger()` during a run no longer only joins it: when the run is waiting to retry, the wait ends. This applies only with `retry` set.

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

- 9f0dc94: Add a persisted hybrid logical clock with JavaScript-safe encoding and shared Rust and TypeScript fixtures.
- Release the coordinated baukit 0.3.0 train.
- 4d1f0d3: Add atomic submitted-batch outcome conformance for late accepted and rejected responses.

  Add the browser scheduler environment and an explicit recovery signal callback for waking product-owned retry delays.

- 9b92f9f: Add optional tombstone-horizon and full-resync conformance callbacks and cases.

## 0.2.1

### Patch Changes

- Release the coordinated baukit 0.2.1 train.

## 0.2.0

### Minor Changes

- Add a callback-driven conformance harness for sync implementations.
- Track pull and push attempt and success timestamps, preserve typed transport
  failures, honor `Retry-After`, and validate pull pages and push results.

## 0.1.2

### Patch Changes

- Release the coordinated baukit 0.1.2 train.

## 0.1.1

### Patch Changes

- Release the coordinated baukit 0.1.1 train.

## 0.1.0

### Minor Changes

- First public release of `@baukit/sync-client`.
