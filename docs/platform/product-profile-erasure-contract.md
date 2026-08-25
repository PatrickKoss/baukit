# Product-profile erasure contract

**Status:** Contract and conformance boundary.
**Applies to:** Authenticated products that erase a user's product data.
**Related:** [local-data ownership](./local-data-ownership-contract.md),
[analytics privacy](./analytics-privacy-contract.md), and
[`@baukit/api-runtime`](../../typescript/packages/api-runtime/README.md).

Product-profile erasure removes the data owned by one user within one product. It
is separate from deletion of the user's account at an identity provider. A
product must state the scope in confirmation copy, progress messages, receipts,
and completion copy. A control that leaves the identity-provider account in place
must not claim to delete that account or all data everywhere.

The client-side composition lives in `@baukit/data-contracts` because it
coordinates the identity-scoped persistence lifecycle. Products inject their
server request, pre-server cleanup, local storage, and sign-out behavior.

Identity-provider account deletion remains a product-owned provider integration.
Baukit does not ship an account-deletion adapter. Product services should not
branch on Keycloak, Clerk, or another provider. A product that offers both
operations composes its provider adapter at the application boundary and
documents whether it runs before or after product-profile erasure.

## 1. Client operation

The client follows this order:

1. Run pre-server hooks, such as push-token unregister. These hooks are best
   effort and run in array order. Record a safe warning for each failure and
   continue to the server request.
2. Request authoritative server erasure with a stable idempotency key. Confirmed
   server success is either completed erasure or durable acceptance of an
   asynchronous operation after database deletion has committed and processor
   work has been registered.
3. After that confirmed server success, close and erase the active local
   partition, including its safe identity-registry entry.
4. Run sign-out in a `finally` path around local deletion. Sign-out therefore runs
   after confirmed server success even when local deletion fails.

A confirmed server failure stops before local deletion and sign-out. The session
and local partition remain available so the user can retry or recover. A timeout,
connection loss, cancellation, or unreadable response is not a confirmed server
failure because the server may already have committed the erasure.

`@baukit/data-contracts` exports
`eraseProductProfile(dependencies: ProductProfileErasureDependencies)`. Its
dependencies are optional `beforeServerErase` hooks and required
`eraseServerProfile`, `eraseLocalPartition`, and `signOut` functions. The helper
does not create an idempotency key, issue HTTP requests, or poll operation status.
The product-owned `eraseServerProfile` adapter performs that work and returns an
`ErasureReceipt`:

```ts
type ErasureReceipt =
  | { readonly operationId: string | null; readonly status: "completed" }
  | { readonly operationId: string; readonly status: "pending" };
```

An operation ID is non-empty when present, and a `pending` receipt always has
one. The helper treats a returned receipt with an invalid status or operation ID
as an ambiguous, unreadable server response. It preserves local data and the
session until the product reconciles the server outcome.

The result is a `ProductProfileErasureResult` with one of these exact `status`
values:

| Status | Result |
| --- | --- |
| `erased` | Server acceptance, local erasure, and sign-out succeeded. The result includes `receipt` and `warnings`; the receipt may be `completed` or `pending`. |
| `server-failure` | `eraseServerProfile` rejected with a failure that was not marked ambiguous. Local erasure and sign-out did not run. |
| `ambiguous` | `eraseServerProfile` rejected with `AmbiguousProductProfileErasureError` or an object whose `code` is `product_profile_erasure_ambiguous`. Local erasure and sign-out did not run. |
| `local-failure` | Server acceptance succeeded but local erasure failed. Sign-out was still attempted. The result includes `signOutError`, which is either a sign-out issue or `null`. |
| `signout-failure` | Server acceptance and local erasure succeeded, but sign-out failed. |

The shared `deletion-outcomes.json` fixture adds two test-policy fields outside
`ProductProfileErasureResult`. `serverRetry` is `retry` for `server-failure`,
`reconcile` for `ambiguous`, and `not-required` for the other outcomes.
`sessionRetained` is `true` for `server-failure`, `ambiguous`, and
`signout-failure`; it is `false` after the other outcomes.

Warnings and errors are `ProductProfileErasureIssue` values. Their `stage` is
`before-server`, `server`, `local`, or `sign-out`. Their `cause` is a bounded
error class name, not the original message. Unknown custom `Error` names become
`Error`, and non-`Error` values become `UnknownError`.

