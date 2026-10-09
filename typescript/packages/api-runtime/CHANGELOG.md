# @baukit/api-runtime

## [Unreleased]

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

- Report failed erasure replays with a typed operation failure and retain the key for reconciliation after repair. Test the shared Rust wire receipts.

### Patch Changes

- Release the coordinated baukit 0.7.1 train.

## 0.7.0

- Remove durable erasure keys after definitive receipts. Preserve committed receipts when local key removal fails.
- Add the `/erasure` client with durable idempotency keys, typed receipts, fenced-subject handling, and bounded, abortable status polling.

### Minor Changes

- Release the coordinated baukit 0.7.0 train.

## 0.6.0

### Minor Changes

- Release the coordinated baukit 0.6.0 train.

## 0.5.2

### Patch Changes

- Release the coordinated baukit 0.5.2 train.

## 0.5.1

### Patch Changes

- fa3e0c3: `MutationIntent` takes any body that JSON carries unchanged. It is now generic, `MutationIntent<Body extends JsonCompatible<Body> = JsonValue>`, so interface-typed request bodies no longer need a cast to `JsonValue`. `keyFor`, `settle`, `sendIdempotentMutation`, and `canonicalJson` infer the body type. Members set to `undefined` are left out of the key, and `undefined` array items encode as `null`, as `JSON.stringify` does. New type: `JsonCompatible`.

  `createIdempotencyKeyStore` accepts `classifyError`, a `MutationErrorClassifier` that maps the product's own API errors to an outcome and returns `undefined` for the rest. The store exposes `classifyError(error)`, which applies it before `classifyMutationError`, and `sendIdempotentMutation` settles failed sends with it. `classifyMutationStatus` takes an optional error code and treats `idempotency_key_in_progress` as possibly committed.

  Overlapping calls for the same intent share one key from the first `keyFor` until the intent settles, even while storage is still reading or when storage drops writes. A failed lookup does not block the next one.

  New subpath `@baukit/api-runtime/backoff` exports `fullJitterBackoffMs` and `BackoffPolicy`, the delay formula `createApiFetch` already used. It imports nothing.

  Breaking: `IdempotencyKeyStore` gains a required `classifyError` method, and its `keyFor` and `settle` are generic. A hand-written store or test double must add the method.

- Release the coordinated baukit 0.5.1 train.

## 0.5.0

### Minor Changes

- 4793121: Move wire names to camelCase to match the Baukit HTTP APIs.

  Breaking changes:

  - `@baukit/api-runtime`: `parseApiErrorEnvelope` reads only `error.requestId`. An envelope that carries `request_id` and no `requestId` is no longer an `ApiError`. `ApiErrorEnvelope.error.request_id` is now `requestId`.
  - `@baukit/events`: `EventEnvelopeSchema` and `EventEnvelope` use `eventId`, `userId`, `occurredAt`, `sourceApp`, and `schemaVersion` instead of `event_id`, `user_id`, `occurred_at`, `source_app`, and `schema_version`, and still describe schema version 1. `IngestOutcomeSchema` uses `ledgerEntryId` instead of `ledger_entry_id`. `EventPayloadSchema` accepts camelCase keys (`/^[a-z][a-zA-Z0-9]{0,63}$/`) and rejects snake_case keys.
  - `@baukit/sync-client`: `toSnakeCaseSnapshot`, `toSnakeCaseFailure`, `SnakeCaseSyncStatusSnapshot`, and `SnakeCaseSyncFailure` are removed. Read the camelCase `SyncStatusSnapshot` directly. The documented `resync_required` detail is `details.horizonRevision` and the pull page flag is `hasMore`.

- bb0121b: Add keyed mutation support.

  `/idempotency` is a new entry point, not re-exported from the package root. `createIdempotencyKeyStore` keeps one `Idempotency-Key` per account, operation, and body, compared by canonical JSON, until the outcome is definite or `ttlMs` passes. Keys default to `crypto.randomUUID()` and in-memory storage; `storage` takes an `IdempotencyKeyStorage` port to keep them across reloads. `classifyMutationStatus` and `classifyMutationError` return `committed`, `not-committed`, or `possibly-committed`. Network errors, aborts, 408, 429, 5xx, 409 `idempotency_key_in_progress`, and unknown throws are possibly committed and keep the key. `sendIdempotentMutation` gets the key, sends, and settles it. `canonicalJson` and `IDEMPOTENCY_KEY_IN_PROGRESS_CODE` are exported too.

  The package root exports `IDEMPOTENCY_KEY_HEADER`. `RetryableMethod` now includes `POST` and `PATCH`, and `createApiFetch` retries them only when `retry.methods` names them and the request carries `Idempotency-Key`. Unkeyed `POST` and `PATCH` still never retry.

  Break: before, `retry.methods` silently dropped `POST` and `PATCH`. A caller that already listed them now gets retries for keyed requests. Remove them from `methods` to keep the old behavior.

- 8d268e1: Add a `default` export condition next to `import` on every export except the ESM-only `./vitest` subpaths. Jest and other CommonJS-condition resolvers now find `@baukit/*` without a `moduleNameMapper`. Each package's `test` script packs the package and resolves every export under `require` conditions from the archive.
- Release the coordinated baukit 0.5.0 train.

## 0.4.0

### Minor Changes

- Release the coordinated baukit 0.4.0 train.

## 0.3.0

### Minor Changes

- Release the coordinated baukit 0.3.0 train.
- a299f89: Add dependency-free, display-only identity hints from unverified JWT claims with product-supplied fallback text.

## 0.2.1

### Patch Changes

- Release the coordinated baukit 0.2.1 train.
- b659dde: Fix generated Expo apps so Metro bundles the shared product-root limits policy without crawling
  sibling build and dependency directories.

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

- First public release of `@baukit/api-runtime`.
