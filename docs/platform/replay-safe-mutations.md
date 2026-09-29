# Replay-safe HTTP mutations

**Status:** Shared protocol with a header parser, a client key store, and a conformance check.
Storage stays in each product.
**Applies to:** `POST` and `PATCH` routes, and any other write whose effect must not repeat when a
client resends it after a lost response.
**Related:** [revision preconditions](../../rust/crates/baukit-http/README.md#revision-preconditions),
[offline readiness](./offline-readiness-contract.md),
[product profile erasure](./product-profile-erasure-contract.md),
[integration reliability](./integration-reliability.md).

A client that sends a create and never hears back cannot tell whether the server committed it. If
it sends the request again, the server must return the first result instead of creating a second
row. The client marks the two attempts as one intent by sending the same `Idempotency-Key`. The
server stores the result of the first attempt in the same transaction as the effect and replays it.

## Pieces

| Piece | Where |
| --- | --- |
| Header parser and error mapping | `baukit_http::IdempotencyKeyRule`, `IdempotencyError` |
| Default CORS allowlist entry for `Idempotency-Key` | `baukit_http::finalize` and `layers` |
| Conformance check for the storage path | `baukit_test::check_replay_safe_mutation_conformance` |
| Client key store and outcome classifier | `@baukit/api-runtime/idempotency` |
| Keyed `POST` and `PATCH` retries | `createApiFetch` with `retry.methods` |
| Replay table, fingerprint, snapshot, cleanup | the product |

## Which routes take a key

Each route declares its key policy, and the OpenAPI document states it.

- **Required** on a create or command that makes a new row, spends a balance, sends a message, or
  starts a job, when the client can retry automatically or after a restart.
- **Optional** when some clients cannot keep a key, such as an MCP tool or a script, and a
  duplicate is a nuisance rather than harm. Without a key the route runs as a plain write.
- **None** on `PUT` and `DELETE` of a known ID, which are idempotent by target, and on writes that
  carry `If-Match`. A revision precondition already rejects the second attempt with 412, and the
  client resolves it by reading the resource again.

A write that is both keyed and conditional puts the expected revision in the fingerprint. The
replay lookup runs before the revision check, so a lost-response retry replays instead of
returning 412.

## Key grammar

`Idempotency-Key` is one header whose value is `min..=max` bytes of visible ASCII (`0x21` to
`0x7E`). The route chooses `min` and `max` with `IdempotencyKeyRule::new(min, max)`, and `max` is
at most 255. A value is opaque. It is not a structured-field string, so quotes are part of the key.

Clients send a random UUID. Use `IdempotencyKeyRule::new(1, 128)` unless the product needs a floor
against guessable keys, such as `new(16, 128)`. Store a SHA-256 digest of the key, never the key
itself, so the column width does not depend on `max` and a leaked table does not reveal keys.

## Errors

| Case | Status | `code` | `details` |
| --- | --- | --- | --- |
| Required key missing | 400 | `idempotency_key_required` | none |
| Repeated header, empty, too short, too long, or a byte outside visible ASCII | 400 | `invalid_idempotency_key` | `reason`, one of `InvalidIdempotencyKey::reason` |
| Same key, same scope, different fingerprint | 409 | `idempotency_key_reused` | none |
| First request with the key still running, and the product chose not to wait | 409 | `idempotency_key_in_progress` | none |

The IETF `Idempotency-Key` draft suggests 422 for key reuse. Every Baukit product already used
409, and a client treats both the same way, so the contract keeps 409.

## Scope

A replay record is keyed by `(caller scope, operation, key digest)`.

- The caller scope is the owner the route acts for, such as the user, or the tenant plus actor. An
  anonymous route scopes by a client installation ID. The key alone is never global.
- The operation is a stable product name such as `create_note`, not the route template, so a path
  rename keeps old records valid.
- Path parameters that pick the target go into the fingerprint, not the scope. Reusing a key on
  another target then returns 409 instead of silently creating a second record.

## Fingerprint

The fingerprint is SHA-256 over canonical bytes of the request as the handler understood it.

1. Parse the body into the typed request and normalize it, such as trimming or defaulting.
2. Build `{"operation": ..., "target": ..., "expectedRevision": ..., "input": ...}` as a
   `serde_json::Value`. Baukit builds `serde_json` without `preserve_order`, so object members
   sort by key.
3. Serialize with `serde_json::to_vec` and hash.

Hashing the raw body makes a retry with different whitespace or member order look like a new
request. Hashing a Rust struct directly ties the digest to field declaration order, so moving a
field is a silent breaking change. Hashing a constant or an empty object makes every reuse replay,
so the conflict check is dead. Leave out server-injected values such as the actor or clock, since
the scope already covers the actor.

## Handler order and the transaction

```text
authenticate -> parse headers -> validate body -> fingerprint
BEGIN
  claim or read the replay row      (locks the key)
    replayable  -> return stored snapshot
    conflict    -> 409 idempotency_key_reused
  revision and state checks         (412, 409, 422 roll back; nothing is stored)
  apply the effect
  write the snapshot on the replay row
COMMIT
```

The claim, the effect, and the snapshot share one transaction. A rollback leaves no replay row, so
the next attempt runs as new. A pending row committed before the effect, with a second transaction
to finish it, is not allowed. A crash between the two leaves a key that can neither run nor replay.

Claim with `INSERT ... ON CONFLICT DO NOTHING RETURNING`, and on no row read it with
`SELECT ... FOR UPDATE`. A second request with the same key blocks on the unique index until the
first commits or rolls back, then replays or runs. A product that sets `lock_timeout` maps the
timeout to 409 `idempotency_key_in_progress`. An advisory lock on the scope works as well, as long
as the lookup happens after the lock.

Only committed effects get a snapshot. Validation and precondition failures are not replayed,
because the rollback removes the row.

## Stored result

Store the response status and body. A replay returns them unchanged. Rebuild `Location` from the
body. An `ETag` in a replay is the one the first response carried, which may be stale by then, and
the next `If-Match` write gets a 412 and reads again. Do not add a replay marker header. The
response is meant to be indistinguishable from the first one, and no client in the survey needed
to tell them apart.

## Expiry and cleanup

Every replay row has `expires_at`. The retention horizon is a product constant of 24 hours to
7 days. An expired row is invisible. A request with that key runs as new and replaces the row.
Clients must stop reusing a key before the server horizon, which is why the client key store takes
`ttlMs`.

Cleanup deletes expired rows in bounded batches, for example
`DELETE ... WHERE pk IN (SELECT pk ... WHERE expires_at < now() ORDER BY expires_at LIMIT $1 FOR
UPDATE SKIP LOCKED)`, looped until a batch comes back short or a per-run cap is hit.

On a table with `FORCE ROW LEVEL SECURITY`, a cleanup job with no owner setting sees no rows and
deletes nothing. Give cleanup its own role and a policy for it, such as
`CREATE POLICY ... FOR ALL TO <cleanup role> USING (expires_at < now())`, or run it as an owner
that holds `BYPASSRLS`. The conformance check fails a product whose cleanup deletes nothing.

## Erasure

Account erasure removes the owner's replay rows in the erasure transaction, through
`ON DELETE CASCADE` from the owner row or an explicit delete. A soft delete or pseudonymization of
the owner row does not fire the cascade, so products that erase that way delete replay rows
explicitly. The replay table belongs in the product's erasure inventory.

## Privacy

- Keys and bodies are never logged, traced, or put in metrics labels. `IdempotencyKey` hides its
  value from `Debug`.
- Error messages do not echo the key or the stored result.
- A snapshot holds only what the first response held. Snapshots of personal content live no longer
  than the retention horizon and go with erasure.

## Stored-result encryption

Encryption of snapshots is not required. It is required for one class: a response that carries a
value the product does not store in plaintext anywhere else, such as a new API token, a webhook
signing secret, or a one-time recovery code. Such a snapshot is encrypted with
`baukit-credential-vault`'s `CredentialCipher`, using the replay row's ID as the scope so the
associated data binds the ciphertext to that row. A product that does not want to hold the key
makes the route non-replayable and documents that a lost response means rotating again.

An ordinary snapshot is a copy of rows that sit in plaintext in the same database,
so encrypting the copy protects nothing an attacker with table access could not read next door.
It does add a key to rotate, a failure mode when the key is missing, and a decrypt on every replay.
A one-time secret is different, because the snapshot would be the only plaintext copy.

## Client

`@baukit/api-runtime/idempotency` keeps one key per account, operation, and body.

```ts
import { createApiFetch, IDEMPOTENCY_KEY_HEADER } from '@baukit/api-runtime';
import {
  createIdempotencyKeyStore,
  sendIdempotentMutation,
} from '@baukit/api-runtime/idempotency';

const keys = createIdempotencyKeyStore({ ttlMs: 12 * 60 * 60 * 1000 });
const apiFetch = createApiFetch({
  baseUrl: 'https://api.example.com',
  environment: 'production',
  retry: { methods: ['GET', 'HEAD', 'POST'] },
});

const intent = { account: accountId, operation: 'createNote', body: note };
await sendIdempotentMutation(keys, intent, (key) =>
  apiFetch('/notes', {
    method: 'POST',
    headers: { [IDEMPOTENCY_KEY_HEADER]: key, 'content-type': 'application/json' },
    body: JSON.stringify(note),
  }),
);
```

`classifyMutationError` sorts each failure into `committed`, `not-committed`, or
`possibly-committed`. Network errors, aborts, 408, 429, 5xx, and 409 `idempotency_key_in_progress`
are possibly committed, and the store keeps the key for the next attempt. Other 4xx responses are
not committed, and the store drops the key. A different body is a different intent and gets a new
key. The default store lives in memory. Pass `storage` to keep keys across reloads or app restarts.
A product whose API client throws its own error type passes `classifyError` to the store, so its
definite rejections drop the key instead of counting as possibly committed.

Two sends of one intent that overlap carry one key, because the store holds the key from the first
`keyFor` until the intent settles. The server then replays the first result or answers
`idempotency_key_in_progress`, and the effect happens once.

`createApiFetch` retries `POST` and `PATCH` only when `retry.methods` names them and the request
carries `Idempotency-Key`. An unkeyed `POST` never retries.

## Relation to revisioned writes

`@baukit/data-contracts/revisioned-writes` decides when to send and resend a document write. The
key store decides which key an attempt carries. They meet in the queue's write callback: a
`PATCH` with `If-Match` needs no key, while a keyed create puts the expected revision, if any, in
the intent body. After an unknown outcome the queue's `retry` sends the same value and revision,
which is the same intent, so the store hands back the same key.

## Testing a product

Implement `baukit_test::ReplaySafeMutationAdapter` against the product's real repository and a
fresh PostgreSQL database, and run `check_replay_safe_mutation_conformance`. The adapter calls
`CommitCheckpoint::reached` inside the transaction right before commit. The check covers a lost
response, an equivalent body, changed input under one key, two simultaneous requests, owner and
operation isolation, a rollback before commit, a crash after commit, expiry, bounded cleanup under
the product's roles, and erasure.
