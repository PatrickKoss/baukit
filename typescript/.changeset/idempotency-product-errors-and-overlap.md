---
'@baukit/api-runtime': patch
---

`MutationIntent` takes any body that JSON carries unchanged. It is now generic, `MutationIntent<Body extends JsonCompatible<Body> = JsonValue>`, so interface-typed request bodies no longer need a cast to `JsonValue`. `keyFor`, `settle`, `sendIdempotentMutation`, and `canonicalJson` infer the body type. Members set to `undefined` are left out of the key, and `undefined` array items encode as `null`, as `JSON.stringify` does. New type: `JsonCompatible`.

`createIdempotencyKeyStore` accepts `classifyError`, a `MutationErrorClassifier` that maps the product's own API errors to an outcome and returns `undefined` for the rest. The store exposes `classifyError(error)`, which applies it before `classifyMutationError`, and `sendIdempotentMutation` settles failed sends with it. `classifyMutationStatus` takes an optional error code and treats `idempotency_key_in_progress` as possibly committed.

Overlapping calls for the same intent share one key from the first `keyFor` until the intent settles, even while storage is still reading or when storage drops writes. A failed lookup does not block the next one.

New subpath `@baukit/api-runtime/backoff` exports `fullJitterBackoffMs` and `BackoffPolicy`, the delay formula `createApiFetch` already used. It imports nothing.

Breaking: `IdempotencyKeyStore` gains a required `classifyError` method, and its `keyFor` and `settle` are generic. A hand-written store or test double must add the method.