A local failure after server success must remain visible to the product so it can
tell the user that device cleanup is incomplete. The product must not present
that result as a full success. Products normally inject
a small `eraseLocalPartition` adapter that awaits
`ScopedPersistenceLifecycle.eraseActivePartition()` and handles its boolean
result. The lifecycle closes the store, resets user-scoped memory, deletes device
data, and removes the registry entry in that order.

Logs may contain bounded operation state, stable error codes, request IDs, an
opaque receipt ID, and the failed cleanup class. They must not contain email,
provider subject, access tokens, request bodies, resource contents, exported
data, object keys derived from user content, or local record values.

## 2. Ambiguous outcomes and idempotency

Create and retain an idempotency key before the first server request. Reuse the
same key after a timeout or connection loss. Do not erase local data or report
server acceptance until a retry or operation-status lookup confirms completed
erasure or a durably accepted asynchronous operation.

The product adapter must classify a timeout, connection loss, cancellation, or
unreadable response as ambiguous by rejecting with
`AmbiguousProductProfileErasureError`. That client error has the stable code
`product_profile_erasure_ambiguous`. An ordinary rejection is returned as
`server-failure`, so the helper cannot infer ambiguity from an error name such as
`TimeoutError` alone.

The server associates the idempotency key with the authenticated product profile
and erasure request. Reusing the key for the same request returns the same receipt
and the same terminal result. It must not start another erasure operation or
create a replacement profile. Reusing a key for an incompatible request is a
conflict.

An asynchronous operation has an opaque erasure operation ID, receipt ID, and a
safe status endpoint. The client persists enough non-sensitive operation state to
resume reconciliation after a restart. Status values distinguish at least
`pending`, `succeeded`, and `failed`. Terminal results are immutable. Status
lookup is authorized for the same identity and must not reveal whether another
user's receipt exists.

Those operation-status values belong to the product's HTTP protocol. They are
separate from `ErasureReceipt.status`, whose values are `completed` and
`pending`, and from the five `ProductProfileErasureResult.status` values above.

## 3. Product-owned HTTP responses and errors

Baukit supplies the result composition and the standard error envelope, but it
does not export an erasure endpoint, idempotency-key implementation, erasure
status model, or the erasure-specific error codes below. Each product defines
those pieces in its handler and OpenAPI description.

An endpoint may return `204 No Content` when the authoritative database deletion
completes during the request and no external processor work remains. A repeated
request with the same idempotency key returns the same terminal success.

If object storage, analytics, exports, provider revocation, or another external
processor must finish later, return `202 Accepted`. The response contains an
opaque receipt ID, the erasure operation ID, its current state, and the status
endpoint. It contains no user content. A `202` is valid only after authoritative
database deletion has committed and every remaining processor task has been
durably registered. The client can then erase the local partition and sign out,
but it presents the external work as pending until the status endpoint reports a
terminal result.

Every non-success response uses the standard envelope emitted by `baukit-http`
and parsed by `@baukit/api-runtime`:

```json
{
  "error": {
    "code": "product_profile_erasure_failed",
    "message": "The product profile could not be erased",
    "request_id": "...",
    "details": {}
  }
}
```

The minimum stable product code set is:

| Code | Meaning |
| --- | --- |
| `validation_failed` | The erasure request or idempotency key is malformed. |
| `unauthenticated` | A valid authenticated principal is required. |
| `permission_denied` | The principal may not erase the addressed profile. |
| `erasure_idempotency_conflict` | The key was already used for an incompatible request. |
| `erasure_operation_not_found` | The receipt is unknown or is not visible to this principal. |
| `product_profile_erasure_failed` | Authoritative erasure definitively failed without a success result. |
| `erasure_operation_failed` | An accepted asynchronous erasure reached a terminal failure. |

Codes are stable snake_case values. `message` is safe fallback copy, `details` is
a JSON object with safe structured values, and `request_id` connects the response
to internal diagnostics. Clients localize `code` plus `details` and use `message`
only as a fallback. Internal causes and processor payloads never cross the API.
Products document these responses in OpenAPI.

A transport timeout, including a `request_timeout` response when the server
cannot prove rollback, remains ambiguous. The client reconciles it with the same
idempotency key or the status endpoint. It does not translate a transport error
into `product_profile_erasure_failed` on its own.

## 4. Mandatory deletion inventory

Every product maintains an inventory that answers how and when each class is
removed:

