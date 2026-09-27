---
'@baukit/api-runtime': minor
---

Add keyed mutation support.

`/idempotency` is a new entry point, not re-exported from the package root. `createIdempotencyKeyStore` keeps one `Idempotency-Key` per account, operation, and body, compared by canonical JSON, until the outcome is definite or `ttlMs` passes. Keys default to `crypto.randomUUID()` and in-memory storage; `storage` takes an `IdempotencyKeyStorage` port to keep them across reloads. `classifyMutationStatus` and `classifyMutationError` return `committed`, `not-committed`, or `possibly-committed`. Network errors, aborts, 408, 429, 5xx, 409 `idempotency_key_in_progress`, and unknown throws are possibly committed and keep the key. `sendIdempotentMutation` gets the key, sends, and settles it. `canonicalJson` and `IDEMPOTENCY_KEY_IN_PROGRESS_CODE` are exported too.

The package root exports `IDEMPOTENCY_KEY_HEADER`. `RetryableMethod` now includes `POST` and `PATCH`, and `createApiFetch` retries them only when `retry.methods` names them and the request carries `Idempotency-Key`. Unkeyed `POST` and `PATCH` still never retry.

Break: before, `retry.methods` silently dropped `POST` and `PATCH`. A caller that already listed them now gets retries for keyed requests. Remove them from `methods` to keep the old behavior.