- relational rows and soft-delete tombstones;
- outbox, inbox, retry, and scheduled job payloads;
- object storage and generated exports;
- device partitions, caches, files, and identity registry entries;
- push registrations and external integration credentials;
- analytics deletion or a documented retention policy;
- backups and the point at which natural expiry removes the data;
- the identity-provider identity, with a clear yes or no.

The confirmation UI and receipt use this inventory to make accurate claims.
Asynchronous processors include their expected completion or retention period.
Backup entries state the maximum time until expiry and whether restored backups
reapply erasure records before serving data.

## 5. Backend conformance

Each product maintains an owned-resource registry. Its entries follow the
`OwnedResourceCheck` struct: `name: &'static str`, `count_sql: &'static str`, and
`cleanup: CleanupKind`. The enum variants are `Cascade`, `Explicit`, and
`AsyncProcessor`. `Cascade` declares database cascade deletion, `Explicit`
declares deletion in product erasure code, and `AsyncProcessor` declares deletion
by a registered background processor. The main harness passes each entry to the
product adapter. It does not execute `count_sql` itself.

Products implement `ProductProfileErasureAdapter` with these methods:
`seed_user_owned_resource_graph`, `owned_resource_count`,
`erase_product_profile`, and `registered_background_job_count`. An adapter may
also return a non-empty `unseeded_resource_reason` when its fixture cannot create
a row for one `Cascade` or `Explicit` entry. The adapter error type must implement
`Error + Send + Sync + 'static`; its message is not included in harness output.

`check_product_profile_erasure_conformance(adapter, subject, resources)` returns
`Result<(), ErasureConformanceError>`. It reports an empty subject or registry,
empty resource names, duplicate resource names, empty count SQL, and count SQL
without the `$1` subject binding as violations. It then seeds once and counts
every registered resource and the aggregate registered-background-job count
before erasure. Every `Cascade` and `Explicit` entry must have at least one row
unless the adapter supplies its reason, and at least one registered resource
must be nonzero. The harness then invokes erasure and checks every resource and
the aggregate registered-background-job count for zero. It invokes erasure a
second time and checks the same counts again. The repeated invocation tests that
the adapter is safe to call twice. `ErasureConformanceError.violations()` exposes
the bounded violation strings. If an asynchronous processor must finish before
zero can be observed, the product adapter waits or polls inside
`erase_product_profile`; the harness has no processor-status or waiting API.

The product-owned graph includes relational descendants, tombstones, jobs, and
each registered resource class. A sampled happy-path graph is not sufficient if
the registry omits a user-owned resource class.

With the `sqlx-postgres` feature,
`audit_user_root_foreign_keys(pool, user_root_table, resources)` inspects direct
foreign keys to the product's user root and returns
`Result<Vec<ForeignKeyDeleteMismatch>, sqlx::Error>`. A registry entry declared
`Cascade` must use `ON DELETE CASCADE`. An unregistered direct reference is also
treated as `Cascade`, so a non-cascading action is returned as a mismatch.
Entries declared `Explicit` or `AsyncProcessor` are accepted with any database
delete action. Each mismatch contains `constraint_name`, schema-qualified
`referencing_table`, `actual_delete_action`, and `declared_cleanup`.

Resources without a direct foreign key, including job payloads and external
systems, remain mandatory registry entries and cannot be discovered by this
audit alone.

Conformance runs with isolated test identities and reports resource names and
counts only. Failure output and database diagnostics must not print inserted user
content, credentials, request bodies, or processor payloads.

## 6. Acceptance checks

- Pre-server hook failure produces a warning and does not block the authoritative
  request.
- Confirmed server failure preserves the session and local partition.
- Timeout and lost-response cases reuse the idempotency key and reconcile before
  any local deletion.
- Immediate success and asynchronous success erase the local partition, remove
  its registry entry, and attempt sign-out in the required order.
- Local deletion and sign-out failures remain distinguishable in the result.
- Repeated requests return the same receipt and terminal result.
- Confirmation and completion copy distinguish product-profile erasure from
  identity-provider account deletion.
- The deletion inventory addresses all eight classes, records `not applicable`
  where needed, and the conformance graph reaches zero for every registered
  resource.
- Direct user-root foreign keys declared `Cascade`, and unregistered direct
  foreign keys, fail the schema audit unless their database action is
  `ON DELETE CASCADE`.
- Tests and logs contain no user content or credentials.
